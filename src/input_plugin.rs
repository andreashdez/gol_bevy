use crate::{
    app::{EditTool, PatternKind},
    canvas_view::{CanvasView, cursor_world},
    shared::{
        BINDING_CLEAR, BINDING_CLEAR_STAMP, BINDING_CYCLE_LAYOUT, BINDING_FLIP_STAMP,
        BINDING_PAN_DOWN, BINDING_PAN_LEFT, BINDING_PAN_RIGHT, BINDING_PAN_UP, BINDING_QUIT,
        BINDING_RANDOMIZE, BINDING_RESET_CAMERA, BINDING_RESET_EPOCH, BINDING_ROTATE_STAMP,
        BINDING_RUN_PAUSE, BINDING_SELECT_BLINKER, BINDING_SELECT_GLIDER, BINDING_SELECT_LWSS,
        BINDING_SELECT_RLE_DEMO, BINDING_SLOW_DOWN, BINDING_SPEED_UP, BINDING_STEP,
        BINDING_TOGGLE_BOUNDARY_MODE, BINDING_TOGGLE_CONTROL_DECK, BINDING_TOGGLE_DIAGNOSTICS,
        BINDING_TOGGLE_GRID, BINDING_TOGGLE_HELP, CAMERA_PAN_SPEED, CAMERA_ZOOM_MAX,
        CAMERA_ZOOM_MIN, CAMERA_ZOOM_STEP, DiagnosticsOverlay, GridOverlay, MainCamera, Simulation,
        UiLayout, UpdateSet, world_to_cell,
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
            handle_layout_cycle
                .run_if(canvas_keyboard_available)
                .in_set(UpdateSet::Input),
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
        );
    }
}

fn handle_layout_cycle(keys: Res<ButtonInput<KeyCode>>, mut ui_layout: ResMut<UiLayout>) {
    if BINDING_CYCLE_LAYOUT.just_pressed(&keys) {
        ui_layout.cycle();
    }
}

fn handle_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut sim: ResMut<Simulation>,
    mut diagnostics: ResMut<DiagnosticsOverlay>,
    mut grid_overlay: ResMut<GridOverlay>,
    mut ui_layout: ResMut<UiLayout>,
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
    if keys.just_pressed(KeyCode::KeyU) {
        sim.app.undo();
        return;
    }
    if command {
        if keys.just_pressed(KeyCode::KeyZ) {
            let shift = keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight);
            if shift {
                sim.app.redo();
            } else {
                sim.app.undo();
            }
        }
        return;
    }
    if BINDING_RUN_PAUSE.just_pressed(&keys) {
        sim.app.toggle_running();
    }
    if BINDING_STEP.just_pressed(&keys) {
        sim.app.increment_epoch();
    }
    if BINDING_RANDOMIZE.just_pressed(&keys) {
        sim.app.randomize_board();
    }
    if BINDING_CLEAR.just_pressed(&keys) {
        sim.app.clear_board();
    }
    if BINDING_RESET_EPOCH.just_pressed(&keys) {
        sim.app.reset_epoch();
    }
    if BINDING_SPEED_UP.just_pressed(&keys) {
        sim.app.speed_up();
    }
    if BINDING_SLOW_DOWN.just_pressed(&keys) {
        sim.app.slow_down();
    }
    if BINDING_TOGGLE_HELP.just_pressed(&keys) {
        sim.app.toggle_help();
    }
    if BINDING_SELECT_GLIDER.just_pressed(&keys) {
        sim.app.select_pattern(PatternKind::Glider);
    }
    if BINDING_SELECT_BLINKER.just_pressed(&keys) {
        sim.app.select_pattern(PatternKind::Blinker);
    }
    if BINDING_SELECT_LWSS.just_pressed(&keys) {
        sim.app.select_pattern(PatternKind::Lwss);
    }
    if BINDING_SELECT_RLE_DEMO.just_pressed(&keys) {
        sim.app.select_pattern(PatternKind::RleDemo);
    }
    if BINDING_CLEAR_STAMP.just_pressed(&keys) {
        sim.app.clear_selected_pattern();
    }
    if BINDING_ROTATE_STAMP.just_pressed(&keys) {
        sim.app.rotate_stamp_clockwise();
    }
    if BINDING_FLIP_STAMP.just_pressed(&keys) {
        sim.app.toggle_stamp_flip();
    }
    if BINDING_TOGGLE_DIAGNOSTICS.just_pressed(&keys) {
        diagnostics.enabled = !diagnostics.enabled;
    }
    if BINDING_TOGGLE_GRID.just_pressed(&keys) {
        grid_overlay.enabled = !grid_overlay.enabled;
    }
    if BINDING_TOGGLE_BOUNDARY_MODE.just_pressed(&keys) {
        sim.app.toggle_boundary_mode();
    }
    if BINDING_TOGGLE_CONTROL_DECK.just_pressed(&keys) {
        ui_layout.toggle_control_deck();
    }
    if BINDING_QUIT.just_pressed(&keys) {
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
) {
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
    }
    if mouse_buttons.just_pressed(MouseButton::Right) {
        canvas_drag.1 = true;
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

fn handle_camera_reset(
    view: Res<CanvasView>,
    keys: Res<ButtonInput<KeyCode>>,
    mut camera_q: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
) {
    if !BINDING_RESET_CAMERA.just_pressed(&keys) {
        return;
    }

    let Ok((mut transform, mut projection)) = camera_q.single_mut() else {
        return;
    };

    transform.translation.x = 0.0;
    transform.translation.y = 0.0;

    if let Projection::Orthographic(ref mut ortho) = *projection {
        ortho.scale = view.fit_scale;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn fast_undo_chord_does_not_rotate_stamp() {
        for modifier in [Some(KeyCode::SuperLeft), Some(KeyCode::ControlLeft), None] {
            let mut world = World::new();
            let mut app = crate::app::App::new(8, 8);
            app.populate_board(3, 3);
            app.select_pattern(PatternKind::Glider);
            world.insert_resource(Simulation::new(app));
            let mut keys = ButtonInput::<KeyCode>::default();
            if let Some(modifier) = modifier {
                keys.press(modifier);
                keys.press(KeyCode::KeyZ);
                keys.release(KeyCode::KeyZ);
                keys.release(modifier);
            } else {
                keys.press(KeyCode::KeyU);
                keys.release(KeyCode::KeyU);
            }
            world.insert_resource(keys);
            world.insert_resource(DiagnosticsOverlay { enabled: false });
            world.insert_resource(GridOverlay { enabled: false });
            world.insert_resource(UiLayout::default());
            world.run_system_once(handle_keyboard).unwrap();
            let app = &world.resource::<Simulation>().app;
            assert_eq!(app.board.live_cells_count(), 0);
            assert_eq!(app.selected_pattern_status_label(), "Glider");
        }
    }
}
