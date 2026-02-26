use std::time::Duration;

use crate::shared::{
    Cell, DEAD_COLOR, MainCamera, RedrawRequested, Simulation, UpdateSet, board_size_for_window,
    cell_to_world,
};
use bevy::{
    prelude::*,
    window::{PrimaryWindow, WindowResized},
};

pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_world).add_systems(
            Update,
            (
                handle_resize.in_set(UpdateSet::Resize),
                sync_cells.in_set(UpdateSet::Render),
            ),
        );
    }
}

fn setup_world(mut commands: Commands, window_q: Query<&Window, With<PrimaryWindow>>) {
    let Ok(window) = window_q.single() else {
        return;
    };
    let (cols, rows) = board_size_for_window(window);

    commands.spawn((Camera2d, MainCamera));
    commands.insert_resource(Simulation {
        app: crate::app::App::new(cols, rows),
        accumulator: Duration::ZERO,
        avg_step_ms: 0.0,
        step_samples: 0,
    });

    spawn_cells(&mut commands, cols, rows);
}

fn spawn_cells(commands: &mut Commands, cols: usize, rows: usize) {
    for col in 0..cols {
        for row in 0..rows {
            commands.spawn((
                Sprite::from_color(DEAD_COLOR, Vec2::splat(crate::shared::CELL_SIZE - 1.0)),
                Transform::from_translation(cell_to_world(col, row, cols, rows)),
                Cell { col, row },
            ));
        }
    }
}

fn handle_resize(
    mut resize_events: MessageReader<WindowResized>,
    mut commands: Commands,
    window_q: Query<&Window, With<PrimaryWindow>>,
    mut sim: ResMut<Simulation>,
    cells_q: Query<Entity, With<Cell>>,
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
    spawn_cells(&mut commands, new_cols, new_rows);
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
            crate::shared::LIVE_COLOR
        } else {
            DEAD_COLOR
        };
    }
}
