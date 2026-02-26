use std::time::Instant;

use crate::shared::{DiagnosticsOverlay, GridOverlay, RedrawRequested, Simulation, UpdateSet};
use bevy::{app::AppExit, prelude::*};

pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                advance_simulation.in_set(UpdateSet::Simulation),
                collect_redraw.in_set(UpdateSet::Redraw),
                apply_quit.in_set(UpdateSet::Quit),
            ),
        );
    }
}

fn advance_simulation(time: Res<Time>, mut sim: ResMut<Simulation>) {
    if !sim.app.is_running() {
        sim.accumulator = std::time::Duration::ZERO;
        return;
    }

    sim.accumulator += time.delta();
    let tick = std::time::Duration::from_millis(sim.app.tick_rate_ms());

    while sim.accumulator >= tick {
        let started = Instant::now();
        sim.app.increment_epoch();
        let step_ms = started.elapsed().as_secs_f64() * 1000.0;
        update_step_timing(&mut sim, step_ms);
        sim.accumulator -= tick;
    }
}

fn update_step_timing(sim: &mut Simulation, step_ms: f64) {
    sim.step_samples = sim.step_samples.saturating_add(1);
    if sim.step_samples == 1 {
        sim.avg_step_ms = step_ms;
        return;
    }

    const EMA_ALPHA: f64 = 0.15;
    sim.avg_step_ms = sim.avg_step_ms * (1.0 - EMA_ALPHA) + step_ms * EMA_ALPHA;
}

fn collect_redraw(
    mut sim: ResMut<Simulation>,
    diagnostics: Res<DiagnosticsOverlay>,
    grid_overlay: Res<GridOverlay>,
    mut redraw_requested: ResMut<RedrawRequested>,
) {
    redraw_requested.0 =
        sim.app.take_redraw() || diagnostics.is_changed() || grid_overlay.is_changed();
}

fn apply_quit(mut sim: ResMut<Simulation>, mut app_exit: MessageWriter<AppExit>) {
    if sim.app.should_quit {
        app_exit.write(AppExit::Success);
        sim.app.should_quit = false;
    }
}
