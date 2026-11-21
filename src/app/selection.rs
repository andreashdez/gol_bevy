//! The Select tool's rectangle and what can be done with it, and writing the
//! board or the selection out as RLE.

use super::{App, EditTool};
use crate::{
    sparse_board::{CellRect, PLANE_EXPORT_CELLS_MAX},
    universe::{Universe, plane_to_rle},
};
use std::fmt;

/// The board written out as RLE, with what it covers for user feedback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardExport {
    pub rle: String,
    pub cells: usize,
    pub width: usize,
    pub height: usize,
}

/// Why the board can't be copied as RLE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardExportError {
    Empty,
    TooLarge,
}

impl fmt::Display for BoardExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Empty => "The board is empty, so there is nothing to export.",
            Self::TooLarge => "The board has too many live cells to export (over 67,108,864).",
        })
    }
}

/// Why the selection can't be copied as RLE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionExportError {
    NothingSelected,
    Empty,
    TooLarge,
}

impl fmt::Display for SelectionExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NothingSelected => "Nothing selected · drag with the Select tool (4)",
            Self::Empty => "No live cells in the selection",
            Self::TooLarge => "Too many live cells in the selection to copy",
        })
    }
}

/// A rectangle picked with the Select tool: the cell a drag started on, the one
/// it reached, and every cell between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Selection {
    anchor: (i64, i64),
    corner: (i64, i64),
    /// Coordinates on the plane mean something else on a finite board.
    unbounded: bool,
}

impl App {
    /// Starts a selection on one cell, where a drag with the Select tool begins.
    pub fn begin_selection(&mut self, col: i64, row: i64) {
        self.selection = Some(Selection {
            anchor: (col, row),
            corner: (col, row),
            unbounded: self.board.finite().is_none(),
        });
    }

    /// Moves the far corner of the selection, keeping the cell it started on.
    pub fn extend_selection(&mut self, col: i64, row: i64) {
        if let Some(selection) = &mut self.selection {
            selection.corner = (col, row);
        }
    }

    /// Returns whether there was a selection to drop.
    pub fn clear_selection(&mut self) -> bool {
        self.selection.take().is_some()
    }

    /// The selected cells, clipped to a finite board. Only the Select tool has a
    /// selection, and none survives a move between the board and the plane.
    pub fn selection(&self) -> Option<CellRect> {
        let selection = self.selection.filter(|selection| {
            self.tool == EditTool::Select && selection.unbounded == self.board.finite().is_none()
        })?;
        let (mut left, mut bottom) = (
            selection.anchor.0.min(selection.corner.0),
            selection.anchor.1.min(selection.corner.1),
        );
        let (mut right, mut top) = (
            selection.anchor.0.max(selection.corner.0),
            selection.anchor.1.max(selection.corner.1),
        );
        // The board may have shrunk since, by a resize or an undo.
        if let Some((cols, rows)) = self.board.size() {
            (left, bottom) = (left.max(0), bottom.max(0));
            (right, top) = (right.min(cols as i64 - 1), top.min(rows as i64 - 1));
        }
        (left <= right && bottom <= top).then(|| CellRect {
            left,
            bottom,
            width: (right - left + 1) as usize,
            height: (top - bottom + 1) as usize,
        })
    }

    /// Switches to the Select tool with the box around every live cell selected.
    /// Returns false, selecting nothing, on an empty board.
    pub fn select_live_cells(&mut self) -> bool {
        let bounds = match &self.board {
            Universe::Finite(board) => {
                let (cols, rows) = board.size();
                let live = (0..rows)
                    .flat_map(|y| (0..cols).map(move |x| (x, y)))
                    .filter(|&(x, y)| board.is_alive(x, y));
                live.fold(None, |bounds: Option<((i64, i64), (i64, i64))>, (x, y)| {
                    let (x, y) = (x as i64, y as i64);
                    Some(bounds.map_or(((x, y), (x, y)), |(min, max)| {
                        ((min.0.min(x), min.1.min(y)), (max.0.max(x), max.1.max(y)))
                    }))
                })
            }
            Universe::Unbounded(plane) => plane.live_bounds(),
        };
        let Some((min, max)) = bounds else {
            return false;
        };
        self.set_tool(EditTool::Select);
        self.begin_selection(min.0, min.1);
        self.extend_selection(max.0, max.1);
        true
    }

    /// The live cells inside `rect`, bottom row first.
    fn live_cells_in(&self, rect: CellRect) -> Vec<(i64, i64)> {
        let (right, top) = (
            rect.left + rect.width as i64,
            rect.bottom + rect.height as i64,
        );
        match &self.board {
            Universe::Finite(board) => (rect.bottom..top)
                .flat_map(|y| (rect.left..right).map(move |x| (x, y)))
                .filter(|&(x, y)| board.is_alive(x as usize, y as usize))
                .collect(),
            // The selection can be far larger than what lives in it.
            Universe::Unbounded(plane) => {
                let mut cells: Vec<_> = plane
                    .live_cells()
                    .filter(|&(x, y)| {
                        (rect.left..right).contains(&x) && (rect.bottom..top).contains(&y)
                    })
                    .collect();
                cells.sort_unstable_by_key(|&(x, y)| (y, x));
                cells
            }
        }
    }

    /// Kills every live cell in the selection, as one edit, and returns how many
    /// there were. The selection stays.
    pub fn erase_selection(&mut self) -> usize {
        let Some(rect) = self.selection() else {
            return 0;
        };
        let doomed = self.live_cells_in(rect);
        if doomed.is_empty() {
            return 0;
        }
        self.end_stroke();
        self.save_edit();
        for &(x, y) in &doomed {
            self.board.depopulate(x, y);
        }
        self.needs_redraw = true;
        doomed.len()
    }

    /// The selection as RLE with the board rule in the header. It keeps the
    /// selection's size, empty margins included, so it lines up where pasted.
    pub fn selection_to_rle(&self) -> Result<BoardExport, SelectionExportError> {
        let rect = self
            .selection()
            .ok_or(SelectionExportError::NothingSelected)?;
        let live = self.live_cells_in(rect);
        if live.is_empty() {
            return Err(SelectionExportError::Empty);
        }
        if live.len() > PLANE_EXPORT_CELLS_MAX {
            return Err(SelectionExportError::TooLarge);
        }
        let cells: Vec<_> = live
            .iter()
            .map(|&(x, y)| ((x - rect.left) as usize, (y - rect.bottom) as usize))
            .collect();
        let comment = format!(
            "{} × {} cells selected at generation {}",
            rect.width, rect.height, self.epoch
        );
        Ok(BoardExport {
            rle: crate::patterns::to_rle(
                &cells,
                rect.width,
                rect.height,
                Some(self.board.rule()),
                &comment,
            ),
            cells: cells.len(),
            width: rect.width,
            height: rect.height,
        })
    }

    /// The live cells cropped to their bounding box, as RLE with the board rule
    /// in the header.
    pub fn board_to_rle(&self) -> Result<BoardExport, BoardExportError> {
        let board = match &self.board {
            Universe::Finite(board) => board,
            Universe::Unbounded(plane) => {
                if plane.live_cells_count() > PLANE_EXPORT_CELLS_MAX {
                    return Err(BoardExportError::TooLarge);
                }
                let comment = format!("Generation {} of an unbounded board", self.epoch);
                let export = plane_to_rle(plane, &comment).ok_or(BoardExportError::Empty)?;
                return Ok(BoardExport {
                    rle: export.rle,
                    cells: plane.live_cells_count(),
                    width: export.width,
                    height: export.height,
                });
            }
        };
        let (cols, rows) = board.size();
        let live: Vec<(usize, usize)> = board.live_cells().collect();
        let bounds = (|| {
            Some((
                live.iter().map(|cell| cell.0).min()?,
                live.iter().map(|cell| cell.1).min()?,
                live.iter().map(|cell| cell.0).max()?,
                live.iter().map(|cell| cell.1).max()?,
            ))
        })();
        let (min_x, min_y, max_x, max_y) = bounds.ok_or(BoardExportError::Empty)?;
        let (width, height) = (max_x - min_x + 1, max_y - min_y + 1);
        let cells: Vec<_> = live.iter().map(|&(x, y)| (x - min_x, y - min_y)).collect();
        let comment = format!("Generation {} of a {cols}x{rows} board", self.epoch);
        Ok(BoardExport {
            rle: crate::patterns::to_rle(&cells, width, height, Some(self.board.rule()), &comment),
            cells: cells.len(),
            width,
            height,
        })
    }
}
