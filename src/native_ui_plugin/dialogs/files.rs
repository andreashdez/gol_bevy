//! Native file dialogs for workspaces and `.rle` files, and their shortcuts.
use super::*;
use crate::workspace::Workspace;
use bevy::tasks::{IoTaskPool, Task, futures::check_ready};
use std::path::Path;

pub(super) enum FileRequest {
    /// Captured at the click, so the file holds what was on screen then, even
    /// if playback continues while the user browses.
    Save(Box<Workspace>),
    Open,
    /// An `.rle` file to load into the import fields.
    OpenRle,
}

/// The native file dialog in progress, if any. It runs off the main thread so
/// the app keeps drawing while the user browses.
#[derive(Resource, Default)]
pub(in crate::native_ui_plugin) struct FileDialog(Option<(FileRequest, Task<Option<PathBuf>>)>);

pub(super) fn request_save(state: &mut DialogState) {
    if file_dialog_busy(state) {
        return;
    }
    match state.sim.app.workspace() {
        Ok(workspace) => request_file(state, FileRequest::Save(Box::new(workspace))),
        Err(error) => {
            state.drafts.error = true;
            state.drafts.feedback = format!("Could not save the workspace: {error}");
        }
    }
}

pub(super) fn request_open(state: &mut DialogState) {
    if !file_dialog_busy(state) {
        request_file(state, FileRequest::Open);
    }
}

pub(super) fn file_dialog_busy(state: &mut DialogState) -> bool {
    let busy = state.file_dialog.0.is_some();
    if busy {
        state.drafts.error = false;
        state.drafts.feedback = "A file dialog is already open.".into();
    }
    busy
}

pub(super) fn request_file(state: &mut DialogState, request: FileRequest) {
    let previous = state.drafts.workspace_path.as_deref();
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
    let task =
        IoTaskPool::get().spawn(async move { chosen.await.map(|file| file.path().to_owned()) });
    state.drafts.error = false;
    state.drafts.feedback = match request {
        FileRequest::Save(_) => "Choose where to save the workspace.",
        FileRequest::Open => "Choose a workspace file to open.",
        FileRequest::OpenRle => "Choose an RLE pattern file.",
    }
    .into();
    state.file_dialog.0 = Some((request, task));
}

pub(in crate::native_ui_plugin) fn poll_file_dialog(mut state: DialogState) {
    let Some((_, task)) = state.file_dialog.0.as_mut() else {
        return;
    };
    let Some(chosen) = check_ready(task) else {
        return;
    };
    let Some((request, _)) = state.file_dialog.0.take() else {
        return;
    };
    finish_file_request(&mut state, request, chosen);
}

pub(super) fn finish_file_request(
    state: &mut DialogState,
    request: FileRequest,
    chosen: Option<PathBuf>,
) {
    let Some(path) = chosen else {
        report(state, false, "No file was chosen.".into());
        return;
    };
    let workspace_file = !matches!(request, FileRequest::OpenRle);
    let result = match request {
        FileRequest::Save(workspace) => workspace
            .write(&path)
            .map(|()| format!("Workspace saved to {}.", path.display()))
            .map_err(|error| format!("Could not save {}: {error}", path.display())),
        // Validate the whole file first, so only a usable workspace asks to replace yours.
        FileRequest::Open => Workspace::read(&path)
            .and_then(Workspace::decode)
            .map(|decoded| {
                state.drafts.pending_open = Some((path.clone(), decoded));
                state.mode.dialog = Some(DialogKind::ConfirmOpen);
                String::new()
            })
            .map_err(|error| format!("Could not open {}: {error}", path.display())),
        FileRequest::OpenRle => load_rle(state, &path),
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

/// Puts a pattern file's text and name into the import fields, to preview
/// before importing.
fn load_rle(state: &mut DialogState, path: &Path) -> Result<String, String> {
    let fail =
        |error: &dyn std::fmt::Display| format!("Could not open {}: {error}", path.display());
    let size = std::fs::metadata(path).map_err(|e| fail(&e))?.len();
    if size > crate::patterns::MAX_RLE_BYTES as u64 {
        return Err(fail(&"the file is larger than 8 MiB"));
    }
    let source = std::fs::read_to_string(path).map_err(|e| fail(&e))?;
    let stem = path
        .file_stem()
        .map_or_else(|| "Imported pattern".into(), |stem| stem.to_string_lossy());
    // A file that does not parse still loads, so its error shows in the preview.
    state.drafts.name = crate::patterns::Pattern::from_named_rle(&stem, &source)
        .map_or_else(|_| stem.into_owned(), |pattern| pattern.name);
    state.drafts.source = source;
    state.drafts.rebuild = true;
    state.mode.dialog = Some(DialogKind::Import);
    Ok(format!(
        "Loaded {}. Check the preview, then import it.",
        path.display()
    ))
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
