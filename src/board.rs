const CELL_ALIVE: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryMode {
    Wrap,
    Fixed,
}

#[derive(Debug, PartialEq, Eq)]
pub enum DirtyCells {
    None,
    Full,
    Cells(Vec<usize>),
}

#[derive(Debug)]
pub struct Board {
    /// current state field
    current: Vec<u8>,
    /// future state field
    future: Vec<u8>,
    /// changed cells since last render sync
    dirty_cells: Vec<usize>,
    /// dedupe flags for dirty cells
    dirty_flags: Vec<u8>,
    /// request a full redraw on next render sync
    dirty_full: bool,
    /// board width in cells
    cols: usize,
    /// board height in cells
    rows: usize,
    /// whether render cache needs rebuild
    render_dirty: bool,
    /// board boundary behavior
    boundary_mode: BoundaryMode,
}

impl Board {
    pub fn new(cols: usize, rows: usize) -> Self {
        let cells = cols.saturating_mul(rows);
        Self {
            current: vec![0; cells],
            future: vec![0; cells],
            dirty_cells: Vec::new(),
            dirty_flags: vec![0; cells],
            dirty_full: true,
            cols,
            rows,
            render_dirty: true,
            boundary_mode: BoundaryMode::Wrap,
        }
    }

    pub fn resize(&mut self, new_cols: usize, new_rows: usize) -> bool {
        if self.cols == new_cols && self.rows == new_rows {
            return false;
        }

        let old_cols = self.cols;
        let old_rows = self.rows;
        let mut new_current = vec![0; new_cols.saturating_mul(new_rows)];
        let copy_cols = old_cols.min(new_cols);
        let copy_rows = old_rows.min(new_rows);

        for row in 0..copy_rows {
            for col in 0..copy_cols {
                let src = row * old_cols + col;
                let dst = row * new_cols + col;
                new_current[dst] = self.current[src];
            }
        }

        self.cols = new_cols;
        self.rows = new_rows;
        self.current = new_current;
        let new_len = new_cols.saturating_mul(new_rows);
        self.future = vec![0; new_len];
        self.dirty_flags = vec![0; new_len];
        self.mark_dirty_full();
        self.render_dirty = true;
        true
    }

    pub fn populate(&mut self, col: usize, row: usize) -> bool {
        if col < self.cols && row < self.rows {
            let idx = self.idx(col, row);
            if self.current[idx] == CELL_ALIVE {
                return false;
            }
            self.current[idx] = CELL_ALIVE;
            self.mark_dirty_cell(idx);
            self.render_dirty = true;
            return true;
        }
        false
    }

    pub fn depopulate(&mut self, col: usize, row: usize) -> bool {
        if col < self.cols && row < self.rows {
            let idx = self.idx(col, row);
            if self.current[idx] != CELL_ALIVE {
                return false;
            }
            self.current[idx] = 0;
            self.mark_dirty_cell(idx);
            self.render_dirty = true;
            return true;
        }
        false
    }

    pub fn populate_random(&mut self) {
        if self.cols == 0 || self.rows == 0 {
            return;
        }

        let size = self.current.len() / 4;
        for _ in 0..size {
            let x = fastrand::usize(0..self.cols);
            let y = fastrand::usize(0..self.rows);
            let idx = self.idx(x, y);
            self.current[idx] = CELL_ALIVE;
        }
        self.mark_dirty_full();
        self.render_dirty = true;
    }

    pub fn clear(&mut self) -> bool {
        let mut changed = false;
        for cell in &mut self.current {
            if *cell == CELL_ALIVE {
                *cell = 0;
                changed = true;
            }
        }

        if changed {
            self.mark_dirty_full();
            self.render_dirty = true;
        }
        changed
    }

    fn alive(&self, x: i32, y: i32) -> bool {
        let w = self.cols as i32;
        let h = self.rows as i32;
        if w == 0 || h == 0 {
            return false;
        }

        match self.boundary_mode {
            BoundaryMode::Wrap => {
                let i = x.rem_euclid(w) as usize;
                let j = y.rem_euclid(h) as usize;
                self.current[self.idx(i, j)] == CELL_ALIVE
            }
            BoundaryMode::Fixed => {
                if x < 0 || y < 0 || x >= w || y >= h {
                    return false;
                }
                self.current[self.idx(x as usize, y as usize)] == CELL_ALIVE
            }
        }
    }

    fn next(&self, x: i32, y: i32) -> bool {
        let mut alive = 0;
        for j in y - 1..y + 2 {
            for i in x - 1..x + 2 {
                if (j != y || i != x) && self.alive(i, j) {
                    alive += 1;
                }
            }
        }
        alive == 3 || (alive == 2 && self.alive(x, y))
    }

    pub fn step(&mut self) -> bool {
        if self.cols == 0 || self.rows == 0 {
            return false;
        }

        let mut changed = false;
        for row in 0..self.rows {
            for col in 0..self.cols {
                let idx = self.idx(col, row);
                let next_state = u8::from(self.next(col as i32, row as i32));
                changed |= next_state != self.current[idx];
                if next_state != self.current[idx] {
                    self.mark_dirty_cell(idx);
                }
                self.future[idx] = next_state;
            }
        }
        std::mem::swap(&mut self.current, &mut self.future);
        self.render_dirty = true;
        changed
    }

    pub fn size(&self) -> (usize, usize) {
        (self.cols, self.rows)
    }

    pub fn toggle_boundary_mode(&mut self) {
        self.boundary_mode = match self.boundary_mode {
            BoundaryMode::Wrap => BoundaryMode::Fixed,
            BoundaryMode::Fixed => BoundaryMode::Wrap,
        };
        self.render_dirty = true;
    }

    pub fn set_boundary_mode(&mut self, mode: BoundaryMode) {
        self.boundary_mode = mode;
        self.render_dirty = true;
    }

    pub fn boundary_mode(&self) -> BoundaryMode {
        self.boundary_mode
    }

    pub fn boundary_mode_label(&self) -> &'static str {
        match self.boundary_mode {
            BoundaryMode::Wrap => "Wrap",
            BoundaryMode::Fixed => "Fixed",
        }
    }

    pub fn is_alive(&self, col: usize, row: usize) -> bool {
        if col < self.cols && row < self.rows {
            return self.current[self.idx(col, row)] == CELL_ALIVE;
        }
        false
    }

    pub fn live_cells_count(&self) -> usize {
        self.current
            .iter()
            .filter(|&&cell| cell == CELL_ALIVE)
            .count()
    }

    pub fn take_dirty_cells(&mut self) -> DirtyCells {
        if self.dirty_full {
            self.dirty_full = false;
            self.dirty_cells.clear();
            self.dirty_flags.fill(0);
            return DirtyCells::Full;
        }

        if self.dirty_cells.is_empty() {
            return DirtyCells::None;
        }

        let mut cells = Vec::new();
        std::mem::swap(&mut self.dirty_cells, &mut cells);
        for &idx in &cells {
            self.dirty_flags[idx] = 0;
        }
        DirtyCells::Cells(cells)
    }

    fn idx(&self, col: usize, row: usize) -> usize {
        row * self.cols + col
    }

    fn mark_dirty_cell(&mut self, idx: usize) {
        if self.dirty_full || idx >= self.dirty_flags.len() || self.dirty_flags[idx] != 0 {
            return;
        }
        self.dirty_flags[idx] = 1;
        self.dirty_cells.push(idx);
    }

    fn mark_dirty_full(&mut self) {
        self.dirty_full = true;
        self.dirty_cells.clear();
        self.dirty_flags.fill(0);
    }
}

#[cfg(test)]
mod tests {
    use super::{Board, BoundaryMode, DirtyCells};

    fn live_cells(board: &Board) -> Vec<(usize, usize)> {
        let mut cells = Vec::new();
        let (cols, rows) = board.size();
        for x in 0..cols {
            for y in 0..rows {
                if board.is_alive(x, y) {
                    cells.push((x, y));
                }
            }
        }
        cells.sort_unstable();
        cells
    }

    #[test]
    fn still_life_block_stays_stable() {
        let mut board = Board::new(4, 4);
        board.populate(1, 1);
        board.populate(1, 2);
        board.populate(2, 1);
        board.populate(2, 2);

        let before = live_cells(&board);
        board.step();
        let after = live_cells(&board);

        assert_eq!(before, after);
    }

    #[test]
    fn blinker_oscillates_every_step() {
        let mut board = Board::new(5, 5);
        board.populate(1, 2);
        board.populate(2, 2);
        board.populate(3, 2);

        board.step();
        assert_eq!(live_cells(&board), vec![(2, 1), (2, 2), (2, 3)]);

        board.step();
        assert_eq!(live_cells(&board), vec![(1, 2), (2, 2), (3, 2)]);
    }

    #[test]
    fn wraparound_neighbors_are_counted() {
        let mut board = Board::new(5, 5);
        board.set_boundary_mode(BoundaryMode::Wrap);
        board.populate(4, 4);
        board.populate(4, 0);
        board.populate(0, 4);

        board.step();

        assert!(board.is_alive(0, 0));
    }

    #[test]
    fn fixed_boundary_does_not_wrap_neighbors() {
        let mut board = Board::new(5, 5);
        board.set_boundary_mode(BoundaryMode::Fixed);
        board.populate(4, 4);
        board.populate(4, 0);
        board.populate(0, 4);

        board.step();

        assert!(!board.is_alive(0, 0));
    }

    #[test]
    fn resize_preserves_overlapping_cells_only() {
        let mut board = Board::new(4, 4);
        board.populate(0, 0);
        board.populate(3, 3);

        board.resize(2, 3);

        assert_eq!(board.size(), (2, 3));
        assert!(board.is_alive(0, 0));
        assert_eq!(live_cells(&board), vec![(0, 0)]);
    }

    #[test]
    fn clear_removes_all_live_cells() {
        let mut board = Board::new(3, 3);
        board.populate(0, 0);
        board.populate(1, 1);

        let changed = board.clear();

        assert!(changed);
        assert!(live_cells(&board).is_empty());
    }

    #[test]
    fn glider_moves_diagonally_after_four_steps() {
        let mut board = Board::new(10, 10);
        board.populate(1, 0);
        board.populate(2, 1);
        board.populate(0, 2);
        board.populate(1, 2);
        board.populate(2, 2);

        for _ in 0..4 {
            board.step();
        }

        assert_eq!(
            live_cells(&board),
            vec![(1, 3), (2, 1), (2, 3), (3, 2), (3, 3)]
        );
    }

    #[test]
    fn populate_marks_single_dirty_cell() {
        let mut board = Board::new(4, 4);
        assert_eq!(board.take_dirty_cells(), DirtyCells::Full);

        board.populate(2, 1);
        assert_eq!(board.take_dirty_cells(), DirtyCells::Cells(vec![6]));
        assert_eq!(board.take_dirty_cells(), DirtyCells::None);
    }

    #[test]
    fn step_marks_only_changed_cells() {
        let mut board = Board::new(5, 5);
        let _ = board.take_dirty_cells();
        board.populate(1, 2);
        board.populate(2, 2);
        board.populate(3, 2);
        let _ = board.take_dirty_cells();

        board.step();

        let DirtyCells::Cells(indices) = board.take_dirty_cells() else {
            panic!("expected incremental dirty cells");
        };
        assert_eq!(indices, vec![7, 11, 13, 17]);
    }

    #[test]
    fn clear_requests_full_redraw() {
        let mut board = Board::new(4, 4);
        let _ = board.take_dirty_cells();
        board.populate(1, 1);
        let _ = board.take_dirty_cells();

        board.clear();

        assert_eq!(board.take_dirty_cells(), DirtyCells::Full);
    }
}
