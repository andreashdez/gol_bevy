use crate::{
    app::App as GolApp,
    board::frontier_dense_threshold_percent_for_diagnostics,
    shared::{
        CONTROLS_TEXT, DiagnosticsOverlay, GridOverlay, Simulation, UiLayout, UiLayoutPreset,
    },
};
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use std::path::PathBuf;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiLayout>()
            .init_resource::<UiState>()
            .add_systems(Startup, setup_layout_persistence)
            .add_systems(Update, persist_layout_preset)
            .add_systems(EguiPrimaryContextPass, render_ui);
    }
}

#[derive(Resource)]
struct UiLayoutPersistence {
    path: PathBuf,
    last_saved: Option<UiLayoutPreset>,
}

#[derive(Resource)]
struct UiState {
    rle_name: String,
    rle_source: String,
    import_feedback: Option<(bool, String)>,
    style_initialized: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            rle_name: "Imported Pattern".to_string(),
            rle_source: "x = 3, y = 3\nbo$2bo$3o!".to_string(),
            import_feedback: None,
            style_initialized: false,
        }
    }
}

fn render_ui(
    mut contexts: EguiContexts,
    mut sim: ResMut<Simulation>,
    mut diagnostics: ResMut<DiagnosticsOverlay>,
    mut grid_overlay: ResMut<GridOverlay>,
    mut ui_layout: ResMut<UiLayout>,
    mut ui_state: ResMut<UiState>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    initialize_visuals(&mut ui_state, ctx);

    let mut viewport_ui = egui::Ui::new(
        ctx.clone(),
        "viewport".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );

    let mode = if sim.app.is_running() {
        "Running"
    } else {
        "Paused"
    };
    let live_cells = sim.app.board.live_cells_count();
    let (cols, rows) = sim.app.board.size();
    let total_cells = cols.saturating_mul(rows).max(1);
    let density = live_cells as f32 / total_cells as f32;
    let boundary = sim.app.boundary_mode_label();

    egui::Panel::top("status_bar")
        .resizable(false)
        .show(&mut viewport_ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.heading("GOL");
                ui.separator();
                ui.label(format!("Epoch {}", sim.app.epoch));
                ui.label(format!("Mode: {mode}"));
                ui.label(format!("Tick: {} ms", sim.app.tick_rate_ms()));
                ui.label(format!("Live: {live_cells}"));
                ui.label(format!("Boundary: {boundary}"));
                if diagnostics.enabled {
                    ui.label(format!("Step(avg): {:.3} ms", sim.avg_step_ms));
                    ui.label(format!(
                        "Frontier dense fallback: {}%",
                        frontier_dense_threshold_percent_for_diagnostics()
                    ));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    render_layout_selector(ui, &mut ui_layout.preset);
                    ui.separator();
                    if ui_layout.preset.uses_side_panel() {
                        if ui
                            .button(if ui_layout.control_deck_visible {
                                "Hide Deck"
                            } else {
                                "Show Deck"
                            })
                            .clicked()
                        {
                            ui_layout.toggle_control_deck();
                        }
                        ui.separator();
                    }
                    if ui
                        .button(if sim.app.is_help_visible() {
                            "Hide Help"
                        } else {
                            "Show Help"
                        })
                        .clicked()
                    {
                        sim.app.toggle_help();
                    }
                });
            });
            if ui_layout.preset != UiLayoutPreset::Minimal {
                ui.add(
                    egui::ProgressBar::new(density)
                        .text(format!("Population density: {:>5.1}%", density * 100.0)),
                );
            }
        });

    if ui_layout.is_control_deck_visible() {
        render_control_deck(
            &mut viewport_ui,
            &mut sim,
            &mut diagnostics,
            &mut grid_overlay,
            ui_layout.preset,
            &mut ui_state,
        );
    } else if ui_layout.preset == UiLayoutPreset::Minimal {
        render_minimal_controls(ctx, &mut sim, &mut diagnostics, &mut grid_overlay);
    }

    let mut show_help = sim.app.is_help_visible();
    if show_help {
        egui::Window::new("Controls and Tips")
            .default_size([420.0, 640.0])
            .resizable(false)
            .open(&mut show_help)
            .show(ctx, |ui| {
                ui.label(
                    "Keyboard and mouse shortcuts remain active when focus is not in a UI widget.",
                );
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.label(egui::RichText::new(CONTROLS_TEXT.as_str()).monospace());
                });
            });
    }
    sim.app.set_help_visible(show_help);
}

fn render_control_deck(
    viewport_ui: &mut egui::Ui,
    sim: &mut Simulation,
    diagnostics: &mut DiagnosticsOverlay,
    grid_overlay: &mut GridOverlay,
    preset: UiLayoutPreset,
    ui_state: &mut UiState,
) {
    egui::Panel::left("control_deck")
        .default_size(preset.side_panel_width())
        .resizable(preset == UiLayoutPreset::Full)
        .show(viewport_ui, |ui| {
            ui.heading("GOL Control Deck");
            if preset == UiLayoutPreset::Full {
                ui.label("Hands-on controls for simulation, overlays, and stamp workflows.");
            }
            ui.separator();

            render_simulation_actions(ui, sim);
            ui.separator();
            render_world_controls(ui, sim, diagnostics, grid_overlay);
            ui.separator();
            render_stamp_studio(ui, sim);

            if preset.show_pattern_lab() {
                ui.separator();
                egui::CollapsingHeader::new("Pattern Lab")
                    .default_open(preset == UiLayoutPreset::Full)
                    .show(ui, |ui| render_pattern_lab(ui, sim, ui_state));
            }
        });
}

fn render_minimal_controls(
    ctx: &egui::Context,
    sim: &mut Simulation,
    diagnostics: &mut DiagnosticsOverlay,
    grid_overlay: &mut GridOverlay,
) {
    egui::Window::new("Quick Controls")
        .default_size([330.0, 260.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui
                    .add_sized(
                        [96.0, 26.0],
                        egui::Button::new(if sim.app.is_running() { "Pause" } else { "Run" }),
                    )
                    .clicked()
                {
                    sim.app.toggle_running();
                }
                if ui
                    .add_sized([70.0, 26.0], egui::Button::new("Step"))
                    .clicked()
                {
                    sim.app.increment_epoch();
                }
                if ui
                    .add_sized([74.0, 26.0], egui::Button::new("Clear"))
                    .clicked()
                {
                    sim.app.clear_board();
                }
                if ui
                    .add_sized([74.0, 26.0], egui::Button::new("Random"))
                    .clicked()
                {
                    sim.app.randomize_board();
                }
            });

            let (tick_min, tick_max, tick_step) = GolApp::tick_rate_bounds();
            let mut tick_rate = sim.app.tick_rate_ms();
            if ui
                .add(
                    egui::Slider::new(&mut tick_rate, tick_min..=tick_max)
                        .step_by(tick_step as f64)
                        .text("Tick (ms)"),
                )
                .changed()
            {
                sim.app.set_tick_rate_ms(tick_rate);
            }

            ui.horizontal(|ui| {
                let mut diagnostics_enabled = diagnostics.enabled;
                if ui
                    .checkbox(&mut diagnostics_enabled, "Diagnostics")
                    .changed()
                {
                    diagnostics.enabled = diagnostics_enabled;
                }

                let mut grid_enabled = grid_overlay.enabled;
                if ui.checkbox(&mut grid_enabled, "Grid").changed() {
                    grid_overlay.enabled = grid_enabled;
                }
            });

            render_stamp_selector(ui, sim);
            ui.horizontal(|ui| {
                let has_stamp = sim.app.has_selected_pattern();
                if ui
                    .add_enabled(has_stamp, egui::Button::new("Rotate"))
                    .clicked()
                {
                    sim.app.rotate_stamp_clockwise();
                }
                if ui
                    .add_enabled(has_stamp, egui::Button::new("Flip"))
                    .clicked()
                {
                    sim.app.toggle_stamp_flip();
                }
            });
        });
}

fn render_layout_selector(ui: &mut egui::Ui, layout: &mut UiLayoutPreset) {
    egui::ComboBox::from_id_salt("layout_preset_select")
        .selected_text(format!("Layout: {}", layout.label()))
        .show_ui(ui, |ui| {
            for preset in UiLayoutPreset::ALL {
                ui.selectable_value(layout, preset, preset.label());
            }
        });
}

fn render_simulation_actions(ui: &mut egui::Ui, sim: &mut Simulation) {
    ui.horizontal(|ui| {
        if ui
            .add_sized(
                [104.0, 28.0],
                egui::Button::new(if sim.app.is_running() { "Pause" } else { "Run" }),
            )
            .clicked()
        {
            sim.app.toggle_running();
        }
        if ui
            .add_sized([88.0, 28.0], egui::Button::new("Step"))
            .clicked()
        {
            sim.app.increment_epoch();
        }
        if ui
            .add_sized([104.0, 28.0], egui::Button::new("Reset Epoch"))
            .clicked()
        {
            sim.app.reset_epoch();
        }
    });

    ui.horizontal(|ui| {
        if ui
            .add_sized([104.0, 28.0], egui::Button::new("Randomize"))
            .clicked()
        {
            sim.app.randomize_board();
        }
        if ui
            .add_sized([88.0, 28.0], egui::Button::new("Clear"))
            .clicked()
        {
            sim.app.clear_board();
        }
        if ui
            .add_sized([104.0, 28.0], egui::Button::new("Quit"))
            .clicked()
        {
            sim.app.quit();
        }
    });
}

fn render_world_controls(
    ui: &mut egui::Ui,
    sim: &mut Simulation,
    diagnostics: &mut DiagnosticsOverlay,
    grid_overlay: &mut GridOverlay,
) {
    let (tick_min, tick_max, tick_step) = GolApp::tick_rate_bounds();
    let mut tick_rate = sim.app.tick_rate_ms();
    if ui
        .add(
            egui::Slider::new(&mut tick_rate, tick_min..=tick_max)
                .step_by(tick_step as f64)
                .text("Tick interval (ms)"),
        )
        .changed()
    {
        sim.app.set_tick_rate_ms(tick_rate);
    }

    let mut diagnostics_enabled = diagnostics.enabled;
    if ui
        .checkbox(&mut diagnostics_enabled, "Diagnostics overlay")
        .changed()
    {
        diagnostics.enabled = diagnostics_enabled;
    }

    let mut grid_enabled = grid_overlay.enabled;
    if ui.checkbox(&mut grid_enabled, "Grid overlay").changed() {
        grid_overlay.enabled = grid_enabled;
    }

    let boundary = sim.app.boundary_mode_label();
    if ui
        .button(format!("Boundary mode: {boundary} (toggle)"))
        .clicked()
    {
        sim.app.toggle_boundary_mode();
    }
}

fn render_stamp_studio(ui: &mut egui::Ui, sim: &mut Simulation) {
    ui.heading("Stamp Studio");
    render_stamp_selector(ui, sim);

    let has_stamp = sim.app.has_selected_pattern();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(has_stamp, egui::Button::new("Rotate 90"))
            .clicked()
        {
            sim.app.rotate_stamp_clockwise();
        }
        if ui
            .add_enabled(has_stamp, egui::Button::new("Flip Horizontal"))
            .clicked()
        {
            sim.app.toggle_stamp_flip();
        }
        if ui
            .add_enabled(has_stamp, egui::Button::new("Clear Stamp"))
            .clicked()
        {
            sim.app.clear_selected_pattern();
        }
    });
    ui.label(format!(
        "Current tool: {}",
        sim.app.selected_pattern_status_label()
    ));
}

fn render_stamp_selector(ui: &mut egui::Ui, sim: &mut Simulation) {
    let prior_selection = sim.app.selected_pattern_index();
    let mut selected = prior_selection;
    let selected_text = sim
        .app
        .selected_pattern_name()
        .unwrap_or("Draw (manual)")
        .to_string();

    egui::ComboBox::from_label("Active Stamp")
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut selected, None, "Draw (manual)");
            for idx in 0..sim.app.pattern_count() {
                if let Some(name) = sim.app.pattern_name_at(idx) {
                    ui.selectable_value(&mut selected, Some(idx), name);
                }
            }
        });

    if selected != prior_selection {
        if let Some(index) = selected {
            sim.app.select_pattern_index(index);
        } else {
            sim.app.clear_selected_pattern();
        }
    }
}

fn render_pattern_lab(ui: &mut egui::Ui, sim: &mut Simulation, ui_state: &mut UiState) {
    ui.label("Import custom stamps from RLE text.");
    ui.horizontal(|ui| {
        ui.label("Name");
        ui.text_edit_singleline(&mut ui_state.rle_name);
    });
    ui.add(
        egui::TextEdit::multiline(&mut ui_state.rle_source)
            .desired_rows(8)
            .code_editor(),
    );

    ui.horizontal(|ui| {
        if ui.button("Import RLE as Stamp").clicked() {
            let generated_name = format!("Imported {}", sim.app.pattern_count() + 1);
            let name = if ui_state.rle_name.trim().is_empty() {
                generated_name
            } else {
                ui_state.rle_name.trim().to_string()
            };
            match sim
                .app
                .import_rle_pattern(name.clone(), &ui_state.rle_source)
            {
                Ok(()) => {
                    ui_state.import_feedback =
                        Some((true, format!("Imported '{name}' and selected it.")));
                }
                Err(err) => {
                    ui_state.import_feedback = Some((false, format!("RLE parse error: {err}")));
                }
            }
        }

        if ui.button("Clear Editor").clicked() {
            ui_state.rle_source.clear();
            ui_state.import_feedback = None;
        }
    });

    if let Some((ok, message)) = &ui_state.import_feedback {
        let color = if *ok {
            egui::Color32::from_rgb(120, 210, 150)
        } else {
            egui::Color32::from_rgb(230, 120, 120)
        };
        ui.colored_label(color, message);
    }
}

fn initialize_visuals(ui_state: &mut UiState, ctx: &egui::Context) {
    if ui_state.style_initialized {
        return;
    }

    let mut visuals = egui::Visuals::dark();
    visuals.override_text_color = Some(egui::Color32::from_rgb(230, 234, 240));
    visuals.window_fill = egui::Color32::from_rgb(22, 25, 30);
    visuals.panel_fill = egui::Color32::from_rgb(17, 20, 24);
    visuals.widgets.noninteractive.bg_fill = egui::Color32::from_rgb(35, 39, 45);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(45, 52, 60);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(56, 66, 76);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(31, 129, 106);
    visuals.widgets.open.bg_fill = egui::Color32::from_rgb(47, 58, 68);
    visuals.selection.bg_fill = egui::Color32::from_rgb(31, 129, 106);
    visuals.hyperlink_color = egui::Color32::from_rgb(118, 190, 237);
    ctx.set_visuals(visuals);

    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(10.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 7.0);
        style.spacing.indent = 16.0;
        style.visuals.window_corner_radius = egui::CornerRadius::same(8);
        style.visuals.menu_corner_radius = egui::CornerRadius::same(8);
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(6);
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(6);
        style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(6);
    });

    ui_state.style_initialized = true;
}

fn setup_layout_persistence(mut commands: Commands, mut ui_layout: ResMut<UiLayout>) {
    let path = ui_layout_config_path();

    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            if let Some(preset) = parse_layout_preset_from_toml(&contents) {
                ui_layout.preset = preset;
            } else {
                warn!(
                    "Ignoring invalid persisted layout preset in {}",
                    path.display()
                );
            }
        }
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    "Failed to read persisted layout preset from {}: {}",
                    path.display(),
                    err
                );
            }
        }
    }

    commands.insert_resource(UiLayoutPersistence {
        path,
        last_saved: Some(ui_layout.preset),
    });
}

fn persist_layout_preset(ui_layout: Res<UiLayout>, mut persistence: ResMut<UiLayoutPersistence>) {
    if persistence.last_saved == Some(ui_layout.preset) {
        return;
    }

    if let Some(parent) = persistence.path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        warn!(
            "Failed to create config directory {}: {}",
            parent.display(),
            err
        );
        return;
    }

    let contents = format!(
        "layout_preset = \"{}\"\n",
        ui_layout.preset.as_storage_str()
    );
    if let Err(err) = std::fs::write(&persistence.path, contents) {
        warn!(
            "Failed to persist layout preset to {}: {}",
            persistence.path.display(),
            err
        );
        return;
    }

    persistence.last_saved = Some(ui_layout.preset);
}

fn parse_layout_preset_from_toml(contents: &str) -> Option<UiLayoutPreset> {
    for line in contents.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }

        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "layout_preset" {
            continue;
        }

        let value = raw_value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|inner| inner.strip_suffix('"'))
            .or_else(|| {
                value
                    .strip_prefix('\'')
                    .and_then(|inner| inner.strip_suffix('\''))
            })
            .unwrap_or(value)
            .trim();

        return UiLayoutPreset::from_storage_str(value);
    }

    None
}

fn ui_layout_config_path() -> PathBuf {
    ui_layout_config_path_with_file("ui_layout.toml")
}

fn ui_layout_config_path_with_file(file_name: &str) -> PathBuf {
    if let Some(config_dir) = std::env::var_os("GOL_BEVY_CONFIG_DIR") {
        return PathBuf::from(config_dir).join(file_name);
    }

    if let Some(config_dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(config_dir).join("gol-bevy").join(file_name);
    }

    if let Some(home_dir) = std::env::var_os("HOME") {
        return PathBuf::from(home_dir)
            .join(".config")
            .join("gol-bevy")
            .join(file_name);
    }

    PathBuf::from(".gol-bevy").join("config").join(file_name)
}
