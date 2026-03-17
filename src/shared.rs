use std::{sync::LazyLock, time::Duration};

use crate::app::App as GolApp;
use bevy::{ecs::schedule::SystemSet, prelude::*, window::Window};

pub const CELL_SIZE: f32 = 12.0;
pub const DEAD_COLOR: Color = Color::srgb(0.08, 0.09, 0.11);
pub const LIVE_COLOR: Color = Color::srgb(0.22, 0.85, 0.56);
pub const GHOST_LIVE_COLOR: Color = Color::srgba(0.22, 0.85, 0.56, 0.30);
pub const BG_COLOR: Color = Color::srgb(0.02, 0.02, 0.03);
pub const WALL_COLOR: Color = Color::srgba(0.96, 0.92, 0.70, 0.55);
pub const WALL_BRIGHT_COLOR: Color = Color::srgba(1.0, 0.98, 0.82, 0.92);
pub const GRID_LINE_COLOR: Color = Color::srgba(0.9, 0.9, 0.95, 0.14);
pub const CAMERA_ZOOM_STEP: f32 = 0.10;
pub const CAMERA_ZOOM_MIN: f32 = 0.25;
pub const CAMERA_ZOOM_MAX: f32 = 8.0;
pub const CAMERA_PAN_SPEED: f32 = 700.0;

#[derive(Clone, Copy)]
pub struct KeyBinding {
    pub keys: &'static [KeyCode],
    pub label: &'static str,
    pub action: &'static str,
}

impl KeyBinding {
    pub fn just_pressed(self, input: &ButtonInput<KeyCode>) -> bool {
        self.keys.iter().copied().any(|key| input.just_pressed(key))
    }

    pub fn pressed(self, input: &ButtonInput<KeyCode>) -> bool {
        self.keys.iter().copied().any(|key| input.pressed(key))
    }
}

pub const BINDING_RUN_PAUSE: KeyBinding = KeyBinding {
    keys: &[KeyCode::Space],
    label: "Space",
    action: "run/pause",
};
pub const BINDING_STEP: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyJ],
    label: "j",
    action: "step",
};
pub const BINDING_RANDOMIZE: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyR],
    label: "r",
    action: "randomize",
};
pub const BINDING_CLEAR: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyX],
    label: "x",
    action: "clear",
};
pub const BINDING_RESET_EPOCH: KeyBinding = KeyBinding {
    keys: &[KeyCode::Digit0],
    label: "0",
    action: "reset epoch",
};
pub const BINDING_SPEED_UP: KeyBinding = KeyBinding {
    keys: &[KeyCode::Equal, KeyCode::NumpadAdd],
    label: "+",
    action: "speed up",
};
pub const BINDING_SLOW_DOWN: KeyBinding = KeyBinding {
    keys: &[KeyCode::Minus, KeyCode::NumpadSubtract],
    label: "-",
    action: "slow down",
};

const SIMULATION_BINDINGS: &[KeyBinding] = &[
    BINDING_RUN_PAUSE,
    BINDING_STEP,
    BINDING_RANDOMIZE,
    BINDING_CLEAR,
    BINDING_RESET_EPOCH,
    BINDING_SPEED_UP,
    BINDING_SLOW_DOWN,
];

pub const BINDING_SELECT_GLIDER: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyG],
    label: "g",
    action: "select glider stamp",
};
pub const BINDING_SELECT_BLINKER: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyB],
    label: "b",
    action: "select blinker stamp",
};
pub const BINDING_SELECT_LWSS: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyL],
    label: "l",
    action: "select LWSS stamp",
};
pub const BINDING_SELECT_RLE_DEMO: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyP],
    label: "p",
    action: "select RLE 8x4 stamp",
};
pub const BINDING_ROTATE_STAMP: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyZ],
    label: "z",
    action: "rotate stamp",
};
pub const BINDING_FLIP_STAMP: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyF],
    label: "f",
    action: "flip stamp horizontally",
};
pub const BINDING_CLEAR_STAMP: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyC],
    label: "c",
    action: "clear active stamp",
};

const STAMP_BINDINGS: &[KeyBinding] = &[
    BINDING_SELECT_GLIDER,
    BINDING_SELECT_BLINKER,
    BINDING_SELECT_LWSS,
    BINDING_SELECT_RLE_DEMO,
    BINDING_ROTATE_STAMP,
    BINDING_FLIP_STAMP,
    BINDING_CLEAR_STAMP,
];

pub const BINDING_TOGGLE_BOUNDARY_MODE: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyM],
    label: "m",
    action: "toggle boundary mode",
};
pub const BINDING_CYCLE_LAYOUT: KeyBinding = KeyBinding {
    keys: &[KeyCode::F2],
    label: "F2",
    action: "cycle layout preset",
};
pub const BINDING_TOGGLE_CONTROL_DECK: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyH],
    label: "h",
    action: "hide/show Control Deck sidebar",
};
pub const BINDING_TOGGLE_DIAGNOSTICS: KeyBinding = KeyBinding {
    keys: &[KeyCode::F3],
    label: "F3",
    action: "toggle diagnostics",
};
pub const BINDING_TOGGLE_GRID: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyT],
    label: "t",
    action: "toggle grid",
};
pub const BINDING_TOGGLE_HELP: KeyBinding = KeyBinding {
    keys: &[KeyCode::Slash],
    label: "/",
    action: "toggle help",
};

const UI_BINDINGS: &[KeyBinding] = &[
    BINDING_TOGGLE_BOUNDARY_MODE,
    BINDING_CYCLE_LAYOUT,
    BINDING_TOGGLE_CONTROL_DECK,
    BINDING_TOGGLE_DIAGNOSTICS,
    BINDING_TOGGLE_GRID,
    BINDING_TOGGLE_HELP,
];

pub const BINDING_PAN_LEFT: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyA, KeyCode::ArrowLeft],
    label: "A / Left",
    action: "pan camera left",
};
pub const BINDING_PAN_RIGHT: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyD, KeyCode::ArrowRight],
    label: "D / Right",
    action: "pan camera right",
};
pub const BINDING_PAN_UP: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyW, KeyCode::ArrowUp],
    label: "W / Up",
    action: "pan camera up",
};
pub const BINDING_PAN_DOWN: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyS, KeyCode::ArrowDown],
    label: "S / Down",
    action: "pan camera down",
};
pub const BINDING_PAN_CAMERA: KeyBinding = KeyBinding {
    keys: &[
        KeyCode::KeyW,
        KeyCode::ArrowUp,
        KeyCode::KeyA,
        KeyCode::ArrowLeft,
        KeyCode::KeyS,
        KeyCode::ArrowDown,
        KeyCode::KeyD,
        KeyCode::ArrowRight,
    ],
    label: "WASD / Arrows",
    action: "pan camera",
};
pub const BINDING_RESET_CAMERA: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyV],
    label: "v",
    action: "reset camera",
};
pub const BINDING_QUIT: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyQ, KeyCode::Escape],
    label: "q / Esc",
    action: "quit",
};

pub static CONTROLS_TEXT: LazyLock<String> = LazyLock::new(build_controls_text);

fn append_binding_lines(out: &mut String, bindings: &[KeyBinding]) {
    for binding in bindings {
        out.push_str(binding.label);
        out.push_str(": ");
        out.push_str(binding.action);
        out.push('\n');
    }
}

fn build_controls_text() -> String {
    let mut out = String::from("Controls\n\n");
    append_binding_lines(&mut out, SIMULATION_BINDINGS);
    append_binding_lines(&mut out, STAMP_BINDINGS);
    append_binding_lines(&mut out, UI_BINDINGS);
    out.push_str("Wheel: zoom camera\n");
    out.push_str(BINDING_PAN_CAMERA.label);
    out.push_str(": ");
    out.push_str(BINDING_PAN_CAMERA.action);
    out.push('\n');
    out.push_str(BINDING_RESET_CAMERA.label);
    out.push_str(": ");
    out.push_str(BINDING_RESET_CAMERA.action);
    out.push('\n');
    out.push_str("Left drag: draw\n");
    out.push_str("Left click (stamp): place pattern\n");
    out.push_str("Right drag: erase\n");
    out.push_str(BINDING_QUIT.label);
    out.push_str(": ");
    out.push_str(BINDING_QUIT.action);
    out.push_str("\n\n");
    out.push_str("Pattern Lab panel:\n");
    out.push_str("- Paste RLE and import as a new stamp\n");
    out.push_str("- Select imported stamp in Active Stamp\n\n");
    out.push_str("Layout presets:\n");
    out.push_str("- Choose Full, Compact, or Minimal from the status bar\n");
    out.push_str("- Press F2 to cycle Full -> Compact -> Minimal\n");
    out.push_str("- Layout preset is saved between runs");
    out
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum UiLayoutPreset {
    #[default]
    Full,
    Compact,
    Minimal,
}

impl UiLayoutPreset {
    pub const ALL: [Self; 3] = [Self::Full, Self::Compact, Self::Minimal];

    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "Full",
            Self::Compact => "Compact",
            Self::Minimal => "Minimal",
        }
    }

    pub fn side_panel_width(self) -> f32 {
        match self {
            Self::Full => 350.0,
            Self::Compact => 280.0,
            Self::Minimal => 0.0,
        }
    }

    pub fn uses_side_panel(self) -> bool {
        !matches!(self, Self::Minimal)
    }

    pub fn show_pattern_lab(self) -> bool {
        matches!(self, Self::Full | Self::Compact)
    }

    pub fn next(self) -> Self {
        match self {
            Self::Full => Self::Compact,
            Self::Compact => Self::Minimal,
            Self::Minimal => Self::Full,
        }
    }

    pub fn as_storage_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Compact => "compact",
            Self::Minimal => "minimal",
        }
    }

    pub fn from_storage_str(value: &str) -> Option<Self> {
        match value {
            "full" => Some(Self::Full),
            "compact" => Some(Self::Compact),
            "minimal" => Some(Self::Minimal),
            _ => None,
        }
    }
}

#[derive(Resource)]
pub struct UiLayout {
    pub preset: UiLayoutPreset,
    pub control_deck_visible: bool,
}

impl Default for UiLayout {
    fn default() -> Self {
        Self {
            preset: UiLayoutPreset::default(),
            control_deck_visible: true,
        }
    }
}

impl UiLayout {
    pub fn cycle(&mut self) {
        self.preset = self.preset.next();
    }

    pub fn toggle_control_deck(&mut self) {
        if self.preset.uses_side_panel() {
            self.control_deck_visible = !self.control_deck_visible;
        }
    }

    pub fn is_control_deck_visible(&self) -> bool {
        self.preset.uses_side_panel() && self.control_deck_visible
    }
}

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub enum UpdateSet {
    Input,
    Simulation,
    Camera,
    Resize,
    Redraw,
    Render,
    Quit,
}

#[derive(Resource)]
pub struct Simulation {
    pub app: GolApp,
    pub accumulator: Duration,
    pub avg_step_ms: f64,
    pub step_samples: u64,
}

#[derive(Resource, Default)]
pub struct RedrawRequested(pub bool);

#[derive(Resource)]
pub struct DiagnosticsOverlay {
    pub enabled: bool,
}

#[derive(Resource)]
pub struct GridOverlay {
    pub enabled: bool,
}

#[derive(Component)]
pub struct Cell {
    pub col: usize,
    pub row: usize,
}

#[derive(Component)]
pub struct MainCamera;

#[derive(Component)]
pub struct GridLine;

pub fn board_size_for_window(window: &Window) -> (usize, usize) {
    let cols = (window.width() / CELL_SIZE).floor() as usize;
    let rows = (window.height() / CELL_SIZE).floor() as usize;
    (cols.max(1), rows.max(1))
}

pub fn cell_to_world(col: usize, row: usize, cols: usize, rows: usize) -> Vec3 {
    let width = cols as f32 * CELL_SIZE;
    let height = rows as f32 * CELL_SIZE;
    let x = -width / 2.0 + CELL_SIZE * 0.5 + col as f32 * CELL_SIZE;
    let y = -height / 2.0 + CELL_SIZE * 0.5 + row as f32 * CELL_SIZE;
    Vec3::new(x, y, 0.0)
}

pub fn world_to_cell(world: Vec2, cols: usize, rows: usize) -> Option<(usize, usize)> {
    let width = cols as f32 * CELL_SIZE;
    let height = rows as f32 * CELL_SIZE;

    let min_x = -width / 2.0;
    let min_y = -height / 2.0;
    let local_x = world.x - min_x;
    let local_y = world.y - min_y;
    if local_x < 0.0 || local_y < 0.0 {
        return None;
    }

    let col = (local_x / CELL_SIZE).floor() as usize;
    let row = (local_y / CELL_SIZE).floor() as usize;
    if col < cols && row < rows {
        return Some((col, row));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::CONTROLS_TEXT;

    #[test]
    fn readme_contains_controls_text() {
        let readme = include_str!("../README.md");
        assert!(readme.contains(CONTROLS_TEXT.as_str()));
    }
}
