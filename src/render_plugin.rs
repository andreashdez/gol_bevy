use std::f32::consts::PI;

use crate::app::{App as GolApp, EditTool};
use crate::board::{BoundaryMode, DirtyCells, RowCells};
use crate::canvas_view::cursor_world;
use crate::shared::{
    BoardColors, BoardSettings, CELL_SIZE, CellAge, GridOverlay, MainCamera, RedrawRequested,
    Simulation, StartupSet, Theme, UpdateSet, board_size_for_window,
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

/// The board cell under the pointer, as `(col, row)` with row 0 at the bottom.
#[derive(Resource, Default)]
pub struct HoveredCell(pub Option<(i64, i64)>);

/// The single sprite that shows the stamp under the pointer.
#[derive(Component)]
struct StampGhost;

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
pub(crate) struct BoardSprite;

/// Generations since each cell last changed state. The renderer keeps these so
/// the engine's step loop pays nothing for them. A cell stops ageing once its
/// colour has settled.
#[derive(Resource, Default)]
pub(crate) struct CellAges {
    ages: Vec<u8>,
    /// The generation counter at the last sync, to tell how many generations ran since.
    epoch: usize,
    /// The first cell of each run of the board's rows with a cell still ageing,
    /// so a generation visits those cells and not the whole board. Most of a
    /// board that has run for a while is settled.
    ageing: Vec<u32>,
    /// The cells of each run that may still be ageing, by the run's place among
    /// the board's runs. A run is in `ageing` while it has any.
    ageing_cells: Vec<u64>,
}

impl CellAges {
    /// Has `cells` of the run starting at `first` age from now on. `across` is
    /// how many runs a row of the board is handed over in.
    fn keep_ageing(&mut self, first: usize, cells: u64, cols: usize, across: usize) {
        let ageing = &mut self.ageing_cells[run_index(first, cols, across)];
        if *ageing == 0 && cells != 0 {
            self.ageing.push(first as u32);
        }
        *ageing |= cells;
    }
}

/// The place of the run starting at `first` among the board's runs, row by row.
fn run_index(first: usize, cols: usize, across: usize) -> usize {
    first / cols * across + first % cols / RUN_CELLS
}

/// The most cells the board hands over at once.
const RUN_CELLS: usize = u64::BITS as usize;

/// Generations a newborn cell takes to settle into the live colour.
const MATURE_AGE: u8 = 6;
/// Generations after which a cell has the aged colour.
const OLD_AGE: u8 = 60;
/// Generations a trail takes to fade.
const TRAIL_LENGTH: u8 = 6;
/// Trails fade sooner than cells grow old, so past this age no colour changes.
pub(crate) const SETTLED_AGE: u8 = OLD_AGE;
const _: () = assert!(TRAIL_LENGTH <= SETTLED_AGE);

/// Cells that were edited, loaded, or restored rather than stepped: the plain
/// live colour, and no trail.
pub(crate) fn settled_age(alive: bool) -> u8 {
    if alive { MATURE_AGE } else { u8::MAX }
}

pub(crate) fn live_shade(colors: &BoardColors, age: u8) -> Color {
    if age < MATURE_AGE {
        blend_color(
            colors.newborn,
            colors.live,
            f32::from(age) / f32::from(MATURE_AGE),
        )
    } else {
        blend_color(
            colors.live,
            colors.aged,
            f32::from(age - MATURE_AGE) / f32::from(OLD_AGE - MATURE_AGE),
        )
    }
}

pub(crate) fn dead_shade(colors: &BoardColors, age: u8) -> Color {
    blend_color(
        colors.trail,
        colors.dead,
        f32::from(age) / f32::from(TRAIL_LENGTH),
    )
}

/// Texels for every state and age, so painting a cell is a table lookup.
pub(crate) struct CellPalette {
    live: [[u8; 4]; 256],
    dead: [[u8; 4]; 256],
}

impl CellPalette {
    pub(crate) fn new(colors: &BoardColors, by_age: bool) -> Self {
        let mut palette = Self {
            live: [[0; 4]; 256],
            dead: [[0; 4]; 256],
        };
        for age in 0..=u8::MAX {
            let (live, dead) = if by_age {
                (live_shade(colors, age), dead_shade(colors, age))
            } else {
                (colors.live, colors.dead)
            };
            palette.live[usize::from(age)] = color_to_texel(live);
            palette.dead[usize::from(age)] = color_to_texel(dead);
        }
        palette
    }

    pub(crate) fn paint(&self, data: &mut [u8], offset: usize, alive: bool, age: u8) {
        data[offset..offset + 4].copy_from_slice(&self.texel(alive, age));
    }

    pub(crate) fn texel(&self, alive: bool, age: u8) -> [u8; 4] {
        let table = if alive { &self.live } else { &self.dead };
        table[usize::from(age)]
    }
}

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
pub(crate) fn texel_offset(idx: usize, cols: usize, rows: usize) -> usize {
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

/// Which stamp the ghost texture was drawn from. The stamp is a paste or a
/// library pattern, turned and flipped; any of those changing means new cells.
/// The library revision is in it because a restored workspace starts the paste
/// count again and can put another pattern at the same index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StampKey {
    library_pattern: Option<usize>,
    paste_revision: u64,
    library_revision: u64,
    orientation: (u8, bool),
}

impl StampKey {
    fn of(app: &GolApp) -> Self {
        Self {
            library_pattern: app.selected_pattern_index(),
            paste_revision: app.paste_revision(),
            library_revision: app.library_revision(),
            orientation: app.stamp_orientation(),
        }
    }
}

/// The stamp ghost's texture and the stamp it shows. Laying the stamp out
/// touches every one of its cells, so it happens only when the stamp changes,
/// not every frame the pointer moves.
#[derive(Resource, Default)]
struct StampGhostImage {
    key: Option<StampKey>,
    handle: Handle<Image>,
    width: usize,
    height: usize,
}

/// The stamp ghost's texture and the assets it lives in, which change together.
#[derive(SystemParam)]
struct GhostTexture<'w> {
    images: ResMut<'w, Assets<Image>>,
    current: ResMut<'w, StampGhostImage>,
}

impl GhostTexture<'_> {
    /// The texture of the app's stamp, drawn again only when the stamp changed.
    fn of_stamp(&mut self, app: &GolApp) -> Option<&StampGhostImage> {
        let key = StampKey::of(app);
        if self.current.key != Some(key) {
            let (cells, width, height) = app.selected_pattern_layout()?;
            *self.current = StampGhostImage {
                key: Some(key),
                handle: self.images.add(stamp_ghost_image(&cells, width, height)),
                width,
                height,
            };
        }
        Some(&self.current)
    }

    /// Drops the texture; the assets free it once no sprite holds it.
    fn release(&mut self) {
        if self.current.key.is_some() {
            *self.current = StampGhostImage::default();
        }
    }
}

/// One texel per stamp cell: opaque white where a cell is alive, clear elsewhere,
/// so the sprite's colour tints it and a theme change needs no new texture. Row 0
/// of the stamp is its bottom row, as on the board.
fn stamp_ghost_image(cells: &[(usize, usize)], width: usize, height: usize) -> Image {
    let mut data = vec![0; width * height * 4];
    for &(dx, dy) in cells {
        let offset = texel_offset(dy * width + dx, width, height);
        data[offset..offset + 4].fill(u8::MAX);
    }
    let mut image = Image::new(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        // Never read back, so only the GPU keeps a copy.
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

#[derive(Resource)]
struct BoundaryWallPulse {
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

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<HoveredCell>()
            .init_resource::<Theme>()
            .init_resource::<CellAge>()
            .insert_resource(BoundaryWallPulse::default())
            .insert_resource(BoardTexture::default())
            .init_resource::<CellAges>()
            .init_resource::<StampGhostImage>()
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
                    render_selection.in_set(UpdateSet::Render),
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
    if sim.app.board.size() == Some((cols, rows))
        && (canvas.texture.cols, canvas.texture.rows) == (cols, rows)
    {
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

/// What decides a cell's colour besides its state.
#[derive(SystemParam)]
pub(crate) struct CellStyle<'w> {
    pub(crate) theme: Res<'w, Theme>,
    pub(crate) cell_age: Res<'w, CellAge>,
    ages: ResMut<'w, CellAges>,
}

fn sync_cells(
    redraw_requested: Res<RedrawRequested>,
    mut style: CellStyle,
    mut sim: ResMut<Simulation>,
    board_texture: Res<BoardTexture>,
    mut images: ResMut<Assets<Image>>,
) {
    // A new theme or cell style recolours every cell, not just the ones that changed.
    let recolor = style.theme.is_changed() || style.cell_age.is_changed();
    if !redraw_requested.0 && !recolor {
        return;
    }

    let app = &mut sim.app;
    let epoch = app.epoch;
    let Some(board) = app.board.finite_mut() else {
        return;
    };
    let (cols, rows) = board.size();
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

    let dirty = board.take_dirty_cells();
    // Undo and resetting the counter move it backwards; neither is a generation.
    let steps = u8::try_from(epoch.saturating_sub(style.ages.epoch)).unwrap_or(u8::MAX);
    style.ages.epoch = epoch;
    let board = &*board;
    let by_age = style.cell_age.enabled;
    let palette = CellPalette::new(style.theme.board(), by_age);
    let len = cols * rows;
    let across = cols.div_ceil(RUN_CELLS);
    let aged = &mut *style.ages;

    if !by_age {
        let mut paint = |run: RowCells| {
            for (idx, alive) in run.iter().filter(|&(idx, _)| idx < len) {
                palette.paint(data, texel_offset(idx, cols, rows), alive, 0);
            }
        };
        match dirty {
            DirtyCells::Cells(changed) if !recolor => changed.into_iter().for_each(paint),
            DirtyCells::None if !recolor => {}
            _ => board.row_cells().for_each(&mut paint),
        }
        return;
    }

    // Switching the style on starts from a calm board: nobody knows the ages yet.
    let reset = style.cell_age.is_changed() || aged.ages.len() != len || dirty == DirtyCells::Full;
    if reset {
        aged.ages.clear();
        aged.ages.resize(len, settled_age(false));
        aged.ageing.clear();
        aged.ageing_cells.clear();
        aged.ageing_cells.resize(rows * across, 0);
        for run in board.row_cells() {
            for (age, bit) in aged.ages[run.first..][..run.len()].iter_mut().zip(0..) {
                *age = settled_age(run.alive >> bit & 1 != 0);
            }
            // Live cells look settled, but have yet to grow old.
            aged.keep_ageing(run.first, run.alive, cols, across);
        }
    } else {
        if steps > 0 {
            let CellAges {
                ages,
                ageing,
                ageing_cells,
                ..
            } = aged;
            ageing.retain(|&first| {
                let run = board.cells_from(first as usize);
                let cells = &mut ageing_cells[run_index(run.first, cols, across)];
                // Texels of a row sit side by side, as its cells do.
                let texels = texel_offset(run.first, cols, rows);
                for (idx, alive) in (RowCells {
                    cells: *cells,
                    ..run
                })
                .iter()
                {
                    let (age, bit) = (&mut ages[idx], idx - run.first);
                    if *age < SETTLED_AGE {
                        *age = age.saturating_add(steps);
                        palette.paint(data, texels + bit * 4, alive, *age);
                    }
                    if *age >= SETTLED_AGE {
                        *cells &= !(1 << bit);
                    }
                }
                *cells != 0
            });
        }
        if let DirtyCells::Cells(changed) = dirty {
            for run in changed {
                for (idx, alive) in run.iter().filter(|&(idx, _)| idx < len) {
                    // A generation gives births and deaths age 0; hand edits look settled.
                    let age = if steps > 0 { 0 } else { settled_age(alive) };
                    aged.ages[idx] = age;
                    palette.paint(data, texel_offset(idx, cols, rows), alive, age);
                }
                // A cell that just died by hand has no trail, and settles at once.
                let ageing = if steps > 0 {
                    run.cells
                } else {
                    run.cells & run.alive
                };
                aged.keep_ageing(run.first, ageing, cols, across);
            }
        }
    }
    let ages = &aged.ages;
    if reset || recolor {
        for run in board.row_cells() {
            for (&age, bit) in ages[run.first..][..run.len()].iter().zip(0..) {
                let alive = run.alive >> bit & 1 != 0;
                palette.paint(data, texel_offset(run.first + bit, cols, rows), alive, age);
            }
        }
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

type GhostSprite<'a> = (
    Entity,
    &'a mut Transform,
    &'a mut Visibility,
    &'a mut Sprite,
);

/// Shows the stamp as one sprite over the footprint it would fill, however many
/// cells it has. On a finite board the sprite is cropped to the board like the
/// cells a click places; the board holds a stamp back from its low edges, so
/// only its top and right can run off. Nothing is written on frames where the
/// ghost stays put.
fn sync_pattern_preview(
    mut commands: Commands,
    hovered: Res<HoveredCell>,
    sim: Res<Simulation>,
    theme: Res<Theme>,
    mut texture: GhostTexture,
    mut ghost_q: Query<GhostSprite, With<StampGhost>>,
    mut gizmos: Gizmos,
) {
    let ghost = ghost_q.single_mut().ok();
    if !sim.app.has_selected_pattern() {
        // Without a stamp the texture goes too, rather than sit on the GPU.
        if let Some((entity, ..)) = ghost {
            commands.entity(entity).despawn();
        }
        texture.release();
        return;
    }
    let Some(cursor) = hovered.0 else {
        if let Some((_, _, mut visibility, _)) = ghost {
            visibility.set_if_neq(Visibility::Hidden);
        }
        return;
    };

    let Some(ghost_image) = texture.of_stamp(&sim.app) else {
        return;
    };
    let (width, height) = (ghost_image.width as i64, ghost_image.height as i64);
    let (origin_col, origin_row) = sim
        .app
        .board
        .stamp_origin(cursor, (ghost_image.width, ghost_image.height));

    let (mut shown_width, mut shown_height) = (width, height);
    if let Some((cols, rows)) = sim.app.board.size() {
        shown_width = shown_width.min(cols as i64 - origin_col);
        shown_height = shown_height.min(rows as i64 - origin_row);
    }
    if shown_width <= 0 || shown_height <= 0 {
        if let Some((_, _, mut visibility, _)) = ghost {
            visibility.set_if_neq(Visibility::Hidden);
        }
        return;
    }

    let map = sim.cell_map();
    let first = map.cell_to_world(origin_col, origin_row).truncate();
    let last = map
        .cell_to_world(origin_col + shown_width - 1, origin_row + shown_height - 1)
        .truncate();
    let centre = (first + last) / 2.0;
    gizmos.rect_2d(
        Isometry2d::from_translation(centre),
        (last - first).abs() + Vec2::splat(CELL_SIZE),
        theme.board().ghost_outline,
    );

    // The stamp's bottom row is the texture's last, so cropping the top keeps
    // the texture's bottom rows.
    let sprite = Sprite {
        image: ghost_image.handle.clone(),
        color: theme.board().ghost,
        custom_size: Some(Vec2::new(shown_width as f32, shown_height as f32) * CELL_SIZE),
        rect: Some(Rect::new(
            0.0,
            (height - shown_height) as f32,
            shown_width as f32,
            height as f32,
        )),
        ..default()
    };
    let transform = Transform::from_translation(centre.extend(2.0));
    match ghost {
        Some((_, mut current_transform, mut visibility, mut current_sprite)) => {
            current_transform.set_if_neq(transform);
            visibility.set_if_neq(Visibility::Visible);
            current_sprite.set_if_neq(sprite);
        }
        None => {
            commands.spawn((sprite, transform, Visibility::Visible, StampGhost));
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
    hovered.0 = sim.cell_map().world_to_cell(world);
}

/// Outlines the selection, in the stamp outline's colour: both mark cells a tool
/// is about to act on, rather than cells of the board.
fn render_selection(sim: Res<Simulation>, theme: Res<Theme>, mut gizmos: Gizmos) {
    let Some(area) = sim.app.selection() else {
        return;
    };
    let map = sim.cell_map();
    let first = map.cell_to_world(area.left, area.bottom).truncate();
    let last = map
        .cell_to_world(
            area.left + area.width as i64 - 1,
            area.bottom + area.height as i64 - 1,
        )
        .truncate();
    gizmos.rect_2d(
        Isometry2d::from_translation((first + last) / 2.0),
        (last - first).abs() + Vec2::splat(CELL_SIZE),
        theme.board().ghost_outline,
    );
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
    // A stamp shows its own outline instead.
    if sim.app.tool() == EditTool::Stamp && sim.app.has_selected_pattern() {
        return;
    }
    let erasing = sim.app.tool() == EditTool::Erase || mouse_buttons.pressed(MouseButton::Right);
    let color = if erasing {
        theme.board().hover_erase
    } else {
        theme.board().hover
    };
    gizmos.rect_2d(
        Isometry2d::from_translation(sim.cell_map().cell_to_world(col, row).truncate()),
        Vec2::splat(CELL_SIZE),
        color,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::cell_to_world;

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
            let lower = cell_to_world(0, row as i64 - 1, cols, rows).y;
            let upper = cell_to_world(0, row as i64, cols, rows).y;
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
    fn restored_board_dimensions_rebuild_the_texture_and_repaint_loaded_cells() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        let mut app = GolApp::new(16, 24);
        app.populate_board(15, 23);
        world.insert_resource(Simulation::new(app));
        world.insert_resource(BoardSettings {
            cols: 16,
            rows: 24,
            fill_canvas: false,
        });
        world.insert_resource(Theme::Dark);
        world.insert_resource(RedrawRequested(true));
        world.insert_resource(CellAge { enabled: false });
        world.init_resource::<CellAges>();
        let mut images = Assets::<Image>::default();
        let old = create_board_image(&mut images, 8, 8, Theme::Dark.board().dead);
        world.insert_resource(images);
        world.insert_resource(BoardTexture {
            handle: old,
            cols: 8,
            rows: 8,
        });
        let sprite = world.spawn((BoardSprite, Sprite::default())).id();
        world.run_system_once(apply_board_settings).unwrap();
        let texture = world.resource::<BoardTexture>();
        assert_eq!((texture.cols, texture.rows), (16, 24));
        assert_eq!(world.get::<Sprite>(sprite).unwrap().image, texture.handle);
        world.run_system_once(sync_cells).unwrap();
        let texture = world.resource::<BoardTexture>();
        let images = world.resource::<Assets<Image>>();
        let data = images.get(&texture.handle).unwrap().data.as_ref().unwrap();
        let offset = texel_offset(23 * 16 + 15, 16, 24);
        assert_eq!(
            data[offset..offset + 4],
            color_to_texel(Theme::Dark.board().live)
        );
    }

    #[test]
    fn syncing_cells_leaves_the_texture_alone_on_the_unbounded_board() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        let mut app = GolApp::new(4, 3);
        app.set_unbounded().unwrap();
        app.board.populate(1, 2);
        world.insert_resource(Simulation::new(app));
        world.insert_resource(RedrawRequested(true));
        world.insert_resource(Theme::Dark);
        world.insert_resource(CellAge { enabled: false });
        world.init_resource::<CellAges>();
        let mut images = Assets::<Image>::default();
        let handle = create_board_image(&mut images, 4, 3, Theme::Dark.board().dead);
        world.insert_resource(images);
        world.insert_resource(BoardTexture {
            handle: handle.clone(),
            cols: 4,
            rows: 3,
        });
        world.run_system_once(sync_cells).unwrap();

        let images = world.resource::<Assets<Image>>();
        let data = images.get(&handle).unwrap().data.as_ref().unwrap();
        let dead = color_to_texel(Theme::Dark.board().dead);
        assert!(data.as_chunks::<4>().0.iter().all(|&texel| texel == dead));
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
        world.insert_resource(CellAge { enabled: false });
        world.init_resource::<CellAges>();
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

    #[test]
    fn generations_shade_births_deaths_and_survivors_but_edits_look_settled() {
        let mut world = World::new();
        // A horizontal blinker in the middle row.
        let mut app = GolApp::new(5, 5);
        for col in 1..=3 {
            app.board.populate(col, 2);
        }
        world.insert_resource(Simulation::new(app));
        world.insert_resource(RedrawRequested(true));
        world.insert_resource(Theme::Dark);
        world.insert_resource(CellAge { enabled: true });
        world.init_resource::<CellAges>();
        let mut images = Assets::<Image>::default();
        let handle = create_board_image(&mut images, 5, 5, Theme::Dark.board().dead);
        world.insert_resource(images);
        world.insert_resource(BoardTexture {
            handle: handle.clone(),
            cols: 5,
            rows: 5,
        });
        // A registered system remembers what it has seen, unlike `run_system_once`.
        let sync = world.register_system(sync_cells);
        let texel = |world: &World, col: usize, row: usize| {
            let images = world.resource::<Assets<Image>>();
            let data = images.get(&handle).unwrap().data.as_ref().unwrap();
            let offset = texel_offset(row * 5 + col, 5, 5);
            <[u8; 4]>::try_from(&data[offset..offset + 4]).unwrap()
        };
        let colors = Theme::Dark.board();

        // Cells that were drawn rather than born start out plain.
        world.run_system(sync).unwrap();
        assert_eq!(texel(&world, 1, 2), color_to_texel(colors.live));
        assert_eq!(texel(&world, 1, 1), color_to_texel(colors.dead));

        world.resource_mut::<Simulation>().app.increment_epoch();
        world.run_system(sync).unwrap();
        assert_eq!(texel(&world, 2, 1), color_to_texel(colors.newborn));
        assert_eq!(texel(&world, 1, 2), color_to_texel(colors.trail));
        let survivor = color_to_texel(live_shade(colors, MATURE_AGE + 1));
        assert_eq!(texel(&world, 2, 2), survivor);

        // Trails fade and newborns settle as generations pass.
        for _ in 0..TRAIL_LENGTH {
            world.resource_mut::<Simulation>().app.increment_epoch();
        }
        world.run_system(sync).unwrap();
        assert_eq!(texel(&world, 0, 0), color_to_texel(colors.dead));
        assert_ne!(texel(&world, 2, 2), survivor, "the centre keeps ageing");

        // A cell drawn while paused looks like any live cell, not a birth.
        world.resource_mut::<Simulation>().app.populate_board(0, 0);
        world.run_system(sync).unwrap();
        assert_eq!(texel(&world, 0, 0), color_to_texel(colors.live));

        // Switching the style off paints plain colours again.
        world.resource_mut::<CellAge>().enabled = false;
        world.resource_mut::<RedrawRequested>().0 = false;
        world.run_system(sync).unwrap();
        assert_eq!(texel(&world, 2, 2), color_to_texel(colors.live));
    }
    #[test]
    fn a_board_wider_and_taller_than_a_chunk_paints_every_cell_at_its_own_texel() {
        // Cells are stored a chunk at a time, so this is where a cell could land
        // on a neighbour's texel: past column 64, past row 64, and at both edges.
        let (cols, rows) = (130, 70);
        for by_age in [false, true] {
            let mut world = World::new();
            let mut app = GolApp::new(cols, rows);
            let noise = crate::board::RandomizeSettings::new(30, Some(5)).unwrap();
            app.set_randomize_settings(noise);
            app.randomize_board();
            world.insert_resource(Simulation::new(app));
            world.insert_resource(RedrawRequested(true));
            world.insert_resource(Theme::Dark);
            world.insert_resource(CellAge { enabled: by_age });
            world.init_resource::<CellAges>();
            let mut images = Assets::<Image>::default();
            let handle = create_board_image(&mut images, cols, rows, Theme::Dark.board().dead);
            world.insert_resource(images);
            world.insert_resource(BoardTexture {
                handle: handle.clone(),
                cols,
                rows,
            });
            let sync = world.register_system(sync_cells);
            let palette = CellPalette::new(Theme::Dark.board(), by_age);
            let cells = |world: &World| -> Vec<bool> {
                let board = &world.resource::<Simulation>().app.board;
                (0..cols * rows)
                    .map(|idx| board.is_alive((idx % cols) as i64, (idx / cols) as i64))
                    .collect()
            };

            world.run_system(sync).unwrap();
            let mut before = cells(&world);
            // What the ages should be, kept a cell at a time with nothing skipped.
            let mut ages: Vec<u8> = before.iter().map(|&alive| settled_age(alive)).collect();

            // Generations run before each frame: one, several, and none, which is a
            // frame with only a hand edit in it. The run of nines takes cells born at
            // different times past settling a few at a time, so that a run of cells
            // has some settled and some not, and the seventy settles all the rest.
            let frames = [
                1, 1, 3, 1, 0, 7, 1, 1, 20, 1, 0, 1, 9, 9, 9, 9, 9, 9, 9, 70, 1, 1, 2, 0, 1,
            ];
            for (frame, generations) in frames.into_iter().enumerate() {
                let mut changed = vec![false; cols * rows];
                let mut after = before.clone();
                for _ in 0..generations {
                    world.resource_mut::<Simulation>().app.increment_epoch();
                    let next = cells(&world);
                    for idx in 0..cols * rows {
                        // A cell that flips and flips back has still changed.
                        changed[idx] |= next[idx] != after[idx];
                    }
                    after = next;
                }
                if generations == 0 {
                    // A cell past the first chunk, drawn or erased by hand.
                    let (col, row) = (64 + frame, 65);
                    let app = &mut world.resource_mut::<Simulation>().app;
                    if before[row * cols + col] {
                        app.depopulate_board(col as i64, row as i64);
                    } else {
                        app.populate_board(col as i64, row as i64);
                    }
                    after = cells(&world);
                    changed[row * cols + col] = true;
                }
                world.resource_mut::<RedrawRequested>().0 = true;
                world.run_system(sync).unwrap();

                for idx in 0..cols * rows {
                    if ages[idx] < SETTLED_AGE {
                        ages[idx] = ages[idx].saturating_add(generations);
                    }
                    if changed[idx] {
                        ages[idx] = if generations > 0 {
                            0
                        } else {
                            settled_age(after[idx])
                        };
                    }
                }
                let images = world.resource::<Assets<Image>>();
                let data = images.get(&handle).unwrap().data.as_ref().unwrap();
                for idx in 0..cols * rows {
                    let offset = texel_offset(idx, cols, rows);
                    assert_eq!(
                        data[offset..offset + 4],
                        palette.texel(after[idx], if by_age { ages[idx] } else { 0 }),
                        "cell {idx} in frame {frame}, by age: {by_age}"
                    );
                }
                if by_age {
                    assert_eq!(world.resource::<CellAges>().ages, ages, "frame {frame}");
                }
                before = after;
            }
        }
    }

    #[test]
    fn only_cells_still_ageing_are_visited() {
        let mut world = World::new();
        let mut app = GolApp::new(200, 150);
        // A block, which never changes, and a blinker, which never stops.
        for (col, row) in [(100, 100), (101, 100), (100, 101), (101, 101)] {
            app.board.populate(col, row);
        }
        for col in 9..12 {
            app.board.populate(col, 10);
        }
        world.insert_resource(Simulation::new(app));
        world.insert_resource(RedrawRequested(true));
        world.insert_resource(Theme::Dark);
        world.insert_resource(CellAge { enabled: true });
        world.init_resource::<CellAges>();
        let mut images = Assets::<Image>::default();
        let handle = create_board_image(&mut images, 200, 150, Theme::Dark.board().dead);
        world.insert_resource(images);
        world.insert_resource(BoardTexture {
            handle,
            cols: 200,
            rows: 150,
        });
        let sync = world.register_system(sync_cells);
        let frame = |world: &mut World| {
            world.resource_mut::<Simulation>().app.increment_epoch();
            world.resource_mut::<RedrawRequested>().0 = true;
            world.run_system(sync).unwrap();
            let mut ageing = world.resource::<CellAges>().ageing.clone();
            ageing.sort_unstable();
            ageing
        };

        world.run_system(sync).unwrap();
        // Drawn cells have yet to grow old: the block's two rows, and the blinker's
        // three now it has turned.
        let block = [100 * 200 + 64, 101 * 200 + 64];
        assert_eq!(
            frame(&mut world),
            [9 * 200, 10 * 200, 11 * 200, block[0], block[1]]
        );

        for _ in 0..usize::from(SETTLED_AGE) {
            frame(&mut world);
        }
        // The block has settled; the blinker's three rows change every generation.
        assert_eq!(frame(&mut world), [9 * 200, 10 * 200, 11 * 200]);
        // Of its cells, only the four that turn on and off: its centre has grown old.
        let cells = &world.resource::<CellAges>().ageing_cells;
        assert_eq!(cells.iter().map(|cells| cells.count_ones()).sum::<u32>(), 4);
    }

    #[test]
    fn the_stamp_ghost_texture_has_one_texel_per_cell_with_the_bottom_row_last() {
        // An L, three tall: a foot two wide on the bottom row.
        let image = stamp_ghost_image(&[(0, 0), (1, 0), (0, 1), (0, 2)], 2, 3);
        assert_eq!((image.width(), image.height()), (2, 3));
        assert_eq!(image.sampler, ImageSampler::nearest());
        let texels = image.data.as_ref().unwrap().as_chunks::<4>().0;
        let (alive, clear) = ([u8::MAX; 4], [0; 4]);
        // Texel rows run top to bottom.
        assert_eq!(texels, [alive, clear, alive, clear, alive, alive]);
    }

    fn stamp_world(app: GolApp) -> World {
        use bevy::gizmos::config::{DefaultGizmoConfigGroup, GizmoConfig, GizmoConfigStore};
        let mut world = World::new();
        world.insert_resource(Simulation::new(app));
        world.insert_resource(Theme::Dark);
        world.insert_resource(HoveredCell(Some((8, 12))));
        world.insert_resource(Assets::<Image>::default());
        world.init_resource::<StampGhostImage>();
        let mut gizmos = GizmoConfigStore::default();
        gizmos.insert(GizmoConfig::default(), DefaultGizmoConfigGroup);
        world.insert_resource(gizmos);
        world
    }

    fn ghosts(world: &mut World) -> Vec<Entity> {
        world
            .query_filtered::<Entity, With<StampGhost>>()
            .iter(world)
            .collect()
    }

    #[test]
    fn a_large_stamp_is_one_cropped_sprite_laid_out_once() {
        use bevy::ecs::system::RunSystemOnce;
        let (width, height) = (300, 200);
        let mut rle = format!("x = {width}, y = {height}\n");
        for _ in 1..height {
            rle.push_str(&format!("{width}o$"));
        }
        rle.push_str(&format!("{width}o!"));
        let mut app = GolApp::new(16, 24);
        app.paste_rle(&rle).unwrap();
        let mut world = stamp_world(app);

        world.run_system_once(sync_pattern_preview).unwrap();
        let [ghost] = ghosts(&mut world)[..] else {
            panic!("one ghost sprite");
        };
        let handle = world.resource::<StampGhostImage>().handle.clone();
        let sprite = world.get::<Sprite>(ghost).unwrap();
        assert_eq!(sprite.image, handle);
        assert_eq!(sprite.color, Theme::Dark.board().ghost);
        // Held back to the board's corner and cropped to the whole 16 × 24 board,
        // keeping the stamp's bottom rows, which are the texture's last.
        assert_eq!(sprite.custom_size, Some(Vec2::new(16.0, 24.0) * CELL_SIZE));
        assert_eq!(sprite.rect, Some(Rect::new(0.0, 176.0, 16.0, 200.0)));
        assert_eq!(
            world.get::<Transform>(ghost).unwrap().translation,
            Vec3::new(0.0, 0.0, 2.0)
        );
        assert_eq!(world.get::<Visibility>(ghost), Some(&Visibility::Visible));

        // A frame with the same stamp in the same place neither lays it out
        // again nor touches the sprite.
        let ticks = |world: &World| {
            let entity = world.entity(ghost);
            [
                entity.get_ref::<Sprite>().unwrap().last_changed(),
                entity.get_ref::<Transform>().unwrap().last_changed(),
                entity.get_ref::<Visibility>().unwrap().last_changed(),
            ]
        };
        let before = ticks(&world);
        world.run_system_once(sync_pattern_preview).unwrap();
        assert_eq!(ticks(&world), before);
        assert_eq!(world.resource::<StampGhostImage>().handle, handle);
        assert_eq!(world.resource::<Assets<Image>>().len(), 1);
        assert_eq!(ghosts(&mut world), [ghost]);

        // Turning the stamp draws it again, on its side.
        world
            .resource_mut::<Simulation>()
            .app
            .rotate_stamp_clockwise();
        world.run_system_once(sync_pattern_preview).unwrap();
        let ghost_image = world.resource::<StampGhostImage>();
        assert_ne!(ghost_image.handle, handle);
        assert_eq!((ghost_image.width, ghost_image.height), (height, width));
        assert_eq!(ghosts(&mut world), [ghost]);

        // Off the board it hides; without a stamp it goes, texture and all.
        world.resource_mut::<HoveredCell>().0 = None;
        world.run_system_once(sync_pattern_preview).unwrap();
        assert_eq!(world.get::<Visibility>(ghost), Some(&Visibility::Hidden));
        world
            .resource_mut::<Simulation>()
            .app
            .set_tool(EditTool::Draw);
        world.run_system_once(sync_pattern_preview).unwrap();
        assert!(ghosts(&mut world).is_empty());
        assert_eq!(world.resource::<StampGhostImage>().key, None);
    }
}
