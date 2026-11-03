use crate::{
    board::{self, BoundaryMode, RandomizeSettings, Rule},
    patterns::{Pattern, PatternParseError},
    settle::{SettleWatch, Settled},
    shared::BOARD_SIZE_MAX,
    sparse_board::{CellRect, PLANE_CHUNKS_MAX, PLANE_EXPORT_CELLS_MAX, UnboundedRuleError},
    universe::{CellSnapshot, Universe, board_from_plane, plane_from_board, plane_to_rle},
};
use std::{collections::VecDeque, fmt};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum EditTool {
    #[default]
    Draw,
    Erase,
    Stamp,
    Select,
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

/// Why clipboard text can't become the stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteError {
    Parse(PatternParseError),
    Empty,
}

impl fmt::Display for PasteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Ordinary text has no header with the pattern's size.
            Self::Parse(PatternParseError::MissingHeader | PatternParseError::MissingDimension) => {
                f.write_str("The clipboard holds no RLE pattern")
            }
            Self::Parse(error) => write!(f, "Could not paste the clipboard's RLE: {error}"),
            Self::Empty => f.write_str("The pattern on the clipboard has no live cells"),
        }
    }
}

/// Why playback stopped by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoPause {
    Settled(Settled),
    /// The unbounded board reached its memory budget.
    BoardFull,
}

impl fmt::Display for AutoPause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Settled(settled) => settled.fmt(f),
            Self::BoardFull => f.write_str("the board reached its memory limit"),
        }
    }
}

#[derive(Debug)]
struct EditSnapshot {
    cells: CellSnapshot,
    epoch: usize,
    rule: Rule,
}

/// Generations of population kept for the status bar's line.
pub const POPULATION_HISTORY: usize = 256;

const EDIT_HISTORY_MAX: usize = 32;
const EDIT_HISTORY_BYTE_BUDGET: usize = 16 * 1024 * 1024;

/// Keeps a history within both the edit-count cap and the byte budget, always
/// retaining at least one entry.
fn push_bounded(history: &mut VecDeque<EditSnapshot>, snapshot: EditSnapshot) {
    let limit =
        (EDIT_HISTORY_BYTE_BUDGET / snapshot.cells.bytes().max(1)).clamp(1, EDIT_HISTORY_MAX);
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

/// A rectangle picked with the Select tool: the cell a drag started on, the one
/// it reached, and every cell between them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Selection {
    anchor: (i64, i64),
    corner: (i64, i64),
    /// Coordinates on the plane mean something else on a finite board.
    unbounded: bool,
}

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
    /// A stamp pasted from the clipboard, kept out of the library. While set,
    /// it is the stamp instead of `selected_pattern`.
    pasted: Option<Pattern>,
    /// Counts pastes, so a view of the stamp can tell one paste from the next.
    paste_revision: u64,
    /// transform applied to stamp before placement
    stamp_transform: StampTransform,
    /// available stamp patterns
    patterns: Vec<Pattern>,
    library_revision: u64,
    randomize_settings: RandomizeSettings,
    tool: EditTool,
    /// The palette was hidden by hand, while stamping, to reach the cells under it.
    palette_hidden: bool,
    undo_history: VecDeque<EditSnapshot>,
    redo_history: VecDeque<EditSnapshot>,
    stroke_active: bool,
    stroke_saved: bool,
    selection: Option<Selection>,
    /// The latest generation, with the live cells before and after it.
    last_step: Option<(usize, usize, usize)>,
    /// The board before the first generation since the latest edit.
    run_start: Option<EditSnapshot>,
    settle: SettleWatch,
    pause_when_settled: bool,
    /// Why playback paused itself, until the UI has said so.
    auto_paused: Option<AutoPause>,
    /// Live cells per generation of this run, oldest first.
    population_history: VecDeque<usize>,

    /// The size the board has, or has to return to from the unbounded board.
    finite_size: (usize, usize),
    /// Chunks of the unbounded board that stop a run.
    chunk_budget: usize,
    /// The part of the unbounded board on screen.
    view_cells: Option<CellRect>,

    pub board: Universe,
}

impl App {
    const TICK_RATE_MIN_MS: u64 = 25;
    const TICK_RATE_MAX_MS: u64 = 1000;
    const TICK_RATE_STEP_MS: u64 = 25;

    /// Constructs a new instance of [`App`].
    pub fn new(cols: usize, rows: usize) -> Self {
        let board = Universe::Finite(board::Board::new(cols, rows));
        Self {
            should_quit: false,
            epoch: 0,
            tick_rate_ms: 25,
            max_speed: false,
            running: false,
            needs_redraw: true,
            show_help: false,
            selected_pattern: None,
            pasted: None,
            paste_revision: 0,
            stamp_transform: StampTransform::default(),
            patterns: Pattern::builtins(),
            library_revision: 0,
            randomize_settings: RandomizeSettings::default(),
            tool: EditTool::Draw,
            palette_hidden: false,
            undo_history: VecDeque::new(),
            redo_history: VecDeque::new(),
            stroke_active: false,
            stroke_saved: false,
            selection: None,
            last_step: None,
            run_start: None,
            settle: SettleWatch::default(),
            pause_when_settled: false,
            auto_paused: None,
            population_history: VecDeque::with_capacity(POPULATION_HISTORY),
            finite_size: (cols, rows),
            chunk_budget: PLANE_CHUNKS_MAX,
            view_cells: None,
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

    /// Steps through wrapping, fixed, and unbounded edges. A rule that can't run
    /// unbounded goes straight back to wrapping.
    pub fn toggle_boundary_mode(&mut self) {
        match self.board.boundary_mode() {
            Some(BoundaryMode::Wrap) => self.set_boundary_mode(BoundaryMode::Fixed),
            Some(BoundaryMode::Fixed) => {
                if self.set_unbounded().is_err() {
                    self.set_boundary_mode(BoundaryMode::Wrap);
                }
            }
            None => self.set_boundary_mode(BoundaryMode::Wrap),
        }
    }

    /// Choosing edges on the unbounded board returns to a finite one, keeping
    /// only the cells that fit it. Undo brings the rest back.
    pub fn set_boundary_mode(&mut self, mode: BoundaryMode) {
        if let Universe::Unbounded(plane) = &self.board {
            let (cols, rows) = self.finite_size;
            let board = board_from_plane(plane, cols, rows, mode);
            self.replace_board(Universe::Finite(board));
            return;
        }
        if Some(mode) != self.board.boundary_mode() {
            self.settle.reset();
        }
        self.board.set_boundary_mode(mode);
        self.needs_redraw = true;
    }

    /// Moves the board onto the unbounded plane, which can't hold a B0 rule.
    /// Returns whether it moved.
    pub fn set_unbounded(&mut self) -> Result<bool, UnboundedRuleError> {
        let Universe::Finite(board) = &self.board else {
            return Ok(false);
        };
        let plane = plane_from_board(board)?;
        self.replace_board(Universe::Unbounded(plane));
        Ok(true)
    }

    /// An edit like any other, since a snapshot brings back the board that took it.
    fn replace_board(&mut self, board: Universe) {
        self.end_stroke();
        self.save_edit();
        // Cells move between the board and the plane, so the selection would not
        // cover the same ones.
        self.clear_selection();
        self.board = board;
        self.needs_redraw = true;
    }

    /// The board's size, or the one it will have on leaving the unbounded board.
    pub fn finite_size(&self) -> (usize, usize) {
        self.finite_size
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

    /// Whether the unbounded board has outgrown what a run may use. Anything that
    /// frees chunks lets playback go on.
    fn board_is_full(&self) -> bool {
        matches!(&self.board, Universe::Unbounded(plane) if plane.chunk_count() >= self.chunk_budget)
    }

    pub fn increment_epoch(&mut self) {
        if self.board_is_full() {
            self.running = false;
            self.auto_paused = Some(AutoPause::BoardFull);
            return;
        }
        if let Some(res) = self.epoch.checked_add(1) {
            if self.run_start.is_none() {
                self.run_start = Some(self.current_snapshot());
            }
            self.epoch = res;
            let before = self.board.live_cells_count();
            let fingerprint = self.board.fingerprint();
            self.board.step();
            self.last_step = Some((res, before, self.board.live_cells_count()));
            if self.population_history.is_empty() {
                self.population_history.push_back(before);
            }
            if self.population_history.len() == POPULATION_HISTORY {
                self.population_history.pop_front();
            }
            self.population_history
                .push_back(self.board.live_cells_count());
            let settled = self
                .settle
                .record(fingerprint, self.board.fingerprint(), res);
            if self.running && self.pause_when_settled && settled.is_some() {
                self.running = false;
                self.auto_paused = settled.map(AutoPause::Settled);
            }
            self.needs_redraw = true;
        }
    }

    /// Live cells over the latest generations of this run, oldest first.
    pub fn population_history(&self) -> &VecDeque<usize> {
        &self.population_history
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
    pub fn take_auto_paused(&mut self) -> Option<AutoPause> {
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

    #[cfg(test)]
    pub fn set_chunk_budget(&mut self, chunks: usize) {
        self.chunk_budget = chunks;
    }

    /// Tells the app what the canvas shows of the unbounded board.
    pub fn set_view_cells(&mut self, view: CellRect) {
        self.view_cells = Some(view);
    }

    /// What randomizing fills on the unbounded board, which has no size of its
    /// own: as much of the view as a board could hold, or a board's worth around
    /// the origin if nothing has been drawn yet. `None` on a finite board.
    pub fn randomize_area(&self) -> Option<CellRect> {
        self.board.finite().is_none().then(|| {
            let (cols, rows) = self.finite_size;
            self.view_cells
                .unwrap_or_else(|| CellRect::around_origin(cols, rows))
                .middle(BOARD_SIZE_MAX)
        })
    }

    /// Replaces every cell of a finite board. The unbounded board has too many,
    /// so the cells outside [`randomize_area`](Self::randomize_area) stay.
    pub fn randomize_board(&mut self) {
        self.end_stroke();
        self.save_edit();
        let area = self.randomize_area();
        match &mut self.board {
            Universe::Finite(board) => board.populate_random_with(self.randomize_settings),
            Universe::Unbounded(plane) => {
                if let Some(area) = area {
                    plane.replace_rect(area, self.randomize_settings.draws());
                }
            }
        }
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

    pub fn populate_board(&mut self, col: i64, row: i64) {
        if !self.board.contains(col, row) || self.board.is_alive(col, row) {
            return;
        }
        self.save_edit();
        if self.board.populate(col, row) {
            self.needs_redraw = true;
        }
    }

    pub fn depopulate_board(&mut self, col: i64, row: i64) {
        if !self.board.is_alive(col, row) {
            return;
        }
        self.save_edit();
        if self.board.depopulate(col, row) {
            self.needs_redraw = true;
        }
    }

    pub fn resize_board(&mut self, new_cols: usize, new_rows: usize) {
        self.finite_size = (new_cols, new_rows);
        match &mut self.board {
            Universe::Finite(board) => {
                if board.resize(new_cols, new_rows) {
                    // Snapshots hold cells in the old layout, so they no longer fit.
                    self.undo_history.clear();
                    self.redo_history.clear();
                    self.forget_run();
                    self.end_stroke();
                    self.needs_redraw = true;
                }
            }
            // The plane has no size to change, only finite boards in its history.
            Universe::Unbounded(_) => {
                self.undo_history.retain(|edit| edit.cells.is_unbounded());
                self.redo_history.retain(|edit| edit.cells.is_unbounded());
            }
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
        self.pasted = None;
        self.tool = EditTool::Stamp;
        self.selection = None;
        self.palette_hidden = false;
        self.needs_redraw = true;
        Ok(PatternImport { applied_rule })
    }

    pub fn rule_label(&self) -> String {
        self.board.rule().to_string()
    }

    /// Changes the board rule as an undoable edit, so undo can return to the old
    /// rule. Returns whether the rule actually changed.
    pub fn set_rule(&mut self, rule: Rule) -> bool {
        if rule == self.board.rule() || !self.board.allows_rule(rule) {
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
        restored.board = decoded.board;
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
        let live: Vec<(usize, usize)> = (0..rows)
            .flat_map(|y| (0..cols).map(move |x| (x, y)))
            .filter(|&(x, y)| board.is_alive(x, y))
            .collect();
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

    pub fn clear_selected_pattern(&mut self) {
        self.tool = EditTool::Draw;
        self.selection = None;
        self.palette_hidden = false;
        if self.selected_pattern.take().is_some() | self.pasted.take().is_some() {
            self.needs_redraw = true;
        }
    }

    /// The library pattern that is the stamp. `None` while a paste is.
    pub fn selected_pattern_index(&self) -> Option<usize> {
        self.selected_pattern
            .filter(|&idx| self.pasted.is_none() && idx < self.patterns.len())
    }

    /// The pattern a click places: a paste, or else the library's choice.
    fn stamp_pattern(&self) -> Option<&Pattern> {
        self.pasted
            .as_ref()
            .or_else(|| self.selected_pattern.and_then(|idx| self.patterns.get(idx)))
    }

    /// Makes RLE, such as a copied selection, the stamp, without adding it to
    /// the library. It starts unrotated, as it was copied, and the rule in its
    /// header is left alone: a paste brings cells, not a rule.
    pub fn paste_rle(&mut self, source: &str) -> Result<&Pattern, PasteError> {
        let pattern = Pattern::from_named_rle("Pasted", source).map_err(PasteError::Parse)?;
        if pattern.cells.is_empty() {
            return Err(PasteError::Empty);
        }
        self.pasted = Some(pattern);
        self.paste_revision = self.paste_revision.wrapping_add(1);
        self.stamp_transform = StampTransform::default();
        self.set_tool(EditTool::Stamp);
        Ok(self.pasted.as_ref().expect("just pasted"))
    }

    /// Changes whenever a paste replaces the stamp.
    pub fn paste_revision(&self) -> u64 {
        self.paste_revision
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
            || (self.selected_pattern_index() == Some(index) && self.tool == EditTool::Stamp)
        {
            return false;
        }

        self.selected_pattern = Some(index);
        self.pasted = None;
        self.tool = EditTool::Stamp;
        self.selection = None;
        self.needs_redraw = true;
        true
    }

    pub fn has_selected_pattern(&self) -> bool {
        self.tool == EditTool::Stamp && self.stamp_pattern().is_some()
    }

    pub fn selected_pattern_name(&self) -> Option<&str> {
        self.stamp_pattern().map(|pattern| pattern.name.as_str())
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
        self.stamp_pattern()
            .map(|pattern| self.transform_pattern(&pattern.cells, pattern.width, pattern.height))
    }

    pub fn place_selected_pattern_at(&mut self, col: i64, row: i64) {
        let Some((pattern, width, height)) = self
            .stamp_pattern()
            .map(|pattern| self.transform_pattern(&pattern.cells, pattern.width, pattern.height))
        else {
            return;
        };
        self.place_pattern_at(&pattern, width, height, col, row);
    }

    fn place_pattern_at(
        &mut self,
        pattern: &[(usize, usize)],
        width: usize,
        height: usize,
        center_col: i64,
        center_row: i64,
    ) {
        let (origin_x, origin_y) = self
            .board
            .stamp_origin((center_col, center_row), (width, height));
        let changes_board = pattern.iter().any(|&(dx, dy)| {
            let (col, row) = (origin_x + dx as i64, origin_y + dy as i64);
            self.board.contains(col, row) && !self.board.is_alive(col, row)
        });
        if !changes_board {
            return;
        }
        self.end_stroke();
        self.save_edit();
        let mut changed = false;

        for &(dx, dy) in pattern {
            changed |= self
                .board
                .populate(origin_x + dx as i64, origin_y + dy as i64);
        }

        if changed {
            self.needs_redraw = true;
        }
    }

    pub fn tool(&self) -> EditTool {
        self.tool
    }

    /// Choosing a tool, even the one in use, shows the palette again if it was
    /// hidden: it is how a pointer brings the palette back.
    pub fn set_tool(&mut self, tool: EditTool) {
        self.end_stroke();
        if tool != EditTool::Select {
            self.clear_selection();
        }
        self.tool = tool;
        self.palette_hidden = false;
        if tool == EditTool::Stamp && self.stamp_pattern().is_none() && !self.patterns.is_empty() {
            self.selected_pattern = Some(0);
        }
        self.needs_redraw = true;
    }

    /// The palette shows while stamping, unless hidden by hand.
    pub fn palette_shown(&self) -> bool {
        self.tool == EditTool::Stamp && !self.palette_hidden
    }

    /// Only stamping has a palette to show or hide.
    pub fn toggle_palette(&mut self) {
        if self.tool == EditTool::Stamp {
            self.palette_hidden = !self.palette_hidden;
        }
    }

    pub fn pattern_at(&self, index: usize) -> Option<&Pattern> {
        self.patterns.get(index)
    }

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
        self.population_history.clear();
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
        let (cols, rows) = app.finite_size();
        app.place_selected_pattern_at(cols as i64 / 2, rows as i64 / 2);
    }

    fn unbounded_app() -> App {
        let mut app = App::new(16, 16);
        assert_eq!(app.set_unbounded(), Ok(true));
        app
    }

    /// A board with a blinker, and its cells before any generation has run.
    fn blinker_board() -> (App, CellSnapshot) {
        let mut app = App::new(16, 16);
        stamp_centered(&mut app, PatternKind::Blinker);
        let cells = app.board.snapshot_cells();
        (app, cells)
    }

    #[test]
    fn the_unbounded_board_edits_and_runs_past_where_a_finite_one_stops() {
        let mut app = unbounded_app();
        assert_eq!(app.board.size(), None);
        for x in -1..=1 {
            app.populate_board(x - 5000, -5000);
        }
        assert_eq!(app.board.live_cells_count(), 3);

        app.increment_epoch();
        assert!(app.board.is_alive(-5000, -5001));
        assert!(app.board.is_alive(-5000, -4999));
        assert_eq!(app.population_change(), Some(0));

        app.depopulate_board(-5000, -5000);
        assert_eq!(app.board.live_cells_count(), 2);
    }

    #[test]
    fn undo_and_reset_work_on_the_unbounded_board() {
        let mut app = unbounded_app();
        app.populate_board(-3, -3);
        let drawn = app.board.snapshot_cells();
        app.populate_board(-4, -3);
        assert!(app.undo());
        assert_eq!(app.board.snapshot_cells(), drawn);
        assert!(app.redo());
        assert_eq!(app.board.live_cells_count(), 2);

        let edited = app.board.snapshot_cells();
        app.increment_epoch();
        assert_eq!(app.board.live_cells_count(), 0);
        assert!(app.reset_to_start());
        assert_eq!(app.board.snapshot_cells(), edited);
    }

    #[test]
    fn stamps_can_land_past_the_origin_only_on_the_unbounded_board() {
        let mut app = unbounded_app();
        app.select_pattern(PatternKind::Blinker);
        app.place_selected_pattern_at(-40, -40);
        assert_eq!(app.board.live_cells_count(), 3);

        // The same cursor is off a finite board, where stamps start at its edge.
        let mut app = App::new(16, 16);
        app.select_pattern(PatternKind::Blinker);
        app.place_selected_pattern_at(-40, -40);
        assert_eq!(app.board.live_cells_count(), 3);
        assert!(app.board.is_alive(0, 0));
    }

    #[test]
    fn a_finite_board_ignores_edits_off_its_edges() {
        let mut app = App::new(16, 16);
        app.populate_board(-1, 0);
        app.populate_board(0, 16);
        app.depopulate_board(-1, 0);
        assert_eq!(app.board.live_cells_count(), 0);
        assert!(!app.can_undo());
    }

    #[test]
    fn a_blinker_settles_on_the_unbounded_board_and_a_glider_does_not() {
        let mut app = unbounded_app();
        stamp_centered(&mut app, PatternKind::Blinker);
        for _ in 0..4 {
            app.increment_epoch();
        }
        assert!(app.settled().is_some());

        let mut app = unbounded_app();
        stamp_centered(&mut app, PatternKind::Glider);
        for _ in 0..200 {
            app.increment_epoch();
        }
        assert_eq!(app.settled(), None, "it never returns to the same cells");
    }

    #[test]
    fn b0_rules_stay_off_the_unbounded_board_without_touching_history() {
        let b0 = Rule::parse("B0/S").unwrap();
        let mut app = unbounded_app();
        assert!(!app.set_rule(b0));
        assert_eq!(app.board.rule(), Rule::LIFE);
        assert!(app.undo());
        assert_eq!(app.board.size(), Some((16, 16)), "the refusal left no edit");

        // A finite board may hold one, and then can't move to the plane.
        let mut app = App::new(16, 16);
        assert!(app.set_rule(b0));
        assert_eq!(app.set_unbounded(), Err(UnboundedRuleError));
        assert_eq!(app.board.size(), Some((16, 16)));
        assert!(app.can_undo(), "a refused move forgets nothing");
    }

    #[test]
    fn moving_onto_the_plane_keeps_the_cells_and_the_run_counter() {
        let mut app = App::new(16, 16);
        app.populate_board(8, 8);
        app.populate_board(0, 0);
        app.increment_epoch();
        app.populate_board(8, 8);
        let epoch = app.epoch;

        assert_eq!(app.set_unbounded(), Ok(true));
        assert_eq!(app.set_unbounded(), Ok(false), "it is already there");
        assert!(app.board.is_alive(0, 0));
        assert_eq!(app.board.live_cells_count(), 1);
        assert_eq!(app.epoch, epoch);
        assert!(!app.can_reset_to_start());
        assert!(app.take_redraw());
    }

    #[test]
    fn switching_kinds_of_board_can_be_undone_and_redone() {
        let mut app = App::new(16, 16);
        app.set_boundary_mode(BoundaryMode::Fixed);
        app.populate_board(8, 8);
        let finite = app.board.snapshot_cells();

        app.set_unbounded().unwrap();
        app.populate_board(500, 500);
        let plane = app.board.snapshot_cells();

        // Leaving drops the far cell, and undo has to bring it back.
        app.set_boundary_mode(BoundaryMode::Wrap);
        assert_eq!(app.board.live_cells_count(), 1);
        assert!(app.undo());
        assert_eq!(app.board.snapshot_cells(), plane);
        assert!(app.board.is_alive(500, 500));

        assert!(app.undo(), "the cell drawn on the plane");
        assert!(app.undo(), "the move onto the plane");
        assert_eq!(app.board.snapshot_cells(), finite);
        assert_eq!(app.board.boundary_mode(), Some(BoundaryMode::Fixed));

        assert!(app.redo());
        assert_eq!(app.board.size(), None);
        assert!(app.redo());
        assert_eq!(app.board.snapshot_cells(), plane);
    }

    #[test]
    fn undo_across_kinds_of_board_keeps_the_rule() {
        let highlife = Rule::parse("B36/S23").unwrap();
        let mut app = App::new(16, 16);
        app.set_rule(highlife);
        app.set_unbounded().unwrap();
        assert_eq!(app.board.rule(), highlife);

        assert!(app.undo());
        assert_eq!(app.board.size(), Some((16, 16)));
        assert_eq!(app.board.rule(), highlife);
        assert!(app.redo());
        assert_eq!(app.board.size(), None);
        assert_eq!(app.board.rule(), highlife);
    }

    #[test]
    fn a_finite_rule_the_plane_refuses_comes_back_with_its_board() {
        let b0 = Rule::parse("B0/S").unwrap();
        let mut app = App::new(16, 16);
        // A rule the plane takes, swapped for B0 only after a snapshot of the
        // plane exists to undo onto and leave again.
        app.set_unbounded().unwrap();
        app.set_boundary_mode(BoundaryMode::Wrap);
        assert!(app.set_rule(b0));
        assert!(app.undo(), "the rule");
        assert!(app.undo(), "back onto the plane");
        assert_eq!(app.board.size(), None);
        assert!(app.redo(), "off the plane");
        assert!(app.redo(), "the rule again");
        assert_eq!(app.board.rule(), b0);
        assert_eq!(app.board.size(), Some((16, 16)));
    }

    #[test]
    fn choosing_edges_returns_to_a_finite_board_of_the_old_size() {
        let mut app = App::new(16, 8);
        assert_eq!(app.set_unbounded(), Ok(true));
        app.populate_board(0, 0);
        app.populate_board(100, 100);

        app.set_boundary_mode(BoundaryMode::Fixed);
        assert_eq!(app.board.size(), Some((16, 8)));
        assert_eq!(app.board.boundary_mode(), Some(BoundaryMode::Fixed));
        assert_eq!(app.board.live_cells_count(), 1);
        assert!(app.board.is_alive(8, 4));
    }

    #[test]
    fn the_edges_shortcut_steps_through_all_three_modes() {
        let mut app = App::new(16, 16);
        assert_eq!(app.board.boundary_mode(), Some(BoundaryMode::Wrap));
        app.toggle_boundary_mode();
        assert_eq!(app.board.boundary_mode(), Some(BoundaryMode::Fixed));
        app.toggle_boundary_mode();
        assert_eq!(app.board.boundary_mode(), None);
        assert_eq!(app.boundary_mode_label(), "Unbounded");
        app.toggle_boundary_mode();
        assert_eq!(app.board.boundary_mode(), Some(BoundaryMode::Wrap));
    }

    #[test]
    fn the_edges_shortcut_skips_unbounded_under_a_b0_rule() {
        let mut app = App::new(16, 16);
        app.set_rule(Rule::parse("B0/S").unwrap());
        app.toggle_boundary_mode();
        app.toggle_boundary_mode();
        assert_eq!(app.board.boundary_mode(), Some(BoundaryMode::Wrap));
    }

    #[test]
    fn resizing_the_unbounded_board_only_changes_the_size_to_return_to() {
        let mut app = unbounded_app();
        app.populate_board(0, 0);
        let drawn = app.board.snapshot_cells();
        app.populate_board(1, 0);
        app.resize_board(32, 64);
        assert_eq!(app.finite_size(), (32, 64));
        assert_eq!(app.board.size(), None);
        assert!(app.undo(), "nothing about the plane changed");
        assert_eq!(app.board.snapshot_cells(), drawn);
        assert!(app.undo());
        assert_eq!(app.board.live_cells_count(), 0);

        // History ends there: before it is a board of a size no longer in use.
        assert!(!app.can_undo());
        assert_eq!(app.board.size(), None);

        app.set_boundary_mode(BoundaryMode::Wrap);
        assert_eq!(app.board.size(), Some((32, 64)));
    }

    /// Cells in chunks of their own, far enough apart to have nothing to do.
    fn scattered_app(chunks: i64) -> App {
        let mut app = unbounded_app();
        for chunk in 0..chunks {
            app.populate_board(chunk * 1000, 0);
        }
        app
    }

    #[test]
    fn a_full_unbounded_board_stops_the_run_until_something_frees_chunks() {
        let mut app = scattered_app(3);
        app.chunk_budget = 3;
        app.toggle_running();
        app.increment_epoch();
        assert_eq!(app.epoch, 0, "no generation ran");
        assert!(!app.is_running());
        assert_eq!(app.take_auto_paused(), Some(AutoPause::BoardFull));
        assert_eq!(app.take_auto_paused(), None);
        assert_eq!(
            AutoPause::BoardFull.to_string(),
            "the board reached its memory limit"
        );

        // Playing or stepping again can't get past it.
        app.toggle_running();
        app.increment_epoch();
        assert!(!app.is_running());
        app.increment_epoch();
        assert_eq!(app.epoch, 0);
        assert_eq!(app.take_auto_paused(), Some(AutoPause::BoardFull));

        app.depopulate_board(0, 0);
        app.increment_epoch();
        assert_eq!(app.epoch, 1);
        assert_eq!(app.take_auto_paused(), None);
    }

    #[test]
    fn a_board_under_budget_or_with_edges_keeps_running() {
        let mut app = scattered_app(2);
        app.chunk_budget = 3;
        app.toggle_running();
        app.increment_epoch();
        assert!(app.is_running());
        assert_eq!(app.epoch, 1);

        let mut app = App::new(16, 16);
        app.chunk_budget = 0;
        app.toggle_running();
        app.increment_epoch();
        assert!(app.is_running());
        assert_eq!(app.epoch, 1);
    }

    #[test]
    fn randomizing_the_plane_fills_the_view_and_leaves_the_rest() {
        let mut app = unbounded_app();
        app.set_randomize_settings(RandomizeSettings::new(100, None).unwrap());
        app.populate_board(500, 500);
        let view = CellRect {
            left: -5,
            bottom: 100,
            width: 70,
            height: 3,
        };
        app.set_view_cells(view);
        assert_eq!(app.randomize_area(), Some(view));
        app.randomize_board();
        assert_eq!(app.board.live_cells_count(), 70 * 3 + 1);
        assert!(app.board.is_alive(-5, 100) && app.board.is_alive(64, 102));
        assert!(!app.board.is_alive(-6, 100) && !app.board.is_alive(65, 102));
        assert!(app.board.is_alive(500, 500), "left outside the view alone");

        // What was there goes, as on a finite board.
        app.set_randomize_settings(RandomizeSettings::new(0, None).unwrap());
        app.randomize_board();
        assert_eq!(app.board.live_cells_count(), 1);

        assert!(app.undo());
        assert_eq!(app.board.live_cells_count(), 70 * 3 + 1);
        assert!(app.undo());
        assert_eq!(app.board.live_cells_count(), 1);
    }

    #[test]
    fn a_seed_repeats_on_the_plane() {
        let randomized = |seed| {
            let mut app = unbounded_app();
            app.set_randomize_settings(RandomizeSettings::new(50, Some(seed)).unwrap());
            app.randomize_board();
            let mut cells: Vec<_> = match &app.board {
                Universe::Unbounded(plane) => plane.live_cells().collect(),
                Universe::Finite(_) => panic!("the board is unbounded"),
            };
            cells.sort_unstable();
            cells
        };
        assert_eq!(randomized(7), randomized(7));
        assert_ne!(randomized(7), randomized(8));
    }

    #[test]
    fn randomizing_starts_from_where_the_board_would_be_and_is_capped_at_a_board() {
        let mut app = App::new(32, 16);
        app.set_unbounded().unwrap();
        assert_eq!(app.randomize_area(), Some(CellRect::around_origin(32, 16)));

        // A view zoomed far out is bigger than anything worth filling.
        app.set_view_cells(CellRect {
            left: 1 << 40,
            bottom: -(1 << 30),
            width: 1 << 30,
            height: BOARD_SIZE_MAX - 2,
        });
        let area = app.randomize_area().unwrap();
        assert_eq!(
            (area.width, area.height),
            (BOARD_SIZE_MAX, BOARD_SIZE_MAX - 2)
        );
        assert_eq!(area.bottom, -(1 << 30));
        let middle = (1i64 << 40) + (1 << 29);
        assert_eq!(area.left, middle - BOARD_SIZE_MAX as i64 / 2);

        assert_eq!(App::new(16, 16).randomize_area(), None);
    }

    #[test]
    fn the_plane_exports_runs_of_any_extent_from_the_corner_of_its_cells() {
        let mut app = unbounded_app();
        assert_eq!(app.board_to_rle(), Err(BoardExportError::Empty));

        let far = 1i64 << 40;
        for (x, y) in [(far, -far), (far + 1, -far), (far + 1000, -far + 2)] {
            app.populate_board(x, y);
        }
        app.epoch = 9;
        let export = app.board_to_rle().unwrap();
        assert_eq!((export.cells, export.width, export.height), (3, 1001, 3));
        assert_eq!(
            export.rle,
            "#C Generation 9 of an unbounded board\n\
             x = 1001, y = 3, rule = B3/S23\n2o2$1000bo!\n"
        );
    }

    #[test]
    fn a_plane_too_big_to_write_out_says_so() {
        let mut app = unbounded_app();
        let Universe::Unbounded(plane) = &mut app.board else {
            panic!("the board is unbounded");
        };
        let side = 1 << 13;
        let area = CellRect::around_origin(side + 1, side);
        plane.replace_rect(area, || true);
        assert!(plane.live_cells_count() > PLANE_EXPORT_CELLS_MAX);
        assert_eq!(app.board_to_rle(), Err(BoardExportError::TooLarge));
        assert!(
            app.workspace()
                .unwrap_err()
                .to_string()
                .contains("67,108,864")
        );

        let Universe::Unbounded(plane) = &mut app.board else {
            panic!("the board is unbounded");
        };
        plane.clear();
        plane.replace_rect(CellRect::around_origin(side, side), || true);
        assert_eq!(plane.live_cells_count(), PLANE_EXPORT_CELLS_MAX);
        assert_eq!(app.board_to_rle().unwrap().cells, PLANE_EXPORT_CELLS_MAX);
    }

    #[test]
    fn a_stamp_with_nothing_new_on_the_board_is_not_an_edit() {
        let mut app = App::new(8, 8);
        app.select_pattern(PatternKind::Blinker);
        // Its third cell is off the board, and the two it can reach are alive.
        app.board.populate(6, 0);
        app.board.populate(7, 0);
        app.place_selected_pattern_at(7, 0);
        assert!(!app.can_undo());
    }

    #[test]
    fn the_palette_follows_the_stamp_tool_and_can_be_hidden_while_stamping() {
        let mut app = App::new(16, 16);
        assert!(!app.palette_shown());
        app.toggle_palette();
        app.set_tool(EditTool::Stamp);
        assert!(app.palette_shown(), "there was nothing to hide before");
        assert!(app.has_selected_pattern());

        // Hidden, it stays hidden through stamping and a change of stamp.
        app.toggle_palette();
        assert!(!app.palette_shown());
        app.place_selected_pattern_at(8, 8);
        app.rotate_stamp_clockwise();
        app.select_pattern_index(1);
        assert!(!app.palette_shown());
        app.toggle_palette();
        assert!(app.palette_shown());

        // Choosing Stamp again brings it back, as does importing a pattern.
        app.toggle_palette();
        app.set_tool(EditTool::Stamp);
        assert!(app.palette_shown());
        app.toggle_palette();
        app.import_rle_pattern("Dot", "x = 1, y = 1\no!").unwrap();
        assert!(app.palette_shown());

        // Leaving the tool closes it, and coming back opens it, however it was left.
        app.toggle_palette();
        app.clear_selected_pattern();
        assert!(!app.palette_shown());
        app.set_tool(EditTool::Stamp);
        assert!(app.palette_shown());
        app.toggle_palette();
        app.set_tool(EditTool::Draw);
        app.select_pattern_index(0);
        assert!(app.palette_shown());
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
    fn population_history_starts_with_the_run_and_keeps_the_latest_generations() {
        let (mut app, _) = blinker_board();
        assert!(app.population_history().is_empty());
        app.increment_epoch();
        assert_eq!(app.population_history().len(), 2);

        for _ in 0..POPULATION_HISTORY * 2 {
            app.increment_epoch();
        }
        assert_eq!(app.population_history().len(), POPULATION_HISTORY);
        assert_eq!(
            app.population_history().back(),
            Some(&app.board.live_cells_count())
        );

        app.populate_board(1, 1);
        assert!(app.population_history().is_empty());
        app.increment_epoch();
        assert!(app.reset_to_start());
        assert!(app.population_history().is_empty());
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
        app.set_boundary_mode(app.board.boundary_mode().unwrap());
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
        assert_eq!(
            app.take_auto_paused(),
            app.settled().map(AutoPause::Settled)
        );
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
        assert_eq!(app.board_to_rle(), Err(BoardExportError::Empty));

        let cells = [(10, 5), (11, 6), (9, 7), (10, 7), (11, 7), (20, 15)];
        for (x, y) in cells {
            app.populate_board(x, y);
        }
        app.set_rule(Rule::parse("B36/S23").unwrap());
        let export = app.board_to_rle().unwrap();
        assert_eq!((export.cells, export.width, export.height), (6, 12, 11));

        let pattern = Pattern::from_rle("export", &export.rle).unwrap();
        assert_eq!(pattern.rule, Some(app.board.rule()));
        let mut read_back: Vec<_> = pattern
            .cells
            .iter()
            .map(|&(x, y)| (x as i64 + 9, y as i64 + 5))
            .collect();
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
        assert_eq!(
            app.board.finite_mut().unwrap().take_dirty_cells(),
            board::DirtyCells::Full
        );
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
        assert_eq!(app.boundary_mode_label(), "Unbounded");
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

    fn selecting(app: &mut App, from: (i64, i64), to: (i64, i64)) {
        app.set_tool(EditTool::Select);
        app.begin_selection(from.0, from.1);
        app.extend_selection(to.0, to.1);
    }

    #[test]
    fn a_selection_spans_its_corners_whichever_way_it_was_dragged() {
        let mut app = App::new(16, 16);
        selecting(&mut app, (9, 2), (4, 7));
        let expected = CellRect {
            left: 4,
            bottom: 2,
            width: 6,
            height: 6,
        };
        assert_eq!(app.selection(), Some(expected));

        // Only the Select tool shows one, and leaving it drops it.
        app.set_tool(EditTool::Draw);
        assert_eq!(app.selection(), None);
        app.set_tool(EditTool::Select);
        assert_eq!(app.selection(), None);
    }

    #[test]
    fn a_selection_stays_within_a_board_that_shrank() {
        let mut app = App::new(16, 16);
        selecting(&mut app, (4, 4), (14, 14));
        app.resize_board(10, 12);
        assert_eq!(
            app.selection(),
            Some(CellRect {
                left: 4,
                bottom: 4,
                width: 6,
                height: 8,
            })
        );
        app.resize_board(8, 8);
        selecting(&mut app, (9, 9), (12, 12));
        assert_eq!(app.selection(), None);
    }

    #[test]
    fn erasing_a_selection_kills_only_its_cells_as_one_edit() {
        for mut app in [App::new(16, 16), unbounded_app()] {
            for cell in [(2, 2), (3, 3), (4, 4), (8, 8)] {
                app.populate_board(cell.0, cell.1);
            }
            let before = app.board.snapshot_cells();
            selecting(&mut app, (3, 3), (8, 4));
            assert_eq!(app.erase_selection(), 2);
            assert!(app.board.is_alive(2, 2) && app.board.is_alive(8, 8));
            assert!(!app.board.is_alive(3, 3) && !app.board.is_alive(4, 4));
            // Nothing left to erase is no edit.
            assert_eq!(app.erase_selection(), 0);
            assert!(app.undo());
            assert_eq!(app.board.snapshot_cells(), before);
        }
    }

    #[test]
    fn a_selection_copies_as_rle_that_keeps_its_margins() {
        for mut app in [App::new(16, 16), unbounded_app()] {
            assert_eq!(
                app.selection_to_rle(),
                Err(SelectionExportError::NothingSelected)
            );
            selecting(&mut app, (5, 5), (9, 7));
            assert_eq!(app.selection_to_rle(), Err(SelectionExportError::Empty));
            // An L in the selection's upper right.
            for cell in [(7, 6), (7, 7), (8, 7)] {
                app.populate_board(cell.0, cell.1);
            }
            let export = app.selection_to_rle().unwrap();
            assert_eq!((export.cells, export.width, export.height), (3, 5, 3));
            let pattern = Pattern::from_rle("copy", &export.rle).unwrap();
            assert_eq!((pattern.width, pattern.height), (5, 3));
            let mut cells = pattern.cells.clone();
            cells.sort_unstable();
            assert_eq!(cells, vec![(2, 1), (2, 2), (3, 2)]);
            assert_eq!(pattern.rule, Some(app.board.rule()));
        }
    }

    #[test]
    fn selecting_the_live_cells_boxes_them_with_the_select_tool() {
        for mut app in [App::new(16, 16), unbounded_app()] {
            assert!(!app.select_live_cells());
            assert_eq!(app.tool(), EditTool::Draw);
            app.populate_board(3, 9);
            app.populate_board(6, 2);
            assert!(app.select_live_cells());
            assert_eq!(app.tool(), EditTool::Select);
            assert_eq!(
                app.selection(),
                Some(CellRect {
                    left: 3,
                    bottom: 2,
                    width: 4,
                    height: 8,
                })
            );
        }
    }

    #[test]
    fn moving_between_the_board_and_the_plane_drops_the_selection() {
        let mut app = App::new(16, 16);
        selecting(&mut app, (1, 1), (4, 4));
        assert_eq!(app.set_unbounded(), Ok(true));
        assert_eq!(app.selection(), None);

        // Undo brings the board back, but not a plane selection onto it.
        selecting(&mut app, (1, 1), (4, 4));
        assert!(app.undo());
        assert!(app.board.finite().is_some());
        assert_eq!(app.selection(), None);
    }

    #[test]
    fn a_copied_selection_pastes_as_a_stamp_that_places_the_same_cells() {
        for mut app in [App::new(32, 32), unbounded_app()] {
            // An L with an empty column on its left, kept by the copy.
            for cell in [(4, 4), (4, 5), (5, 5)] {
                app.populate_board(cell.0, cell.1);
            }
            selecting(&mut app, (3, 4), (5, 6));
            let rle = app.selection_to_rle().unwrap().rle;
            let library = app.pattern_count();
            app.rotate_stamp_clockwise();

            let pasted = app.paste_rle(&rle).unwrap();
            assert_eq!((pasted.width, pasted.height), (3, 3));
            assert_eq!(app.tool(), EditTool::Stamp);
            assert_eq!(app.selection(), None);
            assert!(app.has_selected_pattern());
            assert_eq!(app.selected_pattern_name(), Some("Pasted"));
            assert_eq!(app.stamp_orientation(), (0, false), "as it was copied");
            assert_eq!(app.selected_pattern_index(), None);
            assert_eq!(app.pattern_count(), library, "the library is unchanged");

            // Centred on (21, 21), the 3 × 3 stamp starts at (20, 20).
            app.place_selected_pattern_at(21, 21);
            for cell in [(21, 20), (21, 21), (22, 21)] {
                assert!(app.board.is_alive(cell.0, cell.1), "{cell:?}");
            }
            assert_eq!(app.board.live_cells_count(), 6);
        }
    }

    #[test]
    fn a_paste_stays_the_stamp_until_another_is_chosen_or_put_away() {
        let mut app = App::new(16, 16);
        app.paste_rle("x = 2, y = 1\n2o!").unwrap();
        let revision = app.paste_revision();

        // Drawing and coming back keeps the paste.
        app.set_tool(EditTool::Draw);
        assert!(!app.has_selected_pattern());
        app.set_tool(EditTool::Stamp);
        assert_eq!(app.selected_pattern_name(), Some("Pasted"));

        // A library pattern replaces it, and so does a later paste.
        app.select_pattern(PatternKind::Glider);
        assert_eq!(app.selected_pattern_name(), Some("Glider"));
        app.paste_rle("#N Domino\nx = 1, y = 2\no$o!").unwrap();
        assert_eq!(app.selected_pattern_name(), Some("Domino"));
        assert_ne!(app.paste_revision(), revision);
        // Choosing the library pattern again works while a paste is the stamp.
        assert!(app.select_pattern_index(PatternKind::Glider.builtin_index().unwrap()));

        app.paste_rle("x = 1, y = 1\no!").unwrap();
        app.clear_selected_pattern();
        assert!(!app.has_selected_pattern());
        assert_eq!(app.selected_pattern_name(), None);
    }

    #[test]
    fn a_paste_that_is_not_a_pattern_leaves_the_stamp_and_rule_alone() {
        let mut app = App::new(16, 16);
        app.select_pattern(PatternKind::Glider);
        for text in ["", "hello"] {
            let error = app.paste_rle(text).unwrap_err();
            assert_eq!(error.to_string(), "The clipboard holds no RLE pattern");
        }
        assert_eq!(
            app.paste_rle("x = 3, y = 3\n!").err(),
            Some(PasteError::Empty)
        );
        assert_eq!(app.selected_pattern_name(), Some("Glider"));

        let rule = app.board.rule();
        app.paste_rle("x = 1, y = 1, rule = B36/S23\no!").unwrap();
        assert_eq!(app.board.rule(), rule);
        assert!(!app.can_undo(), "pasting is no edit");
    }
}
