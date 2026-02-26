use crate::board;

/// Application.
#[derive(Debug)]
pub struct App {
    /// should the application exit?
    pub should_quit: bool,
    /// current epoch
    pub epoch: usize,
    /// tick interval in milliseconds
    tick_rate_ms: u64,
    /// whether simulation advances on each tick
    running: bool,
    /// whether UI should redraw
    needs_redraw: bool,
    /// whether help overlay is visible
    show_help: bool,

    pub board: board::Board,
}

impl App {
    const TICK_RATE_MIN_MS: u64 = 25;
    const TICK_RATE_MAX_MS: u64 = 1000;
    const TICK_RATE_STEP_MS: u64 = 25;

    /// Constructs a new instance of [`App`].
    pub fn new(cols: usize, rows: usize) -> Self {
        let board = board::Board::new(cols, rows);
        Self {
            should_quit: false,
            epoch: 0,
            tick_rate_ms: 250,
            running: false,
            needs_redraw: true,
            show_help: false,
            board,
        }
    }

    /// Handles the tick event of the terminal.
    pub fn tick(&mut self) {
        if self.running {
            self.increment_epoch();
        }
    }

    pub fn toggle_running(&mut self) {
        self.running = !self.running;
        self.needs_redraw = true;
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    pub fn is_help_visible(&self) -> bool {
        self.show_help
    }

    pub fn toggle_help(&mut self) {
        self.show_help = !self.show_help;
        self.needs_redraw = true;
    }

    pub fn tick_rate_ms(&self) -> u64 {
        self.tick_rate_ms
    }

    pub fn speed_up(&mut self) -> bool {
        if self.tick_rate_ms <= Self::TICK_RATE_MIN_MS {
            return false;
        }

        self.tick_rate_ms = self
            .tick_rate_ms
            .saturating_sub(Self::TICK_RATE_STEP_MS)
            .max(Self::TICK_RATE_MIN_MS);
        self.needs_redraw = true;
        true
    }

    pub fn slow_down(&mut self) -> bool {
        if self.tick_rate_ms >= Self::TICK_RATE_MAX_MS {
            return false;
        }

        self.tick_rate_ms =
            (self.tick_rate_ms + Self::TICK_RATE_STEP_MS).min(Self::TICK_RATE_MAX_MS);
        self.needs_redraw = true;
        true
    }

    /// Set should_quit to true to quit the application.
    pub fn quit(&mut self) {
        self.should_quit = true;
        self.needs_redraw = true;
    }

    pub fn increment_epoch(&mut self) {
        if let Some(res) = self.epoch.checked_add(1) {
            self.epoch = res;
            self.board.step();
            self.needs_redraw = true;
        }
    }

    pub fn reset_epoch(&mut self) {
        if self.epoch != 0 {
            self.epoch = 0;
            self.needs_redraw = true;
        }
    }

    pub fn randomize_board(&mut self) {
        self.board.clear();
        self.board.populate_random();
        self.needs_redraw = true;
    }

    pub fn clear_board(&mut self) {
        if self.board.clear() {
            self.needs_redraw = true;
        }
    }

    pub fn populate_board(&mut self, col: usize, row: usize) {
        if self.board.populate(col, row) {
            self.needs_redraw = true;
        }
    }

    pub fn depopulate_board(&mut self, col: usize, row: usize) {
        if self.board.depopulate(col, row) {
            self.needs_redraw = true;
        }
    }

    pub fn resize_board(&mut self, new_cols: usize, new_rows: usize) {
        if self.board.resize(new_cols, new_rows) {
            self.needs_redraw = true;
        }
    }

    pub fn take_redraw(&mut self) -> bool {
        let redraw = self.needs_redraw;
        self.needs_redraw = false;
        redraw
    }

    pub fn insert_glider(&mut self) {
        const GLIDER: &[(usize, usize)] = &[(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];
        self.place_pattern_centered(GLIDER, 3, 3);
    }

    pub fn insert_blinker(&mut self) {
        const BLINKER: &[(usize, usize)] = &[(0, 0), (1, 0), (2, 0)];
        self.place_pattern_centered(BLINKER, 3, 1);
    }

    pub fn insert_lwss(&mut self) {
        const LWSS: &[(usize, usize)] = &[
            (1, 0),
            (4, 0),
            (0, 1),
            (0, 2),
            (3, 2),
            (0, 3),
            (1, 3),
            (2, 3),
            (3, 3),
        ];
        self.place_pattern_centered(LWSS, 5, 4);
    }

    fn place_pattern_centered(&mut self, pattern: &[(usize, usize)], width: usize, height: usize) {
        let (cols, rows) = self.board.size();
        if cols == 0 || rows == 0 {
            return;
        }

        let origin_x = (cols.saturating_sub(width)) / 2;
        let origin_y = (rows.saturating_sub(height)) / 2;
        let mut changed = false;

        for &(dx, dy) in pattern {
            changed |= self.board.populate(origin_x + dx, origin_y + dy);
        }

        if changed {
            self.needs_redraw = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_increment_epoch() {
        let mut app = App::new(4, 4);
        app.increment_epoch();
        assert_eq!(app.epoch, 1);
    }

    #[test]
    fn test_reset_epoch() {
        let mut app = App::new(4, 4);
        app.increment_epoch();
        app.reset_epoch();
        assert_eq!(app.epoch, 0);
    }

    #[test]
    fn test_clear_board_removes_cells() {
        let mut app = App::new(4, 4);
        app.populate_board(1, 1);
        app.clear_board();
        assert!(!app.board.rendered().contains("██"));
    }

    #[test]
    fn test_toggle_help() {
        let mut app = App::new(4, 4);
        assert!(!app.is_help_visible());
        app.toggle_help();
        assert!(app.is_help_visible());
    }

    #[test]
    fn test_insert_glider_populates_board() {
        let mut app = App::new(8, 8);
        app.insert_glider();
        assert!(app.board.rendered().contains("██"));
    }
}
