//! The stamp preview under the pointer, the hovered cell, and the selection overlay
use super::{HoveredCell, board_texture::texel_offset};
use crate::app::{App as GolApp, EditTool};
use crate::canvas_view::cursor_world;
use crate::coords::CELL_SIZE;
use crate::shared::{MainCamera, Simulation, Theme};
use crate::ui_input::UiInputCapture;
use bevy::{
    asset::RenderAssetUsages,
    ecs::system::SystemParam,
    image::ImageSampler,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    window::PrimaryWindow,
};

/// The single sprite that shows the stamp under the pointer.
#[derive(Component)]
pub(super) struct StampGhost;

/// Which stamp the ghost texture was drawn from. The stamp is a paste or a
/// library pattern, turned and flipped; any of those changing means new cells.
/// The library revision is in it because a restored workspace starts the paste
/// count again and can put another pattern at the same index.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct StampKey {
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
pub(super) struct StampGhostImage {
    pub(super) key: Option<StampKey>,
    pub(super) handle: Handle<Image>,
    pub(super) width: usize,
    pub(super) height: usize,
}

/// The stamp ghost's texture and the assets it lives in, which change together.
#[derive(SystemParam)]
pub(super) struct GhostTexture<'w> {
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
pub(super) fn stamp_ghost_image(cells: &[(usize, usize)], width: usize, height: usize) -> Image {
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
pub(super) fn sync_pattern_preview(
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

pub(super) fn update_hovered_cell(
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
pub(super) fn render_selection(sim: Res<Simulation>, theme: Res<Theme>, mut gizmos: Gizmos) {
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

pub(super) fn render_hover_cell(
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
