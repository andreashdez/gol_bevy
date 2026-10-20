use crate::{
    app::EditTool,
    canvas_view::{CanvasView, cursor_world},
    shared::{
        BINDING_CLEAR, BINDING_CLEAR_STAMP, BINDING_FLIP_STAMP, BINDING_PAN_DOWN, BINDING_PAN_LEFT,
        BINDING_PAN_RIGHT, BINDING_PAN_UP, BINDING_QUIT, BINDING_RANDOMIZE, BINDING_RESET_CAMERA,
        BINDING_RESET_EPOCH, BINDING_ROTATE_STAMP, BINDING_RUN_PAUSE, BINDING_SLOW_DOWN,
        BINDING_SPEED_UP, BINDING_STEP, BINDING_TOGGLE_BOUNDARY_MODE, BINDING_TOGGLE_CONTROL_DECK,
        BINDING_TOGGLE_DIAGNOSTICS, BINDING_TOGGLE_GRID, BINDING_TOGGLE_HELP, CAMERA_PAN_SPEED,
        CAMERA_ZOOM_MAX, CAMERA_ZOOM_MIN, CAMERA_ZOOM_STEP, DiagnosticsOverlay, EditFeedback,
        GridOverlay, MainCamera, Simulation, TypedKeys, UiLayout, UpdateSet, collect_typed_keys,
        world_to_cell,
    },
    ui_input::{UiInputCapture, canvas_keyboard_available},
};
use bevy::{input::mouse::MouseWheel, prelude::*, window::PrimaryWindow};
pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        // Capture is refreshed before camera controls and board input. Mouse mapping
        // waits for the current frame's camera projection and transforms.
        app.configure_sets(
            PostUpdate,
            UpdateSet::Input
                .after(bevy::camera::CameraUpdateSystems)
                .after(bevy::transform::TransformSystems::Propagate),
        )
        .add_systems(
            PostUpdate,
            finish_mouse_stroke
                .in_set(UpdateSet::Input)
                .before(handle_mouse),
        )
        .add_systems(
            PostUpdate,
            handle_keyboard
                .run_if(canvas_keyboard_available)
                .in_set(UpdateSet::Input),
        )
        .add_systems(PostUpdate, handle_mouse.in_set(UpdateSet::Input))
        .add_systems(Update, handle_camera_zoom.in_set(UpdateSet::Camera))
        .add_systems(Update, handle_camera_drag.in_set(UpdateSet::Camera))
        .add_systems(
            Update,
            handle_camera_pan
                .run_if(canvas_keyboard_available)
                .in_set(UpdateSet::Camera),
        )
        .add_systems(
            Update,
            handle_camera_reset
                .run_if(canvas_keyboard_available)
                .in_set(UpdateSet::Camera),
        )
        .init_resource::<TypedKeys>()
        .add_systems(
            PreUpdate,
            collect_typed_keys.after(bevy::input::InputSystems),
        );
    }
}

fn handle_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    typed: Res<TypedKeys>,
    mut sim: ResMut<Simulation>,
    mut diagnostics: ResMut<DiagnosticsOverlay>,
    mut grid_overlay: ResMut<GridOverlay>,
    mut ui_layout: ResMut<UiLayout>,
    mut feedback: ResMut<EditFeedback>,
) {
    let command_keys = [
        KeyCode::SuperLeft,
        KeyCode::SuperRight,
        KeyCode::ControlLeft,
        KeyCode::ControlRight,
    ];
    // Fast chords can press and release the modifier within one frame.
    let command = keys.any_pressed(command_keys)
        || keys.any_just_pressed(command_keys)
        || keys.any_just_released(command_keys);
    if typed.just_typed("u") {
        feedback.undo(&mut sim.app);
        return;
    }
    if command {
        if typed.just_typed("z") {
            let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
            if shift {
                feedback.redo(&mut sim.app);
            } else {
                feedback.undo(&mut sim.app);
            }
        }
        return;
    }
    if BINDING_RUN_PAUSE.just_pressed(&keys, &typed) {
        sim.app.toggle_running();
    }
    if BINDING_STEP.just_pressed(&keys, &typed) {
        sim.app.increment_epoch();
    }
    if BINDING_RANDOMIZE.just_pressed(&keys, &typed) {
        feedback.randomize(&mut sim.app);
    }
    if BINDING_CLEAR.just_pressed(&keys, &typed) {
        feedback.clear(&mut sim.app);
    }
    if BINDING_RESET_EPOCH.just_pressed(&keys, &typed) {
        sim.app.reset_epoch();
    }
    if BINDING_SPEED_UP.just_pressed(&keys, &typed) {
        sim.app.speed_up();
    }
    if BINDING_SLOW_DOWN.just_pressed(&keys, &typed) {
        sim.app.slow_down();
    }
    if BINDING_TOGGLE_HELP.just_pressed(&keys, &typed) {
        sim.app.toggle_help();
    }
    if BINDING_CLEAR_STAMP.just_pressed(&keys, &typed) {
        sim.app.clear_selected_pattern();
    }
    if BINDING_ROTATE_STAMP.just_pressed(&keys, &typed) {
        sim.app.rotate_stamp_clockwise();
    }
    if BINDING_FLIP_STAMP.just_pressed(&keys, &typed) {
        sim.app.toggle_stamp_flip();
    }
    if BINDING_TOGGLE_DIAGNOSTICS.just_pressed(&keys, &typed) {
        diagnostics.enabled = !diagnostics.enabled;
    }
    if BINDING_TOGGLE_GRID.just_pressed(&keys, &typed) {
        grid_overlay.enabled = !grid_overlay.enabled;
    }
    if BINDING_TOGGLE_BOUNDARY_MODE.just_pressed(&keys, &typed) {
        sim.app.toggle_boundary_mode();
    }
    if BINDING_TOGGLE_CONTROL_DECK.just_pressed(&keys, &typed) {
        ui_layout.toggle_control_deck();
    }
    if BINDING_QUIT.just_pressed(&keys, &typed) {
        sim.app.quit();
    }
}

fn finish_mouse_stroke(mouse_buttons: Res<ButtonInput<MouseButton>>, mut sim: ResMut<Simulation>) {
    // Always end strokes, including releases over UI widgets or outside the board.
    if !mouse_buttons.pressed(MouseButton::Left) && !mouse_buttons.pressed(MouseButton::Right) {
        sim.app.end_stroke();
    }
}

fn handle_mouse(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    window_q: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut sim: ResMut<Simulation>,
    mut canvas_drag: Local<(bool, bool)>,
    pointer_capture: Res<UiInputCapture>,
    mut feedback: ResMut<EditFeedback>,
) {
    if mouse_buttons.pressed(MouseButton::Middle) {
        *canvas_drag = (false, false);
        sim.app.end_stroke();
        return;
    }
    if !mouse_buttons.pressed(MouseButton::Left) {
        canvas_drag.0 = false;
    }
    if !mouse_buttons.pressed(MouseButton::Right) {
        canvas_drag.1 = false;
    }
    if pointer_capture.pointer {
        return;
    }
    let Ok(window) = window_q.single() else {
        return;
    };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };
    let Ok((camera, camera_transform)) = camera_q.single() else {
        return;
    };
    let Some(world_pos) = cursor_world(camera, camera_transform, cursor_pos) else {
        return;
    };

    let (cols, rows) = sim.app.board.size();
    let Some((col, row)) = world_to_cell(world_pos, cols, rows) else {
        return;
    };

    if mouse_buttons.just_pressed(MouseButton::Left) {
        canvas_drag.0 = true;
        feedback.remaining = 0.0;
    }
    if mouse_buttons.just_pressed(MouseButton::Right) {
        canvas_drag.1 = true;
        feedback.remaining = 0.0;
    }

    if mouse_buttons.just_pressed(MouseButton::Left) && sim.app.has_selected_pattern() {
        sim.app.place_selected_pattern_at(col, row);
    } else if canvas_drag.0 && !sim.app.has_selected_pattern() {
        sim.app.begin_stroke();
        match sim.app.tool() {
            EditTool::Erase => sim.app.depopulate_board(col, row),
            _ => sim.app.populate_board(col, row),
        }
    }

    if canvas_drag.1 {
        sim.app.begin_stroke();
        sim.app.depopulate_board(col, row);
    }
}

fn handle_camera_zoom(
    mut wheel_events: MessageReader<MouseWheel>,
    mut camera_q: Query<&mut Projection, With<MainCamera>>,
    capture: Res<UiInputCapture>,
) {
    if capture.pointer {
        wheel_events.clear();
        return;
    }
    let Ok(mut projection) = camera_q.single_mut() else {
        return;
    };

    let mut total_scroll = 0.0;
    for event in wheel_events.read() {
        total_scroll += event.y;
    }
    if total_scroll == 0.0 {
        return;
    }

    let Projection::Orthographic(ref mut ortho) = *projection else {
        return;
    };
    let factor = (1.0 - total_scroll * CAMERA_ZOOM_STEP).clamp(0.5, 2.0);
    ortho.scale = (ortho.scale * factor).clamp(CAMERA_ZOOM_MIN, CAMERA_ZOOM_MAX);
}

fn handle_camera_pan(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut camera_q: Query<(&mut Transform, &Projection), With<MainCamera>>,
) {
    let Ok((mut camera_transform, projection)) = camera_q.single_mut() else {
        return;
    };

    let mut direction = Vec3::ZERO;
    if BINDING_PAN_LEFT.pressed(&keys) {
        direction.x -= 1.0;
    }
    if BINDING_PAN_RIGHT.pressed(&keys) {
        direction.x += 1.0;
    }
    if BINDING_PAN_UP.pressed(&keys) {
        direction.y += 1.0;
    }
    if BINDING_PAN_DOWN.pressed(&keys) {
        direction.y -= 1.0;
    }
    if direction == Vec3::ZERO {
        return;
    }

    let zoom = match projection {
        Projection::Orthographic(ortho) => ortho.scale,
        _ => 1.0,
    };
    let travel = direction.normalize() * CAMERA_PAN_SPEED * zoom * time.delta_secs();
    camera_transform.translation += travel;
}

fn handle_camera_drag(
    buttons: Res<ButtonInput<MouseButton>>,
    capture: Res<UiInputCapture>,
    windows: Query<&Window, With<PrimaryWindow>>,
    layout: Res<UiLayout>,
    mut cameras: Query<(&mut Transform, &Projection), With<MainCamera>>,
    mut previous: Local<Option<Vec2>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let cursor = window.cursor_position().filter(|&position| {
        window.focused
            && !capture.pointer
            && buttons.pressed(MouseButton::Middle)
            && crate::native_ui_plugin::canvas_rect(window.size(), &layout).contains(position)
    });
    let Some(cursor) = cursor else {
        *previous = None;
        return;
    };
    if buttons.just_pressed(MouseButton::Middle) {
        *previous = Some(cursor);
    }
    if let Some(last) = *previous {
        if let Ok((mut transform, Projection::Orthographic(ortho))) = cameras.single_mut() {
            let delta = cursor - last;
            transform.translation += Vec3::new(-delta.x, delta.y, 0.0) * ortho.scale;
        }
        *previous = Some(cursor);
    }
}

fn handle_camera_reset(
    view: Res<CanvasView>,
    keys: Res<ButtonInput<KeyCode>>,
    typed: Res<TypedKeys>,
    mut camera_q: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
) {
    if !BINDING_RESET_CAMERA.just_pressed(&keys, &typed) {
        return;
    }

    if let Ok((mut transform, mut projection)) = camera_q.single_mut() {
        view.fit_camera(&mut transform, &mut projection);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn middle_drag_moves_in_world_units_and_cannot_start_or_resume_over_ui() {
        let mut world = World::new();
        world.init_resource::<ButtonInput<MouseButton>>();
        world.init_resource::<UiInputCapture>();
        world.init_resource::<UiLayout>();
        let mut window = Window {
            focused: true,
            ..default()
        };
        window.set_cursor_position(Some(Vec2::new(500.0, 200.0)));
        let window = world.spawn((window, PrimaryWindow)).id();
        let camera = world.spawn((Camera2d, MainCamera)).id();
        if let Projection::Orthographic(ortho) = &mut *world.get_mut::<Projection>(camera).unwrap()
        {
            ortho.scale = 2.0;
        }
        let drag = world.register_system(handle_camera_drag);
        world
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        world.run_system(drag).unwrap();
        world.resource_mut::<ButtonInput<MouseButton>>().clear();
        world
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(520.0, 210.0)));
        world.run_system(drag).unwrap();
        let moved = world.get::<Transform>(camera).unwrap().translation;
        assert_eq!(moved.truncate(), Vec2::new(-40.0, 20.0));

        world.resource_mut::<UiInputCapture>().pointer = true;
        world.run_system(drag).unwrap();
        world.resource_mut::<UiInputCapture>().pointer = false;
        world
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(600.0, 300.0)));
        world.run_system(drag).unwrap();
        assert_eq!(world.get::<Transform>(camera).unwrap().translation, moved);

        world.resource_mut::<ButtonInput<MouseButton>>().reset_all();
        world
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Middle);
        world
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(500.0, 50.0)));
        world.run_system(drag).unwrap();
        world.resource_mut::<ButtonInput<MouseButton>>().clear();
        world
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(Vec2::new(600.0, 300.0)));
        world.run_system(drag).unwrap();
        assert_eq!(world.get::<Transform>(camera).unwrap().translation, moved);
    }

    #[test]
    fn middle_drag_never_draws_even_with_the_left_button_pressed() {
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
        let mut world = World::new();
        let mut buttons = ButtonInput::<MouseButton>::default();
        buttons.press(MouseButton::Middle);
        buttons.press(MouseButton::Left);
        world.insert_resource(buttons);
        world.init_resource::<UiInputCapture>();
        world.init_resource::<EditFeedback>();
        world.insert_resource(Simulation::new(crate::app::App::new(8, 8)));
        let mut window = Window::default();
        window.resolution.set(800.0, 600.0);
        window.set_cursor_position(Some(Vec2::new(400.0, 300.0)));
        world.spawn((window, PrimaryWindow));
        world.spawn((
            MainCamera,
            GlobalTransform::default(),
            Camera {
                computed: ComputedCameraValues {
                    target_info: Some(RenderTargetInfo {
                        physical_size: UVec2::new(800, 600),
                        scale_factor: 1.0,
                    }),
                    clip_from_view: Mat4::IDENTITY,
                    ..default()
                },
                ..default()
            },
        ));
        world.run_system_once(handle_mouse).unwrap();
        assert_eq!(
            world.resource::<Simulation>().app.board.live_cells_count(),
            0
        );
        // The same valid cursor and left press do draw without the pan button.
        world
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Middle);
        world.run_system_once(handle_mouse).unwrap();
        assert_eq!(
            world.resource::<Simulation>().app.board.live_cells_count(),
            1
        );
    }

    #[test]
    fn fast_undo_chord_does_not_rotate_stamp() {
        for modifier in [Some(KeyCode::SuperLeft), Some(KeyCode::ControlLeft), None] {
            let mut world = World::new();
            let mut app = crate::app::App::new(8, 8);
            app.populate_board(3, 3);
            app.select_pattern(crate::app::PatternKind::Glider);
            world.insert_resource(Simulation::new(app));
            let mut keys = ButtonInput::<KeyCode>::default();
            let mut typed = TypedKeys::default();
            if let Some(modifier) = modifier {
                keys.press(modifier);
                keys.press(KeyCode::KeyZ);
                keys.release(KeyCode::KeyZ);
                keys.release(modifier);
                typed.press("z");
            } else {
                keys.press(KeyCode::KeyU);
                keys.release(KeyCode::KeyU);
                typed.press("u");
            }
            world.insert_resource(keys);
            world.insert_resource(typed);
            world.insert_resource(DiagnosticsOverlay { enabled: false });
            world.insert_resource(GridOverlay { enabled: false });
            world.insert_resource(UiLayout::default());
            world.init_resource::<EditFeedback>();
            world.run_system_once(handle_keyboard).unwrap();
            let app = &world.resource::<Simulation>().app;
            assert_eq!(app.board.live_cells_count(), 0);
            assert_eq!(app.selected_pattern_status_label(), "Glider");
        }
    }

    #[test]
    fn character_shortcuts_follow_the_keyboard_layout() {
        use bevy::input::{
            ButtonState,
            keyboard::{Key, KeyboardInput},
        };
        let mut app = App::new();
        app.add_plugins(bevy::input::InputPlugin)
            .init_resource::<TypedKeys>()
            .add_systems(
                PreUpdate,
                collect_typed_keys.after(bevy::input::InputSystems),
            );
        let window = app.world_mut().spawn(Window::default()).id();
        let press = |app: &mut App, key_code, character: &str| {
            for state in [ButtonState::Pressed, ButtonState::Released] {
                app.world_mut().write_message(KeyboardInput {
                    key_code,
                    logical_key: Key::Character(character.into()),
                    state,
                    text: None,
                    repeat: false,
                    window,
                });
            }
            app.world_mut().run_schedule(PreUpdate);
        };
        let mut sim = crate::app::App::new(8, 8);
        sim.select_pattern(crate::app::PatternKind::Glider);
        app.insert_resource(Simulation::new(sim))
            .insert_resource(DiagnosticsOverlay { enabled: false })
            .insert_resource(GridOverlay { enabled: false })
            .insert_resource(UiLayout::default())
            .init_resource::<EditFeedback>();
        let help = |app: &App| app.world().resource::<Simulation>().app.is_help_visible();

        // German layout: `/` is Shift+7 and the key US layouts use for `/` types `-`.
        press(&mut app, KeyCode::Slash, "-");
        app.world_mut().run_system_once(handle_keyboard).unwrap();
        assert!(!help(&app));
        press(&mut app, KeyCode::Digit7, "/");
        app.world_mut().run_system_once(handle_keyboard).unwrap();
        assert!(help(&app));

        // QWERTZ swaps Z and Y; the key labelled Z rotates, even with Shift held.
        let rotation = |app: &App| app.world().resource::<Simulation>().app.stamp_orientation();
        let before = rotation(&app);
        press(&mut app, KeyCode::KeyZ, "y");
        app.world_mut().run_system_once(handle_keyboard).unwrap();
        assert_eq!(rotation(&app), before);
        press(&mut app, KeyCode::KeyY, "Z");
        app.world_mut().run_system_once(handle_keyboard).unwrap();
        assert_ne!(rotation(&app), before);
    }
}
