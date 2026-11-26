//! Native file dialogs for workspaces, `.rle` files, images, and animations,
//! and their shortcuts.
use super::*;
use crate::{
    cell_gif::{GifColors, GifLength, GifProgress, GifRecording, GifRequest},
    cell_image::CellImage,
    workspace::{DecodedWorkspace, Workspace},
};
use bevy::tasks::{IoTaskPool, Task, TaskPool, futures::check_ready};
use std::{path::Path, sync::Arc};

pub(super) enum FileRequest {
    /// Captured at the click, so the file holds what was on screen then, even
    /// if playback continues while the user browses.
    Save(Box<Workspace>),
    Open,
    /// An `.rle` file to load into the import fields.
    OpenRle,
    /// Captured at the click like a save, with the theme's colours for dead
    /// and live cells.
    SaveImage(Box<CellImage>, [[u8; 3]; 2]),
    /// A copy of the board to record from, taken at the click like an image,
    /// with the colours the board shows, by age if cell age is on.
    SaveGif(Box<GifRequest>, GifColors),
}

/// The native file dialog in progress, if any, then the reading or writing of
/// the chosen file. Both run off the main thread so the app keeps drawing while
/// the user browses and while a large workspace is saved or decoded.
#[derive(Resource, Default)]
pub(in crate::native_ui_plugin) struct FileDialog(pub(super) Option<FileWork>);

impl FileDialog {
    /// Whether a GIF is being recorded, which can be cancelled.
    pub(super) fn recording(&self) -> bool {
        match &self.0 {
            Some(FileWork::Working(_, _, progress)) => progress.is_some(),
            #[cfg(target_arch = "wasm32")]
            Some(FileWork::Recording(..)) => true,
            _ => false,
        }
    }
}

pub(super) enum FileWork {
    Picking(FileRequest, Task<Option<PathBuf>>),
    Working(PathBuf, Task<FileOutcome>, Option<Progress>),
    /// A GIF the web version records between frames, as it has one thread.
    #[cfg(target_arch = "wasm32")]
    Recording(Box<crate::cell_gif::GifRecorder>, GifLength),
}

/// How far a GIF recording has got, of how many generations if that's known
/// before the board comes back round, and the way to cancel it.
pub(super) struct Progress {
    shared: Arc<GifProgress>,
    total: Option<usize>,
}

/// What the file task produced, applied to the app back on the main thread.
pub(super) enum FileOutcome {
    Saved(Result<(), String>),
    /// Validated in full, so only a usable workspace asks to replace yours.
    Opened(Result<Box<DecodedWorkspace>, String>),
    Rle(Result<String, String>),
    /// What an image or animation saved, to report.
    ImageSaved(Result<String, String>),
    /// A recording stopped before it finished, saving nothing.
    Cancelled,
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

/// Saves the selection, or else the board, as a PNG in the theme's colours.
pub(super) fn request_save_image(state: &mut DialogState) {
    if file_dialog_busy(state) {
        return;
    }
    match state.sim.app.board_image() {
        Ok(image) => {
            let colors = cell_colors(state);
            request_file(state, FileRequest::SaveImage(Box::new(image), colors));
        }
        Err(error) => report(state, true, error.to_string()),
    }
}

/// Records the selection, or else the board, as a GIF.
pub(super) fn request_save_gif(state: &mut DialogState, length: GifLength) {
    if file_dialog_busy(state) {
        return;
    }
    match state.sim.app.board_gif(length) {
        Ok(request) => {
            let colors =
                crate::render_plugin::gif_colors(state.theme.board(), state.cell_age.enabled);
            request_file(state, FileRequest::SaveGif(Box::new(request), colors));
        }
        Err(error) => report(state, true, error.to_string()),
    }
}

/// The theme's dead and live cell colours.
fn cell_colors(state: &DialogState) -> [[u8; 3]; 2] {
    let colors = state.theme.board();
    let rgb = |color| {
        let [red, green, blue, _] = crate::render_plugin::color_to_texel(color);
        [red, green, blue]
    };
    [rgb(colors.dead), rgb(colors.live)]
}

/// Whether a file dialog is already open, which Cmd/Ctrl+S or O from the board
/// must say over the board, as no dialog is there to show it.
pub(super) fn file_dialog_busy(state: &mut DialogState) -> bool {
    let notice = match state.file_dialog.0 {
        None => return false,
        Some(FileWork::Picking(..)) => "A file dialog is already open.",
        Some(FileWork::Working(..)) => "Still working on the last file.",
        #[cfg(target_arch = "wasm32")]
        Some(FileWork::Recording(..)) => "Still recording the last GIF.",
    };
    report(state, false, notice.into());
    true
}

pub(super) fn request_file(state: &mut DialogState, request: FileRequest) {
    let Some(task) = spawn_file_dialog(&request, state.drafts.workspace_path.as_deref()) else {
        without_file_dialog(state, request);
        return;
    };
    state.drafts.error = false;
    state.drafts.feedback = match request {
        FileRequest::Save(_) => "Choose where to save the workspace.",
        FileRequest::Open => "Choose a workspace file to open.",
        FileRequest::OpenRle => "Choose an RLE pattern file.",
        FileRequest::SaveImage(..) => "Choose where to save the image.",
        FileRequest::SaveGif(..) => "Choose where to save the GIF.",
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
    let file_name = match request {
        FileRequest::SaveImage(..) => "board.png".into(),
        FileRequest::SaveGif(..) => "board.gif".into(),
        _ => previous
            .and_then(Path::file_name)
            .map_or_else(|| "workspace.golw".into(), |name| name.to_string_lossy()),
    };
    let mut dialog = match request {
        FileRequest::OpenRle => rfd::AsyncFileDialog::new()
            .add_filter("RLE pattern", &["rle"])
            .add_filter("Text", &["txt"]),
        FileRequest::SaveImage(..) => rfd::AsyncFileDialog::new().add_filter("PNG image", &["png"]),
        FileRequest::SaveGif(..) => {
            rfd::AsyncFileDialog::new().add_filter("GIF animation", &["gif"])
        }
        _ => rfd::AsyncFileDialog::new().add_filter("Game of Life workspace", &["golw"]),
    };
    if let Some(directory) = directory {
        dialog = dialog.set_directory(directory);
    }
    // The system dialog asks before replacing an existing file.
    let chosen: std::pin::Pin<Box<dyn Future<Output = Option<rfd::FileHandle>> + Send>> =
        match request {
            FileRequest::Save(_) | FileRequest::SaveImage(..) | FileRequest::SaveGif(..) => {
                Box::pin(dialog.set_file_name(file_name).save_file())
            }
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

/// Without file dialogs, the web version downloads images and animations, and
/// has nowhere to put anything else.
#[cfg(target_arch = "wasm32")]
fn without_file_dialog(state: &mut DialogState, request: FileRequest) {
    match request {
        FileRequest::SaveImage(image, colors) => super::browser::save_image(state, &image, colors),
        FileRequest::SaveGif(gif, colors) => super::browser::start_gif(state, *gif, colors),
        _ => report(
            state,
            true,
            "Files are not available in the web version.".into(),
        ),
    }
}

/// The native file dialog is always there.
#[cfg(not(target_arch = "wasm32"))]
fn without_file_dialog(state: &mut DialogState, _: FileRequest) {
    report(state, true, "No file dialog is available.".into());
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
        Some(FileWork::Working(_, task, progress)) => {
            let Some(outcome) = check_ready(task) else {
                if let Some(Progress { shared, total }) = progress
                    && !shared.cancelled()
                {
                    let feedback =
                        recording_progress(shared.measuring(), shared.generations(), *total);
                    if state.drafts.feedback != feedback {
                        state.drafts.feedback = feedback;
                    }
                }
                return;
            };
            let Some(FileWork::Working(path, ..)) = state.file_dialog.0.take() else {
                return;
            };
            finish_file_work(&mut state, path, outcome);
        }
        #[cfg(target_arch = "wasm32")]
        Some(FileWork::Recording(..)) => super::browser::keep_recording(&mut state),
        None => {}
    }
}

/// The work finishes off the main thread, which can't wake the app itself.
pub(in crate::native_ui_plugin) fn keep_awake_while_working(
    dialog: Res<FileDialog>,
    mut awake: MessageWriter<crate::frame_pacing::StayAwake>,
) {
    if dialog.0.is_some() {
        awake.write(crate::frame_pacing::StayAwake);
    }
}

/// Starts reading or writing the chosen file on the IO pool: serializing and
/// writing a save, reading and decoding a workspace, reading an `.rle` file, or
/// drawing and writing an image or an animation.
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
        FileRequest::Save(_) | FileRequest::SaveImage(..) | FileRequest::SaveGif(..) => "Saving…",
        FileRequest::Open | FileRequest::OpenRle => "Opening…",
    };
    report(state, false, feedback.into());
    let progress = match &request {
        FileRequest::SaveGif(gif, _) => Some(Progress {
            shared: Arc::default(),
            total: match gif.length() {
                GifLength::Generations(generations) => Some(generations),
                GifLength::Period => None,
            },
        }),
        _ => None,
    };
    let shared = progress
        .as_ref()
        .map(|progress| Arc::clone(&progress.shared));
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
            FileRequest::SaveImage(image, colors) => {
                let ((width, height), scale) = (image.size(), image.scale());
                FileOutcome::ImageSaved(
                    image
                        .to_png(scale, colors)
                        .and_then(|file| crate::atomic_file::write(&target, &file))
                        .map(|()| {
                            format!(
                                "Saved {width} × {height} cells, {scale} pixels across each, to {shown}."
                            )
                        })
                        .map_err(|error| format!("Could not save {shown}: {error}")),
                )
            }
            FileRequest::SaveGif(gif, colors) => {
                let progress = shared.expect("a GIF has its progress");
                let length = gif.length();
                match gif.record(colors, &progress) {
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                        FileOutcome::Cancelled
                    }
                    recorded => FileOutcome::ImageSaved(
                        recorded
                            .and_then(|gif| {
                                crate::atomic_file::write(&target, &gif.file)?;
                                Ok(gif_saved("Saved", length, &gif, &format!("to {shown}")))
                            })
                            .map_err(|error| format!("Could not save {shown}: {error}")),
                    ),
                }
            }
        }
    };
    let task = IoTaskPool::get_or_init(TaskPool::default).spawn(work);
    state.file_dialog.0 = Some(FileWork::Working(path, task, progress));
}

pub(super) const RECORDING_CANCELLED: &str = "Recording cancelled; nothing was saved.";

/// Stops the GIF being recorded. The background one stops before its next
/// generation and reports then; the web version's stops at once.
pub(super) fn cancel_recording(state: &mut DialogState) {
    match &state.file_dialog.0 {
        Some(FileWork::Working(_, _, Some(progress))) => {
            progress.shared.cancel();
            report(state, false, "Cancelling…".into());
        }
        #[cfg(target_arch = "wasm32")]
        Some(FileWork::Recording(..)) => {
            state.file_dialog.0 = None;
            report(state, false, RECORDING_CANCELLED.into());
        }
        _ => {}
    }
}

/// How far a GIF recording has got, of `total` generations if that's known,
/// and whether it's still measuring how far the pattern grows first.
pub(super) fn recording_progress(measuring: bool, done: usize, total: Option<usize>) -> String {
    let what = if measuring {
        "Measuring how far it grows…"
    } else {
        "Recording…"
    };
    match total {
        Some(total) => format!("{what} {done} of {total} generations."),
        None => format!("{what} {done} generations, until the board comes back round."),
    }
}

/// What a saved GIF holds and where it went, `verb` and `place` as in "Saved …
/// to board.gif".
pub(super) fn gif_saved(verb: &str, length: GifLength, gif: &GifRecording, place: &str) -> String {
    let ((width, height), scale, generations) = (gif.size, gif.scale, gif.generations);
    let what = match length {
        GifLength::Period => format!("one period, {generations} generations,"),
        GifLength::Generations(_) => format!("{generations} generations"),
    };
    let early = match length {
        GifLength::Generations(asked) if generations < asked => {
            " It stops early, where the board reached its memory limit."
        }
        _ => "",
    };
    format!(
        "{verb} {what} of {width} × {height} cells, {scale} pixels across each, {place}.{early}"
    )
}

/// Applies a finished file task and reports its result.
pub(super) fn finish_file_work(state: &mut DialogState, path: PathBuf, outcome: FileOutcome) {
    // Images and patterns aren't where the next workspace dialog should start.
    let workspace_file = matches!(outcome, FileOutcome::Saved(_) | FileOutcome::Opened(_));
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
        FileOutcome::ImageSaved(result) => result,
        FileOutcome::Cancelled => Ok(RECORDING_CANCELLED.into()),
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
pub(super) fn report(state: &mut DialogState, error: bool, feedback: String) {
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
    typed: Res<crate::keybindings::TypedKeys>,
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
    typed: &crate::keybindings::TypedKeys,
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
