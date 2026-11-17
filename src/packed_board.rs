//! A finite board at a bit per cell, stepped by the unbounded board's row kernel.
//! It is a prototype, measured against [`Board`](crate::board::Board) and checked
//! cell for cell against it. The app does not use it yet.
//!
//! Cells sit in the same 64 × 64 chunks as on the unbounded board, in a grid
//! that covers the board instead of a map, and rows are scheduled the same way.
//! What differs is the edges: a row's neighbours come from the far side of the
//! board when it wraps and from nowhere when it does not, and the last chunk
//! across and the last one up may be only partly on the board.
use crate::{
    board::{BoundaryMode, DirtyCells, Rule, cell_key},
    sparse_board::{CHUNK_SIZE, ChangedRows, Chunk, RowPlanes, bits, next_cells},
};
use bevy::tasks::{ComputeTaskPool, TaskPool};

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
pub struct PackedBoard {
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
    /// XOR of [`cell_key`] for every flip made by stepping, as on [`Board`]
    ///
    /// [`Board`]: crate::board::Board
    fingerprint: u64,
    rule: Rule,
}

impl PackedBoard {
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

    pub fn size(&self) -> (usize, usize) {
        (self.shape.cols, self.shape.rows)
    }

    pub fn boundary_mode(&self) -> BoundaryMode {
        self.shape.mode
    }

    pub fn set_boundary_mode(&mut self, mode: BoundaryMode) {
        if self.shape.mode != mode {
            self.shape.mode = mode;
            self.wake_all();
        }
    }

    pub fn rule(&self) -> Rule {
        self.rule
    }

    pub fn set_rule(&mut self, rule: Rule) -> bool {
        if self.rule == rule {
            return false;
        }
        self.rule = rule;
        self.wake_all();
        true
    }

    pub fn is_alive(&self, col: usize, row: usize) -> bool {
        self.locate(col, row)
            .is_some_and(|(slot, x, y)| self.chunks[slot][y] >> x & 1 != 0)
    }

    pub fn populate(&mut self, col: usize, row: usize) -> bool {
        self.set_cell(col, row, true)
    }

    pub fn depopulate(&mut self, col: usize, row: usize) -> bool {
        self.set_cell(col, row, false)
    }

    pub fn clear(&mut self) -> bool {
        if self.live_count == 0 {
            return false;
        }
        self.chunks.fill(EMPTY_CHUNK);
        self.live_count = 0;
        self.mark_dirty_full();
        self.wake_all();
        true
    }

    pub fn live_cells_count(&self) -> usize {
        self.live_count
    }

    /// Identifies the current generation among others of the same run, with the
    /// same value [`Board`](crate::board::Board) gives the same flips.
    pub fn fingerprint(&self) -> u64 {
        self.fingerprint
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

    /// The cells that changed since this was last called, as
    /// [`Board`](crate::board::Board) reports them.
    pub fn take_dirty_cells(&mut self) -> DirtyCells {
        if self.dirty_full {
            self.dirty_full = false;
            return DirtyCells::Full;
        }
        if self.dirty_list.is_empty() {
            return DirtyCells::None;
        }

        // Counted first, as growing a list of a million cells copies it many times.
        let mut count = 0;
        for &slot in &self.dirty_list {
            let slot = slot as usize;
            for row in bits(self.dirty_rows[slot]) {
                count += self.dirty[slot][row].count_ones();
            }
        }
        let mut cells = Vec::with_capacity(count as usize);
        for slot in self.dirty_list.drain(..) {
            let slot = slot as usize;
            let (cx, cy) = (slot % self.shape.across, slot / self.shape.across);
            for row in bits(std::mem::take(&mut self.dirty_rows[slot])) {
                let first = (cy * SIDE + row) * self.shape.cols + cx * SIDE;
                let changed = std::mem::take(&mut self.dirty[slot][row]);
                cells.extend(bits(changed).map(|col| first + col));
            }
        }
        DirtyCells::Cells(cells)
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
    /// that can see them: round the board where it wraps.
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

#[cfg(test)]
mod tests {
    use super::{MIN_BAND_CHUNKS, PackedBoard};
    use crate::board::{Board, BoundaryMode, DirtyCells, Rule};

    /// The same random cells on both kinds of board.
    fn soup(cols: usize, rows: usize, density: f64, seed: u64) -> (PackedBoard, Board) {
        let mut rng = fastrand::Rng::with_seed(seed);
        let (mut packed, mut board) = (PackedBoard::new(cols, rows), Board::new(cols, rows));
        for row in 0..rows {
            for col in 0..cols {
                if rng.f64() < density {
                    assert!(packed.populate(col, row));
                    assert!(board.populate(col, row));
                }
            }
        }
        (packed, board)
    }

    fn sorted(dirty: DirtyCells) -> DirtyCells {
        match dirty {
            DirtyCells::Cells(mut cells) => {
                cells.sort_unstable();
                DirtyCells::Cells(cells)
            }
            other => other,
        }
    }

    fn assert_same(packed: &mut PackedBoard, board: &mut Board, context: &str) {
        let (cols, rows) = board.size();
        for row in 0..rows {
            for col in 0..cols {
                assert_eq!(
                    packed.is_alive(col, row),
                    board.is_alive(col, row),
                    "cell ({col}, {row}), {context}"
                );
            }
        }
        assert_eq!(
            packed.live_cells_count(),
            board.live_cells_count(),
            "{context}"
        );
        assert_eq!(packed.fingerprint(), board.fingerprint(), "{context}");
        assert_eq!(
            sorted(packed.take_dirty_cells()),
            sorted(board.take_dirty_cells()),
            "{context}"
        );
    }

    #[test]
    fn stepping_matches_the_byte_board_at_every_size_edge_and_rule() {
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
                    let seed = (cols * 1000 + rows + index) as u64;
                    let (mut packed, mut board) = soup(cols, rows, 0.35, seed);
                    packed.set_boundary_mode(mode);
                    board.set_boundary_mode(mode);
                    packed.set_rule(rule);
                    board.set_rule(rule);
                    assert_same(&mut packed, &mut board, "before stepping");

                    for generation in 1..=24 {
                        let context = format!("{cols}x{rows} {mode:?} {rule:?} gen {generation}");
                        assert_eq!(packed.step(), board.step(), "{context}");
                        assert_same(&mut packed, &mut board, &context);
                    }
                }
            }
        }
    }

    #[test]
    fn edits_between_generations_match_the_byte_board() {
        let (cols, rows) = (150, 90);
        for mode in [BoundaryMode::Wrap, BoundaryMode::Fixed] {
            let (mut packed, mut board) = soup(cols, rows, 0.2, 9);
            packed.set_boundary_mode(mode);
            board.set_boundary_mode(mode);
            let mut rng = fastrand::Rng::with_seed(3);
            for generation in 0..60 {
                // Corners and edges as often as anywhere else.
                for _ in 0..8 {
                    let col = [0, 63, 64, cols - 1, rng.usize(..cols)][rng.usize(..5)];
                    let row = [0, 63, 64, rows - 1, rng.usize(..rows)][rng.usize(..5)];
                    if rng.bool() {
                        assert_eq!(packed.populate(col, row), board.populate(col, row));
                    } else {
                        assert_eq!(packed.depopulate(col, row), board.depopulate(col, row));
                    }
                }
                assert_eq!(packed.step(), board.step());
                assert_same(
                    &mut packed,
                    &mut board,
                    &format!("{mode:?} gen {generation}"),
                );
            }
        }
    }

    #[test]
    fn a_settled_board_wakes_for_an_edit_a_rule_or_new_edges() {
        let (mut packed, mut board) = (PackedBoard::new(200, 140), Board::new(200, 140));
        // A block on the east edge, still under either kind of edge.
        for (col, row) in [(198, 70), (199, 70), (198, 71), (199, 71)] {
            packed.populate(col, row);
            board.populate(col, row);
        }
        assert!(!packed.step());
        assert!(packed.active_list.is_empty());

        // Cells on the west edge that only see the block round the board.
        packed.populate(0, 69);
        board.populate(0, 69);
        for _ in 0..4 {
            assert_eq!(packed.step(), board.step());
        }
        assert_same(&mut packed, &mut board, "after an edit");

        packed.set_boundary_mode(BoundaryMode::Fixed);
        board.set_boundary_mode(BoundaryMode::Fixed);
        let seeds = Rule::parse("B2/S").unwrap();
        assert!(packed.set_rule(seeds) && board.set_rule(seeds));
        assert!(!packed.set_rule(seeds));
        for _ in 0..12 {
            assert_eq!(packed.step(), board.step());
        }
        assert_same(&mut packed, &mut board, "after new edges and a new rule");
    }

    #[test]
    fn bands_step_like_a_serial_scan() {
        let (mut banded, _) = soup(600, 520, 0.3, 21);
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
        let mut board = PackedBoard::new(1024, 1024);
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
    fn edits_and_clearing_report_what_changed() {
        let mut board = PackedBoard::new(100, 70);
        assert_eq!(board.take_dirty_cells(), DirtyCells::Full);
        assert_eq!(board.take_dirty_cells(), DirtyCells::None);

        assert!(board.populate(99, 69));
        assert!(!board.populate(99, 69));
        assert!(!board.populate(100, 69) && !board.populate(99, 70));
        assert!(board.is_alive(99, 69) && !board.is_alive(100, 69));
        assert_eq!(board.live_cells_count(), 1);
        assert_eq!(
            board.take_dirty_cells(),
            DirtyCells::Cells(vec![69 * 100 + 99])
        );

        assert!(board.depopulate(99, 69));
        assert!(!board.depopulate(99, 69));
        assert!(!board.clear());
        board.populate(5, 5);
        assert!(board.clear());
        assert_eq!(board.live_cells_count(), 0);
        assert_eq!(board.take_dirty_cells(), DirtyCells::Full);
        assert!(!board.step());
    }
}
