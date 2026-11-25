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

/// Fewest scheduled rows that justify another parallel band, on either board. A
/// row of 30% noise takes about 50 ns, fingerprint and all, so a band of these
/// is some 50 µs of work, well beyond what it costs to wake a thread of the task
/// pool and wait for it. It is also sixteen whole chunks, so a board busy
/// everywhere splits as it would if bands were counted in chunks, while settled
/// debris, whose chunks have a row or two scheduled each, stays on one thread
/// rather than spreading a few microseconds of work across all of them.
pub(crate) const MIN_BAND_ROWS: usize = 1024;

/// Changed chunks tracked between render syncs before settling for a full redraw.
/// Nothing else bounds the list, as a run may cross new chunks forever between syncs.
const DIRTY_CHUNKS_MAX: usize = 1 << 14;

/// Staged changes a band's list may keep between generations without having to
/// account for them. A staged chunk takes about half a kilobyte, so this is about
/// half a megabyte a band: enough that ordinary patterns never give their lists
/// back and grow them again, while a list that once held a busy plane's worth is
/// cut down once the pattern calms, rather than holding its peak for good.
const CHANGE_SCRATCH_KEPT: usize = 1 << 10;

/// Chunks a run may reach: a generation that would leave more doesn't run. A
/// chunk's cells take 512 bytes, so this is half a gigabyte of them; with its
/// bookkeeping a plane this size takes about 0.9 GB, and stepping it, which
/// stages a copy of every chunk it changes, about 1.5 GB. History holds on to
/// the chunks the plane has since replaced on top. See docs/engine.qd.
pub const PLANE_CHUNKS_MAX: usize = 1 << 20;

/// Live cells that can be written out, as RLE or to a workspace file. Far more
/// than fit a workspace unless most of them are in long runs.
pub const PLANE_EXPORT_CELLS_MAX: usize = 1 << 26;

/// Returned by a step that would have taken the plane past its chunk budget,
/// which leaves the plane as it was.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaneFull;

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

/// A rectangle of cells, from its lowest corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRect {
    pub left: i64,
    pub bottom: i64,
    pub width: usize,
    pub height: usize,
}

impl CellRect {
    /// A `width` × `height` rectangle with the origin in its middle, where a board
    /// of that size would land on the plane.
    pub fn around_origin(width: usize, height: usize) -> Self {
        Self {
            left: -((width / 2) as i64),
            bottom: -((height / 2) as i64),
            width,
            height,
        }
    }

    /// The middle of this rectangle, at most `side` cells across each way.
    pub fn middle(self, side: usize) -> Self {
        let keep = |start: i64, length: usize| {
            let kept = length.min(side);
            (start + ((length - kept) / 2) as i64, kept)
        };
        let (left, width) = keep(self.left, self.width);
        let (bottom, height) = keep(self.bottom, self.height);
        Self {
            left,
            bottom,
            width,
            height,
        }
    }
}

/// A grid of square blocks over part of the plane, for drawing it. A block is
/// `1 << level` cells across, and `left` and `bottom` are multiples of that, so
/// blocks line up with chunks however far the grid is from the origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockGrid {
    pub left: i64,
    pub bottom: i64,
    pub level: u32,
    /// Width in blocks.
    pub width: usize,
    /// Height in blocks.
    pub height: usize,
}

impl BlockGrid {
    /// The finest grid of at most `side` blocks each way over the inclusive
    /// bounds. Lining up with its own blocks can cost it a level.
    pub fn covering((min, max): ((i64, i64), (i64, i64)), side: usize) -> Self {
        let span = (max.0 - min.0 + 1).max(max.1 - min.1 + 1) as u64;
        let mut level = span.div_ceil(side as u64).next_power_of_two().ilog2();
        loop {
            let block = 1i64 << level;
            let (left, bottom) = (min.0.div_euclid(block), min.1.div_euclid(block));
            let width = (max.0.div_euclid(block) - left + 1) as usize;
            let height = (max.1.div_euclid(block) - bottom + 1) as usize;
            if width.max(height) <= side {
                return Self {
                    left: left * block,
                    bottom: bottom * block,
                    level,
                    width,
                    height,
                };
            }
            level += 1;
        }
    }

    /// One past the last cell in each direction.
    fn end(&self) -> (i64, i64) {
        let span = |blocks: usize| (blocks as i64).saturating_mul(1 << self.level);
        (
            self.left.saturating_add(span(self.width)),
            self.bottom.saturating_add(span(self.height)),
        )
    }

    /// The chunks at the low and high corners of the grid.
    pub fn chunk_span(&self) -> (ChunkKey, ChunkKey) {
        let (right, top) = self.end();
        (
            (self.left >> CHUNK_BITS, self.bottom >> CHUNK_BITS),
            ((right - 1) >> CHUNK_BITS, (top - 1) >> CHUNK_BITS),
        )
    }

    /// The block holding a cell, if the grid covers it.
    fn block(&self, x: i64, y: i64) -> Option<(usize, usize)> {
        let (right, top) = self.end();
        ((self.left..right).contains(&x) && (self.bottom..top).contains(&y)).then(|| {
            (
                ((x - self.left) >> self.level) as usize,
                ((y - self.bottom) >> self.level) as usize,
            )
        })
    }
}

/// Chunks of an earlier state. They are shared with the board until it changes
/// them, so taking a snapshot costs a pointer per chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SparseSnapshot(HashMap<ChunkKey, Arc<Chunk>>);

impl SparseSnapshot {
    /// Memory the snapshot could be holding on its own, once the board has moved on.
    pub fn bytes(&self) -> usize {
        self.0.len() * size_of::<Chunk>()
    }
}

/// Which rows of a chunk changed: any of their cells, and those in its first and
/// last columns, which is what the chunks either side of it can see.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ChangedRows {
    pub(crate) any: u64,
    pub(crate) west: u64,
    pub(crate) east: u64,
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

    /// Brings to life the `len` cells from `(x, y)` eastwards, as calling
    /// [`populate`](Self::populate) on each would, but a chunk at a time.
    pub fn populate_run(&mut self, x: i64, y: i64, len: usize) {
        let (mut x, mut left) = (x, len);
        while left > 0 {
            let (key, col, row) = locate(x, y);
            let cols = left.min(CHUNK_SIZE as usize - col);
            let cells = u64::MAX >> (64 - cols) << col;
            let born = self.write_row(key, row, cells, cells);
            if born != 0 {
                self.mark_dirty(key, row, born);
            }
            x += cols as i64;
            left -= cols;
        }
    }

    /// How many chunks [`populate_run`](Self::populate_run) would add for the
    /// same run, so a caller can refuse it first.
    pub fn chunks_added_by_run(&self, x: i64, y: i64, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        let row = y >> CHUNK_BITS;
        let (first, last) = (x >> CHUNK_BITS, (x + (len - 1) as i64) >> CHUNK_BITS);
        (first..=last)
            .filter(|&col| !self.chunks.contains_key(&(col, row)))
            .count()
    }

    pub fn clear(&mut self) -> bool {
        if self.chunks.is_empty() {
            return false;
        }
        self.chunks.clear();
        self.active.clear();
        self.live_count = 0;
        // Nothing is left to step, so no step would cut the lists down.
        self.change_scratch = Vec::new();
        self.mark_dirty_full();
        true
    }

    /// Replaces every cell of `rect`, asking `alive` about each in turn, row by
    /// row from the lowest and left to right.
    pub fn replace_rect(&mut self, rect: CellRect, mut alive: impl FnMut() -> bool) {
        let right = rect.left + rect.width as i64;
        for y in rect.bottom..rect.bottom + rect.height as i64 {
            let mut x = rect.left;
            while x < right {
                let (key, col, row) = locate(x, y);
                let cols = (right - x).min(CHUNK_SIZE - col as i64) as usize;
                let mut cells = 0;
                for bit in col..col + cols {
                    cells |= u64::from(alive()) << bit;
                }
                self.write_row(key, row, (u64::MAX >> (64 - cols)) << col, cells);
                x += cols as i64;
            }
        }
        self.mark_dirty_full();
    }

    /// Sets the cells of a row that `mask` selects, returning those that flipped.
    fn write_row(&mut self, key: ChunkKey, row: usize, mask: u64, cells: u64) -> u64 {
        let old = self.chunks.get(&key).map_or(0, |chunk| chunk[row]);
        let flipped = (old ^ cells) & mask;
        if flipped == 0 {
            return 0;
        }
        let new = old ^ flipped;
        let chunk = self
            .chunks
            .entry(key)
            .or_insert_with(|| Arc::new(EMPTY_CHUNK));
        Arc::make_mut(chunk)[row] = new;
        if new == 0 && **chunk == EMPTY_CHUNK {
            self.chunks.remove(&key);
        }
        self.live_count =
            (self.live_count + new.count_ones() as usize).saturating_sub(old.count_ones() as usize);
        let rows = ChangedRows {
            any: 1 << row,
            west: (flipped & 1) << row,
            east: (flipped >> LAST_LINE) << row,
        };
        wake(&mut self.active, key, rows);
        flipped
    }

    pub fn step(&mut self) -> bool {
        self.step_within(usize::MAX)
            .expect("no plane holds usize::MAX chunks")
    }

    /// Steps unless the generation would leave more than `max_chunks` chunks,
    /// in which case nothing changes. Returns whether any cell changed.
    pub fn step_within(&mut self, max_chunks: usize) -> Result<bool, PlaneFull> {
        let threads = ComputeTaskPool::get_or_init(TaskPool::default).thread_num();
        self.step_in_bands(|rows| threads.min(rows / MIN_BAND_ROWS), max_chunks)
    }

    /// Evaluates the scheduled rows in as many bands of whole chunks as `bands`
    /// asks for, given the number of rows, each with about as many rows. Bands
    /// run in parallel on the compute task pool when there is more than one, and
    /// only read the chunks and stage their changes, which are applied once all
    /// have finished, so no band can see another's half of the generation, and
    /// a generation that would outgrow `max_chunks` can be dropped whole.
    fn step_in_bands(
        &mut self,
        bands: impl FnOnce(usize) -> usize,
        max_chunks: usize,
    ) -> Result<bool, PlaneFull> {
        if self.active.is_empty() {
            return Ok(false);
        }

        let mut work = std::mem::take(&mut self.work_scratch);
        work.clear();
        work.extend(self.active.drain());
        let rows = work
            .iter()
            .map(|&(_, rows)| rows.count_ones() as usize)
            .sum();
        let bands = bands(rows).max(1);
        let mut changes = std::mem::take(&mut self.change_scratch);
        if changes.len() < bands {
            changes.resize_with(bands, Vec::new);
        }

        let (chunks, rule) = (&self.chunks, self.rule);
        let (live_delta, flips) = if bands == 1 {
            step_band(chunks, rule, &work, &mut changes[0])
        } else {
            ComputeTaskPool::get_or_init(TaskPool::default)
                .scope(|scope| {
                    for (band, changes) in split_bands(&work, rows, bands).zip(&mut changes) {
                        scope.spawn(async move { step_band(chunks, rule, band, changes) });
                    }
                })
                .into_iter()
                .fold((0, 0), |(live, flips), (band_live, band_flips)| {
                    (live + band_live, flips ^ band_flips)
                })
        };

        if self.outgrows(&changes, max_chunks) {
            for list in &mut changes {
                list.clear();
            }
            // Scheduled again, so the same generation runs once there is room.
            self.active.extend(work.iter().copied());
            self.work_scratch = work;
            self.change_scratch = changes;
            return Err(PlaneFull);
        }

        let mut changed = false;
        for list in &mut changes {
            changed |= !list.is_empty();
            for change in list.iter() {
                self.apply(change);
            }
            let used = list.len();
            list.clear();
            if list.capacity() > (used * 4).max(CHANGE_SCRATCH_KEPT) {
                list.shrink_to(used * 2);
            }
        }
        self.work_scratch = work;
        self.change_scratch = changes;
        self.live_count = self.live_count.saturating_add_signed(live_delta);
        self.fingerprint ^= flips;
        Ok(changed)
    }

    /// Whether applying `changes` would leave more than `max_chunks` chunks.
    fn outgrows(&self, changes: &[Vec<ChunkChange>], max_chunks: usize) -> bool {
        let staged: usize = changes.iter().map(Vec::len).sum();
        // Each change adds a chunk at most, so most generations need no count.
        if self.chunks.len().saturating_add(staged) <= max_chunks {
            return false;
        }
        let after = changes
            .iter()
            .flatten()
            .fold(self.chunks.len(), |count, change| {
                let held = self.chunks.contains_key(&change.key);
                let kept = change.next != EMPTY_CHUNK;
                count + usize::from(kept && !held) - usize::from(held && !kept)
            });
        after > max_chunks
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

    /// A chunk's cells, unless it has no live ones.
    pub fn chunk(&self, key: ChunkKey) -> Option<&Chunk> {
        self.chunks.get(&key).map(|chunk| &**chunk)
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

    /// Calls `visit` with the chunks the grid touches that have live cells, in no
    /// particular order. The cost follows whichever is fewer, the chunks in
    /// memory or those under the grid, so a view of empty space is free and so is
    /// one that dwarfs the pattern.
    pub fn for_each_chunk_in<'a>(
        &'a self,
        grid: BlockGrid,
        mut visit: impl FnMut(ChunkKey, &'a Chunk),
    ) {
        let ((first_x, first_y), (last_x, last_y)) = grid.chunk_span();
        let span = |first: i64, last: i64| (last - first + 1) as u128;
        if span(first_x, last_x) * span(first_y, last_y) > self.chunks.len() as u128 {
            for (&key, chunk) in &self.chunks {
                if (first_x..=last_x).contains(&key.0) && (first_y..=last_y).contains(&key.1) {
                    visit(key, chunk);
                }
            }
            return;
        }
        for y in first_y..=last_y {
            for x in first_x..=last_x {
                if let Some(chunk) = self.chunks.get(&(x, y)) {
                    visit((x, y), chunk);
                }
            }
        }
    }

    /// Calls `visit` with the column and row of each block holding a live cell,
    /// perhaps more than once. Blocks are merged a row of the chunk at a time, so
    /// the work per chunk falls with the size of the blocks.
    pub fn for_each_live_block(&self, grid: BlockGrid, mut visit: impl FnMut(usize, usize)) {
        let block = 1usize << grid.level.min(CHUNK_BITS);
        self.for_each_chunk_in(grid, |key, chunk| {
            let (left, bottom) = chunk_origin(key);
            if grid.level >= CHUNK_BITS {
                // A live chunk is all its block needs to know.
                if let Some((col, row)) = grid.block(left, bottom) {
                    visit(col, row);
                }
                return;
            }
            for (group, rows) in chunk.chunks_exact(block).enumerate() {
                let mut cells = rows.iter().fold(0, |merged, &cells| merged | cells);
                let y = bottom + (group * block) as i64;
                while cells != 0 {
                    let col = cells.trailing_zeros() as usize & !(block - 1);
                    if let Some(cell) = grid.block(left + col as i64, y) {
                        visit(cell.0, cell.1);
                    }
                    // Past the rest of this block, which is lit already.
                    cells &= !(u64::MAX >> (64 - block) << col);
                }
            }
        });
    }

    /// Calls `visit` with each run of live cells as `(x, y, length)`, row by row
    /// from the lowest and left to right, the order RLE wants them in. A run
    /// across chunks comes in pieces.
    pub fn for_each_live_run(&self, visit: impl FnMut(i64, i64, usize)) {
        let chunks = self.chunks.iter().map(|(&key, chunk)| (key, &**chunk));
        visit_runs(chunks.collect(), None, visit);
    }

    /// The same for the runs inside `rect`, cut off at its sides. Like
    /// [`for_each_chunk_in`](Self::for_each_chunk_in), it reads only the chunks
    /// the rectangle touches.
    pub fn for_each_live_run_in(&self, rect: CellRect, visit: impl FnMut(i64, i64, usize)) {
        let mut chunks = Vec::new();
        self.for_each_chunk_in(rect_grid(rect), |key, chunk| chunks.push((key, chunk)));
        visit_runs(chunks, Some(rect), visit);
    }

    /// Kills every live cell in `rect` a chunk's row at a time, and returns how
    /// many there were.
    pub fn clear_rect(&mut self, rect: CellRect) -> usize {
        let mut keys = Vec::new();
        self.for_each_chunk_in(rect_grid(rect), |key, _| keys.push(key));
        let mut killed = 0;
        for key in keys {
            let (left, bottom) = chunk_origin(key);
            for row in 0..CHUNK_SIZE as usize {
                let mask = rect_mask(rect, left, bottom + row as i64);
                if mask == 0 {
                    continue;
                }
                let flipped = self.write_row(key, row, mask, 0);
                if flipped != 0 {
                    killed += flipped.count_ones() as usize;
                    self.mark_dirty(key, row, flipped);
                }
            }
        }
        killed
    }

    /// Chunks in memory, each of [`CHUNK_SIZE`] squared bits plus some overhead.
    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Inclusive bounds, computed on demand rather than during simulation steps.
    /// Every chunk in memory has a live cell, so only the chunks on the edges of
    /// the box of chunks need reading.
    pub fn live_bounds(&self) -> Option<((i64, i64), (i64, i64))> {
        let mut keys = self.chunks.keys();
        let &first = keys.next()?;
        let (low, high) = keys.fold((first, first), |(low, high), &(x, y)| {
            ((low.0.min(x), low.1.min(y)), (high.0.max(x), high.1.max(y)))
        });
        // The live columns of the westmost and eastmost chunks, and the rows with
        // a live cell of the lowest and highest.
        let (mut west, mut east, mut south, mut north) = (0, 0, 0, 0);
        for (&(x, y), chunk) in &self.chunks {
            if x == low.0 || x == high.0 {
                let columns = chunk.iter().fold(0, |columns, &cells| columns | cells);
                west |= if x == low.0 { columns } else { 0 };
                east |= if x == high.0 { columns } else { 0 };
            }
            if y == low.1 || y == high.1 {
                let rows = chunk.iter().enumerate().fold(0u64, |rows, (row, &cells)| {
                    rows | u64::from(cells != 0) << row
                });
                south |= if y == low.1 { rows } else { 0 };
                north |= if y == high.1 { rows } else { 0 };
            }
        }
        let ((left, bottom), (right, top)) = (chunk_origin(low), chunk_origin(high));
        let last = |cells: u64| i64::from(LAST_LINE - cells.leading_zeros());
        Some((
            (
                left + i64::from(west.trailing_zeros()),
                bottom + i64::from(south.trailing_zeros()),
            ),
            (right + last(east), top + last(north)),
        ))
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

/// Walks the runs of `chunks` as [`SparseBoard::for_each_live_run`] does, only
/// inside `clip` if there is one.
fn visit_runs(
    mut chunks: Vec<(ChunkKey, &Chunk)>,
    clip: Option<CellRect>,
    mut visit: impl FnMut(i64, i64, usize),
) {
    chunks.sort_unstable_by_key(|(key, _)| (key.1, key.0));
    for band in chunks.chunk_by(|a, b| a.0.1 == b.0.1) {
        for row in 0..CHUNK_SIZE as usize {
            for &(key, chunk) in band {
                let (left, bottom) = chunk_origin(key);
                let y = bottom + row as i64;
                let mut cells = chunk[row] & clip.map_or(u64::MAX, |rect| rect_mask(rect, left, y));
                while cells != 0 {
                    let start = cells.trailing_zeros();
                    let length = (cells >> start).trailing_ones();
                    visit(left + i64::from(start), y, length as usize);
                    cells &= u64::MAX.checked_shl(start + length).unwrap_or(0);
                }
            }
        }
    }
}

/// The cells of `rect` as a grid of single-cell blocks, to find its chunks with.
fn rect_grid(rect: CellRect) -> BlockGrid {
    BlockGrid {
        left: rect.left,
        bottom: rect.bottom,
        level: 0,
        width: rect.width,
        height: rect.height,
    }
}

/// Which cells of the chunk row starting at `(left, y)` lie in `rect`.
fn rect_mask(rect: CellRect, left: i64, y: i64) -> u64 {
    let y = i128::from(y);
    let bottom = i128::from(rect.bottom);
    if y < bottom || y >= bottom + rect.height as i128 {
        return 0;
    }
    let from = i128::from(rect.left) - i128::from(left);
    let start = from.clamp(0, CHUNK_SIZE.into());
    let end = (from + rect.width as i128).clamp(0, CHUNK_SIZE.into());
    if start >= end {
        return 0;
    }
    u64::MAX >> (CHUNK_SIZE as i128 - (end - start)) << start
}

/// Positions of the set bits, lowest first.
pub(crate) fn bits(mut mask: u64) -> impl Iterator<Item = usize> {
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

/// Splits scheduled chunks into at most `bands` runs of whole chunks, each with
/// about as many of the `rows` scheduled rows as the others, so that bands take
/// about as long however unevenly the rows fall among the chunks.
pub(crate) fn split_bands<K>(
    work: &[(K, u64)],
    rows: usize,
    bands: usize,
) -> impl Iterator<Item = &[(K, u64)]> {
    // Every band but the last has at least this many rows, so there are no more
    // than `bands` of them.
    let band_rows = rows.div_ceil(bands.max(1)).max(1);
    let mut rest = work;
    std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let (mut end, mut taken) = (0, 0);
        while end < rest.len() && taken < band_rows {
            taken += rest[end].1.count_ones() as usize;
            end += 1;
        }
        let (band, tail) = rest.split_at(end);
        rest = tail;
        Some(band)
    })
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
    let rule = Transitions::new(rule);
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
        next_rows(scheduled, &rule, |line| row_sums(&around, line), &mut next);
        let mut rows = ChangedRows::default();
        for row in bits(scheduled) {
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

/// The sums of row `line - 1` of the middle column of `around`, as
/// [`next_rows`] numbers them.
fn row_sums(around: &Around, line: usize) -> RowSums {
    let (first, row) = match line {
        0 => (0, LAST_LINE as usize),
        ROW_ABOVE => (6, 0),
        _ => (3, line - 1),
    };
    let centre = around[first + 1][row];
    RowSums::new(
        centre << 1 | around[first][row] >> LAST_LINE,
        centre,
        centre >> 1 | around[first + 2][row] << LAST_LINE,
    )
}

/// The line [`next_rows`] asks for the row above a chunk's last row by.
const ROW_ABOVE: usize = CHUNK_SIZE as usize + 1;

/// A row of 64 cells, with each cell's west and east neighbours added up
/// alongside it, bit-sliced so that bit `i` of each word belongs to cell `i`.
/// A row's sums serve three rows: its own, and those either side of it.
#[derive(Clone, Copy)]
pub(crate) struct RowSums {
    centre: u64,
    /// The ones and twos of each cell and its two neighbours, 0 to 3, which is
    /// what the rows either side see of this one.
    ones: u64,
    twos: u64,
    /// The ones and twos of just the two neighbours, 0 to 2, which is what a
    /// cell sees of its own row.
    side_ones: u64,
    side_twos: u64,
}

impl RowSums {
    /// From a row's cells and the same row moved one cell east, `west`, which
    /// lines each cell up with its west neighbour, and one cell west, `east`.
    pub(crate) fn new(west: u64, centre: u64, east: u64) -> Self {
        let (side_ones, side_twos) = (west ^ east, west & east);
        Self {
            centre,
            ones: side_ones ^ centre,
            twos: side_twos | side_ones & centre,
            side_ones,
            side_twos,
        }
    }
}

/// A rule as [`next_cells`] applies it to 64 neighbour counts at once, worked
/// out once a generation rather than once a row.
#[derive(Clone, Copy)]
pub(crate) struct Transitions {
    /// Life, which most boards run, reduces to a handful of operations.
    life: bool,
    /// For each count, all ones if a dead cell with that many neighbours is
    /// born, and all ones if a live cell's fate differs from a dead one's.
    born: [u64; 9],
    differs: [u64; 9],
}

impl Transitions {
    pub(crate) fn new(rule: Rule) -> Self {
        let mask = rule.transition_mask();
        let word = |bit: usize| 0u64.wrapping_sub(u64::from(mask >> bit & 1));
        Self {
            life: rule == Rule::LIFE,
            born: std::array::from_fn(word),
            differs: std::array::from_fn(|count| word(count) ^ word(count + 9)),
        }
    }

    /// The next state of each cell of `alive`, from its count of live
    /// neighbours as four bit planes, lowest first.
    fn apply(&self, alive: u64, [n0, n1, n2, n3]: [u64; 4]) -> u64 {
        if self.life {
            // Three neighbours, or two and alive. Eight leaves `n1` clear.
            return n1 & !n2 & (n0 | alive);
        }
        // Each cell's outcome for every count, then halving the candidates by
        // one bit of the count at a time: `pick` takes `high` where `bit` is set.
        let pick = |low: u64, high: u64, bit: u64| low ^ ((low ^ high) & bit);
        let outcome = |count: usize| self.born[count] ^ (self.differs[count] & alive);
        let pairs = [0, 2, 4, 6].map(|count| pick(outcome(count), outcome(count + 1), n0));
        let quads = [pick(pairs[0], pairs[1], n1), pick(pairs[2], pairs[3], n1)];
        // A count of eight has the three low bits clear.
        pick(pick(quads[0], quads[1], n2), outcome(8), n3)
    }
}

/// Works out the next state of the `scheduled` rows of a chunk into `next`.
/// `sums(line)` gives the sums of row `line - 1` of the chunk's column, so line
/// 0 is the last row of the chunk below and [`ROW_ABOVE`] the first row of the
/// chunk above. Rows go lowest first, keeping the last three rows' sums, so a
/// run of scheduled rows works out each row's sums once rather than three times.
pub(crate) fn next_rows(
    scheduled: u64,
    rule: &Transitions,
    mut sums: impl FnMut(usize) -> RowSums,
    next: &mut Chunk,
) {
    // The last row worked out, with the sums of the rows below, level with, and
    // above it.
    let mut window: Option<(usize, [RowSums; 3])> = None;
    for row in bits(scheduled) {
        let rows = match window {
            Some((last, [_, level, above])) if last + 1 == row => [level, above, sums(row + 2)],
            Some((last, [.., above])) if last + 2 == row => [above, sums(row + 1), sums(row + 2)],
            _ => [sums(row), sums(row + 1), sums(row + 2)],
        };
        next[row] = next_cells(&rows, rule);
        window = Some((row, rows));
    }
}

/// The next state of the 64 cells of the middle row, from the sums of the rows
/// below, level with, and above it.
fn next_cells([below, level, above]: &[RowSums; 3], rule: &Transitions) -> u64 {
    // Each cell's live neighbours, 0 to 8, as four bit planes, added by
    // carry-save: the three cells of each row either side, and the two of its own.
    let ones = below.ones ^ above.ones;
    let n0 = ones ^ level.side_ones;
    let carry = below.ones & above.ones | ones & level.side_ones;
    let twos = below.twos ^ above.twos;
    let more_twos = twos ^ level.side_twos;
    let fours = below.twos & above.twos | twos & level.side_twos;
    let n1 = more_twos ^ carry;
    let more_fours = more_twos & carry;
    rule.apply(
        level.centre,
        [n0, n1, fours ^ more_fours, fours & more_fours],
    )
}

#[cfg(test)]
mod tests {
    use super::{
        BlockGrid, CHANGE_SCRATCH_KEPT, CHUNK_SIZE, CellRect, Chunk, DIRTY_CHUNKS_MAX, DirtyChunks,
        SparseBoard, Transitions, UnboundedRuleError, next_rows, row_sums, split_bands,
    };
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
        for rule in ["B3/S23", "B36/S23", "B3678/S34678", "B2/S", "B35678/S5678"] {
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
            serial.step_in_bands(|_| 1, usize::MAX).unwrap();
            parallel.step_in_bands(|_| 4, usize::MAX).unwrap();
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

    fn live_blocks(board: &SparseBoard, grid: BlockGrid) -> HashSet<(usize, usize)> {
        let mut blocks = HashSet::new();
        board.for_each_live_block(grid, |col, row| {
            assert!(col < grid.width && row < grid.height, "({col}, {row})");
            blocks.insert((col, row));
        });
        blocks
    }

    #[test]
    fn live_blocks_match_the_cells_at_every_level() {
        let cells = soup(1500, 300);
        let board = board_with(cells.iter().copied());
        for level in 0..9 {
            let block = 1i64 << level;
            let grid = BlockGrid {
                // Off to one side, so that some cells fall outside the grid.
                left: -128 / block * block,
                bottom: -256 / block * block,
                level,
                width: (300 / block) as usize + 1,
                height: (400 / block) as usize + 1,
            };
            let expected: HashSet<_> = cells
                .iter()
                .filter_map(|&(x, y)| {
                    let col = (x - grid.left).div_euclid(block);
                    let row = (y - grid.bottom).div_euclid(block);
                    let inside = (0..grid.width as i64).contains(&col)
                        && (0..grid.height as i64).contains(&row);
                    inside.then_some((col as usize, row as usize))
                })
                .collect();
            assert_eq!(live_blocks(&board, grid), expected, "level {level}");
        }
    }

    #[test]
    fn a_grid_finds_chunks_either_way_round() {
        let board = board_with([(0, 0), (1000, 1000)]);
        let visited = |grid| {
            let mut keys = Vec::new();
            board.for_each_chunk_in(grid, |key, _| keys.push(key));
            keys
        };
        // Fewer chunks in view than in memory, then far more in view.
        let small = BlockGrid {
            left: 0,
            bottom: 0,
            level: 0,
            width: 8,
            height: 8,
        };
        let huge = BlockGrid {
            left: -(1 << 40),
            bottom: -(1 << 40),
            level: 20,
            width: 1 << 21,
            height: 1 << 21,
        };
        assert_eq!(visited(small), [(0, 0)]);
        assert_eq!(visited(huge).len(), 2);
    }

    #[test]
    fn a_grid_wider_than_memory_still_leaves_out_what_it_misses() {
        // Every chunk shares a column or a row with the grid, but only one has both.
        let board = board_with([(0, 0), (0, 1000), (1000, 0), (-1000, 0), (0, -1000)]);
        let grid = BlockGrid {
            left: -256,
            bottom: -256,
            level: 0,
            width: 512,
            height: 512,
        };
        let mut keys = Vec::new();
        board.for_each_chunk_in(grid, |key, _| keys.push(key));
        assert_eq!(keys, [(0, 0)]);
    }

    #[test]
    fn a_block_past_a_chunk_covers_all_of_a_far_away_pattern() {
        let far = 1 << 40;
        let board = board_with([(far, -far)]);
        let grid = BlockGrid {
            left: far - (1 << 9),
            bottom: -far - (1 << 9),
            level: 9,
            width: 2,
            height: 2,
        };
        assert_eq!(live_blocks(&board, grid), HashSet::from([(1, 1)]));
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

    #[test]
    fn replacing_a_rectangle_matches_setting_each_cell_by_hand() {
        let rect = CellRect {
            left: -70,
            bottom: -3,
            width: 150,
            height: 70,
        };
        let draws = |seed| {
            let mut state: u32 = seed;
            move || {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                state >> 16 & 3 == 0
            }
        };
        let mut plane = SparseBoard::default();
        // Cells inside to lose, and outside to keep, on both sides of a chunk edge.
        for cell in [(-70, -3), (79, 66), (-71, 0), (80, 0), (0, -4), (0, 67)] {
            plane.populate(cell.0, cell.1);
        }
        let mut by_hand = plane.clone();
        let mut alive = draws(1);
        for y in rect.bottom..rect.bottom + rect.height as i64 {
            for x in rect.left..rect.left + rect.width as i64 {
                if alive() {
                    by_hand.populate(x, y);
                } else {
                    by_hand.depopulate(x, y);
                }
            }
        }
        plane.replace_rect(rect, draws(1));
        assert_eq!(sorted(plane.live_cells()), sorted(by_hand.live_cells()));
        assert_eq!(plane.live_cells_count(), by_hand.live_cells_count());
        assert_eq!(plane.chunk_count(), by_hand.chunk_count());
        // Waking the same rows, so that the two go on to the same place.
        for _ in 0..6 {
            plane.step();
            by_hand.step();
            assert_eq!(sorted(plane.live_cells()), sorted(by_hand.live_cells()));
        }
    }

    #[test]
    fn replacing_a_rectangle_wakes_the_chunks_beside_it() {
        // A block that has gone quiet, and a cell across the chunk edge that makes
        // the corner next to the block come alive, which only a woken chunk finds.
        let one_cell = |left, bottom| CellRect {
            left,
            bottom,
            width: 1,
            height: 1,
        };
        for (block, cell, birth) in [
            ([(62, 10), (63, 10), (62, 11), (63, 11)], (64, 12), (63, 12)),
            ([(64, 10), (65, 10), (64, 11), (65, 11)], (63, 12), (64, 12)),
        ] {
            let mut plane = board_with(block);
            while plane.step() {}
            plane.step();
            plane.replace_rect(one_cell(cell.0, cell.1), || true);
            plane.step();
            assert!(plane.is_alive(birth.0, birth.1), "{cell:?}");
        }
    }

    #[test]
    fn replacing_a_rectangle_drops_chunks_it_empties_and_redraws_everything() {
        let mut plane = SparseBoard::default();
        plane.populate(3, 3);
        assert_eq!(plane.take_dirty_chunks(), DirtyChunks::Full);
        plane.replace_rect(CellRect::around_origin(256, 256), || false);
        assert_eq!(plane.chunk_count(), 0);
        assert_eq!(plane.live_cells_count(), 0);
        assert_eq!(plane.take_dirty_chunks(), DirtyChunks::Full);
    }

    #[test]
    fn replacing_cells_a_snapshot_holds_leaves_the_snapshot_alone() {
        let mut plane = SparseBoard::default();
        plane.populate(3, 3);
        let snapshot = plane.snapshot();
        plane.replace_rect(CellRect::around_origin(32, 32), || true);
        plane.restore(snapshot);
        assert_eq!(sorted(plane.live_cells()), vec![(3, 3)]);
    }

    fn live_runs(plane: &SparseBoard) -> Vec<(i64, i64, usize)> {
        let mut runs = Vec::new();
        plane.for_each_live_run(|x, y, length| runs.push((x, y, length)));
        runs
    }

    #[test]
    fn live_runs_come_row_by_row_from_the_lowest_and_cover_every_cell() {
        let mut plane = SparseBoard::default();
        let cells = [
            (-1, -65),
            (-65, -1),
            (0, -1),
            (5, 0),
            (6, 0),
            (-100, 0),
            (70, 0),
            (70, 200),
        ];
        for (x, y) in cells {
            plane.populate(x, y);
        }
        assert_eq!(
            live_runs(&plane),
            vec![
                (-1, -65, 1),
                (-65, -1, 1),
                (0, -1, 1),
                (-100, 0, 1),
                (5, 0, 2),
                (70, 0, 1),
                (70, 200, 1),
            ]
        );
    }

    #[test]
    fn a_run_to_the_edge_of_a_chunk_ends_there_and_the_next_begins_beyond_it() {
        let mut plane = SparseBoard::default();
        plane.replace_rect(
            CellRect {
                left: 0,
                bottom: 0,
                width: 70,
                height: 1,
            },
            || true,
        );
        assert_eq!(live_runs(&plane), vec![(0, 0, 64), (64, 0, 6)]);
    }

    #[test]
    fn a_grid_covers_its_bounds_in_blocks_of_its_own_alignment() {
        for (min, max, side) in [
            ((0, 0), (0, 0), 280),
            ((-1, -1), (0, 0), 2),
            ((-300, 5), (900, 20), 280),
            ((1 << 40, -(1 << 41)), ((1 << 40) + 5, 1 << 41), 280),
            ((-1000, -1000), (-999, -999), 1),
            // Two blocks wide at the level its span asks for, until it goes up one.
            ((1, 1), (2, 2), 1),
        ] {
            let grid = BlockGrid::covering((min, max), side);
            let block = 1i64 << grid.level;
            assert!(grid.width.max(grid.height) <= side, "{grid:?}");
            assert_eq!(
                (grid.left.rem_euclid(block), grid.bottom.rem_euclid(block)),
                (0, 0)
            );
            assert!(grid.block(min.0, min.1).is_some(), "{grid:?}");
            assert!(grid.block(max.0, max.1).is_some(), "{grid:?}");
            assert!(grid.block(max.0, min.1).is_some(), "{grid:?}");
        }
    }

    #[test]
    fn a_grid_uses_the_side_it_is_given() {
        let grid = BlockGrid::covering(((0, 0), (279, 99)), 280);
        assert_eq!((grid.level, grid.width, grid.height), (0, 280, 100));
        assert_eq!(BlockGrid::covering(((0, 0), (280, 0)), 280).level, 1);
    }

    #[test]
    fn the_middle_of_a_rectangle_stays_in_the_middle() {
        let rect = CellRect {
            left: -100,
            bottom: 7,
            width: 100,
            height: 20,
        };
        assert_eq!(rect.middle(100), rect);
        let middle = rect.middle(10);
        assert_eq!((middle.left, middle.width), (-55, 10));
        assert_eq!((middle.bottom, middle.height), (12, 10));
    }

    #[test]
    fn a_rectangle_around_the_origin_is_where_a_board_lands() {
        let board = CellRect::around_origin(4, 3);
        assert_eq!((board.left, board.bottom), (-2, -1));
    }

    #[test]
    fn staged_changes_give_back_their_room_once_the_pattern_calms() {
        let scratch =
            |board: &SparseBoard| -> usize { board.change_scratch.iter().map(Vec::capacity).sum() };
        let blinker = |x: i64| (0..3).map(move |dy| (x * CHUNK_SIZE + 10, 10 + dy));
        let busy_chunks = 4 * CHANGE_SCRATCH_KEPT as i64;
        let mut board = board_with((0..busy_chunks).flat_map(blinker));
        let calm = board_with(blinker(0)).snapshot();

        board.step_in_bands(|_| 1, usize::MAX).unwrap();
        assert!(scratch(&board) >= busy_chunks as usize);

        board.restore(calm);
        board.step_in_bands(|_| 1, usize::MAX).unwrap();
        assert!(scratch(&board) < CHANGE_SCRATCH_KEPT);
        assert_eq!(live_cells(&board), sorted((9..12).map(|x| (x, 11))));

        board.clear();
        assert_eq!(scratch(&board), 0);
    }

    /// A rule with each count in `birth` and `survival` with probability one half.
    fn random_rule(rng: &mut fastrand::Rng) -> Rule {
        let mut counts = || {
            (0..=8)
                .filter(|_| rng.bool())
                .map(|count| char::from(b'0' + count))
                .collect::<String>()
        };
        let (birth, survival) = (counts(), counts());
        Rule::parse(&format!("B{birth}/S{survival}")).unwrap()
    }

    #[test]
    fn the_row_kernel_matches_counting_each_cells_neighbours() {
        let mut rng = fastrand::Rng::with_seed(17);
        let named = [
            "B3/S23",
            "B36/S23",
            "B3678/S34678",
            "B2/S",
            "B0/S8",
            "B012345678/S",
        ];
        let mut rules: Vec<_> = named.map(|rule| Rule::parse(rule).unwrap()).into();
        rules.extend((0..60).map(|_| random_rule(&mut rng)));
        for rule in rules {
            let transitions = Transitions::new(rule);
            let mask = rule.transition_mask();
            for _ in 0..8 {
                // Nine chunks of their own density, from empty to full.
                let around: [Chunk; 9] = std::array::from_fn(|_| {
                    let density = rng.u32(0..=8);
                    std::array::from_fn(|_| {
                        (0..64).fold(0, |row, col| {
                            row | u64::from(rng.u32(0..8) < density) << col
                        })
                    })
                });
                let alive = |x: i64, y: i64| {
                    let chunk = &around[(y.div_euclid(64) * 3 + x.div_euclid(64) + 4) as usize];
                    chunk[y.rem_euclid(64) as usize] >> x.rem_euclid(64) & 1 != 0
                };
                // Runs and gaps of every length, and every row.
                let scheduled = match rng.u32(0..3) {
                    0 => !0,
                    1 => rng.u64(..),
                    _ => rng.u64(..) & rng.u64(..) & rng.u64(..),
                };
                let refs = around.each_ref();
                let mut next = around[4];
                next_rows(
                    scheduled,
                    &transitions,
                    |line| row_sums(&refs, line),
                    &mut next,
                );

                for (row, &cells) in next.iter().enumerate() {
                    for col in 0..64 {
                        let (x, y) = (col as i64, row as i64);
                        let expected = if scheduled >> row & 1 == 0 {
                            alive(x, y)
                        } else {
                            let count = (-1..=1)
                                .flat_map(|dy| (-1..=1).map(move |dx| (dx, dy)))
                                .filter(|&(dx, dy)| (dx, dy) != (0, 0) && alive(x + dx, y + dy))
                                .count() as u32;
                            mask >> (count + 9 * u32::from(alive(x, y))) & 1 != 0
                        };
                        assert_eq!(cells >> col & 1 != 0, expected, "{rule} ({col}, {row})");
                    }
                }
            }
        }
    }

    #[test]
    fn bands_split_the_rows_evenly_and_keep_every_chunk() {
        let mut rng = fastrand::Rng::with_seed(5);
        for _ in 0..200 {
            let work: Vec<_> = (0..rng.usize(1..60))
                .map(|key| (key, rng.u64(1..) & rng.u64(..) | 1 << rng.u32(0..64)))
                .collect();
            let rows: usize = work
                .iter()
                .map(|&(_, rows)| rows.count_ones() as usize)
                .sum();
            for bands in 1..=9 {
                let split: Vec<_> = split_bands(&work, rows, bands).collect();
                assert!(split.len() <= bands, "{} bands of {bands}", split.len());
                assert_eq!(split.concat(), work);
                let band_rows = rows.div_ceil(bands);
                for band in &split {
                    let taken: usize = band
                        .iter()
                        .map(|&(_, rows)| rows.count_ones() as usize)
                        .sum();
                    assert!(taken < band_rows + 64, "{taken} rows for {band_rows}");
                }
            }
        }
    }

    /// The bounds of every live cell, one at a time.
    fn bounds_cell_by_cell(plane: &SparseBoard) -> Option<((i64, i64), (i64, i64))> {
        plane.live_cells().fold(None, |bounds, (x, y)| {
            Some(
                bounds.map_or(((x, y), (x, y)), |(low, high): ((i64, i64), (i64, i64))| {
                    ((low.0.min(x), low.1.min(y)), (high.0.max(x), high.1.max(y)))
                }),
            )
        })
    }

    #[test]
    fn live_bounds_match_every_cell_at_every_scale() {
        let mut rng = fastrand::Rng::with_seed(23);
        for trial in 0..300 {
            let spread = [3, 64, 200, 5000, 1 << 40][trial % 5];
            let centre = (rng.i64(-1000..1000), rng.i64(-1000..1000));
            let mut plane = SparseBoard::default();
            for _ in 0..rng.usize(1..80) {
                let x = centre.0 + rng.i64(-spread..=spread);
                let y = centre.1 + rng.i64(-spread..=spread);
                plane.populate(x, y);
            }
            // Emptying some chunks, which leaves others as the extremes.
            let cells: Vec<_> = plane.live_cells().collect();
            for (x, y) in cells {
                if rng.u32(0..4) == 0 {
                    plane.depopulate(x, y);
                }
            }
            assert_eq!(
                plane.live_bounds(),
                bounds_cell_by_cell(&plane),
                "trial {trial}"
            );
        }
        assert_eq!(SparseBoard::default().live_bounds(), None);
    }

    #[test]
    fn runs_in_and_clearing_a_rect_touch_exactly_its_cells() {
        let mut rng = fastrand::Rng::with_seed(31);
        for trial in 0..200 {
            let mut plane = SparseBoard::default();
            for _ in 0..rng.usize(0..400) {
                plane.populate(rng.i64(-150..150), rng.i64(-150..150));
            }
            let rect = CellRect {
                left: rng.i64(-200..200),
                bottom: rng.i64(-200..200),
                width: rng.usize(1..200),
                height: rng.usize(1..200),
            };
            let inside = |&(x, y): &(i64, i64)| {
                (rect.left..rect.left + rect.width as i64).contains(&x)
                    && (rect.bottom..rect.bottom + rect.height as i64).contains(&y)
            };
            let mut expected: Vec<_> = plane.live_cells().filter(inside).collect();
            expected.sort_unstable_by_key(|&(x, y)| (y, x));

            let mut runs = Vec::new();
            plane.for_each_live_run_in(rect, |x, y, length| {
                runs.extend((0..length as i64).map(|offset| (x + offset, y)));
            });
            assert_eq!(runs, expected, "trial {trial}");

            let outside: HashSet<_> = plane.live_cells().filter(|cell| !inside(cell)).collect();
            assert_eq!(plane.clear_rect(rect), expected.len(), "trial {trial}");
            assert_eq!(plane.live_cells().collect::<HashSet<_>>(), outside);
            assert_eq!(plane.live_cells_count(), outside.len());
            assert!(plane.chunks.values().all(|chunk| **chunk != [0; 64]));
        }
    }

    #[test]
    fn a_run_brings_cells_to_life_as_populating_each_would() {
        let mut rng = fastrand::Rng::with_seed(29);
        for trial in 0..40 {
            // Under way, so the fingerprint has something in it to keep.
            let mut by_run = board_with(soup(400, 80));
            for _ in 0..5 {
                by_run.step();
            }
            let _ = by_run.take_dirty_chunks();
            let mut by_cell = by_run.clone();
            for _ in 0..30 {
                let (x, y) = (rng.i64(-150..150), rng.i64(-150..150));
                let len = [1, 63, 64, 65, 129, rng.usize(0..300)][rng.usize(0..6)];
                by_run.populate_run(x, y, len);
                for x in x..x + len as i64 {
                    by_cell.populate(x, y);
                }
            }
            let context = format!("trial {trial}");
            assert_eq!(by_run.take_dirty_chunks(), by_cell.take_dirty_chunks());
            for generation in 0..6 {
                assert_eq!(
                    live_cells(&by_run),
                    live_cells(&by_cell),
                    "{context} {generation}"
                );
                assert_eq!(by_run.live_cells_count(), by_cell.live_cells_count());
                assert_eq!(by_run.chunk_count(), by_cell.chunk_count());
                assert_eq!(by_run.fingerprint(), by_cell.fingerprint(), "{context}");
                // Waking the same rows, so the two go on to the same place.
                assert_eq!(by_run.active, by_cell.active, "{context} {generation}");
                by_run.step();
                by_cell.step();
            }
        }
    }
}
