//! Frontend-independent routing between UI controls and canvas input.
//! The UI publishes rectangles and modal state; this module owns capture and focus.
use crate::shared::UpdateSet;
use bevy::{
    input::ButtonState,
    input_focus::InputFocus,
    prelude::*,
    text::EditableText,
    ui_widgets::Slider,
    window::{PrimaryWindow, WindowEvent},
};

#[derive(Resource, Default)]
pub struct UiInputRegions {
    pub panels: Vec<Rect>,
    pub modal: bool,
}

impl UiInputRegions {
    fn contains(&self, point: Vec2) -> bool {
        self.modal || self.panels.iter().any(|rect| rect.contains(point))
    }
}

/// A focusable stand-in for the board: while it has focus, board shortcuts apply.
#[derive(Component)]
pub struct BoardFocus;

#[derive(Resource, Default)]
pub struct UiInputCapture {
    pub pointer: bool,
    pub keyboard: bool,
}
impl UiInputCapture {
    /// A dialog's closing click/key belongs to that dialog for the entire frame.
    pub fn block_modal_frame(&mut self) {
        self.pointer = true;
        self.keyboard = true;
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UiInputSet {
    Capture,
}

type Editors = Or<(With<EditableText>, With<Slider>)>;
type BoardTargets = Or<(With<PrimaryWindow>, With<BoardFocus>)>;

pub struct UiInputPlugin;
impl Plugin for UiInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiInputRegions>()
            .init_resource::<UiInputCapture>()
            .add_systems(First, begin_frame)
            .add_systems(
                Update,
                capture_input
                    .in_set(UiInputSet::Capture)
                    .before(UpdateSet::Camera),
            );
    }
}

fn begin_frame(
    regions: Res<UiInputRegions>,
    focus: Res<InputFocus>,
    editors: Query<(), Editors>,
    mut capture: ResMut<UiInputCapture>,
) {
    capture.pointer = regions.modal;
    capture.keyboard = regions.modal || focus.get().is_some_and(|e| editors.contains(e));
}

#[derive(SystemParam)]
struct Inputs<'w, 's> {
    windows: Query<'w, 's, (Entity, &'static Window), With<PrimaryWindow>>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    events: MessageReader<'w, 's, WindowEvent>,
    editors: Query<'w, 's, (), Editors>,
}
use bevy::ecs::system::SystemParam;

fn capture_input(
    regions: Res<UiInputRegions>,
    mut capture: ResMut<UiInputCapture>,
    mut focus: ResMut<InputFocus>,
    mut inputs: Inputs,
    mut cursor: Local<Option<Vec2>>,
) {
    let Ok((window_entity, window)) = inputs.windows.single() else {
        capture.block_modal_frame();
        inputs.events.clear();
        return;
    };
    // Replay ordered window events so a toolbar click followed by a move into
    // the canvas cannot become a board click in the same frame.
    for event in inputs.events.read() {
        match event {
            WindowEvent::CursorMoved(event) if event.window == window_entity => {
                *cursor = Some(event.position)
            }
            WindowEvent::CursorLeft(event) if event.window == window_entity => *cursor = None,
            WindowEvent::MouseButtonInput(event)
                if event.window == window_entity && event.state == ButtonState::Pressed =>
            {
                capture.pointer |= cursor
                    .or(window.cursor_position())
                    .is_some_and(|p| regions.contains(p));
            }
            _ => {}
        }
    }
    capture.pointer |= window.cursor_position().is_none_or(|p| regions.contains(p));
    let editing = focus.get().is_some_and(|e| inputs.editors.contains(e));
    capture.keyboard |= regions.modal || editing;
    if !regions.modal
        && ((editing && inputs.keys.just_pressed(KeyCode::Escape))
            || (inputs
                .mouse
                .any_just_pressed([MouseButton::Left, MouseButton::Middle])
                && !capture.pointer))
    {
        focus.clear();
    }
}

/// Whether keyboard focus rests on the board rather than on a control.
#[derive(SystemParam)]
pub struct KeyboardFocus<'w, 's> {
    focus: Res<'w, InputFocus>,
    boards: Query<'w, 's, (), BoardTargets>,
}

impl KeyboardFocus<'_, '_> {
    /// Bevy focuses the window at startup; that is the board, not a control.
    pub fn on_board(&self) -> bool {
        self.focus.get().is_none_or(|e| self.boards.contains(e))
    }
}

pub fn canvas_keyboard_available(
    capture: Res<UiInputCapture>,
    keyboard: KeyboardFocus,
    keys: Res<ButtonInput<KeyCode>>,
) -> bool {
    !capture.keyboard
        && (keyboard.on_board()
            || !(keys.just_pressed(KeyCode::Space) || keys.just_pressed(KeyCode::Enter)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        ecs::system::RunSystemOnce, input::mouse::MouseButtonInput, input_focus::FocusCause,
        window::CursorMoved,
    };

    fn world() -> (World, Entity) {
        let mut world = World::new();
        world.init_resource::<InputFocus>();
        world.init_resource::<UiInputCapture>();
        world.init_resource::<ButtonInput<MouseButton>>();
        world.init_resource::<ButtonInput<KeyCode>>();
        world.init_resource::<Messages<WindowEvent>>();
        world.insert_resource(UiInputRegions {
            panels: vec![Rect::new(0.0, 0.0, 800.0, 112.0)],
            modal: false,
        });
        let mut window = Window::default();
        window.set_cursor_position(Some(Vec2::new(500.0, 300.0)));
        let window = world.spawn((window, PrimaryWindow)).id();
        (world, window)
    }

    #[test]
    fn toolbar_press_then_move_into_canvas_is_captured_for_that_frame() {
        let (mut world, window) = world();
        let mut messages = world.resource_mut::<Messages<WindowEvent>>();
        messages.write(WindowEvent::CursorMoved(CursorMoved {
            window,
            position: Vec2::new(50.0, 50.0),
            delta: None,
        }));
        messages.write(WindowEvent::MouseButtonInput(MouseButtonInput {
            window,
            button: MouseButton::Left,
            state: ButtonState::Pressed,
        }));
        messages.write(WindowEvent::CursorMoved(CursorMoved {
            window,
            position: Vec2::new(500.0, 300.0),
            delta: None,
        }));
        world.run_system_once(capture_input).unwrap();
        assert!(world.resource::<UiInputCapture>().pointer);
    }

    #[test]
    fn panel_background_and_modal_closing_frame_block_canvas_input() {
        let (mut world, window) = world();
        world
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(300.0, 40.0)));
        world.run_system_once(capture_input).unwrap();
        assert!(world.resource::<UiInputCapture>().pointer);
        world.resource_mut::<UiInputRegions>().modal = true;
        world.run_system_once(begin_frame).unwrap();
        world.resource_mut::<UiInputRegions>().modal = false;
        world
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(500.0, 300.0)));
        world.run_system_once(capture_input).unwrap();
        assert!(world.resource::<UiInputCapture>().pointer);
        assert!(!world.run_system_once(canvas_keyboard_available).unwrap());
        world.run_system_once(begin_frame).unwrap();
        world.run_system_once(capture_input).unwrap();
        assert!(!world.resource::<UiInputCapture>().pointer);
        assert!(world.run_system_once(canvas_keyboard_available).unwrap());
    }

    #[test]
    fn space_reaches_board_while_window_or_board_has_focus() {
        let (mut world, window) = world();
        let board = world.spawn(BoardFocus).id();
        let button = world.spawn_empty().id();
        world
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Space);
        for entity in [window, board] {
            world
                .resource_mut::<InputFocus>()
                .set(entity, FocusCause::Navigated);
            assert!(world.run_system_once(canvas_keyboard_available).unwrap());
        }
        world
            .resource_mut::<InputFocus>()
            .set(button, FocusCause::Navigated);
        assert!(!world.run_system_once(canvas_keyboard_available).unwrap());
    }

    #[test]
    fn editors_block_shortcuts_and_escape_releases_focus_without_quitting() {
        let (mut world, _) = world();
        let field = world.spawn(EditableText::new("")).id();
        let slider = world.spawn(Slider::default()).id();
        for entity in [field, slider] {
            world
                .resource_mut::<InputFocus>()
                .set(entity, FocusCause::Navigated);
            world.run_system_once(begin_frame).unwrap();
            world.run_system_once(capture_input).unwrap();
            assert!(!world.run_system_once(canvas_keyboard_available).unwrap());
        }
        world
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        world.run_system_once(capture_input).unwrap();
        assert!(world.resource::<InputFocus>().get().is_none());
        assert!(!world.run_system_once(canvas_keyboard_available).unwrap());
        world.resource_mut::<ButtonInput<KeyCode>>().reset_all();
        world.run_system_once(begin_frame).unwrap();
        world.run_system_once(capture_input).unwrap();
        assert!(world.run_system_once(canvas_keyboard_available).unwrap());
    }
}
