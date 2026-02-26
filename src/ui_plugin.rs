use crate::shared::{
    CONTROLS_TEXT, DiagnosticsOverlay, GridOverlay, HELP_FONT_SIZE, HELP_TEXT_COLOR, HUD_FONT_SIZE,
    HUD_TEXT_COLOR, HelpText, Hud, RedrawRequested, Simulation, UI_MARGIN, UpdateSet,
};
use bevy::prelude::*;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup_ui)
            .add_systems(Update, sync_hud.in_set(UpdateSet::Ui));
    }
}

fn setup_ui(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(UI_MARGIN),
            left: Val::Px(UI_MARGIN),
            ..default()
        },
        TextFont {
            font_size: HUD_FONT_SIZE,
            ..default()
        },
        TextColor(HUD_TEXT_COLOR),
        Hud,
    ));

    commands.spawn((
        Text::new(CONTROLS_TEXT),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(UI_MARGIN),
            top: Val::Px(UI_MARGIN),
            ..default()
        },
        TextFont {
            font_size: HELP_FONT_SIZE,
            ..default()
        },
        TextColor(HELP_TEXT_COLOR),
        Visibility::Hidden,
        HelpText,
    ));
}

fn sync_hud(
    redraw_requested: Res<RedrawRequested>,
    diagnostics: Res<DiagnosticsOverlay>,
    grid_overlay: Res<GridOverlay>,
    sim: Res<Simulation>,
    mut hud_q: Query<&mut Text, (With<Hud>, Without<HelpText>)>,
    mut help_q: Query<&mut Visibility, With<HelpText>>,
) {
    if !redraw_requested.0 {
        return;
    }

    let Ok(mut hud) = hud_q.single_mut() else {
        return;
    };
    let mode = if sim.app.is_running() {
        "Running"
    } else {
        "Paused"
    };
    let live_cells = sim.app.board.live_cells_count();
    let stamp = sim.app.selected_pattern_name().unwrap_or("Draw");
    let grid = if grid_overlay.enabled { "On" } else { "Off" };
    let diagnostics_text = if diagnostics.enabled {
        format!(" | Step(avg): {:.3}ms", sim.avg_step_ms)
    } else {
        String::new()
    };
    *hud = Text::new(format!(
        "Epoch: {} | {} | Tick: {}ms | Live: {} | Stamp: {} | Grid: {}{}",
        sim.app.epoch,
        mode,
        sim.app.tick_rate_ms(),
        live_cells,
        stamp,
        grid,
        diagnostics_text
    ));

    let Ok(mut help_visibility) = help_q.single_mut() else {
        return;
    };
    *help_visibility = if sim.app.is_help_visible() {
        Visibility::Visible
    } else {
        Visibility::Hidden
    };
}
