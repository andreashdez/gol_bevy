use std::f32::consts::PI;

use crate::app::{App as GolApp, EditTool};
use crate::board::{BoundaryMode, DirtyCells};
use crate::canvas_view::cursor_world;
use crate::shared::{
    BoardSettings, CELL_SIZE, GridOverlay, MainCamera, RedrawRequested, Simulation, StartupSet,
    Theme, UpdateSet, board_size_for_window, cell_to_world, world_to_cell,
};
use crate::ui_input::UiInputCapture;
use bevy::{
    asset::RenderAssetUsages,
    camera::visibility::RenderLayers,
    ecs::system::SystemParam,
    image::ImageSampler,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    window::PrimaryWindow,
};

pub struct RenderPlugin;

#[derive(Resource, Default)]
struct HoveredCell(Option<(usize, usize)>);

#[derive(Component)]
struct GhostPreviewCell;

#[derive(Component)]
struct BoundaryWall;

/// The board is drawn as one texture with a single texel per cell, so rendering
/// cost tracks the cells that changed instead of one sprite entity per cell.
#[derive(Resource, Default)]
struct BoardTexture {
    handle: Handle<Image>,
    cols: usize,
    rows: usize,
}

#[derive(Component)]
struct BoardSprite;

pub(crate) fn color_to_texel(color: Color) -> [u8; 4] {
    let srgba = color.to_srgba();
    [
        (srgba.red * 255.0).round().clamp(0.0, 255.0) as u8,
        (srgba.green * 255.0).round().clamp(0.0, 255.0) as u8,
        (srgba.blue * 255.0).round().clamp(0.0, 255.0) as u8,
        (srgba.alpha * 255.0).round().clamp(0.0, 255.0) as u8,
    ]
}

/// Board row 0 sits at the bottom in world space while texel row 0 sits at the
/// top, so the vertical axis is flipped when addressing the texture.
fn texel_offset(idx: usize, cols: usize, rows: usize) -> usize {
    let col = idx % cols;
    let row = idx / cols;
    ((rows - 1 - row) * cols + col) * 4
}

fn board_sprite_size(cols: usize, rows: usize) -> Vec2 {
    Vec2::new(cols as f32 * CELL_SIZE, rows as f32 * CELL_SIZE)
}

fn create_board_image(
    images: &mut Assets<Image>,
    cols: usize,
    rows: usize,
    dead: Color,
) -> Handle<Image> {
    let mut image = Image::new_fill(
        Extent3d {
            width: cols as u32,
            height: rows as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &color_to_texel(dead),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    // Cells are crisp squares; filtering would smear neighbours together.
    image.sampler = ImageSampler::nearest();
    images.add(image)
}

/// The board texture, its dimensions, and the sprite that draws it always change
/// together, so they travel as one system parameter.
#[derive(SystemParam)]
struct BoardCanvas<'w, 's> {
    images: ResMut<'w, Assets<Image>>,
    texture: ResMut<'w, BoardTexture>,
    sprite_q: Query<'w, 's, &'static mut Sprite, With<BoardSprite>>,
    theme: Res<'w, Theme>,
}

impl BoardCanvas<'_, '_> {
    /// Point the sprite at a freshly sized texture. The previous image is dropped
    /// once the old handle goes away.
    fn rebuild(&mut self, cols: usize, rows: usize) {
        let handle = create_board_image(&mut self.images, cols, rows, self.theme.board().dead);
        if let Ok(mut sprite) = self.sprite_q.single_mut() {
            sprite.image = handle.clone();
            sprite.custom_size = Some(board_sprite_size(cols, rows));
        }
        *self.texture = BoardTexture { handle, cols, rows };
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
            .init_resource::<Theme>()
            .insert_resource(BoundaryWallPulse::default())
            .insert_resource(BoardTexture::default())
            .insert_resource(PatternPreviewPool::default())
            .add_systems(Startup, setup_world.in_set(StartupSet::BuildWorld))
            .add_systems(
                Update,
                (
                    apply_board_settings.in_set(UpdateSet::Resize),
                    sync_cells.in_set(UpdateSet::Render),
                    render_grid.in_set(UpdateSet::Render),
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
    mut images: ResMut<Assets<Image>>,
    mut board_texture: ResMut<BoardTexture>,
    mut settings: ResMut<BoardSettings>,
    theme: Res<Theme>,
) {
    let Ok(window) = window_q.single() else {
        return;
    };
    // A persisted size wins; otherwise start with a board that fills the window.
    if !settings.is_set() {
        let (cols, rows) = board_size_for_window(window);
        settings.set(cols, rows);
    }
    let (cols, rows) = (settings.cols, settings.rows);

    commands.spawn((Camera2d, MainCamera));
    commands.spawn((
        Camera2d,
        Camera {
            order: 1,
            clear_color: ClearColorConfig::None,
            ..default()
        },
        RenderLayers::none(),
        IsDefaultUiCamera,
    ));
    commands.insert_resource(Simulation::new(GolApp::new(cols, rows)));

    let handle = create_board_image(&mut images, cols, rows, theme.board().dead);
    commands.spawn((
        Sprite {
            image: handle.clone(),
            custom_size: Some(board_sprite_size(cols, rows)),
            ..default()
        },
        Transform::from_translation(Vec3::ZERO),
        BoardSprite,
    ));
    *board_texture = BoardTexture { handle, cols, rows };

    spawn_boundary_walls(&mut commands, cols, rows, theme.board().wall);
}

fn spawn_boundary_walls(commands: &mut Commands, cols: usize, rows: usize, color: Color) {
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

fn ensure_preview_pool(
    commands: &mut Commands,
    pool: &mut PatternPreviewPool,
    target: usize,
    color: Color,
) {
    while pool.entities.len() < target {
        let entity = commands
            .spawn((
                Sprite::from_color(color, Vec2::splat(CELL_SIZE - 2.0)),
                Transform::from_translation(Vec3::new(0.0, 0.0, 2.0)),
                Visibility::Hidden,
                GhostPreviewCell,
            ))
            .id();
        pool.entities.push(entity);
    }
}

/// Window resizes no longer touch the board; only an explicit size change does.
fn apply_board_settings(
    settings: Res<BoardSettings>,
    mut commands: Commands,
    mut sim: ResMut<Simulation>,
    mut canvas: BoardCanvas,
    wall_q: Query<Entity, With<BoundaryWall>>,
) {
    if !settings.is_changed() || !settings.is_set() {
        return;
    }
    let (cols, rows) = (settings.cols, settings.rows);
    if sim.app.board.size() == (cols, rows) {
        return;
    }

    sim.app.resize_board(cols, rows);
    // resize_board already marked the board fully dirty, so the next sync repaints
    // the new texture.
    canvas.rebuild(cols, rows);

    for entity in &wall_q {
        commands.entity(entity).despawn();
    }
    let wall = canvas.theme.board().wall;
    spawn_boundary_walls(&mut commands, cols, rows, wall);
}

/// Draws only the grid lines that intersect the viewport, so the cost tracks
/// screen area rather than board area. Spawning a sprite per line would put the
/// per-cell entity problem straight back.
fn render_grid(
    grid_overlay: Res<GridOverlay>,
    theme: Res<Theme>,
    sim: Res<Simulation>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    mut gizmos: Gizmos,
) {
    if !grid_overlay.enabled {
        return;
    }
    let (cols, rows) = sim.app.board.size();
    if cols == 0 || rows == 0 {
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

    let half_width = cols as f32 * CELL_SIZE / 2.0;
    let half_height = rows as f32 * CELL_SIZE / 2.0;

    let x_min = (-half_width).max(visible_min.x);
    let x_max = half_width.min(visible_max.x);
    let y_min = (-half_height).max(visible_min.y);
    let y_max = half_height.min(visible_max.y);
    if x_min > x_max || y_min > y_max {
        return;
    }

    let color = theme.board().grid_line;
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

fn sync_cells(
    redraw_requested: Res<RedrawRequested>,
    theme: Res<Theme>,
    mut sim: ResMut<Simulation>,
    board_texture: Res<BoardTexture>,
    mut images: ResMut<Assets<Image>>,
) {
    // A new theme recolours every cell, not just the ones that changed.
    let recolor = theme.is_changed();
    if !redraw_requested.0 && !recolor {
        return;
    }

    let (cols, rows) = sim.app.board.size();
    // A resize rebuilds the texture and marks the board fully dirty, so bailing
    // out on a mismatch defers the repaint by a frame rather than dropping it.
    if cols == 0 || rows == 0 || cols != board_texture.cols || rows != board_texture.rows {
        return;
    }
    let Some(mut image) = images.get_mut(&board_texture.handle) else {
        return;
    };
    let Some(data) = image.data.as_mut() else {
        return;
    };

    let dirty = sim.app.board.take_dirty_cells();
    let indices: Box<dyn Iterator<Item = usize>> = match dirty {
        _ if recolor => Box::new(0..cols * rows),
        DirtyCells::None => return,
        DirtyCells::Full => Box::new(0..cols * rows),
        DirtyCells::Cells(cells) => Box::new(cells.into_iter()),
    };

    let live = color_to_texel(theme.board().live);
    let dead = color_to_texel(theme.board().dead);
    for idx in indices {
        if idx >= cols * rows {
            continue;
        }
        let texel = if sim.app.board.is_alive(idx % cols, idx / cols) {
            &live
        } else {
            &dead
        };
        let offset = texel_offset(idx, cols, rows);
        data[offset..offset + 4].copy_from_slice(texel);
    }
}

fn sync_boundary_wall_visibility(
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
        pulse.active = mode == BoundaryMode::Fixed;
        if pulse.active {
            pulse.timer.reset();
            pulse.pulses_done = 0;
        }
    }

    if !redraw_requested.0 && !pulse.active && !theme.is_changed() {
        return;
    }
    let colors = theme.board();

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
        blend_color(colors.wall, colors.wall_bright, pulse_t)
    } else {
        colors.wall
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
    theme: Res<Theme>,
    mut preview_pool: ResMut<PatternPreviewPool>,
    mut preview_q: Query<(&mut Transform, &mut Visibility), With<GhostPreviewCell>>,
    mut ghosts: Query<&mut Sprite, With<GhostPreviewCell>>,
) {
    let ghost = theme.board().ghost;
    if theme.is_changed() {
        for mut sprite in &mut ghosts {
            sprite.color = ghost;
        }
    }
    let Some((cursor_col, cursor_row)) = hovered.0 else {
        hide_preview_cells(&preview_pool, &mut preview_q);
        return;
    };
    let Some((pattern, width, height)) = sim.app.selected_pattern_layout() else {
        hide_preview_cells(&preview_pool, &mut preview_q);
        return;
    };
    ensure_preview_pool(&mut commands, &mut preview_pool, pattern.len(), ghost);
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
    mut hovered: ResMut<HoveredCell>,
    pointer_capture: Res<UiInputCapture>,
) {
    hovered.0 = None;
    if pointer_capture.pointer {
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
    let Some(world) = cursor_world(camera, transform, cursor) else {
        return;
    };
    let (cols, rows) = sim.app.board.size();
    hovered.0 = world_to_cell(world, cols, rows);
}

fn render_hover_cell(
    hovered: Res<HoveredCell>,
    theme: Res<Theme>,
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
        theme.board().hover_erase
    } else {
        theme.board().hover
    };
    gizmos.rect_2d(
        Isometry2d::from_translation(cell_to_world(col, row, cols, rows).truncate()),
        Vec2::splat(CELL_SIZE),
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn texel_rows_are_flipped_relative_to_board_rows() {
        let (cols, rows) = (4, 3);

        // Board row 0 is the bottom row, so it maps to the last texel row.
        assert_eq!(texel_offset(0, cols, rows), (2 * cols) * 4);
        assert_eq!(texel_offset(3, cols, rows), (2 * cols + 3) * 4);
        // The top board row maps to texel row 0.
        assert_eq!(texel_offset(2 * cols, cols, rows), 0);
        assert_eq!(texel_offset(2 * cols + 1, cols, rows), 4);
        // Every cell lands on a distinct, in-bounds texel.
        let mut offsets: Vec<_> = (0..cols * rows)
            .map(|idx| texel_offset(idx, cols, rows))
            .collect();
        offsets.sort_unstable();
        offsets.dedup();
        assert_eq!(offsets.len(), cols * rows);
        assert_eq!(offsets.last(), Some(&((cols * rows - 1) * 4)));
    }

    #[test]
    fn texel_order_matches_world_space_vertical_order() {
        let (cols, rows) = (4, 3);
        // A cell higher up in world space must sit earlier in the texture.
        for row in 1..rows {
            let lower = cell_to_world(0, row - 1, cols, rows).y;
            let upper = cell_to_world(0, row, cols, rows).y;
            assert!(upper > lower);
            assert!(
                texel_offset(row * cols, cols, rows) < texel_offset((row - 1) * cols, cols, rows)
            );
        }
        // Sanity check the world mapping the texture has to agree with.
        assert_eq!(
            cell_to_world(0, 0, cols, rows).y,
            -(rows as f32) * CELL_SIZE / 2.0 + CELL_SIZE * 0.5
        );
    }

    #[test]
    fn cell_colors_round_trip_to_srgb_bytes() {
        let colors = Theme::Dark.board();
        assert_eq!(color_to_texel(colors.live), [56, 217, 143, 255]);
        assert_eq!(color_to_texel(colors.dead), [20, 23, 28, 255]);
    }

    #[test]
    fn switching_themes_repaints_every_cell_without_a_board_change() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        let mut app = GolApp::new(4, 3);
        app.board.populate(1, 2);
        world.insert_resource(Simulation::new(app));
        world.insert_resource(RedrawRequested(true));
        world.insert_resource(Theme::Dark);
        let mut images = Assets::<Image>::default();
        let handle = create_board_image(&mut images, 4, 3, Theme::Dark.board().dead);
        world.insert_resource(images);
        world.insert_resource(BoardTexture {
            handle: handle.clone(),
            cols: 4,
            rows: 3,
        });
        world.run_system_once(sync_cells).unwrap();

        // Nothing on the board changes, so only the theme can cause a repaint.
        world.resource_mut::<RedrawRequested>().0 = false;
        *world.resource_mut::<Theme>() = Theme::Light;
        world.run_system_once(sync_cells).unwrap();
        let colors = Theme::Light.board();
        let images = world.resource::<Assets<Image>>();
        let data = images.get(&handle).unwrap().data.as_ref().unwrap();
        for idx in 0..12 {
            let expected = if idx == 2 * 4 + 1 {
                colors.live
            } else {
                colors.dead
            };
            let offset = texel_offset(idx, 4, 3);
            assert_eq!(data[offset..offset + 4], color_to_texel(expected), "{idx}");
        }
    }
}
