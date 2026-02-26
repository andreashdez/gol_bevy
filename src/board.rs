const CENTER_FULL: &str = "██";
const CENTER_EMPTY: &str = "  ";

#[derive(Debug)]
pub struct Board {
    /// current state field
    current: Vec<Vec<bool>>,
    /// future state field
    future: Vec<Vec<bool>>,
    /// cached board render output
    render_cache: String,
    /// whether render cache needs rebuild
    render_dirty: bool,
}

impl Board {
    pub fn new(cols: usize, rows: usize) -> Self {
        Self {
            current: vec![vec![false; rows]; cols],
            future: vec![vec![false; rows]; cols],
            render_cache: String::new(),
            render_dirty: true,
        }
    }

    pub fn resize(&mut self, new_cols: usize, new_rows: usize) -> bool {
        if self.cols() == new_cols && self.rows() == new_rows {
            return false;
        }

        let mut new_current = vec![vec![false; new_rows]; new_cols];
        let copy_cols = self.cols().min(new_cols);
        let copy_rows = self.rows().min(new_rows);
        for (x, new_col) in new_current.iter_mut().enumerate().take(copy_cols) {
            new_col[..copy_rows].copy_from_slice(&self.current[x][..copy_rows]);
        }
        self.current = new_current;
        self.future = vec![vec![false; new_rows]; new_cols];
        self.render_dirty = true;
        true
    }

    pub fn populate(&mut self, col: usize, row: usize) -> bool {
        if col < self.cols() && row < self.rows() {
            if self.current[col][row] {
                return false;
            }
            self.current[col][row] = true;
            self.render_dirty = true;
            return true;
        }
        false
    }

    pub fn depopulate(&mut self, col: usize, row: usize) -> bool {
        if col < self.cols() && row < self.rows() {
            if !self.current[col][row] {
                return false;
            }
            self.current[col][row] = false;
            self.render_dirty = true;
            return true;
        }
        false
    }

    pub fn populate_random(&mut self) {
        let cols = self.cols();
        let rows = self.rows();
        if cols == 0 || rows == 0 {
            return;
        }
        let size = cols * rows / 4;
        for _ in 0..size {
            let x = fastrand::usize(0..cols);
            let y = fastrand::usize(0..rows);
            self.current[x][y] = true;
        }
        self.render_dirty = true;
    }

    pub fn clear(&mut self) -> bool {
        let mut changed = false;
        for col in &mut self.current {
            for cell in col {
                if *cell {
                    *cell = false;
                    changed = true;
                }
            }
        }

        if changed {
            self.render_dirty = true;
        }
        changed
    }

    fn alive(&self, x: i32, y: i32) -> bool {
        let w = self.cols() as i32;
        let h = self.rows() as i32;
        if w == 0 || h == 0 {
            return false;
        }
        let i = ((x + w) % w) as usize;
        let j = ((y + h) % h) as usize;
        self.current[i][j]
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
        let cols = self.cols();
        let rows = self.rows();
        if cols == 0 || rows == 0 {
            return false;
        }

        let mut changed = false;
        for y in 0..rows {
            for x in 0..cols {
                let next_state = self.next(x as i32, y as i32);
                changed |= next_state != self.current[x][y];
                self.future[x][y] = next_state;
            }
        }
        std::mem::swap(&mut self.current, &mut self.future);
        self.render_dirty = true;
        changed
    }

    pub fn rendered(&mut self) -> &str {
        if self.render_dirty {
            self.rebuild_render_cache();
            self.render_dirty = false;
        }
        self.render_cache.as_str()
    }

    pub fn size(&self) -> (usize, usize) {
        (self.cols(), self.rows())
    }

    pub fn is_alive(&self, col: usize, row: usize) -> bool {
        if col < self.cols() && row < self.rows() {
            return self.current[col][row];
        }
        false
    }

    pub fn live_cells_count(&self) -> usize {
        self.current
            .iter()
            .map(|col| col.iter().filter(|&&cell| cell).count())
            .sum()
    }

    fn cols(&self) -> usize {
        self.current.len()
    }

    fn rows(&self) -> usize {
        self.current.first().map_or(0, Vec::len)
    }

    fn rebuild_render_cache(&mut self) {
        let cols = self.cols();
        let rows = self.rows();
        let capacity = rows.saturating_mul(cols.saturating_mul(CENTER_FULL.len()) + 1);
        let mut cache = String::with_capacity(capacity);
        for y in 0..rows {
            for x in 0..cols {
                if self.current[x][y] {
                    cache.push_str(CENTER_FULL);
                } else {
                    cache.push_str(CENTER_EMPTY);
                }
            }
            cache.push('\n');
        }
        self.render_cache = cache;
    }
}

#[cfg(test)]
mod tests {
    use super::Board;

    fn live_cells(board: &Board) -> Vec<(usize, usize)> {
        let mut cells = Vec::new();
        for x in 0..board.current.len() {
            for y in 0..board.current[x].len() {
                if board.current[x][y] {
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
        board.populate(4, 4);
        board.populate(4, 0);
        board.populate(0, 4);

        board.step();

        assert!(board.current[0][0]);
    }

    #[test]
    fn resize_preserves_overlapping_cells_only() {
        let mut board = Board::new(4, 4);
        board.populate(0, 0);
        board.populate(3, 3);

        board.resize(2, 3);

        assert_eq!(board.current.len(), 2);
        assert_eq!(board.current[0].len(), 3);
        assert!(board.current[0][0]);
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
}
