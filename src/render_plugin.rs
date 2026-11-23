use std::f32::consts::PI;

use crate::app::{App as GolApp, EditTool};
use crate::board::{BoundaryMode, DirtyCells, RowCells};
use crate::canvas_view::cursor_world;
use crate::coords::{CELL_SIZE, board_size_for_window};
use crate::frame_pacing::StayAwake;
use crate::shared::{
    BoardColors, BoardSettings, CellAge, GridOverlay, MainCamera, RedrawRequested, Simulation,
    StartupSet, Theme, UpdateSet,
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

/// The app's frame without a window, for the frame-loop benchmark.
#[doc(hidden)]
pub mod frame_loop;

pub struct RenderPlugin;

/// The board cell under the pointer, or the keyboard's cell cursor while it
/// shows, as `(col, row)` with row 0 at the bottom.
#[derive(Resource, Default)]
pub struct HoveredCell(pub Option<(i64, i64)>);

/// The single sprite that shows the stamp under the pointer.
#[derive(Component)]
struct StampGhost;

#[derive(Component)]
struct BoundaryWall;

/// The board is drawn as textures with a single texel per cell, so rendering
/// cost tracks the cells that changed instead of one sprite entity per cell.
/// The texture is cut into tiles, each its own image, so that painting a few
/// cells sends the GPU only the tiles they are on rather than the whole board.
#[derive(Resource, Default)]
struct BoardTexture {
    layout: TileLayout,
    /// One image per tile, in the order of [`TileLayout::rect`].
    tiles: Vec<Handle<Image>>,
    /// The tiles painted on since they were last handed to the renderer.
    touched: Vec<bool>,
}

/// Cells along a side of a board tile. An edit re-uploads the 256 KiB of its
/// tile instead of the whole board (16 MiB at 2048 × 2048), while the largest
/// board is still only 64 tiles, each a sprite and a draw call. A multiple of
/// the runs the board hands cells over in, so a run never straddles two tiles.
const TILE_CELLS: usize = 256;
const _: () = assert!(TILE_CELLS.is_multiple_of(RUN_CELLS));

/// How a `cols` × `rows` board is cut into tiles: row by row from the bottom
/// left, with the tiles at the right and top edges cut short.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct TileLayout {
    cols: usize,
    rows: usize,
}

impl TileLayout {
    fn across(self) -> usize {
        self.cols.div_ceil(TILE_CELLS)
    }

    fn count(self) -> usize {
        self.across() * self.rows.div_ceil(TILE_CELLS)
    }

    /// The first column and row of a tile, and its width and height in cells.
    fn rect(self, tile: usize) -> (usize, usize, usize, usize) {
        let (col, row) = (
            tile % self.across() * TILE_CELLS,
            tile / self.across() * TILE_CELLS,
        );
        let width = (self.cols - col).min(TILE_CELLS);
        let height = (self.rows - row).min(TILE_CELLS);
        (col, row, width, height)
    }

    /// The tile cell `idx` is on, the byte offset of its texel in that tile's
    /// texture, and how many cells of its row the tile holds from it on.
    fn locate(self, idx: usize) -> (usize, usize, usize) {
        let (col, row) = (idx % self.cols, idx / self.cols);
        let tile = row / TILE_CELLS * self.across() + col / TILE_CELLS;
        let (first_col, first_row, width, height) = self.rect(tile);
        let (x, y) = (col - first_col, row - first_row);
        (tile, texel_offset(y * width + x, width, height), width - x)
    }
}

impl BoardTexture {
    /// Blank tiles, the colour of dead cells, for a `cols` × `rows` board.
    fn new(images: &mut Assets<Image>, cols: usize, rows: usize, dead: Color) -> Self {
        let layout = TileLayout { cols, rows };
        let tiles = (0..layout.count())
            .map(|tile| {
                let (_, _, width, height) = layout.rect(tile);
                images.add(tile_image(width, height, dead))
            })
            .collect();
        Self {
            layout,
            tiles,
            touched: vec![false; layout.count()],
        }
    }

    /// A sprite for each tile, under `board`, which sits at the board's centre.
    fn spawn_sprites(&self, commands: &mut Commands, board: Entity) {
        let TileLayout { cols, rows } = self.layout;
        let corner = -board_sprite_size(cols, rows) / 2.0;
        for (tile, handle) in self.tiles.iter().enumerate() {
            let (col, row, width, height) = self.layout.rect(tile);
            let size = board_sprite_size(width, height);
            let min = corner + Vec2::new(col as f32, row as f32) * CELL_SIZE;
            commands.spawn((
                Sprite {
                    image: handle.clone(),
                    custom_size: Some(size),
                    ..default()
                },
                Transform::from_translation((min + size / 2.0).extend(0.0)),
                BoardTile,
                ChildOf(board),
            ));
        }
    }

    /// The texels of the cells from `first` to the end of its row on its tile,
    /// side by side. The tile counts as painted on, but the renderer sees the
    /// paint only once it is [`upload`](Self::upload)ed.
    fn row_texels<'a>(
        &mut self,
        images: &'a mut Assets<Image>,
        first: usize,
    ) -> Option<&'a mut [u8]> {
        let (tile, offset, cells) = self.layout.locate(first);
        self.touched[tile] = true;
        let data = images.get_mut_untracked(&self.tiles[tile])?.data.as_mut()?;
        Some(&mut data[offset..][..cells * 4])
    }

    /// Marks the tiles painted on as modified, so the renderer uploads those
    /// and leaves the rest of the board alone.
    fn upload(&mut self, images: &mut Assets<Image>) {
        for (handle, touched) in self.tiles.iter().zip(&mut self.touched) {
            if std::mem::take(touched)
                && let Some(image) = images.get_mut(handle)
            {
                image.into_inner();
            }
        }
    }
}

/// The entity the board's tile sprites hang from, at the board's centre;
/// hiding it hides the board.
#[derive(Component)]
pub(crate) struct BoardSprite;

/// A sprite drawing one tile of the board.
#[derive(Component)]
struct BoardTile;

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

fn tile_image(width: usize, height: usize, dead: Color) -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &color_to_texel(dead),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    // Cells are crisp squares; filtering would smear neighbours together.
    image.sampler = ImageSampler::nearest();
    image
}

/// The board texture, its dimensions, and the sprites that draw it always
/// change together, so they travel as one system parameter.
#[derive(SystemParam)]
struct BoardCanvas<'w, 's> {
    commands: Commands<'w, 's>,
    images: ResMut<'w, Assets<Image>>,
    texture: ResMut<'w, BoardTexture>,
    board_q: Query<'w, 's, Entity, With<BoardSprite>>,
    tile_q: Query<'w, 's, Entity, With<BoardTile>>,
    theme: Res<'w, Theme>,
}

impl BoardCanvas<'_, '_> {
    /// Draw the board with freshly sized tiles. The previous images are dropped
    /// once their sprites go.
    fn rebuild(&mut self, cols: usize, rows: usize) {
        for tile in &self.tile_q {
            self.commands.entity(tile).despawn();
        }
        *self.texture = BoardTexture::new(&mut self.images, cols, rows, self.theme.board().dead);
        if let Ok(board) = self.board_q.single() {
            self.texture.spawn_sprites(&mut self.commands, board);
        }
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
            .add_message::<StayAwake>()
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
                    keep_awake_while_pulsing,
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

    *board_texture = BoardTexture::new(&mut images, cols, rows, theme.board().dead);
    let board = commands
        .spawn((Transform::default(), Visibility::default(), BoardSprite))
        .id();
    board_texture.spawn_sprites(&mut commands, board);

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
    mut texture: ResMut<BoardTexture>,
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
    if cols == 0 || rows == 0 || texture.layout != (TileLayout { cols, rows }) {
        return;
    }

    let dirty = board.take_dirty_cells();
    // Undo and resetting the counter move it backwards; neither is a generation.
    let steps = u8::try_from(epoch.saturating_sub(style.ages.epoch)).unwrap_or(u8::MAX);
    style.ages.epoch = epoch;
    let by_age = style.cell_age.enabled;
    let len = cols * rows;
    // A redraw with nothing to show, such as turning the stamp, leaves the
    // texture alone rather than sending it to the GPU again.
    let ageing = steps > 0 && !style.ages.ageing.is_empty();
    let ages_known = style.ages.ages.len() == len;
    if !recolor && dirty == DirtyCells::None && (!by_age || (!ageing && ages_known)) {
        return;
    }

    let board = &*board;
    let palette = CellPalette::new(style.theme.board(), by_age);
    let across = cols.div_ceil(RUN_CELLS);
    let aged = &mut *style.ages;
    let (texture, images) = (&mut *texture, &mut *images);

    if !by_age {
        let mut paint = |run: RowCells| {
            if run.first >= len {
                return;
            }
            let Some(texels) = texture.row_texels(images, run.first) else {
                return;
            };
            for (idx, alive) in run.iter() {
                palette.paint(texels, (idx - run.first) * 4, alive, 0);
            }
        };
        match dirty {
            DirtyCells::Cells(changed) if !recolor => changed.into_iter().for_each(paint),
            DirtyCells::None if !recolor => {}
            _ => board.row_cells().for_each(&mut paint),
        }
        texture.upload(images);
        return;
    }

    // Switching the style on starts from a calm board: nobody knows the ages yet.
    let reset = style.cell_age.is_changed() || !ages_known || dirty == DirtyCells::Full;
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
                // Texels of a row sit side by side on its tile, as its cells do.
                let mut texels = texture.row_texels(images, run.first);
                for (idx, alive) in (RowCells {
                    cells: *cells,
                    ..run
                })
                .iter()
                {
                    let (age, bit) = (&mut ages[idx], idx - run.first);
                    if *age < SETTLED_AGE {
                        *age = age.saturating_add(steps);
                        if let Some(texels) = texels.as_deref_mut() {
                            palette.paint(texels, bit * 4, alive, *age);
                        }
                    }
                    if *age >= SETTLED_AGE {
                        *cells &= !(1 << bit);
                    }
                }
                *cells != 0
            });
        }
        if let DirtyCells::Cells(changed) = dirty {
            for run in changed.into_iter().filter(|run| run.first < len) {
                let mut texels = texture.row_texels(images, run.first);
                for (idx, alive) in run.iter() {
                    // A generation gives births and deaths age 0; hand edits look settled.
                    let age = if steps > 0 { 0 } else { settled_age(alive) };
                    aged.ages[idx] = age;
                    if let Some(texels) = texels.as_deref_mut() {
                        palette.paint(texels, (idx - run.first) * 4, alive, age);
                    }
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
            let Some(texels) = texture.row_texels(images, run.first) else {
                continue;
            };
            for (&age, bit) in ages[run.first..][..run.len()].iter().zip(0..) {
                let alive = run.alive >> bit & 1 != 0;
                palette.paint(texels, bit * 4, alive, age);
            }
        }
    }
    texture.upload(images);
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

/// The pulse fades on its own, long after the input that started it.
fn keep_awake_while_pulsing(pulse: Res<BoundaryWallPulse>, mut awake: MessageWriter<StayAwake>) {
    if pulse.active {
        awake.write(StayAwake);
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
    cursor: Res<crate::input_plugin::BoardCursor>,
) {
    // The keyboard's cell cursor stands in for the pointer while it shows.
    hovered.0 = cursor.shown();
    if hovered.0.is_some() || pointer_capture.pointer {
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
    use crate::coords::cell_to_world;

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

    /// A world with `app`'s finite board on blank tiles, ready for [`sync_cells`].
    fn board_world(app: GolApp, by_age: bool) -> World {
        let mut world = World::new();
        fill_board_world(&mut world, app, by_age);
        world
    }

    fn fill_board_world(world: &mut World, app: GolApp, by_age: bool) {
        let (cols, rows) = app.board.size().unwrap();
        world.insert_resource(Simulation::new(app));
        world.insert_resource(RedrawRequested(true));
        world.insert_resource(Theme::Dark);
        world.insert_resource(CellAge { enabled: by_age });
        world.init_resource::<CellAges>();
        world.init_resource::<Assets<Image>>();
        let mut images = world.resource_mut::<Assets<Image>>();
        let texture = BoardTexture::new(&mut images, cols, rows, Theme::Dark.board().dead);
        world.insert_resource(texture);
    }

    /// The tiles put back together into the one texture a whole board used to
    /// be drawn on: a texel per cell, rows from the top of the board down.
    fn board_texels(world: &World) -> Vec<u8> {
        let texture = world.resource::<BoardTexture>();
        let images = world.resource::<Assets<Image>>();
        let TileLayout { cols, rows } = texture.layout;
        let mut whole = vec![0; cols * rows * 4];
        for (tile, handle) in texture.tiles.iter().enumerate() {
            let (col, row, width, height) = texture.layout.rect(tile);
            let image = images.get(handle).unwrap();
            assert_eq!(image.size(), UVec2::new(width as u32, height as u32));
            assert_eq!(image.sampler, ImageSampler::nearest());
            let data = image.data.as_ref().unwrap();
            // A tile's texel rows run top down too, from its top row of cells.
            for (y, line) in data.chunks_exact(width * 4).enumerate() {
                let board_row = row + height - 1 - y;
                let start = ((rows - 1 - board_row) * cols + col) * 4;
                whole[start..][..width * 4].copy_from_slice(line);
            }
        }
        whole
    }

    fn texel_at(texels: &[u8], idx: usize, cols: usize, rows: usize) -> [u8; 4] {
        let offset = texel_offset(idx, cols, rows);
        texels[offset..offset + 4].try_into().unwrap()
    }

    #[test]
    fn restored_board_dimensions_rebuild_the_texture_and_repaint_loaded_cells() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = GolApp::new(16, 24);
        app.populate_board(15, 23);
        let mut world = board_world(GolApp::new(8, 8), false);
        world.resource_mut::<Simulation>().app = app;
        world.insert_resource(BoardSettings {
            cols: 16,
            rows: 24,
            fill_canvas: false,
        });
        let board = world
            .spawn((BoardSprite, Transform::default(), Visibility::default()))
            .id();
        let old_tile = world
            .spawn((BoardTile, Sprite::default(), ChildOf(board)))
            .id();
        world.run_system_once(apply_board_settings).unwrap();
        let texture = world.resource::<BoardTexture>();
        assert_eq!(texture.layout, TileLayout { cols: 16, rows: 24 });
        let tiles = texture.tiles.clone();
        assert!(world.get_entity(old_tile).is_err());
        let sprites: Vec<_> = world
            .query_filtered::<(&Sprite, &ChildOf), With<BoardTile>>()
            .iter(&world)
            .map(|(sprite, parent)| (sprite.image.clone(), parent.parent()))
            .collect();
        assert_eq!(sprites, [(tiles[0].clone(), board)]);

        world.run_system_once(sync_cells).unwrap();
        assert_eq!(
            texel_at(&board_texels(&world), 23 * 16 + 15, 16, 24),
            color_to_texel(Theme::Dark.board().live)
        );
    }

    #[test]
    fn syncing_cells_leaves_the_texture_alone_on_the_unbounded_board() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = board_world(GolApp::new(4, 3), false);
        let app = &mut world.resource_mut::<Simulation>().app;
        app.set_unbounded().unwrap();
        app.board.populate(1, 2);
        world.run_system_once(sync_cells).unwrap();

        let texels = board_texels(&world);
        let dead = color_to_texel(Theme::Dark.board().dead);
        assert!(texels.as_chunks::<4>().0.iter().all(|&texel| texel == dead));
    }

    #[test]
    fn switching_themes_repaints_every_cell_without_a_board_change() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = GolApp::new(4, 3);
        app.board.populate(1, 2);
        let mut world = board_world(app, false);
        world.run_system_once(sync_cells).unwrap();

        // Nothing on the board changes, so only the theme can cause a repaint.
        world.resource_mut::<RedrawRequested>().0 = false;
        *world.resource_mut::<Theme>() = Theme::Light;
        world.run_system_once(sync_cells).unwrap();
        let colors = Theme::Light.board();
        let texels = board_texels(&world);
        for idx in 0..12 {
            let expected = if idx == 2 * 4 + 1 {
                colors.live
            } else {
                colors.dead
            };
            assert_eq!(
                texel_at(&texels, idx, 4, 3),
                color_to_texel(expected),
                "{idx}"
            );
        }
    }

    #[test]
    fn generations_shade_births_deaths_and_survivors_but_edits_look_settled() {
        // A horizontal blinker in the middle row.
        let mut app = GolApp::new(5, 5);
        for col in 1..=3 {
            app.board.populate(col, 2);
        }
        let mut world = board_world(app, true);
        // A registered system remembers what it has seen, unlike `run_system_once`.
        let sync = world.register_system(sync_cells);
        let texel = |world: &World, col: usize, row: usize| {
            texel_at(&board_texels(world), row * 5 + col, 5, 5)
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
    fn a_board_of_several_tiles_paints_every_cell_at_its_own_texel() {
        // Cells are stored a chunk at a time and drawn a tile at a time, so this
        // is where a cell could land on a neighbour's texel: past columns 64 and
        // 256, past rows 64 and 256, and at the edges of the tiles cut short.
        let (cols, rows) = (300, 270);
        for by_age in [false, true] {
            let mut app = GolApp::new(cols, rows);
            let noise = crate::board::RandomizeSettings::new(30, Some(5)).unwrap();
            app.set_randomize_settings(noise);
            app.randomize_board();
            let mut world = board_world(app, by_age);
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
                    // A cell drawn or erased by hand, by the corner of four tiles.
                    let (col, row) = (250 + frame, 255);
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
                let texels = board_texels(&world);
                for idx in 0..cols * rows {
                    assert_eq!(
                        texel_at(&texels, idx, cols, rows),
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
    fn tile_sprites_cover_their_own_cells_and_together_the_board() {
        let (cols, rows) = (300, 270);
        let mut world = board_world(GolApp::new(cols, rows), false);
        let board = world
            .spawn((BoardSprite, Transform::default(), Visibility::default()))
            .id();
        world.resource_scope(|world, texture: Mut<BoardTexture>| {
            texture.spawn_sprites(&mut world.commands(), board);
        });
        world.flush();

        let texture = world.resource::<BoardTexture>();
        let (tiles, layout) = (texture.tiles.clone(), texture.layout);
        let sprites: Vec<_> = world
            .query_filtered::<(&Sprite, &Transform, &ChildOf), With<BoardTile>>()
            .iter(&world)
            .map(|(sprite, transform, parent)| {
                assert_eq!(parent.parent(), board);
                let rect = Rect::from_center_size(
                    transform.translation.truncate(),
                    sprite.custom_size.unwrap(),
                );
                (sprite.image.clone(), rect)
            })
            .collect();
        assert_eq!(sprites.len(), 4);
        let mut area = 0.0;
        for (tile, handle) in tiles.iter().enumerate() {
            let (col, row, width, height) = layout.rect(tile);
            let rects: Vec<_> = sprites
                .iter()
                .filter(|(image, _)| image == handle)
                .map(|(_, rect)| *rect)
                .collect();
            // Its corners are the outer corners of its corner cells.
            let half = Vec2::splat(CELL_SIZE / 2.0);
            let low = cell_to_world(col as i64, row as i64, cols, rows).truncate() - half;
            let (right, top) = ((col + width - 1) as i64, (row + height - 1) as i64);
            let high = cell_to_world(right, top, cols, rows).truncate() + half;
            assert_eq!(rects, [Rect::from_corners(low, high)], "tile {tile}");
            area += rects[0].size().x * rects[0].size().y;
        }
        let whole = board_sprite_size(cols, rows);
        assert_eq!(area, whole.x * whole.y);
    }

    #[test]
    fn a_frame_sends_the_gpu_only_the_tiles_it_painted_on() {
        use bevy::{app::TaskPoolPlugin, asset::AssetPlugin, ecs::message::Messages};
        for by_age in [false, true] {
            let mut app = App::new();
            app.add_plugins((
                TaskPoolPlugin::default(),
                AssetPlugin {
                    watch_for_changes_override: Some(false),
                    use_asset_processor_override: Some(false),
                    ..default()
                },
            ))
            .init_asset::<Image>()
            .add_systems(Update, sync_cells);
            fill_board_world(app.world_mut(), GolApp::new(1024, 1024), by_age);
            // The tiles a frame modified, after `change` to the board.
            let mut frame = |change: &dyn Fn(&mut GolApp)| {
                change(&mut app.world_mut().resource_mut::<Simulation>().app);
                app.world_mut().resource_mut::<RedrawRequested>().0 = true;
                app.update();
                let tiles = app.world().resource::<BoardTexture>().tiles.clone();
                let mut events = app
                    .world_mut()
                    .resource_mut::<Messages<AssetEvent<Image>>>();
                let modified: Vec<_> = events.drain().collect();
                (0..tiles.len())
                    .filter(|&tile| {
                        let id = tiles[tile].id();
                        modified.iter().any(|event| event.is_modified(id))
                    })
                    .collect::<Vec<_>>()
            };
            let (nothing, none) = (|_: &mut GolApp| {}, [0usize; 0]);

            assert_eq!(frame(&nothing), (0..16).collect::<Vec<_>>(), "first paint");
            assert_eq!(frame(&nothing), none, "nothing changed, by age: {by_age}");
            // Column 600 and row 300 are on the third tile of the second row.
            assert_eq!(frame(&|app| app.populate_board(600, 300)), [6]);
            assert_eq!(frame(&nothing), none, "by age: {by_age}");
            // The lone cell dies, and its trail fades on its own tile.
            for generation in 0..=SETTLED_AGE {
                let tiles = frame(&|app| app.increment_epoch());
                if by_age || generation == 0 {
                    assert_eq!(tiles, [6], "generation {generation}, by age: {by_age}");
                } else {
                    assert_eq!(tiles, none, "generation {generation}");
                }
            }
            // Settled, so nothing is left to paint.
            assert_eq!(
                frame(&|app| app.increment_epoch()),
                none,
                "by age: {by_age}"
            );
        }
    }

    #[test]
    fn only_cells_still_ageing_are_visited() {
        let mut app = GolApp::new(200, 150);
        // A block, which never changes, and a blinker, which never stops.
        for (col, row) in [(100, 100), (101, 100), (100, 101), (101, 101)] {
            app.board.populate(col, row);
        }
        for col in 9..12 {
            app.board.populate(col, 10);
        }
        let mut world = board_world(app, true);
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
