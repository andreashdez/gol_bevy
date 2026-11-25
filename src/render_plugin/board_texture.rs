//! The tiled board texture: its tiles and sprites, cell ages and the ageing masks, the cell palette, and painting changed cells
use super::blend_color;
use crate::board::{DirtyCells, RowCells};
use crate::coords::CELL_SIZE;
use crate::shared::{BoardColors, CellAge, RedrawRequested, Simulation, Theme};
use bevy::{
    asset::RenderAssetUsages,
    ecs::system::SystemParam,
    image::ImageSampler,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};

/// The board is drawn as textures with a single texel per cell, so rendering
/// cost tracks the cells that changed instead of one sprite entity per cell.
/// The texture is cut into tiles, each its own image, so that painting a few
/// cells sends the GPU only the tiles they are on rather than the whole board.
#[derive(Resource, Default)]
pub(super) struct BoardTexture {
    pub(super) layout: TileLayout,
    /// One image per tile, in the order of [`TileLayout::rect`].
    pub(super) tiles: Vec<Handle<Image>>,
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
pub(super) struct TileLayout {
    pub(super) cols: usize,
    pub(super) rows: usize,
}

impl TileLayout {
    fn across(self) -> usize {
        self.cols.div_ceil(TILE_CELLS)
    }

    fn count(self) -> usize {
        self.across() * self.rows.div_ceil(TILE_CELLS)
    }

    /// The first column and row of a tile, and its width and height in cells.
    pub(super) fn rect(self, tile: usize) -> (usize, usize, usize, usize) {
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
    pub(super) fn new(images: &mut Assets<Image>, cols: usize, rows: usize, dead: Color) -> Self {
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
    pub(super) fn spawn_sprites(&self, commands: &mut Commands, board: Entity) {
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
pub(super) struct BoardTile;

/// Generations since each cell last changed state. The renderer keeps these so
/// the engine's step loop pays nothing for them. A cell stops ageing once its
/// colour has settled.
#[derive(Resource, Default)]
pub(crate) struct CellAges {
    pub(super) ages: Vec<u8>,
    /// The generation counter at the last sync, to tell how many generations ran since.
    epoch: usize,
    /// The first cell of each run of the board's rows with a cell still ageing,
    /// so a generation visits those cells and not the whole board. Most of a
    /// board that has run for a while is settled.
    pub(super) ageing: Vec<u32>,
    /// The cells of each run that may still be ageing, by the run's place among
    /// the board's runs. A run is in `ageing` while it has any.
    pub(super) ageing_cells: Vec<u64>,
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
pub(super) const MATURE_AGE: u8 = 6;
/// Generations after which a cell has the aged colour.
const OLD_AGE: u8 = 60;
/// Generations a trail takes to fade.
pub(super) const TRAIL_LENGTH: u8 = 6;
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

/// The colours a GIF draws cells in, as the board shows them: by age with cell
/// age on, through to the ages their colours settle at, or plain.
pub(crate) fn gif_colors(colors: &BoardColors, by_age: bool) -> crate::cell_gif::GifColors {
    let rgb = |color| {
        let [red, green, blue, _] = color_to_texel(color);
        [red, green, blue]
    };
    if !by_age {
        return crate::cell_gif::GifColors::plain(rgb(colors.dead), rgb(colors.live));
    }
    crate::cell_gif::GifColors::by_age(
        (0..=TRAIL_LENGTH)
            .map(|age| rgb(dead_shade(colors, age)))
            .collect(),
        (0..=SETTLED_AGE)
            .map(|age| rgb(live_shade(colors, age)))
            .collect(),
        settled_age(true),
    )
}

/// Board row 0 sits at the bottom in world space while texel row 0 sits at the
/// top, so the vertical axis is flipped when addressing the texture.
pub(crate) fn texel_offset(idx: usize, cols: usize, rows: usize) -> usize {
    let col = idx % cols;
    let row = idx / cols;
    ((rows - 1 - row) * cols + col) * 4
}

pub(super) fn board_sprite_size(cols: usize, rows: usize) -> Vec2 {
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
pub(super) struct BoardCanvas<'w, 's> {
    pub(super) commands: Commands<'w, 's>,
    images: ResMut<'w, Assets<Image>>,
    pub(super) texture: ResMut<'w, BoardTexture>,
    board_q: Query<'w, 's, Entity, With<BoardSprite>>,
    tile_q: Query<'w, 's, Entity, With<BoardTile>>,
    pub(super) theme: Res<'w, Theme>,
}

impl BoardCanvas<'_, '_> {
    /// Draw the board with freshly sized tiles. The previous images are dropped
    /// once their sprites go.
    pub(super) fn rebuild(&mut self, cols: usize, rows: usize) {
        for tile in &self.tile_q {
            self.commands.entity(tile).despawn();
        }
        *self.texture = BoardTexture::new(&mut self.images, cols, rows, self.theme.board().dead);
        if let Ok(board) = self.board_q.single() {
            self.texture.spawn_sprites(&mut self.commands, board);
        }
    }
}

/// What decides a cell's colour besides its state.
#[derive(SystemParam)]
pub(crate) struct CellStyle<'w> {
    pub(crate) theme: Res<'w, Theme>,
    pub(crate) cell_age: Res<'w, CellAge>,
    ages: ResMut<'w, CellAges>,
}

pub(super) fn sync_cells(
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
