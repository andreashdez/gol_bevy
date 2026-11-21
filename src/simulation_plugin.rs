// Bevy's `Instant` reads the browser clock on the web, where `std`'s panics.
use bevy::platform::time::Instant;
use std::time::Duration;

use crate::shared::{
    DiagnosticsOverlay, EditFeedback, GridOverlay, RedrawRequested, Simulation, UpdateSet,
};
use bevy::{
    app::AppExit,
    prelude::*,
    window::{Monitor, OnMonitor, PrimaryMonitor, PrimaryWindow},
};

pub struct SimulationPlugin;

impl Plugin for SimulationPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EditFeedback>().add_systems(
            Update,
            (
                advance_simulation.in_set(UpdateSet::Simulation),
                collect_redraw.in_set(UpdateSet::Redraw),
                apply_quit.in_set(UpdateSet::Quit),
            ),
        );
    }
}

/// Generations a single frame may run to catch up. Without a cap, a stalled
/// frame (window drag, debugger pause, slow step) leaves a backlog whose
/// catch-up frame is slower still, and the app never recovers.
const MAX_STEPS_PER_FRAME: u64 = 8;

/// Share of a display refresh that max-speed stepping may take, leaving the
/// rest of the frame for input, UI, and rendering. At least one step runs
/// however long it takes.
const MAX_SPEED_FRAME_SHARE: f64 = 0.5;

/// Bounds on a max-speed frame's stepping time: at least enough for a step of
/// a busy board to be worth the frame, at most what a 60 Hz display leaves.
const MAX_SPEED_BUDGET_MIN: Duration = Duration::from_millis(2);
const MAX_SPEED_BUDGET_MAX: Duration = Duration::from_millis(8);

/// Assumed where the display does not say, as in the browser.
const DEFAULT_REFRESH_MILLIHERTZ: u32 = 60_000;

/// Bounds max speed on boards that step almost for free, such as empty or
/// settled ones, so that a frame does not spin through its whole budget.
const MAX_SPEED_STEPS_PER_FRAME: u64 = 1024;

fn advance_simulation(
    time: Res<Time>,
    mut sim: ResMut<Simulation>,
    mut feedback: ResMut<EditFeedback>,
    window: Query<&OnMonitor, With<PrimaryWindow>>,
    monitors: Query<(&Monitor, Has<PrimaryMonitor>)>,
) {
    // The monitor the window is on, or else the primary one.
    let refresh = window
        .single()
        .ok()
        .and_then(|on| monitors.get(on.0).ok())
        .or_else(|| monitors.iter().find(|(_, primary)| *primary))
        .and_then(|(monitor, _)| monitor.refresh_rate_millihertz);
    let budget = max_speed_budget(refresh);
    let steps = run_due_steps(&mut sim, time.delta(), budget);
    sim.rate.record(steps, time.delta());
    if let Some(settled) = sim.app.take_auto_paused() {
        feedback.show(format!("Paused · {settled}"));
    }
}

/// Stepping time for a max-speed frame on a display refreshing at
/// `refresh_millihertz`: a share of its frame, so a 120 Hz display keeps its
/// frame rate rather than dropping to 60 Hz under a fixed 60 Hz budget.
fn max_speed_budget(refresh_millihertz: Option<u32>) -> Duration {
    let refresh = refresh_millihertz
        .filter(|&refresh| refresh > 0)
        .unwrap_or(DEFAULT_REFRESH_MILLIHERTZ);
    let frame = Duration::from_secs_f64(1000.0 / f64::from(refresh));
    frame
        .mul_f64(MAX_SPEED_FRAME_SHARE)
        .clamp(MAX_SPEED_BUDGET_MIN, MAX_SPEED_BUDGET_MAX)
}

/// Returns how many generations ran.
fn run_due_steps(sim: &mut Simulation, delta: Duration, max_speed_budget: Duration) -> u64 {
    if !sim.app.is_running() {
        sim.accumulator = Duration::ZERO;
        return 0;
    }

    if sim.app.is_max_speed() {
        sim.accumulator = Duration::ZERO;
        let started = Instant::now();
        let mut steps = 0;
        loop {
            step(sim);
            steps += 1;
            // A step can pause the board by settling it.
            if steps == MAX_SPEED_STEPS_PER_FRAME
                || started.elapsed() >= max_speed_budget
                || !sim.app.is_running()
            {
                return steps;
            }
        }
    }

    sim.accumulator += delta;
    let tick = Duration::from_millis(sim.app.tick_rate_ms());

    let mut steps = 0;
    while sim.app.is_running() && sim.accumulator >= tick {
        if steps == MAX_STEPS_PER_FRAME {
            // Drop the backlog rather than carry it into the next frame.
            sim.accumulator = Duration::ZERO;
            break;
        }
        step(sim);
        sim.accumulator -= tick;
        steps += 1;
    }
    steps
}

fn step(sim: &mut Simulation) {
    let started = Instant::now();
    sim.app.increment_epoch();
    let step_ms = started.elapsed().as_secs_f64() * 1000.0;
    update_step_timing(sim, step_ms);
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

#[cfg(test)]
mod tests {
    use super::{MAX_SPEED_STEPS_PER_FRAME, MAX_STEPS_PER_FRAME, run_due_steps as run};
    use crate::{app::App as GolApp, shared::Simulation};
    use std::time::Duration;

    fn running_simulation() -> Simulation {
        let mut app = GolApp::new(8, 8);
        app.set_tick_rate_ms(25);
        app.toggle_running();
        Simulation::new(app)
    }

    fn run_due_steps(sim: &mut Simulation, delta: Duration) -> u64 {
        run(sim, delta, Duration::ZERO)
    }

    #[test]
    fn a_board_out_of_room_stops_a_max_speed_run_and_says_why() {
        use crate::shared::EditFeedback;
        use bevy::{ecs::system::RunSystemOnce, prelude::*};

        let mut sim = running_simulation();
        sim.app.set_unbounded().unwrap();
        sim.app.populate_board(3, 3);
        sim.app.set_chunk_budget(1);
        sim.app.set_max_speed();
        let mut world = World::new();
        world.insert_resource(sim);
        world.init_resource::<Time>();
        world.init_resource::<EditFeedback>();
        world.run_system_once(super::advance_simulation).unwrap();

        let sim = world.resource::<Simulation>();
        assert!(!sim.app.is_running());
        assert_eq!(sim.app.epoch, 0);
        assert_eq!(
            world.resource::<EditFeedback>().message,
            "Paused · the board reached its memory limit"
        );
    }

    #[test]
    fn steps_accumulate_across_frames() {
        let mut sim = running_simulation();
        run_due_steps(&mut sim, Duration::from_millis(60));
        assert_eq!(sim.app.epoch, 2);
        assert_eq!(sim.accumulator, Duration::from_millis(10));

        run_due_steps(&mut sim, Duration::from_millis(15));
        assert_eq!(sim.app.epoch, 3);
    }

    #[test]
    fn a_stalled_frame_runs_a_bounded_number_of_steps_and_drops_the_backlog() {
        let mut sim = running_simulation();
        run_due_steps(&mut sim, Duration::from_secs(10));
        assert_eq!(sim.app.epoch, MAX_STEPS_PER_FRAME as usize);
        assert_eq!(sim.accumulator, Duration::ZERO);

        // The next ordinary frame proceeds at the normal rate.
        run_due_steps(&mut sim, Duration::from_millis(30));
        assert_eq!(sim.app.epoch, MAX_STEPS_PER_FRAME as usize + 1);
    }

    #[test]
    fn a_paused_simulation_discards_elapsed_time() {
        let mut sim = running_simulation();
        sim.app.toggle_running();
        run_due_steps(&mut sim, Duration::from_millis(100));
        assert_eq!(sim.app.epoch, 0);
        assert_eq!(sim.accumulator, Duration::ZERO);

        sim.app.set_max_speed();
        assert_eq!(run_due_steps(&mut sim, Duration::from_millis(16)), 0);
        assert_eq!(sim.app.epoch, 0);
    }

    #[test]
    fn a_frame_stops_at_the_generation_that_settles_the_board() {
        let settling_simulation = || {
            let mut sim = running_simulation();
            sim.app.populate_board(3, 3);
            sim.app.set_pause_when_settled(true);
            sim
        };

        // A lone cell dies on the first generation, and the second repeats it.
        let mut sim = settling_simulation();
        assert_eq!(run_due_steps(&mut sim, Duration::from_secs(10)), 2);
        assert_eq!(sim.app.epoch, 2);
        assert!(!sim.app.is_running());

        let mut sim = settling_simulation();
        sim.app.set_max_speed();
        assert_eq!(run(&mut sim, Duration::ZERO, Duration::from_secs(60)), 2);
        assert_eq!(sim.app.epoch, 2);
        assert!(!sim.app.is_running());
    }

    #[test]
    fn max_speed_ignores_the_interval_and_always_makes_progress() {
        let mut sim = running_simulation();
        sim.app.set_max_speed();

        // Even with no budget left, every frame advances one generation.
        assert_eq!(run(&mut sim, Duration::ZERO, Duration::ZERO), 1);
        assert_eq!(sim.app.epoch, 1);

        // A generous budget on a cheap board runs up to the per-frame cap.
        let steps = run(&mut sim, Duration::ZERO, Duration::from_secs(60));
        assert_eq!(steps, MAX_SPEED_STEPS_PER_FRAME);
        assert_eq!(sim.app.epoch, 1 + MAX_SPEED_STEPS_PER_FRAME as usize);
    }

    #[test]
    fn max_speed_takes_half_of_the_displays_frame_within_bounds() {
        use super::max_speed_budget;
        let ms = |budget: Duration| budget.as_secs_f64() * 1000.0;
        // 60 Hz, or a display that doesn't say, keeps the old 8 ms.
        assert_eq!(max_speed_budget(None), Duration::from_millis(8));
        assert_eq!(max_speed_budget(Some(0)), Duration::from_millis(8));
        assert_eq!(max_speed_budget(Some(60_000)), Duration::from_millis(8));
        // 120 Hz gets half of its 8.3 ms frame.
        assert!((ms(max_speed_budget(Some(120_000))) - 25.0 / 6.0).abs() < 0.01);
        // Very fast displays still give a step room to be worth it.
        assert_eq!(max_speed_budget(Some(500_000)), Duration::from_millis(2));
    }
}
