pub mod app;
pub mod board;

use std::time::Duration;

use app::App as GolApp;
use bevy::{
    prelude::*,
    window::{PrimaryWindow, WindowResolution},
};

const CELL_SIZE: f32 = 12.0;
const DEAD_COLOR: Color = Color::srgb(0.08, 0.09, 0.11);
const LIVE_COLOR: Color = Color::srgb(0.22, 0.85, 0.56);
const BG_COLOR: Color = Color::srgb(0.02, 0.02, 0.03);

#[derive(Resource)]
struct Simulation {
    app: GolApp,
    accumulator: Duration,
}

#[derive(Component)]
struct Cell {
    col: usize,
    row: usize,
}

#[derive(Component)]
struct Hud;

#[derive(Component)]
struct HelpText;

#[derive(Component)]
struct MainCamera;

fn main() {
    bevy::prelude::App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Conway's Game of Life (Bevy)".into(),
                resolution: WindowResolution::new(1200, 800),
                resizable: true,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(BG_COLOR))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (
                handle_keyboard,
                handle_mouse,
                advance_simulation,
                handle_resize,
                sync_cells,
                sync_hud,
                apply_quit,
            ),
        )
        .run();
}

fn setup(mut commands: Commands, window_q: Query<&Window, With<PrimaryWindow>>) {
    let Ok(window) = window_q.single() else {
        return;
    };
    let (cols, rows) = board_size_for_window(window);

    commands.spawn((Camera2d, MainCamera));
    commands.insert_resource(Simulation {
        app: GolApp::new(cols, rows),
        accumulator: Duration::ZERO,
    });

    spawn_cells(&mut commands, cols, rows);

    commands.spawn((
        Text::new(""),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.0),
            left: Val::Px(12.0),
            ..default()
        },
        TextFont {
            font_size: 19.0,
            ..default()
        },
        TextColor(Color::srgb(0.95, 0.95, 0.95)),
        Hud,
    ));

    commands.spawn((
        Text::new(controls_text()),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(12.0),
            top: Val::Px(12.0),
            ..default()
        },
        TextFont {
            font_size: 16.0,
            ..default()
        },
        TextColor(Color::srgb(0.80, 0.82, 0.88)),
        Visibility::Hidden,
        HelpText,
    ));
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

fn handle_keyboard(keys: Res<ButtonInput<KeyCode>>, mut sim: ResMut<Simulation>) {
    if keys.just_pressed(KeyCode::Space) {
        sim.app.toggle_running();
    }
    if keys.just_pressed(KeyCode::KeyJ) {
        sim.app.increment_epoch();
    }
    if keys.just_pressed(KeyCode::KeyR) {
        sim.app.randomize_board();
    }
    if keys.just_pressed(KeyCode::KeyX) {
        sim.app.clear_board();
    }
    if keys.just_pressed(KeyCode::Digit0) {
        sim.app.reset_epoch();
    }
    if keys.just_pressed(KeyCode::Equal) || keys.just_pressed(KeyCode::NumpadAdd) {
        sim.app.speed_up();
    }
    if keys.just_pressed(KeyCode::Minus) || keys.just_pressed(KeyCode::NumpadSubtract) {
        sim.app.slow_down();
    }
    if keys.just_pressed(KeyCode::Slash) {
        sim.app.toggle_help();
    }
    if keys.just_pressed(KeyCode::KeyG) {
        sim.app.insert_glider();
    }
    if keys.just_pressed(KeyCode::KeyB) {
        sim.app.insert_blinker();
    }
    if keys.just_pressed(KeyCode::KeyL) {
        sim.app.insert_lwss();
    }
    if keys.just_pressed(KeyCode::KeyQ) || keys.just_pressed(KeyCode::Escape) {
        sim.app.quit();
    }
}

fn handle_mouse(
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    window_q: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut sim: ResMut<Simulation>,
) {
    let Ok(window) = window_q.single() else {
        return;
    };
    let Some(cursor_pos) = window.cursor_position() else {
        return;
    };
    let Ok((camera, camera_transform)) = camera_q.single() else {
        return;
    };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_transform, cursor_pos) else {
        return;
    };

    let (cols, rows) = sim.app.board.size();
    let Some((col, row)) = world_to_cell(world_pos, cols, rows) else {
        return;
    };

    if mouse_buttons.pressed(MouseButton::Left) {
        sim.app.populate_board(col, row);
    }
    if mouse_buttons.pressed(MouseButton::Right) {
        sim.app.depopulate_board(col, row);
    }
}

fn advance_simulation(time: Res<Time>, mut sim: ResMut<Simulation>) {
    if !sim.app.is_running() {
        sim.accumulator = Duration::ZERO;
        return;
    }

    sim.accumulator += time.delta();
    let tick = Duration::from_millis(sim.app.tick_rate_ms());

    while sim.accumulator >= tick {
        sim.app.increment_epoch();
        sim.accumulator -= tick;
    }
}

fn handle_resize(
    mut commands: Commands,
    window_q: Query<&Window, With<PrimaryWindow>>,
    mut sim: ResMut<Simulation>,
    cells_q: Query<Entity, With<Cell>>,
) {
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

fn sync_cells(sim: Res<Simulation>, mut cell_q: Query<(&Cell, &mut Sprite)>) {
    for (cell, mut sprite) in &mut cell_q {
        sprite.color = if sim.app.board.is_alive(cell.col, cell.row) {
            LIVE_COLOR
        } else {
            DEAD_COLOR
        };
    }
}

fn sync_hud(
    sim: Res<Simulation>,
    mut hud_q: Query<&mut Text, (With<Hud>, Without<HelpText>)>,
    mut help_q: Query<&mut Visibility, With<HelpText>>,
) {
    let Ok(mut hud) = hud_q.single_mut() else {
        return;
    };
    let mode = if sim.app.is_running() {
        "Running"
    } else {
        "Paused"
    };
    *hud = Text::new(format!(
        "Epoch: {} | {} | Tick: {}ms",
        sim.app.epoch,
        mode,
        sim.app.tick_rate_ms()
    ));

    let Ok(mut help_visibility) = help_q.single_mut() else {
        return;
    };
    *help_visibility = if sim.app.is_help_visible() {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
}

fn apply_quit(sim: Res<Simulation>) {
    if sim.app.should_quit {
        std::process::exit(0);
    }
}

fn board_size_for_window(window: &Window) -> (usize, usize) {
    let cols = (window.width() / CELL_SIZE).floor() as usize;
    let rows = (window.height() / CELL_SIZE).floor() as usize;
    (cols.max(1), rows.max(1))
}

fn cell_to_world(col: usize, row: usize, cols: usize, rows: usize) -> Vec3 {
    let width = cols as f32 * CELL_SIZE;
    let height = rows as f32 * CELL_SIZE;
    let x = -width / 2.0 + CELL_SIZE * 0.5 + col as f32 * CELL_SIZE;
    let y = -height / 2.0 + CELL_SIZE * 0.5 + row as f32 * CELL_SIZE;
    Vec3::new(x, y, 0.0)
}

fn world_to_cell(world: Vec2, cols: usize, rows: usize) -> Option<(usize, usize)> {
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

fn controls_text() -> &'static str {
    "Controls\n\nSpace: run/pause\nj: step\nr: randomize\nx: clear\n0: reset epoch\n+ / -: speed\ng: glider\nb: blinker\nl: LWSS\n/: toggle help\nLeft drag: draw\nRight drag: erase\nq / Esc: quit"
}
