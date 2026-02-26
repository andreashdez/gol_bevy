use crate::shared::{DiagnosticsOverlay, MainCamera, Simulation, UpdateSet, world_to_cell};
use bevy::{prelude::*, window::PrimaryWindow};

pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                handle_keyboard.in_set(UpdateSet::Input),
                handle_mouse.in_set(UpdateSet::Input),
            ),
        );
    }
}

fn handle_keyboard(
    keys: Res<ButtonInput<KeyCode>>,
    mut sim: ResMut<Simulation>,
    mut diagnostics: ResMut<DiagnosticsOverlay>,
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
        sim.app.insert_glider();
    }
    if keys.just_pressed(KeyCode::KeyB) {
        sim.app.insert_blinker();
    }
    if keys.just_pressed(KeyCode::KeyL) {
        sim.app.insert_lwss();
    }
    if keys.just_pressed(KeyCode::F3) {
        diagnostics.enabled = !diagnostics.enabled;
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

    if mouse_buttons.pressed(MouseButton::Left) {
        sim.app.populate_board(col, row);
    }
    if mouse_buttons.pressed(MouseButton::Right) {
        sim.app.depopulate_board(col, row);
    }
}
