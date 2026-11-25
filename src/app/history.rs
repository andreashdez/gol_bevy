//! Undo and redo, the strokes that group edits into one, and the board kept
//! from before a run so it can be reset to.

use super::App;
use crate::{rule::Rule, universe::CellSnapshot};
use std::collections::VecDeque;

const EDIT_HISTORY_MAX: usize = 32;
const EDIT_HISTORY_BYTE_BUDGET: usize = 16 * 1024 * 1024;

/// The board, generation, and rule an edit or a run started from.
#[derive(Debug)]
pub(super) struct EditSnapshot {
    cells: CellSnapshot,
    epoch: usize,
    rule: Rule,
}

/// Edit snapshots, oldest first, kept within both the edit-count cap and the
/// byte budget across all of them. The newest is always kept, however large.
#[derive(Debug, Default)]
pub(super) struct EditHistory {
    edits: VecDeque<EditSnapshot>,
    /// What `edits` hold, by `CellSnapshot::bytes`.
    bytes: usize,
}

impl EditHistory {
    fn push(&mut self, snapshot: EditSnapshot) {
        let bytes = snapshot.cells.bytes();
        while !self.edits.is_empty()
            && (self.edits.len() >= EDIT_HISTORY_MAX
                || self.bytes + bytes > EDIT_HISTORY_BYTE_BUDGET)
        {
            self.pop_front();
        }
        self.bytes += bytes;
        self.edits.push_back(snapshot);
    }

    fn pop_front(&mut self) {
        if let Some(oldest) = self.edits.pop_front() {
            self.bytes -= oldest.cells.bytes();
        }
    }

    fn pop(&mut self) -> Option<EditSnapshot> {
        let newest = self.edits.pop_back()?;
        self.bytes -= newest.cells.bytes();
        Some(newest)
    }

    fn retain(&mut self, keep: impl FnMut(&EditSnapshot) -> bool) {
        self.edits.retain(keep);
        self.bytes = self.edits.iter().map(|edit| edit.cells.bytes()).sum();
    }

    fn clear(&mut self) {
        self.edits.clear();
        self.bytes = 0;
    }

    fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.edits.len()
    }
}

impl App {
    pub fn begin_stroke(&mut self) {
        self.stroke_active = true;
    }

    pub fn end_stroke(&mut self) {
        self.stroke_active = false;
        self.stroke_saved = false;
    }

    /// The board no longer follows from the run so far.
    pub(super) fn forget_run(&mut self) {
        self.run_start = None;
        self.settle.reset();
        self.population_history.clear();
        self.last_step = None;
    }

    pub(super) fn save_edit(&mut self) {
        // Ahead of the stroke check: a stroke can span generations.
        self.forget_run();
        if self.stroke_active && self.stroke_saved {
            return;
        }
        let snapshot = self.current_snapshot();
        self.undo_history.push(snapshot);
        // A fresh edit invalidates whatever had been undone.
        self.redo_history.clear();
        self.stroke_saved = self.stroke_active;
    }

    /// Keeps only the edits made on the unbounded board, for when the size of
    /// a finite board to return to changes while on it.
    pub(super) fn forget_finite_edits(&mut self) {
        self.undo_history.retain(|edit| edit.cells.is_unbounded());
        self.redo_history.retain(|edit| edit.cells.is_unbounded());
    }

    pub(super) fn current_snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            cells: self.board.snapshot_cells(),
            epoch: self.epoch,
            rule: self.board.rule(),
        }
    }

    fn restore(&mut self, snapshot: EditSnapshot) {
        // Cells first: a board they bring back starts out on the default rule.
        self.board.restore_cells(snapshot.cells);
        self.board.set_rule(snapshot.rule);
        self.epoch = snapshot.epoch;
        self.running = false;
        self.forget_run();
        self.needs_redraw = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_history.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_history.is_empty()
    }

    pub fn undo(&mut self) -> bool {
        self.end_stroke();
        let Some(snapshot) = self.undo_history.pop() else {
            return false;
        };
        let current = self.current_snapshot();
        self.redo_history.push(current);
        self.restore(snapshot);
        true
    }

    pub fn redo(&mut self) -> bool {
        self.end_stroke();
        let Some(snapshot) = self.redo_history.pop() else {
            return false;
        };
        let current = self.current_snapshot();
        self.undo_history.push(current);
        self.restore(snapshot);
        true
    }

    pub fn can_reset_to_start(&self) -> bool {
        self.run_start.is_some()
    }

    /// Returns to the board before the run, as an undoable edit, and pauses.
    pub fn reset_to_start(&mut self) -> bool {
        self.end_stroke();
        let Some(start) = self.run_start.take() else {
            return false;
        };
        self.save_edit();
        self.restore(start);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{board, universe::Universe};

    #[test]
    fn undo_history_is_bounded_and_survives_a_resize() {
        let mut app = App::new(8, 8);
        for i in 0..40 {
            app.populate_board(i % 8, i / 8);
        }
        assert_eq!(app.undo_history.len(), 32);
        app.resize_board(9, 8);
        assert_eq!(app.undo_history.len(), 32);
        assert!(app.undo());
    }

    #[test]
    fn undo_history_stays_within_its_byte_budget_across_snapshots_of_any_size() {
        let snapshot = |cols, rows| EditSnapshot {
            cells: Universe::Finite(board::Board::new(cols, rows)).snapshot_cells(),
            epoch: 0,
            rule: Rule::LIFE,
        };
        let mut history = EditHistory::default();
        // 12 MiB, then 6 MiB: both together are over the 16 MiB budget, though
        // either alone is not.
        history.push(snapshot(8192, 12288));
        assert_eq!(history.bytes, 12 << 20);
        history.push(snapshot(8192, 6144));
        assert_eq!(history.len(), 1, "the older snapshot made room");
        assert_eq!(history.bytes, 6 << 20);
        assert!(history.pop().is_some());
        assert_eq!(history.bytes, 0);
    }

    #[test]
    fn the_largest_board_keeps_its_whole_undo_history() {
        // The largest size Settings allows.
        let mut app = App::new(2048, 2048);
        let edits = EDIT_HISTORY_MAX as i64 + 2;
        for col in 0..edits {
            app.populate_board(col, 0);
        }
        assert_eq!(app.undo_history.len(), EDIT_HISTORY_MAX);

        assert!(app.undo());
        assert!(!app.board.is_alive(edits - 1, 0));
        assert!(app.board.is_alive(edits - 2, 0));
    }
}
