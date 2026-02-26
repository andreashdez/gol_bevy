use crate::shared::{DiagnosticsOverlay, HelpText, Hud, RedrawRequested, Simulation, UpdateSet};
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
            top: Val::Px(12.0),
            left: Val::Px(12.0),
            ..default()
        },
        TextFont {
            font_size: 19.0,
            ..default()
        },
        TextColor(Color::srgb(0.95, 0.95, 0.95)),
        Hud,
    ));

    commands.spawn((
        Text::new(controls_text()),
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(12.0),
            top: Val::Px(12.0),
            ..default()
        },
        TextFont {
            font_size: 16.0,
            ..default()
        },
        TextColor(Color::srgb(0.80, 0.82, 0.88)),
        Visibility::Hidden,
        HelpText,
    ));
}

fn sync_hud(
    redraw_requested: Res<RedrawRequested>,
    diagnostics: Res<DiagnosticsOverlay>,
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
    let diagnostics_text = if diagnostics.enabled {
        format!(" | Step(avg): {:.3}ms", sim.avg_step_ms)
    } else {
        String::new()
    };
    *hud = Text::new(format!(
        "Epoch: {} | {} | Tick: {}ms | Live: {}{}",
        sim.app.epoch,
        mode,
        sim.app.tick_rate_ms(),
        live_cells,
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

fn controls_text() -> &'static str {
    "Controls\n\nSpace: run/pause\nj: step\nr: randomize\nx: clear\n0: reset epoch\n+ / -: speed\ng: glider\nb: blinker\nl: LWSS\nF3: toggle diagnostics\n/: toggle help\nLeft drag: draw\nRight drag: erase\nq / Esc: quit"
}
