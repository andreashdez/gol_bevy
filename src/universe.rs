//! The board the app runs on: a finite [`Board`] with edges, or the unbounded
//! [`SparseBoard`]. Cells have signed coordinates here, so the app has one way to
//! address a cell, and a finite board takes only those it has.
use crate::{
    board::{Board, BoundaryMode, Rule},
    sparse_board::{SparseBoard, SparseSnapshot, UnboundedRuleError},
};

#[derive(Debug, Clone)]
pub enum Universe {
    Finite(Board),
    Unbounded(SparseBoard),
}

/// The cells of an earlier state, for restoring to the kind of board that took it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CellSnapshot {
    Finite(Vec<u8>),
    Unbounded(SparseSnapshot),
}

impl CellSnapshot {
    /// Memory this holds, for keeping undo history within its budget.
    pub fn bytes(&self) -> usize {
        match self {
            Self::Finite(cells) => cells.len(),
            Self::Unbounded(snapshot) => snapshot.bytes(),
        }
    }
}

impl Universe {
    /// The board, for what only makes sense with edges or a fixed size.
    pub fn finite(&self) -> Option<&Board> {
        match self {
            Self::Finite(board) => Some(board),
            Self::Unbounded(_) => None,
        }
    }

    pub fn finite_mut(&mut self) -> Option<&mut Board> {
        match self {
            Self::Finite(board) => Some(board),
            Self::Unbounded(_) => None,
        }
    }

    /// Width and height in cells. `None` on the unbounded board.
    pub fn size(&self) -> Option<(usize, usize)> {
        self.finite().map(Board::size)
    }

    /// Whether the board has this cell, dead or alive.
    pub fn contains(&self, x: i64, y: i64) -> bool {
        match self {
            Self::Finite(board) => board_cell(board, x, y).is_some(),
            Self::Unbounded(_) => true,
        }
    }

    pub fn is_alive(&self, x: i64, y: i64) -> bool {
        match self {
            Self::Finite(board) => {
                board_cell(board, x, y).is_some_and(|(col, row)| board.is_alive(col, row))
            }
            Self::Unbounded(plane) => plane.is_alive(x, y),
        }
    }

    pub fn populate(&mut self, x: i64, y: i64) -> bool {
        match self {
            Self::Finite(board) => {
                board_cell(board, x, y).is_some_and(|(col, row)| board.populate(col, row))
            }
            Self::Unbounded(plane) => plane.populate(x, y),
        }
    }

    pub fn depopulate(&mut self, x: i64, y: i64) -> bool {
        match self {
            Self::Finite(board) => {
                board_cell(board, x, y).is_some_and(|(col, row)| board.depopulate(col, row))
            }
            Self::Unbounded(plane) => plane.depopulate(x, y),
        }
    }

    pub fn clear(&mut self) -> bool {
        match self {
            Self::Finite(board) => board.clear(),
            Self::Unbounded(plane) => plane.clear(),
        }
    }

    pub fn step(&mut self) -> bool {
        match self {
            Self::Finite(board) => board.step(),
            Self::Unbounded(plane) => plane.step(),
        }
    }

    pub fn rule(&self) -> Rule {
        match self {
            Self::Finite(board) => board.rule(),
            Self::Unbounded(plane) => plane.rule(),
        }
    }

    pub fn allows_rule(&self, rule: Rule) -> bool {
        match self {
            Self::Finite(_) => true,
            Self::Unbounded(_) => !rule.births_from_nothing(),
        }
    }

    /// Returns whether the rule actually changed, which it won't if refused by
    /// [`allows_rule`](Self::allows_rule).
    pub fn set_rule(&mut self, rule: Rule) -> bool {
        match self {
            Self::Finite(board) => board.set_rule(rule),
            Self::Unbounded(plane) => plane.set_rule(rule).unwrap_or(false),
        }
    }

    /// `None` on the unbounded board, which has no edges.
    pub fn boundary_mode(&self) -> Option<BoundaryMode> {
        self.finite().map(Board::boundary_mode)
    }

    /// Changes the edges of a finite board. Giving the unbounded board edges has
    /// to pick a size, which is [`App::set_boundary_mode`]'s to do.
    ///
    /// [`App::set_boundary_mode`]: crate::app::App::set_boundary_mode
    pub fn set_boundary_mode(&mut self, mode: BoundaryMode) {
        if let Self::Finite(board) = self {
            board.set_boundary_mode(mode);
        }
    }

    pub fn boundary_mode_label(&self) -> &'static str {
        match self {
            Self::Finite(board) => board.boundary_mode_label(),
            Self::Unbounded(_) => "Unbounded",
        }
    }

    pub fn live_cells_count(&self) -> usize {
        match self {
            Self::Finite(board) => board.live_cells_count(),
            Self::Unbounded(plane) => plane.live_cells_count(),
        }
    }

    /// Identifies the current generation among others of the same run.
    pub fn fingerprint(&self) -> u64 {
        match self {
            Self::Finite(board) => board.fingerprint(),
            Self::Unbounded(plane) => plane.fingerprint(),
        }
    }

    pub fn snapshot_cells(&self) -> CellSnapshot {
        match self {
            Self::Finite(board) => CellSnapshot::Finite(board.snapshot_cells()),
            Self::Unbounded(plane) => CellSnapshot::Unbounded(plane.snapshot()),
        }
    }

    pub fn restore_cells(&mut self, snapshot: CellSnapshot) {
        match (self, snapshot) {
            (Self::Finite(board), CellSnapshot::Finite(cells)) => board.restore_cells(cells),
            (Self::Unbounded(plane), CellSnapshot::Unbounded(chunks)) => plane.restore(chunks),
            _ => panic!("a snapshot only fits the kind of board that took it"),
        }
    }

    /// The first cell of a `width` × `height` stamp centred on `(x, y)`. A finite
    /// board holds it back from its low edges; the plane has none.
    pub fn stamp_origin(&self, (x, y): (i64, i64), (width, height): (usize, usize)) -> (i64, i64) {
        let origin = (x - width as i64 / 2, y - height as i64 / 2);
        match self {
            Self::Finite(_) => (origin.0.max(0), origin.1.max(0)),
            Self::Unbounded(_) => origin,
        }
    }
}

fn board_cell(board: &Board, x: i64, y: i64) -> Option<(usize, usize)> {
    let (cols, rows) = board.size();
    let (col, row) = (usize::try_from(x).ok()?, usize::try_from(y).ok()?);
    (col < cols && row < rows).then_some((col, row))
}

/// A board's cells and rule on the plane, with the board's centre at the origin,
/// where a canvas centred on the board stays centred on the plane.
pub fn plane_from_board(board: &Board) -> Result<SparseBoard, UnboundedRuleError> {
    let mut plane = SparseBoard::default();
    plane.set_rule(board.rule())?;
    let (cols, rows) = board.size();
    for (index, _) in board.alive_cells().enumerate().filter(|(_, alive)| *alive) {
        plane.populate(
            (index % cols) as i64 - (cols / 2) as i64,
            (index / cols) as i64 - (rows / 2) as i64,
        );
    }
    Ok(plane)
}

/// The part of the plane a `cols` × `rows` board centred on the origin can hold.
pub fn board_from_plane(
    plane: &SparseBoard,
    cols: usize,
    rows: usize,
    mode: BoundaryMode,
) -> Board {
    let mut board = Board::new(cols, rows);
    board.set_rule(plane.rule());
    board.set_boundary_mode(mode);
    for (x, y) in plane.live_cells() {
        let cell = (
            x.saturating_add((cols / 2) as i64),
            y.saturating_add((rows / 2) as i64),
        );
        if let Some((col, row)) = board_cell(&board, cell.0, cell.1) {
            board.populate(col, row);
        }
    }
    board
}

#[cfg(test)]
mod tests {
    use super::{CellSnapshot, Universe, board_from_plane, plane_from_board};
    use crate::{
        board::{Board, BoundaryMode, Rule},
        sparse_board::{SparseBoard, UnboundedRuleError},
    };

    fn finite() -> Universe {
        Universe::Finite(Board::new(16, 8))
    }

    fn unbounded() -> Universe {
        Universe::Unbounded(SparseBoard::default())
    }

    #[test]
    fn a_finite_board_turns_away_cells_it_does_not_have() {
        let mut board = finite();
        for (x, y) in [(-1, 0), (0, -1), (16, 0), (0, 8), (i64::MAX, i64::MIN)] {
            assert!(!board.contains(x, y));
            assert!(!board.populate(x, y));
            assert!(!board.is_alive(x, y));
            assert!(!board.depopulate(x, y));
        }
        assert!(board.contains(15, 7));
        assert!(board.populate(15, 7));
        assert!(board.is_alive(15, 7));
        assert_eq!(board.live_cells_count(), 1);
    }

    #[test]
    fn the_unbounded_board_has_every_cell() {
        let mut board = unbounded();
        assert!(board.contains(-1, -1));
        assert!(board.contains(1 << 40, -(1 << 40)));
        assert!(board.populate(-1, 1 << 40));
        assert!(board.is_alive(-1, 1 << 40));
        assert!(board.depopulate(-1, 1 << 40));
        assert_eq!(board.size(), None);
        assert_eq!(board.boundary_mode(), None);
    }

    #[test]
    fn only_the_unbounded_board_refuses_b0() {
        let b0 = Rule::parse("B03/S23").unwrap();
        let mut board = finite();
        assert!(board.allows_rule(b0));
        assert!(board.set_rule(b0));

        let mut board = unbounded();
        let rule = board.rule();
        assert!(!board.allows_rule(b0));
        assert!(!board.set_rule(b0));
        assert_eq!(board.rule(), rule);
    }

    #[test]
    fn snapshots_restore_both_kinds_of_board() {
        for mut board in [finite(), unbounded()] {
            board.populate(3, 3);
            let snapshot = board.snapshot_cells();
            board.populate(4, 4);
            assert_ne!(board.snapshot_cells(), snapshot);
            board.restore_cells(snapshot.clone());
            assert_eq!(board.snapshot_cells(), snapshot);
            assert!(!board.is_alive(4, 4));
        }
    }

    #[test]
    #[should_panic(expected = "kind of board")]
    fn a_snapshot_cannot_cross_kinds_of_board() {
        finite().restore_cells(CellSnapshot::Unbounded(SparseBoard::default().snapshot()));
    }

    #[test]
    fn snapshots_measure_the_memory_they_hold() {
        let mut board = unbounded();
        assert_eq!(board.snapshot_cells().bytes(), 0);
        // Two chunks, whose cells take 512 bytes each.
        board.populate(0, 0);
        board.populate(-1, 0);
        assert_eq!(board.snapshot_cells().bytes(), 1024);
        assert_eq!(finite().snapshot_cells().bytes(), 16 * 8);
    }

    #[test]
    fn stamps_start_past_a_finite_edge_only_on_the_plane() {
        assert_eq!(finite().stamp_origin((1, 2), (6, 6)), (0, 0));
        assert_eq!(finite().stamp_origin((10, 5), (6, 4)), (7, 3));
        assert_eq!(unbounded().stamp_origin((1, 2), (6, 6)), (-2, -1));
    }

    #[test]
    fn the_board_centre_becomes_the_origin_and_back() {
        let mut board = Board::new(16, 8);
        let rule = Rule::parse("B36/S23").unwrap();
        board.set_rule(rule);
        for (col, row) in [(8, 4), (0, 0), (15, 7)] {
            board.populate(col, row);
        }

        let plane = plane_from_board(&board).unwrap();
        assert_eq!(plane.rule(), rule);
        assert_eq!(plane.live_cells_count(), 3);
        assert!(plane.is_alive(0, 0));
        assert!(plane.is_alive(-8, -4));
        assert!(plane.is_alive(7, 3));

        let back = board_from_plane(&plane, 16, 8, BoundaryMode::Fixed);
        assert_eq!(back.snapshot_cells(), board.snapshot_cells());
        assert_eq!(back.rule(), rule);
        assert_eq!(back.boundary_mode(), BoundaryMode::Fixed);
    }

    #[test]
    fn a_b0_board_cannot_move_onto_the_plane() {
        let mut board = Board::new(8, 8);
        board.set_rule(Rule::parse("B0/S").unwrap());
        assert_eq!(plane_from_board(&board).err(), Some(UnboundedRuleError));
    }

    #[test]
    fn a_board_keeps_only_the_cells_that_fit() {
        let mut plane = SparseBoard::default();
        for (x, y) in [
            (0, 0),
            (-4, -4),
            (3, 3),
            (4, 0),
            (0, 4),
            (-5, 0),
            (i64::MAX, 0),
            (0, i64::MAX),
        ] {
            plane.populate(x, y);
        }
        let board = board_from_plane(&plane, 8, 8, BoundaryMode::Wrap);
        assert_eq!(board.live_cells_count(), 3);
        assert!(board.is_alive(4, 4));
        assert!(board.is_alive(0, 0));
        assert!(board.is_alive(7, 7));
    }
}
