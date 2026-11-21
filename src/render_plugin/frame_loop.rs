//! A headless copy of the app's frame, for `benches/frame_loop.rs`: the
//! simulation plugin steps the board as the app does, then the board's tiles
//! are painted, with no window, UI, or GPU. Not used by the app.
use super::{BoardTexture, CellAges, sync_cells};
use crate::{
    app::App as GolApp,
    shared::{
        CellAge, DiagnosticsOverlay, GridOverlay, RedrawRequested, Simulation, Theme, UpdateSet,
    },
    simulation_plugin::SimulationPlugin,
};
use bevy::{
    app::TaskPoolPlugin,
    asset::AssetPlugin,
    prelude::*,
    time::TimePlugin,
    window::{Monitor, OnMonitor, PrimaryMonitor, PrimaryWindow},
};

/// An app whose `update` runs one frame of the simulation and board painting
/// for `board`, a finite board, with cell age shown or not. A primary window
/// sits on a primary monitor refreshing at `refresh_millihertz`, for anything
/// in the frame that reads the display.
pub fn frame_app(board: GolApp, cell_age: bool, refresh_millihertz: u32) -> App {
    let (cols, rows) = board
        .board
        .size()
        .expect("the frame loop draws a finite board");
    let mut app = App::new();
    app.add_plugins((
        TaskPoolPlugin::default(),
        TimePlugin,
        AssetPlugin {
            watch_for_changes_override: Some(false),
            use_asset_processor_override: Some(false),
            ..default()
        },
        SimulationPlugin,
    ))
    .init_asset::<Image>()
    // The same order as the app's frame, which only these sets take part in.
    .configure_sets(
        Update,
        (
            UpdateSet::Simulation,
            UpdateSet::Redraw,
            UpdateSet::Render,
            UpdateSet::Quit,
        )
            .chain(),
    )
    .add_systems(Update, sync_cells.in_set(UpdateSet::Render))
    .insert_resource(Simulation::new(board))
    .insert_resource(RedrawRequested(true))
    .insert_resource(DiagnosticsOverlay { enabled: false })
    .insert_resource(GridOverlay { enabled: false })
    .insert_resource(Theme::Dark)
    .insert_resource(CellAge { enabled: cell_age })
    .init_resource::<CellAges>();

    let world = app.world_mut();
    let texture = BoardTexture::new(
        &mut world.resource_mut::<Assets<Image>>(),
        cols,
        rows,
        Theme::Dark.board().dead,
    );
    world.insert_resource(texture);
    let monitor = world
        .spawn((
            Monitor {
                name: None,
                physical_height: 1080,
                physical_width: 1920,
                physical_position: IVec2::ZERO,
                refresh_rate_millihertz: Some(refresh_millihertz),
                scale_factor: 1.0,
                video_modes: Vec::new(),
            },
            PrimaryMonitor,
        ))
        .id();
    world.spawn((Window::default(), PrimaryWindow, OnMonitor(monitor)));
    app
}
