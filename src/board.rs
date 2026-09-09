const CELL_ALIVE: u8 = 1;
const FRONTIER_DENSE_THRESHOLD_PERCENT_DEFAULT: usize = 60;

fn frontier_dense_threshold_percent() -> usize {
    static THRESHOLD: std::sync::LazyLock<usize> = std::sync::LazyLock::new(|| {
        std::env::var("GOL_FRONTIER_DENSE_THRESHOLD_PERCENT")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .map(|value| value.min(100))
            .unwrap_or(FRONTIER_DENSE_THRESHOLD_PERCENT_DEFAULT)
    });
    *THRESHOLD
}

pub fn frontier_dense_threshold_percent_for_diagnostics() -> usize {
    frontier_dense_threshold_percent()
}

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

#[derive(Debug, Clone)]
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
    /// cells changed by the latest state transition
    frontier_cells: Vec<usize>,
    /// dedupe flags for frontier cells
    frontier_flags: Vec<u8>,
    /// force dense stepping on next generation
    frontier_full: bool,
    /// dedupe flags for frontier candidate expansion
    candidate_flags: Vec<u8>,
    /// board width in cells
    cols: usize,
    /// board height in cells
    rows: usize,
    /// board boundary behavior
    boundary_mode: BoundaryMode,
}

impl Board {
    pub(crate) fn snapshot_cells(&self) -> Vec<u8> {
        self.current.clone()
    }

    pub(crate) fn restore_cells(&mut self, cells: Vec<u8>) {
        assert_eq!(cells.len(), self.current.len());
        self.current = cells;
        self.mark_dirty_full();
        self.mark_frontier_full();
    }

    pub fn new(cols: usize, rows: usize) -> Self {
        let cells = cols.saturating_mul(rows);
        Self {
            current: vec![0; cells],
            future: vec![0; cells],
            dirty_cells: Vec::new(),
            dirty_flags: vec![0; cells],
            dirty_full: true,
            frontier_cells: Vec::new(),
            frontier_flags: vec![0; cells],
            frontier_full: false,
            candidate_flags: vec![0; cells],
            cols,
            rows,
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
        self.frontier_flags = vec![0; new_len];
        self.candidate_flags = vec![0; new_len];
        self.mark_dirty_full();
        self.mark_frontier_full();
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
            self.mark_frontier_cell(idx);
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
            self.mark_frontier_cell(idx);
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
        self.mark_frontier_full();
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
            self.mark_frontier_full();
        }
        changed
    }

    pub fn step(&mut self) -> bool {
        if self.cols == 0 || self.rows == 0 {
            return false;
        }

        if self.frontier_full {
            return self.step_dense();
        }

        if self.frontier_cells.is_empty() {
            return false;
        }

        self.step_frontier()
    }

    fn step_dense(&mut self) -> bool {
        let mut changed = false;
        let mut next_frontier = Vec::new();

        for row in 0..self.rows {
            for col in 0..self.cols {
                let idx = self.idx(col, row);
                let next_state = self.next_state(col, row);
                changed |= next_state != self.current[idx];
                if next_state != self.current[idx] {
                    self.mark_dirty_cell(idx);
                    next_frontier.push(idx);
                }
                self.future[idx] = next_state;
            }
        }

        std::mem::swap(&mut self.current, &mut self.future);
        self.replace_frontier(next_frontier);
        changed
    }

    fn step_frontier(&mut self) -> bool {
        let mut candidates = Vec::with_capacity(self.frontier_cells.len().saturating_mul(9));
        self.collect_frontier_candidates(&mut candidates);

        let dense_threshold = self
            .current
            .len()
            .saturating_mul(frontier_dense_threshold_percent())
            / 100;
        if candidates.len() > dense_threshold.max(1) {
            for idx in candidates {
                self.candidate_flags[idx] = 0;
            }
            return self.step_dense();
        }

        self.future.copy_from_slice(&self.current);

        let mut changed = false;
        let mut next_frontier = Vec::with_capacity(candidates.len());
        for idx in candidates {
            self.candidate_flags[idx] = 0;

            let col = idx % self.cols;
            let row = idx / self.cols;
            let next_state = self.next_state(col, row);
            changed |= next_state != self.current[idx];
            if next_state != self.current[idx] {
                self.mark_dirty_cell(idx);
                next_frontier.push(idx);
            }
            self.future[idx] = next_state;
        }

        std::mem::swap(&mut self.current, &mut self.future);
        self.replace_frontier(next_frontier);
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
        self.mark_frontier_full();
    }

    pub fn set_boundary_mode(&mut self, mode: BoundaryMode) {
        if self.boundary_mode == mode {
            return;
        }
        self.boundary_mode = mode;
        self.mark_frontier_full();
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

    fn next_state(&self, col: usize, row: usize) -> u8 {
        let alive_neighbors = match self.boundary_mode {
            BoundaryMode::Wrap => self.count_neighbors_wrap(col, row),
            BoundaryMode::Fixed => self.count_neighbors_fixed(col, row),
        };
        let currently_alive = self.current[self.idx(col, row)] == CELL_ALIVE;
        u8::from(alive_neighbors == 3 || (alive_neighbors == 2 && currently_alive))
    }

    fn count_neighbors_wrap(&self, col: usize, row: usize) -> u8 {
        let cols = self.cols as isize;
        let rows = self.rows as isize;
        let col = col as isize;
        let row = row as isize;
        let mut alive = 0;

        for dy in -1..=1 {
            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }

                let ncol = (col + dx).rem_euclid(cols) as usize;
                let nrow = (row + dy).rem_euclid(rows) as usize;
                alive += u8::from(self.current[self.idx(ncol, nrow)] == CELL_ALIVE);
            }
        }

        alive
    }

    fn count_neighbors_fixed(&self, col: usize, row: usize) -> u8 {
        let cols = self.cols as isize;
        let rows = self.rows as isize;
        let col = col as isize;
        let row = row as isize;
        let mut alive = 0;

        for dy in -1..=1 {
            let nrow = row + dy;
            if nrow < 0 || nrow >= rows {
                continue;
            }

            for dx in -1..=1 {
                if dx == 0 && dy == 0 {
                    continue;
                }

                let ncol = col + dx;
                if ncol < 0 || ncol >= cols {
                    continue;
                }

                alive +=
                    u8::from(self.current[self.idx(ncol as usize, nrow as usize)] == CELL_ALIVE);
            }
        }

        alive
    }

    fn collect_frontier_candidates(&mut self, out: &mut Vec<usize>) {
        let frontier_cells = &self.frontier_cells;
        let candidate_flags = &mut self.candidate_flags;
        let cols_usize = self.cols;
        let cols = self.cols as isize;
        let rows = self.rows as isize;

        match self.boundary_mode {
            BoundaryMode::Wrap => {
                for &idx in frontier_cells {
                    let col = (idx % self.cols) as isize;
                    let row = (idx / self.cols) as isize;

                    for dy in -1..=1 {
                        for dx in -1..=1 {
                            let ncol = (col + dx).rem_euclid(cols) as usize;
                            let nrow = (row + dy).rem_euclid(rows) as usize;
                            let neighbor_idx = nrow * cols_usize + ncol;
                            if candidate_flags[neighbor_idx] == 0 {
                                candidate_flags[neighbor_idx] = 1;
                                out.push(neighbor_idx);
                            }
                        }
                    }
                }
            }
            BoundaryMode::Fixed => {
                for &idx in frontier_cells {
                    let col = (idx % self.cols) as isize;
                    let row = (idx / self.cols) as isize;

                    for dy in -1..=1 {
                        let nrow = row + dy;
                        if nrow < 0 || nrow >= rows {
                            continue;
                        }

                        for dx in -1..=1 {
                            let ncol = col + dx;
                            if ncol < 0 || ncol >= cols {
                                continue;
                            }

                            let neighbor_idx = nrow as usize * cols_usize + ncol as usize;
                            if candidate_flags[neighbor_idx] == 0 {
                                candidate_flags[neighbor_idx] = 1;
                                out.push(neighbor_idx);
                            }
                        }
                    }
                }
            }
        }
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

    fn mark_frontier_cell(&mut self, idx: usize) {
        if self.frontier_full || idx >= self.frontier_flags.len() || self.frontier_flags[idx] != 0 {
            return;
        }
        self.frontier_flags[idx] = 1;
        self.frontier_cells.push(idx);
    }

    fn mark_frontier_full(&mut self) {
        self.frontier_full = true;
        self.frontier_cells.clear();
        self.frontier_flags.fill(0);
    }

    fn replace_frontier(&mut self, mut next_frontier: Vec<usize>) {
        if self.frontier_full {
            self.frontier_full = false;
        } else {
            for &idx in &self.frontier_cells {
                self.frontier_flags[idx] = 0;
            }
        }

        std::mem::swap(&mut self.frontier_cells, &mut next_frontier);
        for idx in next_frontier {
            self.frontier_flags[idx] = 0;
        }
        for &idx in &self.frontier_cells {
            self.frontier_flags[idx] = 1;
        }
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

        let DirtyCells::Cells(mut indices) = board.take_dirty_cells() else {
            panic!("expected incremental dirty cells");
        };
        indices.sort_unstable();
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

    #[test]
    fn render_sync_does_not_clear_step_frontier() {
        let mut board = Board::new(5, 5);
        let _ = board.take_dirty_cells();
        board.populate(1, 2);
        board.populate(2, 2);
        board.populate(3, 2);

        let _ = board.take_dirty_cells();
        board.step();

        assert_eq!(live_cells(&board), vec![(2, 1), (2, 2), (2, 3)]);
    }

    #[test]
    fn boundary_mode_toggle_marks_frontier_full() {
        let mut board = Board::new(5, 5);
        assert!(!board.frontier_full);

        board.toggle_boundary_mode();

        assert!(board.frontier_full);
    }
}
