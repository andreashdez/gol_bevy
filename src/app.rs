use crate::{
    board::{self, RandomizeSettings, Rule},
    patterns::{Pattern, PatternParseError},
    settle::{SettleWatch, Settled},
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

/// The board written out as RLE, with what it covers for user feedback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardExport {
    pub rle: String,
    pub cells: usize,
    pub width: usize,
    pub height: usize,
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

/// Built-in patterns that tests select by name.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternKind {
    Glider,
    Blinker,
    Lwss,
}

#[cfg(test)]
impl PatternKind {
    /// Name of the file under `assets/patterns/`, without `.rle`.
    fn file_stem(self) -> &'static str {
        match self {
            Self::Glider => "glider",
            Self::Blinker => "blinker",
            Self::Lwss => "lwss",
        }
    }

    /// Looked up by file name, so the order of the built-in list is free to
    /// change. `None` only if the file has been removed.
    pub fn builtin_index(self) -> Option<usize> {
        Pattern::builtin_index(self.file_stem())
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
    library_revision: u64,
    randomize_settings: RandomizeSettings,
    tool: EditTool,
    undo_history: VecDeque<EditSnapshot>,
    redo_history: VecDeque<EditSnapshot>,
    stroke_active: bool,
    stroke_saved: bool,
    /// The latest generation, with the live cells before and after it.
    last_step: Option<(usize, usize, usize)>,
    /// The board before the first generation since the latest edit.
    run_start: Option<EditSnapshot>,
    settle: SettleWatch,
    pause_when_settled: bool,
    /// Why playback paused itself, until the UI has said so.
    auto_paused: Option<Settled>,

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
            library_revision: 0,
            randomize_settings: RandomizeSettings::default(),
            tool: EditTool::Draw,
            undo_history: VecDeque::new(),
            redo_history: VecDeque::new(),
            stroke_active: false,
            stroke_saved: false,
            last_step: None,
            run_start: None,
            settle: SettleWatch::default(),
            pause_when_settled: false,
            auto_paused: None,
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
    /// interval is kept so switching off Max restores the selected fixed rate.
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
        self.settle.reset();
        self.needs_redraw = true;
    }

    pub fn set_boundary_mode(&mut self, mode: crate::board::BoundaryMode) {
        if mode != self.board.boundary_mode() {
            self.settle.reset();
        }
        self.board.set_boundary_mode(mode);
        self.needs_redraw = true;
    }

    pub fn boundary_mode_label(&self) -> &'static str {
        self.board.boundary_mode_label()
    }

    pub fn speed_up(&mut self) -> bool {
        if self.max_speed {
            return false;
        }
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
            if self.run_start.is_none() {
                self.run_start = Some(self.current_snapshot());
            }
            self.epoch = res;
            let before = self.board.live_cells_count();
            let fingerprint = self.board.fingerprint();
            self.board.step();
            self.last_step = Some((res, before, self.board.live_cells_count()));
            let settled = self
                .settle
                .record(fingerprint, self.board.fingerprint(), res);
            if self.running && self.pause_when_settled && settled.is_some() {
                self.running = false;
                self.auto_paused = settled;
            }
            self.needs_redraw = true;
        }
    }

    /// Set once the board repeats an earlier generation, until it is edited.
    pub fn settled(&self) -> Option<Settled> {
        self.settle.settled()
    }

    pub fn pause_when_settled(&self) -> bool {
        self.pause_when_settled
    }

    pub fn set_pause_when_settled(&mut self, pause: bool) {
        self.pause_when_settled = pause;
    }

    /// What just paused playback, once, so the UI can explain it.
    pub fn take_auto_paused(&mut self) -> Option<Settled> {
        self.auto_paused.take()
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

    /// How the latest generation changed the live cell count. `None` once an
    /// edit, undo, or counter reset means it no longer describes the board.
    pub fn population_change(&self) -> Option<isize> {
        let (epoch, before, after) = self.last_step?;
        (epoch == self.epoch && after == self.board.live_cells_count())
            .then(|| after as isize - before as isize)
    }

    pub fn reset_epoch(&mut self) {
        if self.epoch != 0 {
            self.epoch = 0;
            self.settle.renumber();
            self.needs_redraw = true;
        }
    }

    pub fn randomize_settings(&self) -> RandomizeSettings {
        self.randomize_settings
    }

    /// Preferences apply to the next randomization and do not edit the board.
    pub fn set_randomize_settings(&mut self, settings: RandomizeSettings) {
        self.randomize_settings = settings;
    }

    pub fn randomize_board(&mut self) {
        self.end_stroke();
        self.save_edit();
        self.board.populate_random_with(self.randomize_settings);
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
            self.forget_run();
            self.end_stroke();
            self.needs_redraw = true;
        }
    }

    pub fn take_redraw(&mut self) -> bool {
        let redraw = self.needs_redraw;
        self.needs_redraw = false;
        redraw
    }

    #[cfg(test)]
    pub fn select_pattern(&mut self, pattern: PatternKind) {
        let index = pattern.builtin_index().expect("the built-in exists");
        self.select_pattern_index(index);
    }

    pub fn import_rle_pattern(
        &mut self,
        name: impl Into<String>,
        source: &str,
    ) -> Result<PatternImport, PatternParseError> {
        let pattern = Pattern::from_rle(name, source)?;
        if pattern.name.len() > crate::patterns::MAX_PATTERN_NAME_BYTES {
            return Err(PatternParseError::NameTooLong);
        }
        let imported = &self.patterns[Pattern::builtin_count()..];
        let cells: usize = imported.iter().map(|p| p.cells.len()).sum();
        if imported.len() >= crate::patterns::MAX_LIBRARY_PATTERNS
            || cells + pattern.cells.len() > crate::patterns::MAX_LIBRARY_CELLS
        {
            return Err(PatternParseError::LibraryFull);
        }

        // Honour a declared rule instead of simulating the pattern as plain Life.
        let applied_rule = pattern.rule.filter(|&rule| self.set_rule(rule));

        self.patterns.push(pattern);
        self.library_revision = self.library_revision.wrapping_add(1);
        self.selected_pattern = Some(self.patterns.len() - 1);
        self.tool = EditTool::Stamp;
        self.needs_redraw = true;
        Ok(PatternImport { applied_rule })
    }

    pub fn rule_label(&self) -> String {
        self.board.rule().to_string()
    }

    /// Changes the board rule as an undoable edit, so undo can return to the old
    /// rule. Returns whether the rule actually changed.
    pub fn set_rule(&mut self, rule: Rule) -> bool {
        if rule == self.board.rule() {
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
        restored.board.set_rule(decoded.rule);
        restored.board.set_boundary_mode(decoded.boundary);
        restored.board.restore_cells(decoded.cells);
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

    /// The live cells cropped to their bounding box, as RLE with the board rule
    /// in the header. `None` for an empty board.
    pub fn board_to_rle(&self) -> Option<BoardExport> {
        let (cols, rows) = self.board.size();
        let live: Vec<(usize, usize)> = (0..rows)
            .flat_map(|y| (0..cols).map(move |x| (x, y)))
            .filter(|&(x, y)| self.board.is_alive(x, y))
            .collect();
        let min_x = live.iter().map(|cell| cell.0).min()?;
        let min_y = live.iter().map(|cell| cell.1).min()?;
        let width = live.iter().map(|cell| cell.0).max()? - min_x + 1;
        let height = live.iter().map(|cell| cell.1).max()? - min_y + 1;
        let cells: Vec<_> = live.iter().map(|&(x, y)| (x - min_x, y - min_y)).collect();
        let comment = format!("Generation {} of a {cols}x{rows} board", self.epoch);
        Some(BoardExport {
            rle: crate::patterns::to_rle(&cells, width, height, Some(self.board.rule()), &comment),
            cells: cells.len(),
            width,
            height,
        })
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

    pub fn library_revision(&self) -> u64 {
        self.library_revision
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

    /// The board no longer follows from the run so far.
    fn forget_run(&mut self) {
        self.run_start = None;
        self.settle.reset();
    }

    fn save_edit(&mut self) {
        // Ahead of the stroke check: a stroke can span generations.
        self.forget_run();
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

    /// A board with a blinker, and its cells before any generation has run.
    fn blinker_board() -> (App, Vec<u8>) {
        let mut app = App::new(16, 16);
        stamp_centered(&mut app, PatternKind::Blinker);
        let cells = app.board.snapshot_cells();
        (app, cells)
    }

    #[test]
    fn reset_returns_to_the_board_before_the_first_generation() {
        let (mut app, drawn) = blinker_board();
        assert!(!app.can_reset_to_start());
        assert!(!app.reset_to_start());

        // Pausing and stepping stay part of the same run.
        app.toggle_running();
        app.increment_epoch();
        app.toggle_running();
        app.increment_epoch();
        app.increment_epoch();
        assert!(app.can_reset_to_start());
        app.toggle_running();

        assert!(app.reset_to_start());
        assert_eq!(app.board.snapshot_cells(), drawn);
        assert_eq!(app.epoch, 0);
        assert!(!app.is_running());
        assert!(!app.can_reset_to_start());
    }

    #[test]
    fn an_edit_moves_the_start_of_the_run_even_within_a_stroke() {
        let (mut app, _) = blinker_board();
        app.increment_epoch();
        app.begin_stroke();
        app.populate_board(1, 1);
        assert!(!app.can_reset_to_start());
        app.increment_epoch();
        assert!(app.can_reset_to_start());
        // Later cells of the same stroke are one undo edit, but still edits.
        app.populate_board(2, 1);
        assert!(!app.can_reset_to_start());
        app.end_stroke();
        let edited = app.board.snapshot_cells();

        app.increment_epoch();
        assert!(app.reset_to_start());
        assert_eq!(app.board.snapshot_cells(), edited);
        assert_eq!(app.epoch, 2);
    }

    #[test]
    fn reset_is_an_undoable_edit() {
        let (mut app, drawn) = blinker_board();
        for _ in 0..3 {
            app.increment_epoch();
        }
        let ran = app.board.snapshot_cells();
        assert!(app.reset_to_start());

        assert!(app.undo());
        assert_eq!(app.board.snapshot_cells(), ran);
        assert_eq!(app.epoch, 3);
        assert!(app.redo());
        assert_eq!(app.board.snapshot_cells(), drawn);
        assert_eq!(app.epoch, 0);
    }

    #[test]
    fn undo_drops_the_start_of_the_run() {
        let (mut app, _) = blinker_board();
        app.increment_epoch();
        assert!(app.undo());
        assert!(!app.can_reset_to_start());
    }

    #[test]
    fn resizing_drops_the_start_of_the_run() {
        let (mut app, _) = blinker_board();
        app.increment_epoch();
        app.resize_board(32, 32);
        assert!(!app.can_reset_to_start());
    }

    #[test]
    fn a_still_life_settles_on_the_first_generation_and_an_oscillator_on_its_period() {
        let mut app = App::new(16, 16);
        for (col, row) in [(4, 4), (5, 4), (4, 5), (5, 5)] {
            app.populate_board(col, row);
        }
        assert_eq!(app.settled(), None);
        app.increment_epoch();
        let settled = app.settled().unwrap();
        assert_eq!((settled.since, settled.period), (0, 1));

        let (mut app, _) = blinker_board();
        app.increment_epoch();
        assert_eq!(app.settled(), None);
        app.increment_epoch();
        let settled = app.settled().unwrap();
        assert_eq!((settled.since, settled.period), (0, 2));
    }

    #[test]
    fn a_glider_only_settles_once_it_crashes_into_a_fixed_corner() {
        let mut app = App::new(32, 32);
        app.set_boundary_mode(crate::board::BoundaryMode::Fixed);
        stamp_centered(&mut app, PatternKind::Glider);
        let mut generation = 0;
        while app.settled().is_none() {
            assert!(generation < 200, "the glider should end in the corner");
            app.increment_epoch();
            generation += 1;
        }
        let settled = app.settled().unwrap();
        assert_eq!(settled.period, 1);
        assert!(settled.since > 20);
    }

    #[test]
    fn anything_that_changes_the_evolution_clears_settled() {
        let settled = || {
            let (mut app, _) = blinker_board();
            app.increment_epoch();
            app.increment_epoch();
            assert!(app.settled().is_some());
            app
        };
        let mut app = settled();
        app.populate_board(1, 1);
        assert_eq!(app.settled(), None);

        let mut app = settled();
        app.undo();
        assert_eq!(app.settled(), None);

        let mut app = settled();
        app.reset_to_start();
        assert_eq!(app.settled(), None);

        let mut app = settled();
        app.set_rule(Rule::parse("B36/S23").unwrap());
        assert_eq!(app.settled(), None);

        let mut app = settled();
        app.toggle_boundary_mode();
        assert_eq!(app.settled(), None);

        let mut app = settled();
        app.resize_board(32, 32);
        assert_eq!(app.settled(), None);

        // Choosing the mode already in use changes nothing.
        let mut app = settled();
        app.set_boundary_mode(app.board.boundary_mode());
        assert!(app.settled().is_some());
    }

    #[test]
    fn an_edit_between_generations_does_not_read_as_a_repeat() {
        let (mut app, _) = blinker_board();
        app.increment_epoch();
        app.populate_board(1, 1);
        app.increment_epoch();
        assert_eq!(app.settled(), None);
    }

    #[test]
    fn resetting_the_counter_renumbers_a_settled_board() {
        let (mut app, _) = blinker_board();
        for _ in 0..5 {
            app.increment_epoch();
        }
        app.reset_epoch();
        assert_eq!(app.settled().map(|settled| settled.since), Some(0));
    }

    #[test]
    fn playback_pauses_once_when_settled_if_asked() {
        let (mut app, _) = blinker_board();
        app.set_pause_when_settled(true);
        app.toggle_running();
        app.increment_epoch();
        assert!(app.is_running());
        assert_eq!(app.take_auto_paused(), None);
        app.increment_epoch();
        assert!(!app.is_running());
        assert_eq!(app.take_auto_paused(), app.settled());
        assert_eq!(app.take_auto_paused(), None);

        // Playing on from there is a choice, so it is not interrupted again.
        app.toggle_running();
        app.increment_epoch();
        assert!(app.is_running());
        assert_eq!(app.take_auto_paused(), None);
    }

    #[test]
    fn stepping_to_a_settled_board_does_not_report_a_pause() {
        let (mut app, _) = blinker_board();
        app.set_pause_when_settled(true);
        app.increment_epoch();
        app.increment_epoch();
        assert!(app.settled().is_some());
        assert_eq!(app.take_auto_paused(), None);
    }

    #[test]
    fn playback_keeps_running_past_a_settled_board_by_default() {
        let (mut app, _) = blinker_board();
        app.toggle_running();
        app.increment_epoch();
        app.increment_epoch();
        assert!(app.settled().is_some());
        assert!(app.is_running());
    }

    #[test]
    fn opening_a_workspace_keeps_the_pause_preference_but_not_the_run() {
        let (mut app, _) = blinker_board();
        app.set_pause_when_settled(true);
        let workspace = app.workspace().unwrap();
        app.increment_epoch();
        app.increment_epoch();
        app.restore_workspace(workspace).unwrap();
        assert!(app.pause_when_settled());
        assert!(!app.can_reset_to_start());
        assert_eq!(app.settled(), None);
    }

    #[test]
    fn configured_randomization_repeats_and_remains_one_undoable_edit() {
        let mut app = App::new(32, 24);
        app.populate_board(5, 6);
        app.set_rule(Rule::parse("B36/S23").unwrap());
        app.epoch = 123;
        let original = app.board.snapshot_cells();
        let settings = RandomizeSettings::new(70, Some(u64::MAX)).unwrap();
        app.set_randomize_settings(settings);
        app.randomize_board();
        let randomized = app.board.snapshot_cells();
        assert!(app.undo());
        assert_eq!(app.board.snapshot_cells(), original);
        assert_eq!(app.epoch, 123);
        assert_eq!(app.rule_label(), "B36/S23");
        assert_eq!(app.randomize_settings(), settings);
        assert!(app.redo());
        assert_eq!(app.board.snapshot_cells(), randomized);
        app.randomize_board();
        assert_eq!(app.board.snapshot_cells(), randomized);
    }

    #[test]
    fn imports_beyond_the_library_limits_leave_the_library_and_rule_unchanged() {
        use crate::patterns::{MAX_LIBRARY_PATTERNS, MAX_PATTERN_NAME_BYTES, MAX_PATTERN_SIDE};
        let mut app = App::new(16, 16);
        for index in 0..MAX_LIBRARY_PATTERNS {
            app.import_rle_pattern(format!("dot {index}"), "x = 1, y = 1\no!")
                .unwrap();
        }
        let (count, revision) = (app.pattern_count(), app.library_revision());
        assert_eq!(
            app.import_rle_pattern("one too many", "x = 1, y = 1, rule = B36/S23\no!")
                .err(),
            Some(PatternParseError::LibraryFull)
        );
        assert_eq!(
            (app.pattern_count(), app.library_revision()),
            (count, revision)
        );
        assert_eq!(app.rule_label(), "B3/S23");

        // A maximum-size pattern uses the whole cell budget.
        let mut app = App::new(16, 16);
        let side = MAX_PATTERN_SIDE;
        let full = format!(
            "x = {side}, y = {side}\n{}{side}o!",
            format!("{side}o$").repeat(side - 1)
        );
        app.import_rle_pattern("full", &full).unwrap();
        assert_eq!(
            app.import_rle_pattern("dot", "x = 1, y = 1\no!").err(),
            Some(PatternParseError::LibraryFull)
        );
        assert_eq!(
            app.import_rle_pattern("n".repeat(MAX_PATTERN_NAME_BYTES + 1), "x = 1, y = 1\no!")
                .err(),
            Some(PatternParseError::NameTooLong)
        );
        assert_eq!(app.pattern_count(), Pattern::builtin_count() + 1);
    }

    #[test]
    fn setting_the_rule_is_an_undoable_edit() {
        let mut app = App::new(8, 8);
        app.populate_board(2, 2);
        let seeds = Rule::parse("B2/S").unwrap();
        assert!(app.set_rule(seeds));
        assert!(!app.set_rule(seeds));
        assert_eq!(app.board.rule(), seeds);
        assert!(app.undo());
        assert_eq!(app.board.rule(), Rule::LIFE);
        // Undoing the rule leaves the earlier drawing in place.
        assert!(app.board.is_alive(2, 2));
        assert!(app.redo());
        assert_eq!(app.board.rule(), seeds);
    }

    #[test]
    fn board_export_crops_to_the_live_cells_and_reads_back() {
        let mut app = App::new(32, 24);
        assert_eq!(app.board_to_rle(), None);

        let cells = [(10, 5), (11, 6), (9, 7), (10, 7), (11, 7), (20, 15)];
        for (x, y) in cells {
            app.populate_board(x, y);
        }
        app.set_rule(Rule::parse("B36/S23").unwrap());
        let export = app.board_to_rle().unwrap();
        assert_eq!((export.cells, export.width, export.height), (6, 12, 11));

        let pattern = Pattern::from_rle("export", &export.rle).unwrap();
        assert_eq!(pattern.rule, Some(app.board.rule()));
        let mut read_back: Vec<_> = pattern.cells.iter().map(|&(x, y)| (x + 9, y + 5)).collect();
        read_back.sort_unstable();
        let mut expected = cells.to_vec();
        expected.sort_unstable();
        assert_eq!(read_back, expected);
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
    fn population_change_describes_the_latest_generation_until_an_edit() {
        let mut app = App::new(8, 8);
        assert_eq!(app.population_change(), None);
        // A blinker keeps its three cells; a lone cell dies.
        for col in 1..=3 {
            app.populate_board(col, 4);
        }
        app.increment_epoch();
        assert_eq!(app.population_change(), Some(0));
        app.populate_board(6, 6);
        assert_eq!(app.population_change(), None, "an edit is not a generation");
        app.increment_epoch();
        assert_eq!(app.population_change(), Some(-1));
        app.reset_epoch();
        assert_eq!(app.population_change(), None);
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
}
