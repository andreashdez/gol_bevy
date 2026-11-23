mod camera;
mod cursor;
mod pointer;
mod shortcuts;
#[cfg(test)]
mod tests;

use crate::{
    keybindings::{TypedKeys, collect_typed_keys},
    shared::{UpdateSet, WEB_BUILD},
    ui_input::canvas_keyboard_available,
};
use bevy::prelude::*;
use camera::{handle_camera_drag, handle_camera_pan, handle_camera_reset, handle_camera_zoom};
pub use cursor::BoardCursor;
use cursor::handle_board_cursor;
use pointer::{finish_mouse_stroke, handle_mouse};
pub use shortcuts::COMMAND_KEYS;
use shortcuts::{finish_paste, handle_keyboard, release_keys_held_through_command};
#[cfg(test)]
use {
    crate::{
        app::EditTool,
        coords::CAMERA_ZOOM_STEP,
        feedback::EditFeedback,
        keybindings::BINDING_PAN_LEFT,
        shared::{DiagnosticsOverlay, GridOverlay, MainCamera, Simulation},
        sparse_board::CellRect,
        ui_input::UiInputCapture,
    },
    bevy::{
        input::{
            ButtonState,
            keyboard::KeyboardInput,
            mouse::{MouseScrollPixelsPerLine, MouseScrollUnit, MouseWheel},
        },
        window::PrimaryWindow,
    },
    camera::{scroll_lines, zoom_factor},
    pointer::stroke_cells,
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
