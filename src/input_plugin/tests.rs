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
    if let Projection::Orthographic(ortho) = &mut *world.get_mut::<Projection>(camera).unwrap() {
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
fn letting_go_of_cmd_releases_keys_the_browser_never_reported_released() {
    let mut world = World::new();
    let mut keys = ButtonInput::<KeyCode>::default();
    keys.press(KeyCode::SuperLeft);
    keys.press(KeyCode::ShiftLeft);
    keys.press(KeyCode::KeyA);
    world.insert_resource(keys);

    // Cmd still down: nothing changes yet.
    world
        .run_system_once(release_keys_held_through_command)
        .unwrap();
    assert!(
        world
            .resource::<ButtonInput<KeyCode>>()
            .pressed(KeyCode::KeyA)
    );

    // The browser reports Cmd going up but never A.
    world.resource_mut::<ButtonInput<KeyCode>>().clear();
    world
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::SuperLeft);
    world
        .run_system_once(release_keys_held_through_command)
        .unwrap();
    let keys = world.resource::<ButtonInput<KeyCode>>();
    assert!(!keys.pressed(KeyCode::KeyA));
    assert!(!BINDING_PAN_LEFT.pressed(keys));
    assert!(keys.pressed(KeyCode::ShiftLeft));
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

    // Russian layout: letter keys type Cyrillic, so they fall back to the
    // US letter on the same key.
    let before = rotation(&app);
    press(&mut app, KeyCode::KeyZ, "я");
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
fn a_fast_drag_draws_a_line_but_not_across_the_ui() {
    let (mut world, window) = selection_world();
    world
        .resource_mut::<Simulation>()
        .app
        .set_tool(EditTool::Draw);
    let mouse = world.register_system(handle_mouse);
    let alive = |world: &World, cell: (i64, i64)| {
        world
            .resource::<Simulation>()
            .app
            .board
            .is_alive(cell.0, cell.1)
    };

    point_at(&mut world, window, (1.0, 1.0));
    world
        .resource_mut::<ButtonInput<MouseButton>>()
        .press(MouseButton::Left);
    world.run_system(mouse).unwrap();
    world.resource_mut::<ButtonInput<MouseButton>>().clear();
    // Five cells right and three up within one frame.
    point_at(&mut world, window, (6.0, 4.0));
    world.run_system(mouse).unwrap();
    let line = [(1, 1), (2, 2), (3, 2), (4, 3), (5, 3), (6, 4)];
    assert!(line.iter().all(|&cell| alive(&world, cell)));
    assert_eq!(
        world.resource::<Simulation>().app.board.live_cells_count(),
        line.len()
    );

    // Over a panel and back, the stroke picks up where the pointer is.
    world.resource_mut::<UiInputCapture>().pointer = true;
    world.run_system(mouse).unwrap();
    world.resource_mut::<UiInputCapture>().pointer = false;
    point_at(&mut world, window, (6.0, 7.0));
    world.run_system(mouse).unwrap();
    assert!(alive(&world, (6, 7)));
    assert!(!alive(&world, (6, 5)) && !alive(&world, (6, 6)));
}

#[test]
fn a_stroke_far_zoomed_out_takes_a_cell_per_pixel_moved() {
    let cells: Vec<_> = stroke_cells(
        Some(((0, 0), Vec2::ZERO)),
        (1 << 40, -(1 << 40)),
        Vec2::new(4.0, 4.0),
    )
    .collect();
    let quarter = 1 << 38;
    assert_eq!(
        cells,
        [
            (quarter, -quarter),
            (2 * quarter, -2 * quarter),
            (3 * quarter, -3 * quarter),
            (4 * quarter, -4 * quarter)
        ]
    );
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

#[test]
fn pixel_scrolls_zoom_as_far_as_the_same_scroll_in_lines() {
    let wheel = |unit, y| MouseWheel {
        unit,
        x: 0.0,
        y,
        window: Entity::PLACEHOLDER,
        phase: bevy::input::touch::TouchPhase::Moved,
    };
    let per_line = MouseScrollPixelsPerLine::default();
    let notch = zoom_factor(scroll_lines(&wheel(MouseScrollUnit::Line, 1.0), per_line));
    assert!((notch - (1.0 - CAMERA_ZOOM_STEP)).abs() < 1e-6);

    // A browser notch in pixels zooms one step, not to the frame's cap.
    let browser = zoom_factor(scroll_lines(
        &wheel(MouseScrollUnit::Pixel, *per_line),
        per_line,
    ));
    assert!((browser - notch).abs() < 1e-6);

    // A trackpad gesture of many small events adds up to the same zoom.
    let gesture: f32 = (0..20)
        .map(|_| zoom_factor(scroll_lines(&wheel(MouseScrollUnit::Pixel, 5.0), per_line)))
        .product();
    assert!((gesture - notch).abs() < 1e-4, "{gesture} vs {notch}");
}

/// An 8 × 8 board with visible keyboard focus, as after Tab, filling an
/// 800 × 600 canvas one world unit to a pixel, and the cursor system, run
/// once to put the cursor on the board.
fn cursor_world(app: crate::app::App) -> (World, bevy::ecs::system::SystemId) {
    use bevy::{
        camera::{ComputedCameraValues, RenderTargetInfo},
        input_focus::{InputFocus, InputFocusVisible},
    };
    let mut world = World::new();
    world.init_resource::<ButtonInput<KeyCode>>();
    world.init_resource::<Messages<KeyboardInput>>();
    world.init_resource::<TypedKeys>();
    world.init_resource::<UiInputCapture>();
    world.init_resource::<crate::ui_input::UiInputRegions>();
    world.init_resource::<InputFocus>();
    world.insert_resource(InputFocusVisible(true));
    world.init_resource::<EditFeedback>();
    world.init_resource::<BoardCursor>();
    world.insert_resource(Simulation::new(app));
    world.spawn((Window::default(), PrimaryWindow));
    world.spawn((
        Camera2d,
        MainCamera,
        Camera {
            computed: ComputedCameraValues {
                target_info: Some(RenderTargetInfo {
                    physical_size: UVec2::new(800, 600),
                    scale_factor: 1.0,
                }),
                ..default()
            },
            ..default()
        },
    ));
    let system = world.register_system(handle_board_cursor);
    world.run_system(system).unwrap();
    (world, system)
}

/// One frame in which `keys` go down in order, a key named again arriving
/// as a key repeat, then all come up.
fn press(world: &mut World, system: bevy::ecs::system::SystemId, keys: &[KeyCode]) {
    use bevy::input::keyboard::{Key, NativeKey};
    let window = world
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(world)
        .unwrap();
    for (index, &key_code) in keys.iter().enumerate() {
        world.resource_mut::<ButtonInput<KeyCode>>().press(key_code);
        world.write_message(KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state: ButtonState::Pressed,
            text: None,
            repeat: keys[..index].contains(&key_code),
            window,
        });
    }
    world.run_system(system).unwrap();
    let mut input = world.resource_mut::<ButtonInput<KeyCode>>();
    for &key in keys {
        input.release(key);
    }
    input.clear();
}

fn cursor(world: &World) -> Option<(i64, i64)> {
    world.resource::<BoardCursor>().cell()
}

fn camera_at(world: &mut World) -> Vec2 {
    world
        .query_filtered::<&Transform, With<MainCamera>>()
        .single(world)
        .unwrap()
        .translation
        .truncate()
}

#[test]
fn arrows_move_the_cell_cursor_and_stop_at_the_board_edge() {
    let (mut world, system) = cursor_world(crate::app::App::new(8, 8));
    // It starts in the middle of the view, outlined in place of the pointer.
    assert_eq!(cursor(&world), Some((4, 4)));
    assert_eq!(world.resource::<BoardCursor>().shown(), Some((4, 4)));
    press(&mut world, system, &[KeyCode::ArrowRight]);
    press(&mut world, system, &[KeyCode::ArrowUp]);
    assert_eq!(cursor(&world), Some((5, 5)));

    // Held keys repeat, and the board's edges stop the cursor.
    press(&mut world, system, &[KeyCode::ArrowLeft; 7]);
    press(&mut world, system, &[KeyCode::ArrowDown; 7]);
    assert_eq!(cursor(&world), Some((0, 0)));
    // The whole board is in view, so the view stays put.
    assert_eq!(camera_at(&mut world), Vec2::ZERO);

    // Moving the mouse gives the outline back to the pointer, until the
    // next arrow key.
    let mut windows = world.query_filtered::<&mut Window, With<PrimaryWindow>>();
    windows
        .single_mut(&mut world)
        .unwrap()
        .set_cursor_position(Some(Vec2::new(100.0, 100.0)));
    press(&mut world, system, &[]);
    assert_eq!(world.resource::<BoardCursor>().shown(), None);
    assert_eq!(cursor(&world), Some((0, 0)));
    press(&mut world, system, &[KeyCode::ArrowUp]);
    assert_eq!(world.resource::<BoardCursor>().shown(), Some((0, 1)));
}

#[test]
fn the_view_follows_the_cell_cursor_across_the_plane() {
    let mut app = crate::app::App::new(8, 8);
    app.set_unbounded().unwrap();
    let (mut world, system) = cursor_world(app);
    assert_eq!(cursor(&world), Some((0, 0)));
    press(&mut world, system, &[KeyCode::ArrowLeft; 40]);
    assert_eq!(cursor(&world), Some((-40, 0)));
    // Just far enough to keep a cell between the cursor and the view's
    // edge: the cell's middle is 474 to the left, the edge 400.
    assert_eq!(camera_at(&mut world), Vec2::new(-86.0, 0.0));
}

#[test]
fn enter_draws_or_erases_at_the_cursor_one_undo_step_at_a_time() {
    let (mut world, system) = cursor_world(crate::app::App::new(8, 8));
    press(&mut world, system, &[KeyCode::ArrowRight]);
    press(&mut world, system, &[KeyCode::Enter]);
    press(&mut world, system, &[KeyCode::ArrowRight]);
    press(&mut world, system, &[KeyCode::NumpadEnter]);
    {
        let app = &mut world.resource_mut::<Simulation>().into_inner().app;
        assert!(app.board.is_alive(5, 4) && app.board.is_alive(6, 4));
        assert_eq!(app.board.live_cells_count(), 2);
        assert!(app.undo());
        assert!(app.board.is_alive(5, 4) && !app.board.is_alive(6, 4));
        assert!(app.undo());
        assert_eq!(app.board.live_cells_count(), 0);
        assert!(!app.can_undo());
        assert!(app.redo() && app.redo());
        app.set_tool(EditTool::Erase);
    }
    press(&mut world, system, &[KeyCode::Enter]);
    let app = &world.resource::<Simulation>().app;
    assert!(app.board.is_alive(5, 4) && !app.board.is_alive(6, 4));
}

#[test]
fn enter_places_the_stamp_where_a_click_on_the_cursor_would() {
    let stamping = || {
        let mut app = crate::app::App::new(8, 8);
        app.select_pattern(crate::app::PatternKind::Glider);
        app
    };
    let mut clicked = stamping();
    clicked.place_selected_pattern_at(4, 4);
    let (mut world, system) = cursor_world(stamping());
    press(&mut world, system, &[KeyCode::Enter]);
    let app = &mut world.resource_mut::<Simulation>().into_inner().app;
    assert_eq!(app.board.live_cells_count(), 5);
    for (col, row) in (0..8).flat_map(|col| (0..8).map(move |row| (col, row))) {
        assert_eq!(
            app.board.is_alive(col, row),
            clicked.board.is_alive(col, row),
            "({col}, {row})"
        );
    }
    assert!(app.undo());
    assert_eq!(app.board.live_cells_count(), 0);
}

#[test]
fn shift_arrows_and_enter_select_from_the_cursor() {
    let mut app = crate::app::App::new(8, 8);
    app.set_tool(EditTool::Select);
    let (mut world, system) = cursor_world(app);
    let selection = |world: &World| world.resource::<Simulation>().app.selection();
    let area = |left, bottom, width, height| {
        Some(CellRect {
            left,
            bottom,
            width,
            height,
        })
    };

    // Shift+arrows start a selection on the cursor's cell and stretch it.
    world
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ShiftLeft);
    press(
        &mut world,
        system,
        &[KeyCode::ArrowRight, KeyCode::ArrowRight, KeyCode::ArrowDown],
    );
    assert_eq!(selection(&world), area(4, 3, 3, 2));
    // Without Shift, the cursor moves on and the selection stays.
    world
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::ShiftLeft);
    press(&mut world, system, &[KeyCode::ArrowLeft]);
    assert_eq!(selection(&world), area(4, 3, 3, 2));
    assert_eq!(cursor(&world), Some((5, 3)));

    // Enter anchors a new selection, arrows stretch it, and Enter ends it.
    press(&mut world, system, &[KeyCode::Enter]);
    assert_eq!(selection(&world), area(5, 3, 1, 1));
    press(&mut world, system, &[KeyCode::ArrowLeft]);
    assert_eq!(selection(&world), area(4, 3, 2, 1));
    press(&mut world, system, &[KeyCode::Enter]);
    press(&mut world, system, &[KeyCode::ArrowUp]);
    assert_eq!(selection(&world), area(4, 3, 2, 1));
    assert_eq!(cursor(&world), Some((4, 4)));
    assert_eq!(
        world.resource::<Simulation>().app.board.live_cells_count(),
        0,
        "selecting draws nothing"
    );
}

#[test]
fn arrows_pan_unless_the_board_has_visible_keyboard_focus() {
    use bevy::input_focus::InputFocusVisible;
    let (mut world, system) = cursor_world(crate::app::App::new(8, 8));
    let mut time = Time::<()>::default();
    time.advance_by(std::time::Duration::from_millis(100));
    world.insert_resource(time);
    let pan = world.register_system(handle_camera_pan);
    let panned_to =
        |world: &mut World, x: f32| camera_at(world).abs_diff_eq(Vec2::new(x, 0.0), 1e-3);

    world
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::ArrowLeft);
    world.run_system(pan).unwrap();
    assert!(panned_to(&mut world, 0.0), "the arrows move the cursor");
    world
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::KeyA);
    world.run_system(pan).unwrap();
    assert!(panned_to(&mut world, -70.0), "WASD still pans");

    // A click hides keyboard focus: the arrows pan again, and the cursor
    // goes away.
    world
        .resource_mut::<ButtonInput<KeyCode>>()
        .release(KeyCode::KeyA);
    world.resource_mut::<InputFocusVisible>().0 = false;
    world.run_system(pan).unwrap();
    assert!(panned_to(&mut world, -140.0));
    press(&mut world, system, &[KeyCode::ArrowRight]);
    assert_eq!(cursor(&world), None);
    assert_eq!(world.resource::<BoardCursor>().shown(), None);
    assert!(panned_to(&mut world, -140.0));
}
