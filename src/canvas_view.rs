//! The board camera occupies the workspace while UI keeps a full-window camera.
use crate::{
    native_ui_plugin::canvas_rect,
    shared::{
        BoardSettings, CAMERA_ZOOM_MAX, CAMERA_ZOOM_MIN, CELL_SIZE, MainCamera, Simulation,
        UpdateSet, board_size_for_canvas, camera_scale_to_fit_board, camera_scale_to_fit_pattern,
    },
    universe::Universe,
};
use bevy::{camera::Viewport, prelude::*, window::PrimaryWindow};

/// How far the camera may drift from the world origin before the plane's origin
/// moves to meet it. World positions are good to a hundredth of a cell out here.
const PLANE_DRIFT_MAX: f32 = (1 << 16) as f32;

#[derive(Resource, Default)]
pub struct CanvasView {
    pub fit_scale: f32,
    board: Option<(usize, usize)>,
    /// The canvas size in logical points.
    canvas: Vec2,
    /// Whether the view is on the plane, to notice leaving it.
    on_plane: bool,
}

impl CanvasView {
    /// Whether the plane's live cells reach into view, or there are none to miss.
    fn sees_live_cells(&self, sim: &Simulation, transform: &Transform, scale: f32) -> bool {
        let Universe::Unbounded(plane) = &sim.app.board else {
            return true;
        };
        let Some((min, max)) = plane.live_bounds() else {
            return true;
        };
        let map = sim.cell_map();
        let seen = Rect::from_center_size(transform.translation.truncate(), self.canvas * scale);
        map.cell_to_world(min.0, min.1)
            .truncate()
            .cmple(seen.max)
            .all()
            && map
                .cell_to_world(max.0, max.1)
                .truncate()
                .cmpge(seen.min)
                .all()
    }

    /// Centres the board and zooms so all of it fits the canvas. On the plane
    /// that means the live cells, or the origin's neighbourhood if there are none.
    pub fn fit_camera(
        &self,
        sim: &mut Simulation,
        transform: &mut Transform,
        projection: &mut Projection,
    ) {
        let bounds = match &sim.app.board {
            Universe::Finite(_) => None,
            Universe::Unbounded(plane) => plane.live_bounds(),
        };
        let mut scale = self.fit_scale;
        let mut centre = Vec2::ZERO;
        if let Some((min, max)) = bounds {
            let (width, height) = (max.0 - min.0 + 1, max.1 - min.1 + 1);
            sim.plane_origin = (min.0 + width / 2, min.1 + height / 2);
            // An odd width has its middle halfway across the origin's cell.
            centre = Vec2::new((width % 2) as f32, (height % 2) as f32) * CELL_SIZE / 2.0;
            scale = camera_scale_to_fit_pattern(self.canvas, width as f32, height as f32);
        }
        transform.translation.x = centre.x;
        transform.translation.y = centre.y;
        if let Projection::Orthographic(ortho) = projection {
            ortho.scale = scale;
        }
    }
}

pub struct CanvasViewPlugin;
impl Plugin for CanvasViewPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CanvasView>().add_systems(
            Update,
            (
                update_viewport
                    .before(UpdateSet::Camera)
                    .after(crate::native_ui_plugin::NativeLayout),
                follow_plane.in_set(UpdateSet::Resize),
            ),
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
    mut settings: ResMut<BoardSettings>,
    mut view: ResMut<CanvasView>,
    mut cameras: Query<(&mut Camera, &mut Projection, &mut Transform), With<MainCamera>>,
    sim: Option<Res<Simulation>>,
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
    let rect = canvas_rect(window.size());
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
    if settings.fill_canvas {
        let (cols, rows) = board_size_for_canvas(size);
        if (settings.cols, settings.rows) != (cols, rows) {
            // Resized here, before the camera fits, so both change in one frame.
            settings.set(cols, rows);
        }
    }
    view.canvas = size;
    let board = (settings.cols, settings.rows);
    let fit = camera_scale_to_fit_board(size, board.0, board.1);
    // The plane's view owes nothing to the size of the board it might return to.
    let unbounded = sim.is_some_and(|sim| sim.app.board.finite().is_none());
    if let Projection::Orthographic(ortho) = &mut *projection
        && !unbounded
    {
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

/// Keeps the view steady across the plane: the origin moves to meet a camera that
/// has drifted, and a finite board gets back a view of itself.
fn follow_plane(
    mut sim: ResMut<Simulation>,
    mut view: ResMut<CanvasView>,
    mut cameras: Query<(&mut Transform, &mut GlobalTransform, &mut Projection), With<MainCamera>>,
) {
    let Ok((mut transform, mut global, mut projection)) = cameras.single_mut() else {
        return;
    };
    let unbounded = sim.app.board.finite().is_none();
    if view.on_plane && !unbounded {
        view.fit_camera(&mut sim, &mut transform, &mut projection);
        sim.plane_origin = (0, 0);
    }
    // Undo can return to a plane whose cells are nowhere near this view.
    if !view.on_plane
        && unbounded
        && let Projection::Orthographic(ortho) = &*projection
        && !view.sees_live_cells(&sim, &transform, ortho.scale)
    {
        view.fit_camera(&mut sim, &mut transform, &mut projection);
    }
    view.on_plane = unbounded;

    let drift = transform.translation.truncate();
    if !unbounded || drift.abs().max_element() < PLANE_DRIFT_MAX {
        return;
    }
    let cells = (drift / CELL_SIZE).round();
    sim.plane_origin.0 += cells.x as i64;
    sim.plane_origin.1 += cells.y as i64;
    transform.translation -= (cells * CELL_SIZE).extend(0.0);
    // Pointer mapping this frame uses this rather than `transform`, and has to
    // agree with the origin that just moved.
    *global = GlobalTransform::from(*transform);
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

    fn plane_world(app: crate::app::App) -> (World, Entity) {
        let mut world = World::new();
        world.insert_resource(Simulation::new(app));
        world.insert_resource(CanvasView {
            fit_scale: 3.0,
            board: Some((16, 16)),
            canvas: Vec2::new(1000.0, 500.0),
            on_plane: false,
        });
        let camera = world.spawn((Camera2d, MainCamera)).id();
        (world, camera)
    }

    fn unbounded_app() -> crate::app::App {
        let mut app = crate::app::App::new(16, 16);
        app.set_unbounded().unwrap();
        app
    }

    fn fit(world: &mut World) {
        world
            .run_system_once(
                |mut sim: ResMut<Simulation>,
                 view: Res<CanvasView>,
                 mut cameras: Query<(&mut Transform, &mut Projection)>| {
                    let (mut transform, mut projection) = cameras.single_mut().unwrap();
                    view.fit_camera(&mut sim, &mut transform, &mut projection);
                },
            )
            .unwrap();
    }

    fn scale_of(world: &World, camera: Entity) -> f32 {
        match world.get::<Projection>(camera).unwrap() {
            Projection::Orthographic(ortho) => ortho.scale,
            _ => panic!("the camera is orthographic"),
        }
    }

    #[test]
    fn a_camera_far_from_the_world_origin_brings_the_plane_origin_with_it() {
        let (mut world, camera) = plane_world(unbounded_app());
        world.resource_mut::<Simulation>().plane_origin = (5, -5);
        let start = Vec3::new(120_000.0, -240_006.0, 0.0);
        world.get_mut::<Transform>(camera).unwrap().translation = start;
        let before = world
            .resource::<Simulation>()
            .cell_map()
            .world_to_cell(start.truncate());
        world.run_system_once(follow_plane).unwrap();

        let moved = world.get::<Transform>(camera).unwrap().translation;
        assert!(moved.abs().max_element() < CELL_SIZE, "{moved}");
        assert_eq!(
            world.get::<GlobalTransform>(camera).unwrap().translation(),
            moved
        );
        let sim = world.resource::<Simulation>();
        assert_eq!(sim.plane_origin, (10_005, -20_006));
        // The cell at the middle of the view is the same one as before.
        assert_eq!(sim.cell_map().world_to_cell(moved.truncate()), before);
    }

    #[test]
    fn a_camera_near_the_world_origin_leaves_the_plane_origin_alone() {
        let (mut world, camera) = plane_world(unbounded_app());
        let start = Vec3::new(PLANE_DRIFT_MAX - 1.0, 0.0, 0.0);
        world.get_mut::<Transform>(camera).unwrap().translation = start;
        world.run_system_once(follow_plane).unwrap();
        assert_eq!(world.get::<Transform>(camera).unwrap().translation, start);
        assert_eq!(world.resource::<Simulation>().plane_origin, (0, 0));
    }

    #[test]
    fn a_camera_stays_put_on_a_finite_board() {
        let (mut world, camera) = plane_world(crate::app::App::new(16, 16));
        let start = Vec3::new(2.0 * PLANE_DRIFT_MAX, 0.0, 0.0);
        world.get_mut::<Transform>(camera).unwrap().translation = start;
        world.run_system_once(follow_plane).unwrap();
        assert_eq!(world.get::<Transform>(camera).unwrap().translation, start);
    }

    #[test]
    fn leaving_the_plane_shows_the_board_from_the_world_origin_again() {
        let (mut world, camera) = plane_world(unbounded_app());
        world.run_system_once(follow_plane).unwrap();
        world.resource_mut::<Simulation>().plane_origin = (900, 900);
        world.get_mut::<Transform>(camera).unwrap().translation = Vec3::splat(50.0).with_z(0.0);

        let mut sim = world.resource_mut::<Simulation>();
        sim.app.set_boundary_mode(crate::board::BoundaryMode::Wrap);
        world.run_system_once(follow_plane).unwrap();

        assert_eq!(world.resource::<Simulation>().plane_origin, (0, 0));
        let at = world.get::<Transform>(camera).unwrap().translation;
        assert_eq!(at.truncate(), Vec2::ZERO);
        assert_eq!(scale_of(&world, camera), 3.0);
    }

    #[test]
    fn arriving_on_a_plane_with_nothing_in_view_finds_the_cells() {
        let mut app = unbounded_app();
        app.populate_board(1 << 40, 1 << 40);
        app.set_boundary_mode(crate::board::BoundaryMode::Wrap);
        assert!(app.undo());
        let (mut world, _) = plane_world(app);
        world.run_system_once(follow_plane).unwrap();

        let sim = world.resource::<Simulation>();
        let cell = (1 << 40, 1 << 40);
        assert_eq!(sim.plane_origin, cell);
    }

    #[test]
    fn arriving_on_a_plane_with_cells_in_view_leaves_the_view_alone() {
        let mut app = unbounded_app();
        app.populate_board(3, 3);
        let (mut world, camera) = plane_world(app);
        world.get_mut::<Transform>(camera).unwrap().translation.x = 10.0;
        world.run_system_once(follow_plane).unwrap();

        assert_eq!(world.resource::<Simulation>().plane_origin, (0, 0));
        assert_eq!(world.get::<Transform>(camera).unwrap().translation.x, 10.0);
    }

    #[test]
    fn fitting_the_plane_frames_the_live_cells_however_far_away_they_are() {
        let mut app = unbounded_app();
        let far = 1 << 40;
        app.populate_board(far, far);
        app.populate_board(far + 4999, far + 99);
        let (mut world, camera) = plane_world(app);
        fit(&mut world);

        let sim = world.resource::<Simulation>();
        assert_eq!(sim.plane_origin, (far + 2500, far + 50));
        let map = sim.cell_map();
        let left = map.cell_to_world(far, far).x - CELL_SIZE / 2.0;
        let right = map.cell_to_world(far + 4999, far).x + CELL_SIZE / 2.0;
        let x = world.get::<Transform>(camera).unwrap().translation.x;
        assert!(
            ((left + right) / 2.0 - x).abs() < 0.01,
            "{left} {right} {x}"
        );
        // Wider than tall, so the width sets the scale.
        let scale = scale_of(&world, camera);
        assert!(scale * 1000.0 >= right - left, "{scale}");
        assert!(scale * 1000.0 < (right - left) * 1.1, "{scale}");
    }

    #[test]
    fn fitting_a_small_pattern_stops_at_full_size() {
        let mut app = unbounded_app();
        app.populate_board(-3, 2);
        let (mut world, camera) = plane_world(app);
        fit(&mut world);
        assert_eq!(scale_of(&world, camera), 1.0);
    }

    #[test]
    fn fitting_an_empty_plane_returns_to_the_origin() {
        let (mut world, camera) = plane_world(unbounded_app());
        world.get_mut::<Transform>(camera).unwrap().translation.x = 900.0;
        fit(&mut world);
        assert_eq!(world.get::<Transform>(camera).unwrap().translation.x, 0.0);
        assert_eq!(scale_of(&world, camera), 3.0);
    }

    #[test]
    fn the_largest_board_fits_at_the_minimum_window_size() {
        use crate::shared::{BOARD_SIZE_MAX, CELL_SIZE};
        let size = Vec2::new(800.0, 420.0);
        let rect = canvas_rect(size);
        let fit = camera_scale_to_fit_board(rect.size(), BOARD_SIZE_MAX, BOARD_SIZE_MAX);
        assert!(rect.min.cmpge(Vec2::ZERO).all());
        assert!(rect.max.cmple(size).all());
        assert!(fit * rect.size().min_element() >= BOARD_SIZE_MAX as f32 * CELL_SIZE);
    }

    #[test]
    fn viewport_rounding_stays_inside_retina_and_fractional_targets() {
        for scale in [1.0, 1.25, 2.0] {
            let target = UVec2::new(1108, 738);
            let rect = canvas_rect(target.as_vec2() / scale);
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
        world.insert_resource(BoardSettings {
            cols: 128,
            rows: 128,
            fill_canvas: false,
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
    fn fill_canvas_sizes_the_board_to_the_canvas_at_100_percent_and_follows_it() {
        use crate::shared::{CELL_SIZE, FILL_CELL_POINTS};
        let mut world = World::new();
        world.insert_resource(BoardSettings {
            cols: 64,
            rows: 64,
            fill_canvas: true,
        });
        world.init_resource::<CanvasView>();
        let window = world.spawn((Window::default(), PrimaryWindow)).id();
        world.spawn((Camera2d, MainCamera));
        let check = |world: &mut World| {
            world.run_system_once(update_viewport).unwrap();
            let size = canvas_rect(world.get::<Window>(window).unwrap().size()).size();
            let settings = *world.resource::<BoardSettings>();
            assert_eq!((settings.cols, settings.rows), board_size_for_canvas(size));
            // At 100%, the fit, cells are `FILL_CELL_POINTS` across, leaving less
            // than a cell of gutter on the snug axis.
            let fit = world.resource::<CanvasView>().fit_scale;
            let target = CELL_SIZE / FILL_CELL_POINTS;
            assert!(
                fit <= target * 1.0001
                    && fit > target * (1.0 - 1.1 * FILL_CELL_POINTS / size.min_element()),
                "{fit}"
            );
        };
        check(&mut world);
        world
            .get_mut::<Window>(window)
            .unwrap()
            .resolution
            .set(1000.0, 650.0);
        check(&mut world);

        // Switched off, the board keeps its size whatever the window does.
        world.resource_mut::<BoardSettings>().fill_canvas = false;
        let before = *world.resource::<BoardSettings>();
        world
            .get_mut::<Window>(window)
            .unwrap()
            .resolution
            .set(1400.0, 900.0);
        world.run_system_once(update_viewport).unwrap();
        assert!(*world.resource::<BoardSettings>() == before);
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
