//! What the web version does instead of file dialogs: it hands images and
//! animations to the browser to download. With one thread, a GIF is recorded a
//! slice at a time between frames rather than in the background.
use super::files::{FileWork, gif_saved, recording_progress, report};
use super::*;
use crate::{
    cell_gif::{GifColors, GifLength, GifRequest},
    cell_image::CellImage,
};
use bevy::platform::time::Instant;
use std::{cell::Cell, time::Duration};

/// How long a frame may spend recording, about half of a 60 Hz frame, so the
/// board and the dialog keep drawing meanwhile.
const RECORDING_BUDGET: Duration = Duration::from_millis(8);

pub(super) fn save_image(state: &mut DialogState, image: &CellImage, colors: [[u8; 3]; 2]) {
    let ((width, height), scale) = (image.size(), image.scale());
    let downloaded = image
        .to_png(scale, colors)
        .map_err(|error| error.to_string())
        .and_then(|file| download("board.png", "image/png", &file));
    match downloaded {
        Ok(()) => report(
            state,
            false,
            format!(
                "Downloaded {width} × {height} cells, {scale} pixels across each, as board.png."
            ),
        ),
        Err(error) => report(
            state,
            true,
            format!("Could not download the image: {error}"),
        ),
    }
}

pub(super) fn start_gif(state: &mut DialogState, gif: GifRequest, colors: GifColors) {
    let length = gif.length();
    match gif.recorder(colors) {
        Ok(recorder) => {
            let measuring = recorder.measuring();
            state.file_dialog.0 = Some(FileWork::Recording(Box::new(recorder), length));
            report(
                state,
                false,
                recording_progress(measuring, 0, total(length)),
            );
        }
        Err(error) => report(state, true, format!("Could not record the GIF: {error}")),
    }
}

/// Records as much of the GIF as fits in this frame, and downloads it once
/// it's done.
pub(super) fn keep_recording(state: &mut DialogState) {
    let Some(FileWork::Recording(recorder, length)) = state.file_dialog.0.as_mut() else {
        return;
    };
    let started = Instant::now();
    let finished = loop {
        match recorder.step() {
            Ok(true) => break Ok(()),
            Err(error) => break Err(error),
            Ok(false) if started.elapsed() >= RECORDING_BUDGET => {
                state.drafts.feedback =
                    recording_progress(recorder.measuring(), recorder.progress(), total(*length));
                return;
            }
            Ok(false) => {}
        }
    };
    let Some(FileWork::Recording(recorder, length)) = state.file_dialog.0.take() else {
        return;
    };
    let downloaded = finished
        .and_then(|()| recorder.finish())
        .map_err(|error| error.to_string())
        .and_then(|gif| {
            download("board.gif", "image/gif", &gif.file)?;
            Ok(gif_saved("Downloaded", length, &gif, "as board.gif"))
        });
    match downloaded {
        Ok(feedback) => report(state, false, feedback),
        Err(error) => report(state, true, format!("Could not record the GIF: {error}")),
    }
}

fn total(length: GifLength) -> Option<usize> {
    match length {
        GifLength::Generations(generations) => Some(generations),
        GifLength::Period => None,
    }
}

thread_local! {
    /// The address of the last download, kept until the next one, as the
    /// browser reads the file after the click that starts it.
    static LAST_DOWNLOAD: Cell<Option<String>> = const { Cell::new(None) };
}

/// Has the browser download `bytes` as `file_name`, through a link to them
/// clicked from code.
fn download(file_name: &str, mime: &str, bytes: &[u8]) -> Result<(), String> {
    use wasm_bindgen::{JsCast, JsValue};
    let fail = |error: JsValue| {
        error
            .as_string()
            .unwrap_or_else(|| "the browser refused the download".to_owned())
    };
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let options = web_sys::BlobPropertyBag::new();
    options.set_type(mime);
    let blob =
        web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).map_err(fail)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(fail)?;
    let document = web_sys::window()
        .and_then(|window| window.document())
        .ok_or("there is no page to download from")?;
    let body = document.body().ok_or("there is no page to download from")?;
    let link = document
        .create_element("a")
        .map_err(fail)?
        .dyn_into::<web_sys::HtmlAnchorElement>()
        .map_err(|_| "the browser made no link")?;
    link.set_href(&url);
    link.set_download(file_name);
    // Some browsers follow only links that are on the page.
    body.append_child(&link).map_err(fail)?;
    link.click();
    link.remove();
    if let Some(previous) = LAST_DOWNLOAD.replace(Some(url)) {
        let _ = web_sys::Url::revoke_object_url(&previous);
    }
    Ok(())
}
