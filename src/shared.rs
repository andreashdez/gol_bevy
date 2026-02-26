use std::time::Duration;

use crate::app::App as GolApp;
use bevy::{ecs::schedule::SystemSet, prelude::*, window::Window};

pub const CELL_SIZE: f32 = 12.0;
pub const DEAD_COLOR: Color = Color::srgb(0.08, 0.09, 0.11);
pub const LIVE_COLOR: Color = Color::srgb(0.22, 0.85, 0.56);
pub const GHOST_LIVE_COLOR: Color = Color::srgba(0.22, 0.85, 0.56, 0.30);
pub const BG_COLOR: Color = Color::srgb(0.02, 0.02, 0.03);
pub const WALL_COLOR: Color = Color::srgba(0.96, 0.92, 0.70, 0.55);
pub const WALL_BRIGHT_COLOR: Color = Color::srgba(1.0, 0.98, 0.82, 0.92);
pub const HUD_TEXT_COLOR: Color = Color::srgb(0.95, 0.95, 0.95);
pub const HELP_TEXT_COLOR: Color = Color::srgb(0.80, 0.82, 0.88);
pub const GRID_LINE_COLOR: Color = Color::srgba(0.9, 0.9, 0.95, 0.14);
pub const HUD_FONT_SIZE: f32 = 19.0;
pub const HELP_FONT_SIZE: f32 = 16.0;
pub const UI_MARGIN: f32 = 12.0;
pub const CAMERA_ZOOM_STEP: f32 = 0.10;
pub const CAMERA_ZOOM_MIN: f32 = 0.25;
pub const CAMERA_ZOOM_MAX: f32 = 8.0;
pub const CAMERA_PAN_SPEED: f32 = 700.0;
pub const MAX_PATTERN_CELLS: usize = 20;
pub const CONTROLS_TEXT: &str = "Controls\n\nSpace: run/pause\nj: step\nr: randomize\nx: clear\n0: reset epoch\n+ / -: speed\ng: select glider stamp\nb: select blinker stamp\nl: select LWSS stamp\nz: rotate stamp\nf: flip stamp horizontally\nc: clear active stamp\nm: toggle boundary mode\nF3: toggle diagnostics\nt: toggle grid\n/: toggle help\nWheel: zoom camera\nWASD / Arrows: pan camera\nv: reset camera\nLeft drag: draw\nLeft click (stamp): place pattern\nRight drag: erase\nq / Esc: quit";

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub enum UpdateSet {
    Input,
    Simulation,
    Camera,
    Resize,
    Redraw,
    Render,
    Ui,
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
pub struct Hud;

#[derive(Component)]
pub struct HelpText;

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
        assert!(readme.contains(CONTROLS_TEXT));
    }
}
