//! How often the app updates: every frame while anything on screen moves, and
//! otherwise only when input arrives, so an idle window costs no CPU or GPU.
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
        self.sim.app.is_running()
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
    fn a_running_board_keeps_frames_coming_but_its_own_redraw_requests_do_not() {
        let mut world = world();
        let pace = world.register_system(pace_frames);
        world.resource_mut::<Simulation>().app.toggle_running();
        world.run_system(pace).unwrap();
        assert!(updates_continuously(&world));

        world.resource_mut::<Simulation>().app.toggle_running();
        world.resource_mut::<Time<Real>>().advance_by(SETTLE);
        world.write_message(WindowEvent::RequestRedraw(bevy::window::RequestRedraw));
        world.run_system(pace).unwrap();
        assert!(!updates_continuously(&world), "paused and settled");
    }
}
