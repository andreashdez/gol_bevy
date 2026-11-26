//! What both boards are made of: 64 × 64 chunks of cells at a bit each, and the
//! step that works out a chunk's next rows 64 cells at a time. The finite board
//! keeps chunks in a grid and the unbounded one in a map; each reads a chunk's
//! neighbours its own way, past its edges or across the plane, and hands the
//! row sums to `next_rows`.
use crate::rule::Rule;
use bevy::tasks::{ComputeTaskPool, TaskPool};

pub(crate) const CHUNK_BITS: u32 = 6;
/// Width and height of a chunk in cells.
pub const CHUNK_SIZE: i64 = 1 << CHUNK_BITS;
/// [`CHUNK_SIZE`], to index rows and columns with.
pub(crate) const SIDE: usize = CHUNK_SIZE as usize;
/// Index of a chunk's last row, and of its last column.
pub(crate) const LAST: usize = SIDE - 1;

/// One row per `u64`, with cell `x` in bit `x`. A row holds as many cells as a
/// machine word, so a generation updates them all at once.
pub type Chunk = [u64; SIDE];

pub(crate) static EMPTY_CHUNK: Chunk = [0; SIDE];

/// Fewest scheduled rows that justify another parallel band, on either board. A
/// row of 30% noise takes about 50 ns, fingerprint and all, so a band of these
/// is some 50 µs of work, well beyond what it costs to wake a thread of the task
/// pool and wait for it. It is also sixteen whole chunks, so a board busy
/// everywhere splits as it would if bands were counted in chunks, while settled
/// debris, whose chunks have a row or two scheduled each, stays on one thread
/// rather than spreading a few microseconds of work across all of them.
const MIN_BAND_ROWS: usize = 1024;

/// Which rows of a chunk changed: any of their cells, and those in its first and
/// last columns, which is what the chunks either side of it can see.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ChangedRows {
    pub(crate) any: u64,
    pub(crate) west: u64,
    pub(crate) east: u64,
}

impl ChangedRows {
    pub(crate) const ALL: Self = Self {
        any: !0,
        west: !0,
        east: !0,
    };

    pub(crate) fn cell(x: usize, y: usize) -> Self {
        let row = 1 << y;
        Self {
            any: row,
            west: if x == 0 { row } else { 0 },
            east: if x == LAST { row } else { 0 },
        }
    }
}

/// A chunk's next generation, staged so bands can share the old one. `K` finds
/// the chunk: a slot of the finite board's grid, or a key of the plane's map.
#[derive(Debug, Clone)]
pub(crate) struct ChunkChange<K> {
    pub(crate) key: K,
    pub(crate) next: Chunk,
    pub(crate) rows: ChangedRows,
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

/// A well-mixed key per cell (SplitMix64), so that no set of flips is likely to
/// cancel out unless each cell flipped back.
pub(crate) fn cell_key(idx: u64) -> u64 {
    let value = idx.wrapping_add(1).wrapping_mul(0x9E3779B97F4A7C15);
    let value = (value ^ (value >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    let value = (value ^ (value >> 27)).wrapping_mul(0x94D049BB133111EB);
    value ^ (value >> 31)
}

/// The number of bands to step `rows` scheduled rows in: one per thread of the
/// compute task pool, as long as each gets [`MIN_BAND_ROWS`].
pub(crate) fn bands_for(rows: usize) -> usize {
    let threads = ComputeTaskPool::get_or_init(TaskPool::default).thread_num();
    threads.min(rows / MIN_BAND_ROWS)
}

/// Evaluates the scheduled rows of `work` in as many bands of whole chunks as
/// `bands` asks for, given the number of rows, each with about as many rows.
/// Bands run in parallel on the compute task pool when there is more than one,
/// and `step_band` only reads the chunks and stages a band's changes in its list
/// of `changes`, to be applied once all have finished, so no band can see
/// another's half of the generation. Returns the change in the live count and
/// the XOR of the keys of every cell that flipped.
pub(crate) fn step_bands<K: Sync, C: Send>(
    work: &[(K, u64)],
    bands: impl FnOnce(usize) -> usize,
    changes: &mut Vec<Vec<C>>,
    step_band: impl Fn(&[(K, u64)], &mut Vec<C>) -> (isize, u64) + Sync,
) -> (isize, u64) {
    let rows = work
        .iter()
        .map(|&(_, rows)| rows.count_ones() as usize)
        .sum();
    let bands = bands(rows).max(1);
    if changes.len() < bands {
        changes.resize_with(bands, Vec::new);
    }
    if bands == 1 {
        return step_band(work, &mut changes[0]);
    }
    let step_band = &step_band;
    ComputeTaskPool::get_or_init(TaskPool::default)
        .scope(|scope| {
            for (band, changes) in split_bands(work, rows, bands).zip(changes) {
                scope.spawn(async move { step_band(band, changes) });
            }
        })
        .into_iter()
        .fold((0, 0), |(live, flips), (band_live, band_flips)| {
            (live + band_live, flips ^ band_flips)
        })
}

/// Splits scheduled chunks into at most `bands` runs of whole chunks, each with
/// about as many of the `rows` scheduled rows as the others, so that bands take
/// about as long however unevenly the rows fall among the chunks.
fn split_bands<K>(
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

/// Keeps the cells of the `scheduled` rows of `next` that are `on_board`, and
/// compares those rows with `old`. Returns which rows changed, with `east_col`
/// the chunk's last column on the board, the change in live cells, and the XOR
/// of `flip_key(row_key(row), col)` for each cell that flipped.
pub(crate) fn compare_rows(
    old: &Chunk,
    next: &mut Chunk,
    scheduled: u64,
    on_board: u64,
    east_col: usize,
    row_key: impl Fn(usize) -> u64,
    flip_key: impl Fn(u64, usize) -> u64,
) -> (ChangedRows, isize, u64) {
    let (mut rows, mut live_delta, mut flips) = (ChangedRows::default(), 0, 0);
    for row in bits(scheduled) {
        next[row] &= on_board;
        let flipped = old[row] ^ next[row];
        if flipped == 0 {
            continue;
        }
        rows.any |= 1 << row;
        rows.west |= (flipped & 1) << row;
        rows.east |= (flipped >> east_col & 1) << row;
        live_delta += next[row].count_ones() as isize - old[row].count_ones() as isize;
        let row_key = row_key(row);
        for col in bits(flipped) {
            flips ^= flip_key(row_key, col);
        }
    }
    (rows, live_delta, flips)
}

/// The line [`next_rows`] asks for the row above a chunk's last row by.
pub(crate) const ROW_ABOVE: usize = SIDE + 1;

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
    use super::split_bands;

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
}
