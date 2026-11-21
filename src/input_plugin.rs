use crate::{
    app::EditTool,
    canvas_view::{CanvasView, cursor_world},
    coords::{CAMERA_PAN_SPEED, CAMERA_ZOOM_MIN, CAMERA_ZOOM_STEP, CELL_SIZE},
    feedback::EditFeedback,
    keybindings::{
        BINDING_APPLY_AT_CURSOR, BINDING_CLEAR, BINDING_CLEAR_STAMP, BINDING_DESELECT,
        BINDING_ERASE_SELECTION, BINDING_EXTEND_SELECTION, BINDING_FLIP_STAMP, BINDING_MOVE_CURSOR,
        BINDING_PAN_DOWN, BINDING_PAN_LEFT, BINDING_PAN_RIGHT, BINDING_PAN_UP, BINDING_QUIT,
        BINDING_RANDOMIZE, BINDING_RESET_CAMERA, BINDING_RESET_EPOCH, BINDING_RESET_TO_START,
        BINDING_ROTATE_STAMP, BINDING_RUN_PAUSE, BINDING_SLOW_DOWN, BINDING_SPEED_UP, BINDING_STEP,
        BINDING_TOGGLE_BOUNDARY_MODE, BINDING_TOGGLE_DIAGNOSTICS, BINDING_TOGGLE_GRID,
        BINDING_TOGGLE_HELP, BINDING_TOGGLE_PALETTE, KeyBinding, TOOL_SHORTCUTS, TypedKeys,
        collect_typed_keys,
    },
    shared::{DiagnosticsOverlay, GridOverlay, MainCamera, Simulation, UpdateSet, WEB_BUILD},
    sparse_board::CellRect,
    ui_input::{BoardFocusVisible, UiInputCapture, canvas_keyboard_available},
};
use bevy::{
    clipboard::Clipboard,
    input::{
        ButtonState,
        keyboard::KeyboardInput,
        mouse::{MouseScrollPixelsPerLine, MouseScrollUnit, MouseWheel},
    },
    prelude::*,
    window::PrimaryWindow,
};
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
        .add_systems(
            PostUpdate,
            finish_paste.in_set(UpdateSet::Input).after(handle_keyboard),
        )
        .add_systems(Update, handle_camera_zoom.in_set(UpdateSet::Camera))
        .add_systems(Update, handle_camera_drag.in_set(UpdateSet::Camera))
        .init_resource::<BoardCursor>()
        // Not gated on `canvas_keyboard_available`: it drains its key events
        // while it may not use them, so none arrive late.
        .add_systems(Update, handle_board_cursor.in_set(UpdateSet::Camera))
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
        if WEB_BUILD {
            app.add_systems(
                PreUpdate,
                release_keys_held_through_command.after(bevy::input::InputSystems),
            );
        }
    }
}

/// Cmd on macOS and Ctrl elsewhere start app shortcuts like undo and save.
pub const COMMAND_KEYS: [KeyCode; 4] = [
    KeyCode::SuperLeft,
    KeyCode::SuperRight,
    KeyCode::ControlLeft,
    KeyCode::ControlRight,
];

/// Keys that stay down through a Cmd release, like Shift in Cmd+Shift+Z.
const MODIFIER_KEYS: [KeyCode; 8] = [
    KeyCode::SuperLeft,
    KeyCode::SuperRight,
    KeyCode::ControlLeft,
    KeyCode::ControlRight,
    KeyCode::ShiftLeft,
    KeyCode::ShiftRight,
    KeyCode::AltLeft,
    KeyCode::AltRight,
];

/// Browsers on macOS never report releasing a key pressed while Cmd was
/// down, so after Cmd+A the app would think A is still held and pan forever.
/// When Cmd is let go, every other key counts as released; one that really is
/// still held is pressed again by the browser's key repeat.
fn release_keys_held_through_command(mut keys: ResMut<ButtonInput<KeyCode>>) {
    if !keys.any_just_released([KeyCode::SuperLeft, KeyCode::SuperRight]) {
        return;
    }
    let stuck: Vec<KeyCode> = keys
        .get_pressed()
        .filter(|key| !MODIFIER_KEYS.contains(key))
        .copied()
        .collect();
    for key in stuck {
        keys.release(key);
    }
}

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
    // Quitting would only freeze the page in the web build.
    if !WEB_BUILD && BINDING_QUIT.just_pressed(&keys, &typed) {
        sim.app.quit();
    }
}

/// Finishes a paste whose clipboard text arrives after the key press, as it
/// does in browsers.
fn finish_paste(mut sim: ResMut<Simulation>, mut feedback: ResMut<EditFeedback>) {
    feedback.finish_paste(&mut sim.app);
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
struct CursorKeys<'w, 's> {
    events: MessageReader<'w, 's, KeyboardInput>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    typed: Res<'w, TypedKeys>,
    capture: Res<'w, UiInputCapture>,
    board_focus: BoardFocusVisible<'w, 's>,
}

/// Moves the cell cursor with the arrow keys, again on each key repeat, and
/// applies the tool at it with Enter. The view pans to keep the cursor in it.
fn handle_board_cursor(
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

fn handle_camera_zoom(
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
fn scroll_lines(event: &MouseWheel, pixels_per_line: MouseScrollPixelsPerLine) -> f32 {
    match event.unit {
        MouseScrollUnit::Line => event.y,
        MouseScrollUnit::Pixel => event.y / pixels_per_line,
    }
}

/// How much the view scale changes for a scroll of `lines`: a step per line,
/// compounded, so many small events add up to the same zoom as one large one.
/// A frame's change is still capped, so one flick cannot jump across the range.
fn zoom_factor(lines: f32) -> f32 {
    (1.0 - CAMERA_ZOOM_STEP).powf(lines).clamp(0.5, 2.0)
}

fn handle_camera_pan(
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
}
