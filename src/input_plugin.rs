use crate::{
    app::EditTool,
    canvas_view::{CanvasView, cursor_world},
    shared::{
        BINDING_CLEAR, BINDING_CLEAR_STAMP, BINDING_DESELECT, BINDING_ERASE_SELECTION,
        BINDING_FLIP_STAMP, BINDING_PAN_DOWN, BINDING_PAN_LEFT, BINDING_PAN_RIGHT, BINDING_PAN_UP,
        BINDING_QUIT, BINDING_RANDOMIZE, BINDING_RESET_CAMERA, BINDING_RESET_EPOCH,
        BINDING_RESET_TO_START, BINDING_ROTATE_STAMP, BINDING_RUN_PAUSE, BINDING_SLOW_DOWN,
        BINDING_SPEED_UP, BINDING_STEP, BINDING_TOGGLE_BOUNDARY_MODE, BINDING_TOGGLE_DIAGNOSTICS,
        BINDING_TOGGLE_GRID, BINDING_TOGGLE_HELP, BINDING_TOGGLE_PALETTE, CAMERA_PAN_SPEED,
        CAMERA_ZOOM_MIN, CAMERA_ZOOM_STEP, DiagnosticsOverlay, EditFeedback, GridOverlay,
        MainCamera, Simulation, TOOL_SHORTCUTS, TypedKeys, UpdateSet, collect_typed_keys,
    },
    ui_input::{UiInputCapture, canvas_keyboard_available},
};
use bevy::{clipboard::Clipboard, input::mouse::MouseWheel, prelude::*, window::PrimaryWindow};
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

/// Cmd on macOS and Ctrl elsewhere start app shortcuts like undo and save.
pub const COMMAND_KEYS: [KeyCode; 4] = [
    KeyCode::SuperLeft,
    KeyCode::SuperRight,
    KeyCode::ControlLeft,
    KeyCode::ControlRight,
];

fn handle_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    typed: Res<TypedKeys>,
    mut sim: ResMut<Simulation>,
    mut diagnostics: ResMut<DiagnosticsOverlay>,
    mut grid_overlay: ResMut<GridOverlay>,
    mut feedback: ResMut<EditFeedback>,
    // Absent in tests, which must not touch the system clipboard.
    clipboard: Option<ResMut<Clipboard>>,
) {
    // Fast chords can press and release the modifier within one frame.
    let command = keys.any_pressed(COMMAND_KEYS)
        || keys.any_just_pressed(COMMAND_KEYS)
        || keys.any_just_released(COMMAND_KEYS);
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
        } else if typed.just_typed("a") {
            feedback.select_live_cells(&mut sim.app);
        } else if typed.just_typed("c") {
            feedback.copy_selection(&mut sim.app, clipboard, false);
        } else if typed.just_typed("x") {
            feedback.copy_selection(&mut sim.app, clipboard, true);
        } else if typed.just_typed("v") {
            feedback.paste(&mut sim.app, clipboard);
        }
        return;
    }
    if BINDING_ERASE_SELECTION.just_pressed(&keys, &typed) {
        feedback.erase_selection(&mut sim.app);
    }
    if BINDING_DESELECT.just_pressed(&keys, &typed) {
        sim.app.clear_selection();
    }
    if BINDING_RUN_PAUSE.just_pressed(&keys, &typed) {
        sim.app.toggle_running();
    }
    if BINDING_STEP.just_pressed(&keys, &typed) {
        sim.app.increment_epoch();
    }
    if BINDING_RESET_TO_START.just_pressed(&keys, &typed) {
        feedback.reset_to_start(&mut sim.app);
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
    for (binding, tool) in TOOL_SHORTCUTS {
        if binding.just_pressed(&keys, &typed) {
            sim.app.set_tool(tool);
        }
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
    if BINDING_TOGGLE_PALETTE.just_pressed(&keys, &typed) {
        sim.app.toggle_palette();
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

/// Mouse drags that began on the canvas.
#[derive(Default)]
struct CanvasDrag {
    left: bool,
    right: bool,
    /// The cell a Select drag started on, until the button is released.
    select_anchor: Option<(i64, i64)>,
}

fn handle_mouse(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    window_q: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut sim: ResMut<Simulation>,
    mut canvas_drag: Local<CanvasDrag>,
    pointer_capture: Res<UiInputCapture>,
    mut feedback: ResMut<EditFeedback>,
) {
    if mouse_buttons.pressed(MouseButton::Middle) {
        *canvas_drag = CanvasDrag::default();
        sim.app.end_stroke();
        return;
    }
    if !mouse_buttons.pressed(MouseButton::Left) {
        canvas_drag.left = false;
        canvas_drag.select_anchor = None;
    }
    if !mouse_buttons.pressed(MouseButton::Right) {
        canvas_drag.right = false;
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

    // A selection follows the pointer past a board's edge, stopping at it.
    if let Some(anchor) = canvas_drag.select_anchor
        && mouse_buttons.pressed(MouseButton::Left)
    {
        let corner = sim.cell_map().nearest_cell(world_pos);
        // A click without a drag selects nothing.
        if corner != anchor || sim.app.selection().is_some() {
            if sim.app.selection().is_none() {
                sim.app.begin_selection(anchor.0, anchor.1);
            }
            sim.app.extend_selection(corner.0, corner.1);
        }
    }

    let Some((col, row)) = sim.cell_map().world_to_cell(world_pos) else {
        return;
    };

    if mouse_buttons.just_pressed(MouseButton::Left) {
        canvas_drag.left = true;
        feedback.remaining = 0.0;
    }
    if mouse_buttons.just_pressed(MouseButton::Right) {
        canvas_drag.right = true;
        feedback.remaining = 0.0;
    }

    if sim.app.tool() == EditTool::Select {
        if mouse_buttons.just_pressed(MouseButton::Left) {
            sim.app.clear_selection();
            canvas_drag.select_anchor = Some((col, row));
        }
    } else if mouse_buttons.just_pressed(MouseButton::Left) && sim.app.has_selected_pattern() {
        sim.app.place_selected_pattern_at(col, row);
    } else if canvas_drag.left && !sim.app.has_selected_pattern() {
        sim.app.begin_stroke();
        match sim.app.tool() {
            EditTool::Erase => sim.app.depopulate_board(col, row),
            _ => sim.app.populate_board(col, row),
        }
    }

    if canvas_drag.right {
        sim.app.begin_stroke();
        sim.app.depopulate_board(col, row);
    }
}

fn handle_camera_zoom(
    mut wheel_events: MessageReader<MouseWheel>,
    mut camera_q: Query<&mut Projection, With<MainCamera>>,
    capture: Res<UiInputCapture>,
    sim: Res<Simulation>,
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
    ortho.scale = (ortho.scale * factor).clamp(CAMERA_ZOOM_MIN, sim.zoom_max());
}

fn handle_camera_pan(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut camera_q: Query<(&mut Transform, &Projection), With<MainCamera>>,
) {
    let Ok((mut camera_transform, projection)) = camera_q.single_mut() else {
        return;
    };
    // S is a pan key, but Cmd/Ctrl+S saves.
    if keys.any_pressed(COMMAND_KEYS) {
        return;
    }

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
            && crate::native_ui_plugin::canvas_rect(window.size()).contains(position)
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
    mut sim: ResMut<Simulation>,
    mut camera_q: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
) {
    if !BINDING_RESET_CAMERA.just_pressed(&keys, &typed) {
        return;
    }

    if let Ok((mut transform, mut projection)) = camera_q.single_mut() {
        view.fit_camera(&mut sim, &mut transform, &mut projection);
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

    /// A click at the world origin, on a plane whose origin is at `origin`.
    fn click_on_plane(origin: (i64, i64), button: MouseButton) -> crate::app::App {
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
        let mut world = World::new();
        let mut buttons = ButtonInput::<MouseButton>::default();
        buttons.press(button);
        world.insert_resource(buttons);
        world.init_resource::<UiInputCapture>();
        world.init_resource::<EditFeedback>();
        let mut app = crate::app::App::new(8, 8);
        app.set_unbounded().unwrap();
        app.board.populate(origin.0, origin.1);
        let mut sim = Simulation::new(app);
        sim.plane_origin = origin;
        world.insert_resource(sim);
        let mut window = Window::default();
        window.resolution.set(800.0, 600.0);
        window.set_cursor_position(Some(Vec2::new(400.5, 299.5)));
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
        world.remove_resource::<Simulation>().unwrap().app
    }

    #[test]
    fn the_mouse_edits_the_plane_at_its_origin_however_far_out_that_is() {
        let origin = (-(1 << 40), 1 << 41);
        let app = click_on_plane((origin.0 + 3, origin.1), MouseButton::Left);
        assert!(app.board.is_alive(origin.0 + 3, origin.1));
        assert_eq!(
            app.board.live_cells_count(),
            1,
            "that cell was alive already"
        );

        // The origin's cell is what the click lands on, so it can be erased too.
        let app = click_on_plane(origin, MouseButton::Right);
        assert_eq!(app.board.live_cells_count(), 0);
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

        // Digits pick tools, by the digit typed or by the key's position, since
        // AZERTY layouts type `"` on the unshifted 3 key.
        let tool = |app: &App| app.world().resource::<Simulation>().app.tool();
        press(&mut app, KeyCode::Digit2, "2");
        app.world_mut().run_system_once(handle_keyboard).unwrap();
        assert_eq!(tool(&app), crate::app::EditTool::Erase);
        press(&mut app, KeyCode::Digit3, "\"");
        app.world_mut().run_system_once(handle_keyboard).unwrap();
        assert_eq!(tool(&app), crate::app::EditTool::Stamp);
        press(&mut app, KeyCode::Numpad1, "1");
        app.world_mut().run_system_once(handle_keyboard).unwrap();
        assert_eq!(tool(&app), crate::app::EditTool::Draw);
    }

    /// An 8 × 8 board filling an 800 × 600 window one cell to 12 pixels, with
    /// the Select tool picked.
    fn selection_world() -> (World, Entity) {
        use bevy::camera::{ComputedCameraValues, RenderTargetInfo};
        let mut world = World::new();
        world.init_resource::<ButtonInput<MouseButton>>();
        world.init_resource::<UiInputCapture>();
        world.init_resource::<EditFeedback>();
        let mut app = crate::app::App::new(8, 8);
        app.set_tool(EditTool::Select);
        world.insert_resource(Simulation::new(app));
        let mut window = Window::default();
        window.resolution.set(800.0, 600.0);
        let window = world.spawn((window, PrimaryWindow)).id();
        world.spawn((
            MainCamera,
            GlobalTransform::default(),
            Camera {
                computed: ComputedCameraValues {
                    target_info: Some(RenderTargetInfo {
                        physical_size: UVec2::new(800, 600),
                        scale_factor: 1.0,
                    }),
                    // Orthographic, one world unit to a pixel, centred.
                    clip_from_view: Mat4::from_scale(Vec3::new(1.0 / 400.0, 1.0 / 300.0, 1.0)),
                    ..default()
                },
                ..default()
            },
        ));
        (world, window)
    }

    /// Moves the pointer to the middle of a cell, which may lie off the board.
    fn point_at(world: &mut World, window: Entity, (col, row): (f32, f32)) {
        // The board's lower left corner is at (-48, -48) in the world, which
        // the window shows 400 px right of its left edge and 300 px down.
        let position = Vec2::new(352.0 + (col + 0.5) * 12.0, 348.0 - (row + 0.5) * 12.0);
        world
            .get_mut::<Window>(window)
            .unwrap()
            .set_cursor_position(Some(position));
    }

    #[test]
    fn dragging_with_the_select_tool_selects_up_to_the_board_edge() {
        let (mut world, window) = selection_world();
        let mouse = world.register_system(handle_mouse);
        let selection = |world: &World| world.resource::<Simulation>().app.selection();

        point_at(&mut world, window, (2.0, 3.0));
        world
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        world.run_system(mouse).unwrap();
        world.resource_mut::<ButtonInput<MouseButton>>().clear();
        assert_eq!(selection(&world), None);

        // Past the board's upper right, the selection stops at its corner.
        point_at(&mut world, window, (20.0, 20.0));
        world.run_system(mouse).unwrap();
        let expected = crate::sparse_board::CellRect {
            left: 2,
            bottom: 3,
            width: 6,
            height: 5,
        };
        assert_eq!(selection(&world), Some(expected));
        world
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        world.run_system(mouse).unwrap();
        world.resource_mut::<ButtonInput<MouseButton>>().clear();
        assert_eq!(selection(&world), Some(expected));
        assert_eq!(
            world.resource::<Simulation>().app.board.live_cells_count(),
            0,
            "selecting draws nothing"
        );

        // A click without a drag lets go of it.
        point_at(&mut world, window, (1.0, 1.0));
        world
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        world.run_system(mouse).unwrap();
        world
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        world.run_system(mouse).unwrap();
        assert_eq!(selection(&world), None);
    }

    #[test]
    fn selection_keys_clear_cells_and_deselect() {
        let run = |app: crate::app::App, key: Option<KeyCode>, typed_key: Option<&str>| {
            let mut world = World::new();
            world.insert_resource(Simulation::new(app));
            let mut keys = ButtonInput::<KeyCode>::default();
            let mut typed = TypedKeys::default();
            if let Some(key) = key {
                keys.press(key);
            }
            if let Some(character) = typed_key {
                keys.press(KeyCode::ControlLeft);
                typed.press(character);
            }
            world.insert_resource(keys);
            world.insert_resource(typed);
            world.insert_resource(DiagnosticsOverlay { enabled: false });
            world.insert_resource(GridOverlay { enabled: false });
            world.init_resource::<EditFeedback>();
            world.run_system_once(handle_keyboard).unwrap();
            let message = world.resource::<EditFeedback>().message.to_string();
            (world.remove_resource::<Simulation>().unwrap().app, message)
        };
        let mut app = crate::app::App::new(8, 8);
        app.populate_board(1, 1);
        app.populate_board(5, 5);

        let (selected, message) = run(app, None, Some("a"));
        assert_eq!(selected.tool(), EditTool::Select);
        assert_eq!(message, "Selected 5 × 5 cells");

        let (mut app, message) = run(selected, Some(KeyCode::Escape), None);
        assert_eq!(app.selection(), None);
        assert_eq!(message, "");

        app.begin_selection(0, 0);
        app.extend_selection(2, 2);
        let (app, message) = run(app, Some(KeyCode::Backspace), None);
        assert_eq!(app.board.live_cells_count(), 1);
        assert!(app.board.is_alive(5, 5));
        assert_eq!(message, "Cleared 1 cells · U to undo");

        // Without a clipboard, cutting reports it and keeps the cells.
        let mut app = app;
        app.extend_selection(7, 7);
        let (app, message) = run(app, None, Some("x"));
        assert_eq!(app.board.live_cells_count(), 1);
        assert!(message.contains("clipboard"), "{message}");
    }
}
