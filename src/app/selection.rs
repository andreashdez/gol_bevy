//! The Select tool's rectangle and what can be done with it, and writing the
//! board or the selection out as RLE, as an image, or as an animation.

use super::{App, EditTool};
use crate::{
    cell_gif::{FASTEST_MS_PER_GENERATION, GifArea, GifLength, GifRequest, MAX_GIF_SIDE},
    cell_image::{CellImage, MAX_IMAGE_SIDE},
    patterns::RleWriter,
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

/// Why the board can't be saved as an image or an animation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageExportError {
    Empty,
    /// Wider or taller than this many cells.
    TooLarge(usize),
}

impl fmt::Display for ImageExportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("The board is empty, so there is nothing to draw."),
            Self::TooLarge(limit) => write!(
                f,
                "That's over {limit} cells on a side, too large to draw · select a smaller area."
            ),
        }
    }
}

/// The area a GIF records, for the Export dialog to say.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordedArea {
    Selection(CellRect),
    /// The whole of a finite board.
    Board(CellRect),
    /// The box the live cells of an unbounded board fill over the whole
    /// recording, which starts as this one around them now.
    Run(CellRect),
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

    /// Kills every live cell in the selection, as one edit, and returns how many
    /// there were. The selection stays.
    pub fn erase_selection(&mut self) -> usize {
        let Some(rect) = self.selection() else {
            return 0;
        };
        let mut live = 0usize;
        self.board
            .for_each_live_run_in(rect, |_, _, length| live += length);
        if live == 0 {
            return 0;
        }
        self.end_stroke();
        self.save_edit();
        let killed = self.board.clear_rect(rect);
        self.needs_redraw = true;
        killed
    }

    /// The selection as RLE with the board rule in the header. It keeps the
    /// selection's size, empty margins included, so it lines up where pasted.
    pub fn selection_to_rle(&self) -> Result<BoardExport, SelectionExportError> {
        let rect = self
            .selection()
            .ok_or(SelectionExportError::NothingSelected)?;
        // Written as the runs come, since an unbounded board's selection can
        // hold more cells than would fit in memory one by one.
        let mut writer = RleWriter::default();
        let mut cells = 0usize;
        self.board.for_each_live_run_in(rect, |x, y, length| {
            cells = cells.saturating_add(length);
            if cells <= PLANE_EXPORT_CELLS_MAX {
                writer.cells((x - rect.left) as usize, (y - rect.bottom) as usize, length);
            }
        });
        if cells == 0 {
            return Err(SelectionExportError::Empty);
        }
        if cells > PLANE_EXPORT_CELLS_MAX {
            return Err(SelectionExportError::TooLarge);
        }
        let comment = format!(
            "{} × {} cells selected at generation {}",
            rect.width, rect.height, self.epoch
        );
        Ok(BoardExport {
            rle: writer.finish(rect.width, rect.height, Some(self.board.rule()), &comment),
            cells,
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

    /// The selection as an image, or else the whole of a finite board, or the
    /// live cells of an unbounded one, cropped to their bounding box.
    pub fn board_image(&self) -> Result<CellImage, ImageExportError> {
        let rect = self.image_area(MAX_IMAGE_SIDE)?;
        Ok(CellImage::capture(&self.board, rect))
    }

    /// `length` generations from the current one, to record over the
    /// [`recorded_area`](Self::recorded_area) at the playback speed, from a
    /// copy of the board.
    pub fn board_gif(&self, length: GifLength) -> Result<GifRequest, ImageExportError> {
        let area = match self.recorded_area()? {
            RecordedArea::Selection(rect) | RecordedArea::Board(rect) => GifArea::Fixed(rect),
            RecordedArea::Run(_) => GifArea::Run,
        };
        Ok(GifRequest {
            board: self.board.clone(),
            area,
            length,
            ms_per_generation: if self.is_max_speed() {
                FASTEST_MS_PER_GENERATION
            } else {
                self.tick_rate_ms()
            },
        })
    }

    /// What a GIF records: the selection, or else the whole of a finite board,
    /// or else, on an unbounded board, the box its live cells fill over the
    /// whole recording, so a growing pattern stays in the picture.
    pub fn recorded_area(&self) -> Result<RecordedArea, ImageExportError> {
        let rect = self.image_area(MAX_GIF_SIDE)?;
        Ok(
            match (self.selection().is_some(), self.board.finite().is_none()) {
                (true, _) => RecordedArea::Selection(rect),
                (false, false) => RecordedArea::Board(rect),
                (false, true) => RecordedArea::Run(rect),
            },
        )
    }

    /// The selection, or else the whole of a finite board, or the box around
    /// the live cells of an unbounded one, if it is at most `max_side` cells on
    /// a side.
    fn image_area(&self, max_side: usize) -> Result<CellRect, ImageExportError> {
        let rect = match (self.selection(), &self.board) {
            (Some(rect), _) => rect,
            (None, Universe::Finite(board)) => {
                let (width, height) = board.size();
                CellRect {
                    left: 0,
                    bottom: 0,
                    width,
                    height,
                }
            }
            (None, Universe::Unbounded(plane)) => {
                let (min, max) = plane.live_bounds().ok_or(ImageExportError::Empty)?;
                CellRect::spanning(min, max)
            }
        };
        if rect.width > max_side || rect.height > max_side {
            return Err(ImageExportError::TooLarge(max_side));
        }
        Ok(rect)
    }
}
