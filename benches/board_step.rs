use criterion::{black_box, criterion_group, criterion_main, Criterion};

#[allow(dead_code, unused_imports)]
#[path = "../src/board.rs"]
mod board;

use board::{Board, DirtyCells};

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

fn seed_blinker_field(board: &mut Board) {
    let (cols, rows) = board.size();
    if cols < 3 || rows == 0 {
        return;
    }

    for row in (0..rows).step_by(3) {
        for col in (0..cols.saturating_sub(2)).step_by(4) {
            let _ = board.populate(col, row);
            let _ = board.populate(col + 1, row);
            let _ = board.populate(col + 2, row);
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

    let mut sparse_board = Board::new(256, 256);
    seed_glider(&mut sparse_board);
    let _ = sparse_board.take_dirty_cells();
    group.bench_function("sparse_glider_256", |b| {
        b.iter(|| {
            let dirty_count = step_and_extract(&mut sparse_board);
            black_box(dirty_count);
        })
    });

    let mut dense_board = Board::new(256, 256);
    seed_blinker_field(&mut dense_board);
    let _ = dense_board.take_dirty_cells();
    group.bench_function("dense_blinkers_256", |b| {
        b.iter(|| {
            let dirty_count = step_and_extract(&mut dense_board);
            black_box(dirty_count);
        })
    });

    group.finish();
}

criterion_group!(benches, bench_board_step_and_dirty);
criterion_main!(benches);
