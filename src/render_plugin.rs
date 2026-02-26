use std::time::Duration;

use crate::app::App as GolApp;
use crate::shared::{
    CELL_SIZE, Cell, DEAD_COLOR, GHOST_LIVE_COLOR, GRID_LINE_COLOR, GridLine, GridOverlay,
    LIVE_COLOR, MAX_PATTERN_CELLS, MainCamera, RedrawRequested, Simulation, UpdateSet,
    board_size_for_window, cell_to_world, world_to_cell,
};
use bevy::{
    prelude::*,
    window::{PrimaryWindow, WindowResized},
};

pub struct RenderPlugin;

#[derive(Component)]
struct GhostPreviewCell {
    slot: usize,
}

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_world).add_systems(
            Update,
            (
                handle_resize.in_set(UpdateSet::Resize),
                sync_cells.in_set(UpdateSet::Render),
                sync_grid_visibility.in_set(UpdateSet::Render),
                sync_pattern_preview.in_set(UpdateSet::Render),
            ),
        );
    }
}

fn setup_world(
    mut commands: Commands,
    window_q: Query<&Window, With<PrimaryWindow>>,
    grid_overlay: Res<GridOverlay>,
) {
    let Ok(window) = window_q.single() else {
        return;
    };
    let (cols, rows) = board_size_for_window(window);

    commands.spawn((Camera2d, MainCamera));
    commands.insert_resource(Simulation {
        app: GolApp::new(cols, rows),
        accumulator: Duration::ZERO,
        avg_step_ms: 0.0,
        step_samples: 0,
    });

    spawn_cells(&mut commands, cols, rows);
    spawn_grid_lines(&mut commands, cols, rows, grid_overlay.enabled);
    spawn_pattern_preview_cells(&mut commands);
}

fn spawn_cells(commands: &mut Commands, cols: usize, rows: usize) {
    for col in 0..cols {
        for row in 0..rows {
            commands.spawn((
                Sprite::from_color(DEAD_COLOR, Vec2::splat(CELL_SIZE - 1.0)),
                Transform::from_translation(cell_to_world(col, row, cols, rows)),
                Cell { col, row },
            ));
        }
    }
}

fn spawn_grid_lines(commands: &mut Commands, cols: usize, rows: usize, is_visible: bool) {
    let visibility = if is_visible {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    let width = cols as f32 * CELL_SIZE;
    let height = rows as f32 * CELL_SIZE;

    for x in 0..=cols {
        let x_pos = -width / 2.0 + x as f32 * CELL_SIZE;
        commands.spawn((
            Sprite::from_color(GRID_LINE_COLOR, Vec2::new(1.0, height)),
            Transform::from_translation(Vec3::new(x_pos, 0.0, 1.0)),
            visibility,
            GridLine,
        ));
    }

    for y in 0..=rows {
        let y_pos = -height / 2.0 + y as f32 * CELL_SIZE;
        commands.spawn((
            Sprite::from_color(GRID_LINE_COLOR, Vec2::new(width, 1.0)),
            Transform::from_translation(Vec3::new(0.0, y_pos, 1.0)),
            visibility,
            GridLine,
        ));
    }
}

fn spawn_pattern_preview_cells(commands: &mut Commands) {
    for slot in 0..MAX_PATTERN_CELLS {
        commands.spawn((
            Sprite::from_color(GHOST_LIVE_COLOR, Vec2::splat(CELL_SIZE - 2.0)),
            Transform::from_translation(Vec3::new(0.0, 0.0, 2.0)),
            Visibility::Hidden,
            GhostPreviewCell { slot },
        ));
    }
}

fn handle_resize(
    mut resize_events: MessageReader<WindowResized>,
    mut commands: Commands,
    window_q: Query<&Window, With<PrimaryWindow>>,
    mut sim: ResMut<Simulation>,
    cells_q: Query<Entity, With<Cell>>,
    grid_q: Query<Entity, With<GridLine>>,
    grid_overlay: Res<GridOverlay>,
) {
    let mut saw_resize = false;
    for _ in resize_events.read() {
        saw_resize = true;
    }
    if !saw_resize {
        return;
    }

    let Ok(window) = window_q.single() else {
        return;
    };
    let (new_cols, new_rows) = board_size_for_window(window);
    let (cols, rows) = sim.app.board.size();
    if cols == new_cols && rows == new_rows {
        return;
    }

    sim.app.resize_board(new_cols, new_rows);

    for entity in &cells_q {
        commands.entity(entity).despawn();
    }
    for entity in &grid_q {
        commands.entity(entity).despawn();
    }
    spawn_cells(&mut commands, new_cols, new_rows);
    spawn_grid_lines(&mut commands, new_cols, new_rows, grid_overlay.enabled);
}

fn sync_cells(
    redraw_requested: Res<RedrawRequested>,
    sim: Res<Simulation>,
    mut cell_q: Query<(&Cell, &mut Sprite)>,
) {
    if !redraw_requested.0 {
        return;
    }

    for (cell, mut sprite) in &mut cell_q {
        sprite.color = if sim.app.board.is_alive(cell.col, cell.row) {
            LIVE_COLOR
        } else {
            DEAD_COLOR
        };
    }
}

fn sync_grid_visibility(
    grid_overlay: Res<GridOverlay>,
    mut grid_q: Query<&mut Visibility, With<GridLine>>,
) {
    if !grid_overlay.is_changed() {
        return;
    }

    let visibility = if grid_overlay.enabled {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
    for mut line_visibility in &mut grid_q {
        *line_visibility = visibility;
    }
}

fn sync_pattern_preview(
    window_q: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    sim: Res<Simulation>,
    mut preview_q: Query<(&GhostPreviewCell, &mut Transform, &mut Visibility)>,
) {
    let Some((pattern, width, height)) = sim.app.selected_pattern_layout() else {
        hide_preview_cells(&mut preview_q);
        return;
    };

    let Ok(window) = window_q.single() else {
        hide_preview_cells(&mut preview_q);
        return;
    };
    let Some(cursor_pos) = window.cursor_position() else {
        hide_preview_cells(&mut preview_q);
        return;
    };
    let Ok((camera, camera_transform)) = camera_q.single() else {
        hide_preview_cells(&mut preview_q);
        return;
    };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) else {
        hide_preview_cells(&mut preview_q);
        return;
    };

    let (cols, rows) = sim.app.board.size();
    let Some((cursor_col, cursor_row)) = world_to_cell(world_pos, cols, rows) else {
        hide_preview_cells(&mut preview_q);
        return;
    };

    let origin_col = cursor_col.saturating_sub(width / 2);
    let origin_row = cursor_row.saturating_sub(height / 2);

    for (ghost, mut transform, mut visibility) in &mut preview_q {
        let Some(&(dx, dy)) = pattern.get(ghost.slot) else {
            *visibility = Visibility::Hidden;
            continue;
        };
        let col = origin_col + dx;
        let row = origin_row + dy;
        if col >= cols || row >= rows {
            *visibility = Visibility::Hidden;
            continue;
        }

        *transform =
            Transform::from_translation(cell_to_world(col, row, cols, rows) + Vec3::Z * 2.0);
        *visibility = Visibility::Visible;
    }
}

fn hide_preview_cells(preview_q: &mut Query<(&GhostPreviewCell, &mut Transform, &mut Visibility)>) {
    for (_, _, mut visibility) in preview_q {
        *visibility = Visibility::Hidden;
    }
}
