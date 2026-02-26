pub mod app;
pub mod board;
pub mod input_plugin;
pub mod render_plugin;
pub mod shared;
pub mod simulation_plugin;
pub mod ui_plugin;

use bevy::{prelude::*, window::WindowResolution};
use input_plugin::InputPlugin;
use render_plugin::RenderPlugin;
use shared::{BG_COLOR, DiagnosticsOverlay, RedrawRequested, UpdateSet};
use simulation_plugin::SimulationPlugin;
use ui_plugin::UiPlugin;

fn main() {
    bevy::prelude::App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Conway's Game of Life (Bevy)".into(),
                resolution: WindowResolution::new(1200, 800),
                resizable: true,
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(BG_COLOR))
        .insert_resource(RedrawRequested::default())
        .insert_resource(DiagnosticsOverlay { enabled: true })
        .configure_sets(
            Update,
            (
                UpdateSet::Input,
                UpdateSet::Simulation,
                UpdateSet::Resize,
                UpdateSet::Redraw,
                UpdateSet::Render,
                UpdateSet::Ui,
                UpdateSet::Quit,
            )
                .chain(),
        )
        .add_plugins((RenderPlugin, UiPlugin, InputPlugin, SimulationPlugin))
        .run();
}
