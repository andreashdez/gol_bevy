//! Where cells sit in world space, and the camera scales that frame them: the
//! cell size, the zoom and pan limits, fitting a board or pattern to the canvas,
//! and the board size that fills a canvas or window.
use bevy::{prelude::*, window::Window};

pub const CELL_SIZE: f32 = 12.0;
/// On-screen size of a cell, in logical points, when the board is sized to the
/// canvas or window. Smaller than `CELL_SIZE`, so the fitted view starts zoomed out.
pub const FILL_CELL_POINTS: f32 = 4.0;
pub const CAMERA_ZOOM_STEP: f32 = 0.10;
pub const CAMERA_ZOOM_MIN: f32 = 0.25;
// Fit the largest board even in the native canvas at the minimum window size.
pub const CAMERA_ZOOM_MAX: f32 = 160.0;
/// The plane has no size to fit, so its limit is simply far out: a pixel covers
/// some 87,000 cells here, and positions stay accurate to much less than that.
pub const PLANE_ZOOM_MAX: f32 = (1 << 20) as f32;
pub const CAMERA_PAN_SPEED: f32 = 700.0;

/// Where cells sit in world space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CellMap {
    /// Centred on the origin, with nothing past the edges.
    Board { cols: usize, rows: usize },
    /// The cell at `origin` has its corner on the world origin.
    Plane { origin: (i64, i64) },
}

impl CellMap {
    pub fn cell_to_world(self, col: i64, row: i64) -> Vec3 {
        match self {
            Self::Board { cols, rows } => cell_to_world(col, row, cols, rows),
            Self::Plane { origin } => Vec3::new(
                ((col - origin.0) as f32 + 0.5) * CELL_SIZE,
                ((row - origin.1) as f32 + 0.5) * CELL_SIZE,
                0.0,
            ),
        }
    }

    pub fn world_to_cell(self, world: Vec2) -> Option<(i64, i64)> {
        match self {
            Self::Board { cols, rows } => world_to_cell(world, cols, rows),
            Self::Plane { origin } => Some((
                (world.x / CELL_SIZE).floor() as i64 + origin.0,
                (world.y / CELL_SIZE).floor() as i64 + origin.1,
            )),
        }
    }

    /// The cell under `world`, or the nearest one on a board's edge, for a drag
    /// that runs past it.
    pub fn nearest_cell(self, world: Vec2) -> (i64, i64) {
        match self {
            Self::Board { cols, rows } => {
                let corner = cell_to_world(0, 0, cols, rows).truncate() - CELL_SIZE / 2.0;
                let cell = ((world - corner) / CELL_SIZE).floor();
                (
                    (cell.x.max(0.0) as i64).min(cols as i64 - 1),
                    (cell.y.max(0.0) as i64).min(rows as i64 - 1),
                )
            }
            Self::Plane { .. } => self.world_to_cell(world).expect("the plane has every cell"),
        }
    }
}

/// A cell's coordinates as the status bar and screen readers give them: rows
/// are counted downwards, like RLE and other Life tools, from a finite board's
/// top row or from the plane's row 0.
pub fn shown_cell((col, row): (i64, i64), size: Option<(usize, usize)>) -> (i64, i64) {
    match size {
        Some((_, rows)) => (col, rows as i64 - 1 - row),
        None => (col, -row),
    }
}

/// How much more than the board a fitted view shows: just enough to keep the
/// fixed-edge walls, which sit outside the board, on screen.
const FIT_MARGIN: f32 = 1.02;

/// Orthographic scale that frames the whole board in the available canvas, with a margin.
pub fn camera_scale_to_fit_board(size: Vec2, cols: usize, rows: usize) -> f32 {
    scale_to_fit(size, cols as f32, rows as f32).clamp(CAMERA_ZOOM_MIN, CAMERA_ZOOM_MAX)
}

/// Orthographic scale that frames a `width` × `height` pattern on the plane. A
/// small pattern stops at 100% rather than filling the canvas with a few cells.
pub fn camera_scale_to_fit_pattern(size: Vec2, width: f32, height: f32) -> f32 {
    scale_to_fit(size, width, height).clamp(1.0, PLANE_ZOOM_MAX)
}

fn scale_to_fit(size: Vec2, width: f32, height: f32) -> f32 {
    let horizontal = width * CELL_SIZE / size.x.max(1.0);
    let vertical = height * CELL_SIZE / size.y.max(1.0);
    horizontal.max(vertical) * FIT_MARGIN
}

/// The largest board that fits a canvas of `size` at 100%, the fitted zoom.
pub fn board_size_for_canvas(size: Vec2) -> (usize, usize) {
    let cells = (size / (FILL_CELL_POINTS * FIT_MARGIN)).floor();
    (cells.x.max(1.0) as usize, cells.y.max(1.0) as usize)
}

pub fn board_size_for_window(window: &Window) -> (usize, usize) {
    let cols = (window.width() / FILL_CELL_POINTS).floor() as usize;
    let rows = (window.height() / FILL_CELL_POINTS).floor() as usize;
    (cols.max(1), rows.max(1))
}

pub fn cell_to_world(col: i64, row: i64, cols: usize, rows: usize) -> Vec3 {
    let width = cols as f32 * CELL_SIZE;
    let height = rows as f32 * CELL_SIZE;
    let x = -width / 2.0 + CELL_SIZE * 0.5 + col as f32 * CELL_SIZE;
    let y = -height / 2.0 + CELL_SIZE * 0.5 + row as f32 * CELL_SIZE;
    Vec3::new(x, y, 0.0)
}

pub fn world_to_cell(world: Vec2, cols: usize, rows: usize) -> Option<(i64, i64)> {
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
        return Some((col as i64, row as i64));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        CAMERA_ZOOM_MAX, CELL_SIZE, CellMap, PLANE_ZOOM_MAX, Vec2, camera_scale_to_fit_board,
        camera_scale_to_fit_pattern,
    };
    use crate::shared::BOARD_SIZE_MAX;
    use bevy::window::{Window, WindowResolution};

    fn window(width: u32, height: u32) -> Window {
        Window {
            resolution: WindowResolution::new(width, height),
            ..Default::default()
        }
    }

    #[test]
    fn the_plane_maps_cells_to_world_space_around_its_origin() {
        let far = 1 << 50;
        let map = CellMap::Plane {
            origin: (far, -far),
        };
        let world = map.cell_to_world(far + 2, -far - 3).truncate();
        assert_eq!(world, Vec2::new(2.5, -2.5) * CELL_SIZE);
        assert_eq!(map.world_to_cell(world), Some((far + 2, -far - 3)));
        // Cells below the origin round down, not towards it.
        assert_eq!(
            map.world_to_cell(Vec2::splat(-0.5)),
            Some((far - 1, -far - 1))
        );
    }

    #[test]
    fn a_pattern_fit_stays_within_full_size_and_the_zoom_limit() {
        let size = Vec2::new(1000.0, 500.0);
        assert_eq!(camera_scale_to_fit_pattern(size, 3.0, 3.0), 1.0);
        assert_eq!(camera_scale_to_fit_pattern(size, 1e12, 1.0), PLANE_ZOOM_MAX);
        let scale = camera_scale_to_fit_pattern(size, 1000.0, 1.0);
        assert!(scale * size.x >= 1000.0 * CELL_SIZE);
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
}
