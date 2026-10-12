// Release builds on Windows open no console window next to the game window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bevy::{prelude::*, window::WindowResolution};
use gol_bevy::canvas_view::CanvasViewPlugin;
use gol_bevy::input_plugin::InputPlugin;
use gol_bevy::native_ui_plugin::NativeUiPlugin;
use gol_bevy::render_plugin::RenderPlugin;
use gol_bevy::settings_plugin::SettingsPlugin;
use gol_bevy::shared::{
    BG_COLOR, BoardSettings, DiagnosticsOverlay, GridOverlay, RedrawRequested, StartupSet,
    UpdateSet,
};
use gol_bevy::simulation_plugin::SimulationPlugin;
use gol_bevy::ui_input::UiInputPlugin;

fn main() {
    bevy::prelude::App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Conway's Game of Life".into(),
                resolution: WindowResolution::new(1200, 800),
                resizable: true,
                resize_constraints: bevy::window::WindowResizeConstraints {
                    min_width: 680.0,
                    min_height: 420.0,
                    ..default()
                },
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(BG_COLOR))
        .insert_resource(RedrawRequested::default())
        .insert_resource(DiagnosticsOverlay { enabled: false })
        .insert_resource(GridOverlay { enabled: false })
        .init_resource::<BoardSettings>()
        .configure_sets(
            Startup,
            (StartupSet::LoadSettings, StartupSet::BuildWorld).chain(),
        )
        .configure_sets(
            Update,
            (
                UpdateSet::Input,
                UpdateSet::Simulation,
                UpdateSet::Camera,
                UpdateSet::Resize,
                UpdateSet::Redraw,
                UpdateSet::Render,
                UpdateSet::Quit,
            )
                .chain(),
        )
        .add_plugins((
            RenderPlugin,
            CanvasViewPlugin,
            SettingsPlugin,
            UiInputPlugin,
            NativeUiPlugin,
            InputPlugin,
            SimulationPlugin,
        ))
        .run();
}
