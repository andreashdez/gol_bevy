//! Moving the view: wheel zoom, middle-button drag, keyboard panning, and reset
use super::COMMAND_KEYS;
use crate::{
    canvas_view::CanvasView,
    coords::{CAMERA_PAN_SPEED, CAMERA_ZOOM_MIN, CAMERA_ZOOM_STEP},
    keybindings::{
        BINDING_MOVE_CURSOR, BINDING_PAN_DOWN, BINDING_PAN_LEFT, BINDING_PAN_RIGHT, BINDING_PAN_UP,
        BINDING_RESET_CAMERA, KeyBinding, TypedKeys,
    },
    shared::{MainCamera, Simulation},
    ui_input::{BoardFocusVisible, UiInputCapture},
};
use bevy::{
    input::mouse::{MouseScrollPixelsPerLine, MouseScrollUnit, MouseWheel},
    prelude::*,
    window::PrimaryWindow,
};

pub(super) fn handle_camera_zoom(
    mut wheel_events: MessageReader<MouseWheel>,
    mut camera_q: Query<&mut Projection, With<MainCamera>>,
    capture: Res<UiInputCapture>,
    pixels_per_line: Res<MouseScrollPixelsPerLine>,
    sim: Res<Simulation>,
) {
    if capture.pointer {
        wheel_events.clear();
        return;
    }
    let Ok(mut projection) = camera_q.single_mut() else {
        return;
    };

    let lines: f32 = wheel_events
        .read()
        .map(|event| scroll_lines(event, *pixels_per_line))
        .sum();
    if lines == 0.0 {
        return;
    }

    let Projection::Orthographic(ortho) = &mut *projection else {
        return;
    };
    ortho.scale = (ortho.scale * zoom_factor(lines)).clamp(CAMERA_ZOOM_MIN, sim.zoom_max());
}

/// A wheel event's vertical scroll in lines. Trackpads, and browser wheels in
/// Chrome, report pixels, many small events per gesture or about a hundred
/// pixels per notch, which must zoom as far as the same scroll on a wheel that
/// reports lines.
pub(super) fn scroll_lines(event: &MouseWheel, pixels_per_line: MouseScrollPixelsPerLine) -> f32 {
    match event.unit {
        MouseScrollUnit::Line => event.y,
        MouseScrollUnit::Pixel => event.y / pixels_per_line,
    }
}

/// How much the view scale changes for a scroll of `lines`: a step per line,
/// compounded, so many small events add up to the same zoom as one large one.
/// A frame's change is still capped, so one flick cannot jump across the range.
pub(super) fn zoom_factor(lines: f32) -> f32 {
    (1.0 - CAMERA_ZOOM_STEP).powf(lines).clamp(0.5, 2.0)
}

pub(super) fn handle_camera_pan(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    board_focus: BoardFocusVisible,
    mut camera_q: Query<(&mut Transform, &Projection), With<MainCamera>>,
) {
    let Ok((mut camera_transform, projection)) = camera_q.single_mut() else {
        return;
    };
    // S is a pan key, but Cmd/Ctrl+S saves.
    if keys.any_pressed(COMMAND_KEYS) {
        return;
    }

    // While the board has visible keyboard focus, the arrow keys move the cell
    // cursor instead.
    let cursor_keys = board_focus.get();
    let held = |binding: KeyBinding| {
        binding.keys.iter().any(|key| {
            keys.pressed(*key) && !(cursor_keys && BINDING_MOVE_CURSOR.keys.contains(key))
        })
    };
    let mut direction = Vec3::ZERO;
    if held(BINDING_PAN_LEFT) {
        direction.x -= 1.0;
    }
    if held(BINDING_PAN_RIGHT) {
        direction.x += 1.0;
    }
    if held(BINDING_PAN_UP) {
        direction.y += 1.0;
    }
    if held(BINDING_PAN_DOWN) {
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

pub(super) fn handle_camera_drag(
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

pub(super) fn handle_camera_reset(
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
