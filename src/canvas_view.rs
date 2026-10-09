//! The board camera occupies the workspace while UI keeps a full-window camera.
use crate::{
    board::Board,
    native_ui_plugin::canvas_rect,
    shared::{
        BoardSettings, CAMERA_ZOOM_MAX, CAMERA_ZOOM_MIN, MainCamera, UiLayout, UpdateSet,
        camera_scale_to_fit_board, cell_to_world,
    },
};
use bevy::{camera::Viewport, prelude::*, window::PrimaryWindow};

#[derive(Resource, Default)]
pub struct CanvasView {
    pub fit_scale: f32,
    board: Option<(usize, usize)>,
}

impl CanvasView {
    /// Centres the board and zooms so all of it fits the canvas.
    pub fn fit_camera(&self, transform: &mut Transform, projection: &mut Projection) {
        transform.translation.x = 0.0;
        transform.translation.y = 0.0;
        if let Projection::Orthographic(ortho) = projection {
            ortho.scale = self.fit_scale;
        }
    }

    /// Fit the occupied rectangle, including a two-cell margin for small patterns.
    /// Empty boards fall back to the whole board.
    pub fn fit_live_cells(
        &self,
        board: &Board,
        canvas_size: Vec2,
        transform: &mut Transform,
        projection: &mut Projection,
    ) {
        let Some((min, max)) = board.live_bounds() else {
            self.fit_camera(transform, projection);
            return;
        };
        let (cols, rows) = board.size();
        let centre = (cell_to_world(min.0, min.1, cols, rows)
            + cell_to_world(max.0, max.1, cols, rows))
            / 2.0;
        transform.translation.x = centre.x;
        transform.translation.y = centre.y;
        if let Projection::Orthographic(ortho) = projection {
            ortho.scale =
                camera_scale_to_fit_board(canvas_size, max.0 - min.0 + 5, max.1 - min.1 + 5);
        }
    }
}

pub struct CanvasViewPlugin;
impl Plugin for CanvasViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CanvasView>().add_systems(
            Update,
            update_viewport
                .before(UpdateSet::Camera)
                .after(crate::native_ui_plugin::NativeLayout),
        );
    }
}

fn physical_viewport(rect: Rect, scale: f32, target: UVec2) -> Viewport {
    let min = (rect.min * scale)
        .round()
        .as_uvec2()
        .min(target - UVec2::ONE);
    let max = (rect.max * scale)
        .round()
        .as_uvec2()
        .min(target)
        .max(min + UVec2::ONE);
    Viewport {
        physical_position: min,
        physical_size: max - min,
        ..default()
    }
}

fn update_viewport(
    windows: Query<&Window, With<PrimaryWindow>>,
    layout: Res<UiLayout>,
    settings: Res<BoardSettings>,
    mut view: ResMut<CanvasView>,
    mut cameras: Query<(&mut Camera, &mut Projection, &mut Transform), With<MainCamera>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let physical = window.physical_size();
    if physical.min_element() == 0 || !settings.is_set() {
        return;
    }
    let Ok((mut camera, mut projection, mut transform)) = cameras.single_mut() else {
        return;
    };
    let rect = canvas_rect(window.size(), &layout);
    let viewport = Some(physical_viewport(rect, window.scale_factor(), physical));
    if camera
        .viewport
        .as_ref()
        .map(|v| (v.physical_position, v.physical_size))
        != viewport
            .as_ref()
            .map(|v| (v.physical_position, v.physical_size))
    {
        camera.viewport = viewport;
    }
    let size = camera.viewport.as_ref().map_or(window.size(), |v| {
        v.physical_size.as_vec2() / window.scale_factor()
    });
    let board = (settings.cols, settings.rows);
    let fit = camera_scale_to_fit_board(size, board.0, board.1);
    if let Projection::Orthographic(ortho) = &mut *projection {
        if view.board != Some(board) {
            transform.translation.x = 0.0;
            transform.translation.y = 0.0;
            ortho.scale = fit;
        } else if view.fit_scale != fit {
            // Keep the user's magnification relative to Fit and the same world
            // position at the canvas centre as the available area changes.
            ortho.scale =
                (ortho.scale * fit / view.fit_scale).clamp(CAMERA_ZOOM_MIN, CAMERA_ZOOM_MAX);
        }
    }
    view.fit_scale = fit;
    view.board = Some(board);
}

/// Bevy converts points outside a viewport too; board interactions must not.
pub fn cursor_world(camera: &Camera, transform: &GlobalTransform, cursor: Vec2) -> Option<Vec2> {
    let rect = camera.logical_viewport_rect()?;
    if cursor.x < rect.min.x
        || cursor.y < rect.min.y
        || cursor.x >= rect.max.x
        || cursor.y >= rect.max.y
    {
        return None;
    }
    camera.viewport_to_world_2d(transform, cursor).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        camera::{CameraProjection, ComputedCameraValues, RenderTargetInfo},
        ecs::system::RunSystemOnce,
    };

    #[test]
    fn live_fit_centres_off_axis_patterns_and_empty_boards_reset_the_view() {
        let mut board = Board::new(100, 80);
        let view = CanvasView {
            fit_scale: 3.0,
            ..default()
        };
        let size = Vec2::new(800.0, 400.0);
        let mut transform = Transform::from_xyz(0.0, 0.0, 10.0);
        let mut projection = Projection::Orthographic(OrthographicProjection::default_2d());
        board.populate(5, 60);
        board.populate(15, 70);
        view.fit_live_cells(&board, size, &mut transform, &mut projection);
        assert_eq!(
            transform.translation,
            cell_to_world(10, 65, 100, 80) + Vec3::Z * 10.0
        );
        let Projection::Orthographic(ortho) = &projection else {
            panic!()
        };
        // Eleven occupied cells plus four margin cells fit in both directions.
        assert!(ortho.scale * size.min_element() >= 15.0 * crate::shared::CELL_SIZE);
        assert!(ortho.scale < view.fit_scale);

        board.clear();
        view.fit_live_cells(&board, size, &mut transform, &mut projection);
        assert_eq!(transform.translation, Vec3::new(0.0, 0.0, 10.0));
        let Projection::Orthographic(ortho) = &projection else {
            panic!()
        };
        assert_eq!(ortho.scale, view.fit_scale);
    }

    #[test]
    fn the_largest_board_fits_at_the_minimum_window_size_with_or_without_the_palette() {
        use crate::shared::{BOARD_SIZE_MAX, CELL_SIZE};
        let size = Vec2::new(680.0, 420.0);
        for control_deck_visible in [true, false] {
            let layout = UiLayout {
                control_deck_visible,
            };
            let rect = canvas_rect(size, &layout);
            let fit = camera_scale_to_fit_board(rect.size(), BOARD_SIZE_MAX, BOARD_SIZE_MAX);
            assert!(rect.min.cmpge(Vec2::ZERO).all());
            assert!(rect.max.cmple(size).all());
            assert!(fit * rect.size().min_element() >= BOARD_SIZE_MAX as f32 * CELL_SIZE);
            for panel in crate::native_ui_plugin::panel_rects(size, &layout) {
                assert!(rect.intersect(panel).is_empty());
            }
        }
    }

    #[test]
    fn viewport_rounding_stays_inside_retina_and_fractional_targets() {
        for scale in [1.0, 1.25, 2.0] {
            let target = UVec2::new(1108, 738);
            let rect = canvas_rect(target.as_vec2() / scale, &UiLayout::default());
            let v = physical_viewport(rect, scale, target);
            assert!(v.physical_size.min_element() > 0);
            assert!((v.physical_position + v.physical_size).cmple(target).all());
            assert!(
                (v.physical_position.as_vec2() / scale - rect.min)
                    .abs()
                    .max_element()
                    <= 0.5 / scale + 0.001
            );
        }
    }

    #[test]
    fn layout_resize_preserves_pan_and_relative_zoom_but_board_resize_fits() {
        let mut world = World::new();
        world.insert_resource(UiLayout::default());
        world.insert_resource(BoardSettings {
            cols: 128,
            rows: 128,
        });
        world.init_resource::<CanvasView>();
        let window = world.spawn((Window::default(), PrimaryWindow)).id();
        let camera = world.spawn((Camera2d, MainCamera)).id();
        world.run_system_once(update_viewport).unwrap();
        let original_fit = world.resource::<CanvasView>().fit_scale;
        if let Projection::Orthographic(o) = &mut *world.get_mut::<Projection>(camera).unwrap() {
            o.scale = original_fit * 0.5;
        }
        world.get_mut::<Transform>(camera).unwrap().translation.x = 123.0;
        world.resource_mut::<UiLayout>().control_deck_visible = false;
        world
            .get_mut::<Window>(window)
            .unwrap()
            .resolution
            .set(900.0, 700.0);
        world.run_system_once(update_viewport).unwrap();
        let fit = world.resource::<CanvasView>().fit_scale;
        let Projection::Orthographic(o) = world.get::<Projection>(camera).unwrap() else {
            panic!()
        };
        assert!((o.scale / fit - 0.5).abs() < 0.0001);
        assert_eq!(world.get::<Transform>(camera).unwrap().translation.x, 123.0);
        world.resource_mut::<BoardSettings>().cols = 256;
        world.run_system_once(update_viewport).unwrap();
        let Projection::Orthographic(o) = world.get::<Projection>(camera).unwrap() else {
            panic!()
        };
        assert_eq!(o.scale, world.resource::<CanvasView>().fit_scale);
        assert_eq!(world.get::<Transform>(camera).unwrap().translation.x, 0.0);
    }

    #[test]
    fn cursor_mapping_uses_offset_viewport_and_excludes_chrome() {
        let rect = Rect::new(332.0, 128.0, 1184.0, 740.0);
        let mut projection = OrthographicProjection::default_2d();
        projection.update(rect.width(), rect.height());
        let camera = Camera {
            viewport: Some(physical_viewport(rect, 2.0, UVec2::new(2400, 1600))),
            computed: ComputedCameraValues {
                target_info: Some(RenderTargetInfo {
                    physical_size: UVec2::new(2400, 1600),
                    scale_factor: 2.0,
                }),
                clip_from_view: projection.get_clip_from_view(),
                ..default()
            },
            ..default()
        };
        let transform = GlobalTransform::from_translation(Vec3::new(30.0, 40.0, 0.0));
        let centre = cursor_world(&camera, &transform, rect.center()).unwrap();
        assert!((centre - Vec2::new(30.0, 40.0)).length() < 0.001);
        assert!(cursor_world(&camera, &transform, Vec2::new(100.0, 300.0)).is_none());
        assert!(cursor_world(&camera, &transform, rect.max).is_none());
        let cell = Vec3::new(66.0, 76.0, 0.0);
        let screen = camera.world_to_viewport(&transform, cell).unwrap();
        assert!(
            (cursor_world(&camera, &transform, screen).unwrap() - cell.truncate()).length() < 0.001
        );
    }
}
