//! Finds where the dense scan overtakes the frontier step, to tune
//! `FRONTIER_DENSE_THRESHOLD_PERCENT`. Each case times one generation both ways
//! from the same state, for two workloads: young soup in a block covering part
//! of the board, and the settled debris a whole random board decays into. The
//! printed candidate share is what the threshold is compared against.
//!
//! Re-run after changing the cost of either path:
//! `cargo bench --bench dense_crossover`
use criterion::{BatchSize, Criterion, criterion_group};
use gol_bevy::board::{Board, DirtyCells};
use std::hint::black_box;

const SEED_LIVE_PERCENT: u64 = 30;
const SOUP_GENERATIONS: usize = 20;
const BLOCK_PERCENTS: [usize; 5] = [1, 2, 4, 6, 10];
/// Long enough for random noise to settle into still lifes and oscillators.
const DEBRIS_GENERATIONS: usize = 2000;

fn hash_live(idx: u64) -> bool {
    let mut x = idx.wrapping_add(0x9E3779B97F4A7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D049BB133111EB);
    x ^= x >> 31;
    x % 100 < SEED_LIVE_PERCENT
}

/// Noise in a centred square covering `block_percent` of the board, run for
/// `generations` so its frontier is the cells that changed in the latest one.
fn seeded_board(size: usize, block_percent: usize, generations: usize) -> (Board, f64) {
    let mut board = Board::new(size, size);
    let side = ((size * size * block_percent / 100) as f64).sqrt() as usize;
    let origin = (size - side) / 2;
    for row in origin..origin + side {
        for col in origin..origin + side {
            if hash_live((row * size + col) as u64) {
                board.populate(col, row);
            }
        }
    }
    for _ in 0..generations - 1 {
        board.step();
    }
    let _ = board.take_dirty_cells();
    board.step();
    let DirtyCells::Cells(frontier) = board.take_dirty_cells() else {
        panic!("a board that is still active reports the cells it changed");
    };
    (board, candidate_share(&frontier, size))
}

/// Share of the board the frontier step would evaluate: the changed cells and
/// their wrapped neighbours.
fn candidate_share(frontier: &[usize], size: usize) -> f64 {
    let mut candidate = vec![false; size * size];
    for &idx in frontier {
        let (col, row) = (idx % size, idx / size);
        for dy in [size - 1, 0, 1] {
            for dx in [size - 1, 0, 1] {
                candidate[(row + dy) % size * size + (col + dx) % size] = true;
            }
        }
    }
    candidate.iter().filter(|&&c| c).count() as f64 / (size * size) as f64
}

fn bench_crossover(c: &mut Criterion) {
    let mut group = c.benchmark_group("dense_crossover");
    group.sample_size(20);

    let mut cases = Vec::new();
    for size in [1024, 2048] {
        for block_percent in BLOCK_PERCENTS {
            cases.push((
                format!("soup{size}_block{block_percent}"),
                size,
                block_percent,
                SOUP_GENERATIONS,
            ));
        }
    }
    for size in [256, 512, 1024, 2048] {
        cases.push((format!("debris{size}"), size, 100, DEBRIS_GENERATIONS));
    }

    for (label, size, block_percent, generations) in cases {
        let (board, share) = seeded_board(size, block_percent, generations);
        eprintln!(
            "{label}: candidates cover {:.2}% of the board",
            share * 100.0
        );
        bench_both_paths(&mut group, &label, &board);
    }

    group.finish();
}

fn bench_both_paths(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    label: &str,
    board: &Board,
) {
    group.bench_function(format!("{label}/frontier"), |b| {
        b.iter_batched_ref(
            || board.clone(),
            |board| black_box(board.step()),
            BatchSize::LargeInput,
        )
    });
    group.bench_function(format!("{label}/dense"), |b| {
        b.iter_batched_ref(
            || {
                let mut board = board.clone();
                // A boundary change invalidates the frontier, so toggling
                // twice leaves the mode alone but forces the dense scan.
                board.toggle_boundary_mode();
                board.toggle_boundary_mode();
                board
            },
            |board| black_box(board.step()),
            BatchSize::LargeInput,
        )
    });
}

criterion_group!(benches, bench_crossover);

fn main() {
    // The frontier case must never fall back to the dense scan, whatever the
    // default threshold is. Nothing has read the variable yet.
    // SAFETY: set before criterion starts any threads.
    unsafe { std::env::set_var("GOL_FRONTIER_DENSE_THRESHOLD_PERCENT", "100") };
    benches();
    Criterion::default().configure_from_args().final_summary();
}
