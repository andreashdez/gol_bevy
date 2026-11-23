//! The keyboard's cell cursor, for editing the board without a pointer
use super::COMMAND_KEYS;
use crate::{
    app::EditTool,
    coords::CELL_SIZE,
    feedback::EditFeedback,
    keybindings::{BINDING_APPLY_AT_CURSOR, BINDING_EXTEND_SELECTION, TypedKeys},
    shared::{MainCamera, Simulation},
    sparse_board::CellRect,
    ui_input::{BoardFocusVisible, UiInputCapture},
};
use bevy::{
    input::{ButtonState, keyboard::KeyboardInput},
    prelude::*,
    window::PrimaryWindow,
};

/// The keyboard's cell cursor, for editing the board without a pointer. It is
/// active while the board has visible keyboard focus: the arrow keys move it,
/// and Enter applies the tool at it.
#[derive(Resource, Default)]
pub struct BoardCursor {
    /// Kept while the cursor is inactive, so it comes back where it was.
    cell: Option<(i64, i64)>,
    active: bool,
    /// Whether the hover outline and the stamp ghost mark the cursor rather
    /// than the pointer. Moving the mouse hands them back until the next key.
    shown: bool,
    selection: Option<KeyboardSelection>,
}

/// A selection made from the keyboard, as the keyboard last left it.
#[derive(Clone, Copy)]
struct KeyboardSelection {
    area: CellRect,
    /// The selection's moving corner, where the cursor was.
    corner: (i64, i64),
    /// Started with Enter: every move extends it until Enter again. Otherwise
    /// only Shift+arrows extend it.
    anchored: bool,
}

impl BoardCursor {
    /// The cell the hover outline and the stamp ghost mark instead of the
    /// pointer's.
    pub fn shown(&self) -> Option<(i64, i64)> {
        self.cell.filter(|_| self.active && self.shown)
    }

    /// The cursor's cell while the board has visible keyboard focus.
    pub fn cell(&self) -> Option<(i64, i64)> {
        self.cell.filter(|_| self.active)
    }

    /// A cursor showing on `cell`, as after Tab.
    #[cfg(test)]
    pub(crate) fn at(cell: (i64, i64)) -> Self {
        Self {
            cell: Some(cell),
            active: true,
            shown: true,
            selection: None,
        }
    }

    /// The selection the keyboard is making, while the board still has it as
    /// the keyboard left it and the cursor is on its moving corner. A drag,
    /// Esc, or another tool lets go of it.
    fn keyboard_selection(&mut self, app: &crate::app::App) -> Option<&mut KeyboardSelection> {
        let cell = self.cell;
        if let Some(made) = self.selection
            && (app.selection() != Some(made.area) || cell != Some(made.corner))
        {
            self.selection = None;
        }
        self.selection.as_mut()
    }
}

#[derive(bevy::ecs::system::SystemParam)]
pub(super) struct CursorKeys<'w, 's> {
    events: MessageReader<'w, 's, KeyboardInput>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    typed: Res<'w, TypedKeys>,
    capture: Res<'w, UiInputCapture>,
    board_focus: BoardFocusVisible<'w, 's>,
}

/// Moves the cell cursor with the arrow keys, again on each key repeat, and
/// applies the tool at it with Enter. The view pans to keep the cursor in it.
pub(super) fn handle_board_cursor(
    mut input: CursorKeys,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut cameras: Query<(&Camera, &mut Transform, &Projection), With<MainCamera>>,
    mut sim: ResMut<Simulation>,
    mut feedback: ResMut<EditFeedback>,
    mut cursor: ResMut<BoardCursor>,
    mut pointer: Local<Option<Vec2>>,
) {
    let position = windows.single().ok().and_then(Window::cursor_position);
    if position.is_some() && position != *pointer {
        cursor.shown = false;
    }
    *pointer = position;
    if input.capture.keyboard || !input.board_focus.get() {
        input.events.clear();
        cursor.active = false;
        return;
    }
    let Ok((camera, mut transform, projection)) = cameras.single_mut() else {
        input.events.clear();
        return;
    };
    let map = sim.cell_map();
    let scale = match projection {
        Projection::Orthographic(ortho) => ortho.scale,
        _ => 1.0,
    };
    let seen = camera
        .logical_viewport_size()
        .map(|size| Rect::from_center_size(transform.translation.truncate(), size * scale));
    if !cursor.active {
        // Back where it was while that is still on the board and in view, or
        // else in the middle of the view.
        let kept = cursor.cell.filter(|&(col, row)| {
            sim.app.board.contains(col, row)
                && seen.is_none_or(|seen| seen.contains(map.cell_to_world(col, row).truncate()))
        });
        cursor.cell =
            Some(kept.unwrap_or_else(|| map.nearest_cell(transform.translation.truncate())));
        cursor.active = true;
        cursor.shown = true;
    }
    // The board may have shrunk under it.
    let start = clamp_to_board(&sim, cursor.cell.unwrap_or_default());
    cursor.cell = Some(start);

    // Cmd/Ctrl+arrows are left to the platform.
    let command = input.keys.any_pressed(COMMAND_KEYS);
    let extend =
        BINDING_EXTEND_SELECTION.pressed(&input.keys) && sim.app.tool() == EditTool::Select;
    for event in input.events.read() {
        let Some((cols, rows)) =
            cursor_step(event.key_code).filter(|_| event.state == ButtonState::Pressed && !command)
        else {
            continue;
        };
        let from = cursor.cell.unwrap_or(start);
        if extend && cursor.keyboard_selection(&sim.app).is_none() {
            sim.app.clear_selection();
            sim.app.begin_selection(from.0, from.1);
            cursor.selection = sim.app.selection().map(|area| KeyboardSelection {
                area,
                corner: from,
                anchored: false,
            });
        }
        let to = clamp_to_board(
            &sim,
            (from.0.saturating_add(cols), from.1.saturating_add(rows)),
        );
        let made = cursor.keyboard_selection(&sim.app).copied();
        cursor.cell = Some(to);
        cursor.shown = true;
        if let Some(made) = made
            && (extend || made.anchored)
        {
            sim.app.extend_selection(to.0, to.1);
            cursor.selection = sim.app.selection().map(|area| KeyboardSelection {
                area,
                corner: to,
                ..made
            });
        }
    }
    let cell = cursor.cell.unwrap_or(start);
    if cell != start
        && let Some(seen) = seen
    {
        // Keep a cell's width between the cursor and the edge of the view,
        // where the view has room for it.
        let room = (seen.half_size() - Vec2::splat(CELL_SIZE)).max(Vec2::ZERO);
        let offset = map.cell_to_world(cell.0, cell.1).truncate() - seen.center();
        transform.translation += (offset - offset.clamp(-room, room)).extend(0.0);
    }

    if !BINDING_APPLY_AT_CURSOR.just_pressed(&input.keys, &input.typed) || command {
        return;
    }
    cursor.shown = true;
    feedback.remaining = 0.0;
    let (col, row) = cell;
    match sim.app.tool() {
        EditTool::Select => match cursor.keyboard_selection(&sim.app) {
            // A second Enter ends the selection, keeping it.
            Some(made) if made.anchored => made.anchored = false,
            _ => {
                sim.app.clear_selection();
                sim.app.begin_selection(col, row);
                cursor.selection = sim.app.selection().map(|area| KeyboardSelection {
                    area,
                    corner: cell,
                    anchored: true,
                });
            }
        },
        _ if sim.app.has_selected_pattern() => sim.app.place_selected_pattern_at(col, row),
        tool => {
            // One key press, one undo step.
            sim.app.begin_stroke();
            if tool == EditTool::Erase {
                sim.app.depopulate_board(col, row);
            } else {
                sim.app.populate_board(col, row);
            }
            sim.app.end_stroke();
        }
    }
}

/// Which way an arrow key moves the cell cursor, in columns and rows; rows
/// count up from the bottom.
fn cursor_step(key: KeyCode) -> Option<(i64, i64)> {
    match key {
        KeyCode::ArrowLeft => Some((-1, 0)),
        KeyCode::ArrowRight => Some((1, 0)),
        KeyCode::ArrowUp => Some((0, 1)),
        KeyCode::ArrowDown => Some((0, -1)),
        _ => None,
    }
}

/// The nearest cell on a finite board; the plane has every cell.
fn clamp_to_board(sim: &Simulation, (col, row): (i64, i64)) -> (i64, i64) {
    match sim.app.board.size() {
        Some((cols, rows)) => (col.clamp(0, cols as i64 - 1), row.clamp(0, rows as i64 - 1)),
        None => (col, row),
    }
}
