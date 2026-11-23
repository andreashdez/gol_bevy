//! How often the app updates: every frame while anything on screen moves,
//! twice a generation while only a run at a set speed does, and otherwise only
//! when input arrives, so an idle window costs no CPU or GPU.
use crate::{
    feedback::EditFeedback,
    shared::{DiagnosticsOverlay, Simulation},
};
use bevy::{
    a11y::AccessibilityRequested,
    input_focus::InputFocus,
    prelude::*,
    text::EditableText,
    window::WindowEvent,
    winit::{UpdateMode, WinitSettings},
};
use std::time::Duration;

pub struct FramePacingPlugin;

impl Plugin for FramePacingPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<StayAwake>()
            .insert_resource(busy())
            .add_systems(Last, pace_frames);
    }
}

/// Keeps frames coming for one more frame and [`SETTLE`] after it. Whatever
/// changes on screen without input writes it on every frame it is busy, such as
/// a file being read or a pulse that is still fading.
#[derive(Message)]
pub struct StayAwake;

/// How long frames keep coming after the last input or busy frame. Edits made
/// from input show a frame later, and some things follow input after a short
/// delay of their own, such as a tooltip, the minimap, or saving settings; each
/// of those checks that its delay fits in this.
pub const SETTLE: Duration = Duration::from_secs(1);

/// Bevy's default: as fast as the display draws while focused, and at 60 Hz
/// while another window is.
fn busy() -> WinitSettings {
    WinitSettings::game()
}

/// Waiting for input. The waits are only a backstop, since anything that changes
/// without input keeps the app busy.
fn idle() -> WinitSettings {
    WinitSettings {
        focused_mode: UpdateMode::reactive_low_power(Duration::from_secs(5)),
        unfocused_mode: UpdateMode::reactive_low_power(Duration::from_secs(60)),
    }
}

/// The longest wait between paced frames, so a slow run still updates often
/// enough for the minimap and the measured rate. It stays under the most a
/// frame's virtual clock may advance (250 ms by default), or slow runs would
/// lose time.
const PACED_FRAME_MAX: Duration = Duration::from_millis(100);

/// While a run at a set speed is all that changes: two frames a generation, so
/// every generation shows however the frames fall, rather than one a display
/// refresh. The frame work of Bevy and the GPU, not the board, is most of what
/// a frame costs, so on a small board this is most of the saving.
fn paced(generation: Duration) -> WinitSettings {
    let frame = (generation / 2).min(PACED_FRAME_MAX);
    WinitSettings {
        focused_mode: UpdateMode::reactive_low_power(frame),
        unfocused_mode: UpdateMode::reactive_low_power(frame),
    }
}

/// What changes on screen without input, read where it is kept.
#[derive(bevy::ecs::system::SystemParam)]
struct Activity<'w, 's> {
    sim: Res<'w, Simulation>,
    feedback: Res<'w, EditFeedback>,
    diagnostics: Res<'w, DiagnosticsOverlay>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    focus: Res<'w, InputFocus>,
    text_fields: Query<'w, 's, (), With<EditableText>>,
}

impl Activity<'_, '_> {
    fn any(&self) -> bool {
        // Max speed steps for as long as each frame allows.
        (self.sim.app.is_running() && self.sim.app.is_max_speed())
            // A notice counts down, and a paste may still be on its way.
            || self.feedback.is_active()
            // The diagnostics show the frame rate.
            || self.diagnostics.enabled
            // Held keys pan the camera, and a held button may be mid-drag.
            || self.keys.get_pressed().next().is_some()
            || self.mouse.get_pressed().next().is_some()
            // The text cursor blinks.
            || self
                .focus
                .get()
                .is_some_and(|entity| self.text_fields.contains(entity))
    }

    /// The time between generations of a run at a set speed.
    fn generation_interval(&self) -> Option<Duration> {
        let app = &self.sim.app;
        (app.is_running() && !app.is_max_speed()).then(|| Duration::from_millis(app.tick_rate_ms()))
    }
}

fn pace_frames(
    time: Res<Time<Real>>,
    mut window_events: MessageReader<WindowEvent>,
    mut stay_awake: MessageReader<StayAwake>,
    activity: Activity,
    // A screen reader's requests reach the app without waking it, so with one
    // listening the app keeps updating.
    accessibility: Res<AccessibilityRequested>,
    mut settle_until: Local<Duration>,
    mut settings: ResMut<WinitSettings>,
) {
    // Redraw requests are how a busy app asks for its next frame, and pointer
    // motion is reported even outside the window, so neither counts as input.
    // Every event is read, so none is left over to count again next frame.
    let input = window_events
        .read()
        .filter(|event| {
            !matches!(
                event,
                WindowEvent::RequestRedraw(_) | WindowEvent::MouseMotion(_)
            )
        })
        .count()
        != 0;
    let asked = !stay_awake.is_empty();
    stay_awake.clear();
    let now = time.elapsed();
    if input || asked || activity.any() {
        *settle_until = now + SETTLE;
    }
    let wanted = if now < *settle_until || accessibility.get() {
        busy()
    } else if let Some(generation) = activity.generation_interval() {
        paced(generation)
    } else {
        idle()
    };
    if settings.focused_mode != wanted.focused_mode
        || settings.unfocused_mode != wanted.unfocused_mode
    {
        *settings = wanted;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App as GolApp;
    use bevy::{ecs::message::Messages, window::WindowFocused};

    fn world() -> World {
        let mut world = World::new();
        world.init_resource::<Time<Real>>();
        world.init_resource::<Messages<WindowEvent>>();
        world.init_resource::<Messages<StayAwake>>();
        world.insert_resource(Simulation::new(GolApp::new(8, 8)));
        world.init_resource::<EditFeedback>();
        world.insert_resource(DiagnosticsOverlay { enabled: false });
        world.init_resource::<ButtonInput<KeyCode>>();
        world.init_resource::<ButtonInput<MouseButton>>();
        world.init_resource::<InputFocus>();
        world.init_resource::<AccessibilityRequested>();
        world.insert_resource(busy());
        world
    }

    fn updates_continuously(world: &World) -> bool {
        world.resource::<WinitSettings>().focused_mode == UpdateMode::Continuous
    }

    #[test]
    fn input_keeps_frames_coming_until_the_app_settles() {
        let mut world = world();
        let pace = world.register_system(pace_frames);
        world.run_system(pace).unwrap();
        assert!(!updates_continuously(&world), "an untouched app waits");

        world.write_message(WindowEvent::WindowFocused(WindowFocused {
            window: Entity::PLACEHOLDER,
            focused: true,
        }));
        world.run_system(pace).unwrap();
        assert!(updates_continuously(&world));

        let step = SETTLE - Duration::from_millis(1);
        world.resource_mut::<Time<Real>>().advance_by(step);
        world.run_system(pace).unwrap();
        assert!(updates_continuously(&world), "still settling");

        world
            .resource_mut::<Time<Real>>()
            .advance_by(Duration::from_millis(1));
        world.run_system(pace).unwrap();
        assert!(!updates_continuously(&world), "settled");
    }

    #[test]
    fn a_run_paces_frames_to_its_speed_but_its_own_redraw_requests_do_not_count() {
        let mut world = world();
        let pace = world.register_system(pace_frames);
        let mode = |world: &World| world.resource::<WinitSettings>().focused_mode;
        let run = |world: &mut World, change: &dyn Fn(&mut GolApp)| {
            change(&mut world.resource_mut::<Simulation>().app);
            world.run_system(pace).unwrap();
        };

        // Two frames a generation, and at least ten a second.
        run(&mut world, &|app| {
            app.set_tick_rate_ms(100);
            app.toggle_running();
        });
        let frame = |ms| UpdateMode::reactive_low_power(Duration::from_millis(ms));
        assert_eq!(mode(&world), frame(50));
        run(&mut world, &|app| {
            app.set_tick_rate_ms(1000);
        });
        assert_eq!(mode(&world), frame(100));

        // Max speed steps every frame it gets.
        run(&mut world, &|app| {
            app.set_max_speed();
        });
        assert!(updates_continuously(&world));
        assert!(PACED_FRAME_MAX < Time::<Virtual>::default().max_delta());

        run(&mut world, &|app| app.toggle_running());
        world.resource_mut::<Time<Real>>().advance_by(SETTLE);
        world.write_message(WindowEvent::RequestRedraw(bevy::window::RequestRedraw));
        run(&mut world, &|_| {});
        assert_eq!(mode(&world), idle().focused_mode, "paused and settled");
    }
}
