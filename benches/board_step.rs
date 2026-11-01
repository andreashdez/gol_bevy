use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use gol_bevy::{
    board::{Board, DirtyCells},
    sparse_board::SparseBoard,
};
use std::hint::black_box;

fn seed_glider(board: &mut Board) {
    let (cols, rows) = board.size();
    if cols < 3 || rows < 3 {
        return;
    }

    let origin_col = cols / 2;
    let origin_row = rows / 2;
    let pattern = [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)];
    for (dx, dy) in pattern {
        let _ = board.populate(origin_col + dx, origin_row + dy);
    }
}

/// Deterministic noise over a `cols` × `rows` area, so both boards get the same cells.
fn noise_cells(
    cols: usize,
    rows: usize,
    live_percent: u64,
) -> impl Iterator<Item = (usize, usize)> {
    (0..rows)
        .flat_map(move |row| (0..cols).map(move |col| (col, row)))
        .filter(move |&(col, row)| {
            let idx = (row * cols + col) as u64;
            let mut x = idx.wrapping_add(0x9E3779B97F4A7C15);
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
            x ^= x >> 31;
            x % 100 < live_percent
        })
}

fn seed_noise(board: &mut Board, live_percent: u64) {
    let (cols, rows) = board.size();
    for (col, row) in noise_cells(cols, rows, live_percent) {
        let _ = board.populate(col, row);
    }
}

fn step_and_extract(board: &mut Board) -> usize {
    let _ = board.step();
    match board.take_dirty_cells() {
        DirtyCells::None => 0,
        DirtyCells::Full => usize::MAX,
        DirtyCells::Cells(indices) => indices.len(),
    }
}

fn bench_board_step_and_dirty(c: &mut Criterion) {
    let mut group = c.benchmark_group("board_step_dirty");

    for size in [256, 512, 1024] {
        let mut sparse_board = Board::new(size, size);
        seed_glider(&mut sparse_board);
        let _ = sparse_board.take_dirty_cells();
        group.bench_function(format!("sparse_glider_{size}"), |b| {
            b.iter(|| {
                let dirty_count = step_and_extract(black_box(&mut sparse_board));
                black_box(dirty_count);
            })
        });

        let mut medium_board = Board::new(size, size);
        seed_noise(&mut medium_board, 30);
        let _ = medium_board.take_dirty_cells();
        group.bench_function(format!("noise30_{size}"), |b| {
            b.iter(|| {
                let dirty_count = step_and_extract(black_box(&mut medium_board));
                black_box(dirty_count);
            })
        });

        let mut dense_board = Board::new(size, size);
        seed_noise(&mut dense_board, 45);
        let _ = dense_board.take_dirty_cells();
        group.bench_function(format!("noise45_{size}"), |b| {
            b.iter(|| {
                let dirty_count = step_and_extract(black_box(&mut dense_board));
                black_box(dirty_count);
            })
        });
    }

    group.finish();
}

/// The groups above step one board thousands of times, so noise settles into
/// sparse debris and mostly exercises the frontier path. Stepping a fresh copy
/// of a 30%-live board each iteration keeps every measurement on the dense scan.
fn bench_dense_scan(c: &mut Criterion) {
    let mut group = c.benchmark_group("board_step_dense");
    group.sample_size(20);

    for size in [512, 1024, 2048] {
        let mut seeded = Board::new(size, size);
        seed_noise(&mut seeded, 30);
        let _ = seeded.take_dirty_cells();
        group.bench_function(format!("fresh_noise30_{size}"), |b| {
            b.iter_batched_ref(
                || seeded.clone(),
                |board| black_box(step_and_extract(board)),
                BatchSize::LargeInput,
            )
        });
    }

    group.finish();
}

/// The same glider and fresh noise as above on the unbounded board, so the two
/// boards can be compared directly.
fn bench_sparse_board(c: &mut Criterion) {
    let mut group = c.benchmark_group("sparse_board_step");
    group.sample_size(20);

    let mut glider = SparseBoard::default();
    for (x, y) in [(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)] {
        glider.populate(x, y);
    }
    let _ = glider.take_dirty_chunks();
    group.bench_function("glider", |b| {
        b.iter(|| {
            let _ = black_box(&mut glider).step();
            black_box(glider.take_dirty_chunks())
        })
    });

    for size in [512, 1024, 2048] {
        let mut seeded = SparseBoard::default();
        for (col, row) in noise_cells(size, size, 30) {
            seeded.populate(col as i64, row as i64);
        }
        let _ = seeded.take_dirty_chunks();
        group.bench_function(format!("fresh_noise30_{size}"), |b| {
            b.iter_batched_ref(
                || seeded.clone(),
                |board| {
                    let _ = board.step();
                    black_box(board.take_dirty_chunks())
                },
                BatchSize::LargeInput,
            )
        });
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_board_step_and_dirty,
    bench_dense_scan,
    bench_sparse_board
);
criterion_main!(benches);
