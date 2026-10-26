use std::{borrow::Cow, time::Duration};

use crate::app::{App as GolApp, EditTool};
use bevy::{ecs::schedule::SystemSet, prelude::*, window::Window};

pub const CELL_SIZE: f32 = 12.0;
/// On-screen size of a cell, in logical points, when the board is sized to the
/// canvas or window. Smaller than `CELL_SIZE`, so the fitted view starts zoomed out.
pub const FILL_CELL_POINTS: f32 = 4.0;
pub const BG_COLOR: Color = Color::srgb(0.02, 0.02, 0.03);
pub const CAMERA_ZOOM_STEP: f32 = 0.10;
pub const CAMERA_ZOOM_MIN: f32 = 0.25;
// Fit the largest board even in the native canvas at the minimum window size.
pub const CAMERA_ZOOM_MAX: f32 = 160.0;
pub const CAMERA_PAN_SPEED: f32 = 700.0;

#[derive(Clone, Copy)]
pub struct KeyBinding {
    /// Characters matched on the logical key, so the shortcut follows the active
    /// layout (`/` is Shift+7 on a German keyboard). Matched case-insensitively.
    pub chars: &'static [&'static str],
    /// Physical keys, for bindings that depend on position (WASD) or type no
    /// character.
    pub keys: &'static [KeyCode],
    /// Key caps shown in Help and tooltips, one per alternative.
    pub caps: &'static [&'static str],
    pub action: &'static str,
}

impl KeyBinding {
    pub fn just_pressed(self, input: &ButtonInput<KeyCode>, typed: &TypedKeys) -> bool {
        self.keys.iter().copied().any(|key| input.just_pressed(key))
            || self
                .chars
                .iter()
                .any(|character| typed.just_typed(character))
    }

    /// Held state is only tracked for physical keys.
    pub fn pressed(self, input: &ButtonInput<KeyCode>) -> bool {
        self.keys.iter().copied().any(|key| input.pressed(key))
    }
}

/// Characters typed this frame, lowercased. Rebuilt from key events every frame
/// instead of using `ButtonInput<Key>`, whose held state sticks when a modifier
/// changes the character between press and release (Shift+= pressed as `+`,
/// released as `=`), which would swallow the next press.
#[derive(Resource, Default)]
pub struct TypedKeys(Vec<String>);

impl TypedKeys {
    pub fn just_typed(&self, character: &str) -> bool {
        self.0.iter().any(|typed| typed == character)
    }

    pub fn press(&mut self, character: &str) {
        self.0.push(character.to_lowercase());
    }
}

pub fn collect_typed_keys(
    mut events: MessageReader<bevy::input::keyboard::KeyboardInput>,
    mut typed: ResMut<TypedKeys>,
) {
    if !typed.0.is_empty() {
        typed.0.clear();
    }
    for event in events.read() {
        if event.state.is_pressed()
            && !event.repeat
            && let bevy::input::keyboard::Key::Character(character) = &event.logical_key
        {
            typed.press(character);
        }
    }
}

pub const BINDING_RUN_PAUSE: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::Space],
    caps: &["Space"],
    action: "Play / pause",
};
pub const BINDING_STEP: KeyBinding = KeyBinding {
    chars: &["j"],
    keys: &[],
    caps: &["J"],
    action: "Step one generation",
};
pub const BINDING_RANDOMIZE: KeyBinding = KeyBinding {
    chars: &["r"],
    keys: &[],
    caps: &["R"],
    action: "Randomize the board",
};
pub const BINDING_CLEAR: KeyBinding = KeyBinding {
    chars: &["x"],
    keys: &[],
    caps: &["X"],
    action: "Clear the board",
};
pub const BINDING_RESET_EPOCH: KeyBinding = KeyBinding {
    chars: &["0"],
    keys: &[],
    caps: &["0"],
    action: "Reset generation counter",
};
pub const BINDING_SPEED_UP: KeyBinding = KeyBinding {
    // `=` shares the key with `+` on US layouts.
    chars: &["+", "="],
    keys: &[],
    caps: &["+"],
    action: "Speed up",
};
pub const BINDING_SLOW_DOWN: KeyBinding = KeyBinding {
    chars: &["-"],
    keys: &[],
    caps: &["−"],
    action: "Slow down",
};

// Digits accept the physical key too, so AZERTY layouts, which need Shift
// for digits, can use the unshifted key.
pub const BINDING_TOOL_DRAW: KeyBinding = KeyBinding {
    chars: &["1"],
    keys: &[KeyCode::Digit1],
    caps: &["1"],
    action: "Draw tool",
};
pub const BINDING_TOOL_ERASE: KeyBinding = KeyBinding {
    chars: &["2"],
    keys: &[KeyCode::Digit2],
    caps: &["2"],
    action: "Erase tool",
};
pub const BINDING_TOOL_STAMP: KeyBinding = KeyBinding {
    chars: &["3"],
    keys: &[KeyCode::Digit3],
    caps: &["3"],
    action: "Stamp tool",
};
/// Each tool with the shortcut that picks it.
pub const TOOL_SHORTCUTS: [(KeyBinding, EditTool); 3] = [
    (BINDING_TOOL_DRAW, EditTool::Draw),
    (BINDING_TOOL_ERASE, EditTool::Erase),
    (BINDING_TOOL_STAMP, EditTool::Stamp),
];

pub const BINDING_ROTATE_STAMP: KeyBinding = KeyBinding {
    chars: &["z"],
    keys: &[],
    caps: &["Z"],
    action: "Rotate 90° clockwise",
};
pub const BINDING_FLIP_STAMP: KeyBinding = KeyBinding {
    chars: &["f"],
    keys: &[],
    caps: &["F"],
    action: "Flip horizontally",
};
pub const BINDING_CLEAR_STAMP: KeyBinding = KeyBinding {
    chars: &["c"],
    keys: &[],
    caps: &["C"],
    action: "Put the stamp away and draw",
};

pub const BINDING_TOGGLE_BOUNDARY_MODE: KeyBinding = KeyBinding {
    chars: &["m"],
    keys: &[],
    caps: &["M"],
    action: "Wrap / fixed edges",
};
pub const BINDING_TOGGLE_CONTROL_DECK: KeyBinding = KeyBinding {
    chars: &["h"],
    keys: &[],
    caps: &["H"],
    action: "Show / hide the palette",
};
pub const BINDING_TOGGLE_DIAGNOSTICS: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::F3],
    caps: &["F3"],
    action: "Diagnostics",
};
pub const BINDING_TOGGLE_GRID: KeyBinding = KeyBinding {
    chars: &["t"],
    keys: &[],
    caps: &["T"],
    action: "Show / hide the grid",
};
pub const BINDING_TOGGLE_HELP: KeyBinding = KeyBinding {
    chars: &["/"],
    keys: &[],
    caps: &["/"],
    action: "Help",
};

pub const BINDING_PAN_LEFT: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::KeyA, KeyCode::ArrowLeft],
    caps: &["A", "Left"],
    action: "Pan left",
};
pub const BINDING_PAN_RIGHT: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::KeyD, KeyCode::ArrowRight],
    caps: &["D", "Right"],
    action: "Pan right",
};
pub const BINDING_PAN_UP: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::KeyW, KeyCode::ArrowUp],
    caps: &["W", "Up"],
    action: "Pan up",
};
pub const BINDING_PAN_DOWN: KeyBinding = KeyBinding {
    chars: &[],
    keys: &[KeyCode::KeyS, KeyCode::ArrowDown],
    caps: &["S", "Down"],
    action: "Pan down",
};
pub const BINDING_PAN_CAMERA: KeyBinding = KeyBinding {
    chars: &[],
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
    caps: &["WASD", "Arrow keys"],
    action: "Pan",
};
pub const BINDING_RESET_CAMERA: KeyBinding = KeyBinding {
    chars: &["v"],
    keys: &[],
    caps: &["V"],
    action: "Fit board",
};
pub const BINDING_QUIT: KeyBinding = KeyBinding {
    chars: &["q"],
    keys: &[],
    caps: &["Q"],
    action: "Quit",
};

/// A row of the Help dialog: alternative key caps and what they do. `Mod`
/// stands for Cmd on macOS and Ctrl elsewhere.
pub struct HelpEntry {
    pub caps: &'static [&'static str],
    pub action: &'static str,
}

impl From<KeyBinding> for HelpEntry {
    fn from(binding: KeyBinding) -> Self {
        Self {
            caps: binding.caps,
            action: binding.action,
        }
    }
}

pub struct HelpSection {
    pub title: &'static str,
    pub entries: Vec<HelpEntry>,
}

/// Every shortcut and pointer gesture, grouped as the Help dialog shows them.
pub fn help_sections() -> [HelpSection; 5] {
    let entry = |caps, action| HelpEntry { caps, action };
    [
        HelpSection {
            title: "Simulation",
            entries: vec![
                BINDING_RUN_PAUSE.into(),
                BINDING_STEP.into(),
                BINDING_SPEED_UP.into(),
                BINDING_SLOW_DOWN.into(),
                BINDING_TOGGLE_BOUNDARY_MODE.into(),
                BINDING_RESET_EPOCH.into(),
            ],
        },
        HelpSection {
            title: "Editing",
            entries: vec![
                BINDING_TOOL_DRAW.into(),
                BINDING_TOOL_ERASE.into(),
                BINDING_TOOL_STAMP.into(),
                entry(&["Left drag"], "Draw or erase with the tool"),
                entry(&["Right drag"], "Erase"),
                BINDING_RANDOMIZE.into(),
                BINDING_CLEAR.into(),
                entry(&["U", "Mod+Z"], "Undo and pause"),
                entry(&["Mod+Shift+Z"], "Redo"),
            ],
        },
        HelpSection {
            title: "Stamps",
            entries: vec![
                entry(&["Click"], "Place the stamp"),
                BINDING_ROTATE_STAMP.into(),
                BINDING_FLIP_STAMP.into(),
                BINDING_CLEAR_STAMP.into(),
            ],
        },
        HelpSection {
            title: "Camera",
            entries: vec![
                entry(&["Wheel"], "Zoom"),
                entry(&["Middle drag"], "Pan"),
                BINDING_PAN_CAMERA.into(),
                BINDING_RESET_CAMERA.into(),
            ],
        },
        HelpSection {
            title: "Interface",
            entries: vec![
                BINDING_TOGGLE_GRID.into(),
                BINDING_TOGGLE_CONTROL_DECK.into(),
                BINDING_TOGGLE_DIAGNOSTICS.into(),
                BINDING_TOGGLE_HELP.into(),
                entry(&["Mod+S"], "Save workspace"),
                entry(&["Mod+O"], "Open workspace"),
                BINDING_QUIT.into(),
            ],
        },
    ]
}

/// The name of the command modifier on this platform, for `Mod` in key caps.
pub fn command_key() -> &'static str {
    if cfg!(target_os = "macos") {
        "Cmd"
    } else {
        "Ctrl"
    }
}

/// The shortcut list in the README, generated so it cannot drift from the app.
pub fn controls_markdown() -> String {
    let mut out = String::new();
    for section in help_sections() {
        out.push_str(&format!(
            "### {}\n\n| Keys | Action |\n| --- | --- |\n",
            section.title
        ));
        for entry in section.entries {
            let caps = entry
                .caps
                .iter()
                .map(|cap| format!("`{}`", cap.replace("Mod", "Cmd/Ctrl")))
                .collect::<Vec<_>>()
                .join(" or ");
            out.push_str(&format!("| {caps} | {} |\n", entry.action));
        }
        out.push('\n');
    }
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
    pub message: Cow<'static, str>,
    pub remaining: f32,
}

impl EditFeedback {
    /// Seconds a notice stays on screen. Brief, since it covers part of the board.
    const DURATION: f32 = 2.0;

    pub fn show(&mut self, message: impl Into<Cow<'static, str>>) {
        self.message = message.into();
        self.remaining = Self::DURATION;
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

/// Shades live cells by age and leaves a fading trail where cells died.
#[derive(Resource)]
pub struct CellAge {
    pub enabled: bool,
}

impl Default for CellAge {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Colour scheme for the interface and the board.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Theme {
    #[default]
    Dark,
    Light,
    HighContrast,
}

impl Theme {
    pub const ALL: [Theme; 3] = [Theme::Dark, Theme::Light, Theme::HighContrast];

    pub fn label(self) -> &'static str {
        match self {
            Theme::Dark => "Dark",
            Theme::Light => "Light",
            Theme::HighContrast => "High contrast",
        }
    }

    /// Title bar and other window chrome drawn by the OS.
    pub fn window_theme(self) -> bevy::window::WindowTheme {
        match self {
            Theme::Light => bevy::window::WindowTheme::Light,
            Theme::Dark | Theme::HighContrast => bevy::window::WindowTheme::Dark,
        }
    }

    pub fn board(self) -> &'static BoardColors {
        match self {
            Theme::Dark => &DARK_BOARD,
            Theme::Light => &LIGHT_BOARD,
            Theme::HighContrast => &HIGH_CONTRAST_BOARD,
        }
    }
}

pub struct BoardColors {
    pub live: Color,
    pub dead: Color,
    /// Cells born in the latest generation; they settle into `live`.
    pub newborn: Color,
    /// Cells that have lived a long time.
    pub aged: Color,
    /// Cells that just died, fading into `dead`.
    pub trail: Color,
    /// Cells of the stamp under the pointer, before it is placed. A hue of
    /// their own, so a preview never reads as cells already on the board.
    pub ghost: Color,
    /// Outline around the footprint of the stamp under the pointer.
    pub ghost_outline: Color,
    pub wall: Color,
    /// Peak of the pulse when the edges switch to fixed.
    pub wall_bright: Color,
    pub grid_line: Color,
    pub hover: Color,
    pub hover_erase: Color,
}

const DARK_BOARD: BoardColors = BoardColors {
    live: Color::srgb(0.22, 0.85, 0.56),
    dead: Color::srgb(0.08, 0.09, 0.11),
    newborn: Color::srgb(0.78, 1.0, 0.86),
    aged: Color::srgb(0.14, 0.64, 0.58),
    trail: Color::srgb(0.11, 0.18, 0.17),
    ghost: Color::srgba(0.45, 0.70, 1.0, 0.80),
    ghost_outline: Color::srgb(0.58, 0.79, 1.0),
    wall: Color::srgba(0.96, 0.92, 0.70, 0.55),
    wall_bright: Color::srgba(1.0, 0.98, 0.82, 0.92),
    grid_line: Color::srgba(0.02, 0.02, 0.03, 0.8),
    hover: Color::srgb(0.75, 0.95, 0.85),
    hover_erase: Color::srgb(1.0, 0.60, 0.55),
};

const LIGHT_BOARD: BoardColors = BoardColors {
    live: Color::srgb(0.04, 0.50, 0.31),
    dead: Color::srgb(0.985, 0.99, 0.99),
    newborn: Color::srgb(0.08, 0.62, 0.30),
    aged: Color::srgb(0.02, 0.30, 0.30),
    trail: Color::srgb(0.87, 0.945, 0.91),
    ghost: Color::srgba(0.15, 0.40, 0.90, 0.70),
    ghost_outline: Color::srgb(0.10, 0.33, 0.80),
    wall: Color::srgba(0.75, 0.45, 0.0, 0.65),
    wall_bright: Color::srgba(0.90, 0.55, 0.0, 0.95),
    grid_line: Color::srgba(0.70, 0.75, 0.78, 0.8),
    hover: Color::srgb(0.02, 0.30, 0.18),
    hover_erase: Color::srgb(0.75, 0.12, 0.10),
};

const HIGH_CONTRAST_BOARD: BoardColors = BoardColors {
    live: Color::srgb(0.35, 1.0, 0.65),
    dead: Color::srgb(0.10, 0.10, 0.10),
    newborn: Color::srgb(0.90, 1.0, 0.92),
    aged: Color::srgb(0.20, 0.78, 0.82),
    trail: Color::srgb(0.15, 0.24, 0.20),
    ghost: Color::srgba(0.45, 0.80, 1.0, 0.85),
    ghost_outline: Color::srgb(0.65, 0.88, 1.0),
    wall: Color::srgba(1.0, 0.85, 0.0, 0.85),
    wall_bright: Color::srgb(1.0, 0.95, 0.55),
    grid_line: Color::srgba(0.45, 0.45, 0.45, 0.9),
    hover: Color::WHITE,
    hover_erase: Color::srgb(1.0, 0.45, 0.40),
};

pub const BOARD_SIZE_MIN: usize = 8;
pub const BOARD_SIZE_MAX: usize = 2048;
pub const BOARD_SIZE_PRESETS: [(usize, usize); 4] =
    [(128, 128), (256, 256), (512, 512), (1024, 1024)];

/// Board dimensions, held independently of the window. Resizing the window only
/// changes how much of the board is visible; only a change here resizes the board.
/// With `fill_canvas`, the canvas view keeps the size matched to the canvas instead.
#[derive(Resource, Clone, Copy, PartialEq, Eq)]
pub struct BoardSettings {
    pub cols: usize,
    pub rows: usize,
    pub fill_canvas: bool,
}

impl Default for BoardSettings {
    /// Fill canvas is on until a size is chosen, so the board uses the whole canvas.
    fn default() -> Self {
        Self {
            cols: 0,
            rows: 0,
            fill_canvas: true,
        }
    }
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

/// How much more than the board a fitted view shows: just enough to keep the
/// fixed-edge walls, which sit outside the board, on screen.
const FIT_MARGIN: f32 = 1.02;

/// Orthographic scale that frames the whole board in the available canvas, with a margin.
pub fn camera_scale_to_fit_board(size: Vec2, cols: usize, rows: usize) -> f32 {
    let board_width = cols as f32 * CELL_SIZE;
    let board_height = rows as f32 * CELL_SIZE;
    let horizontal = board_width / size.x.max(1.0);
    let vertical = board_height / size.y.max(1.0);
    (horizontal.max(vertical) * FIT_MARGIN).clamp(CAMERA_ZOOM_MIN, CAMERA_ZOOM_MAX)
}

/// The largest board that fits a canvas of `size` at 100%, the fitted zoom.
pub fn board_size_for_canvas(size: Vec2) -> (usize, usize) {
    let cells = (size / (FILL_CELL_POINTS * FIT_MARGIN)).floor();
    (cells.x.max(1.0) as usize, cells.y.max(1.0) as usize)
}

#[derive(Component)]
pub struct MainCamera;

pub fn board_size_for_window(window: &Window) -> (usize, usize) {
    let cols = (window.width() / FILL_CELL_POINTS).floor() as usize;
    let rows = (window.height() / FILL_CELL_POINTS).floor() as usize;
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
        BOARD_SIZE_MAX, BOARD_SIZE_MIN, BoardSettings, CAMERA_ZOOM_MAX, CELL_SIZE,
        camera_scale_to_fit_board, controls_markdown,
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
        assert!(
            settings.fill_canvas,
            "the board fills the canvas until a size is chosen"
        );
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
    fn readme_lists_every_shortcut_as_help_shows_it() {
        let readme = include_str!("../README.md");
        assert!(
            readme.contains(&controls_markdown()),
            "update the README's controls to:\n{}",
            controls_markdown()
        );
    }
}
