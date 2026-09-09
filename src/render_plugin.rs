use std::{f32::consts::PI, time::Duration};

use crate::app::{App as GolApp, EditTool};
use crate::board::{BoundaryMode, DirtyCells};
use crate::shared::UiPointerCapture;
use crate::shared::{
    CELL_SIZE, Cell, DEAD_COLOR, GHOST_LIVE_COLOR, GRID_LINE_COLOR, GridLine, GridOverlay,
    LIVE_COLOR, MainCamera, RedrawRequested, Simulation, UpdateSet, WALL_BRIGHT_COLOR, WALL_COLOR,
    board_size_for_window, cell_to_world, world_to_cell,
};
use bevy::{
    prelude::*,
    window::{PrimaryWindow, WindowResized},
};
use bevy_egui::input::EguiWantsInput;

pub struct RenderPlugin;

type BoardEntityFilter = Or<(With<Cell>, With<GridLine>, With<BoundaryWall>)>;

#[derive(Resource, Default)]
struct HoveredCell(Option<(usize, usize)>);

#[derive(Component)]
struct GhostPreviewCell;

#[derive(Component)]
struct BoundaryWall;

#[derive(Resource, Default)]
struct CellEntities {
    cols: usize,
    entities: Vec<Entity>,
}

impl CellEntities {
    fn replace(&mut self, cols: usize, entities: Vec<Entity>) {
        self.cols = cols;
        self.entities = entities;
    }

    fn coords_for_index(&self, idx: usize) -> Option<(usize, usize)> {
        if self.cols == 0 || idx >= self.entities.len() {
            return None;
        }
        Some((idx % self.cols, idx / self.cols))
    }
}

#[derive(Resource, Default)]
struct PatternPreviewPool {
    entities: Vec<Entity>,
}

#[derive(Resource)]
struct BoundaryWallPulse {
    timer: Timer,
    active: bool,
    last_mode: BoundaryMode,
    pulses_done: u8,
}

impl Default for BoundaryWallPulse {
    fn default() -> Self {
        Self {
            timer: Timer::from_seconds(0.35, TimerMode::Repeating),
            active: false,
            last_mode: BoundaryMode::Wrap,
            pulses_done: 0,
        }
    }
}

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HoveredCell>()
            .insert_resource(BoundaryWallPulse::default())
            .insert_resource(CellEntities::default())
            .insert_resource(PatternPreviewPool::default())
            .add_systems(Startup, setup_world)
            .add_systems(
                Update,
                (
                    handle_resize.in_set(UpdateSet::Resize),
                    sync_cells.in_set(UpdateSet::Render),
                    sync_grid_visibility.in_set(UpdateSet::Render),
                    sync_boundary_wall_visibility.in_set(UpdateSet::Render),
                    update_hovered_cell
                        .in_set(UpdateSet::Render)
                        .before(sync_pattern_preview)
                        .before(render_hover_cell),
                    sync_pattern_preview.in_set(UpdateSet::Render),
                    render_hover_cell.in_set(UpdateSet::Render),
                ),
            );
    }
}

fn setup_world(
    mut commands: Commands,
    window_q: Query<&Window, With<PrimaryWindow>>,
    grid_overlay: Res<GridOverlay>,
    mut cell_entities: ResMut<CellEntities>,
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

    let cells = spawn_cells(&mut commands, cols, rows);
    cell_entities.replace(cols, cells);
    spawn_grid_lines(&mut commands, cols, rows, grid_overlay.enabled);
    spawn_boundary_walls(&mut commands, cols, rows);
}

fn spawn_cells(commands: &mut Commands, cols: usize, rows: usize) -> Vec<Entity> {
    let mut entities = Vec::with_capacity(cols.saturating_mul(rows));
    for row in 0..rows {
        for col in 0..cols {
            let entity = commands
                .spawn((
                    Sprite::from_color(DEAD_COLOR, Vec2::splat(CELL_SIZE - 1.0)),
                    Transform::from_translation(cell_to_world(col, row, cols, rows)),
                    Cell { col, row },
                ))
                .id();
            entities.push(entity);
        }
    }
    entities
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

fn spawn_boundary_walls(commands: &mut Commands, cols: usize, rows: usize) {
    let width = cols as f32 * CELL_SIZE;
    let height = rows as f32 * CELL_SIZE;
    let wall_thickness = 3.0;
    let offset_x = width / 2.0 + wall_thickness / 2.0;
    let offset_y = height / 2.0 + wall_thickness / 2.0;

    commands.spawn((
        Sprite::from_color(
            WALL_COLOR,
            Vec2::new(wall_thickness, height + wall_thickness * 2.0),
        ),
        Transform::from_translation(Vec3::new(-offset_x, 0.0, 3.0)),
        Visibility::Hidden,
        BoundaryWall,
    ));
    commands.spawn((
        Sprite::from_color(
            WALL_COLOR,
            Vec2::new(wall_thickness, height + wall_thickness * 2.0),
        ),
        Transform::from_translation(Vec3::new(offset_x, 0.0, 3.0)),
        Visibility::Hidden,
        BoundaryWall,
    ));
    commands.spawn((
        Sprite::from_color(
            WALL_COLOR,
            Vec2::new(width + wall_thickness * 2.0, wall_thickness),
        ),
        Transform::from_translation(Vec3::new(0.0, offset_y, 3.0)),
        Visibility::Hidden,
        BoundaryWall,
    ));
    commands.spawn((
        Sprite::from_color(
            WALL_COLOR,
            Vec2::new(width + wall_thickness * 2.0, wall_thickness),
        ),
        Transform::from_translation(Vec3::new(0.0, -offset_y, 3.0)),
        Visibility::Hidden,
        BoundaryWall,
    ));
}

fn ensure_preview_pool(commands: &mut Commands, pool: &mut PatternPreviewPool, target: usize) {
    while pool.entities.len() < target {
        let entity = commands
            .spawn((
                Sprite::from_color(GHOST_LIVE_COLOR, Vec2::splat(CELL_SIZE - 2.0)),
                Transform::from_translation(Vec3::new(0.0, 0.0, 2.0)),
                Visibility::Hidden,
                GhostPreviewCell,
            ))
            .id();
        pool.entities.push(entity);
    }
}

fn handle_resize(
    mut resize_events: MessageReader<WindowResized>,
    mut commands: Commands,
    window_q: Query<&Window, With<PrimaryWindow>>,
    mut sim: ResMut<Simulation>,
    mut cell_entities: ResMut<CellEntities>,
    board_entities_q: Query<Entity, BoardEntityFilter>,
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

    for entity in &board_entities_q {
        commands.entity(entity).despawn();
    }
    let cells = spawn_cells(&mut commands, new_cols, new_rows);
    cell_entities.replace(new_cols, cells);
    spawn_grid_lines(&mut commands, new_cols, new_rows, grid_overlay.enabled);
    spawn_boundary_walls(&mut commands, new_cols, new_rows);
}

fn sync_cells(
    redraw_requested: Res<RedrawRequested>,
    mut sim: ResMut<Simulation>,
    cell_entities: Res<CellEntities>,
    mut cell_q: Query<&mut Sprite, With<Cell>>,
) {
    if !redraw_requested.0 {
        return;
    }

    let dirty = sim.app.board.take_dirty_cells();
    match dirty {
        DirtyCells::None => {}
        DirtyCells::Full => {
            for idx in 0..cell_entities.entities.len() {
                let Some((col, row)) = cell_entities.coords_for_index(idx) else {
                    continue;
                };
                let Some(entity) = cell_entities.entities.get(idx).copied() else {
                    continue;
                };
                let Ok(mut sprite) = cell_q.get_mut(entity) else {
                    continue;
                };
                sprite.color = if sim.app.board.is_alive(col, row) {
                    LIVE_COLOR
                } else {
                    DEAD_COLOR
                };
            }
        }
        DirtyCells::Cells(indices) => {
            for idx in indices {
                let Some((col, row)) = cell_entities.coords_for_index(idx) else {
                    continue;
                };
                let Some(entity) = cell_entities.entities.get(idx).copied() else {
                    continue;
                };
                let Ok(mut sprite) = cell_q.get_mut(entity) else {
                    continue;
                };
                sprite.color = if sim.app.board.is_alive(col, row) {
                    LIVE_COLOR
                } else {
                    DEAD_COLOR
                };
            }
        }
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

fn sync_boundary_wall_visibility(
    time: Res<Time>,
    redraw_requested: Res<RedrawRequested>,
    sim: Res<Simulation>,
    mut pulse: ResMut<BoundaryWallPulse>,
    mut wall_q: Query<(&mut Visibility, &mut Sprite), With<BoundaryWall>>,
) {
    let mode = sim.app.board.boundary_mode();
    if mode != pulse.last_mode {
        pulse.last_mode = mode;
        pulse.active = mode == BoundaryMode::Fixed;
        if pulse.active {
            pulse.timer.reset();
            pulse.pulses_done = 0;
        }
    }

    if !redraw_requested.0 && !pulse.active {
        return;
    }

    let visibility = if mode == BoundaryMode::Fixed {
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
        blend_color(WALL_COLOR, WALL_BRIGHT_COLOR, pulse_t)
    } else {
        WALL_COLOR
    };

    for (mut wall_visibility, mut wall_sprite) in &mut wall_q {
        *wall_visibility = visibility;
        wall_sprite.color = wall_color;
    }
}

fn blend_color(from: Color, to: Color, t: f32) -> Color {
    let t = t.clamp(0.0, 1.0);
    let from = from.to_srgba();
    let to = to.to_srgba();
    Color::srgba(
        from.red + (to.red - from.red) * t,
        from.green + (to.green - from.green) * t,
        from.blue + (to.blue - from.blue) * t,
        from.alpha + (to.alpha - from.alpha) * t,
    )
}

fn sync_pattern_preview(
    mut commands: Commands,
    hovered: Res<HoveredCell>,
    sim: Res<Simulation>,
    mut preview_pool: ResMut<PatternPreviewPool>,
    mut preview_q: Query<(&mut Transform, &mut Visibility), With<GhostPreviewCell>>,
) {
    let Some((cursor_col, cursor_row)) = hovered.0 else {
        hide_preview_cells(&preview_pool, &mut preview_q);
        return;
    };
    let Some((pattern, width, height)) = sim.app.selected_pattern_layout() else {
        hide_preview_cells(&preview_pool, &mut preview_q);
        return;
    };
    ensure_preview_pool(&mut commands, &mut preview_pool, pattern.len());
    let (cols, rows) = sim.app.board.size();

    let origin_col = cursor_col.saturating_sub(width / 2);
    let origin_row = cursor_row.saturating_sub(height / 2);

    for (idx, entity) in preview_pool.entities.iter().copied().enumerate() {
        let Ok((mut transform, mut visibility)) = preview_q.get_mut(entity) else {
            continue;
        };
        let Some(&(dx, dy)) = pattern.get(idx) else {
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

fn hide_preview_cells(
    preview_pool: &PatternPreviewPool,
    preview_q: &mut Query<(&mut Transform, &mut Visibility), With<GhostPreviewCell>>,
) {
    for entity in &preview_pool.entities {
        if let Ok((_, mut visibility)) = preview_q.get_mut(*entity) {
            *visibility = Visibility::Hidden;
        }
    }
}

fn update_hovered_cell(
    window_q: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    sim: Res<Simulation>,
    wants_input: Res<EguiWantsInput>,
    mut hovered: ResMut<HoveredCell>,
    pointer_capture: Res<UiPointerCapture>,
) {
    hovered.0 = None;
    if wants_input.wants_any_pointer_input() || pointer_capture.blocked {
        return;
    }
    let Ok(window) = window_q.single() else {
        return;
    };
    let Some(cursor) = window.cursor_position() else {
        return;
    };
    let Ok((camera, transform)) = camera_q.single() else {
        return;
    };
    let Ok(world) = camera.viewport_to_world_2d(transform, cursor) else {
        return;
    };
    let (cols, rows) = sim.app.board.size();
    hovered.0 = world_to_cell(world, cols, rows);
}

fn render_hover_cell(
    hovered: Res<HoveredCell>,
    sim: Res<Simulation>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut gizmos: Gizmos,
) {
    let Some((col, row)) = hovered.0 else {
        return;
    };
    let (cols, rows) = sim.app.board.size();
    let erasing = sim.app.tool() == EditTool::Erase || mouse_buttons.pressed(MouseButton::Right);
    let color = if erasing {
        Color::srgb(1.0, 0.60, 0.55)
    } else {
        Color::srgb(0.75, 0.95, 0.85)
    };
    gizmos.rect_2d(
        Isometry2d::from_translation(cell_to_world(col, row, cols, rows).truncate()),
        Vec2::splat(CELL_SIZE),
        color,
    );
}
