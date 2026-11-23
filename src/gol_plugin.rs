//! Everything the app adds to Bevy's default plugins: its shared resources, the
//! order of its system sets, and its plugins. `main.rs` adds it after
//! `DefaultPlugins`, as does the headless schedule test in `tests/`.
use crate::canvas_view::CanvasViewPlugin;
use crate::frame_pacing::FramePacingPlugin;
use crate::input_plugin::InputPlugin;
use crate::native_ui_plugin::NativeUiPlugin;
use crate::plane_view::PlaneViewPlugin;
use crate::render_plugin::RenderPlugin;
use crate::settings_plugin::SettingsPlugin;
use crate::shared::{
    BG_COLOR, BoardSettings, DiagnosticsOverlay, GridOverlay, RedrawRequested, StartupSet,
    UpdateSet,
};
use crate::simulation_plugin::SimulationPlugin;
use crate::ui_input::UiInputPlugin;
use bevy::prelude::*;

pub struct GolPlugin;

impl Plugin for GolPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(BG_COLOR))
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
                PlaneViewPlugin,
                CanvasViewPlugin,
                SettingsPlugin,
                UiInputPlugin,
                NativeUiPlugin,
                InputPlugin,
                SimulationPlugin,
                FramePacingPlugin,
            ));
    }
}
