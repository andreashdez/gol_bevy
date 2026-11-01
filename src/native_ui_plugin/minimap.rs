//! A map of the whole board that appears while part of it is out of view, with
//! a frame around what the canvas shows. Clicking or dragging moves the view.
//! The unbounded board has no whole, so its map runs from the live cells to the
//! view and appears while any of those cells are out of sight.
//! Like the icons, the map is a white mask tinted by [`ThemedImage`].
use super::*;
use crate::{
    plane_view::{grid_rect, view_cells},
    shared::{CELL_SIZE, MainCamera},
    sparse_board::{BlockGrid, CellRect, SparseBoard},
    universe::Universe,
};
use bevy::{asset::uuid::Uuid, image::ImageSampler};
use std::marker::PhantomData;

const MAX_SIDE: f32 = 140.0;
/// The share of the canvas's shorter side the map may take.
const MAX_SHARE_OF_CANVAS: f32 = 0.4;
/// Very wide or tall boards would otherwise leave nothing to click.
const MIN_SIDE: f32 = 16.0;
const MARGIN: f32 = 12.0;
const BORDER: f32 = 1.0;
/// Twice the largest side, for Retina displays.
const MAX_TEXELS: usize = 280;
/// A quarter of those on the unbounded board, whose cells tend to be specks a
/// long way apart, each of which has to show up as more than a pixel.
const MAX_PLANE_TEXELS: usize = MAX_TEXELS / 4;
/// The share of its width the unbounded board's map leaves around what it shows,
/// which would otherwise sit on the map's border, frame and all.
const PLANE_MAP_MARGIN: i64 = 16;
/// A smaller frame could not be seen. It stays centred on the view.
const MIN_FRAME: f32 = 5.0;
/// Seconds between redraws. Sampling a large board is cheap but not free.
const REFRESH: f32 = 0.2;
/// How opaque a block with any live cell is, so a lone glider stays visible
/// however many cells share its block.
const MIN_LIVE_ALPHA: f32 = 0.5;

#[derive(Component)]
pub(super) struct Minimap;
#[derive(Component)]
pub(super) struct MinimapFrame;

/// Where the map is on screen while it shows, for input routing.
#[derive(Resource, Default)]
pub(super) struct MinimapArea(pub Option<Rect>);

fn handle() -> Handle<Image> {
    const ID: u128 = 0x3c1e_0b9a_5f2d_4e57_9a61_8d0c_47b2_1001;
    Handle::Uuid(Uuid::from_u128(ID), PhantomData)
}

pub(super) fn spawn(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            Minimap,
            ImageNode::new(handle()),
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                right: px(MARGIN),
                bottom: px(MARGIN),
                border: UiRect::all(px(BORDER)),
                overflow: Overflow::clip(),
                ..default()
            },
            ThemedBackground(Token::Field),
            ThemedBorder(Token::Line),
            ThemedImage(Token::Accent),
            controls::Tooltip("A map of the board · click or drag to move the view".into()),
            Hovered::default(),
            accessibility::named_node(accesskit::Role::Image, "Board overview"),
        ))
        .with_child((
            MinimapFrame,
            Node {
                position_type: PositionType::Absolute,
                border: UiRect::all(px(1.5)),
                ..default()
            },
            ThemedBorder(Token::Ink),
            Pickable::IGNORE,
        ));
}

/// The board in world units, centred on the origin.
fn board_rect(cols: usize, rows: usize) -> Rect {
    Rect::from_center_size(Vec2::ZERO, Vec2::new(cols as f32, rows as f32) * CELL_SIZE)
}

/// Whether the canvas shows every cell. A small tolerance keeps rounding at
/// the fitted zoom from flashing the map.
fn shows_whole_board(view: Rect, board: Rect) -> bool {
    const TOLERANCE: Vec2 = Vec2::splat(0.5);
    view.min.cmple(board.min + TOLERANCE).all() && view.max.cmpge(board.max - TOLERANCE).all()
}

/// The map's outer size, keeping the board's shape.
fn map_size(board: Vec2, canvas: Vec2) -> Vec2 {
    let side = MAX_SIDE.min(canvas.min_element() * MAX_SHARE_OF_CANVAS);
    let inner = (board * (side - 2.0 * BORDER) / board.max_element()).max(Vec2::splat(MIN_SIDE));
    inner + 2.0 * BORDER
}

/// Where `world` falls on the map, from 0 at the top left to 1 at the bottom right.
fn to_map(world: Vec2, board: Rect) -> Vec2 {
    Vec2::new(world.x - board.min.x, board.max.y - world.y) / board.size()
}

fn to_world(map: Vec2, board: Rect) -> Vec2 {
    Vec2::new(board.min.x, board.max.y) + Vec2::new(map.x, -map.y) * board.size()
}

/// Alpha for each texel, top row first, from how full its block of cells is.
fn coverage(board: &crate::board::Board, width: usize, height: usize) -> Vec<u8> {
    let (cols, rows) = board.size();
    let mut counts = vec![0; width * height];
    board.overview(width, height, &mut counts);
    let block = (cols * rows) as f32 / (width * height) as f32;
    let alpha = |count: u32| match count {
        0 => 0,
        // Rooted, since live cells are sparse even on a busy board.
        count => {
            let share = (count as f32 / block).min(1.0).sqrt();
            ((MIN_LIVE_ALPHA + (1.0 - MIN_LIVE_ALPHA) * share) * 255.0) as u8
        }
    };
    // Board row 0 is at the bottom.
    counts
        .chunks_exact(width)
        .rev()
        .flatten()
        .map(|&count| alpha(count))
        .collect()
}

/// Inclusive bounds of live cells.
type LiveBounds = ((i64, i64), (i64, i64));

/// Bounds around both the live cells and the view, so the map can show either.
fn plane_extent((min, max): LiveBounds, view: CellRect) -> LiveBounds {
    let (left, bottom) = (min.0.min(view.left), min.1.min(view.bottom));
    let right = max.0.max(view.left + view.width as i64 - 1);
    let top = max.1.max(view.bottom + view.height as i64 - 1);
    let margin = (right - left).max(top - bottom) / PLANE_MAP_MARGIN;
    (
        (left - margin, bottom - margin),
        (right + margin, top + margin),
    )
}

/// Whether some of the bounds lie outside the view.
fn hides_live_cells((min, max): LiveBounds, view: CellRect) -> bool {
    min.0 < view.left
        || min.1 < view.bottom
        || max.0 >= view.left + view.width as i64
        || max.1 >= view.bottom + view.height as i64
}

/// Alpha for each block of the grid, top row first. Blocks here can be any number
/// of chunks across, so they only say whether anything lives there.
fn plane_coverage(plane: &SparseBoard, grid: BlockGrid) -> Vec<u8> {
    let mut alpha = vec![0; grid.width * grid.height];
    plane.for_each_live_block(grid, |col, row| {
        alpha[(grid.height - 1 - row) * grid.width + col] = 255;
    });
    alpha
}

fn image(coverage: &[u8], width: usize, height: usize) -> Image {
    let data = coverage
        .iter()
        .flat_map(|&alpha| [255, 255, 255, alpha])
        .collect();
    let mut image = Image::new(
        Extent3d {
            width: width as u32,
            height: height as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    image
}

/// Everything the map follows, or moves in the case of the camera.
#[derive(SystemParam)]
pub(super) struct MapState<'w, 's> {
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    sim: Res<'w, Simulation>,
    time: Res<'w, Time>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    regions: Res<'w, UiInputRegions>,
    cameras: Query<'w, 's, (&'static mut Transform, &'static Projection), With<MainCamera>>,
}

/// What the map last drew of the unbounded board.
#[derive(Clone, Copy)]
struct PlaneMap {
    live: LiveBounds,
    grid: BlockGrid,
}

/// Carried between frames.
#[derive(Default)]
pub(super) struct MapProgress {
    since_redraw: f32,
    /// The board size last drawn, so a new board redraws at once.
    drawn: Option<(usize, usize)>,
    plane: Option<PlaneMap>,
    dragging: bool,
}

impl MapProgress {
    /// Redraws the map of the unbounded board if it's time to. A drag keeps the
    /// grid it started with, or the map would slide around under the pointer.
    fn redraw_plane(
        &mut self,
        plane: &SparseBoard,
        view: CellRect,
        images: &mut Assets<Image>,
    ) -> Option<PlaneMap> {
        if self.plane.is_some() && self.since_redraw < REFRESH {
            return self.plane;
        }
        self.since_redraw = 0.0;
        let kept = self.plane.filter(|_| self.dragging).map(|map| map.grid);
        self.plane = plane.live_bounds().map(|live| PlaneMap {
            live,
            grid: kept
                .unwrap_or_else(|| BlockGrid::covering(plane_extent(live, view), MAX_PLANE_TEXELS)),
        });
        if let Some(PlaneMap { grid, .. }) = self.plane {
            let alpha = plane_coverage(plane, grid);
            images
                .insert(handle().id(), image(&alpha, grid.width, grid.height))
                .expect("the static minimap handle is valid");
        }
        self.plane
    }
}

pub(super) fn sync(
    mut state: MapState,
    mut images: ResMut<Assets<Image>>,
    mut area: ResMut<MinimapArea>,
    mut maps: Query<&mut Node, (With<Minimap>, Without<MinimapFrame>)>,
    mut frames: Query<&mut Node, (With<MinimapFrame>, Without<Minimap>)>,
    mut progress: Local<MapProgress>,
) {
    let (Ok(window), Ok((mut transform, Projection::Orthographic(ortho)))) =
        (state.windows.single(), state.cameras.single_mut())
    else {
        return;
    };
    let canvas = canvas_rect(window.size());
    let centre = transform.translation.truncate();
    let view = Rect::from_center_size(centre, canvas.size() * ortho.scale);
    let scale = ortho.scale;
    progress.since_redraw += state.time.delta_secs();

    let (board, hidden) = match &state.sim.app.board {
        Universe::Finite(cells) => {
            progress.plane = None;
            let (cols, rows) = cells.size();
            let board = board_rect(cols, rows);
            (board, !shows_whole_board(view, board))
        }
        Universe::Unbounded(plane) => {
            progress.drawn = None;
            let origin = state.sim.plane_origin;
            let seen = view_cells(view, origin);
            let Some(map) = progress.redraw_plane(plane, seen, &mut images) else {
                area.0 = None;
                progress.dragging = false;
                for mut node in &mut maps {
                    set_if_neq!(node.display, Display::None);
                }
                return;
            };
            (
                grid_rect(map.grid, origin),
                hides_live_cells(map.live, seen),
            )
        }
    };

    // A drag outlives the map's reason to show, so a jump cannot strand it.
    let shown = progress.dragging || hidden;
    let size = map_size(board.size(), canvas.size());
    let rect = Rect::from_corners(
        canvas.max - Vec2::splat(MARGIN) - size,
        canvas.max - Vec2::splat(MARGIN),
    );
    area.0 = shown.then_some(rect);
    for mut node in &mut maps {
        set_if_neq!(
            node.display,
            if shown { Display::Flex } else { Display::None }
        );
        set_if_neq!(node.width, px(size.x));
        set_if_neq!(node.height, px(size.y));
    }
    if !shown {
        progress.drawn = None;
        return;
    }

    let inner = rect.inflate(-BORDER);
    let cursor = window.cursor_position();
    if state.mouse.just_pressed(MouseButton::Left) && !state.regions.modal {
        progress.dragging = cursor.is_some_and(|point| inner.contains(point));
    }
    if !state.mouse.pressed(MouseButton::Left) {
        progress.dragging = false;
    }
    if progress.dragging
        && let Some(point) = cursor
    {
        let map = ((point - inner.min) / inner.size()).clamp(Vec2::ZERO, Vec2::ONE);
        let target = to_world(map, board);
        transform.translation.x = target.x;
        transform.translation.y = target.y;
    }

    let centre = transform.translation.truncate();
    let view = Rect::from_center_size(centre, canvas.size() * scale);
    let frame_centre = to_map(centre, board) * inner.size();
    let frame_size = (view.size() / board.size() * inner.size()).max(Vec2::splat(MIN_FRAME));
    for mut node in &mut frames {
        set_if_neq!(node.left, px(frame_centre.x - frame_size.x / 2.0));
        set_if_neq!(node.top, px(frame_centre.y - frame_size.y / 2.0));
        set_if_neq!(node.width, px(frame_size.x));
        set_if_neq!(node.height, px(frame_size.y));
    }

    let Some(cells) = state.sim.app.board.finite() else {
        return;
    };
    let (cols, rows) = cells.size();
    if progress.drawn != Some((cols, rows)) || progress.since_redraw >= REFRESH {
        progress.drawn = Some((cols, rows));
        progress.since_redraw = 0.0;
        let (width, height) = (cols.min(MAX_TEXELS), rows.min(MAX_TEXELS));
        let alpha = coverage(cells, width, height);
        images
            .insert(handle().id(), image(&alpha, width, height))
            .expect("the static minimap handle is valid");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;

    #[test]
    fn the_map_shows_only_while_some_of_the_board_is_out_of_view() {
        let board = board_rect(100, 50);
        let view = |centre: Vec2, size: Vec2| Rect::from_center_size(centre, size);
        assert!(shows_whole_board(view(Vec2::ZERO, board.size()), board));
        assert!(shows_whole_board(
            view(Vec2::ZERO, board.size() * 2.0),
            board
        ));
        assert!(!shows_whole_board(
            view(Vec2::ZERO, board.size() / 2.0),
            board
        ));
        // Zoomed out but panned away also cuts part of the board off.
        let panned = Vec2::new(board.width(), 0.0);
        assert!(!shows_whole_board(view(panned, board.size() * 1.5), board));
    }

    #[test]
    fn world_and_map_positions_convert_both_ways() {
        let board = board_rect(100, 50);
        assert_eq!(
            to_map(Vec2::new(board.min.x, board.max.y), board),
            Vec2::ZERO
        );
        assert_eq!(
            to_map(Vec2::new(board.max.x, board.min.y), board),
            Vec2::ONE
        );
        assert_eq!(to_map(Vec2::ZERO, board), Vec2::splat(0.5));
        let world = Vec2::new(30.0, -12.0) * CELL_SIZE;
        let back = to_world(to_map(world, board), board);
        assert!((back - world).length() < 1e-3);
    }

    #[test]
    fn the_map_keeps_the_board_shape_and_fits_small_canvases() {
        let canvas = Vec2::new(900.0, 600.0);
        assert_eq!(map_size(Vec2::splat(2048.0), canvas), Vec2::splat(MAX_SIDE));
        let wide = map_size(Vec2::new(200.0, 100.0), canvas) - 2.0 * BORDER;
        assert!((wide.x / wide.y - 2.0).abs() < 1e-3);
        let small = map_size(Vec2::splat(2048.0), Vec2::new(590.0, 290.0));
        assert!(small.y <= 290.0 * MAX_SHARE_OF_CANVAS);
        // A sliver of a board is still something to click.
        let sliver = map_size(Vec2::new(2048.0, 8.0), canvas);
        assert!(sliver.y >= MIN_SIDE);
    }

    #[test]
    fn a_lone_cell_shows_however_large_the_board_and_row_zero_is_at_the_bottom() {
        let mut board = Board::new(2048, 2048);
        board.populate(2047, 0);
        let alpha = coverage(&board, MAX_TEXELS, MAX_TEXELS);
        let last = MAX_TEXELS * MAX_TEXELS - 1;
        assert!(alpha[last] >= (MIN_LIVE_ALPHA * 255.0) as u8);
        assert_eq!(alpha.iter().filter(|&&texel| texel > 0).count(), 1);
    }

    #[test]
    fn fuller_blocks_are_more_opaque() {
        let mut board = Board::new(8, 8);
        for (col, row) in [(0, 0), (4, 0), (5, 0), (4, 1), (5, 1)] {
            board.populate(col, row);
        }
        let alpha = coverage(&board, 2, 2);
        // Board row 0 is the map's bottom row: one cell of 16, then four.
        assert_eq!(alpha[0], 0);
        assert!(alpha[2] > 0 && alpha[3] > alpha[2]);
    }

    fn cells_at(left: i64, bottom: i64, width: usize, height: usize) -> CellRect {
        CellRect {
            left,
            bottom,
            width,
            height,
        }
    }

    #[test]
    fn the_plane_map_runs_from_the_live_cells_to_the_view() {
        let view = cells_at(-10, -10, 20, 20);
        // 210 wide, for a margin of 13 on every side.
        assert_eq!(
            plane_extent(((100, -50), (200, 0)), view),
            ((-23, -63), (213, 22))
        );
        assert_eq!(
            plane_extent(((-50, 0), (0, 100)), view),
            ((-56, -16), (15, 106))
        );
    }

    #[test]
    fn the_plane_map_only_matters_with_live_cells_out_of_view() {
        let view = cells_at(0, 0, 10, 10);
        assert!(!hides_live_cells(((0, 0), (9, 9)), view));
        for bounds in [
            ((-1, 0), (9, 9)),
            ((0, -1), (9, 9)),
            ((0, 0), (10, 9)),
            ((0, 0), (9, 10)),
        ] {
            assert!(hides_live_cells(bounds, view), "{bounds:?}");
        }
    }

    #[test]
    fn plane_blocks_are_drawn_top_row_first() {
        let mut plane = SparseBoard::default();
        plane.populate(0, 0);
        plane.populate(1 << 40, 1 << 40);
        let grid = BlockGrid::covering(((0, 0), (1 << 40, 1 << 40)), 4);
        let alpha = plane_coverage(&plane, grid);
        assert_eq!(alpha.iter().filter(|&&texel| texel != 0).count(), 2);
        assert_ne!(alpha[(grid.height - 1) * grid.width], 0, "bottom left");
        assert_ne!(alpha[grid.width - 1], 0, "top right");
    }

    mod system {
        use super::*;
        use bevy::ecs::system::SystemId;

        struct Map {
            world: World,
            sync: SystemId,
            camera: Entity,
            map: Entity,
        }

        impl Map {
            fn new(app: crate::app::App) -> Self {
                let mut world = World::new();
                world.insert_resource(Simulation::new(app));
                world.init_resource::<Assets<Image>>();
                world.init_resource::<Time>();
                world.init_resource::<ButtonInput<MouseButton>>();
                world.init_resource::<UiInputRegions>();
                world.init_resource::<MinimapArea>();
                world.spawn((Window::default(), PrimaryWindow));
                let camera = world.spawn((Camera2d, MainCamera)).id();
                let map = world.spawn((Minimap, Node::default())).id();
                world.spawn((MinimapFrame, Node::default()));
                // Registered rather than run once, to keep its progress between frames.
                let sync = world.register_system(sync);
                Self {
                    world,
                    sync,
                    camera,
                    map,
                }
            }

            fn frame(&mut self) {
                self.world
                    .resource_mut::<Time>()
                    .advance_by(std::time::Duration::from_secs_f32(REFRESH));
                self.world.run_system(self.sync).unwrap();
            }

            fn shown(&self) -> bool {
                self.world.get::<Node>(self.map).unwrap().display != Display::None
                    && self.world.resource::<MinimapArea>().0.is_some()
            }

            fn texture_size(&self) -> UVec2 {
                let images = self.world.resource::<Assets<Image>>();
                images.get(handle().id()).unwrap().size()
            }

            fn sim(&mut self) -> Mut<'_, Simulation> {
                self.world.resource_mut::<Simulation>()
            }

            /// Presses or holds the button at a share of the way across the map.
            fn drag_to(&mut self, share: Vec2) {
                let area = self.world.resource::<MinimapArea>().0.unwrap();
                let point = area.min + area.size() * share;
                let mut windows = self.world.query::<&mut Window>();
                let mut window = windows.single_mut(&mut self.world).unwrap();
                window.set_cursor_position(Some(point));
                self.world
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .press(MouseButton::Left);
                self.frame();
                // Pressed only for the frame it began on.
                self.world
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .clear_just_pressed(MouseButton::Left);
            }

            fn release(&mut self) {
                self.world
                    .resource_mut::<ButtonInput<MouseButton>>()
                    .release(MouseButton::Left);
                self.frame();
            }

            fn camera_x(&self) -> f32 {
                self.world
                    .get::<Transform>(self.camera)
                    .unwrap()
                    .translation
                    .x
            }
        }

        fn unbounded() -> crate::app::App {
            let mut app = crate::app::App::new(16, 16);
            app.set_unbounded().unwrap();
            app
        }

        #[test]
        fn an_empty_plane_has_no_map() {
            let mut map = Map::new(unbounded());
            map.frame();
            assert!(!map.shown());
        }

        #[test]
        fn the_map_comes_and_goes_with_cells_out_of_view() {
            let mut map = Map::new(unbounded());
            map.sim().app.populate_board(1, 1);
            map.frame();
            assert!(!map.shown());

            map.sim().app.populate_board(1 << 20, 0);
            map.frame();
            assert!(map.shown());
            let side = map.texture_size().max_element() as usize;
            assert!(
                (MAX_PLANE_TEXELS / 2..=MAX_PLANE_TEXELS).contains(&side),
                "{side}"
            );

            map.sim().app.depopulate_board(1 << 20, 0);
            map.frame();
            assert!(!map.shown());
        }

        #[test]
        fn a_drag_jumps_the_view_across_a_map_that_holds_still() {
            let mut map = Map::new(unbounded());
            let far = 1i64 << 30;
            map.sim().app.populate_board(far, 0);
            map.frame();
            let start = map.camera_x();
            map.drag_to(Vec2::new(0.5, 0.5));
            let middle = map.camera_x();
            assert!(middle > start + (far as f32) * CELL_SIZE / 4.0, "{middle}");

            // Cells appearing elsewhere mid-drag don't redefine what the map shows.
            map.sim().app.populate_board(-far * 8, 0);
            map.drag_to(Vec2::new(0.5, 0.5));
            assert_eq!(map.camera_x(), middle);

            map.release();
            map.frame();
            assert!(map.shown());
            map.drag_to(Vec2::new(0.5, 0.5));
            assert!(map.camera_x() < middle, "the map takes in the new cells");
        }

        #[test]
        fn returning_to_the_same_board_redraws_its_map_at_once() {
            let mut map = Map::new(crate::app::App::new(16, 16));
            let camera = map.camera;
            let mut projection = map.world.get_mut::<Projection>(camera).unwrap();
            if let Projection::Orthographic(ortho) = &mut *projection {
                ortho.scale = 0.01;
            }
            map.frame();
            assert_eq!(map.texture_size(), UVec2::splat(16));

            map.sim().app.set_unbounded().unwrap();
            map.sim().app.populate_board(1 << 20, 0);
            map.frame();
            assert_ne!(map.texture_size(), UVec2::splat(16));

            // With no time for the usual refresh to come round.
            map.sim()
                .app
                .set_boundary_mode(crate::board::BoundaryMode::Wrap);
            let mut time = map.world.resource_mut::<Time>();
            time.advance_by(std::time::Duration::ZERO);
            map.world.run_system(map.sync).unwrap();
            assert_eq!(map.texture_size(), UVec2::splat(16));
        }

        #[test]
        fn the_map_keeps_following_the_board_it_is_on() {
            let mut map = Map::new(unbounded());
            map.sim().app.populate_board(1 << 20, 0);
            map.frame();
            assert!(map.shown());
            let plane_map = map.texture_size();

            // Zoomed in so that the board needs a map too, of its own size.
            let camera = map.camera;
            let mut projection = map.world.get_mut::<Projection>(camera).unwrap();
            if let Projection::Orthographic(ortho) = &mut *projection {
                ortho.scale = 0.01;
            }
            map.sim()
                .app
                .set_boundary_mode(crate::board::BoundaryMode::Wrap);
            map.frame();
            assert!(map.shown());
            assert_eq!(map.texture_size(), UVec2::splat(16));
            assert_ne!(map.texture_size(), plane_map);
        }
    }
}
