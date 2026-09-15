use criterion::{Criterion, criterion_group, criterion_main};
use gol_bevy::board::{Board, DirtyCells};
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

fn seed_noise(board: &mut Board, live_percent: u64) {
    let (cols, rows) = board.size();
    if cols == 0 || rows == 0 {
        return;
    }

    for row in 0..rows {
        for col in 0..cols {
            let idx = (row * cols + col) as u64;
            let mut x = idx.wrapping_add(0x9E3779B97F4A7C15);
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
            x ^= x >> 31;

            if x % 100 < live_percent {
                let _ = board.populate(col, row);
            }
        }
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

criterion_group!(benches, bench_board_step_and_dirty);
criterion_main!(benches);
