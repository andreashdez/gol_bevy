//! An unbounded board. Live cells sit in a sparse map of fixed-size chunks, so
//! memory follows the live cells rather than any board size, and a generation
//! costs time proportional to what changed in the last one.
use crate::board::{Rule, cell_key};
use bevy::{
    platform::collections::HashMap,
    tasks::{ComputeTaskPool, TaskPool},
};
use std::sync::Arc;

const CHUNK_BITS: u32 = 6;
/// Width and height of a chunk in cells.
pub const CHUNK_SIZE: i64 = 1 << CHUNK_BITS;
const CHUNK_MASK: i64 = CHUNK_SIZE - 1;
/// Index of a chunk's last row, and of its last column.
const LAST_LINE: u32 = CHUNK_SIZE as u32 - 1;

/// One row per `u64`, with cell `x` in bit `x`. A row holds as many cells as a
/// machine word, so a generation updates them all at once.
pub type Chunk = [u64; CHUNK_SIZE as usize];
/// A chunk's position in chunks: the cell's coordinates, divided by [`CHUNK_SIZE`]
/// and rounded down, so cell -1 is in chunk -1.
pub type ChunkKey = (i64, i64);

static EMPTY_CHUNK: Chunk = [0; CHUNK_SIZE as usize];

/// Fewest scheduled chunks that justify another parallel band, matching the cell
/// count [`Board`](crate::board::Board) asks of a band.
const MIN_BAND_CHUNKS: usize = 16;

/// Changed chunks tracked between render syncs before settling for a full redraw.
/// Nothing else bounds the list, as a run may cross new chunks forever between syncs.
const DIRTY_CHUNKS_MAX: usize = 1 << 14;

/// Returned for rules that would light every cell of the plane in one generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnboundedRuleError;

impl std::fmt::Display for UnboundedRuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "a rule with B0 would fill the unbounded plane")
    }
}

impl std::error::Error for UnboundedRuleError {}

/// Cells changed since the last render sync. Chunks hold a mask of changed
/// cells, not their states, and a chunk may have gone empty since.
#[derive(Debug, PartialEq, Eq)]
pub enum DirtyChunks {
    None,
    Full,
    Chunks(HashMap<ChunkKey, Chunk>),
}

/// Chunks of an earlier state. They are shared with the board until it changes
/// them, so taking a snapshot costs a pointer per chunk.
#[derive(Debug, Clone)]
pub struct SparseSnapshot(HashMap<ChunkKey, Arc<Chunk>>);

/// Which rows of a chunk changed: any of their cells, and those in its first and
/// last columns, which is what the chunks either side of it can see.
#[derive(Debug, Clone, Copy, Default)]
struct ChangedRows {
    any: u64,
    west: u64,
    east: u64,
}

impl ChangedRows {
    const ALL: Self = Self {
        any: !0,
        west: !0,
        east: !0,
    };

    fn cell(x: usize, y: usize) -> Self {
        let row = 1 << y;
        Self {
            any: row,
            west: if x == 0 { row } else { 0 },
            east: if x as u32 == LAST_LINE { row } else { 0 },
        }
    }
}

/// A chunk's next generation, staged so bands can share the old one.
#[derive(Debug, Clone)]
struct ChunkChange {
    key: ChunkKey,
    next: Chunk,
    rows: ChangedRows,
}

#[derive(Debug, Clone)]
pub struct SparseBoard {
    /// live chunks. A chunk with no live cells is removed.
    chunks: HashMap<ChunkKey, Arc<Chunk>>,
    /// rows to evaluate next generation, by chunk: those that changed last
    /// generation and those next to them, here or in a neighbouring chunk
    active: HashMap<ChunkKey, u64>,
    /// changed cells since last render sync
    dirty_chunks: HashMap<ChunkKey, Chunk>,
    /// request a full redraw on next render sync
    dirty_full: bool,
    /// recycled buffer for the generation's scheduled chunks
    work_scratch: Vec<(ChunkKey, u64)>,
    /// recycled lists for each band's changes
    change_scratch: Vec<Vec<ChunkChange>>,
    /// running count of live cells, kept in step with every mutation
    live_count: usize,
    /// XOR of the key of every cell flipped by stepping, so generations of one run
    /// match exactly when their cells do. Edits leave it alone.
    fingerprint: u64,
    /// birth/survival rule applied by each step
    rule: Rule,
}

impl Default for SparseBoard {
    fn default() -> Self {
        Self {
            chunks: HashMap::default(),
            active: HashMap::default(),
            dirty_chunks: HashMap::default(),
            dirty_full: true,
            work_scratch: Vec::new(),
            change_scratch: Vec::new(),
            live_count: 0,
            fingerprint: 0,
            rule: Rule::LIFE,
        }
    }
}

impl SparseBoard {
    pub fn populate(&mut self, x: i64, y: i64) -> bool {
        let (key, col, row) = locate(x, y);
        let chunk = self
            .chunks
            .entry(key)
            .or_insert_with(|| Arc::new(EMPTY_CHUNK));
        if chunk[row] >> col & 1 != 0 {
            return false;
        }
        Arc::make_mut(chunk)[row] |= 1 << col;
        self.live_count += 1;
        self.mark_dirty(key, row, 1 << col);
        wake(&mut self.active, key, ChangedRows::cell(col, row));
        true
    }

    pub fn depopulate(&mut self, x: i64, y: i64) -> bool {
        let (key, col, row) = locate(x, y);
        let Some(chunk) = self.chunks.get_mut(&key) else {
            return false;
        };
        if chunk[row] >> col & 1 == 0 {
            return false;
        }
        Arc::make_mut(chunk)[row] &= !(1 << col);
        if **chunk == EMPTY_CHUNK {
            self.chunks.remove(&key);
        }
        self.live_count -= 1;
        self.mark_dirty(key, row, 1 << col);
        wake(&mut self.active, key, ChangedRows::cell(col, row));
        true
    }

    pub fn clear(&mut self) -> bool {
        if self.chunks.is_empty() {
            return false;
        }
        self.chunks.clear();
        self.active.clear();
        self.live_count = 0;
        self.mark_dirty_full();
        true
    }

    pub fn step(&mut self) -> bool {
        let threads = ComputeTaskPool::get_or_init(TaskPool::default).thread_num();
        self.step_in_bands(threads.min(self.active.len() / MIN_BAND_CHUNKS))
    }

    /// Evaluates the scheduled rows in up to `bands` bands of whole chunks, in
    /// parallel on the compute task pool when there is more than one. Bands only
    /// read the chunks and stage their changes, which are applied once all have
    /// finished, so no band can see another's half of the generation.
    fn step_in_bands(&mut self, bands: usize) -> bool {
        if self.active.is_empty() {
            return false;
        }

        let mut work = std::mem::take(&mut self.work_scratch);
        work.clear();
        work.extend(self.active.drain());
        let band_len = work.len().div_ceil(bands.max(1));
        let mut changes = std::mem::take(&mut self.change_scratch);
        changes.resize_with(work.len().div_ceil(band_len), Vec::new);

        let (chunks, rule) = (&self.chunks, self.rule);
        let mut bands = work.chunks(band_len).zip(&mut changes);
        let (live_delta, flips) = if bands.len() == 1 {
            let (band, changes) = bands.next().expect("scheduled work has a band");
            step_band(chunks, rule, band, changes)
        } else {
            ComputeTaskPool::get_or_init(TaskPool::default)
                .scope(|scope| {
                    for (band, changes) in bands {
                        scope.spawn(async move { step_band(chunks, rule, band, changes) });
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
        let ChunkChange { key, next, rows } = change;
        if !self.dirty_full {
            let old = self.chunks.get(key).map_or(&EMPTY_CHUNK, |chunk| &**chunk);
            let mask = self.dirty_chunks.entry(*key).or_insert(EMPTY_CHUNK);
            for row in bits(rows.any) {
                mask[row] |= old[row] ^ next[row];
            }
            if self.dirty_chunks.len() > DIRTY_CHUNKS_MAX {
                self.mark_dirty_full();
            }
        }
        wake(&mut self.active, *key, *rows);

        if *next == EMPTY_CHUNK {
            self.chunks.remove(key);
        } else if let Some(chunk) = self.chunks.get_mut(key) {
            // A chunk still held by a snapshot has to be left as it is.
            match Arc::get_mut(chunk) {
                Some(chunk) => *chunk = *next,
                None => *chunk = Arc::new(*next),
            }
        } else {
            self.chunks.insert(*key, Arc::new(*next));
        }
    }

    pub fn rule(&self) -> Rule {
        self.rule
    }

    /// Returns whether the rule actually changed. A different rule invalidates the
    /// schedule, since cells that were stable under the old rule may not be.
    pub fn set_rule(&mut self, rule: Rule) -> Result<bool, UnboundedRuleError> {
        if rule.births_from_nothing() {
            return Err(UnboundedRuleError);
        }
        if self.rule == rule {
            return Ok(false);
        }
        self.rule = rule;
        self.wake_all();
        Ok(true)
    }

    pub fn is_alive(&self, x: i64, y: i64) -> bool {
        let (key, col, row) = locate(x, y);
        self.chunks
            .get(&key)
            .is_some_and(|chunk| chunk[row] >> col & 1 != 0)
    }

    /// Coordinates of every live cell, in no particular order.
    pub fn live_cells(&self) -> impl Iterator<Item = (i64, i64)> + '_ {
        self.chunks.iter().flat_map(|(&key, chunk)| {
            let (left, bottom) = chunk_origin(key);
            chunk.iter().enumerate().flat_map(move |(row, &cells)| {
                bits(cells).map(move |col| (left + col as i64, bottom + row as i64))
            })
        })
    }

    pub fn live_cells_count(&self) -> usize {
        self.live_count
    }

    /// Chunks in memory, each of [`CHUNK_SIZE`] squared bits plus some overhead.
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Inclusive bounds, computed on demand rather than during simulation steps.
    pub fn live_bounds(&self) -> Option<((i64, i64), (i64, i64))> {
        let mut bounds: Option<((i64, i64), (i64, i64))> = None;
        for (&key, chunk) in &self.chunks {
            let (left, bottom) = chunk_origin(key);
            let columns = chunk.iter().fold(0, |columns, &cells| columns | cells);
            let first_row = chunk.iter().position(|&cells| cells != 0);
            let last_row = chunk.iter().rposition(|&cells| cells != 0);
            let (Some(first_row), Some(last_row)) = (first_row, last_row) else {
                continue;
            };
            let min = (
                left + i64::from(columns.trailing_zeros()),
                bottom + first_row as i64,
            );
            let max = (
                left + i64::from(LAST_LINE - columns.leading_zeros()),
                bottom + last_row as i64,
            );
            bounds = Some(bounds.map_or((min, max), |(low, high)| {
                (
                    (low.0.min(min.0), low.1.min(min.1)),
                    (high.0.max(max.0), high.1.max(max.1)),
                )
            }));
        }
        bounds
    }

    /// Identifies the current generation among others of the same run: stepping
    /// back to the same cells restores it. Only comparable while nothing but
    /// stepping changes the board.
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    pub fn snapshot(&self) -> SparseSnapshot {
        SparseSnapshot(self.chunks.clone())
    }

    pub fn restore(&mut self, snapshot: SparseSnapshot) {
        self.chunks = snapshot.0;
        self.live_count = self
            .chunks
            .values()
            .flat_map(|chunk| chunk.iter())
            .map(|cells| cells.count_ones() as usize)
            .sum();
        self.active.clear();
        self.wake_all();
        self.mark_dirty_full();
    }

    pub fn take_dirty_chunks(&mut self) -> DirtyChunks {
        if self.dirty_full {
            self.dirty_full = false;
            debug_assert!(self.dirty_chunks.is_empty());
            return DirtyChunks::Full;
        }
        if self.dirty_chunks.is_empty() {
            return DirtyChunks::None;
        }
        DirtyChunks::Chunks(std::mem::take(&mut self.dirty_chunks))
    }

    /// Schedules every row of every chunk, and whatever those rows could touch.
    fn wake_all(&mut self) {
        for &key in self.chunks.keys() {
            wake(&mut self.active, key, ChangedRows::ALL);
        }
    }

    fn mark_dirty(&mut self, key: ChunkKey, row: usize, cells: u64) {
        if self.dirty_full {
            return;
        }
        self.dirty_chunks.entry(key).or_insert(EMPTY_CHUNK)[row] |= cells;
        if self.dirty_chunks.len() > DIRTY_CHUNKS_MAX {
            self.mark_dirty_full();
        }
    }

    fn mark_dirty_full(&mut self) {
        self.dirty_chunks.clear();
        self.dirty_full = true;
    }
}

/// The chunk holding a cell, and the cell's column and row within it.
fn locate(x: i64, y: i64) -> (ChunkKey, usize, usize) {
    // The shift rounds down and the mask counts up from there, for negative
    // coordinates as much as positive.
    (
        (x >> CHUNK_BITS, y >> CHUNK_BITS),
        (x & CHUNK_MASK) as usize,
        (y & CHUNK_MASK) as usize,
    )
}

fn chunk_origin((x, y): ChunkKey) -> (i64, i64) {
    (x << CHUNK_BITS, y << CHUNK_BITS)
}

/// Positions of the set bits, lowest first.
fn bits(mut mask: u64) -> impl Iterator<Item = usize> {
    std::iter::from_fn(move || {
        (mask != 0).then(|| {
            let bit = mask.trailing_zeros() as usize;
            mask &= mask - 1;
            bit
        })
    })
}

/// A cell can only change after it or one of its neighbours has, so schedule the
/// rows beside the ones that changed, in this chunk and in any neighbour that can
/// see them. Scheduling rows, not chunks, keeps a lone glider from costing a chunk.
fn wake(active: &mut HashMap<ChunkKey, u64>, (x, y): ChunkKey, rows: ChangedRows) {
    let mut schedule = |key, rows| {
        if rows != 0 {
            *active.entry(key).or_default() |= rows;
        }
    };
    let spread = |rows: u64| rows | rows << 1 | rows >> 1;
    let to_last_row = |rows: u64| (rows & 1) << LAST_LINE;
    let to_first_row = |rows: u64| rows >> LAST_LINE;

    schedule((x, y), spread(rows.any));
    schedule((x - 1, y), spread(rows.west));
    schedule((x + 1, y), spread(rows.east));
    schedule((x, y - 1), to_last_row(rows.any));
    schedule((x - 1, y - 1), to_last_row(rows.west));
    schedule((x + 1, y - 1), to_last_row(rows.east));
    schedule((x, y + 1), to_first_row(rows.any));
    schedule((x - 1, y + 1), to_first_row(rows.west));
    schedule((x + 1, y + 1), to_first_row(rows.east));
}

/// A chunk's 3 × 3 neighbourhood, row by row, with the chunk itself in the middle.
type Around<'a> = [&'a Chunk; 9];

/// Stages the next state of the scheduled rows of `band`, returning the change in
/// the live count and the XOR of the keys of every cell that flipped.
fn step_band(
    chunks: &HashMap<ChunkKey, Arc<Chunk>>,
    rule: Rule,
    band: &[(ChunkKey, u64)],
    changes: &mut Vec<ChunkChange>,
) -> (isize, u64) {
    let mask = rule.transition_mask();
    let (mut live_delta, mut flips) = (0, 0);
    for &(key, scheduled) in band {
        let (x, y) = key;
        let get = |dx, dy| {
            chunks
                .get(&(x + dx, y + dy))
                .map_or(&EMPTY_CHUNK, |chunk| &**chunk)
        };
        #[rustfmt::skip]
        let around: Around = [
            get(-1, -1), get(0, -1), get(1, -1),
            get(-1, 0), get(0, 0), get(1, 0),
            get(-1, 1), get(0, 1), get(1, 1),
        ];

        let (left, bottom) = chunk_origin(key);
        let old = around[4];
        let mut next = *old;
        let mut rows = ChangedRows::default();
        for row in bits(scheduled) {
            next[row] = next_row(&around, row, mask);
            let flipped = old[row] ^ next[row];
            if flipped == 0 {
                continue;
            }
            rows.any |= 1 << row;
            rows.west |= (flipped & 1) << row;
            rows.east |= (flipped >> LAST_LINE) << row;
            live_delta += next[row].count_ones() as isize - old[row].count_ones() as isize;
            let row_key = cell_key((bottom + row as i64) as u64);
            for col in bits(flipped) {
                flips ^= cell_key(cell_key((left + col as i64) as u64) ^ row_key);
            }
        }
        if rows.any != 0 {
            changes.push(ChunkChange { key, next, rows });
        }
    }
    (live_delta, flips)
}

/// A row's cells and the neighbours either side of each, as three aligned planes.
#[derive(Clone, Copy)]
struct RowPlanes {
    west: u64,
    centre: u64,
    east: u64,
}

impl RowPlanes {
    /// Row `row` of the three chunks starting at `around[first]`.
    fn new(around: &Around, first: usize, row: usize) -> Self {
        let centre = around[first + 1][row];
        Self {
            west: centre << 1 | around[first][row] >> LAST_LINE,
            centre,
            east: centre >> 1 | around[first + 2][row] << LAST_LINE,
        }
    }

    fn any(self) -> u64 {
        self.west | self.centre | self.east
    }
}

/// Adds a one-bit plane to a four-bit counter for each of the 64 cells of a row.
fn add_plane(counters: &mut [u64; 4], plane: u64) {
    let mut carry = plane;
    for bit in counters {
        let next_carry = *bit & carry;
        *bit ^= carry;
        carry = next_carry;
    }
}

/// The next state of one row of the middle chunk, 64 cells at a time. `mask` is
/// [`Rule::transition_mask`].
fn next_row(around: &Around, row: usize, mask: u32) -> u64 {
    let last = LAST_LINE as usize;
    let below = if row == 0 {
        RowPlanes::new(around, 0, last)
    } else {
        RowPlanes::new(around, 3, row - 1)
    };
    let level = RowPlanes::new(around, 3, row);
    let above = if row == last {
        RowPlanes::new(around, 6, 0)
    } else {
        RowPlanes::new(around, 3, row + 1)
    };
    if below.any() | level.any() | above.any() == 0 {
        return 0;
    }

    let mut counters = [0; 4];
    for plane in [
        below.west,
        below.centre,
        below.east,
        level.west,
        level.east,
        above.west,
        above.centre,
        above.east,
    ] {
        add_plane(&mut counters, plane);
    }

    let mut next = 0;
    let (born, survives) = (mask & 0x1FF, mask >> 9);
    for count in bits(u64::from(born | survives)) {
        let matches = counters
            .iter()
            .enumerate()
            .fold(!0, |matches, (bit, &counter)| {
                matches
                    & if count >> bit & 1 != 0 {
                        counter
                    } else {
                        !counter
                    }
            });
        let takes_dead = if born >> count & 1 != 0 {
            !level.centre
        } else {
            0
        };
        let takes_live = if survives >> count & 1 != 0 {
            level.centre
        } else {
            0
        };
        next |= matches & (takes_dead | takes_live);
    }
    next
}

#[cfg(test)]
mod tests {
    use super::{CHUNK_SIZE, DIRTY_CHUNKS_MAX, DirtyChunks, SparseBoard, UnboundedRuleError};
    use crate::board::{Board, BoundaryMode, Rule};
    use std::collections::{HashMap, HashSet};

    const GLIDER: [(i64, i64); 5] = [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];

    fn board_with(cells: impl IntoIterator<Item = (i64, i64)>) -> SparseBoard {
        let mut board = SparseBoard::default();
        for (x, y) in cells {
            board.populate(x, y);
        }
        board
    }

    fn sorted(cells: impl IntoIterator<Item = (i64, i64)>) -> Vec<(i64, i64)> {
        let mut cells: Vec<_> = cells.into_iter().collect();
        cells.sort_unstable();
        cells
    }

    fn live_cells(board: &SparseBoard) -> Vec<(i64, i64)> {
        sorted(board.live_cells())
    }

    /// Scattered cells centred on the corner where four chunks meet.
    fn soup(cells: usize, spread: i64) -> Vec<(i64, i64)> {
        let mut rng = fastrand::Rng::with_seed(7);
        (0..cells)
            .map(|_| (rng.i64(-spread..spread), rng.i64(-spread..spread)))
            .collect()
    }

    /// One generation the slow way, as the definition of correct.
    fn brute_force_step(cells: &HashSet<(i64, i64)>, rule: Rule) -> HashSet<(i64, i64)> {
        let mut counts = HashMap::<_, u32>::new();
        for &(x, y) in cells {
            for (dx, dy) in (-1..=1).flat_map(|dy| (-1..=1).map(move |dx| (dx, dy))) {
                if (dx, dy) != (0, 0) {
                    *counts.entry((x + dx, y + dy)).or_default() += 1;
                }
            }
        }
        let mask = rule.transition_mask();
        counts
            .into_iter()
            .filter(|(cell, count)| {
                let alive = cells.contains(cell);
                mask >> (count + 9 * u32::from(alive)) & 1 != 0
            })
            .map(|(cell, _)| cell)
            .collect()
    }

    #[test]
    fn stepping_matches_brute_force_across_chunk_edges() {
        for rule in ["B3/S23", "B36/S23", "B2/S", "B35678/S5678"] {
            let rule = Rule::parse(rule).unwrap();
            let start = soup(3000, 100);
            let mut board = board_with(start.iter().copied());
            board.set_rule(rule).unwrap();
            let mut cells: HashSet<_> = start.into_iter().collect();
            for generation in 0..60 {
                board.step();
                cells = brute_force_step(&cells, rule);
                assert_eq!(
                    live_cells(&board),
                    sorted(cells.iter().copied()),
                    "{rule} generation {generation}",
                );
                assert_eq!(board.live_cells_count(), cells.len());
            }
        }
    }

    #[test]
    fn stepping_matches_the_finite_board_away_from_its_edges() {
        const OFFSET: i64 = 300;
        let cells = soup(4000, 100);
        let mut sparse = board_with(cells.iter().copied());
        let mut finite = Board::new(2 * OFFSET as usize, 2 * OFFSET as usize);
        finite.set_boundary_mode(BoundaryMode::Fixed);
        for &(x, y) in &cells {
            finite.populate((x + OFFSET) as usize, (y + OFFSET) as usize);
        }
        // Change spreads a cell per generation, so these can't reach the edges.
        for _ in 0..150 {
            sparse.step();
            finite.step();
        }
        let expected = (0..2 * OFFSET)
            .flat_map(|y| (0..2 * OFFSET).map(move |x| (x, y)))
            .filter(|&(x, y)| finite.is_alive(x as usize, y as usize))
            .map(|(x, y)| (x - OFFSET, y - OFFSET));
        assert_eq!(live_cells(&sparse), sorted(expected));
    }

    #[test]
    fn bands_step_like_a_serial_scan() {
        let cells = soup(40_000, 400);
        let mut serial = board_with(cells.iter().copied());
        let mut parallel = serial.clone();
        for _ in 0..30 {
            serial.step_in_bands(1);
            parallel.step_in_bands(4);
        }
        assert_eq!(live_cells(&parallel), live_cells(&serial));
        assert_eq!(parallel.live_cells_count(), serial.live_cells_count());
        assert_eq!(parallel.fingerprint(), serial.fingerprint());
    }

    #[test]
    fn the_r_pentomino_reaches_its_known_population() {
        let mut board = board_with([(1, 0), (2, 0), (0, 1), (1, 1), (1, 2)]);
        for _ in 0..1103 {
            board.step();
        }
        // Gliders leave the pattern long before this, and they count too.
        assert_eq!(board.live_cells_count(), 116);
    }

    #[test]
    fn a_glider_crosses_the_origin_in_a_handful_of_chunks() {
        let start = -500;
        let mut board = board_with(GLIDER.map(|(x, y)| (x + start, y + start)));
        let mut most_chunks = 0;
        for _ in 0..40_000 {
            board.step();
            most_chunks = most_chunks.max(board.chunk_count());
        }
        let moved = 40_000 / 4 + start;
        assert_eq!(
            live_cells(&board),
            sorted(GLIDER.map(|(x, y)| (x + moved, y + moved)))
        );
        assert!(most_chunks <= 4, "held {most_chunks} chunks");
    }

    #[test]
    fn a_cell_is_born_from_neighbours_in_three_other_chunks() {
        let edge = CHUNK_SIZE - 1;
        let mut board = board_with([(edge, edge), (edge + 1, edge), (edge, edge + 1)]);
        board.step();
        assert!(board.is_alive(edge + 1, edge + 1));
    }

    #[test]
    fn changes_reach_every_chunk_around_a_corner() {
        let mut rng = fastrand::Rng::with_seed(11);
        // The corners of chunk (0, 0). A small dense patch keeps flipping cells
        // right on the corner, where only the diagonal chunks can see them.
        let corners = [
            (0, 0),
            (CHUNK_SIZE, 0),
            (0, CHUNK_SIZE),
            (CHUNK_SIZE, CHUNK_SIZE),
        ];
        for (corner_x, corner_y) in corners {
            for _ in 0..200 {
                let start: HashSet<_> = (0..40)
                    .map(|_| (corner_x + rng.i64(-5..5), corner_y + rng.i64(-5..5)))
                    .collect();
                let mut board = board_with(start.iter().copied());
                let mut cells = start;
                for generation in 0..8 {
                    board.step();
                    cells = brute_force_step(&cells, Rule::LIFE);
                    assert_eq!(
                        live_cells(&board),
                        sorted(cells.iter().copied()),
                        "corner ({corner_x}, {corner_y}) generation {generation}",
                    );
                }
            }
        }
    }

    #[test]
    fn an_edit_on_a_chunk_edge_wakes_the_chunk_beyond_it() {
        const UPRIGHT: [(i64, i64); 3] = [(0, -1), (0, 0), (0, 1)];
        const LEVEL: [(i64, i64); 3] = [(-1, 0), (0, 0), (1, 0)];
        let (last, middle) = (CHUNK_SIZE - 1, CHUNK_SIZE / 2);
        // A blinker along each edge of a chunk, with a birth just over that edge.
        // The chunk beyond is empty, so only the edits can schedule it.
        for (shape, (x, y), birth) in [
            (UPRIGHT, (last, middle), (last + 1, middle)),
            (UPRIGHT, (0, middle), (-1, middle)),
            (LEVEL, (middle, last), (middle, last + 1)),
            (LEVEL, (middle, 0), (middle, -1)),
        ] {
            let mut board = board_with(shape.map(|(dx, dy)| (x + dx, y + dy)));
            board.step();
            assert!(board.is_alive(birth.0, birth.1), "({x}, {y})");
        }
    }

    #[test]
    fn editing_a_settled_pattern_wakes_it() {
        // A block astride a corner, so the edit has to wake other chunks too.
        let edge = CHUNK_SIZE - 1;
        let block = [
            (edge, edge),
            (edge + 1, edge),
            (edge, edge + 1),
            (edge + 1, edge + 1),
        ];
        let mut board = board_with(block);
        board.step();
        assert!(board.active.is_empty());

        // Three cells of a block grow the fourth back.
        for (x, y) in block {
            assert!(board.depopulate(x, y));
            assert!(board.step(), "removing ({x}, {y}) unsettles the block");
            assert_eq!(live_cells(&board), sorted(block));
            assert!(!board.step());
        }

        // A cell added to a corner of the block upsets it the same way.
        assert!(board.populate(edge + 2, edge + 2));
        assert!(board.step());
    }

    #[test]
    fn negative_coordinates_round_down_to_their_chunk() {
        let mut board = SparseBoard::default();
        assert!(board.populate(-1, -CHUNK_SIZE - 1));
        assert!(board.is_alive(-1, -CHUNK_SIZE - 1));
        assert!(!board.is_alive(1, CHUNK_SIZE + 1));
        assert_eq!(
            board.live_bounds(),
            Some(((-1, -CHUNK_SIZE - 1), (-1, -CHUNK_SIZE - 1)))
        );
    }

    #[test]
    fn edits_report_whether_anything_changed() {
        let mut board = SparseBoard::default();
        assert!(board.populate(3, 4));
        assert!(!board.populate(3, 4));
        assert_eq!(board.live_cells_count(), 1);
        assert!(!board.depopulate(5, 5), "the chunk has no such cell");
        assert!(!board.depopulate(1000, 1000), "there is no such chunk");
        assert!(board.depopulate(3, 4));
        assert_eq!(board.live_cells_count(), 0);
        assert_eq!(board.chunk_count(), 0);
    }

    #[test]
    fn a_dead_pattern_frees_its_chunks_and_stops_working() {
        let mut board = board_with([(CHUNK_SIZE - 1, 0), (CHUNK_SIZE, 0)]);
        assert_eq!(board.chunk_count(), 2);
        assert!(board.step());
        assert_eq!(board.chunk_count(), 0);
        assert_eq!(board.live_bounds(), None);
        assert!(!board.step());
        assert!(board.active.is_empty());
    }

    #[test]
    fn a_still_life_stops_being_scheduled() {
        let mut board = board_with([(0, 0), (1, 0), (0, 1), (1, 1)]);
        assert!(!board.step(), "a block never changes");
        assert!(board.active.is_empty());
    }

    #[test]
    fn work_stays_on_the_rows_beside_a_change() {
        let middle = CHUNK_SIZE / 2;
        let mut board = board_with((-1..=1).map(|dx| (middle + dx, middle)));
        board.step();
        // A blinker's flips, the rows around them, and no neighbouring chunks.
        assert_eq!(board.active.len(), 1);
        assert_eq!(board.active[&(0, 0)].count_ones(), 5);
    }

    #[test]
    fn the_live_bounds_span_chunks() {
        let board = board_with([(-70, 9), (200, -300), (3, 4)]);
        assert_eq!(board.live_bounds(), Some(((-70, -300), (200, 9))));
    }

    #[test]
    fn clearing_empties_the_board_and_asks_for_a_redraw() {
        let mut board = board_with(GLIDER);
        let _ = board.take_dirty_chunks();
        assert!(board.clear());
        assert!(!board.clear());
        assert_eq!(board.live_cells_count(), 0);
        assert_eq!(board.take_dirty_chunks(), DirtyChunks::Full);
        assert!(!board.step());
    }

    #[test]
    fn b0_rules_are_refused_and_change_nothing() {
        let mut board = SparseBoard::default();
        let life = board.rule();
        assert_eq!(
            board.set_rule(Rule::parse("B03/S23").unwrap()),
            Err(UnboundedRuleError)
        );
        assert_eq!(board.rule(), life);
    }

    #[test]
    fn a_new_rule_wakes_a_settled_pattern() {
        let block = [(0, 0), (1, 0), (0, 1), (1, 1)];
        let mut board = board_with(block);
        board.step();
        assert_eq!(board.set_rule(Rule::LIFE), Ok(false));
        assert_eq!(board.set_rule(Rule::parse("B3/S").unwrap()), Ok(true));
        assert!(board.step());
        assert_eq!(board.live_cells_count(), 0);
    }

    #[test]
    fn the_fingerprint_repeats_with_the_cells() {
        let edge = CHUNK_SIZE - 1;
        let mut board = board_with((0..3).map(|dx| (edge + dx, -1)));
        board.step();
        let first = board.fingerprint();
        board.step();
        assert_ne!(board.fingerprint(), first);
        board.step();
        assert_eq!(board.fingerprint(), first);
    }

    #[test]
    fn a_snapshot_is_untouched_by_later_generations_and_restores_exactly() {
        let mut board = board_with((0..3).map(|dx| (dx, 0)));
        let before = live_cells(&board);
        let snapshot = board.snapshot();
        board.step();
        board.populate(100, 100);
        assert_ne!(live_cells(&board), before);

        board.restore(snapshot);
        assert_eq!(live_cells(&board), before);
        assert_eq!(board.live_cells_count(), 3);
        assert_eq!(board.take_dirty_chunks(), DirtyChunks::Full);
        assert!(board.step(), "a restored blinker keeps blinking");
        assert_eq!(live_cells(&board), [(1, -1), (1, 0), (1, 1)]);
    }

    #[test]
    fn a_new_board_asks_for_one_full_redraw() {
        let mut board = SparseBoard::default();
        assert_eq!(board.take_dirty_chunks(), DirtyChunks::Full);
        assert_eq!(board.take_dirty_chunks(), DirtyChunks::None);
    }

    #[test]
    fn dirty_chunks_mask_every_cell_that_flipped_since_the_last_sync() {
        let mut board = board_with((-1..=1).map(|dx| (dx, 5)));
        let _ = board.take_dirty_chunks();
        board.step();
        let DirtyChunks::Chunks(chunks) = board.take_dirty_chunks() else {
            panic!("a blinker flips four cells");
        };
        // The west end died in the chunk it had to itself, which is empty now.
        assert_eq!(board.chunk_count(), 1);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[&(-1, 0)][5], 1 << 63);
        assert_eq!(chunks[&(0, 0)][4..7], [0b01, 0b10, 0b01]);
    }

    #[test]
    fn dirty_chunks_fall_back_to_a_full_redraw_when_unread() {
        let mut board = SparseBoard::default();
        let _ = board.take_dirty_chunks();
        for chunk in 0..=DIRTY_CHUNKS_MAX as i64 {
            board.populate(chunk * CHUNK_SIZE, 0);
        }
        assert_eq!(board.take_dirty_chunks(), DirtyChunks::Full);
        assert_eq!(board.take_dirty_chunks(), DirtyChunks::None);
    }
}
