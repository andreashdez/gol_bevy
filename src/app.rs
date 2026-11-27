//! Application state and the user actions on it, outside of Bevy.
//!
//! This file holds the [`App`] struct, its constructor, and the board editing
//! at its core. Each child module adds an `impl App` block for one concern:
//! `history` for undo, redo, and the start of a run; `stamp` for the pattern
//! library and the stamp; `selection` for the Select tool and RLE and image
//! export; and `playback` for running, speed, settling, and population history.

mod history;
mod playback;
mod selection;
mod stamp;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub use self::stamp::PatternKind;
use self::{
    history::{EditHistory, EditSnapshot},
    selection::Selection,
    stamp::StampTransform,
};
pub use self::{
    playback::{AutoPause, JumpError, POPULATION_HISTORY},
    selection::{
        BoardExport, BoardExportError, ImageExportError, RecordedArea, SelectionExportError,
    },
    stamp::{PasteError, PatternImport},
};
use crate::{
    board::{self, BoundaryMode, RandomizeSettings},
    patterns::Pattern,
    rule::Rule,
    settle::SettleWatch,
    shared::BOARD_SIZE_MAX,
    sparse_board::{CellRect, PLANE_CHUNKS_MAX, UnboundedRuleError},
    universe::{Universe, board_from_plane, plane_from_board},
};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EditTool {
    #[default]
    Draw,
    Erase,
    Stamp,
    Select,
}

/// Application.
#[derive(Debug)]
pub struct App {
    /// should the application exit?
    pub should_quit: bool,
    /// current epoch
    pub epoch: usize,
    /// tick interval in milliseconds
    tick_rate_ms: u64,
    /// run as many generations per frame as time allows, ignoring the interval
    max_speed: bool,
    /// whether simulation advances on each tick
    running: bool,
    /// The generation a jump runs to. Only counts while running: anything that
    /// pauses ends the jump.
    jump_target: Option<usize>,
    /// The Jump dialog was asked for from the keyboard, until the UI opens it.
    jump_requested: bool,
    /// whether UI should redraw
    needs_redraw: bool,
    /// whether help overlay is visible
    show_help: bool,
    /// selected pattern stamp for cursor placement
    selected_pattern: Option<usize>,
    /// A stamp pasted from the clipboard, kept out of the library. While set,
    /// it is the stamp instead of `selected_pattern`.
    pasted: Option<Pattern>,
    /// Counts pastes, so a view of the stamp can tell one paste from the next.
    paste_revision: u64,
    /// transform applied to stamp before placement
    stamp_transform: StampTransform,
    /// available stamp patterns
    patterns: Vec<Pattern>,
    library_revision: u64,
    randomize_settings: RandomizeSettings,
    tool: EditTool,
    /// The palette was hidden by hand, while stamping, to reach the cells under it.
    palette_hidden: bool,
    undo_history: EditHistory,
    redo_history: EditHistory,
    stroke_active: bool,
    stroke_saved: bool,
    selection: Option<Selection>,
    /// The latest generation, with the live cells before and after it.
    last_step: Option<(usize, usize, usize)>,
    /// The board before the first generation since the latest edit.
    run_start: Option<EditSnapshot>,
    settle: SettleWatch,
    pause_when_settled: bool,
    /// Why playback paused itself, until the UI has said so.
    auto_paused: Option<AutoPause>,
    /// Live cells per generation of this run, oldest first.
    population_history: VecDeque<usize>,

    /// The size the board has, or has to return to from the unbounded board.
    finite_size: (usize, usize),
    /// Chunks a generation may not take the unbounded board past.
    chunk_budget: usize,
    /// The part of the unbounded board on screen.
    view_cells: Option<CellRect>,

    pub board: Universe,
}

impl App {
    /// Constructs a new instance of [`App`].
    pub fn new(cols: usize, rows: usize) -> Self {
        let board = Universe::Finite(board::Board::new(cols, rows));
        Self {
            should_quit: false,
            epoch: 0,
            tick_rate_ms: 25,
            max_speed: false,
            running: false,
            jump_target: None,
            jump_requested: false,
            needs_redraw: true,
            show_help: false,
            selected_pattern: None,
            pasted: None,
            paste_revision: 0,
            stamp_transform: StampTransform::default(),
            patterns: Pattern::builtins(),
            library_revision: 0,
            randomize_settings: RandomizeSettings::default(),
            tool: EditTool::Draw,
            palette_hidden: false,
            undo_history: EditHistory::default(),
            redo_history: EditHistory::default(),
            stroke_active: false,
            stroke_saved: false,
            selection: None,
            last_step: None,
            run_start: None,
            settle: SettleWatch::default(),
            pause_when_settled: false,
            auto_paused: None,
            population_history: VecDeque::with_capacity(POPULATION_HISTORY),
            finite_size: (cols, rows),
            chunk_budget: PLANE_CHUNKS_MAX,
            view_cells: None,
            board,
        }
    }

    pub fn is_help_visible(&self) -> bool {
        self.show_help
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
        self.needs_redraw = true;
    }

    pub fn set_help_visible(&mut self, visible: bool) {
        if self.show_help != visible {
            self.show_help = visible;
            self.needs_redraw = true;
        }
    }

    /// Steps through wrapping, fixed, and unbounded edges. A rule that can't run
    /// unbounded goes straight back to wrapping.
    pub fn toggle_boundary_mode(&mut self) {
        match self.board.boundary_mode() {
            Some(BoundaryMode::Wrap) => self.set_boundary_mode(BoundaryMode::Fixed),
            Some(BoundaryMode::Fixed) => {
                if self.set_unbounded().is_err() {
                    self.set_boundary_mode(BoundaryMode::Wrap);
                }
            }
            None => self.set_boundary_mode(BoundaryMode::Wrap),
        }
    }

    /// Choosing edges on the unbounded board returns to a finite one, keeping
    /// only the cells that fit it. Undo brings the rest back.
    pub fn set_boundary_mode(&mut self, mode: BoundaryMode) {
        if let Universe::Unbounded(plane) = &self.board {
            let (cols, rows) = self.finite_size;
            let board = board_from_plane(plane, cols, rows, mode);
            self.replace_board(Universe::Finite(board));
            return;
        }
        if Some(mode) != self.board.boundary_mode() {
            self.settle.reset();
        }
        self.board.set_boundary_mode(mode);
        self.needs_redraw = true;
    }

    /// Moves the board onto the unbounded plane, which can't hold a B0 rule.
    /// Returns whether it moved.
    pub fn set_unbounded(&mut self) -> Result<bool, UnboundedRuleError> {
        let Universe::Finite(board) = &self.board else {
            return Ok(false);
        };
        let plane = plane_from_board(board)?;
        self.replace_board(Universe::Unbounded(plane));
        Ok(true)
    }

    /// An edit like any other, since a snapshot brings back the board that took it.
    fn replace_board(&mut self, board: Universe) {
        self.end_stroke();
        self.save_edit();
        // Cells move between the board and the plane, so the selection would not
        // cover the same ones.
        self.clear_selection();
        self.board = board;
        self.needs_redraw = true;
    }

    /// The board's size, or the one it will have on leaving the unbounded board.
    pub fn finite_size(&self) -> (usize, usize) {
        self.finite_size
    }

    pub fn boundary_mode_label(&self) -> &'static str {
        self.board.boundary_mode_label()
    }

    /// Set should_quit to true to quit the application.
    pub fn quit(&mut self) {
        self.should_quit = true;
        self.needs_redraw = true;
    }

    pub fn randomize_settings(&self) -> RandomizeSettings {
        self.randomize_settings
    }

    /// Preferences apply to the next randomization and do not edit the board.
    pub fn set_randomize_settings(&mut self, settings: RandomizeSettings) {
        self.randomize_settings = settings;
    }

    /// Tells the app what the canvas shows of the unbounded board.
    pub fn set_view_cells(&mut self, view: CellRect) {
        self.view_cells = Some(view);
    }

    /// What randomizing fills on the unbounded board, which has no size of its
    /// own: as much of the view as a board could hold, or a board's worth around
    /// the origin if nothing has been drawn yet. `None` on a finite board.
    pub fn randomize_area(&self) -> Option<CellRect> {
        self.board.finite().is_none().then(|| {
            let (cols, rows) = self.finite_size;
            self.view_cells
                .unwrap_or_else(|| CellRect::around_origin(cols, rows))
                .middle(BOARD_SIZE_MAX)
        })
    }

    /// Replaces every cell of a finite board. The unbounded board has too many,
    /// so the cells outside [`randomize_area`](Self::randomize_area) stay.
    pub fn randomize_board(&mut self) {
        self.end_stroke();
        self.save_edit();
        let area = self.randomize_area();
        match &mut self.board {
            Universe::Finite(board) => board.populate_random_with(self.randomize_settings),
            Universe::Unbounded(plane) => {
                if let Some(area) = area {
                    plane.replace_rect(area, self.randomize_settings.draws());
                }
            }
        }
        self.needs_redraw = true;
    }

    pub fn clear_board(&mut self) {
        if self.board.live_cells_count() == 0 {
            return;
        }
        self.end_stroke();
        self.save_edit();
        if self.board.clear() {
            self.needs_redraw = true;
        }
    }

    pub fn populate_board(&mut self, col: i64, row: i64) {
        if !self.board.contains(col, row) || self.board.is_alive(col, row) {
            return;
        }
        self.save_edit();
        if self.board.populate(col, row) {
            self.needs_redraw = true;
        }
    }

    pub fn depopulate_board(&mut self, col: i64, row: i64) {
        if !self.board.is_alive(col, row) {
            return;
        }
        self.save_edit();
        if self.board.depopulate(col, row) {
            self.needs_redraw = true;
        }
    }

    pub fn resize_board(&mut self, new_cols: usize, new_rows: usize) {
        self.finite_size = (new_cols, new_rows);
        match &mut self.board {
            Universe::Finite(board) => {
                if board.resize(new_cols, new_rows) {
                    // Undo history survives: a snapshot from another size brings
                    // back the cells that fit this one.
                    self.forget_run();
                    self.end_stroke();
                    self.needs_redraw = true;
                }
            }
            // The plane has no size to change, only finite boards in its history.
            Universe::Unbounded(_) => self.forget_finite_edits(),
        }
    }

    pub fn take_redraw(&mut self) -> bool {
        let redraw = self.needs_redraw;
        self.needs_redraw = false;
        redraw
    }

    pub fn rule_label(&self) -> String {
        self.board.rule().to_string()
    }

    /// Changes the board rule as an undoable edit, so undo can return to the old
    /// rule. Returns whether the rule actually changed.
    pub fn set_rule(&mut self, rule: Rule) -> bool {
        if rule == self.board.rule() || !self.board.allows_rule(rule) {
            return false;
        }
        self.end_stroke();
        self.save_edit();
        self.board.set_rule(rule);
        self.needs_redraw = true;
        true
    }

    /// Capture the board and imported library for an explicit workspace save.
    pub fn workspace(
        &self,
    ) -> Result<crate::workspace::Workspace, crate::workspace::WorkspaceError> {
        crate::workspace::Workspace::capture(self, &self.patterns[Pattern::builtin_count()..])
    }

    /// Validate first, then replace the workspace atomically and leave playback paused.
    pub fn restore_workspace(
        &mut self,
        workspace: crate::workspace::Workspace,
    ) -> Result<(), crate::workspace::WorkspaceError> {
        self.restore_decoded(workspace.decode()?);
        Ok(())
    }

    /// Replace the workspace with one already validated by [`Workspace::decode`],
    /// so a caller can confirm with the user between validating and replacing.
    ///
    /// [`Workspace::decode`]: crate::workspace::Workspace::decode
    pub(crate) fn restore_decoded(&mut self, decoded: crate::workspace::DecodedWorkspace) {
        let mut restored = Self::new(decoded.width, decoded.height);
        restored.board = decoded.board;
        restored.epoch = decoded.generation;
        restored.randomize_settings = decoded.randomize_settings;
        restored.set_tick_rate_ms(decoded.tick_rate_ms);
        if decoded.max_speed {
            restored.set_max_speed();
        }
        restored.patterns.extend(decoded.patterns);
        restored.library_revision = self.library_revision.wrapping_add(1);
        restored.pause_when_settled = self.pause_when_settled;
        *self = restored;
    }

    pub fn tool(&self) -> EditTool {
        self.tool
    }

    /// Choosing a tool, even the one in use, shows the palette again if it was
    /// hidden: it is how a pointer brings the palette back.
    pub fn set_tool(&mut self, tool: EditTool) {
        self.end_stroke();
        if tool != EditTool::Select {
            self.clear_selection();
        }
        self.tool = tool;
        self.palette_hidden = false;
        if tool == EditTool::Stamp && self.stamp_pattern().is_none() && !self.patterns.is_empty() {
            self.selected_pattern = Some(0);
        }
        self.needs_redraw = true;
    }

    /// The palette shows while stamping, unless hidden by hand.
    pub fn palette_shown(&self) -> bool {
        self.tool == EditTool::Stamp && !self.palette_hidden
    }

    /// Only stamping has a palette to show or hide.
    pub fn toggle_palette(&mut self) {
        if self.tool == EditTool::Stamp {
            self.palette_hidden = !self.palette_hidden;
        }
    }
}
