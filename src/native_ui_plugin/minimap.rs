//! A map of the whole board that appears while part of it is out of view, with
//! a frame around what the canvas shows. Clicking or dragging moves the view.
//! Like the icons, the map is a white mask tinted by [`ThemedImage`].
use super::*;
use crate::shared::{CELL_SIZE, MainCamera};
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
            controls::Tooltip("The whole board · click or drag to move the view".into()),
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

/// Carried between frames.
#[derive(Default)]
pub(super) struct MapProgress {
    since_redraw: f32,
    /// The board size last drawn, so a new board redraws at once.
    drawn: Option<(usize, usize)>,
    dragging: bool,
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
    let (cols, rows) = state.sim.app.board.size();
    let canvas = canvas_rect(window.size());
    let board = board_rect(cols, rows);
    let centre = transform.translation.truncate();
    let view = Rect::from_center_size(centre, canvas.size() * ortho.scale);
    let scale = ortho.scale;

    // A drag outlives the map's reason to show, so a jump cannot strand it.
    let shown = progress.dragging || !shows_whole_board(view, board);
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

    progress.since_redraw += state.time.delta_secs();
    if progress.drawn != Some((cols, rows)) || progress.since_redraw >= REFRESH {
        progress.drawn = Some((cols, rows));
        progress.since_redraw = 0.0;
        let (width, height) = (cols.min(MAX_TEXELS), rows.min(MAX_TEXELS));
        let alpha = coverage(&state.sim.app.board, width, height);
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
}
