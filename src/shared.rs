use std::{sync::LazyLock, time::Duration};

use crate::app::{App as GolApp, PatternKind};
use bevy::{ecs::schedule::SystemSet, prelude::*, window::Window};

pub const CELL_SIZE: f32 = 12.0;
pub const DEAD_COLOR: Color = Color::srgb(0.08, 0.09, 0.11);
pub const LIVE_COLOR: Color = Color::srgb(0.22, 0.85, 0.56);
pub const GHOST_LIVE_COLOR: Color = Color::srgba(0.22, 0.85, 0.56, 0.30);
pub const BG_COLOR: Color = Color::srgb(0.02, 0.02, 0.03);
pub const WALL_COLOR: Color = Color::srgba(0.96, 0.92, 0.70, 0.55);
pub const WALL_BRIGHT_COLOR: Color = Color::srgba(1.0, 0.98, 0.82, 0.92);
pub const GRID_LINE_COLOR: Color = Color::srgba(0.02, 0.02, 0.03, 0.8);
pub const CAMERA_ZOOM_STEP: f32 = 0.10;
pub const CAMERA_ZOOM_MIN: f32 = 0.25;
// Fit the largest board even in the native canvas at the minimum window size.
pub const CAMERA_ZOOM_MAX: f32 = 160.0;
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
/// Pairs each pattern shortcut with the built-in it selects, for the keyboard
/// handler and the palette tooltips alike.
pub const PATTERN_SHORTCUTS: [(KeyBinding, PatternKind); 4] = [
    (BINDING_SELECT_GLIDER, PatternKind::Glider),
    (BINDING_SELECT_BLINKER, PatternKind::Blinker),
    (BINDING_SELECT_LWSS, PatternKind::Lwss),
    (BINDING_SELECT_RLE_DEMO, PatternKind::RleDemo),
];

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
pub const BINDING_TOGGLE_CONTROL_DECK: KeyBinding = KeyBinding {
    keys: &[KeyCode::KeyH],
    label: "h",
    action: "hide/show pattern sidebar",
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
    out.push_str("Middle drag: pan camera\n");
    out.push_str(BINDING_PAN_CAMERA.label);
    out.push_str(": ");
    out.push_str(BINDING_PAN_CAMERA.action);
    out.push('\n');
    out.push_str(BINDING_RESET_CAMERA.label);
    out.push_str(": ");
    out.push_str(BINDING_RESET_CAMERA.action);
    out.push('\n');
    out.push_str("Left drag: draw or erase with the selected tool\n");
    out.push_str("Left click (stamp): place pattern\n");
    out.push_str("Right drag: erase\n");
    out.push_str("U or Cmd/Ctrl+Z: undo last edit and pause\n");
    out.push_str("Cmd/Ctrl+Shift+Z: redo last undone edit\n");
    out.push_str(BINDING_QUIT.label);
    out.push_str(": ");
    out.push_str(BINDING_QUIT.action);
    out.push_str("\n\n");
    out.push_str("Pattern palette:\n");
    out.push_str("- Use Import / Export to paste RLE as a stamp or copy the board as RLE\n");
    out.push_str("- Select a pattern thumbnail, then click the board to place it");
    out
}

#[derive(Resource)]
pub struct UiLayout {
    pub control_deck_visible: bool,
}

impl Default for UiLayout {
    fn default() -> Self {
        Self {
            control_deck_visible: true,
        }
    }
}

impl UiLayout {
    pub fn toggle_control_deck(&mut self) {
        self.control_deck_visible = !self.control_deck_visible;
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
    pub rate: RateMeter,
}

impl Simulation {
    pub fn new(app: GolApp) -> Self {
        Self {
            app,
            accumulator: Duration::ZERO,
            avg_step_ms: 0.0,
            step_samples: 0,
            rate: RateMeter::default(),
        }
    }
}

/// Generations actually achieved per second, averaged over a short window so a
/// frame that runs no step and the next that runs two read as one steady rate.
#[derive(Debug, Default)]
pub struct RateMeter {
    window: Duration,
    window_steps: u64,
    per_second: f64,
}

impl RateMeter {
    const WINDOW: Duration = Duration::from_millis(500);

    pub fn record(&mut self, steps: u64, elapsed: Duration) {
        self.window += elapsed;
        self.window_steps += steps;
        if self.window >= Self::WINDOW {
            self.per_second = self.window_steps as f64 / self.window.as_secs_f64();
            self.window = Duration::ZERO;
            self.window_steps = 0;
        }
    }

    pub fn per_second(&self) -> f64 {
        self.per_second
    }
}

#[derive(Resource, Default)]
pub struct RedrawRequested(pub bool);

/// Short, non-modal feedback shared by toolbar and keyboard edits.
#[derive(Resource, Default)]
pub struct EditFeedback {
    pub message: &'static str,
    pub remaining: f32,
}

impl EditFeedback {
    pub fn show(&mut self, message: &'static str) {
        self.message = message;
        self.remaining = 4.0;
    }

    pub fn undo(&mut self, app: &mut GolApp) {
        self.show(if app.undo() {
            "Edit undone"
        } else {
            "Nothing to undo"
        });
    }

    pub fn redo(&mut self, app: &mut GolApp) {
        self.show(if app.redo() {
            "Edit redone · U to undo"
        } else {
            "Nothing to redo"
        });
    }

    pub fn clear(&mut self, app: &mut GolApp) {
        if app.board.live_cells_count() == 0 {
            self.show("Board is already empty");
        } else {
            app.clear_board();
            self.show("Board cleared · U to undo");
        }
    }

    pub fn randomize(&mut self, app: &mut GolApp) {
        app.randomize_board();
        self.show("Board randomized · U to undo");
    }
}

#[derive(Resource)]
pub struct DiagnosticsOverlay {
    pub enabled: bool,
}

#[derive(Resource)]
pub struct GridOverlay {
    pub enabled: bool,
}

pub const BOARD_SIZE_MIN: usize = 8;
pub const BOARD_SIZE_MAX: usize = 2048;
pub const BOARD_SIZE_PRESETS: [(usize, usize); 4] =
    [(128, 128), (256, 256), (512, 512), (1024, 1024)];

/// Board dimensions, held independently of the window. Resizing the window only
/// changes how much of the board is visible; only a change here resizes the board.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub struct BoardSettings {
    pub cols: usize,
    pub rows: usize,
}

impl BoardSettings {
    /// Zero means "not chosen yet"; startup fills it from persistence or the window.
    pub fn is_set(self) -> bool {
        self.cols > 0 && self.rows > 0
    }

    pub fn set(&mut self, cols: usize, rows: usize) {
        self.cols = cols.clamp(BOARD_SIZE_MIN, BOARD_SIZE_MAX);
        self.rows = rows.clamp(BOARD_SIZE_MIN, BOARD_SIZE_MAX);
    }
}

/// Persisted settings must land before the world is built from them.
#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub enum StartupSet {
    LoadSettings,
    BuildWorld,
}

/// Orthographic scale that frames the whole board in the available canvas, with a margin.
pub fn camera_scale_to_fit_board(size: Vec2, cols: usize, rows: usize) -> f32 {
    let board_width = cols as f32 * CELL_SIZE;
    let board_height = rows as f32 * CELL_SIZE;
    let horizontal = board_width / size.x.max(1.0);
    let vertical = board_height / size.y.max(1.0);
    (horizontal.max(vertical) * 1.05).clamp(CAMERA_ZOOM_MIN, CAMERA_ZOOM_MAX)
}

#[derive(Component)]
pub struct MainCamera;

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
    use super::{
        BOARD_SIZE_MAX, BOARD_SIZE_MIN, BoardSettings, CAMERA_ZOOM_MAX, CELL_SIZE, CONTROLS_TEXT,
        camera_scale_to_fit_board,
    };
    use bevy::window::{Window, WindowResolution};

    fn window(width: u32, height: u32) -> Window {
        Window {
            resolution: WindowResolution::new(width, height),
            ..Default::default()
        }
    }

    #[test]
    fn board_settings_start_unset_and_clamp_to_supported_sizes() {
        let mut settings = BoardSettings::default();
        assert!(!settings.is_set());

        settings.set(256, 128);
        assert!(settings.is_set());
        assert_eq!((settings.cols, settings.rows), (256, 128));

        settings.set(0, 999_999);
        assert_eq!(
            (settings.cols, settings.rows),
            (BOARD_SIZE_MIN, BOARD_SIZE_MAX)
        );
    }

    #[test]
    fn camera_fit_frames_the_whole_board() {
        let window = window(1200, 800);

        // A board smaller than the window still fits at any scale, so the margin
        // keeps it just inside the viewport.
        let small = camera_scale_to_fit_board(window.size(), 10, 10);
        assert!(small * 1200.0 >= 10.0 * CELL_SIZE, "small board must fit");

        // A board far wider than the window must zoom out enough to show it all.
        let large = camera_scale_to_fit_board(window.size(), 1024, 1024);
        assert!(
            large * 800.0 >= 1024.0 * CELL_SIZE,
            "1024 rows must fit vertically"
        );
        assert!(large <= CAMERA_ZOOM_MAX);

        // The limiting axis wins: a wide, short board is framed by its width.
        let wide = camera_scale_to_fit_board(window.size(), 1024, 16);
        assert!(wide * 1200.0 >= 1024.0 * CELL_SIZE);
    }

    #[test]
    fn largest_supported_board_still_fits_within_the_zoom_ceiling() {
        let window = window(1200, 800);
        let scale = camera_scale_to_fit_board(window.size(), BOARD_SIZE_MAX, BOARD_SIZE_MAX);
        assert!(scale < CAMERA_ZOOM_MAX, "zoom ceiling is too low: {scale}");
        assert!(scale * 800.0 >= BOARD_SIZE_MAX as f32 * CELL_SIZE);
    }

    #[test]
    fn rate_meter_averages_steps_over_its_window() {
        use super::RateMeter;
        use std::time::Duration;

        let mut meter = RateMeter::default();
        // Alternating frames of zero and two steps read as one steady rate:
        // 50 frames of 10 ms fill one window with 50 generations.
        for frame in 0..50 {
            meter.record(
                if frame % 2 == 0 { 0 } else { 2 },
                Duration::from_millis(10),
            );
        }
        assert!((meter.per_second() - 100.0).abs() < 1e-9);

        // A paused stretch decays the reading to zero once a window elapses.
        meter.record(0, Duration::from_secs(1));
        assert_eq!(meter.per_second(), 0.0);
    }

    #[test]
    fn readme_contains_controls_text() {
        let readme = include_str!("../README.md");
        assert!(readme.contains(CONTROLS_TEXT.as_str()));
    }

    #[test]
    fn every_pattern_shortcut_selects_its_own_builtin() {
        use super::PATTERN_SHORTCUTS;
        use crate::patterns::Pattern;

        let builtins = Pattern::builtins();
        let expected = ["Glider", "Blinker", "LWSS", "RLE 8x4"];
        for ((binding, pattern), name) in PATTERN_SHORTCUTS.into_iter().zip(expected) {
            let index = pattern
                .builtin_index()
                .unwrap_or_else(|| panic!("{pattern:?} has no file in assets/patterns"));
            assert_eq!(builtins[index].name, name, "key {}", binding.label);
        }
    }
}
