//! Native file dialogs for workspaces and `.rle` files, and their shortcuts.
use super::*;
use crate::workspace::{DecodedWorkspace, Workspace};
use bevy::tasks::{IoTaskPool, Task, TaskPool, futures::check_ready};
use std::path::Path;

pub(super) enum FileRequest {
    /// Captured at the click, so the file holds what was on screen then, even
    /// if playback continues while the user browses.
    Save(Box<Workspace>),
    Open,
    /// An `.rle` file to load into the import fields.
    OpenRle,
}

/// The native file dialog in progress, if any, then the reading or writing of
/// the chosen file. Both run off the main thread so the app keeps drawing while
/// the user browses and while a large workspace is saved or decoded.
#[derive(Resource, Default)]
pub(in crate::native_ui_plugin) struct FileDialog(pub(super) Option<FileWork>);

pub(super) enum FileWork {
    Picking(FileRequest, Task<Option<PathBuf>>),
    Working(PathBuf, Task<FileOutcome>),
}

/// What the file task produced, applied to the app back on the main thread.
pub(super) enum FileOutcome {
    Saved(Result<(), String>),
    /// Validated in full, so only a usable workspace asks to replace yours.
    Opened(Result<Box<DecodedWorkspace>, String>),
    Rle(Result<String, String>),
}

pub(super) fn request_save(state: &mut DialogState) {
    if file_dialog_busy(state) {
        return;
    }
    match state.sim.app.workspace() {
        Ok(workspace) => request_file(state, FileRequest::Save(Box::new(workspace))),
        Err(error) => report(
            state,
            true,
            format!("Could not save the workspace: {error}"),
        ),
    }
}

pub(super) fn request_open(state: &mut DialogState) {
    if !file_dialog_busy(state) {
        request_file(state, FileRequest::Open);
    }
}

/// Whether a file dialog is already open, which Cmd/Ctrl+S or O from the board
/// must say over the board, as no dialog is there to show it.
pub(super) fn file_dialog_busy(state: &mut DialogState) -> bool {
    let notice = match state.file_dialog.0 {
        None => return false,
        Some(FileWork::Picking(..)) => "A file dialog is already open.",
        Some(FileWork::Working(..)) => "Still working on the last file.",
    };
    report(state, false, notice.into());
    true
}

pub(super) fn request_file(state: &mut DialogState, request: FileRequest) {
    let Some(task) = spawn_file_dialog(&request, state.drafts.workspace_path.as_deref()) else {
        report(
            state,
            true,
            "Files are not available in the web version.".into(),
        );
        return;
    };
    state.drafts.error = false;
    state.drafts.feedback = match request {
        FileRequest::Save(_) => "Choose where to save the workspace.",
        FileRequest::Open => "Choose a workspace file to open.",
        FileRequest::OpenRle => "Choose an RLE pattern file.",
    }
    .into();
    state.file_dialog.0 = Some(FileWork::Picking(request, task));
}

/// Opens the system file dialog off the main thread, starting where the last
/// workspace file was.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_file_dialog(
    request: &FileRequest,
    previous: Option<&Path>,
) -> Option<Task<Option<PathBuf>>> {
    let directory = previous
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(dirs::document_dir);
    let file_name = previous
        .and_then(Path::file_name)
        .map_or_else(|| "workspace.golw".into(), |name| name.to_string_lossy());
    let mut dialog = match request {
        FileRequest::OpenRle => rfd::AsyncFileDialog::new()
            .add_filter("RLE pattern", &["rle"])
            .add_filter("Text", &["txt"]),
        _ => rfd::AsyncFileDialog::new().add_filter("Game of Life workspace", &["golw"]),
    };
    if let Some(directory) = directory {
        dialog = dialog.set_directory(directory);
    }
    // The system dialog asks before replacing an existing file.
    let chosen: std::pin::Pin<Box<dyn Future<Output = Option<rfd::FileHandle>> + Send>> =
        match request {
            FileRequest::Save(_) => Box::pin(dialog.set_file_name(file_name).save_file()),
            FileRequest::Open | FileRequest::OpenRle => Box::pin(dialog.pick_file()),
        };
    Some(IoTaskPool::get().spawn(async move { chosen.await.map(|file| file.path().to_owned()) }))
}

/// The browser has no file paths to hand back, so the web build hides every
/// way to reach a file dialog.
#[cfg(target_arch = "wasm32")]
fn spawn_file_dialog(_: &FileRequest, _: Option<&Path>) -> Option<Task<Option<PathBuf>>> {
    None
}

pub(in crate::native_ui_plugin) fn poll_file_dialog(mut state: DialogState) {
    match state.file_dialog.0.as_mut() {
        Some(FileWork::Picking(_, task)) => {
            let Some(chosen) = check_ready(task) else {
                return;
            };
            let Some(FileWork::Picking(request, _)) = state.file_dialog.0.take() else {
                return;
            };
            finish_file_request(&mut state, request, chosen);
        }
        Some(FileWork::Working(_, task)) => {
            let Some(outcome) = check_ready(task) else {
                return;
            };
            let Some(FileWork::Working(path, _)) = state.file_dialog.0.take() else {
                return;
            };
            finish_file_work(&mut state, path, outcome);
        }
        None => {}
    }
}

/// Starts reading or writing the chosen file on the IO pool: serializing and
/// writing a save, reading and decoding a workspace, or reading an `.rle` file.
pub(super) fn finish_file_request(
    state: &mut DialogState,
    request: FileRequest,
    chosen: Option<PathBuf>,
) {
    let Some(path) = chosen else {
        report(state, false, "No file was chosen.".into());
        return;
    };
    let feedback = match request {
        FileRequest::Save(_) => "Saving…",
        FileRequest::Open | FileRequest::OpenRle => "Opening…",
    };
    report(state, false, feedback.into());
    let target = path.clone();
    let work = async move {
        let shown = target.display();
        match request {
            FileRequest::Save(workspace) => FileOutcome::Saved(
                workspace
                    .write(&target)
                    .map_err(|error| format!("Could not save {shown}: {error}")),
            ),
            FileRequest::Open => FileOutcome::Opened(
                Workspace::read(&target)
                    .and_then(Workspace::decode)
                    .map(Box::new)
                    .map_err(|error| format!("Could not open {shown}: {error}")),
            ),
            FileRequest::OpenRle => FileOutcome::Rle(
                read_rle(&target).map_err(|error| format!("Could not open {shown}: {error}")),
            ),
        }
    };
    let task = IoTaskPool::get_or_init(TaskPool::default).spawn(work);
    state.file_dialog.0 = Some(FileWork::Working(path, task));
}

/// Applies a finished file task and reports its result.
pub(super) fn finish_file_work(state: &mut DialogState, path: PathBuf, outcome: FileOutcome) {
    let workspace_file = !matches!(outcome, FileOutcome::Rle(_));
    let result = match outcome {
        FileOutcome::Saved(result) => {
            result.map(|()| format!("Workspace saved to {}.", path.display()))
        }
        FileOutcome::Opened(result) => result.map(|decoded| {
            state.drafts.pending_open = Some((path.clone(), *decoded));
            state.mode.dialog = Some(DialogKind::ConfirmOpen);
            String::new()
        }),
        FileOutcome::Rle(result) => result.map(|source| load_rle(state, &path, source)),
    };
    match result {
        Ok(feedback) => {
            // The next workspace dialog starts where the last workspace file was.
            if workspace_file {
                state.drafts.workspace_path = Some(path);
            }
            report(state, false, feedback);
        }
        Err(error) => report(state, true, error),
    }
}

/// Reads a pattern file, refusing one too large to import before reading it.
fn read_rle(path: &Path) -> Result<String, String> {
    let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if size > crate::patterns::MAX_RLE_BYTES as u64 {
        return Err("the file is larger than 8 MiB".into());
    }
    std::fs::read_to_string(path).map_err(|e| e.to_string())
}

/// Puts a pattern file's text and name into the import fields, to preview
/// before importing.
fn load_rle(state: &mut DialogState, path: &Path, source: String) -> String {
    let stem = path
        .file_stem()
        .map_or_else(|| "Imported pattern".into(), |stem| stem.to_string_lossy());
    // A file that does not parse still loads, so its error shows in the preview.
    state.drafts.name = crate::patterns::Pattern::from_named_rle(&stem, &source)
        .map_or_else(|_| stem.into_owned(), |pattern| pattern.name);
    state.drafts.source = source;
    state.drafts.rebuild = true;
    state.mode.dialog = Some(DialogKind::Import);
    format!(
        "Loaded {}. Check the preview, then import it.",
        path.display()
    )
}

/// Shows a result in the open dialog, or as a notice over the board when no
/// dialog shows feedback.
fn report(state: &mut DialogState, error: bool, feedback: String) {
    if !state.mode.dialog.is_some_and(DialogKind::shows_feedback) && !feedback.is_empty() {
        state.notice.show(feedback.clone());
    }
    state.drafts.error = error;
    state.drafts.feedback = feedback;
}

/// Cmd/Ctrl+S saves the workspace and Cmd/Ctrl+O opens one, from the board or
/// from the dialogs that show feedback.
pub(in crate::native_ui_plugin) fn file_shortcuts(
    keys: Res<ButtonInput<KeyCode>>,
    typed: Res<crate::shared::TypedKeys>,
    mut state: DialogState,
) {
    if crate::shared::WEB_BUILD {
        return;
    }
    match file_shortcut(&keys, &typed, state.mode.dialog) {
        Some(FileShortcut::Save) => request_save(&mut state),
        Some(FileShortcut::Open) => request_open(&mut state),
        None => {}
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum FileShortcut {
    Save,
    Open,
}

/// Which file shortcut was pressed, if any. Dialogs that show no file
/// feedback, such as Help, leave them alone.
pub(super) fn file_shortcut(
    keys: &ButtonInput<KeyCode>,
    typed: &crate::shared::TypedKeys,
    dialog: Option<DialogKind>,
) -> Option<FileShortcut> {
    let modifier = crate::input_plugin::COMMAND_KEYS;
    // Fast chords can press and release the modifier within one frame.
    let command = keys.any_pressed(modifier)
        || keys.any_just_pressed(modifier)
        || keys.any_just_released(modifier);
    if !command || dialog.is_some_and(|kind| !kind.shows_feedback()) {
        return None;
    }
    if typed.just_typed("s") {
        Some(FileShortcut::Save)
    } else if typed.just_typed("o") {
        Some(FileShortcut::Open)
    } else {
        None
    }
}

pub(super) fn confirm_open(state: &mut DialogState) {
    let Some((path, decoded)) = state.drafts.pending_open.take() else {
        return;
    };
    state.sim.app.restore_decoded(decoded);
    // Its cells may be anywhere, and a new size won't bring a plane into view.
    state.sim.fit_view = state.sim.app.board.finite().is_none();
    let (width, height) = state.sim.app.finite_size();
    state.board.set(width, height);
    state.board.fill_canvas = false;
    state.sim.accumulator = std::time::Duration::ZERO;
    state.sim.avg_step_ms = 0.0;
    state.sim.step_samples = 0;
    state.sim.rate = Default::default();
    state.mode.dialog = Some(DialogKind::Import);
    state.drafts.error = false;
    state.drafts.feedback = format!(
        "Opened {}. Playback is paused; undo history cleared and Fill canvas off.",
        path.display()
    );
}
