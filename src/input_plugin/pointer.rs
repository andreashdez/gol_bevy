//! Mouse editing: drawing, erasing, stamping, and selecting with the pointer
use crate::{
    app::EditTool,
    canvas_view::cursor_world,
    feedback::EditFeedback,
    shared::{MainCamera, Simulation},
    ui_input::UiInputCapture,
};
use bevy::{prelude::*, window::PrimaryWindow};

pub(super) fn finish_mouse_stroke(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut sim: ResMut<Simulation>,
) {
    // Always end strokes, including releases over UI widgets or outside the board.
    if !mouse_buttons.pressed(MouseButton::Left) && !mouse_buttons.pressed(MouseButton::Right) {
        sim.app.end_stroke();
    }
}

/// Mouse drags that began on the canvas.
#[derive(Default)]
pub(super) struct CanvasDrag {
    left: bool,
    right: bool,
    /// The cell a Select drag started on, until the button is released.
    select_anchor: Option<(i64, i64)>,
    /// The cell a draw or erase stroke last reached, and the pointer's position
    /// then, until the pointer lifts or leaves the board.
    last: Option<((i64, i64), Vec2)>,
}

pub(super) fn handle_mouse(
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
    // A stroke that resumes after a trip over the UI or off the board starts
    // afresh, rather than joining up under a panel or past an edge.
    let last = canvas_drag.last.take();
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
        let erase = sim.app.tool() == EditTool::Erase;
        for (x, y) in stroke_cells(last, (col, row), cursor_pos) {
            if erase {
                sim.app.depopulate_board(x, y);
            } else {
                sim.app.populate_board(x, y);
            }
        }
        canvas_drag.last = Some(((col, row), cursor_pos));
    }

    if canvas_drag.right {
        sim.app.begin_stroke();
        for (x, y) in stroke_cells(last, (col, row), cursor_pos) {
            sim.app.depopulate_board(x, y);
        }
        canvas_drag.last = Some(((col, row), cursor_pos));
    }
}

/// The cells a stroke covers on its way to `to`, under the pointer at `pointer`:
/// a straight line on from where it last was, so a fast drag leaves no gaps, or
/// just `to` for a stroke's first cell. When zoomed out past a cell a pixel, the
/// line takes a cell per pixel the pointer moved, which leaves no gap on screen
/// however far apart its ends are on the plane.
pub(super) fn stroke_cells(
    last: Option<((i64, i64), Vec2)>,
    to: (i64, i64),
    pointer: Vec2,
) -> impl Iterator<Item = (i64, i64)> {
    let (from, steps) = match last {
        Some((from, last_pointer)) => {
            let (dx, dy) = (
                i128::from(to.0) - i128::from(from.0),
                i128::from(to.1) - i128::from(from.1),
            );
            let pixels = (pointer - last_pointer).abs().max_element().ceil() as i128;
            (from, dx.abs().max(dy.abs()).min(pixels).max(1))
        }
        None => (to, 1),
    };
    // Rounded to the nearest cell, so the line is as straight as cells allow.
    let along = move |start: i64, end: i64, step: i128| {
        let span = i128::from(end) - i128::from(start);
        start + (2 * step * span + steps).div_euclid(2 * steps) as i64
    };
    (1..=steps).map(move |step| (along(from.0, to.0, step), along(from.1, to.1, step)))
}
