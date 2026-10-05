const CELL_ALIVE: u8 = 1;
/// Measured with `benches/dense_crossover.rs`. The dense scan overtakes the
/// frontier step at about 2% candidate coverage for young soup, but only at 5-6%
/// for settled debris on boards small enough to stay in cache. 5% keeps settled
/// boards no slower than the frontier alone, at the cost of up to roughly twice
/// the dense time for soup between 2% and 5%.
const FRONTIER_DENSE_THRESHOLD_PERCENT_DEFAULT: usize = 5;

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
pub enum RuleParseError {
    Empty,
    Malformed,
    InvalidDigit(char),
    NeighborOutOfRange(u32),
}

impl std::fmt::Display for RuleParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty rulestring"),
            Self::Malformed => write!(f, "rulestring must look like 'B3/S23'"),
            Self::InvalidDigit(ch) => write!(f, "invalid neighbor count '{ch}' in rulestring"),
            Self::NeighborOutOfRange(n) => write!(f, "neighbor count {n} exceeds the maximum of 8"),
        }
    }
}

impl std::error::Error for RuleParseError {}

/// Birth and survival neighbor counts, as written in `B3/S23` notation. Each is a
/// bitmask over the nine possible neighbor counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    birth: u16,
    survive: u16,
}

impl Rule {
    /// Conway's Game of Life.
    pub const LIFE: Self = Self {
        birth: 1 << 3,
        survive: (1 << 2) | (1 << 3),
    };

    /// Accepts `B3/S23`, the equivalent `S23/B3`, and the older `23/3`
    /// survival-first ordering.
    pub fn parse(source: &str) -> Result<Self, RuleParseError> {
        let text = source.trim();
        if text.is_empty() {
            return Err(RuleParseError::Empty);
        }
        let Some((left, right)) = text.split_once('/') else {
            return Err(RuleParseError::Malformed);
        };
        let left = left.trim().to_ascii_lowercase();
        let right = right.trim().to_ascii_lowercase();

        let (birth, survive) = match (
            left.strip_prefix('b'),
            right.strip_prefix('s'),
            left.strip_prefix('s'),
            right.strip_prefix('b'),
        ) {
            (Some(birth), Some(survive), _, _) => (birth, survive),
            (_, _, Some(survive), Some(birth)) => (birth, survive),
            _ if left.chars().all(|c| c.is_ascii_digit())
                && right.chars().all(|c| c.is_ascii_digit()) =>
            {
                // Legacy ordering puts survival counts first, e.g. `23/3`.
                (right.as_str(), left.as_str())
            }
            _ => return Err(RuleParseError::Malformed),
        };

        Ok(Self {
            birth: counts_to_mask(birth)?,
            survive: counts_to_mask(survive)?,
        })
    }

    fn allows_birth(self, neighbors: u8) -> bool {
        self.birth & (1 << neighbors) != 0
    }

    fn allows_survival(self, neighbors: u8) -> bool {
        self.survive & (1 << neighbors) != 0
    }

    /// Birth counts in bits 0..=8 and survival counts in bits 9..=17, so the next
    /// state is bit `neighbors + 9 * alive` without branching on `alive`.
    fn transition_mask(self) -> u32 {
        u32::from(self.birth) | (u32::from(self.survive) << 9)
    }
}

impl Default for Rule {
    fn default() -> Self {
        Self::LIFE
    }
}

impl std::fmt::Display for Rule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "B")?;
        for n in 0..=8u8 {
            if self.allows_birth(n) {
                write!(f, "{n}")?;
            }
        }
        write!(f, "/S")?;
        for n in 0..=8u8 {
            if self.allows_survival(n) {
                write!(f, "{n}")?;
            }
        }
        Ok(())
    }
}

fn counts_to_mask(counts: &str) -> Result<u16, RuleParseError> {
    let mut mask = 0u16;
    for ch in counts.chars() {
        let digit = ch.to_digit(10).ok_or(RuleParseError::InvalidDigit(ch))?;
        if digit > 8 {
            return Err(RuleParseError::NeighborOutOfRange(digit));
        }
        mask |= 1 << digit;
    }
    Ok(mask)
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

fn count_live(cells: &[u8]) -> usize {
    cells.iter().filter(|&&cell| cell == CELL_ALIVE).count()
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
    /// recycled buffer for frontier candidate expansion
    candidate_scratch: Vec<usize>,
    /// recycled buffer for the next generation's frontier
    frontier_scratch: Vec<usize>,
    /// running count of live cells, kept in step with every mutation
    live_count: usize,
    /// board width in cells
    cols: usize,
    /// board height in cells
    rows: usize,
    /// board boundary behavior
    boundary_mode: BoundaryMode,
    /// birth/survival rule applied by each step
    rule: Rule,
    /// candidate share of the board, in percent, above which a step scans densely
    dense_threshold_percent: usize,
}

impl Board {
    pub(crate) fn snapshot_cells(&self) -> Vec<u8> {
        self.current.clone()
    }

    pub(crate) fn restore_cells(&mut self, cells: Vec<u8>) {
        assert_eq!(cells.len(), self.current.len());
        self.current = cells;
        self.live_count = count_live(&self.current);
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
            candidate_scratch: Vec::new(),
            frontier_scratch: Vec::new(),
            live_count: 0,
            cols,
            rows,
            boundary_mode: BoundaryMode::Wrap,
            rule: Rule::LIFE,
            dense_threshold_percent: frontier_dense_threshold_percent(),
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
        // Queued indices address the old layout, so they must go before the flag
        // arrays are replaced; mark_*_full below clears flags by index, not by fill.
        self.dirty_cells.clear();
        self.frontier_cells.clear();
        self.dirty_flags = vec![0; new_len];
        self.frontier_flags = vec![0; new_len];
        self.candidate_flags = vec![0; new_len];
        self.candidate_scratch.clear();
        self.frontier_scratch.clear();
        self.live_count = count_live(&self.current);
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
            self.live_count += 1;
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
            self.live_count -= 1;
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

        // Roll each cell once. Picking `len / 4` coordinates with replacement
        // repeats cells, which landed the density nearer 22% than the intended 25%.
        for cell in &mut self.current {
            if fastrand::u8(0..4) == 0 {
                *cell = CELL_ALIVE;
            }
        }
        self.live_count = count_live(&self.current);
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
            self.live_count = 0;
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
        let mut next_frontier = std::mem::take(&mut self.frontier_scratch);
        next_frontier.clear();

        // Borrow the fields, not `self`, so `future` can be written while
        // `current` is read.
        let view = CellsView {
            cells: &self.current,
            cols: self.cols,
            rows: self.rows,
            boundary_mode: self.boundary_mode,
            rule: self.rule,
        };
        let live_count = view.step_rows(0, &mut self.future, &mut next_frontier);
        for &idx in &next_frontier {
            self.mark_dirty_cell(idx);
        }

        let changed = !next_frontier.is_empty();
        std::mem::swap(&mut self.current, &mut self.future);
        self.live_count = live_count;
        self.replace_frontier(next_frontier);
        changed
    }

    fn step_frontier(&mut self) -> bool {
        let dense_threshold = (self
            .current
            .len()
            .saturating_mul(self.dense_threshold_percent)
            / 100)
            .max(1);

        // Every frontier cell is its own candidate, so a frontier this large is
        // already past the threshold and expanding it would be thrown away.
        if self.frontier_cells.len() > dense_threshold {
            return self.step_dense();
        }

        let mut candidates = std::mem::take(&mut self.candidate_scratch);
        candidates.clear();
        let within_threshold = self.collect_frontier_candidates(&mut candidates, dense_threshold);
        if !within_threshold {
            for &idx in &candidates {
                self.candidate_flags[idx] = 0;
            }
            self.candidate_scratch = candidates;
            return self.step_dense();
        }

        // Evaluate every candidate against the untouched `current` and record only
        // the cells that flip. Staging the generation in `future` instead would
        // need a full-board copy, making a sparse step cost O(board area).
        let mut next_frontier = std::mem::take(&mut self.frontier_scratch);
        next_frontier.clear();
        let view = self.view();
        for &idx in &candidates {
            if view.next_state(idx % self.cols, idx / self.cols) != self.current[idx] {
                next_frontier.push(idx);
            }
        }
        for &idx in &candidates {
            self.candidate_flags[idx] = 0;
        }
        self.candidate_scratch = candidates;

        // Nothing has been written yet, so applying the generation is safe here. A
        // cell that changes always flips between dead and alive, so the new state
        // is the old one toggled and no second board buffer is needed.
        let mut live_delta: isize = 0;
        for &idx in &next_frontier {
            debug_assert!(self.current[idx] <= CELL_ALIVE);
            self.current[idx] = CELL_ALIVE - self.current[idx];
            live_delta += if self.current[idx] == CELL_ALIVE {
                1
            } else {
                -1
            };
            self.mark_dirty_cell(idx);
        }
        self.live_count = self.live_count.saturating_add_signed(live_delta);

        let changed = !next_frontier.is_empty();
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

    pub fn rule(&self) -> Rule {
        self.rule
    }

    /// Returns whether the rule actually changed. A different rule invalidates the
    /// frontier, since cells that were stable under the old rule may not be.
    pub fn set_rule(&mut self, rule: Rule) -> bool {
        if self.rule == rule {
            return false;
        }
        self.rule = rule;
        self.mark_frontier_full();
        true
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
        self.live_count
    }

    pub fn take_dirty_cells(&mut self) -> DirtyCells {
        if self.dirty_full {
            self.dirty_full = false;
            // `mark_dirty_full` drains the queue and `mark_dirty_cell` is a no-op
            // while the flag is set, so nothing can be pending here.
            debug_assert!(self.dirty_cells.is_empty());
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

    fn view(&self) -> CellsView<'_> {
        CellsView {
            cells: &self.current,
            cols: self.cols,
            rows: self.rows,
            boundary_mode: self.boundary_mode,
            rule: self.rule,
        }
    }

    /// Collects the frontier cells and their neighbours into `out`, deduplicated.
    /// Returns false as soon as there are more than `limit`, leaving `out` partial:
    /// the count only grows, so the dense scan is already the better choice and
    /// finishing the expansion would be wasted work.
    fn collect_frontier_candidates(&mut self, out: &mut Vec<usize>, limit: usize) -> bool {
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
                    if out.len() > limit {
                        return false;
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
                    if out.len() > limit {
                        return false;
                    }
                }
            }
        }
        true
    }

    fn mark_dirty_cell(&mut self, idx: usize) {
        if self.dirty_full || idx >= self.dirty_flags.len() || self.dirty_flags[idx] != 0 {
            return;
        }
        self.dirty_flags[idx] = 1;
        self.dirty_cells.push(idx);
    }

    fn mark_dirty_full(&mut self) {
        // Only queued indices can have a flag set, so wiping the whole array would
        // cost O(board area) on every clear, resize, or undo.
        for &idx in &self.dirty_cells {
            self.dirty_flags[idx] = 0;
        }
        self.dirty_cells.clear();
        self.dirty_full = true;
    }

    fn mark_frontier_cell(&mut self, idx: usize) {
        if self.frontier_full || idx >= self.frontier_flags.len() || self.frontier_flags[idx] != 0 {
            return;
        }
        self.frontier_flags[idx] = 1;
        self.frontier_cells.push(idx);
    }

    fn mark_frontier_full(&mut self) {
        for &idx in &self.frontier_cells {
            self.frontier_flags[idx] = 0;
        }
        self.frontier_cells.clear();
        self.frontier_full = true;
    }

    fn replace_frontier(&mut self, mut next_frontier: Vec<usize>) {
        self.frontier_full = false;
        std::mem::swap(&mut self.frontier_cells, &mut next_frontier);

        // `next_frontier` now holds the previous generation's cells. Clearing those
        // before setting the new ones keeps indices present in both marked.
        for &idx in &next_frontier {
            self.frontier_flags[idx] = 0;
        }
        for &idx in &self.frontier_cells {
            self.frontier_flags[idx] = 1;
        }

        next_frontier.clear();
        self.frontier_scratch = next_frontier;
    }
}

/// A read-only generation together with what decides its successor. It borrows
/// only the cells, so the next generation can be written elsewhere meanwhile.
#[derive(Clone, Copy)]
struct CellsView<'a> {
    cells: &'a [u8],
    cols: usize,
    rows: usize,
    boundary_mode: BoundaryMode,
    rule: Rule,
}

impl CellsView<'_> {
    fn next_state(&self, col: usize, row: usize) -> u8 {
        let alive_neighbors = if self.is_interior(col, row) {
            self.count_neighbors_interior(col, row)
        } else {
            match self.boundary_mode {
                BoundaryMode::Wrap => self.count_neighbors_wrap(col, row),
                BoundaryMode::Fixed => self.count_neighbors_fixed(col, row),
            }
        };
        let currently_alive = self.cells[row * self.cols + col] == CELL_ALIVE;
        let alive = if currently_alive {
            self.rule.allows_survival(alive_neighbors)
        } else {
            self.rule.allows_birth(alive_neighbors)
        };
        u8::from(alive)
    }

    /// A cell away from every edge has all eight neighbours in bounds, so the
    /// boundary mode cannot change its count.
    fn is_interior(&self, col: usize, row: usize) -> bool {
        col > 0 && row > 0 && col + 1 < self.cols && row + 1 < self.rows
    }

    /// Interior neighbours are plain index offsets. The wrapping and clamping
    /// paths need a division or a branch per neighbour, and most of the cells
    /// the frontier step evaluates are interior.
    fn count_neighbors_interior(&self, col: usize, row: usize) -> u8 {
        let idx = row * self.cols + col;
        let above = idx - self.cols;
        let below = idx + self.cols;

        u8::from(self.cells[above - 1] == CELL_ALIVE)
            + u8::from(self.cells[above] == CELL_ALIVE)
            + u8::from(self.cells[above + 1] == CELL_ALIVE)
            + u8::from(self.cells[idx - 1] == CELL_ALIVE)
            + u8::from(self.cells[idx + 1] == CELL_ALIVE)
            + u8::from(self.cells[below - 1] == CELL_ALIVE)
            + u8::from(self.cells[below] == CELL_ALIVE)
            + u8::from(self.cells[below + 1] == CELL_ALIVE)
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
                alive += u8::from(self.cells[nrow * self.cols + ncol] == CELL_ALIVE);
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
                    u8::from(self.cells[nrow as usize * self.cols + ncol as usize] == CELL_ALIVE);
            }
        }

        alive
    }

    /// Writes the next generation of the whole rows starting at `first_row` into
    /// `out`, appending the index of every cell that flips to `changed` in
    /// row-major order. Returns how many cells in `out` are alive.
    fn step_rows(&self, first_row: usize, out: &mut [u8], changed: &mut Vec<usize>) -> usize {
        let cols = self.cols;
        if cols == 0 {
            return 0;
        }
        debug_assert_eq!(out.len() % cols, 0);
        let mask = self.rule.transition_mask();
        let mut column_sums = vec![0u8; cols];
        let mut live = 0;

        for (offset, out_row) in out.chunks_exact_mut(cols).enumerate() {
            let row = first_row + offset;
            let start = row * cols;
            let current_row = &self.cells[start..start + cols];

            if row == 0 || row + 1 >= self.rows || cols < 3 {
                // Every cell of an edge row needs the boundary-aware count.
                for (col, next) in out_row.iter_mut().enumerate() {
                    *next = self.next_state(col, row);
                }
            } else {
                let above = &self.cells[start - cols..start];
                let below = &self.cells[start + cols..start + 2 * cols];
                step_interior_row(above, current_row, below, mask, &mut column_sums, out_row);
                out_row[0] = self.next_state(0, row);
                out_row[cols - 1] = self.next_state(cols - 1, row);
            }

            live += out_row.iter().map(|&cell| usize::from(cell)).sum::<usize>();
            collect_changed(current_row, out_row, start, changed);
        }
        live
    }
}

/// Steps columns `1..len - 1` of a row whose neighbours above and below are in
/// bounds. Each vertical three-cell sum serves three neighbourhoods, and the
/// plain slice arithmetic vectorizes. Cells hold exactly 0 or 1, so they add up
/// to neighbour counts directly.
fn step_interior_row(
    above: &[u8],
    current: &[u8],
    below: &[u8],
    mask: u32,
    sums: &mut [u8],
    out: &mut [u8],
) {
    for (((sum, &a), &c), &b) in sums.iter_mut().zip(above).zip(current).zip(below) {
        *sum = a + c + b;
    }

    let len = current.len();
    let lefts = &sums[..len - 2];
    let centres = &sums[1..len - 1];
    let rights = &sums[2..];
    for ((((next, &left), &centre), &right), &cell) in out[1..len - 1]
        .iter_mut()
        .zip(lefts)
        .zip(centres)
        .zip(rights)
        .zip(&current[1..len - 1])
    {
        // The centre column's sum includes the cell itself.
        let neighbors = left + centre + right - cell;
        *next = ((mask >> (u32::from(neighbors) + 9 * u32::from(cell))) & 1) as u8;
    }
}

/// Appends `start + i` for every `i` where the rows differ. Unchanged stretches
/// are common, so eight cells are compared at once before looking closer.
fn collect_changed(before: &[u8], after: &[u8], start: usize, changed: &mut Vec<usize>) {
    let (before_words, before_tail) = before.as_chunks::<8>();
    let (after_words, after_tail) = after.as_chunks::<8>();
    for (word, (b, a)) in before_words.iter().zip(after_words).enumerate() {
        if u64::from_ne_bytes(*b) != u64::from_ne_bytes(*a) {
            push_differences(b, a, start + word * 8, changed);
        }
    }
    let tail_start = start + before_words.len() * 8;
    push_differences(before_tail, after_tail, tail_start, changed);
}

fn push_differences(before: &[u8], after: &[u8], start: usize, changed: &mut Vec<usize>) {
    for (i, (b, a)) in before.iter().zip(after).enumerate() {
        if b != a {
            changed.push(start + i);
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
    fn rulestrings_parse_in_every_accepted_ordering() {
        use super::{Rule, RuleParseError};

        let life = Rule::LIFE;
        assert_eq!(Rule::parse("B3/S23"), Ok(life));
        assert_eq!(Rule::parse("b3/s23"), Ok(life));
        assert_eq!(Rule::parse(" B3 / S23 "), Ok(life));
        assert_eq!(Rule::parse("S23/B3"), Ok(life));
        // Legacy ordering lists survival counts first.
        assert_eq!(Rule::parse("23/3"), Ok(life));
        assert_eq!(life.to_string(), "B3/S23");

        let highlife = Rule::parse("B36/S23").expect("HighLife should parse");
        assert_ne!(highlife, life);
        assert_eq!(highlife.to_string(), "B36/S23");

        assert_eq!(Rule::parse(""), Err(RuleParseError::Empty));
        assert_eq!(Rule::parse("B3S23"), Err(RuleParseError::Malformed));
        assert_eq!(
            Rule::parse("B3/S2x"),
            Err(RuleParseError::InvalidDigit('x'))
        );
        assert_eq!(
            Rule::parse("B9/S23"),
            Err(RuleParseError::NeighborOutOfRange(9))
        );
    }

    #[test]
    fn a_non_life_rule_changes_stepping() {
        use super::Rule;

        // Under B36/S23 a 2x2 block of live cells births into a larger shape,
        // whereas plain Life leaves it as a still life.
        let mut life = Board::new(8, 8);
        for (col, row) in [(3, 3), (4, 3), (3, 4), (4, 4)] {
            life.populate(col, row);
        }
        let mut highlife = life.clone();
        assert!(highlife.set_rule(Rule::parse("B36/S23").unwrap()));
        assert!(!highlife.set_rule(Rule::parse("B36/S23").unwrap()));

        life.step();
        assert_eq!(life.live_cells_count(), 4);

        // Surround a dead cell with exactly six live neighbours: the one count the
        // two rules disagree about.
        let six_neighbors = [(3, 3), (4, 3), (5, 3), (3, 4), (5, 4), (3, 5)];
        let mut b3_board = Board::new(8, 8);
        let mut b36_board = Board::new(8, 8);
        for (col, row) in six_neighbors {
            b3_board.populate(col, row);
            b36_board.populate(col, row);
        }
        b36_board.set_rule(Rule::parse("B36/S23").unwrap());

        b3_board.step();
        b36_board.step();
        assert!(
            !b3_board.is_alive(4, 4),
            "B3/S23 must not birth on six neighbors"
        );
        assert!(
            b36_board.is_alive(4, 4),
            "B36/S23 must birth on six neighbors"
        );
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

    fn seed_deterministic(board: &mut Board, live_percent: u64) {
        let (cols, rows) = board.size();
        for row in 0..rows {
            for col in 0..cols {
                let mut x = ((row * cols + col) as u64).wrapping_add(0x9E3779B97F4A7C15);
                x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
                x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
                x ^= x >> 31;
                if x % 100 < live_percent {
                    board.populate(col, row);
                }
            }
        }
    }

    fn assert_live_count_in_sync(board: &Board, context: &str) {
        assert_eq!(
            board.live_cells_count(),
            super::count_live(&board.current),
            "live count drifted after {context}"
        );
    }

    #[test]
    fn live_count_tracks_every_mutation() {
        let mut board = Board::new(16, 12);
        assert_live_count_in_sync(&board, "construction");

        board.populate(3, 4);
        board.populate(3, 4);
        board.populate(4, 4);
        board.populate(5, 4);
        assert_eq!(board.live_cells_count(), 3);
        assert_live_count_in_sync(&board, "populate");

        board.depopulate(3, 4);
        board.depopulate(3, 4);
        assert_eq!(board.live_cells_count(), 2);
        assert_live_count_in_sync(&board, "depopulate");

        board.dense_threshold_percent = 100;
        for generation in 0..24 {
            board.step();
            assert_live_count_in_sync(&board, &format!("frontier step {generation}"));
        }

        board.mark_frontier_full();
        board.step();
        assert_live_count_in_sync(&board, "dense step");

        let snapshot = board.snapshot_cells();
        board.clear();
        assert_eq!(board.live_cells_count(), 0);
        assert_live_count_in_sync(&board, "clear");

        board.restore_cells(snapshot);
        assert_live_count_in_sync(&board, "restore");

        board.populate_random();
        assert_live_count_in_sync(&board, "populate_random");

        board.resize(20, 8);
        assert_live_count_in_sync(&board, "resize");
    }

    #[test]
    fn interior_fast_path_matches_boundary_aware_counting() {
        // step/dense equivalence cannot catch a bug here, because both paths go
        // through the same next_state and so share the fast path.
        for mode in [BoundaryMode::Wrap, BoundaryMode::Fixed] {
            let mut board = Board::new(12, 9);
            board.set_boundary_mode(mode);
            seed_deterministic(&mut board, 35);

            let (cols, rows) = board.size();
            let board = board.view();
            let mut interior_cells = 0;
            for row in 0..rows {
                for col in 0..cols {
                    if !board.is_interior(col, row) {
                        continue;
                    }
                    interior_cells += 1;
                    let expected = match mode {
                        BoundaryMode::Wrap => board.count_neighbors_wrap(col, row),
                        BoundaryMode::Fixed => board.count_neighbors_fixed(col, row),
                    };
                    assert_eq!(
                        board.count_neighbors_interior(col, row),
                        expected,
                        "({col}, {row}) under {mode:?}"
                    );
                }
            }
            assert_eq!(interior_cells, (cols - 2) * (rows - 2));
        }
    }

    #[test]
    fn row_kernel_matches_per_cell_stepping() {
        use super::Rule;

        // B0 births on zero neighbours, so it flips empty space everywhere and
        // catches a kernel that skips cells it assumes are quiet.
        let rules = ["B3/S23", "B36/S23", "B0/S8", "B12345678/S012345678"];
        let sizes = [
            (1, 1),
            (2, 5),
            (3, 3),
            (5, 2),
            (8, 8),
            (13, 7),
            (17, 11),
            (64, 3),
        ];
        for mode in [BoundaryMode::Wrap, BoundaryMode::Fixed] {
            for rule in rules {
                for (cols, rows) in sizes {
                    let mut board = Board::new(cols, rows);
                    board.set_boundary_mode(mode);
                    board.set_rule(Rule::parse(rule).unwrap());
                    seed_deterministic(&mut board, 40);
                    let view = board.view();

                    let mut expected = Vec::new();
                    let mut changed_expected = Vec::new();
                    for row in 0..rows {
                        for col in 0..cols {
                            let next = view.next_state(col, row);
                            if next != board.current[row * cols + col] {
                                changed_expected.push(row * cols + col);
                            }
                            expected.push(next);
                        }
                    }

                    let mut out = vec![0; cols * rows];
                    let mut changed = Vec::new();
                    let live = view.step_rows(0, &mut out, &mut changed);
                    let context = format!("{cols}x{rows} {rule} {mode:?}");
                    assert_eq!(out, expected, "{context}");
                    assert_eq!(changed, changed_expected, "{context}");
                    assert_eq!(live, super::count_live(&expected), "{context}");

                    // Starting part-way down must give the same rows, which is
                    // what splitting the board into bands will rely on.
                    if rows > 2 {
                        let mut tail = vec![0; cols * (rows - 2)];
                        let mut tail_changed = Vec::new();
                        view.step_rows(2, &mut tail, &mut tail_changed);
                        assert_eq!(tail, expected[cols * 2..], "{context} from row 2");
                        let from_row_two: Vec<_> = changed_expected
                            .iter()
                            .copied()
                            .filter(|&idx| idx >= cols * 2)
                            .collect();
                        assert_eq!(tail_changed, from_row_two, "{context} from row 2");
                    }
                }
            }
        }
    }

    #[test]
    fn populate_random_lands_near_the_intended_density() {
        let mut board = Board::new(200, 200);
        board.populate_random();

        let density = board.live_cells_count() as f64 / 40_000.0;
        assert!(
            (0.23..=0.27).contains(&density),
            "expected roughly 25% live, got {density}"
        );
    }

    #[test]
    fn frontier_stepping_matches_dense_stepping() {
        for mode in [BoundaryMode::Wrap, BoundaryMode::Fixed] {
            for live_percent in [2, 15, 40] {
                let mut frontier_board = Board::new(24, 18);
                // Never fall back, so every step really takes the frontier path.
                frontier_board.dense_threshold_percent = 100;
                frontier_board.set_boundary_mode(mode);
                seed_deterministic(&mut frontier_board, live_percent);
                let mut dense_board = frontier_board.clone();

                for generation in 0..32 {
                    frontier_board.step();
                    // Forcing the frontier full routes this board through step_dense.
                    dense_board.mark_frontier_full();
                    dense_board.step();

                    assert_eq!(
                        frontier_board.current, dense_board.current,
                        "diverged at generation {generation} ({mode:?}, {live_percent}% live)"
                    );
                }
            }
        }
    }

    #[test]
    fn frontier_step_reports_change_and_dirties_only_flipped_cells() {
        let mut board = Board::new(6, 6);
        board.populate(1, 2);
        board.populate(2, 2);
        board.populate(3, 2);
        let _ = board.take_dirty_cells();

        assert!(board.step());
        let DirtyCells::Cells(mut indices) = board.take_dirty_cells() else {
            panic!("expected incremental dirty cells");
        };
        indices.sort_unstable();
        assert_eq!(indices, vec![8, 13, 15, 20]);

        // A still life flips nothing, so the step reports no change.
        let mut block = Board::new(6, 6);
        block.populate(1, 1);
        block.populate(1, 2);
        block.populate(2, 1);
        block.populate(2, 2);
        let _ = block.take_dirty_cells();

        assert!(!block.step());
        assert_eq!(block.take_dirty_cells(), DirtyCells::None);
    }

    #[test]
    fn large_frontiers_skip_to_the_dense_scan_and_leave_no_candidate_flags() {
        // At 10% of 24x18 the limit is 43 cells. 40% live overflows on the
        // frontier alone; 2% live overflows only part-way through the expansion.
        for (live_percent, overflows_on_frontier) in [(40, true), (2, false)] {
            for mode in [BoundaryMode::Wrap, BoundaryMode::Fixed] {
                let mut board = Board::new(24, 18);
                board.dense_threshold_percent = 10;
                board.set_boundary_mode(mode);
                // Changing the mode forces a dense step, which would bypass the
                // frontier entirely. The board is still empty, so seeding alone
                // rebuilds a complete frontier.
                board.frontier_full = false;
                seed_deterministic(&mut board, live_percent);
                let limit = board.current.len() * 10 / 100;
                let context = format!("{live_percent}% live, {mode:?}");

                let mut probe = board.clone();
                let mut all_candidates = Vec::new();
                probe.collect_frontier_candidates(&mut all_candidates, usize::MAX);
                assert!(all_candidates.len() > limit, "{context}");
                assert_eq!(
                    board.frontier_cells.len() > limit,
                    overflows_on_frontier,
                    "{context}"
                );

                let mut dense = board.clone();
                dense.mark_frontier_full();
                board.step();
                dense.step();

                assert_eq!(board.current, dense.current, "{context}");
                assert_eq!(board.frontier_cells, dense.frontier_cells, "{context}");
                assert_eq!(board.live_cells_count(), dense.live_cells_count());
                // A flag left behind by the abandoned expansion would silently
                // drop that cell from every later candidate set.
                assert!(
                    board.candidate_flags.iter().all(|&flag| flag == 0),
                    "{context}"
                );
            }
        }
    }

    #[test]
    fn boundary_mode_toggle_marks_frontier_full() {
        let mut board = Board::new(5, 5);
        assert!(!board.frontier_full);

        board.toggle_boundary_mode();

        assert!(board.frontier_full);
    }
}
