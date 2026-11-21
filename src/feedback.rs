//! Short, non-modal notices for edits made from the toolbar or the keyboard.
use std::borrow::Cow;

use crate::app::App as GolApp;
use bevy::prelude::*;

/// Short, non-modal feedback shared by toolbar and keyboard edits.
#[derive(Resource, Default)]
pub struct EditFeedback {
    pub message: Cow<'static, str>,
    pub remaining: f32,
    /// A Cmd/Ctrl+V clipboard read still on its way. Native reads finish at
    /// once; browsers hand the text over a frame or more later.
    pending_paste: Option<bevy::clipboard::ClipboardRead>,
}

impl EditFeedback {
    /// Seconds a notice stays on screen. Brief, since it covers part of the board.
    const DURATION: f32 = 2.0;

    pub fn show(&mut self, message: impl Into<Cow<'static, str>>) {
        self.message = message.into();
        self.remaining = Self::DURATION;
    }

    pub fn undo(&mut self, app: &mut GolApp) {
        self.show(if app.undo() {
            "Edit undone"
        } else {
            "Nothing to undo"
        });
    }

    pub fn redo(&mut self, app: &mut GolApp) {
        self.show(if app.redo() {
            "Edit redone · U to undo"
        } else {
            "Nothing to redo"
        });
    }

    pub fn reset_to_start(&mut self, app: &mut GolApp) {
        self.show(if app.reset_to_start() {
            "Reset to start · U to undo"
        } else {
            "Nothing to reset"
        });
    }

    pub fn clear(&mut self, app: &mut GolApp) {
        if app.board.live_cells_count() == 0 {
            self.show("Board is already empty");
        } else {
            app.clear_board();
            self.show("Board cleared · U to undo");
        }
    }

    pub fn randomize(&mut self, app: &mut GolApp) {
        app.randomize_board();
        self.show(format!("{} randomized · U to undo", randomized_what(app)));
    }

    pub fn select_live_cells(&mut self, app: &mut GolApp) {
        if !app.select_live_cells() {
            self.show("Board is empty · nothing to select");
        } else if let Some(area) = app.selection() {
            self.show(format!("Selected {} × {} cells", area.width, area.height));
        }
    }

    pub fn erase_selection(&mut self, app: &mut GolApp) {
        if app.selection().is_none() {
            return;
        }
        match app.erase_selection() {
            0 => self.show("No live cells in the selection"),
            cells => self.show(format!("Cleared {cells} cells · U to undo")),
        }
    }

    /// Makes the RLE on the clipboard, such as a copied selection, the stamp.
    pub fn paste(
        &mut self,
        app: &mut GolApp,
        clipboard: Option<ResMut<bevy::clipboard::Clipboard>>,
    ) {
        let Some(mut clipboard) = clipboard else {
            self.show("Could not read the clipboard: no clipboard is available");
            return;
        };
        self.pending_paste = Some(clipboard.fetch_text());
        self.finish_paste(app);
    }

    /// Makes the clipboard text the stamp once the read started by `paste`
    /// has finished. Until then, and with no paste in progress, does nothing.
    pub fn finish_paste(&mut self, app: &mut GolApp) {
        let Some(result) = self
            .pending_paste
            .as_mut()
            .and_then(|read| read.poll_result())
        else {
            return;
        };
        self.pending_paste = None;
        let text = match result {
            Ok(text) => text,
            Err(error) => {
                self.show(format!("Could not read the clipboard: {error}"));
                return;
            }
        };
        match app.paste_rle(&text) {
            Ok(pattern) => self.show(format!(
                "Pasted {} × {} · click to place · Z rotates, F flips",
                pattern.width, pattern.height
            )),
            Err(error) => self.show(error.to_string()),
        }
    }

    /// Copies the selection to the clipboard as RLE, and with `cut` clears it.
    /// Nothing is cleared unless the copy went through.
    pub fn copy_selection(
        &mut self,
        app: &mut GolApp,
        clipboard: Option<ResMut<bevy::clipboard::Clipboard>>,
        cut: bool,
    ) {
        let export = match app.selection_to_rle() {
            Ok(export) => export,
            Err(error) => {
                self.show(error.to_string());
                return;
            }
        };
        let copied = match clipboard {
            Some(mut clipboard) => clipboard.set_text(export.rle).map_err(|e| e.to_string()),
            None => Err("no clipboard is available".to_owned()),
        };
        if let Err(error) = copied {
            self.show(format!("Could not copy to the clipboard: {error}"));
            return;
        }
        let what = format!(
            "{} live cells ({} × {})",
            export.cells, export.width, export.height
        );
        if cut {
            app.erase_selection();
            self.show(format!("Cut {what} as RLE · U to undo"));
        } else {
            self.show(format!("Copied {what} as RLE"));
        }
    }
}

/// What randomizing covers, for telling the user: on the unbounded board, an area
/// that doesn't follow from anything on screen once the view is larger.
pub fn randomized_what(app: &GolApp) -> Cow<'static, str> {
    match app.randomize_area() {
        Some(area) => format!("{} × {} cells", area.width, area.height).into(),
        None => "Board".into(),
    }
}
