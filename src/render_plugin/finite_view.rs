//! Drawing the finite board's grid, the pulsing edge walls, and resizing the board
use std::f32::consts::PI;

use super::blend_color;
use super::board_texture::{BoardCanvas, TileLayout};
use crate::board::BoundaryMode;
use crate::coords::CELL_SIZE;
use crate::frame_pacing::StayAwake;
use crate::shared::{BoardSettings, GridOverlay, MainCamera, RedrawRequested, Simulation, Theme};
use bevy::prelude::*;

#[derive(Component)]
pub(super) struct BoundaryWall;

#[derive(Resource)]
pub(super) struct BoundaryWallPulse {
    timer: Timer,
    active: bool,
    last_mode: Option<BoundaryMode>,
    pulses_done: u8,
}

impl Default for BoundaryWallPulse {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(0.35, TimerMode::Repeating),
            active: false,
            last_mode: Some(BoundaryMode::Wrap),
            pulses_done: 0,
        }
    }
}

pub(super) fn spawn_boundary_walls(
    commands: &mut Commands,
    cols: usize,
    rows: usize,
    color: Color,
) {
    let width = cols as f32 * CELL_SIZE;
    let height = rows as f32 * CELL_SIZE;
    let wall_thickness = 3.0;
    let offset_x = width / 2.0 + wall_thickness / 2.0;
    let offset_y = height / 2.0 + wall_thickness / 2.0;

    commands.spawn((
        Sprite::from_color(
            color,
            Vec2::new(wall_thickness, height + wall_thickness * 2.0),
        ),
        Transform::from_translation(Vec3::new(-offset_x, 0.0, 3.0)),
        Visibility::Hidden,
        BoundaryWall,
    ));
    commands.spawn((
        Sprite::from_color(
            color,
            Vec2::new(wall_thickness, height + wall_thickness * 2.0),
        ),
        Transform::from_translation(Vec3::new(offset_x, 0.0, 3.0)),
        Visibility::Hidden,
        BoundaryWall,
    ));
    commands.spawn((
        Sprite::from_color(
            color,
            Vec2::new(width + wall_thickness * 2.0, wall_thickness),
        ),
        Transform::from_translation(Vec3::new(0.0, offset_y, 3.0)),
        Visibility::Hidden,
        BoundaryWall,
    ));
    commands.spawn((
        Sprite::from_color(
            color,
            Vec2::new(width + wall_thickness * 2.0, wall_thickness),
        ),
        Transform::from_translation(Vec3::new(0.0, -offset_y, 3.0)),
        Visibility::Hidden,
        BoundaryWall,
    ));
}

/// Window resizes no longer touch the board; only an explicit size change does.
pub(super) fn apply_board_settings(
    settings: Res<BoardSettings>,
    mut sim: ResMut<Simulation>,
    mut canvas: BoardCanvas,
    wall_q: Query<Entity, With<BoundaryWall>>,
) {
    if !settings.is_changed() || !settings.is_set() {
        return;
    }
    let (cols, rows) = (settings.cols, settings.rows);
    if sim.app.board.size() == Some((cols, rows))
        && canvas.texture.layout == (TileLayout { cols, rows })
    {
        return;
    }

    sim.app.resize_board(cols, rows);
    // resize_board already marked the board fully dirty, so the next sync repaints
    // the new texture.
    canvas.rebuild(cols, rows);

    for entity in &wall_q {
        canvas.commands.entity(entity).despawn();
    }
    let wall = canvas.theme.board().wall;
    spawn_boundary_walls(&mut canvas.commands, cols, rows, wall);
}

/// Draws only the grid lines that intersect the viewport, so the cost tracks
/// screen area rather than board area. Spawning a sprite per line would put the
/// per-cell entity problem straight back.
pub(super) fn render_grid(
    grid_overlay: Res<GridOverlay>,
    theme: Res<Theme>,
    sim: Res<Simulation>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut gizmos: Gizmos,
) {
    if !grid_overlay.enabled {
        return;
    }
    let Ok((camera, camera_transform)) = camera_q.single() else {
        return;
    };
    let Some(viewport) = camera.logical_viewport_rect() else {
        return;
    };
    let (Ok(corner_a), Ok(corner_b)) = (
        camera.viewport_to_world_2d(camera_transform, viewport.min),
        camera.viewport_to_world_2d(camera_transform, viewport.max),
    ) else {
        return;
    };

    let visible_min = corner_a.min(corner_b);
    let visible_max = corner_a.max(corner_b);
    let visible_width = (visible_max.x - visible_min.x).max(f32::EPSILON);

    // Below a few pixels per cell the lines are denser than the display can
    // resolve, so the grid reads as noise rather than structure.
    if CELL_SIZE * viewport.width() / visible_width < 3.0 {
        return;
    }

    let color = theme.board().grid_line;
    let Some((cols, rows)) = sim.app.board.size() else {
        // The plane's cells have their corners on multiples of the cell size.
        let first = (visible_min / CELL_SIZE).floor() * CELL_SIZE;
        for line in 0..=((visible_width / CELL_SIZE).ceil() as u32) {
            let x = first.x + line as f32 * CELL_SIZE;
            gizmos.line_2d(
                Vec2::new(x, visible_min.y),
                Vec2::new(x, visible_max.y),
                color,
            );
        }
        let visible_height = visible_max.y - visible_min.y;
        for line in 0..=((visible_height / CELL_SIZE).ceil() as u32) {
            let y = first.y + line as f32 * CELL_SIZE;
            gizmos.line_2d(
                Vec2::new(visible_min.x, y),
                Vec2::new(visible_max.x, y),
                color,
            );
        }
        return;
    };
    if cols == 0 || rows == 0 {
        return;
    }

    let half_width = cols as f32 * CELL_SIZE / 2.0;
    let half_height = rows as f32 * CELL_SIZE / 2.0;

    let x_min = (-half_width).max(visible_min.x);
    let x_max = half_width.min(visible_max.x);
    let y_min = (-half_height).max(visible_min.y);
    let y_max = half_height.min(visible_max.y);
    if x_min > x_max || y_min > y_max {
        return;
    }

    let first_col = line_index(visible_min.x + half_width, cols);
    let last_col = line_index((visible_max.x + half_width).ceil(), cols);
    for col in first_col..=last_col {
        let x = -half_width + col as f32 * CELL_SIZE;
        gizmos.line_2d(Vec2::new(x, y_min), Vec2::new(x, y_max), color);
    }

    let first_row = line_index(visible_min.y + half_height, rows);
    let last_row = line_index((visible_max.y + half_height).ceil(), rows);
    for row in first_row..=last_row {
        let y = -half_height + row as f32 * CELL_SIZE;
        gizmos.line_2d(Vec2::new(x_min, y), Vec2::new(x_max, y), color);
    }
}

/// Board-local offset to a grid line index, clamped to the lines that exist.
fn line_index(offset_from_edge: f32, cells: usize) -> usize {
    ((offset_from_edge / CELL_SIZE).floor().max(0.0) as usize).min(cells)
}

pub(super) fn sync_boundary_wall_visibility(
    time: Res<Time>,
    theme: Res<Theme>,
    redraw_requested: Res<RedrawRequested>,
    sim: Res<Simulation>,
    mut pulse: ResMut<BoundaryWallPulse>,
    mut wall_q: Query<(&mut Visibility, &mut Sprite), With<BoundaryWall>>,
) {
    let mode = sim.app.board.boundary_mode();
    if mode != pulse.last_mode {
        pulse.last_mode = mode;
        pulse.active = mode == Some(BoundaryMode::Fixed);
        if pulse.active {
            pulse.timer.reset();
            pulse.pulses_done = 0;
        }
    }

    if !redraw_requested.0 && !pulse.active && !theme.is_changed() {
        return;
    }
    let colors = theme.board();

    let visibility = if mode == Some(BoundaryMode::Fixed) {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };

    let wall_color = if pulse.active {
        pulse.timer.tick(time.delta());
        if pulse.timer.just_finished() {
            pulse.pulses_done = pulse.pulses_done.saturating_add(1);
        }
        if pulse.pulses_done >= 3 {
            pulse.active = false;
        }

        let phase = pulse.timer.fraction();
        let pulse_t = (phase * PI).sin().max(0.0);
        blend_color(colors.wall, colors.wall_bright, pulse_t)
    } else {
        colors.wall
    };

    for (mut wall_visibility, mut wall_sprite) in &mut wall_q {
        *wall_visibility = visibility;
        wall_sprite.color = wall_color;
    }
}

/// The pulse fades on its own, long after the input that started it.
pub(super) fn keep_awake_while_pulsing(
    pulse: Res<BoundaryWallPulse>,
    mut awake: MessageWriter<StayAwake>,
) {
    if pulse.active {
        awake.write(StayAwake);
    }
}
