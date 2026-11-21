//! App-wide Bevy state shared across plugins: the simulation resource, the
//! system sets that order the frame, the overlay toggles, the colour themes,
//! the board size settings, the main camera marker, and whether this is the
//! web build.
use std::time::Duration;

use crate::{
    app::App as GolApp,
    coords::{CAMERA_ZOOM_MAX, CellMap, PLANE_ZOOM_MAX},
};
use bevy::{ecs::schedule::SystemSet, prelude::*};

pub const BG_COLOR: Color = Color::srgb(0.02, 0.02, 0.03);

/// Whether this is the browser build, which has no file system to save
/// settings or workspaces to and no app to quit.
pub const WEB_BUILD: bool = cfg!(target_arch = "wasm32");

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
    /// The plane's cell at the world origin. World space is `f32`, so this moves
    /// with the camera, to keep what's on screen close to the origin however far
    /// the plane has been explored.
    pub plane_origin: (i64, i64),
    /// Set to have the camera frame the board on the next frame.
    pub fit_view: bool,
}

impl Simulation {
    pub fn new(app: GolApp) -> Self {
        Self {
            app,
            accumulator: Duration::ZERO,
            avg_step_ms: 0.0,
            step_samples: 0,
            rate: RateMeter::default(),
            plane_origin: (0, 0),
            fit_view: false,
        }
    }

    pub fn cell_map(&self) -> CellMap {
        match self.app.board.size() {
            Some((cols, rows)) => CellMap::Board { cols, rows },
            None => CellMap::Plane {
                origin: self.plane_origin,
            },
        }
    }

    /// The most a view of this board may be zoomed out.
    pub fn zoom_max(&self) -> f32 {
        if self.app.board.finite().is_some() {
            CAMERA_ZOOM_MAX
        } else {
            PLANE_ZOOM_MAX
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

#[derive(Component)]
pub struct MainCamera;

#[cfg(test)]
mod tests {
    use super::{BOARD_SIZE_MAX, BOARD_SIZE_MIN, BoardSettings};

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
}
