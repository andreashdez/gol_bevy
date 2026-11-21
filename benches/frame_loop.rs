//! Generations per second at max speed, as the app's frames run them: each frame
//! steps the board for the max-speed budget, then paints the board's tiles. The
//! step benchmarks in board_step.rs time a generation; this one also times how
//! many generations a frame gets, so a change to the frame budget or the
//! painting shows up as well as a change to stepping.
//!
//! A frame takes as long as its work, but no less than one refresh of a 120 Hz
//! display, which it waits for. That is how frames run on a variable-refresh
//! display such as a MacBook Pro's, where the app's frames take 8 to 9 ms. The
//! window, UI, and the GPU upload of the tiles are left out, so real frames are
//! a little longer.
//!
//! Criterion reports the time per generation, frames' waits included: the
//! inverse of generations per second at ∞.
use criterion::{Criterion, criterion_group, criterion_main};
use gol_bevy::{
    app::App as GolApp, board::RandomizeSettings, render_plugin::frame_loop::frame_app,
    shared::Simulation,
};
use std::time::{Duration, Instant};

const BOARD_SIDE: usize = 1024;
const REFRESH_MILLIHERTZ: u32 = 120_000;
/// Generations between randomizations, so that the board spends its time like
/// one randomized by hand every few seconds: freshly random, then settling.
const RANDOMIZE_EVERY: usize = 2_000;

fn frame_loop(c: &mut Criterion) {
    let refresh = Duration::from_secs_f64(1000.0 / f64::from(REFRESH_MILLIHERTZ));
    let mut group = c.benchmark_group("frame_loop");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(15));

    for cell_age in [false, true] {
        let name = format!(
            "max_speed_{BOARD_SIDE}_120hz{}",
            if cell_age { "_cell_age" } else { "" }
        );
        let mut board = GolApp::new(BOARD_SIDE, BOARD_SIDE);
        board.set_randomize_settings(RandomizeSettings::new(25, Some(7)).unwrap());
        board.randomize_board();
        board.set_max_speed();
        board.toggle_running();
        let mut app = frame_app(board, cell_age, REFRESH_MILLIHERTZ);
        let mut randomized_at = 0;

        group.bench_function(name, |b| {
            b.iter_custom(|generations| {
                let generation =
                    |app: &bevy::app::App| app.world().resource::<Simulation>().app.epoch;
                let start = generation(&app);
                let mut elapsed = Duration::ZERO;
                while generation(&app) - start < generations as usize {
                    if generation(&app) - randomized_at >= RANDOMIZE_EVERY {
                        randomized_at = generation(&app);
                        app.world_mut()
                            .resource_mut::<Simulation>()
                            .app
                            .randomize_board();
                    }
                    let started = Instant::now();
                    app.update();
                    elapsed += started.elapsed().max(refresh);
                }
                // A frame runs many generations, so the last one overshoots.
                let ran = generation(&app) - start;
                elapsed.mul_f64(generations as f64 / ran as f64)
            })
        });
    }
    group.finish();
}

criterion_group!(benches, frame_loop);
criterion_main!(benches);
