//! Keyboard shortcuts, finishing pastes, and the web build's Cmd-release workaround
use crate::{
    feedback::EditFeedback,
    keybindings::{
        BINDING_CLEAR, BINDING_CLEAR_STAMP, BINDING_DESELECT, BINDING_ERASE_SELECTION,
        BINDING_FLIP_STAMP, BINDING_JUMP, BINDING_QUIT, BINDING_RANDOMIZE, BINDING_RESET_EPOCH,
        BINDING_RESET_TO_START, BINDING_ROTATE_STAMP, BINDING_RUN_PAUSE, BINDING_SLOW_DOWN,
        BINDING_SPEED_UP, BINDING_STEP, BINDING_TOGGLE_BOUNDARY_MODE, BINDING_TOGGLE_DIAGNOSTICS,
        BINDING_TOGGLE_GRID, BINDING_TOGGLE_HELP, BINDING_TOGGLE_PALETTE, TOOL_SHORTCUTS,
        TypedKeys,
    },
    shared::{DiagnosticsOverlay, GridOverlay, Simulation, WEB_BUILD},
};
use bevy::{clipboard::Clipboard, prelude::*};

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
pub(super) fn release_keys_held_through_command(mut keys: ResMut<ButtonInput<KeyCode>>) {
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

pub(super) fn handle_keyboard(
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
    if BINDING_JUMP.just_pressed(&keys, &typed) {
        sim.app.request_jump();
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
pub(super) fn finish_paste(mut sim: ResMut<Simulation>, mut feedback: ResMut<EditFeedback>) {
    feedback.finish_paste(&mut sim.app);
}
