//! Draws the unbounded board. There is no board-sized texture to keep up to date,
//! so one the size of the canvas shows whatever part of the plane is in view,
//! and a zoomed-out view merges cells into blocks that fit its pixels.
use crate::{
    render_plugin::{BoardSprite, CellPalette, CellStyle, SETTLED_AGE, settled_age, texel_offset},
    shared::{CELL_SIZE, MainCamera, RedrawRequested, Simulation, StartupSet, UpdateSet},
    sparse_board::{BlockGrid, CHUNK_SIZE, CellRect, Chunk, ChunkKey, DirtyChunks, SparseBoard},
    universe::Universe,
};
use bevy::{
    asset::RenderAssetUsages,
    ecs::system::SystemParam,
    image::ImageSampler,
    platform::collections::HashMap,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    window::PrimaryWindow,
};

const CHUNK_CELLS: usize = (CHUNK_SIZE * CHUNK_SIZE) as usize;

/// Blocks more than the view needs, so a view that doesn't line up with blocks
/// is still covered, and panning keeps the texture the same size.
const GRID_SPARE: usize = 2;

pub struct PlaneViewPlugin;

impl Plugin for PlaneViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlaneCanvas>()
            .add_systems(Startup, spawn_plane_sprite.in_set(StartupSet::BuildWorld))
            .add_systems(Update, sync_plane.in_set(UpdateSet::Render));
    }
}

#[derive(Component)]
struct PlaneSprite;

/// The texture over the view, and the state of what was last painted on it.
#[derive(Resource, Default)]
struct PlaneCanvas {
    handle: Handle<Image>,
    grid: Option<BlockGrid>,
    ages: PlaneAges,
}

/// Generations since each cell of the chunks in view last changed, as
/// [`CellAges`](crate::render_plugin) keeps them for a finite board. The view is
/// all that needs them, so memory follows the canvas and not the pattern, and a
/// chunk scrolling into view starts out settled.
#[derive(Default)]
struct PlaneAges {
    chunks: HashMap<ChunkKey, Box<[u8; CHUNK_CELLS]>>,
    /// The generation counter at the last sync, to tell how many generations ran since.
    epoch: usize,
}

fn settled_ages(chunk: &Chunk) -> Box<[u8; CHUNK_CELLS]> {
    let mut ages = Box::new([0; CHUNK_CELLS]);
    for (index, age) in ages.iter_mut().enumerate() {
        *age = settled_age(is_set(chunk, index));
    }
    ages
}

fn is_set(chunk: &Chunk, index: usize) -> bool {
    let size = CHUNK_SIZE as usize;
    chunk[index / size] >> (index % size) & 1 != 0
}

impl PlaneAges {
    /// Brings the ages up to date with `steps` generations and the cells in `dirty`.
    fn update(&mut self, plane: &SparseBoard, grid: BlockGrid, dirty: DirtyChunks, steps: u8) {
        if dirty == DirtyChunks::Full {
            self.chunks.clear();
        }

        let mut in_view = Vec::new();
        plane.for_each_chunk_in(grid, |key, _| in_view.push(key));
        let (first, last) = grid.chunk_span();
        self.chunks.retain(|key, _| {
            (first.0..=last.0).contains(&key.0) && (first.1..=last.1).contains(&key.1)
        });
        if steps > 0 {
            for ages in self.chunks.values_mut() {
                for age in ages.iter_mut().filter(|age| **age < SETTLED_AGE) {
                    *age = age.saturating_add(steps);
                }
            }
        }
        for key in in_view {
            self.chunks
                .entry(key)
                .or_insert_with(|| settled_ages(plane.chunk(key).unwrap_or(&EMPTY_CHUNK)));
        }

        let DirtyChunks::Chunks(dirty) = dirty else {
            return;
        };
        let size = CHUNK_SIZE as usize;
        for (key, flipped) in dirty {
            if !(first.0..=last.0).contains(&key.0) || !(first.1..=last.1).contains(&key.1) {
                continue;
            }
            let cells = plane.chunk(key).unwrap_or(&EMPTY_CHUNK);
            // A chunk that just emptied has only its trail left to show.
            let ages = self
                .chunks
                .entry(key)
                .or_insert_with(|| settled_ages(&EMPTY_CHUNK));
            for index in 0..CHUNK_CELLS {
                if flipped[index / size] >> (index % size) & 1 != 0 {
                    // A generation gives births and deaths age 0; hand edits look settled.
                    ages[index] = if steps > 0 {
                        0
                    } else {
                        settled_age(is_set(cells, index))
                    };
                }
            }
        }
    }
}

static EMPTY_CHUNK: Chunk = [0; CHUNK_SIZE as usize];

/// The blocks to draw for a view of the world rectangle `view`. Blocks are as
/// small as the `cells_per_pixel` of the view allows, and aligned to their own
/// size so that panning moves them whole.
fn view_grid(view: Rect, origin: (i64, i64), cells_per_pixel: f32) -> BlockGrid {
    let level = cells_per_pixel.max(1.0).log2().ceil() as u32;
    let block = 1i64 << level;
    let cells = view.size() / CELL_SIZE;
    let blocks = |cells: f32| (cells / block as f32).ceil() as usize + GRID_SPARE;
    let start = |world: f32, origin: i64| {
        ((world / CELL_SIZE).floor() as i64 + origin).div_euclid(block) * block
    };
    BlockGrid {
        left: start(view.min.x, origin.0),
        bottom: start(view.min.y, origin.1),
        level,
        width: blocks(cells.x),
        height: blocks(cells.y),
    }
}

/// The cells the world rectangle `view` touches.
pub(crate) fn view_cells(view: Rect, origin: (i64, i64)) -> CellRect {
    let (min, max) = (
        (view.min / CELL_SIZE).floor().as_i64vec2(),
        (view.max / CELL_SIZE).ceil().as_i64vec2(),
    );
    CellRect {
        left: min.x + origin.0,
        bottom: min.y + origin.1,
        width: (max.x - min.x) as usize,
        height: (max.y - min.y) as usize,
    }
}

/// The world rectangle a grid covers, for placing its sprite.
pub(crate) fn grid_rect(grid: BlockGrid, origin: (i64, i64)) -> Rect {
    let size =
        Vec2::new(grid.width as f32, grid.height as f32) * (1i64 << grid.level) as f32 * CELL_SIZE;
    let min = Vec2::new(
        (grid.left - origin.0) as f32,
        (grid.bottom - origin.1) as f32,
    ) * CELL_SIZE;
    Rect::from_corners(min, min + size)
}

/// Paints the grid's blocks into `data`, with the rows of texture top down as
/// usual. `ages` shows each cell's age, and needs blocks of one cell each.
fn paint(
    data: &mut [u8],
    plane: &SparseBoard,
    grid: BlockGrid,
    palette: &CellPalette,
    ages: Option<&PlaneAges>,
) {
    let dead = palette.texel(false, settled_age(false));
    for texel in data.as_chunks_mut::<4>().0 {
        *texel = dead;
    }

    let (width, height) = (grid.width, grid.height);
    let live = palette.texel(true, settled_age(true));
    plane.for_each_live_block(grid, |col, row| {
        let offset = texel_offset(row * width + col, width, height);
        data[offset..offset + 4].copy_from_slice(&live);
    });

    let Some(ages) = ages else {
        return;
    };
    // Dead cells have their settled colour already, and most of a view is those.
    let settled_dead = settled_age(false);
    let size = CHUNK_SIZE as usize;
    for (&key, chunk_ages) in &ages.chunks {
        let cells = plane.chunk(key).unwrap_or(&EMPTY_CHUNK);
        let (left, bottom) = (key.0 * CHUNK_SIZE, key.1 * CHUNK_SIZE);
        // The columns of the chunk that are in view, and which texels they land on.
        let first_col = (grid.left - left).clamp(0, CHUNK_SIZE) as usize;
        let last_col = (grid.left + width as i64 - left).clamp(0, CHUNK_SIZE) as usize;
        let first_texel = (left + first_col as i64 - grid.left) as usize;
        for (row, row_ages) in chunk_ages.chunks_exact(size).enumerate() {
            let y = bottom + row as i64 - grid.bottom;
            if !(0..height as i64).contains(&y) {
                continue;
            }
            let start = texel_offset(y as usize * width + first_texel, width, height);
            let texels = data[start..].as_chunks_mut::<4>().0;
            let line = row_ages[first_col..last_col].iter().zip(texels);
            for (col, (&age, texel)) in line.enumerate() {
                let alive = cells[row] >> (first_col + col) & 1 != 0;
                if alive || age != settled_dead {
                    *texel = palette.texel(alive, age);
                }
            }
        }
    }
}

fn spawn_plane_sprite(
    mut commands: Commands,
    mut images: ResMut<Assets<Image>>,
    mut canvas: ResMut<PlaneCanvas>,
) {
    let mut image = Image::new_fill(
        Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[0; 4],
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    // Cells are crisp squares; filtering would smear neighbours together.
    image.sampler = ImageSampler::nearest();
    canvas.handle = images.add(image);
    commands.spawn((
        Sprite::from_image(canvas.handle.clone()),
        Transform::default(),
        Visibility::Hidden,
        PlaneSprite,
    ));
}

type PlaneSpriteParts = (
    &'static mut Sprite,
    &'static mut Transform,
    &'static mut Visibility,
);
type OnlyPlaneSprite = (With<PlaneSprite>, Without<MainCamera>, Without<BoardSprite>);

/// The two sprites, of which one shows.
#[derive(SystemParam)]
struct BoardSprites<'w, 's> {
    plane: Query<'w, 's, PlaneSpriteParts, OnlyPlaneSprite>,
    board: Query<'w, 's, &'static mut Visibility, (With<BoardSprite>, Without<PlaneSprite>)>,
}

#[derive(SystemParam)]
struct PlaneViewpoint<'w, 's> {
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    cameras:
        Query<'w, 's, (&'static Camera, &'static Transform, &'static Projection), With<MainCamera>>,
}

impl PlaneViewpoint<'_, '_> {
    /// The world rectangle the camera sees, and how many cells share a pixel there.
    fn view(&self) -> Option<(Rect, f32)> {
        let window = self.windows.single().ok()?;
        let (camera, transform, projection) = self.cameras.single().ok()?;
        let Projection::Orthographic(ortho) = projection else {
            return None;
        };
        let size = camera.logical_viewport_size()? * ortho.scale;
        let view = Rect::from_center_size(transform.translation.truncate(), size);
        Some((view, ortho.scale / (CELL_SIZE * window.scale_factor())))
    }
}

fn sync_plane(
    redraw_requested: Res<RedrawRequested>,
    style: CellStyle,
    mut sim: ResMut<Simulation>,
    viewpoint: PlaneViewpoint,
    mut images: ResMut<Assets<Image>>,
    mut canvas: ResMut<PlaneCanvas>,
    mut sprites: BoardSprites,
) {
    let unbounded = sim.app.board.finite().is_none();
    let Ok((mut sprite, mut transform, mut visibility)) = sprites.plane.single_mut() else {
        return;
    };
    visibility.set_if_neq(if unbounded {
        Visibility::Visible
    } else {
        Visibility::Hidden
    });
    for mut visibility in &mut sprites.board {
        visibility.set_if_neq(if unbounded {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
    }
    if !unbounded {
        canvas.grid = None;
        canvas.ages.chunks.clear();
        return;
    }
    let Some((view, cells_per_pixel)) = viewpoint.view() else {
        return;
    };

    let origin = sim.plane_origin;
    sim.app.set_view_cells(view_cells(view, origin));
    let grid = view_grid(view, origin, cells_per_pixel);
    // The grid's cells stay put in world space only until the origin moves.
    let rect = grid_rect(grid, origin);
    transform.translation = rect.center().extend(0.0);
    sprite.custom_size = Some(rect.size());

    let recolor = style.theme.is_changed() || style.cell_age.is_changed();
    if !redraw_requested.0 && !recolor && canvas.grid == Some(grid) {
        return;
    }

    let app = &mut sim.app;
    let epoch = app.epoch;
    let Universe::Unbounded(plane) = &mut app.board else {
        return;
    };
    let dirty = plane.take_dirty_chunks();
    let plane = &*plane;

    let canvas = &mut *canvas;
    // Undo and resetting the counter move it backwards; neither is a generation.
    let steps = u8::try_from(epoch.saturating_sub(canvas.ages.epoch)).unwrap_or(u8::MAX);
    canvas.ages.epoch = epoch;
    let by_age = style.cell_age.enabled;
    // Ages need a texel per cell to show, and can't be told apart otherwise.
    let ages = by_age && grid.level == 0;
    if style.cell_age.is_changed() || !ages {
        canvas.ages.chunks.clear();
    }
    if ages {
        canvas.ages.update(plane, grid, dirty, steps);
    }
    canvas.grid = Some(grid);

    let Some(mut image) = images.get_mut(&canvas.handle) else {
        return;
    };
    let size = Extent3d {
        width: grid.width as u32,
        height: grid.height as u32,
        depth_or_array_layers: 1,
    };
    if image.texture_descriptor.size != size {
        image.resize(size);
    }
    let Some(data) = image.data.as_mut() else {
        return;
    };
    let palette = CellPalette::new(style.theme.board(), by_age);
    paint(data, plane, grid, &palette, ages.then_some(&canvas.ages));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render_plugin::color_to_texel;
    use crate::shared::Theme;

    fn plane_with(cells: impl IntoIterator<Item = (i64, i64)>) -> SparseBoard {
        let mut plane = SparseBoard::default();
        for (x, y) in cells {
            plane.populate(x, y);
        }
        plane
    }

    fn texel(data: &[u8], grid: BlockGrid, col: usize, row: usize) -> [u8; 4] {
        let offset = texel_offset(row * grid.width + col, grid.width, grid.height);
        data[offset..offset + 4].try_into().unwrap()
    }

    fn view_of(cells: f32, centre: Vec2) -> Rect {
        Rect::from_center_size(centre, Vec2::splat(cells * CELL_SIZE))
    }

    #[test]
    fn a_view_is_covered_at_every_scale_and_position() {
        for (cells_per_pixel, centre) in [
            (0.25, Vec2::ZERO),
            (1.0, Vec2::new(-5000.0, 1234.5)),
            (3.0, Vec2::new(70.0, -70.0)),
            (1000.0, Vec2::new(-1.0e6, 3.0e5)),
        ] {
            let view = view_of(100.0 * cells_per_pixel, centre);
            let origin = (-7_000_000_001, 9);
            let grid = view_grid(view, origin, cells_per_pixel);
            let covered = grid_rect(grid, origin);
            assert!(covered.min.cmple(view.min).all(), "{cells_per_pixel}");
            assert!(covered.max.cmpge(view.max).all(), "{cells_per_pixel}");
        }
    }

    #[test]
    fn blocks_are_no_larger_than_a_pixel_needs_and_stay_on_their_own_grid() {
        let grid = |cells_per_pixel| view_grid(view_of(50.0, Vec2::ZERO), (5, -5), cells_per_pixel);
        assert_eq!(grid(0.5).level, 0);
        assert_eq!(grid(1.0).level, 0);
        assert_eq!(grid(1.5).level, 1);
        assert_eq!(grid(2.0).level, 1);
        assert_eq!(grid(100.0).level, 7);
        for level in [1, 4, 7] {
            let grid = grid((1 << level) as f32);
            assert_eq!(grid.left.rem_euclid(1 << level), 0);
            assert_eq!(grid.bottom.rem_euclid(1 << level), 0);
        }
    }

    #[test]
    fn panning_keeps_the_texture_size() {
        let sizes: Vec<_> = (0..40)
            .map(|step| {
                let view = view_of(100.0, Vec2::new(step as f32 * 7.3, step as f32 * -3.1));
                let grid = view_grid(view, (0, 0), 1.0);
                (grid.width, grid.height)
            })
            .collect();
        assert!(sizes.iter().all(|size| *size == sizes[0]), "{sizes:?}");
    }

    #[test]
    fn a_grid_lands_the_same_cells_on_the_same_world_spots_after_the_origin_moves() {
        let view = view_of(64.0, Vec2::new(120.0, 36.0));
        let grid = view_grid(view, (1000, -1000), 1.0);

        // The same cells, seen from an origin ten cells over.
        let shift = Vec2::splat(10.0 * CELL_SIZE);
        let rebased = Rect::from_center_size(view.center() - shift, view.size());
        assert_eq!(view_grid(rebased, (1010, -990), 1.0), grid);
    }

    #[test]
    fn painting_puts_live_cells_on_their_texels_with_row_zero_at_the_bottom() {
        let colors = Theme::Dark.board();
        let palette = CellPalette::new(colors, false);
        let grid = BlockGrid {
            left: -2,
            bottom: -1,
            level: 0,
            width: 4,
            height: 3,
        };
        let plane = plane_with([(-2, -1), (1, 1), (0, 5), (9, 0)]);
        let mut data = vec![7; 4 * 3 * 4];
        paint(&mut data, &plane, grid, &palette, None);

        let (live, dead) = (color_to_texel(colors.live), color_to_texel(colors.dead));
        for row in 0..3 {
            for col in 0..4 {
                let expected = if [(0, 0), (3, 2)].contains(&(col, row)) {
                    live
                } else {
                    dead
                };
                assert_eq!(texel(&data, grid, col, row), expected, "({col}, {row})");
            }
        }
        // Texture rows run top down, so the top row of cells comes first.
        assert_eq!(data[3 * 4..4 * 4], live);
    }

    #[test]
    fn a_zoomed_out_block_lights_for_any_cell_inside_it() {
        let colors = Theme::Dark.board();
        let palette = CellPalette::new(colors, false);
        let grid = BlockGrid {
            left: 0,
            bottom: 0,
            level: 3,
            width: 2,
            height: 1,
        };
        let plane = plane_with([(15, 7)]);
        let mut data = vec![0; 2 * 4];
        paint(&mut data, &plane, grid, &palette, None);
        assert_eq!(texel(&data, grid, 0, 0), color_to_texel(colors.dead));
        assert_eq!(texel(&data, grid, 1, 0), color_to_texel(colors.live));
    }

    /// The ages after one generation of a blinker across two chunks.
    fn blinker_ages() -> (SparseBoard, PlaneAges, BlockGrid) {
        let grid = BlockGrid {
            left: -64,
            bottom: 0,
            level: 0,
            width: 128,
            height: 64,
        };
        let mut plane = plane_with([(-1, 5), (0, 5), (1, 5)]);
        let mut ages = PlaneAges::default();
        let dirty = plane.take_dirty_chunks();
        ages.update(&plane, grid, dirty, 0);
        plane.step();
        let dirty = plane.take_dirty_chunks();
        ages.update(&plane, grid, dirty, 1);
        (plane, ages, grid)
    }

    fn age_of(ages: &PlaneAges, x: i64, y: i64) -> u8 {
        let key = (x.div_euclid(CHUNK_SIZE), y.div_euclid(CHUNK_SIZE));
        let (col, row) = (x.rem_euclid(CHUNK_SIZE), y.rem_euclid(CHUNK_SIZE));
        ages.chunks[&key][(row * CHUNK_SIZE + col) as usize]
    }

    #[test]
    fn a_generation_ages_the_cells_it_flipped_from_zero() {
        let (_, ages, _) = blinker_ages();
        assert_eq!(age_of(&ages, 0, 4), 0, "born");
        assert_eq!(age_of(&ages, 1, 5), 0, "died");
        // In a chunk that has just emptied, whose trail still has to fade.
        assert_eq!(age_of(&ages, -1, 5), 0, "died");
        assert_eq!(age_of(&ages, 0, 5), settled_age(true) + 1, "unchanged");
    }

    #[test]
    fn hand_edits_look_settled() {
        let (mut plane, mut ages, grid) = blinker_ages();
        plane.populate(20, 20);
        let dirty = plane.take_dirty_chunks();
        ages.update(&plane, grid, dirty, 0);
        assert_eq!(age_of(&ages, 20, 20), settled_age(true));
    }

    #[test]
    fn a_full_redraw_calms_the_ages() {
        let (plane, mut ages, grid) = blinker_ages();
        ages.update(&plane, grid, DirtyChunks::Full, 0);
        assert_eq!(age_of(&ages, 0, 4), settled_age(true));
        assert!(!ages.chunks.contains_key(&(-1, 0)), "nothing lives there");
    }

    #[test]
    fn ages_are_only_kept_for_the_chunks_in_view() {
        let (plane, mut ages, _) = blinker_ages();
        let away = BlockGrid {
            left: 6400,
            bottom: 6400,
            level: 0,
            width: 64,
            height: 64,
        };
        ages.update(&plane, away, DirtyChunks::None, 1);
        assert!(ages.chunks.is_empty());
    }

    #[test]
    fn trails_are_clipped_to_a_view_that_starts_and_ends_inside_a_chunk() {
        let (mut plane, mut ages, _) = blinker_ages();
        // Just past the right of the view, where it mustn't spill into the next row.
        plane.populate(3, 5);
        let grid = BlockGrid {
            left: 1,
            bottom: 4,
            level: 0,
            width: 2,
            height: 2,
        };
        ages.update(&plane, grid, DirtyChunks::None, 0);
        let colors = Theme::Dark.board();
        let palette = CellPalette::new(colors, true);
        let mut data = vec![0; 2 * 2 * 4];
        paint(&mut data, &plane, grid, &palette, Some(&ages));
        // Cell (1, 5) died this generation, and its neighbours are all out of view.
        assert_eq!(texel(&data, grid, 0, 1), palette.texel(false, 0));
        let dead = palette.texel(false, settled_age(false));
        for (col, row) in [(0, 0), (1, 0), (1, 1)] {
            assert_eq!(texel(&data, grid, col, row), dead, "({col}, {row})");
        }
    }

    #[test]
    fn trails_are_painted_onto_the_texels_of_dead_cells() {
        let (plane, ages, grid) = blinker_ages();
        let colors = Theme::Dark.board();
        let palette = CellPalette::new(colors, true);
        let mut data = vec![0; grid.width * grid.height * 4];
        paint(&mut data, &plane, grid, &palette, Some(&ages));
        // Column 64 is cell 0, and the blinker's middle cell stayed alive.
        assert_eq!(texel(&data, grid, 65, 5), palette.texel(false, 0));
        assert_eq!(texel(&data, grid, 63, 5), palette.texel(false, 0));
        assert_eq!(texel(&data, grid, 64, 4), palette.texel(true, 0));
        assert_eq!(
            texel(&data, grid, 64, 5),
            palette.texel(true, settled_age(true) + 1)
        );
    }

    #[test]
    fn the_cells_in_view_take_in_any_it_only_touches() {
        let view = Rect::new(
            -1.5 * CELL_SIZE,
            2.0 * CELL_SIZE,
            1.5 * CELL_SIZE,
            2.5 * CELL_SIZE,
        );
        let cells = view_cells(view, (1 << 40, -5));
        assert_eq!(
            cells,
            CellRect {
                left: (1 << 40) - 2,
                bottom: -3,
                width: 4,
                height: 1,
            }
        );
    }

    #[test]
    fn drawing_the_plane_tells_the_app_what_is_in_view() {
        use bevy::{
            camera::{ComputedCameraValues, RenderTargetInfo},
            ecs::system::RunSystemOnce,
        };
        let mut app = crate::app::App::new(16, 16);
        app.set_unbounded().unwrap();
        let mut sim = Simulation::new(app);
        sim.plane_origin = (1000, -1000);
        let mut world = World::new();
        world.insert_resource(sim);
        world.init_resource::<RedrawRequested>();
        world.init_resource::<Theme>();
        world.init_resource::<crate::shared::CellAge>();
        world.init_resource::<crate::render_plugin::CellAges>();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<PlaneCanvas>();
        world.spawn((Window::default(), PrimaryWindow));
        world.spawn((
            Sprite::default(),
            Transform::default(),
            Visibility::Hidden,
            PlaneSprite,
        ));
        let size = UVec2::new(400, 200);
        world.spawn((
            Camera2d,
            MainCamera,
            Camera {
                viewport: Some(bevy::camera::Viewport {
                    physical_size: size,
                    ..default()
                }),
                computed: ComputedCameraValues {
                    target_info: Some(RenderTargetInfo {
                        physical_size: size,
                        scale_factor: 1.0,
                    }),
                    ..default()
                },
                ..default()
            },
            Projection::Orthographic(OrthographicProjection {
                scale: CELL_SIZE,
                ..OrthographicProjection::default_2d()
            }),
        ));
        world.run_system_once(sync_plane).unwrap();

        // One cell a point, around the plane's origin.
        let area = world.resource::<Simulation>().app.randomize_area().unwrap();
        assert_eq!(
            area,
            CellRect {
                left: 800,
                bottom: -1100,
                width: 400,
                height: 200,
            }
        );
    }
}
