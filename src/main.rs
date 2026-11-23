// Release builds on Windows open no console window next to the game window.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use bevy::{prelude::*, window::WindowResolution};
use gol_bevy::gol_plugin::GolPlugin;
use gol_bevy::shared::WEB_BUILD;

fn main() {
    bevy::prelude::App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Conway's Game of Life".into(),
                resolution: WindowResolution::new(1200, 800),
                resizable: true,
                resize_constraints: bevy::window::WindowResizeConstraints {
                    min_width: 800.0,
                    min_height: 420.0,
                    ..default()
                },
                // The web build draws into the page's canvas and follows the
                // size of the element around it.
                canvas: WEB_BUILD.then(|| "#gol-bevy".into()),
                fit_canvas_to_parent: WEB_BUILD,
                ..default()
            }),
            ..default()
        }))
        .add_plugins(GolPlugin)
        .run();
}
