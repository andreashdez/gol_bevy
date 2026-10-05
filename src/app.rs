use crate::{
    board::{self, Rule},
    patterns::{Pattern, PatternParseError},
};
use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EditTool {
    #[default]
    Draw,
    Erase,
    Stamp,
}

/// What an RLE import changed beyond adding a stamp.
#[derive(Debug, Clone, Copy, Default)]
pub struct PatternImport {
    /// Set when the pattern declared a rule that differed from the board's.
    pub applied_rule: Option<Rule>,
}

#[derive(Debug)]
struct EditSnapshot {
    cells: Vec<u8>,
    epoch: usize,
    rule: Rule,
}

const EDIT_HISTORY_MAX: usize = 32;
const EDIT_HISTORY_BYTE_BUDGET: usize = 16 * 1024 * 1024;

/// Keeps a history within both the edit-count cap and the byte budget, always
/// retaining at least one entry.
fn push_bounded(history: &mut VecDeque<EditSnapshot>, snapshot: EditSnapshot) {
    let limit = (EDIT_HISTORY_BYTE_BUDGET / snapshot.cells.len().max(1)).clamp(1, EDIT_HISTORY_MAX);
    while history.len() >= limit {
        history.pop_front();
    }
    history.push_back(snapshot);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternKind {
    Glider,
    Blinker,
    Lwss,
    RleDemo,
}

impl PatternKind {
    fn slot(self) -> usize {
        match self {
            Self::Glider => 0,
            Self::Blinker => 1,
            Self::Lwss => 2,
            Self::RleDemo => 3,
        }
    }
}

type PatternLayout = (Vec<(usize, usize)>, usize, usize);

#[derive(Debug, Clone, Copy, Default)]
struct StampTransform {
    rotation_quarters: u8,
    flip_horizontal: bool,
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
    /// whether UI should redraw
    needs_redraw: bool,
    /// whether help overlay is visible
    show_help: bool,
    /// selected pattern stamp for cursor placement
    selected_pattern: Option<usize>,
    /// transform applied to stamp before placement
    stamp_transform: StampTransform,
    /// available stamp patterns
    patterns: Vec<Pattern>,
    tool: EditTool,
    undo_history: VecDeque<EditSnapshot>,
    redo_history: VecDeque<EditSnapshot>,
    stroke_active: bool,
    stroke_saved: bool,

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
            tick_rate_ms: 25,
            max_speed: false,
            running: false,
            needs_redraw: true,
            show_help: false,
            selected_pattern: None,
            stamp_transform: StampTransform::default(),
            patterns: Pattern::builtins(),
            tool: EditTool::Draw,
            undo_history: VecDeque::new(),
            redo_history: VecDeque::new(),
            stroke_active: false,
            stroke_saved: false,
            board,
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

    pub fn set_help_visible(&mut self, visible: bool) {
        if self.show_help != visible {
            self.show_help = visible;
            self.needs_redraw = true;
        }
    }

    pub fn tick_rate_ms(&self) -> u64 {
        self.tick_rate_ms
    }

    pub fn tick_rate_bounds() -> (u64, u64, u64) {
        (
            Self::TICK_RATE_MIN_MS,
            Self::TICK_RATE_MAX_MS,
            Self::TICK_RATE_STEP_MS,
        )
    }

    /// Choosing an interval also leaves max speed.
    pub fn set_tick_rate_ms(&mut self, tick_rate_ms: u64) -> bool {
        let clamped = tick_rate_ms.clamp(Self::TICK_RATE_MIN_MS, Self::TICK_RATE_MAX_MS);
        if self.tick_rate_ms == clamped && !self.max_speed {
            return false;
        }

        self.tick_rate_ms = clamped;
        self.max_speed = false;
        self.needs_redraw = true;
        true
    }

    pub fn is_max_speed(&self) -> bool {
        self.max_speed
    }

    /// Runs generations back to back instead of on the tick interval. The
    /// interval is kept so slowing down returns to the fastest fixed rate.
    pub fn set_max_speed(&mut self) -> bool {
        if self.max_speed {
            return false;
        }
        self.max_speed = true;
        self.needs_redraw = true;
        true
    }

    pub fn toggle_boundary_mode(&mut self) {
        self.board.toggle_boundary_mode();
        self.needs_redraw = true;
    }

    pub fn boundary_mode_label(&self) -> &'static str {
        self.board.boundary_mode_label()
    }

    pub fn speed_up(&mut self) -> bool {
        if self.tick_rate_ms <= Self::TICK_RATE_MIN_MS {
            // Past the fastest interval is max speed.
            return self.set_max_speed();
        }

        self.tick_rate_ms = self
            .tick_rate_ms
            .saturating_sub(Self::TICK_RATE_STEP_MS)
            .max(Self::TICK_RATE_MIN_MS);
        self.needs_redraw = true;
        true
    }

    pub fn slow_down(&mut self) -> bool {
        if self.max_speed {
            return self.set_tick_rate_ms(Self::TICK_RATE_MIN_MS);
        }
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
        self.end_stroke();
        self.save_edit();
        self.board.clear();
        self.board.populate_random();
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

    pub fn populate_board(&mut self, col: usize, row: usize) {
        let (cols, rows) = self.board.size();
        if col >= cols || row >= rows || self.board.is_alive(col, row) {
            return;
        }
        self.save_edit();
        if self.board.populate(col, row) {
            self.needs_redraw = true;
        }
    }

    pub fn depopulate_board(&mut self, col: usize, row: usize) {
        if !self.board.is_alive(col, row) {
            return;
        }
        self.save_edit();
        if self.board.depopulate(col, row) {
            self.needs_redraw = true;
        }
    }

    pub fn resize_board(&mut self, new_cols: usize, new_rows: usize) {
        if self.board.resize(new_cols, new_rows) {
            // Snapshots hold cells in the old layout, so they no longer fit.
            self.undo_history.clear();
            self.redo_history.clear();
            self.end_stroke();
            self.needs_redraw = true;
        }
    }

    pub fn take_redraw(&mut self) -> bool {
        let redraw = self.needs_redraw;
        self.needs_redraw = false;
        redraw
    }

    pub fn select_pattern(&mut self, pattern: PatternKind) {
        let slot = pattern.slot();
        if slot >= self.patterns.len() {
            return;
        }
        if self.selected_pattern == Some(slot) && self.tool == EditTool::Stamp {
            self.clear_selected_pattern();
        } else {
            self.selected_pattern = Some(slot);
            self.tool = EditTool::Stamp;
        }
        self.needs_redraw = true;
    }

    pub fn import_rle_pattern(
        &mut self,
        name: impl Into<String>,
        source: &str,
    ) -> Result<PatternImport, PatternParseError> {
        let pattern = Pattern::from_rle(name, source)?;

        // Honour a declared rule instead of simulating the pattern as plain Life.
        // Changing it is an undoable edit, so undo can return to the old rule.
        let mut applied_rule = None;
        if let Some(rule) = pattern.rule
            && rule != self.board.rule()
        {
            self.end_stroke();
            self.save_edit();
            self.board.set_rule(rule);
            applied_rule = Some(rule);
        }

        self.patterns.push(pattern);
        self.selected_pattern = Some(self.patterns.len() - 1);
        self.tool = EditTool::Stamp;
        self.needs_redraw = true;
        Ok(PatternImport { applied_rule })
    }

    pub fn rule_label(&self) -> String {
        self.board.rule().to_string()
    }

    pub fn clear_selected_pattern(&mut self) {
        self.tool = EditTool::Draw;
        if self.selected_pattern.take().is_some() {
            self.needs_redraw = true;
        }
    }

    pub fn selected_pattern_index(&self) -> Option<usize> {
        self.selected_pattern
            .filter(|&idx| idx < self.patterns.len())
    }

    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }

    pub fn pattern_name_at(&self, index: usize) -> Option<&str> {
        self.patterns
            .get(index)
            .map(|pattern| pattern.name.as_str())
    }

    pub fn select_pattern_index(&mut self, index: usize) -> bool {
        if index >= self.patterns.len()
            || (self.selected_pattern == Some(index) && self.tool == EditTool::Stamp)
        {
            return false;
        }

        self.selected_pattern = Some(index);
        self.tool = EditTool::Stamp;
        self.needs_redraw = true;
        true
    }

    pub fn has_selected_pattern(&self) -> bool {
        self.tool == EditTool::Stamp
            && self
                .selected_pattern
                .is_some_and(|idx| idx < self.patterns.len())
    }

    pub fn selected_pattern_name(&self) -> Option<&str> {
        self.selected_pattern
            .and_then(|idx| self.patterns.get(idx))
            .map(|pattern| pattern.name.as_str())
    }

    pub fn selected_pattern_status_label(&self) -> String {
        let Some(name) = self.selected_pattern_name() else {
            return "Draw".to_string();
        };
        if self.stamp_transform.rotation_quarters == 0 && !self.stamp_transform.flip_horizontal {
            return name.to_string();
        }

        let mut suffix = String::new();
        if self.stamp_transform.flip_horizontal {
            suffix.push_str("FlipH");
        }
        if self.stamp_transform.rotation_quarters != 0 {
            if !suffix.is_empty() {
                suffix.push(' ');
            }
            let degrees = self.stamp_transform.rotation_quarters as u32 * 90;
            suffix.push_str(&format!("R{degrees}"));
        }
        format!("{name} ({suffix})")
    }

    pub fn stamp_orientation(&self) -> (u8, bool) {
        (
            self.stamp_transform.rotation_quarters,
            self.stamp_transform.flip_horizontal,
        )
    }

    pub fn rotate_stamp_clockwise(&mut self) {
        self.stamp_transform.rotation_quarters = (self.stamp_transform.rotation_quarters + 1) % 4;
        self.needs_redraw = true;
    }

    pub fn toggle_stamp_flip(&mut self) {
        self.stamp_transform.flip_horizontal = !self.stamp_transform.flip_horizontal;
        self.needs_redraw = true;
    }

    pub fn selected_pattern_layout(&self) -> Option<PatternLayout> {
        if self.tool != EditTool::Stamp {
            return None;
        }
        self.selected_pattern.and_then(|idx| {
            self.patterns.get(idx).map(|pattern| {
                self.transform_pattern(&pattern.cells, pattern.width, pattern.height)
            })
        })
    }

    pub fn place_selected_pattern_at(&mut self, col: usize, row: usize) {
        let Some((cells, width, height)) = self.selected_pattern.and_then(|idx| {
            self.patterns
                .get(idx)
                .map(|pattern| (pattern.cells.clone(), pattern.width, pattern.height))
        }) else {
            return;
        };
        let (pattern, width, height) = self.transform_pattern(&cells, width, height);
        self.place_pattern_at(&pattern, width, height, col, row);
    }

    fn place_pattern_at(
        &mut self,
        pattern: &[(usize, usize)],
        width: usize,
        height: usize,
        center_col: usize,
        center_row: usize,
    ) {
        let origin_x = center_col.saturating_sub(width / 2);
        let origin_y = center_row.saturating_sub(height / 2);
        let (cols, rows) = self.board.size();
        let changes_board = pattern.iter().any(|&(dx, dy)| {
            let (col, row) = (origin_x + dx, origin_y + dy);
            col < cols && row < rows && !self.board.is_alive(col, row)
        });
        if !changes_board {
            return;
        }
        self.end_stroke();
        self.save_edit();
        let mut changed = false;

        for &(dx, dy) in pattern {
            changed |= self.board.populate(origin_x + dx, origin_y + dy);
        }

        if changed {
            self.needs_redraw = true;
        }
    }

    pub fn tool(&self) -> EditTool {
        self.tool
    }

    pub fn set_tool(&mut self, tool: EditTool) {
        self.end_stroke();
        self.tool = tool;
        if tool == EditTool::Stamp && self.selected_pattern.is_none() && !self.patterns.is_empty() {
            self.selected_pattern = Some(0);
        }
        self.needs_redraw = true;
    }

    pub fn pattern_at(&self, index: usize) -> Option<&Pattern> {
        self.patterns.get(index)
    }

    pub fn begin_stroke(&mut self) {
        self.stroke_active = true;
    }

    pub fn end_stroke(&mut self) {
        self.stroke_active = false;
        self.stroke_saved = false;
    }

    fn save_edit(&mut self) {
        if self.stroke_active && self.stroke_saved {
            return;
        }
        let snapshot = self.current_snapshot();
        push_bounded(&mut self.undo_history, snapshot);
        // A fresh edit invalidates whatever had been undone.
        self.redo_history.clear();
        self.stroke_saved = self.stroke_active;
    }

    fn current_snapshot(&self) -> EditSnapshot {
        EditSnapshot {
            cells: self.board.snapshot_cells(),
            epoch: self.epoch,
            rule: self.board.rule(),
        }
    }

    fn restore(&mut self, snapshot: EditSnapshot) {
        self.board.set_rule(snapshot.rule);
        self.board.restore_cells(snapshot.cells);
        self.epoch = snapshot.epoch;
        self.running = false;
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
        let Some(snapshot) = self.undo_history.pop_back() else {
            return false;
        };
        let current = self.current_snapshot();
        push_bounded(&mut self.redo_history, current);
        self.restore(snapshot);
        true
    }

    pub fn redo(&mut self) -> bool {
        self.end_stroke();
        let Some(snapshot) = self.redo_history.pop_back() else {
            return false;
        };
        let current = self.current_snapshot();
        push_bounded(&mut self.undo_history, current);
        self.restore(snapshot);
        true
    }

    fn transform_pattern(
        &self,
        pattern: &[(usize, usize)],
        width: usize,
        height: usize,
    ) -> (Vec<(usize, usize)>, usize, usize) {
        let mut transformed = Vec::with_capacity(pattern.len());
        for &(x0, y0) in pattern {
            let mut x = x0;
            let mut y = y0;
            let mut w = width;
            let mut h = height;

            if self.stamp_transform.flip_horizontal {
                x = w.saturating_sub(1).saturating_sub(x);
            }

            for _ in 0..self.stamp_transform.rotation_quarters {
                let nx = h.saturating_sub(1).saturating_sub(y);
                let ny = x;
                x = nx;
                y = ny;
                std::mem::swap(&mut w, &mut h);
            }

            transformed.push((x, y));
        }

        let (final_w, final_h) = match self.stamp_transform.rotation_quarters {
            1 | 3 => (height, width),
            _ => (width, height),
        };
        (transformed, final_w, final_h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp_centered(app: &mut App, pattern: PatternKind) {
        app.select_pattern(pattern);
        let (cols, rows) = app.board.size();
        app.place_selected_pattern_at(cols / 2, rows / 2);
    }

    #[test]
    fn undo_groups_drag_and_ignores_noop_edits() {
        let mut app = App::new(8, 8);
        app.clear_board();
        app.depopulate_board(0, 0);
        assert!(!app.can_undo());
        app.begin_stroke();
        app.populate_board(2, 2);
        app.populate_board(3, 2);
        app.populate_board(3, 2);
        app.end_stroke();
        app.populate_board(3, 2);
        assert!(app.undo());
        assert_eq!(app.board.live_cells_count(), 0);
        assert!(!app.can_undo());
    }

    #[test]
    fn undo_restores_clear_and_generation_then_simulation_can_continue() {
        let mut app = App::new(8, 8);
        stamp_centered(&mut app, PatternKind::Blinker);
        app.increment_epoch();
        let expected = app.board.snapshot_cells();
        let mut next = app.board.clone();
        next.step();
        app.clear_board();
        app.toggle_running();
        app.increment_epoch();
        assert!(app.undo());
        assert_eq!(app.epoch, 1);
        assert!(!app.is_running());
        assert_eq!(app.board.snapshot_cells(), expected);
        assert_eq!(app.board.take_dirty_cells(), board::DirtyCells::Full);
        app.increment_epoch();
        assert_eq!(app.board.snapshot_cells(), next.snapshot_cells());
    }

    #[test]
    fn undo_restores_erased_cells_randomization_and_stamps() {
        let mut app = App::new(8, 8);
        stamp_centered(&mut app, PatternKind::Glider);
        let original = app.board.snapshot_cells();
        app.begin_stroke();
        for row in 0..8 {
            for col in 0..8 {
                app.depopulate_board(col, row);
            }
        }
        app.end_stroke();
        assert_eq!(app.board.live_cells_count(), 0);
        assert!(app.undo());
        assert_eq!(app.board.snapshot_cells(), original);
        app.randomize_board();
        assert!(app.undo());
        assert_eq!(app.board.snapshot_cells(), original);
        assert!(app.undo());
        assert_eq!(app.board.live_cells_count(), 0);
    }

    #[test]
    fn redo_reapplies_undone_edits_until_a_new_edit_discards_the_chain() {
        let mut app = App::new(8, 8);
        assert!(!app.can_redo());
        assert!(!app.redo());

        app.populate_board(1, 1);
        app.populate_board(2, 2);
        let two_edits = app.board.snapshot_cells();

        assert!(app.undo());
        assert!(app.can_redo());
        assert_eq!(app.board.live_cells_count(), 1);

        assert!(app.redo());
        assert_eq!(app.board.snapshot_cells(), two_edits);
        assert!(!app.can_redo());

        // Undo twice, then make a different edit: the redo chain must be dropped.
        assert!(app.undo());
        assert!(app.undo());
        assert_eq!(app.board.live_cells_count(), 0);
        assert!(app.can_redo());
        app.populate_board(5, 5);
        assert!(!app.can_redo());
        assert!(!app.redo());
    }

    #[test]
    fn redo_restores_generation_and_leaves_playback_paused() {
        let mut app = App::new(8, 8);
        stamp_centered(&mut app, PatternKind::Blinker);
        app.increment_epoch();
        app.increment_epoch();
        let epoch_before_clear = app.epoch;

        app.clear_board();
        assert!(app.undo());
        assert_eq!(app.epoch, epoch_before_clear);

        app.toggle_running();
        assert!(app.redo());
        assert_eq!(app.board.live_cells_count(), 0);
        assert_eq!(app.epoch, epoch_before_clear);
        assert!(!app.is_running());
    }

    #[test]
    fn resize_invalidates_the_redo_chain_too() {
        let mut app = App::new(8, 8);
        app.populate_board(1, 1);
        assert!(app.undo());
        assert!(app.can_redo());

        app.resize_board(9, 8);
        assert!(!app.can_redo());
        assert!(!app.can_undo());
    }

    #[test]
    fn importing_rle_applies_a_declared_rule() {
        let mut app = App::new(12, 12);
        assert_eq!(app.rule_label(), "B3/S23");

        // HighLife: the same survival counts plus birth on six neighbours.
        let import = app
            .import_rle_pattern("HighLife", "x = 3, y = 1, rule = B36/S23\n3o!")
            .expect("RLE with a rule should import");
        assert_eq!(
            import.applied_rule.map(|rule| rule.to_string()),
            Some("B36/S23".to_string())
        );
        assert_eq!(app.rule_label(), "B36/S23");

        // A header without a rule leaves the board rule alone.
        let plain = app
            .import_rle_pattern("Plain", "x = 3, y = 1\n3o!")
            .expect("RLE without a rule should import");
        assert!(plain.applied_rule.is_none());
        assert_eq!(app.rule_label(), "B36/S23");
    }

    #[test]
    fn undo_and_redo_carry_the_rule_an_import_changed() {
        let mut app = App::new(12, 12);
        app.populate_board(1, 1);

        app.import_rle_pattern("HighLife", "x = 3, y = 1, rule = B36/S23\n3o!")
            .expect("RLE with a rule should import");
        assert_eq!(app.rule_label(), "B36/S23");

        // The rule change is its own edit: undo reverts it and keeps the cells.
        assert!(app.undo());
        assert_eq!(app.rule_label(), "B3/S23");
        assert_eq!(app.board.live_cells_count(), 1);

        assert!(app.redo());
        assert_eq!(app.rule_label(), "B36/S23");

        // Undoing past the import also lands on the rule of that earlier state.
        assert!(app.undo());
        assert!(app.undo());
        assert_eq!(app.rule_label(), "B3/S23");
        assert_eq!(app.board.live_cells_count(), 0);

        // An import that leaves the rule alone adds no undo step.
        let mut plain = App::new(12, 12);
        plain
            .import_rle_pattern("Plain", "x = 3, y = 1, rule = B3/S23\n3o!")
            .expect("RLE should import");
        assert!(!plain.can_undo());
    }

    #[test]
    fn undo_history_is_bounded_and_invalidated_only_on_actual_resize() {
        let mut app = App::new(8, 8);
        for i in 0..40 {
            app.populate_board(i % 8, i / 8);
        }
        assert_eq!(app.undo_history.len(), 32);
        app.resize_board(8, 8);
        assert!(app.can_undo());
        app.resize_board(9, 8);
        assert!(!app.can_undo());
        assert!(!app.undo());
    }

    #[test]
    fn switching_tools_preserves_stamp_selection_but_hides_ghost() {
        let mut app = App::new(8, 8);
        app.select_pattern(PatternKind::Blinker);
        app.set_tool(EditTool::Erase);
        assert!(!app.has_selected_pattern());
        assert!(app.selected_pattern_layout().is_none());
        app.set_tool(EditTool::Stamp);
        assert!(app.has_selected_pattern());
        assert_eq!(app.selected_pattern_name(), Some("Blinker"));
        app.set_tool(EditTool::Draw);
        assert!(app.select_pattern_index(1));
        assert_eq!(app.tool(), EditTool::Stamp);
    }

    #[test]
    fn speed_accepts_intervals_between_keyboard_steps_and_clamps_bounds() {
        let mut app = App::new(8, 8);
        app.set_tick_rate_ms(100);
        assert_eq!(1000 / app.tick_rate_ms(), 10);
        app.set_tick_rate_ms(33);
        assert_eq!(app.tick_rate_ms(), 33);
        app.set_tick_rate_ms(0);
        assert_eq!(app.tick_rate_ms(), 25);
        app.set_tick_rate_ms(2000);
        assert_eq!(app.tick_rate_ms(), 1000);
    }

    #[test]
    fn speeding_up_past_the_fastest_interval_reaches_max_speed_and_back() {
        let mut app = App::new(8, 8);
        app.set_tick_rate_ms(50);
        assert!(app.speed_up());
        assert!(!app.is_max_speed());
        assert_eq!(app.tick_rate_ms(), 25);

        assert!(app.speed_up());
        assert!(app.is_max_speed());
        assert!(!app.speed_up(), "max speed is the ceiling");

        assert!(app.slow_down());
        assert!(!app.is_max_speed());
        assert_eq!(app.tick_rate_ms(), 25);

        // Picking any interval, even the current one, leaves max speed.
        app.set_max_speed();
        assert!(app.set_tick_rate_ms(25));
        assert!(!app.is_max_speed());
        assert!(!app.set_tick_rate_ms(25));
    }

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
    fn test_toggle_help() {
        let mut app = App::new(4, 4);
        assert!(!app.is_help_visible());
        app.toggle_help();
        assert!(app.is_help_visible());
    }

    #[test]
    fn test_select_pattern_toggles() {
        let mut app = App::new(8, 8);
        app.select_pattern(PatternKind::Glider);
        assert_eq!(app.selected_pattern_name(), Some("Glider"));
        app.select_pattern(PatternKind::Glider);
        assert_eq!(app.selected_pattern_name(), None);
    }

    #[test]
    fn test_place_selected_pattern_at_cursor() {
        let mut app = App::new(8, 8);
        app.select_pattern(PatternKind::Blinker);
        app.place_selected_pattern_at(4, 4);
        assert!(app.board.is_alive(3, 4));
        assert!(app.board.is_alive(4, 4));
        assert!(app.board.is_alive(5, 4));
    }

    #[test]
    fn test_toggle_boundary_mode() {
        let mut app = App::new(8, 8);
        assert_eq!(app.boundary_mode_label(), "Wrap");
        app.toggle_boundary_mode();
        assert_eq!(app.boundary_mode_label(), "Fixed");
        app.toggle_boundary_mode();
        assert_eq!(app.boundary_mode_label(), "Wrap");
    }

    #[test]
    fn test_rotate_stamp_changes_placement_orientation() {
        let mut app = App::new(8, 8);
        app.select_pattern(PatternKind::Blinker);
        app.rotate_stamp_clockwise();
        app.place_selected_pattern_at(4, 4);

        assert!(app.board.is_alive(4, 3));
        assert!(app.board.is_alive(4, 4));
        assert!(app.board.is_alive(4, 5));
    }

    #[test]
    fn test_flip_stamp_changes_status_label() {
        let mut app = App::new(8, 8);
        app.select_pattern(PatternKind::Glider);
        app.toggle_stamp_flip();
        assert_eq!(app.selected_pattern_status_label(), "Glider (FlipH)");
    }

    #[test]
    fn test_rle_demo_pattern_has_dynamic_size() {
        let mut app = App::new(20, 20);
        app.select_pattern(PatternKind::RleDemo);

        let (cells, width, height) = app
            .selected_pattern_layout()
            .expect("RLE demo pattern should be selectable");
        assert!(cells.len() > 20);
        assert_eq!((width, height), (8, 4));
    }
}
