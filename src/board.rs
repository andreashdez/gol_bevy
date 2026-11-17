//! The finite board: cells at a bit each in 64 × 64 chunks, as on the unbounded
//! board, in a grid that covers the board instead of a map. Rows are scheduled
//! and stepped the same way, 64 cells at a time. What differs is the edges: a
//! row's neighbours come from the far side of the board when it wraps and from
//! nowhere when it does not, and the last chunk across and the last one up may
//! be only partly on the board.
use crate::sparse_board::{CHUNK_SIZE, ChangedRows, Chunk, RowPlanes, bits, next_cells};
use bevy::tasks::{ComputeTaskPool, TaskPool};

/// Preferences for replacing a board with random cells. A fixed seed repeats
/// the same row-major sequence for the same dimensions and density.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RandomizeSettings {
    density_percent: u8,
    seed: Option<u64>,
}

impl Default for RandomizeSettings {
    fn default() -> Self {
        Self {
            density_percent: 25,
            seed: None,
        }
    }
}

impl RandomizeSettings {
    /// Whether each cell in turn is alive, from a local SplitMix64 sequence.
    pub(crate) fn draws(self) -> impl FnMut() -> bool {
        let mut state = self.seed.unwrap_or_else(|| fastrand::u64(..));
        move || {
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut value = state;
            value = (value ^ (value >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            value = (value ^ (value >> 27)).wrapping_mul(0x94D049BB133111EB);
            value ^= value >> 31;
            value % 100 < u64::from(self.density_percent)
        }
    }

    pub fn new(density_percent: u8, seed: Option<u64>) -> Option<Self> {
        (density_percent <= 100).then_some(Self {
            density_percent,
            seed,
        })
    }

    pub fn density_percent(self) -> u8 {
        self.density_percent
    }
    pub fn seed(self) -> Option<u64> {
        self.seed
    }
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

    /// Whether a dead cell with no live neighbours is born, which on an unbounded
    /// board would light every cell of the plane.
    pub(crate) fn births_from_nothing(self) -> bool {
        self.allows_birth(0)
    }

    fn allows_birth(self, neighbors: u8) -> bool {
        self.birth & (1 << neighbors) != 0
    }

    fn allows_survival(self, neighbors: u8) -> bool {
        self.survive & (1 << neighbors) != 0
    }

    /// Birth counts in bits 0..=8 and survival counts in bits 9..=17, so the next
    /// state is bit `neighbors + 9 * alive` without branching on `alive`.
    pub(crate) fn transition_mask(self) -> u32 {
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

/// Up to 64 cells side by side in a row, a bit each from `first` on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowCells {
    /// Index of the cell in bit 0, as `row * cols + col`.
    pub first: usize,
    /// The cells spoken for: those that changed, or those on the board.
    pub cells: u64,
    /// Which cells of the row are alive.
    pub alive: u64,
}

impl RowCells {
    /// The index of each cell spoken for, and whether it is alive.
    pub fn iter(self) -> impl Iterator<Item = (usize, bool)> {
        bits(self.cells).map(move |bit| (self.first + bit, self.alive >> bit & 1 != 0))
    }

    /// How many cells are spoken for.
    pub fn len(self) -> usize {
        self.cells.count_ones() as usize
    }

    pub fn is_empty(self) -> bool {
        self.cells == 0
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum DirtyCells {
    None,
    Full,
    /// The rows with a changed cell, a chunk's width of each at a time.
    Cells(Vec<RowCells>),
}

/// The cells of an earlier board, a bit each, for undo to bring back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedCells(Vec<Chunk>);

impl PackedCells {
    /// Memory this holds.
    pub fn bytes(&self) -> usize {
        self.0.len() * size_of::<Chunk>()
    }
}

const SIDE: usize = CHUNK_SIZE as usize;
const LAST: usize = SIDE - 1;
static EMPTY_CHUNK: Chunk = [0; SIDE];

/// Fewest scheduled chunks that justify another parallel band, as on the
/// unbounded board.
const MIN_BAND_CHUNKS: usize = 16;

/// The low `count` bits, for `count` up to 64.
fn low_bits(count: usize) -> u64 {
    if count >= 64 { !0 } else { (1 << count) - 1 }
}

/// Where the board's edges fall among its chunks, and what lies past them.
#[derive(Debug, Clone, Copy)]
struct Shape {
    cols: usize,
    rows: usize,
    /// Chunks across the board, the last of which may be partly past its edge.
    across: usize,
    /// Chunks up the board, likewise.
    down: usize,
    mode: BoundaryMode,
}

impl Shape {
    fn new(cols: usize, rows: usize, mode: BoundaryMode) -> Self {
        Self {
            cols,
            rows,
            across: cols.div_ceil(SIDE),
            down: rows.div_ceil(SIDE),
            mode,
        }
    }

    fn slot(&self, cx: usize, cy: usize) -> usize {
        cy * self.across + cx
    }

    /// The bit of the board's last column, in the last chunk across.
    fn last_col(&self) -> usize {
        self.cols.saturating_sub(1) % SIDE
    }

    /// The row of the board's last row, in the last chunk up.
    fn last_row(&self) -> usize {
        self.rows.saturating_sub(1) % SIDE
    }

    /// The last column of chunk column `cx` that is on the board.
    fn east_col(&self, cx: usize) -> usize {
        if cx + 1 == self.across {
            self.last_col()
        } else {
            LAST
        }
    }

    /// The last row of chunk row `cy` that is on the board.
    fn top_row(&self, cy: usize) -> usize {
        if cy + 1 == self.down {
            self.last_row()
        } else {
            LAST
        }
    }

    fn valid_cols(&self, cx: usize) -> u64 {
        low_bits(self.east_col(cx) + 1)
    }

    fn valid_rows(&self, cy: usize) -> u64 {
        low_bits(self.top_row(cy) + 1)
    }

    /// The chunk column or row before `index`, round the board if it wraps.
    fn before(&self, index: usize, count: usize) -> Option<usize> {
        match (index, self.mode) {
            (0, BoundaryMode::Wrap) => Some(count - 1),
            (0, BoundaryMode::Fixed) => None,
            _ => Some(index - 1),
        }
    }

    /// The chunk column or row after `index`, round the board if it wraps.
    fn after(&self, index: usize, count: usize) -> Option<usize> {
        match (index + 1 == count, self.mode) {
            (true, BoundaryMode::Wrap) => Some(0),
            (true, BoundaryMode::Fixed) => None,
            _ => Some(index + 1),
        }
    }

    /// Which rows of chunk column `cx` changed, given the cells that flipped in `row`.
    fn changed(&self, cx: usize, row: usize, flipped: u64) -> ChangedRows {
        ChangedRows {
            any: u64::from(flipped != 0) << row,
            west: (flipped & 1) << row,
            east: (flipped >> self.east_col(cx) & 1) << row,
        }
    }
}

/// A chunk's next generation, staged so bands can share the old one.
#[derive(Debug, Clone)]
struct ChunkChange {
    slot: u32,
    next: Chunk,
    rows: ChangedRows,
}

#[derive(Debug, Clone)]
pub struct Board {
    shape: Shape,
    /// chunk `(cx, cy)` at `cy * across + cx`, with cell `(x, y)` of the chunk in
    /// bit `x` of row `y`. Cells past the board's edges stay dead.
    chunks: Vec<Chunk>,
    /// rows to evaluate next generation, by chunk: those that changed last
    /// generation and those next to them, here or in a neighbouring chunk
    active: Vec<u64>,
    /// chunks with a row to evaluate
    active_list: Vec<u32>,
    /// cells changed since last render sync, by chunk
    dirty: Vec<Chunk>,
    /// rows of each chunk with a changed cell
    dirty_rows: Vec<u64>,
    /// chunks with a changed cell
    dirty_list: Vec<u32>,
    /// request a full redraw on next render sync
    dirty_full: bool,
    /// recycled buffer for the generation's scheduled chunks
    work_scratch: Vec<(u32, u64)>,
    /// recycled lists for each band's changes
    change_scratch: Vec<Vec<ChunkChange>>,
    /// running count of live cells, kept in step with every mutation
    live_count: usize,
    /// XOR of [`cell_key`] for every flip made by stepping, so generations of one
    /// run match exactly when their cells do. Edits leave it alone.
    fingerprint: u64,
    /// birth/survival rule applied by each step
    rule: Rule,
}

impl Board {
    pub fn new(cols: usize, rows: usize) -> Self {
        let shape = Shape::new(cols, rows, BoundaryMode::Wrap);
        let chunks = shape.across * shape.down;
        let mut board = Self {
            shape,
            chunks: vec![EMPTY_CHUNK; chunks],
            active: vec![0; chunks],
            active_list: Vec::new(),
            dirty: vec![EMPTY_CHUNK; chunks],
            dirty_rows: vec![0; chunks],
            dirty_list: Vec::new(),
            dirty_full: true,
            work_scratch: Vec::new(),
            change_scratch: Vec::new(),
            live_count: 0,
            fingerprint: 0,
            rule: Rule::LIFE,
        };
        // An empty board is not always a still one: B0 gives birth all over it.
        board.wake_all();
        board
    }

    pub(crate) fn snapshot_cells(&self) -> PackedCells {
        PackedCells(self.chunks.clone())
    }

    pub(crate) fn restore_snapshot(&mut self, cells: &PackedCells) {
        assert_eq!(cells.0.len(), self.chunks.len());
        self.chunks.copy_from_slice(&cells.0);
        self.cells_replaced();
    }

    /// Replaces every cell, leaving alive only those of `cells` that are on the board.
    pub(crate) fn replace_cells(&mut self, cells: impl IntoIterator<Item = (usize, usize)>) {
        self.chunks.fill(EMPTY_CHUNK);
        for (col, row) in cells {
            if let Some((slot, x, y)) = self.locate(col, row) {
                self.chunks[slot][y] |= 1 << x;
            }
        }
        self.cells_replaced();
    }

    /// Catches up with cells written straight into the chunks.
    fn cells_replaced(&mut self) {
        self.live_count = self
            .chunks
            .iter()
            .flatten()
            .map(|row| row.count_ones() as usize)
            .sum();
        self.mark_dirty_full();
        self.wake_all();
    }

    /// Keeps the cells that fit the new size, counted from cell `(0, 0)`.
    pub fn resize(&mut self, new_cols: usize, new_rows: usize) -> bool {
        if self.size() == (new_cols, new_rows) {
            return false;
        }

        let old = self.shape;
        let shape = Shape::new(new_cols, new_rows, old.mode);
        let mut chunks = vec![EMPTY_CHUNK; shape.across * shape.down];
        for cy in 0..old.down.min(shape.down) {
            for cx in 0..old.across.min(shape.across) {
                let (from, to) = (
                    &self.chunks[old.slot(cx, cy)],
                    &mut chunks[shape.slot(cx, cy)],
                );
                let on_board = shape.valid_cols(cx);
                for row in bits(shape.valid_rows(cy)) {
                    to[row] = from[row] & on_board;
                }
            }
        }

        self.shape = shape;
        self.active = vec![0; chunks.len()];
        self.dirty = vec![EMPTY_CHUNK; chunks.len()];
        self.dirty_rows = vec![0; chunks.len()];
        // Queued chunks address the old layout.
        self.dirty_list.clear();
        self.chunks = chunks;
        self.cells_replaced();
        true
    }

    pub fn populate(&mut self, col: usize, row: usize) -> bool {
        self.set_cell(col, row, true)
    }

    pub fn depopulate(&mut self, col: usize, row: usize) -> bool {
        self.set_cell(col, row, false)
    }

    pub fn populate_random(&mut self) {
        self.populate_random_with(RandomizeSettings::default());
    }

    /// Replace every cell, using a local SplitMix64 sequence. The explicit
    /// algorithm keeps seeded boards independent of dependency RNG changes and
    /// other random operations. Arithmetic deliberately wraps at 64 bits.
    pub fn populate_random_with(&mut self, settings: RandomizeSettings) {
        let mut draw = settings.draws();
        let shape = self.shape;
        // Row by row, as the sequence is defined, not chunk by chunk.
        for row in 0..shape.rows {
            for cx in 0..shape.across {
                let cells =
                    (0..=shape.east_col(cx)).fold(0, |cells, col| cells | u64::from(draw()) << col);
                self.chunks[shape.slot(cx, row / SIDE)][row % SIDE] = cells;
            }
        }
        self.cells_replaced();
    }

    pub fn clear(&mut self) -> bool {
        if self.live_count == 0 {
            return false;
        }
        self.chunks.fill(EMPTY_CHUNK);
        self.cells_replaced();
        true
    }

    pub fn step(&mut self) -> bool {
        let threads = ComputeTaskPool::get_or_init(TaskPool::default).thread_num();
        self.step_in_bands(threads.min(self.active_list.len() / MIN_BAND_CHUNKS))
    }

    /// Evaluates the scheduled rows in up to `bands` bands of whole chunks, in
    /// parallel on the compute task pool when there is more than one. Bands only
    /// read the chunks and stage their changes, which are applied once all have
    /// finished, so no band can see another's half of the generation.
    fn step_in_bands(&mut self, bands: usize) -> bool {
        if self.active_list.is_empty() {
            return false;
        }

        let mut work = std::mem::take(&mut self.work_scratch);
        work.clear();
        let active = &mut self.active;
        work.extend(
            self.active_list
                .drain(..)
                .map(|slot| (slot, std::mem::take(&mut active[slot as usize]))),
        );
        let band_len = work.len().div_ceil(bands.max(1));
        let mut changes = std::mem::take(&mut self.change_scratch);
        changes.resize_with(work.len().div_ceil(band_len), Vec::new);

        let (chunks, shape, rule) = (&self.chunks[..], self.shape, self.rule);
        let mut bands = work.chunks(band_len).zip(&mut changes);
        let (live_delta, flips) = if bands.len() == 1 {
            let (band, changes) = bands.next().expect("scheduled work has a band");
            step_band(chunks, shape, rule, band, changes)
        } else {
            ComputeTaskPool::get_or_init(TaskPool::default)
                .scope(|scope| {
                    for (band, changes) in bands {
                        scope.spawn(async move { step_band(chunks, shape, rule, band, changes) });
                    }
                })
                .into_iter()
                .fold((0, 0), |(live, flips), (band_live, band_flips)| {
                    (live + band_live, flips ^ band_flips)
                })
        };

        let mut changed = false;
        for list in &mut changes {
            changed |= !list.is_empty();
            for change in list.iter() {
                self.apply(change);
            }
            list.clear();
        }
        self.work_scratch = work;
        self.change_scratch = changes;
        self.live_count = self.live_count.saturating_add_signed(live_delta);
        self.fingerprint ^= flips;
        changed
    }

    fn apply(&mut self, change: &ChunkChange) {
        let slot = change.slot as usize;
        if !self.dirty_full {
            if self.dirty_rows[slot] == 0 {
                self.dirty_list.push(change.slot);
            }
            self.dirty_rows[slot] |= change.rows.any;
            for row in bits(change.rows.any) {
                self.dirty[slot][row] |= self.chunks[slot][row] ^ change.next[row];
            }
        }
        self.chunks[slot] = change.next;
        self.wake(slot, change.rows);
    }

    pub fn size(&self) -> (usize, usize) {
        (self.shape.cols, self.shape.rows)
    }

    pub fn toggle_boundary_mode(&mut self) {
        self.set_boundary_mode(match self.shape.mode {
            BoundaryMode::Wrap => BoundaryMode::Fixed,
            BoundaryMode::Fixed => BoundaryMode::Wrap,
        });
    }

    /// New edges give the cells beside them new neighbours, so every row is
    /// evaluated again.
    pub fn set_boundary_mode(&mut self, mode: BoundaryMode) {
        if self.shape.mode == mode {
            return;
        }
        self.shape.mode = mode;
        self.wake_all();
    }

    pub fn boundary_mode(&self) -> BoundaryMode {
        self.shape.mode
    }

    pub fn rule(&self) -> Rule {
        self.rule
    }

    /// Returns whether the rule actually changed. A different rule evaluates every
    /// row again, since cells that were stable under the old rule may not be.
    pub fn set_rule(&mut self, rule: Rule) -> bool {
        if self.rule == rule {
            return false;
        }
        self.rule = rule;
        self.wake_all();
        true
    }

    pub fn boundary_mode_label(&self) -> &'static str {
        match self.shape.mode {
            BoundaryMode::Wrap => "Wrap",
            BoundaryMode::Fixed => "Fixed",
        }
    }

    pub fn is_alive(&self, col: usize, row: usize) -> bool {
        self.locate(col, row)
            .is_some_and(|(slot, x, y)| self.chunks[slot][y] >> x & 1 != 0)
    }

    /// Every cell of the board, row by row from row 0, up to 64 at a time: each
    /// item's `cells` are a run from its `first`.
    pub fn row_cells(&self) -> impl Iterator<Item = RowCells> + '_ {
        let shape = self.shape;
        (0..shape.rows).flat_map(move |row| {
            (0..shape.across).map(move |cx| RowCells {
                first: row * shape.cols + cx * SIDE,
                cells: shape.valid_cols(cx),
                alive: self.chunks[shape.slot(cx, row / SIDE)][row % SIDE],
            })
        })
    }

    /// The column and row of every live cell, row by row from row 0.
    pub fn live_cells(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        let cols = self.shape.cols;
        self.row_cells().flat_map(move |run| {
            bits(run.alive).map(move |bit| ((run.first + bit) % cols, run.first / cols))
        })
    }

    /// Counts the live cells under each block of a `width` × `height` grid over
    /// the board, row 0 first, into `counts`. Blocks are at least a cell across,
    /// so the grid may not be larger than the board.
    pub fn overview(&self, width: usize, height: usize, counts: &mut [u32]) {
        let Shape { cols, rows, .. } = self.shape;
        assert!(width <= cols && height <= rows && width > 0 && height > 0);
        assert_eq!(counts.len(), width * height);
        counts.fill(0);
        for row in 0..rows {
            let line = &mut counts[row * height / rows * width..][..width];
            let words = |cx: usize| self.chunks[self.shape.slot(cx, row / SIDE)][row % SIDE];
            for (block, count) in line.iter_mut().enumerate() {
                let (start, end) = (block * cols / width, (block + 1) * cols / width);
                // The block's cells in each chunk it crosses, from `start` on.
                let mut col = start;
                while col < end {
                    let stop = end.min((col / SIDE + 1) * SIDE);
                    let cells = words(col / SIDE) >> (col % SIDE) & low_bits(stop - col);
                    *count += cells.count_ones();
                    col = stop;
                }
            }
        }
    }

    /// Identifies the current generation among others of the same run: stepping
    /// back to the same cells restores it. Only comparable while nothing but
    /// stepping changes the board.
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    pub fn live_cells_count(&self) -> usize {
        self.live_count
    }

    /// Inclusive bounds, computed on demand rather than during simulation steps.
    pub fn live_bounds(&self) -> Option<((usize, usize), (usize, usize))> {
        if self.live_count == 0 {
            return None;
        }
        let mut min = (self.shape.cols, self.shape.rows);
        let mut max = (0, 0);
        for (slot, chunk) in self.chunks.iter().enumerate() {
            let (left, bottom) = (
                slot % self.shape.across * SIDE,
                slot / self.shape.across * SIDE,
            );
            for (row, &cells) in chunk.iter().enumerate().filter(|(_, cells)| **cells != 0) {
                let first = left + cells.trailing_zeros() as usize;
                let last = left + LAST - cells.leading_zeros() as usize;
                min = (min.0.min(first), min.1.min(bottom + row));
                max = (max.0.max(last), max.1.max(bottom + row));
            }
        }
        Some((min, max))
    }

    /// The cells that changed since this was last called, in no particular order.
    pub fn take_dirty_cells(&mut self) -> DirtyCells {
        if self.dirty_full {
            self.dirty_full = false;
            // `mark_dirty_full` drains the queue and nothing joins it while the
            // flag is set, so nothing can be pending here.
            debug_assert!(self.dirty_list.is_empty());
            return DirtyCells::Full;
        }
        if self.dirty_list.is_empty() {
            return DirtyCells::None;
        }

        let rows = self
            .dirty_list
            .iter()
            .map(|&slot| self.dirty_rows[slot as usize].count_ones() as usize)
            .sum();
        let mut changed = Vec::with_capacity(rows);
        for slot in self.dirty_list.drain(..) {
            let slot = slot as usize;
            let (cx, cy) = (slot % self.shape.across, slot / self.shape.across);
            for row in bits(std::mem::take(&mut self.dirty_rows[slot])) {
                changed.push(RowCells {
                    first: (cy * SIDE + row) * self.shape.cols + cx * SIDE,
                    cells: std::mem::take(&mut self.dirty[slot][row]),
                    alive: self.chunks[slot][row],
                });
            }
        }
        DirtyCells::Cells(changed)
    }

    /// The chunk holding a cell, and the cell's column and row within it.
    fn locate(&self, col: usize, row: usize) -> Option<(usize, usize, usize)> {
        (col < self.shape.cols && row < self.shape.rows).then(|| {
            (
                self.shape.slot(col / SIDE, row / SIDE),
                col % SIDE,
                row % SIDE,
            )
        })
    }

    fn set_cell(&mut self, col: usize, row: usize, alive: bool) -> bool {
        let Some((slot, x, y)) = self.locate(col, row) else {
            return false;
        };
        let cell = 1 << x;
        if (self.chunks[slot][y] & cell != 0) == alive {
            return false;
        }
        self.chunks[slot][y] ^= cell;
        if alive {
            self.live_count += 1;
        } else {
            self.live_count -= 1;
        }
        if !self.dirty_full {
            if self.dirty_rows[slot] == 0 {
                self.dirty_list.push(slot as u32);
            }
            self.dirty_rows[slot] |= 1 << y;
            self.dirty[slot][y] |= cell;
        }
        self.wake(slot, self.shape.changed(col / SIDE, y, cell));
        true
    }

    /// A cell can only change after it or one of its neighbours has, so schedule
    /// the rows beside the ones that changed, in this chunk and in any neighbour
    /// that can see them: round the board where it wraps. Scheduling rows, not
    /// chunks, keeps a lone glider from costing a chunk.
    fn wake(&mut self, slot: usize, rows: ChangedRows) {
        let shape = self.shape;
        let (cx, cy) = (slot % shape.across, slot / shape.across);
        let top = shape.top_row(cy);
        let valid = shape.valid_rows(cy);
        let below = shape.before(cy, shape.down);
        let above = shape.after(cy, shape.down);

        for (column, rows) in [
            (Some(cx), rows.any),
            (shape.before(cx, shape.across), rows.west),
            (shape.after(cx, shape.across), rows.east),
        ] {
            let Some(column) = column.filter(|_| rows != 0) else {
                continue;
            };
            self.schedule(
                shape.slot(column, cy),
                (rows | rows << 1 | rows >> 1) & valid,
            );
            if let Some(below) = below.filter(|_| rows & 1 != 0) {
                self.schedule(shape.slot(column, below), 1 << shape.top_row(below));
            }
            if let Some(above) = above.filter(|_| rows >> top & 1 != 0) {
                self.schedule(shape.slot(column, above), 1);
            }
        }
    }

    fn schedule(&mut self, slot: usize, rows: u64) {
        if self.active[slot] == 0 {
            self.active_list.push(slot as u32);
        }
        self.active[slot] |= rows;
    }

    /// Schedules every row, for a change that could reach any cell.
    fn wake_all(&mut self) {
        self.active_list.clear();
        for slot in 0..self.chunks.len() {
            self.active[slot] = self.shape.valid_rows(slot / self.shape.across);
            self.active_list.push(slot as u32);
        }
    }

    fn mark_dirty_full(&mut self) {
        for slot in self.dirty_list.drain(..) {
            self.dirty_rows[slot as usize] = 0;
            self.dirty[slot as usize] = EMPTY_CHUNK;
        }
        self.dirty_full = true;
    }
}

/// A chunk's row and the rows either side of it, read with the board's edges in
/// mind: the neighbours of its first and last columns may be across the board.
struct Rows<'a> {
    /// The chunks to the west of, holding, and to the east of the rows.
    chunks: [&'a Chunk; 3],
    /// The bit of the western chunk that neighbours column 0.
    west_col: usize,
    /// The last column on the board, which the eastern chunk's column 0 neighbours.
    east_col: usize,
}

impl Rows<'_> {
    fn planes(&self, row: usize) -> RowPlanes {
        let [west, centre, east] = self.chunks.map(|chunk| chunk[row]);
        RowPlanes {
            west: centre << 1 | (west >> self.west_col & 1),
            centre,
            east: centre >> 1 | (east & 1) << self.east_col,
        }
    }
}

/// Stages the next state of the scheduled rows of `band`, returning the change in
/// the live count and the XOR of the keys of every cell that flipped.
fn step_band(
    chunks: &[Chunk],
    shape: Shape,
    rule: Rule,
    band: &[(u32, u64)],
    changes: &mut Vec<ChunkChange>,
) -> (isize, u64) {
    let mask = rule.transition_mask();
    // Empty rows stay empty unless the rule gives birth among no neighbours.
    let empty_stays = !rule.births_from_nothing();
    let (mut live_delta, mut flips) = (0, 0);
    for &(slot, scheduled) in band {
        let (cx, cy) = (slot as usize % shape.across, slot as usize / shape.across);
        let west = shape.before(cx, shape.across);
        let east = shape.after(cx, shape.across);
        let rows_of = |cy: Option<usize>| {
            let get = |cx: Option<usize>| {
                cx.zip(cy)
                    .map_or(&EMPTY_CHUNK, |(cx, cy)| &chunks[shape.slot(cx, cy)])
            };
            Rows {
                chunks: [get(west), get(Some(cx)), get(east)],
                west_col: if cx == 0 { shape.last_col() } else { LAST },
                east_col: shape.east_col(cx),
            }
        };
        let below = shape.before(cy, shape.down);
        let below_row = below.map_or(0, |below| shape.top_row(below));
        let (below, level, above) = (
            rows_of(below),
            rows_of(Some(cy)),
            rows_of(shape.after(cy, shape.down)),
        );
        let top = shape.top_row(cy);
        let on_board = shape.valid_cols(cx);

        let old = level.chunks[1];
        let mut next = *old;
        let mut rows = ChangedRows::default();
        for row in bits(scheduled) {
            let under = if row == 0 {
                below.planes(below_row)
            } else {
                level.planes(row - 1)
            };
            let here = level.planes(row);
            let over = if row == top {
                above.planes(0)
            } else {
                level.planes(row + 1)
            };
            next[row] = if empty_stays && under.any() | here.any() | over.any() == 0 {
                0
            } else {
                next_cells(under, here, over, mask) & on_board
            };

            let flipped = old[row] ^ next[row];
            if flipped == 0 {
                continue;
            }
            let changed = shape.changed(cx, row, flipped);
            rows.any |= changed.any;
            rows.west |= changed.west;
            rows.east |= changed.east;
            live_delta += next[row].count_ones() as isize - old[row].count_ones() as isize;
            let first = (cy * SIDE + row) * shape.cols + cx * SIDE;
            for col in bits(flipped) {
                flips ^= cell_key((first + col) as u64);
            }
        }
        if rows.any != 0 {
            changes.push(ChunkChange { slot, next, rows });
        }
    }
    (live_delta, flips)
}

/// A well-mixed key per cell (SplitMix64), so that no set of flips is likely to
/// cancel out unless each cell flipped back.
pub(crate) fn cell_key(idx: u64) -> u64 {
    let value = idx.wrapping_add(1).wrapping_mul(0x9E3779B97F4A7C15);
    let value = (value ^ (value >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    let value = (value ^ (value >> 27)).wrapping_mul(0x94D049BB133111EB);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::{
        Board, BoundaryMode, DirtyCells, MIN_BAND_CHUNKS, RandomizeSettings, Rule, RuleParseError,
        cell_key,
    };

    fn live_cells(board: &Board) -> Vec<(usize, usize)> {
        let mut cells: Vec<_> = board.live_cells().collect();
        cells.sort_unstable();
        cells
    }

    /// The index of each changed cell, lowest first.
    fn changed(dirty: DirtyCells) -> Vec<usize> {
        let DirtyCells::Cells(rows) = dirty else {
            panic!("expected changed cells, not {dirty:?}");
        };
        let mut cells: Vec<_> = rows
            .into_iter()
            .flat_map(|run| run.iter().map(|(idx, _)| idx))
            .collect();
        cells.sort_unstable();
        cells
    }

    /// A board stepped a cell at a time, with nothing to get wrong but the rule.
    #[derive(Clone)]
    struct Reference {
        cols: usize,
        rows: usize,
        cells: Vec<bool>,
        mode: BoundaryMode,
        rule: Rule,
        fingerprint: u64,
    }

    impl Reference {
        fn of(board: &Board) -> Self {
            let (cols, rows) = board.size();
            Self {
                cols,
                rows,
                cells: (0..cols * rows)
                    .map(|idx| board.is_alive(idx % cols, idx / cols))
                    .collect(),
                mode: board.boundary_mode(),
                rule: board.rule(),
                fingerprint: board.fingerprint(),
            }
        }

        fn neighbours(&self, col: usize, row: usize) -> u32 {
            let mut count = 0;
            for (dx, dy) in [
                (-1, -1),
                (0, -1),
                (1, -1),
                (-1, 0),
                (1, 0),
                (-1, 1),
                (0, 1),
                (1, 1),
            ] {
                let (x, y) = (col as isize + dx, row as isize + dy);
                let (cols, rows) = (self.cols as isize, self.rows as isize);
                let cell = match self.mode {
                    BoundaryMode::Wrap => Some((x.rem_euclid(cols), y.rem_euclid(rows))),
                    BoundaryMode::Fixed => {
                        ((0..cols).contains(&x) && (0..rows).contains(&y)).then_some((x, y))
                    }
                };
                count += cell.is_some_and(|(x, y)| self.cells[(y * cols + x) as usize]) as u32;
            }
            count
        }

        /// Returns the cells that flipped, lowest first.
        fn step(&mut self) -> Vec<usize> {
            let mask = self.rule.transition_mask();
            let next: Vec<bool> = (0..self.cells.len())
                .map(|idx| {
                    let count = self.neighbours(idx % self.cols, idx / self.cols);
                    mask >> (count + 9 * u32::from(self.cells[idx])) & 1 != 0
                })
                .collect();
            let flipped: Vec<usize> = (0..next.len())
                .filter(|&idx| next[idx] != self.cells[idx])
                .collect();
            for &idx in &flipped {
                self.fingerprint ^= cell_key(idx as u64);
            }
            self.cells = next;
            flipped
        }
    }

    fn assert_same(board: &Board, reference: &Reference, context: &str) {
        for (idx, &alive) in reference.cells.iter().enumerate() {
            let (col, row) = (idx % reference.cols, idx / reference.cols);
            assert_eq!(
                board.is_alive(col, row),
                alive,
                "cell ({col}, {row}), {context}"
            );
        }
        let live = reference.cells.iter().filter(|&&alive| alive).count();
        assert_eq!(board.live_cells_count(), live, "{context}");
        assert_eq!(board.fingerprint(), reference.fingerprint, "{context}");
        // No cell past the edges may come alive, where it would be counted and drawn.
        let stored: u32 = board
            .chunks
            .iter()
            .flatten()
            .map(|row| row.count_ones())
            .sum();
        assert_eq!(stored as usize, live, "{context}");
    }

    fn soup(cols: usize, rows: usize, density: f64, seed: u64) -> Board {
        let mut rng = fastrand::Rng::with_seed(seed);
        let mut board = Board::new(cols, rows);
        for row in 0..rows {
            for col in 0..cols {
                if rng.f64() < density {
                    assert!(board.populate(col, row));
                }
            }
        }
        board
    }

    #[test]
    fn rulestrings_parse_in_every_accepted_ordering() {
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
        for (col, row) in [(1, 1), (1, 2), (2, 1), (2, 2)] {
            board.populate(col, row);
        }
        let before = live_cells(&board);
        assert!(!board.step());
        assert_eq!(live_cells(&board), before);
    }

    #[test]
    fn blinker_oscillates_every_step() {
        let mut board = Board::new(5, 5);
        for col in 1..4 {
            board.populate(col, 2);
        }

        assert!(board.step());
        assert_eq!(live_cells(&board), vec![(2, 1), (2, 2), (2, 3)]);
        assert!(board.step());
        assert_eq!(live_cells(&board), vec![(1, 2), (2, 2), (3, 2)]);
    }

    #[test]
    fn wraparound_neighbors_are_counted() {
        let mut board = Board::new(5, 5);
        board.set_boundary_mode(BoundaryMode::Wrap);
        for (col, row) in [(4, 4), (4, 0), (0, 4)] {
            board.populate(col, row);
        }
        board.step();
        assert!(board.is_alive(0, 0));
    }

    #[test]
    fn fixed_boundary_does_not_wrap_neighbors() {
        let mut board = Board::new(5, 5);
        board.set_boundary_mode(BoundaryMode::Fixed);
        for (col, row) in [(4, 4), (4, 0), (0, 4)] {
            board.populate(col, row);
        }
        board.step();
        assert!(!board.is_alive(0, 0));
    }

    #[test]
    fn glider_moves_diagonally_after_four_steps() {
        let mut board = Board::new(10, 10);
        for (col, row) in [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)] {
            board.populate(col, row);
        }
        for _ in 0..4 {
            board.step();
        }
        assert_eq!(
            live_cells(&board),
            vec![(1, 3), (2, 1), (2, 3), (3, 2), (3, 3)]
        );
    }

    #[test]
    fn stepping_matches_a_cell_by_cell_reference_at_every_size_edge_and_rule() {
        // Sizes that end mid-chunk, on a chunk's edge, and inside a single chunk,
        // across and up, and too narrow for a cell to have eight neighbours.
        let sizes = [
            (8, 8),
            (3, 5),
            (2, 2),
            (1, 4),
            (64, 64),
            (65, 63),
            (70, 130),
            (128, 64),
            (200, 71),
        ];
        // B0 gives birth in empty space, which only a finite board allows.
        let rules = ["B3/S23", "B36/S23", "B3678/S34678", "B2/S", "B03/S23"];
        for (cols, rows) in sizes {
            for mode in [BoundaryMode::Wrap, BoundaryMode::Fixed] {
                for (index, rule) in rules.into_iter().enumerate() {
                    let rule = Rule::parse(rule).unwrap();
                    let mut board = soup(cols, rows, 0.35, (cols * 1000 + rows + index) as u64);
                    board.set_boundary_mode(mode);
                    board.set_rule(rule);
                    let mut reference = Reference::of(&board);
                    let _ = board.take_dirty_cells();

                    for generation in 1..=20 {
                        let context = format!("{cols}x{rows} {mode:?} {rule} gen {generation}");
                        let flipped = reference.step();
                        assert_eq!(board.step(), !flipped.is_empty(), "{context}");
                        assert_same(&board, &reference, &context);
                        match board.take_dirty_cells() {
                            DirtyCells::None => assert!(flipped.is_empty(), "{context}"),
                            dirty => assert_eq!(changed(dirty), flipped, "{context}"),
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_rule_with_b0_fills_an_empty_board() {
        let mut board = Board::new(70, 9);
        board.set_rule(Rule::parse("B0/S8").unwrap());
        assert!(board.step());
        assert_eq!(board.live_cells_count(), 70 * 9);
    }

    #[test]
    fn edits_between_generations_match_the_reference() {
        let (cols, rows) = (150, 90);
        for mode in [BoundaryMode::Wrap, BoundaryMode::Fixed] {
            let mut board = soup(cols, rows, 0.2, 9);
            board.set_boundary_mode(mode);
            let mut reference = Reference::of(&board);
            let mut rng = fastrand::Rng::with_seed(3);
            for generation in 0..40 {
                // Corners and edges as often as anywhere else.
                for _ in 0..8 {
                    let col = [0, 63, 64, cols - 1, rng.usize(..cols)][rng.usize(..5)];
                    let row = [0, 63, 64, rows - 1, rng.usize(..rows)][rng.usize(..5)];
                    let alive = rng.bool();
                    let cell = &mut reference.cells[row * cols + col];
                    let edited = if alive {
                        board.populate(col, row)
                    } else {
                        board.depopulate(col, row)
                    };
                    assert_eq!(edited, *cell != alive);
                    *cell = alive;
                }
                assert_eq!(board.step(), !reference.step().is_empty());
                assert_same(&board, &reference, &format!("{mode:?} gen {generation}"));
            }
        }
    }

    #[test]
    fn a_settled_board_wakes_for_an_edit_a_rule_or_new_edges() {
        let mut board = Board::new(200, 140);
        // A block on the east edge, still under either kind of edge.
        for (col, row) in [(198, 70), (199, 70), (198, 71), (199, 71)] {
            board.populate(col, row);
        }
        assert!(!board.step());
        assert!(board.active_list.is_empty());

        // A cell on the west edge that only sees the block round the board.
        board.populate(0, 69);
        let mut reference = Reference::of(&board);
        for _ in 0..4 {
            assert_eq!(board.step(), !reference.step().is_empty());
        }
        assert_same(&board, &reference, "after an edit");

        board.toggle_boundary_mode();
        assert_eq!(board.boundary_mode(), BoundaryMode::Fixed);
        assert_eq!(board.boundary_mode_label(), "Fixed");
        let seeds = Rule::parse("B2/S").unwrap();
        assert!(board.set_rule(seeds));
        assert!(!board.set_rule(seeds));
        let mut reference = Reference::of(&board);
        for _ in 0..12 {
            assert_eq!(board.step(), !reference.step().is_empty());
        }
        assert_same(&board, &reference, "after new edges and a new rule");
    }

    #[test]
    fn bands_step_like_a_serial_scan() {
        let mut banded = soup(600, 520, 0.3, 21);
        let mut serial = banded.clone();
        assert!(banded.active_list.len() >= 4 * MIN_BAND_CHUNKS);
        for generation in 0..12 {
            assert_eq!(banded.step_in_bands(4), serial.step_in_bands(1));
            assert_eq!(banded.chunks, serial.chunks, "generation {generation}");
            assert_eq!(banded.live_cells_count(), serial.live_cells_count());
            assert_eq!(banded.fingerprint(), serial.fingerprint());
        }
    }

    #[test]
    fn work_stays_on_the_rows_beside_a_change() {
        let mut board = Board::new(1024, 1024);
        for (col, row) in [(501, 500), (502, 501), (500, 502), (501, 502), (502, 502)] {
            board.populate(col, row);
        }
        for _ in 0..200 {
            assert!(board.step());
            let rows: u32 = board
                .active_list
                .iter()
                .map(|&slot| board.active[slot as usize].count_ones())
                .sum();
            assert!(board.active_list.len() <= 4 && rows <= 12, "{rows} rows");
        }
        assert_eq!(board.live_cells_count(), 5);
    }

    #[test]
    fn the_fingerprint_returns_with_the_cells() {
        let mut board = Board::new(8, 8);
        for col in 3..6 {
            board.populate(col, 4);
        }
        let start = board.fingerprint();
        board.step();
        assert_ne!(board.fingerprint(), start);
        board.step();
        assert_eq!(board.fingerprint(), start);
    }

    #[test]
    fn populate_marks_single_dirty_cell() {
        let mut board = Board::new(4, 4);
        assert_eq!(board.take_dirty_cells(), DirtyCells::Full);

        board.populate(2, 1);
        assert_eq!(changed(board.take_dirty_cells()), vec![6]);
        assert_eq!(board.take_dirty_cells(), DirtyCells::None);
    }

    #[test]
    fn step_marks_only_changed_cells_and_says_which_are_alive() {
        let mut board = Board::new(5, 5);
        for col in 1..4 {
            board.populate(col, 2);
        }
        let _ = board.take_dirty_cells();

        board.step();

        let DirtyCells::Cells(rows) = board.take_dirty_cells() else {
            panic!("expected incremental dirty cells");
        };
        let mut cells: Vec<_> = rows.into_iter().flat_map(|run| run.iter()).collect();
        cells.sort_unstable();
        assert_eq!(cells, vec![(7, true), (11, false), (13, false), (17, true)]);
    }

    #[test]
    fn edits_and_clearing_report_what_changed() {
        let mut board = Board::new(100, 70);
        assert_eq!(board.take_dirty_cells(), DirtyCells::Full);
        assert_eq!(board.take_dirty_cells(), DirtyCells::None);

        assert!(board.populate(99, 69));
        assert!(!board.populate(99, 69));
        assert!(!board.populate(100, 69) && !board.populate(99, 70));
        assert!(board.is_alive(99, 69) && !board.is_alive(100, 69));
        assert_eq!(board.live_cells_count(), 1);
        assert_eq!(changed(board.take_dirty_cells()), vec![69 * 100 + 99]);

        assert!(board.depopulate(99, 69));
        assert!(!board.depopulate(99, 69));
        assert!(!board.clear());
        board.populate(5, 5);
        assert!(board.clear());
        assert_eq!(board.live_cells_count(), 0);
        assert_eq!(board.take_dirty_cells(), DirtyCells::Full);
        assert!(!board.step());
    }

    #[test]
    fn resize_preserves_overlapping_cells_only() {
        let mut board = Board::new(4, 4);
        board.populate(0, 0);
        board.populate(3, 3);

        assert!(!board.resize(4, 4));
        assert!(board.resize(2, 3));

        assert_eq!(board.size(), (2, 3));
        assert_eq!(live_cells(&board), vec![(0, 0)]);
        assert_eq!(board.take_dirty_cells(), DirtyCells::Full);
    }

    #[test]
    fn resizing_across_chunks_keeps_what_fits_and_steps_on() {
        let sizes = [
            (200, 150),
            (64, 64),
            (130, 70),
            (63, 200),
            (300, 10),
            (65, 65),
        ];
        let mut board = soup(100, 100, 0.3, 5);
        for (cols, rows) in sizes {
            let before = Reference::of(&board);
            assert!(board.resize(cols, rows));
            assert_eq!(board.size(), (cols, rows));

            let mut reference = Reference::of(&board);
            for (idx, &alive) in reference.cells.iter().enumerate() {
                let (col, row) = (idx % cols, idx / cols);
                let kept = col < before.cols && row < before.rows;
                assert_eq!(
                    alive,
                    kept && before.cells[row * before.cols + col],
                    "cell ({col}, {row}) at {cols}x{rows}"
                );
            }
            for _ in 0..3 {
                reference.step();
                board.step();
            }
            assert_same(&board, &reference, &format!("{cols}x{rows}"));
        }
    }

    #[test]
    fn a_snapshot_restores_the_cells_and_wakes_the_board() {
        let mut board = soup(130, 70, 0.3, 11);
        let snapshot = board.snapshot_cells();
        let mut reference = Reference::of(&board);
        // A chunk of 64 rows of 8 bytes for each of the six the board covers.
        assert_eq!(snapshot.bytes(), 6 * 512);

        board.clear();
        assert!(!board.step());
        assert_ne!(board.snapshot_cells(), snapshot);
        let _ = board.take_dirty_cells();

        board.restore_snapshot(&snapshot);
        assert_eq!(board.snapshot_cells(), snapshot);
        assert_eq!(board.take_dirty_cells(), DirtyCells::Full);
        reference.fingerprint = board.fingerprint();
        assert_same(&board, &reference, "restored");
        assert_eq!(board.step(), !reference.step().is_empty());
        assert_same(&board, &reference, "restored and stepped");
    }

    #[test]
    fn replacing_cells_drops_the_old_ones_and_those_off_the_board() {
        let mut board = soup(70, 20, 0.3, 2);
        board.replace_cells([(0, 0), (69, 19), (70, 19), (5, 20), (64, 3)]);
        assert_eq!(live_cells(&board), vec![(0, 0), (64, 3), (69, 19)]);
        assert_eq!(board.live_cells_count(), 3);
        assert_eq!(board.take_dirty_cells(), DirtyCells::Full);
    }

    #[test]
    fn every_cell_comes_row_by_row_in_runs_from_its_first() {
        let board = soup(130, 3, 0.4, 8);
        let mut expected = 0;
        for run in board.row_cells() {
            assert_eq!(run.first, expected);
            assert_eq!(run.cells, super::low_bits(run.len()));
            assert!(!run.is_empty());
            for (idx, alive) in run.iter() {
                assert_eq!(alive, board.is_alive(idx % 130, idx / 130));
            }
            expected += run.len();
        }
        assert_eq!(expected, 130 * 3);

        // Live cells come in the same order, which is the order RLE is written in.
        let by_cell: Vec<_> = (0..3)
            .flat_map(|row| (0..130).map(move |col| (col, row)))
            .filter(|&(col, row)| board.is_alive(col, row))
            .collect();
        assert_eq!(board.live_cells().collect::<Vec<_>>(), by_cell);
    }

    #[test]
    fn the_live_bounds_span_chunks() {
        let mut board = Board::new(200, 150);
        assert_eq!(board.live_bounds(), None);
        board.populate(70, 5);
        assert_eq!(board.live_bounds(), Some(((70, 5), (70, 5))));
        board.populate(3, 140);
        board.populate(199, 64);
        assert_eq!(board.live_bounds(), Some(((3, 5), (199, 140))));
    }

    #[test]
    fn the_overview_counts_every_live_cell_in_its_block() {
        let mut board = Board::new(10, 7);
        for (col, row) in [(0, 0), (9, 0), (9, 6), (4, 3), (5, 3)] {
            board.populate(col, row);
        }
        let mut counts = vec![0; 5 * 3];
        board.overview(5, 3, &mut counts);
        assert_eq!(counts.iter().sum::<u32>(), 5);
        assert_eq!((counts[0], counts[4], counts[14]), (1, 1, 1));

        // At full size, a block is a cell.
        let mut counts = vec![0; 10 * 7];
        board.overview(10, 7, &mut counts);
        assert_eq!(counts[3 * 10 + 4], 1);
        assert_eq!(counts.iter().sum::<u32>(), 5);
    }

    #[test]
    fn the_overview_matches_counting_cell_by_cell_across_chunks() {
        let (cols, rows) = (300, 137);
        let board = soup(cols, rows, 0.3, 4);
        for (width, height) in [(1, 1), (7, 5), (64, 64), (100, 137), (300, 100)] {
            let mut counts = vec![0; width * height];
            board.overview(width, height, &mut counts);

            let mut expected = vec![0; width * height];
            for row in 0..rows {
                for block in 0..width {
                    let span = block * cols / width..(block + 1) * cols / width;
                    expected[row * height / rows * width + block] +=
                        span.filter(|&col| board.is_alive(col, row)).count() as u32;
                }
            }
            assert_eq!(counts, expected, "{width}x{height}");
        }
    }

    #[test]
    fn seeded_randomization_has_a_stable_sequence_and_replaces_previous_cells() {
        let settings = RandomizeSettings::new(25, Some(42)).unwrap();
        let mut board = Board::new(8, 8);
        board.populate_random_with(settings);
        let original = board.snapshot_cells();
        let sequence: String = (0..64)
            .map(|idx| {
                if board.is_alive(idx % 8, idx / 8) {
                    '1'
                } else {
                    '0'
                }
            })
            .collect();
        assert_eq!(
            sequence,
            "1000000110100000000100000000011000000001000000000000000100000010"
        );
        board.populate_random_with(RandomizeSettings::new(100, None).unwrap());
        board.populate_random_with(settings);
        assert_eq!(board.snapshot_cells(), original);
        board.populate_random_with(RandomizeSettings::new(25, Some(43)).unwrap());
        assert_ne!(board.snapshot_cells(), original);
        assert_eq!(board.live_cells_count(), live_cells(&board).len());
        assert_eq!(board.take_dirty_cells(), DirtyCells::Full);
    }

    #[test]
    fn a_seeded_sequence_runs_row_by_row_whatever_the_chunks() {
        // The draws a wider board makes for its first rows are the draws a board
        // of one chunk makes for all of its cells.
        let settings = RandomizeSettings::new(40, Some(7)).unwrap();
        let (mut wide, mut flat) = (Board::new(130, 2), Board::new(260, 1));
        wide.populate_random_with(settings);
        flat.populate_random_with(settings);
        for idx in 0..260 {
            assert_eq!(wide.is_alive(idx % 130, idx / 130), flat.is_alive(idx, 0));
        }
    }

    #[test]
    fn configured_randomization_handles_endpoints_and_expected_densities() {
        assert!(RandomizeSettings::new(101, None).is_none());
        let mut board = Board::new(500, 250);
        for (percent, seed) in [(100, 0), (0, u64::MAX), (10, 123), (75, 123)] {
            board.populate_random_with(RandomizeSettings::new(percent, Some(seed)).unwrap());
            let density = board.live_cells_count() as f64 / (500.0 * 250.0);
            if percent == 0 || percent == 100 {
                assert_eq!(density, f64::from(percent) / 100.0);
            } else {
                assert!((density - f64::from(percent) / 100.0).abs() < 0.015);
            }
            assert_eq!(board.live_cells_count(), live_cells(&board).len());
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
}
