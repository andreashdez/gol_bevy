use crate::{
    app::PatternKind,
    shared::{
        world_to_cell, DiagnosticsOverlay, GridOverlay, MainCamera, Simulation, UpdateSet,
        CAMERA_PAN_SPEED, CAMERA_ZOOM_MAX, CAMERA_ZOOM_MIN, CAMERA_ZOOM_STEP,
    },
};
use bevy::{input::mouse::MouseWheel, prelude::*, window::PrimaryWindow};

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                handle_keyboard.in_set(UpdateSet::Input),
                handle_mouse.in_set(UpdateSet::Input),
                handle_camera_zoom.in_set(UpdateSet::Camera),
                handle_camera_pan.in_set(UpdateSet::Camera),
            ),
        );
    }
}

fn handle_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut sim: ResMut<Simulation>,
    mut diagnostics: ResMut<DiagnosticsOverlay>,
    mut grid_overlay: ResMut<GridOverlay>,
) {
    if keys.just_pressed(KeyCode::Space) {
        sim.app.toggle_running();
    }
    if keys.just_pressed(KeyCode::KeyJ) {
        sim.app.increment_epoch();
    }
    if keys.just_pressed(KeyCode::KeyR) {
        sim.app.randomize_board();
    }
    if keys.just_pressed(KeyCode::KeyX) {
        sim.app.clear_board();
    }
    if keys.just_pressed(KeyCode::Digit0) {
        sim.app.reset_epoch();
    }
    if keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd) {
        sim.app.speed_up();
    }
    if keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract) {
        sim.app.slow_down();
    }
    if keys.just_pressed(KeyCode::Slash) {
        sim.app.toggle_help();
    }
    if keys.just_pressed(KeyCode::KeyG) {
        sim.app.select_pattern(PatternKind::Glider);
    }
    if keys.just_pressed(KeyCode::KeyB) {
        sim.app.select_pattern(PatternKind::Blinker);
    }
    if keys.just_pressed(KeyCode::KeyL) {
        sim.app.select_pattern(PatternKind::Lwss);
    }
    if keys.just_pressed(KeyCode::KeyC) {
        sim.app.clear_selected_pattern();
    }
    if keys.just_pressed(KeyCode::F3) {
        diagnostics.enabled = !diagnostics.enabled;
    }
    if keys.just_pressed(KeyCode::KeyT) {
        grid_overlay.enabled = !grid_overlay.enabled;
    }
    if keys.just_pressed(KeyCode::KeyQ) || keys.just_pressed(KeyCode::Escape) {
        sim.app.quit();
    }
}

fn handle_mouse(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    window_q: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut sim: ResMut<Simulation>,
) {
    let Ok(window) = window_q.single() else {
        return;
    };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };
    let Ok((camera, camera_transform)) = camera_q.single() else {
        return;
    };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) else {
        return;
    };

    let (cols, rows) = sim.app.board.size();
    let Some((col, row)) = world_to_cell(world_pos, cols, rows) else {
        return;
    };

    if mouse_buttons.just_released(MouseButton::Left) && sim.app.has_selected_pattern() {
        sim.app.place_selected_pattern_at(col, row);
    } else if mouse_buttons.pressed(MouseButton::Left) && !sim.app.has_selected_pattern() {
        sim.app.populate_board(col, row);
    }

    if mouse_buttons.pressed(MouseButton::Right) {
        sim.app.depopulate_board(col, row);
    }
}

fn handle_camera_zoom(
    mut wheel_events: MessageReader<MouseWheel>,
    mut camera_q: Query<&mut Projection, With<MainCamera>>,
) {
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
    if keys.pressed(KeyCode::KeyA) || keys.pressed(KeyCode::ArrowLeft) {
        direction.x -= 1.0;
    }
    if keys.pressed(KeyCode::KeyD) || keys.pressed(KeyCode::ArrowRight) {
        direction.x += 1.0;
    }
    if keys.pressed(KeyCode::KeyW) || keys.pressed(KeyCode::ArrowUp) {
        direction.y += 1.0;
    }
    if keys.pressed(KeyCode::KeyS) || keys.pressed(KeyCode::ArrowDown) {
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
