use crate::{
    app::{App as GolApp, EditTool},
    board::frontier_dense_threshold_percent_for_diagnostics,
    native_ui_plugin::{NativeUi, panel_rects},
    patterns::Pattern,
    shared::{
        BOARD_SIZE_MAX, BOARD_SIZE_MIN, BOARD_SIZE_PRESETS, BoardSettings, CONTROLS_TEXT,
        DiagnosticsOverlay, GridOverlay, Simulation, StartupSet, UiLayout, UiLayoutPreset,
        UiPointerCapture,
    },
};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use std::path::PathBuf;

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiLayout>()
            .init_resource::<UiPointerCapture>()
            .init_resource::<UiState>()
            .add_systems(
                Startup,
                setup_layout_persistence.in_set(StartupSet::LoadSettings),
            )
            .add_systems(Update, persist_settings)
            .add_systems(EguiPrimaryContextPass, render_ui);
    }
}

#[derive(Resource)]
struct UiLayoutPersistence {
    path: PathBuf,
    last_saved: Option<PersistedSettings>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PersistedSettings {
    preset: UiLayoutPreset,
    board: (usize, usize),
}

#[derive(Default)]
struct ParsedSettings {
    preset: Option<UiLayoutPreset>,
    board: Option<(usize, usize)>,
}

#[derive(Resource)]
struct UiState {
    rle_name: String,
    rle_source: String,
    import_feedback: Option<(bool, String)>,
    import_open: bool,
    style_initialized: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            rle_name: "Imported Pattern".to_string(),
            rle_source: "x = 3, y = 3\nbo$2bo$3o!".to_string(),
            import_feedback: None,
            import_open: false,
            style_initialized: false,
        }
    }
}

const ACCENT: egui::Color32 = egui::Color32::from_rgb(56, 217, 143);
const MUTED: egui::Color32 = egui::Color32::from_rgb(155, 167, 180);

/// The two overlay toggles are always read together by the View menu.
#[derive(SystemParam)]
struct Overlays<'w> {
    diagnostics: ResMut<'w, DiagnosticsOverlay>,
    grid: ResMut<'w, GridOverlay>,
    native: ResMut<'w, NativeUi>,
}

fn render_ui(
    mut contexts: EguiContexts,
    mut sim: ResMut<Simulation>,
    mut overlays: Overlays,
    mut ui_layout: ResMut<UiLayout>,
    mut ui_state: ResMut<UiState>,
    mut pointer_capture: ResMut<UiPointerCapture>,
    mut board_settings: ResMut<BoardSettings>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    if overlays.native.enabled {
        let size = ctx.viewport_rect().size();
        let panels: Vec<_> = panel_rects(Vec2::new(size.x, size.y), &ui_layout)
            .into_iter()
            .map(|r| {
                egui::Rect::from_min_max(egui::pos2(r.min.x, r.min.y), egui::pos2(r.max.x, r.max.y))
            })
            .collect();
        pointer_capture.blocked = overlays.native.dialog.is_some()
            || ctx.input(|input| {
                pointer_over_panels(input.pointer.latest_pos(), &input.events, &panels)
            });
        return;
    }
    initialize_visuals(&mut ui_state, ctx);
    let mut viewport_ui = egui::Ui::new(
        ctx.clone(),
        "viewport".into(),
        egui::UiBuilder::new()
            .layer_id(egui::LayerId::background())
            .max_rect(ctx.viewport_rect()),
    );

    let panels = {
        let toolbar =
            egui::Panel::top("playback_toolbar")
                .resizable(false)
                .show(&mut viewport_ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.strong("LIFE");
                        if ui.button("Native preview").clicked() {
                            overlays.native.enabled = true;
                        }
                        ui.separator();
                        let label = if sim.app.is_running() {
                            "Pause"
                        } else {
                            "Play"
                        };
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(label)
                                        .strong()
                                        .color(egui::Color32::from_rgb(12, 30, 22)),
                                )
                                .fill(ACCENT)
                                .min_size(egui::vec2(80.0, 32.0)),
                            )
                            .on_hover_text("Play / pause · Space")
                            .clicked()
                        {
                            sim.app.toggle_running();
                        }
                        if ui
                            .button("Step")
                            .on_hover_text("Advance one generation · J")
                            .clicked()
                        {
                            sim.app.increment_epoch();
                        }
                        render_speed(ui, &mut sim.app);
                        ui.separator();
                        if ui
                            .add_enabled(sim.app.can_undo(), egui::Button::new("Undo"))
                            .on_hover_text("Restore the previous edit and pause · U or Cmd/Ctrl+Z")
                            .clicked()
                        {
                            sim.app.undo();
                        }
                        if ui
                            .add_enabled(sim.app.can_redo(), egui::Button::new("Redo"))
                            .on_hover_text("Reapply the last undone edit · Cmd/Ctrl+Shift+Z")
                            .clicked()
                        {
                            sim.app.redo();
                        }
                        if !ui_layout.is_control_deck_visible() {
                            ui.menu_button("Tools", |ui| {
                                ui.set_width(260.0);
                                egui::ScrollArea::vertical()
                                    .max_height(420.0)
                                    .show(ui, |ui| {
                                        render_tools(ui, &mut sim.app);
                                        ui.separator();
                                        render_palette(ui, &mut sim.app);
                                        ui.separator();
                                        render_board_controls(
                                            ui,
                                            &mut sim.app,
                                            &mut board_settings,
                                        );
                                        if ui.button("Import pattern…").clicked() {
                                            ui_state.import_open = true;
                                            ui.close();
                                        }
                                    });
                            });
                        }
                        ui.menu_button("View", |ui| {
                            ui.checkbox(&mut overlays.grid.enabled, "Grid");
                            ui.checkbox(&mut overlays.diagnostics.enabled, "Diagnostics");
                            ui.separator();
                            ui.label("Layout");
                            for preset in UiLayoutPreset::ALL {
                                ui.selectable_value(&mut ui_layout.preset, preset, preset.label());
                            }
                            if ui_layout.preset.uses_side_panel() {
                                ui.checkbox(&mut ui_layout.control_deck_visible, "Pattern sidebar");
                            }
                        });
                        ui.menu_button("More", |ui| {
                            if ui
                                .button("Reset generation counter")
                                .on_hover_text("Sets the counter to zero; keeps all cells")
                                .clicked()
                            {
                                sim.app.reset_epoch();
                                ui.close();
                            }
                            if ui.button("Quit").clicked() {
                                sim.app.quit();
                            }
                        });
                        if ui
                            .button("Help")
                            .on_hover_text("Keyboard shortcuts · /")
                            .clicked()
                        {
                            sim.app.toggle_help();
                        }
                    });
                });

        let status =
            egui::Panel::bottom("status_bar")
                .resizable(false)
                .show(&mut viewport_ui, |ui| {
                    let live = sim.app.board.live_cells_count();
                    let (cols, rows) = sim.app.board.size();
                    let density = live as f64 / cols.saturating_mul(rows).max(1) as f64 * 100.0;
                    ui.horizontal_wrapped(|ui| {
                        ui.colored_label(
                            if sim.app.is_running() { ACCENT } else { MUTED },
                            if sim.app.is_running() {
                                "Running"
                            } else {
                                "Paused"
                            },
                        );
                        ui.separator();
                        ui.label(
                            egui::RichText::new(format!("Generation {}", sim.app.epoch))
                                .monospace(),
                        );
                        ui.label(
                            egui::RichText::new(format!("Population {live} · {density:.1}%"))
                                .color(MUTED),
                        );
                        ui.label(
                            egui::RichText::new(sim.app.rule_label())
                                .monospace()
                                .color(MUTED),
                        )
                        .on_hover_text("Birth/survival rule · set by imported RLE headers");
                        ui.separator();
                        ui.colored_label(
                            MUTED,
                            match sim.app.tool() {
                                EditTool::Draw => "Drag to draw · Right-drag to erase",
                                EditTool::Erase => "Drag to erase · U or Cmd/Ctrl+Z undo",
                                EditTool::Stamp => "Click to place · Z rotate · F flip",
                            },
                        );
                    });
                });

        let sidebar = if ui_layout.is_control_deck_visible() {
            Some(
                egui::Panel::left(egui::Id::new((
                    "control_deck",
                    ui_layout.preset.as_storage_str(),
                )))
                .default_size(ui_layout.preset.side_panel_width())
                .size_range(200.0..=420.0)
                .resizable(true)
                .show(&mut viewport_ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            render_tools(ui, &mut sim.app);
                            ui.add_space(8.0);
                            ui.separator();
                            render_palette(ui, &mut sim.app);
                            ui.add_space(8.0);
                            if ui
                                .add_sized(
                                    [ui.available_width(), 32.0],
                                    egui::Button::new("Import pattern…"),
                                )
                                .clicked()
                            {
                                ui_state.import_open = true;
                            }
                            ui.add_space(8.0);
                            ui.separator();
                            render_board_controls(ui, &mut sim.app, &mut board_settings);
                        });
                })
                .response
                .rect,
            )
        } else {
            None
        };

        [
            Some(toolbar.response.rect),
            Some(status.response.rect),
            sidebar,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
    };

    if overlays.diagnostics.enabled {
        egui::Window::new("Diagnostics")
            .open(&mut overlays.diagnostics.enabled)
            .default_pos([720.0, 70.0])
            .resizable(false)
            .show(ctx, |ui| {
                ui.monospace(format!("Average step: {:.3} ms", sim.avg_step_ms));
                ui.label(format!(
                    "Dense fallback threshold: {}%",
                    frontier_dense_threshold_percent_for_diagnostics()
                ));
                ui.label(format!("Tick interval: {} ms", sim.app.tick_rate_ms()));
            });
    }
    let mut import_open = ui_state.import_open;
    egui::Window::new("Import pattern")
        .open(&mut import_open)
        .default_width(380.0)
        .vscroll(true)
        .show(ctx, |ui| render_pattern_lab(ui, &mut sim, &mut ui_state));
    ui_state.import_open = import_open;

    let mut help_open = sim.app.is_help_visible();
    egui::Window::new("Controls and Tips")
        .open(&mut help_open)
        .default_size([420.0, 480.0])
        .vscroll(true)
        .show(ctx, |ui| {
            ui.label("Draw, erase, or choose a pattern, then press Space to play.");
            ui.label("U or Cmd/Ctrl+Z undoes an edit and pauses; Cmd/Ctrl+Shift+Z redoes it.");
            ui.label("One drag is one edit. A new edit discards the redo chain.");
            ui.label("History keeps up to 32 edits and clears when board dimensions change.");
            ui.separator();
            ui.label(egui::RichText::new(CONTROLS_TEXT.as_str()).monospace());
        });
    sim.app.set_help_visible(help_open);
    pointer_capture.blocked =
        ctx.input(|input| pointer_over_panels(input.pointer.latest_pos(), &input.events, &panels));
    // A tool button can retain keyboard focus after a canvas click. Give focus
    // back to the canvas so shortcuts work, while preserving text-editor focus
    // when interacting with import/help windows.
    if !pointer_capture.blocked
        && !ctx.is_pointer_over_egui()
        && ctx.input(|input| input.pointer.any_pressed())
    {
        ctx.memory_mut(|memory| {
            if let Some(focused) = memory.focused() {
                memory.surrender_focus(focused);
            }
        });
    }
}

fn pointer_over_panels(
    position: Option<egui::Pos2>,
    events: &[egui::Event],
    panels: &[egui::Rect],
) -> bool {
    let over_panel = |pos| panels.iter().any(|rect| rect.contains(pos));
    position.is_some_and(over_panel)
        || events.iter().any(|event| {
            matches!(event,
        egui::Event::PointerButton { pos, .. } if over_panel(*pos))
        })
}

fn render_speed(ui: &mut egui::Ui, app: &mut GolApp) {
    let (min_ms, max_ms, _) = GolApp::tick_rate_bounds();
    let mut speed = 1000.0 / app.tick_rate_ms() as f64;
    if ui
        .add(
            egui::Slider::new(
                &mut speed,
                (1000.0 / max_ms as f64)..=(1000.0 / min_ms as f64),
            )
            .logarithmic(true)
            .text("generations/s")
            .max_decimals(1),
        )
        .on_hover_text("Simulation speed · − slower · + faster")
        .changed()
    {
        app.set_tick_rate_ms((1000.0 / speed).round() as u64);
    }
}

fn render_tools(ui: &mut egui::Ui, app: &mut GolApp) {
    ui.label(egui::RichText::new("TOOLS").small().color(MUTED));
    ui.horizontal_wrapped(|ui| {
        for (tool, label) in [
            (EditTool::Draw, "Draw"),
            (EditTool::Erase, "Erase"),
            (EditTool::Stamp, "Stamp"),
        ] {
            if ui.selectable_label(app.tool() == tool, label).clicked() {
                app.set_tool(tool);
            }
        }
    });
    if app.tool() == EditTool::Stamp {
        ui.add_space(4.0);
        ui.label(app.selected_pattern_status_label());
        if let Some((cells, width, height)) = app.selected_pattern_layout() {
            let (rect, _) = ui
                .allocate_exact_size(egui::vec2(ui.available_width(), 72.0), egui::Sense::hover());
            paint_cells(ui, rect.shrink(8.0), &cells, width, height, ACCENT);
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("Rotate 90°")
                .on_hover_text("Rotate clockwise · Z")
                .clicked()
            {
                app.rotate_stamp_clockwise();
            }
            if ui
                .button("Flip")
                .on_hover_text("Flip horizontally · F")
                .clicked()
            {
                app.toggle_stamp_flip();
            }
        });
    }
}

fn render_board_size(ui: &mut egui::Ui, app: &GolApp, settings: &mut BoardSettings) {
    let (cols, rows) = app.board.size();
    ui.label(egui::RichText::new("Size").color(MUTED));
    ui.horizontal_wrapped(|ui| {
        for (preset_cols, preset_rows) in BOARD_SIZE_PRESETS {
            let selected = (cols, rows) == (preset_cols, preset_rows);
            if ui
                .selectable_label(selected, format!("{preset_cols}²"))
                .on_hover_text("Resize the board · clears undo history")
                .clicked()
                && !selected
            {
                settings.set(preset_cols, preset_rows);
            }
        }
    });

    let mut requested_cols = cols;
    let mut requested_rows = rows;
    ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(&mut requested_cols)
                .range(BOARD_SIZE_MIN..=BOARD_SIZE_MAX)
                .prefix("W "),
        );
        ui.add(
            egui::DragValue::new(&mut requested_rows)
                .range(BOARD_SIZE_MIN..=BOARD_SIZE_MAX)
                .prefix("H "),
        );
    });
    if (requested_cols, requested_rows) != (cols, rows) {
        settings.set(requested_cols, requested_rows);
    }

    ui.label(
        egui::RichText::new(format!(
            "{cols} × {rows} cells · window resizing no longer resizes"
        ))
        .small()
        .color(MUTED),
    );
}

fn render_board_controls(ui: &mut egui::Ui, app: &mut GolApp, settings: &mut BoardSettings) {
    ui.label(egui::RichText::new("BOARD").small().color(MUTED));
    render_board_size(ui, app, settings);
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if ui
            .button("Randomize")
            .on_hover_text("Fill with random cells · R")
            .clicked()
        {
            app.randomize_board();
        }
        if ui
            .button("Clear")
            .on_hover_text("Clear the board · X · Recover with Undo")
            .clicked()
        {
            app.clear_board();
        }
    });
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Edges").color(MUTED));
    ui.horizontal_wrapped(|ui| {
        let wrap = app.boundary_mode_label() == "Wrap";
        if ui
            .selectable_label(wrap, "Wrap")
            .on_hover_text("Cells connect across opposite edges")
            .clicked()
            && !wrap
        {
            app.toggle_boundary_mode();
        }
        if ui
            .selectable_label(!wrap, "Fixed")
            .on_hover_text("Cells outside the board stay dead")
            .clicked()
            && wrap
        {
            app.toggle_boundary_mode();
        }
    });
}

fn render_palette(ui: &mut egui::Ui, app: &mut GolApp) {
    ui.label(egui::RichText::new("PATTERNS").small().color(MUTED));
    let available = ui.available_width();
    let gap = ui.spacing().item_spacing.x;
    let columns = ((available + gap) / 110.0).floor().max(1.0);
    let width = ((available - gap * (columns - 1.0)) / columns).max(70.0);
    ui.horizontal_wrapped(|ui| {
        for index in 0..app.pattern_count() {
            let Some(pattern) = app.pattern_at(index) else {
                continue;
            };
            let selected =
                app.tool() == EditTool::Stamp && app.selected_pattern_index() == Some(index);
            if pattern_card(ui, pattern, selected, width).clicked() {
                app.select_pattern_index(index);
            }
        }
    });
}

fn pattern_card(
    ui: &mut egui::Ui,
    pattern: &Pattern,
    selected: bool,
    width: f32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 100.0), egui::Sense::click());
    let fill = if selected {
        egui::Color32::from_rgb(26, 57, 44)
    } else if response.hovered() {
        egui::Color32::from_rgb(38, 44, 51)
    } else {
        egui::Color32::from_rgb(26, 30, 35)
    };
    ui.painter().rect_filled(rect, 6.0, fill);
    let border = if selected || response.has_focus() {
        ACCENT
    } else {
        egui::Color32::from_rgb(50, 58, 65)
    };
    ui.painter().rect_stroke(
        rect,
        6.0,
        egui::Stroke::new(1.0, border),
        egui::StrokeKind::Inside,
    );
    let preview = egui::Rect::from_min_max(
        rect.min + egui::vec2(10.0, 8.0),
        rect.max - egui::vec2(10.0, 30.0),
    );
    paint_cells(
        ui,
        preview,
        &pattern.cells,
        pattern.width,
        pattern.height,
        if selected {
            ACCENT
        } else {
            egui::Color32::from_rgb(130, 200, 163)
        },
    );
    ui.painter()
        .with_clip_rect(rect.shrink(6.0).intersect(ui.clip_rect()))
        .text(
            egui::pos2(rect.center().x, rect.bottom() - 16.0),
            egui::Align2::CENTER_CENTER,
            &pattern.name,
            egui::FontId::proportional(13.0),
            ui.visuals().text_color(),
        );
    response.widget_info(|| {
        egui::WidgetInfo::selected(
            egui::WidgetType::SelectableLabel,
            ui.is_enabled(),
            selected,
            &pattern.name,
        )
    });
    response.on_hover_text(format!(
        "{} · {} × {} · {} cells",
        pattern.name,
        pattern.width,
        pattern.height,
        pattern.cells.len()
    ))
}

fn paint_cells(
    ui: &egui::Ui,
    rect: egui::Rect,
    cells: &[(usize, usize)],
    width: usize,
    height: usize,
    color: egui::Color32,
) {
    let size = (rect.width() / width.max(1) as f32)
        .min(rect.height() / height.max(1) as f32)
        .min(12.0);
    let origin = rect.center() - egui::vec2(width as f32, height as f32) * size * 0.5;
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    for &(x, y) in cells {
        // Board rows grow upward in world space; match the actual stamp preview.
        let position = origin + egui::vec2(x as f32, height.saturating_sub(1 + y) as f32) * size;
        painter.rect_filled(
            egui::Rect::from_min_size(position, egui::Vec2::splat((size - 1.0).max(size * 0.8))),
            0.0,
            color,
        );
    }
}

fn render_pattern_lab(ui: &mut egui::Ui, sim: &mut Simulation, state: &mut UiState) {
    ui.label(egui::RichText::new("Paste RLE text to add a pattern to your palette.").color(MUTED));
    ui.label("Name");
    ui.add(egui::TextEdit::singleline(&mut state.rle_name).desired_width(f32::INFINITY));
    ui.label("RLE source");
    ui.add(
        egui::TextEdit::multiline(&mut state.rle_source)
            .desired_width(f32::INFINITY)
            .desired_rows(8)
            .code_editor(),
    );
    if ui.button("Import and select").clicked() {
        let name = if state.rle_name.trim().is_empty() {
            format!("Imported {}", sim.app.pattern_count() + 1)
        } else {
            state.rle_name.trim().to_string()
        };
        state.import_feedback = Some(
            match sim.app.import_rle_pattern(name.clone(), &state.rle_source) {
                Ok(import) => {
                    let mut message = format!("Imported '{name}'. Click the board to place it.");
                    if let Some(rule) = import.applied_rule {
                        message.push_str(&format!(" Board rule set to {rule}."));
                    }
                    (true, message)
                }
                Err(err) => (false, format!("Could not import: {err}")),
            },
        );
    }
    if let Some((ok, message)) = &state.import_feedback {
        ui.colored_label(
            if *ok {
                ACCENT
            } else {
                egui::Color32::from_rgb(245, 153, 145)
            },
            message,
        );
    }
}

fn initialize_visuals(state: &mut UiState, ctx: &egui::Context) {
    if state.style_initialized {
        return;
    }
    let mut visuals = egui::Visuals::dark();
    visuals.window_fill = egui::Color32::from_rgb(22, 25, 30);
    visuals.panel_fill = egui::Color32::from_rgb(17, 20, 24);
    visuals.widgets.noninteractive.fg_stroke.color = egui::Color32::from_rgb(225, 231, 236);
    visuals.widgets.inactive.bg_fill = egui::Color32::from_rgb(35, 41, 48);
    visuals.widgets.hovered.bg_fill = egui::Color32::from_rgb(48, 59, 66);
    visuals.widgets.active.bg_fill = egui::Color32::from_rgb(30, 93, 66);
    visuals.selection.bg_fill = egui::Color32::from_rgb(31, 93, 67);
    visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    ctx.set_visuals(visuals);
    ctx.global_style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(10.0, 6.0);
        style.spacing.interact_size.y = 32.0;
        style.spacing.slider_width = 110.0;
        style.spacing.indent = 12.0;
        style.visuals.window_corner_radius = egui::CornerRadius::same(8);
        style.visuals.menu_corner_radius = egui::CornerRadius::same(6);
        style.visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(5);
        style.visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(5);
        style.visuals.widgets.active.corner_radius = egui::CornerRadius::same(5);
    });
    state.style_initialized = true;
}

fn setup_layout_persistence(
    mut commands: Commands,
    mut ui_layout: ResMut<UiLayout>,
    mut board_settings: ResMut<BoardSettings>,
) {
    let path = ui_layout_config_path();

    match std::fs::read_to_string(&path) {
        Ok(contents) => {
            let settings = parse_settings_from_toml(&contents);
            match settings.preset {
                Some(preset) => ui_layout.preset = preset,
                None => warn!(
                    "Ignoring invalid persisted layout preset in {}",
                    path.display()
                ),
            }
            // Left unset, startup falls back to a board that fills the window.
            if let Some((cols, rows)) = settings.board {
                board_settings.set(cols, rows);
            }
        }
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    "Failed to read persisted settings from {}: {}",
                    path.display(),
                    err
                );
            }
        }
    }

    commands.insert_resource(UiLayoutPersistence {
        path,
        last_saved: None,
    });
}

fn persist_settings(
    ui_layout: Res<UiLayout>,
    board_settings: Res<BoardSettings>,
    mut persistence: ResMut<UiLayoutPersistence>,
) {
    if !board_settings.is_set() {
        return;
    }
    let current = PersistedSettings {
        preset: ui_layout.preset,
        board: (board_settings.cols, board_settings.rows),
    };
    if persistence.last_saved == Some(current) {
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
        "layout_preset = \"{}\"\nboard_cols = {}\nboard_rows = {}\n",
        current.preset.as_storage_str(),
        current.board.0,
        current.board.1
    );
    if let Err(err) = std::fs::write(&persistence.path, contents) {
        warn!(
            "Failed to persist settings to {}: {}",
            persistence.path.display(),
            err
        );
        return;
    }

    persistence.last_saved = Some(current);
}

/// Minimal key/value reader for the handful of scalars this app persists.
fn parse_settings_from_toml(contents: &str) -> ParsedSettings {
    let mut parsed = ParsedSettings::default();
    let mut cols = None;
    let mut rows = None;

    for line in contents.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let value = unquote(raw_value.trim());

        match key.trim() {
            "layout_preset" => parsed.preset = UiLayoutPreset::from_storage_str(value),
            "board_cols" => cols = value.parse::<usize>().ok(),
            "board_rows" => rows = value.parse::<usize>().ok(),
            _ => {}
        }
    }

    if let (Some(cols), Some(rows)) = (cols, rows) {
        parsed.board = Some((cols, rows));
    }
    parsed
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|inner| inner.strip_suffix('\''))
        })
        .unwrap_or(value)
        .trim()
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

#[cfg(test)]
mod pointer_tests {
    use super::*;

    #[test]
    fn toolbar_click_is_captured_even_if_pointer_moves_back_to_canvas_in_same_frame() {
        let panels = [egui::Rect::from_min_max(
            egui::pos2(0.0, 0.0),
            egui::pos2(800.0, 48.0),
        )];
        let click = egui::Event::PointerButton {
            pos: egui::pos2(80.0, 24.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::default(),
        };
        assert!(pointer_over_panels(
            Some(egui::pos2(500.0, 300.0)),
            &[click],
            &panels
        ));
        assert!(!pointer_over_panels(
            Some(egui::pos2(500.0, 300.0)),
            &[],
            &panels
        ));
    }

    #[test]
    fn panel_background_blocks_painting_even_without_an_interactive_widget() {
        let sidebar = egui::Rect::from_min_max(egui::pos2(0.0, 48.0), egui::pos2(280.0, 760.0));
        assert!(pointer_over_panels(
            Some(egui::pos2(100.0, 700.0)),
            &[],
            &[sidebar]
        ));
        assert!(!pointer_over_panels(
            Some(egui::pos2(100.0, 700.0)),
            &[],
            &[]
        ));
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::{parse_settings_from_toml, unquote};
    use crate::shared::UiLayoutPreset;

    #[test]
    fn reads_layout_preset_and_board_size() {
        let parsed = parse_settings_from_toml(
            "# saved by gol-bevy\nlayout_preset = \"compact\"\nboard_cols = 512\nboard_rows = 256\n",
        );
        assert_eq!(parsed.preset, Some(UiLayoutPreset::Compact));
        assert_eq!(parsed.board, Some((512, 256)));
    }

    #[test]
    fn a_partial_or_invalid_file_leaves_fields_unset() {
        // Only one of the two dimensions is useless on its own.
        let half = parse_settings_from_toml("layout_preset = 'minimal'\nboard_cols = 512\n");
        assert_eq!(half.preset, Some(UiLayoutPreset::Minimal));
        assert_eq!(half.board, None);

        let junk = parse_settings_from_toml("layout_preset = \"nope\"\nboard_cols = wide\n");
        assert_eq!(junk.preset, None);
        assert_eq!(junk.board, None);

        assert_eq!(parse_settings_from_toml("").board, None);
    }

    #[test]
    fn unquote_handles_both_quote_styles_and_bare_values() {
        assert_eq!(unquote("\"full\""), "full");
        assert_eq!(unquote("'full'"), "full");
        assert_eq!(unquote("512"), "512");
    }
}
