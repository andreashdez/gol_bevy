use crate::app::App as GolApp;
use crate::coords::board_size_for_window;
use crate::frame_pacing::StayAwake;
use crate::shared::{BoardSettings, CellAge, MainCamera, Simulation, StartupSet, Theme, UpdateSet};
#[cfg(test)]
use crate::{app::EditTool, coords::CELL_SIZE, shared::RedrawRequested};
#[cfg(test)]
use bevy::image::ImageSampler;
use bevy::{camera::visibility::RenderLayers, prelude::*, window::PrimaryWindow};

mod board_texture;
mod finite_view;
mod preview;
#[cfg(test)]
mod tests;

pub(crate) use board_texture::{
    BoardSprite, CellAges, CellPalette, CellStyle, SETTLED_AGE, color_to_texel, gif_colors,
    settled_age, texel_offset,
};
use board_texture::{BoardTexture, sync_cells};
#[cfg(test)]
use board_texture::{BoardTile, MATURE_AGE, TRAIL_LENGTH, TileLayout, board_sprite_size};
#[cfg(test)]
pub(crate) use board_texture::{dead_shade, live_shade};
use finite_view::{
    BoundaryWallPulse, apply_board_settings, keep_awake_while_pulsing, render_grid,
    spawn_boundary_walls, sync_boundary_wall_visibility,
};
#[cfg(test)]
use preview::{StampGhost, stamp_ghost_image};
use preview::{
    StampGhostImage, render_hover_cell, render_selection, sync_pattern_preview, update_hovered_cell,
};

/// The app's frame without a window, for the frame-loop benchmark.
#[doc(hidden)]
pub mod frame_loop;

pub struct RenderPlugin;

/// The board cell under the pointer, or the keyboard's cell cursor while it
/// shows, as `(col, row)` with row 0 at the bottom.
#[derive(Resource, Default)]
pub struct HoveredCell(pub Option<(i64, i64)>);

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
