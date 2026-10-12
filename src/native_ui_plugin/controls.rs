//! Everyday controls, shortcut hints, and first-use guidance.
use super::*;
use crate::shared::{CAMERA_ZOOM_MAX, CAMERA_ZOOM_MIN, MainCamera, PATTERN_SHORTCUTS};
use bevy::{a11y::AccessibilityNode, text::FontFeatureTag};

#[derive(Component)]
pub(super) struct Tooltip(pub String);
#[derive(Component)]
pub(super) struct WideLabel;
#[derive(Component, Default)]
pub(super) struct TallLabel {
    pub always_show: bool,
}
#[derive(Component)]
pub(super) struct SpeedLabel;
#[derive(Component)]
pub(super) struct ZoomLabel;
#[derive(Component)]
pub(super) struct FeedbackOverlay;
#[derive(Component)]
pub(super) struct FeedbackText;
#[derive(Component)]
pub(super) struct CanvasToolHint;
#[derive(Component)]
pub(super) struct CanvasToolHintText;

pub(super) fn numeric_font(size: f32) -> TextFont {
    TextFont {
        font: UI_FONT.into(),
        font_size: FontSize::Px(size),
        font_features: [FontFeatureTag::TABULAR_FIGURES].into(),
        ..default()
    }
}
#[derive(Component)]
pub(super) enum Overlay {
    Hint {
        target: Option<Entity>,
        elapsed: f32,
    },
    Empty,
}
#[derive(Component)]
pub(super) struct OverlayText(bool);

/// Names the shortcut of a built-in by looking up which one the palette entry
/// shows, since entries are listed in file order rather than shortcut order.
pub(super) fn pattern_hint(name: &str, index: usize) -> String {
    let shortcut = PATTERN_SHORTCUTS
        .iter()
        .find(|(_, pattern)| pattern.builtin_index() == Some(index));
    match shortcut {
        Some((binding, _)) => format!(
            "Select {name} · {} · click the board to place",
            binding.label.to_uppercase()
        ),
        None => format!("Select {name} · click the board to place"),
    }
}

pub(super) fn hint(action: Action) -> &'static str {
    match action {
        Action::Play => "Play / pause · Space",
        Action::Step => "Step one generation · J",
        Action::Undo => "Undo the last edit · U",
        Action::Redo => {
            if cfg!(target_os = "macos") {
                "Redo the last edit · Cmd+Shift+Z"
            } else {
                "Redo the last edit · Ctrl+Shift+Z"
            }
        }
        Action::Grid => "Show / hide the grid · T",
        Action::CellAge => "Brighten newborn cells, deepen old ones, and fade where cells died",
        Action::FitBoard => "Fit the whole board in the canvas · V",
        Action::FitLiveCells => {
            "Fit live cells with a small margin · empty boards fit the whole board"
        }
        Action::ZoomIn => "Zoom in · scroll over the canvas to zoom · middle-drag to pan",
        Action::ZoomOut => "Zoom out · scroll over the canvas to zoom · middle-drag to pan",
        Action::MaxSpeed => "Toggle maximum speed · switch off to restore your selected rate",
        Action::SpeedAdjust(step) if step < 0 => "Decrease speed by 1 gen/s · leaves Max mode",
        Action::SpeedAdjust(_) => "Increase speed by 1 gen/s · leaves Max mode",
        Action::Sidebar => "Show / hide the palette · H",
        Action::ViewMenu => "View options · grid, cell age, and pattern palette",
        Action::MoreMenu => "Settings and help",
        Action::Help => "Controls and tips · /",
        Action::Diagnostics => "Simulation diagnostics · F3",
        Action::Rotate => "Rotate stamp 90° clockwise · Z",
        Action::Flip => "Flip stamp horizontally · F",
        Action::Randomize => "Fill the board with random cells · R",
        Action::Clear => "Clear the board · X · undo with U",
        Action::Boundary => "Wrap / fixed edges · M",
        Action::Tool(EditTool::Draw) => "Draw cells with the left mouse button",
        Action::Tool(EditTool::Erase) => "Erase cells · right-click also erases",
        Action::Tool(EditTool::Stamp) => "Place the selected pattern on the board",
        Action::Import => "Import RLE into your library, or copy the board as RLE",
        Action::Settings => "Board size, rule, theme, grid, cell age, and edges",
        Action::RulePreset(_) => "Switch the board to this rule · undo with U",
        Action::Theme(_) => "Switch the colours of the board and interface · saved automatically",
        Action::ApplyRule => "Use the rulestring above, written in B/S notation",
        Action::ExportRle => "Copy the live cells to the clipboard as RLE",
        Action::CloseDialog => "Close this dialog · Escape",
        _ => "",
    }
}

pub(super) fn navigate(
    event: On<Activate>,
    actions: Query<&Action>,
    view: Res<crate::canvas_view::CanvasView>,
    sim: Res<Simulation>,
    windows: Query<&Window, With<PrimaryWindow>>,
    layout: Res<UiLayout>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
) {
    let Ok(action) = actions.get(event.entity) else {
        return;
    };
    if let Ok((mut transform, mut projection)) = cameras.single_mut() {
        match action {
            Action::FitBoard => view.fit_camera(&mut transform, &mut projection),
            Action::FitLiveCells => {
                if let Ok(window) = windows.single() {
                    view.fit_live_cells(
                        &sim.app.board,
                        canvas_rect(window.size(), &layout).size(),
                        &mut transform,
                        &mut projection,
                    );
                }
            }
            Action::ZoomIn | Action::ZoomOut => {
                if let Projection::Orthographic(ortho) = &mut *projection {
                    let factor = if *action == Action::ZoomIn {
                        1.0 / 1.25
                    } else {
                        1.25
                    };
                    ortho.scale = (ortho.scale * factor).clamp(CAMERA_ZOOM_MIN, CAMERA_ZOOM_MAX);
                }
            }
            _ => {}
        }
    }
}

/// Maximum speed is a separate toggle; the field selects a fixed rate.
const MAX_FIXED_RATE: f32 = 40.0;

pub(super) fn rate_to_ms(rate: f32) -> u64 {
    (1000.0 / rate.clamp(1.0, MAX_FIXED_RATE)).round() as u64
}

pub(super) fn fixed_rate(app: &GolApp) -> f32 {
    1000.0 / app.tick_rate_ms() as f32
}

pub(super) fn sync_speed(
    sim: Res<Simulation>,
    mut labels: Query<(&mut Text, &mut Node), With<SpeedLabel>>,
) {
    let measured = sim.rate.per_second();
    let shown = if sim.app.is_running() && measured > 0.0 {
        format!("{measured:.0}/s")
    } else {
        "— gen/s".to_owned()
    };
    if let Ok((mut label, mut node)) = labels.single_mut() {
        set_if_neq!(label.0, shown);
        set_if_neq!(
            node.display,
            if sim.app.is_max_speed() {
                Display::Flex
            } else {
                Display::None
            }
        );
    }
}

pub(super) fn overlays(commands: &mut Commands) {
    commands
        .spawn((
            NativeRoot,
            CanvasToolHint,
            Node {
                position_type: PositionType::Absolute,
                padding: UiRect::axes(px(10), px(7)),
                border_radius: BorderRadius::all(px(6)),
                ..default()
            },
            ThemedBackground(Token::Panel),
            GlobalZIndex(1),
            Pickable::IGNORE,
            accessibility::named_node(accesskit::Role::GenericContainer, "Canvas editing guidance"),
        ))
        .with_children(|p| {
            p.spawn((
                CanvasToolHintText,
                label("", 12.0, Token::Muted),
                accessibility::named_node(accesskit::Role::Label, ""),
            ));
        });
    for hint in [false, true] {
        commands
            .spawn((
                NativeRoot,
                if hint {
                    Overlay::Hint {
                        target: None,
                        elapsed: 0.0,
                    }
                } else {
                    Overlay::Empty
                },
                Node {
                    display: Display::None,
                    position_type: PositionType::Absolute,
                    padding: UiRect::all(px(12)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(8)),
                    ..default()
                },
                ThemedBackground(Token::Panel),
                ThemedBorder(Token::Line),
                GlobalZIndex(if hint { 100 } else { 1 }),
                Pickable::IGNORE,
            ))
            .with_children(|p| {
                p.spawn((OverlayText(hint), label("", 13.0, Token::Muted)));
            });
    }
}

fn tool_hint(app: &GolApp) -> String {
    match app.tool() {
        EditTool::Draw => "Draw · drag to paint · right-drag to erase".into(),
        EditTool::Erase => "Erase · drag to erase cells".into(),
        EditTool::Stamp => {
            let Some(name) = app.selected_pattern_name() else {
                return "Stamp · choose a pattern from the palette · H shows the palette".into();
            };
            let (rotation, flipped) = app.stamp_orientation();
            format!(
                "Stamp: {name} · {}°{}\nClick to place · Z rotate · F flip",
                rotation as u32 * 90,
                if flipped { " · flipped" } else { "" }
            )
        }
    }
}

/// Passive guidance stays inside the canvas and lets pointer input pass through.
pub(super) fn sync_tool_hint(
    sim: Res<Simulation>,
    view: ViewState,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut nodes: Query<&mut Node, With<CanvasToolHint>>,
    mut texts: Query<&mut Text, With<CanvasToolHintText>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let canvas = canvas_rect(window.size(), &view.layout);
    let menu_open = view
        .menus
        .iter()
        .any(|state| *state != bevy::ui_widgets::MenuFocusState::Closed);
    for mut node in &mut nodes {
        set_if_neq!(
            node.display,
            if view.mode.dialog.is_none() && !menu_open {
                Display::Flex
            } else {
                Display::None
            }
        );
        set_if_neq!(node.left, px(canvas.min.x + 12.0));
        set_if_neq!(node.bottom, px(window.height() - canvas.max.y + 8.0));
        set_if_neq!(node.max_width, px((canvas.width() - 24.0).max(1.0)));
    }
    let hint = tool_hint(&sim.app);
    for mut text in &mut texts {
        set_if_neq!(text.0, hint.clone());
    }
}

type OverlayNode<'a> = (
    &'a mut Node,
    Option<&'a mut Overlay>,
    Has<WideLabel>,
    Option<&'a TallLabel>,
);

pub(super) fn sync_overlays(
    sim: Res<Simulation>,
    view: ViewState,
    windows: Query<&Window, With<PrimaryWindow>>,
    time: Res<Time>,
    mut hints: Query<(
        Entity,
        &Tooltip,
        &Hovered,
        &ComputedNode,
        &UiGlobalTransform,
        &mut AccessibilityNode,
    )>,
    mut nodes: Query<OverlayNode>,
    mut texts: Query<(&mut Text, &OverlayText)>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let mut hovered = None;
    let mut focused = None;
    for (entity, tooltip, hover, computed, transform, mut accessible) in &mut hints {
        if tooltip.0.is_empty() {
            continue;
        }
        accessible.set_description(tooltip.0.clone());
        // Layout works in physical pixels; the overlay is placed in logical ones.
        let scale = computed.inverse_scale_factor();
        let rect = Rect::from_center_size(transform.translation * scale, computed.size() * scale);
        if hover.get() {
            hovered = Some((entity, tooltip.0.clone(), rect));
        }
        // Clicking a control also focuses it, so only keyboard focus counts.
        if view.focus_visible.0 && view.focus.get() == Some(entity) {
            focused = Some((entity, tooltip.0.clone(), rect));
        }
    }
    let candidate = hovered.or(focused);
    let menu_open = view
        .menus
        .iter()
        .any(|state| *state != bevy::ui_widgets::MenuFocusState::Closed);
    let canvas = canvas_rect(window.size(), &view.layout);
    for (mut node, overlay, wide, tall) in &mut nodes {
        if wide {
            set_if_neq!(
                node.display,
                if window.width() < 850.0 {
                    Display::None
                } else {
                    Display::Flex
                }
            );
        }
        if let Some(tall) = tall {
            set_if_neq!(
                node.display,
                if window.height() < 560.0 && !tall.always_show {
                    Display::None
                } else {
                    Display::Flex
                }
            );
        }
        let Some(mut overlay) = overlay else {
            continue;
        };
        let (show, hint, text, left, top, bottom, width) = match &mut *overlay {
            Overlay::Hint { target, elapsed } => {
                let next = candidate.as_ref().map(|c| c.0);
                if *target != next {
                    *target = next;
                    *elapsed = 0.0;
                }
                *elapsed += time.delta_secs();
                let keyboard = view.focus_visible.0 && view.focus.get() == next;
                let show = candidate.is_some() && (*elapsed >= 0.45 || keyboard);
                let width = 280.0;
                let rect = candidate.as_ref().map_or(Rect::default(), |c| c.2);
                let left = rect.min.x.min(window.width() - width - 8.0).max(8.0);
                // Open away from the nearest window edge, anchored to the
                // control so the hint never depends on its own height.
                let (top, bottom) = if rect.center().y < window.height() / 2.0 {
                    (px(rect.max.y + 8.0), Val::Auto)
                } else {
                    (Val::Auto, px(window.height() - rect.min.y + 8.0))
                };
                (
                    show,
                    true,
                    candidate.as_ref().map(|c| c.1.clone()).unwrap_or_default(),
                    left,
                    top,
                    bottom,
                    width,
                )
            }
            Overlay::Empty => {
                let text = if sim.app.has_selected_pattern() {
                    "Place your first pattern\nClick the board to stamp. Z rotates · F flips."
                } else {
                    "An empty board is a fresh start\nChoose a pattern, draw cells, or press R to randomize.\nSpace starts the simulation · V fits the board."
                };
                let width = (canvas.width() - 24.0).clamp(120.0, 340.0);
                (
                    sim.app.board.live_cells_count() == 0,
                    false,
                    text.to_owned(),
                    canvas.center().x - width / 2.0,
                    px(canvas.min.y + 16.0),
                    Val::Auto,
                    width,
                )
            }
        };
        set_if_neq!(
            node.display,
            if show && view.mode.dialog.is_none() && (!hint || !menu_open) {
                Display::Flex
            } else {
                Display::None
            }
        );
        set_if_neq!(node.left, px(left));
        set_if_neq!(node.top, top);
        set_if_neq!(node.bottom, bottom);
        set_if_neq!(node.width, px(width));
        for (mut label, kind) in &mut texts {
            if kind.0 == hint && label.0 != text {
                label.0 = text.clone();
            }
        }
    }
}

type FooterNode<'a> = (
    &'a mut Node,
    Has<Footer>,
    Has<FooterStatus>,
    Has<Sidebar>,
    Option<&'a Readout>,
    Option<&'a Action>,
    Has<toolbar::ToolbarButton>,
);

pub(super) fn sync_footer(
    windows: Query<&Window, With<PrimaryWindow>>,
    view: Res<crate::canvas_view::CanvasView>,
    cameras: Query<&Projection, With<MainCamera>>,
    mut nodes: Query<FooterNode>,
    mut zoom: Query<&mut Text, With<ZoomLabel>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let narrow = window.width() < 1000.0;
    for (mut node, footer, status, sidebar, readout, action, toolbar_button) in &mut nodes {
        if footer {
            set_if_neq!(node.height, px(bottom_height(window.size())));
            set_if_neq!(
                node.flex_direction,
                if narrow {
                    FlexDirection::Column
                } else {
                    FlexDirection::Row
                }
            );
            set_if_neq!(
                node.align_items,
                if narrow {
                    AlignItems::Stretch
                } else {
                    AlignItems::Center
                }
            );
        }
        if status {
            set_if_neq!(node.flex_grow, if narrow { 0.0 } else { 1.0 });
        }
        if sidebar {
            set_if_neq!(node.bottom, px(bottom_height(window.size()) + 16.0));
        }
        if matches!(readout, Some(Readout::Board)) {
            set_if_neq!(
                node.display,
                if window.width() >= 1400.0 {
                    Display::Flex
                } else {
                    Display::None
                }
            );
        }
        if let Some(action) = action
            && !toolbar_button
            && !matches!(action, Action::Pattern(_) | Action::SpeedAdjust(_))
        {
            set_if_neq!(
                node.padding,
                UiRect::axes(px(if window.width() < 850.0 { 8.0 } else { 12.0 }), px(9))
            );
            if *action == Action::Play {
                set_if_neq!(
                    node.min_width,
                    px(if window.width() < 850.0 { 64.0 } else { 72.0 })
                );
            }
        }
    }
    if let Ok(Projection::Orthographic(ortho)) = cameras.single()
        && let Ok(mut text) = zoom.single_mut()
    {
        let value = format!("{:.0}%", view.fit_scale / ortho.scale * 100.0);
        if text.0 != value {
            text.0 = value;
        }
    }
}

pub(super) fn feedback_overlay(commands: &mut Commands) {
    commands
        .spawn((
            NativeRoot,
            FeedbackOverlay,
            Node {
                display: Display::None,
                position_type: PositionType::Absolute,
                padding: UiRect::all(px(12)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(8)),
                ..default()
            },
            ThemedBackground(Token::Panel),
            ThemedBorder(Token::Line),
            GlobalZIndex(20),
            Pickable::IGNORE,
        ))
        .with_children(|p| {
            p.spawn((
                FeedbackText,
                label("", 14.0, Token::Ink),
                accessibility::feedback_node(),
            ));
        });
}

pub(super) fn sync_feedback(
    time: Res<Time>,
    mut feedback: ResMut<EditFeedback>,
    view: ViewState,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut nodes: Query<&mut Node, With<FeedbackOverlay>>,
    mut texts: Query<&mut Text, With<FeedbackText>>,
) {
    feedback.remaining = (feedback.remaining - time.delta_secs()).max(0.0);
    let Ok(window) = windows.single() else {
        return;
    };
    let canvas = canvas_rect(window.size(), &view.layout);
    for mut node in &mut nodes {
        set_if_neq!(
            node.display,
            if feedback.remaining > 0.0 && view.mode.dialog.is_none() {
                Display::Flex
            } else {
                Display::None
            }
        );
        set_if_neq!(node.left, px(canvas.min.x + 12.0));
        set_if_neq!(node.bottom, px(window.height() - canvas.max.y + 12.0));
        set_if_neq!(node.max_width, px((canvas.width() - 24.0).max(0.0)));
    }
    for mut text in &mut texts {
        let value = if feedback.remaining > 0.0 {
            feedback.message
        } else {
            ""
        };
        if text.0 != value {
            text.0 = value.to_owned();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_button_restores_camera_center_and_zoom() {
        let mut world = World::new();
        let mut view = crate::canvas_view::CanvasView::default();
        view.fit_scale = 2.5;
        world.insert_resource(view);
        world.insert_resource(Simulation::new(GolApp::new(8, 8)));
        world.insert_resource(UiLayout::default());
        world.add_observer(navigate);
        let camera = world.spawn((Camera2d, crate::shared::MainCamera)).id();
        world.get_mut::<Transform>(camera).unwrap().translation = Vec3::new(90.0, -40.0, 10.0);
        let fit = world.spawn(Action::FitBoard).id();
        world.trigger(Activate { entity: fit });
        assert_eq!(
            world.get::<Transform>(camera).unwrap().translation,
            Vec3::new(0.0, 0.0, 10.0)
        );
        let Projection::Orthographic(projection) = world.get::<Projection>(camera).unwrap() else {
            panic!("expected an orthographic camera");
        };
        assert_eq!(projection.scale, 2.5);
    }
    #[test]
    fn fixed_rates_map_to_simulation_limits_and_monotonic_rates() {
        assert_eq!(rate_to_ms(1.0), GolApp::tick_rate_bounds().1);
        assert_eq!(rate_to_ms(40.0), GolApp::tick_rate_bounds().0);
        for rate in 1..40 {
            assert!(rate_to_ms(rate as f32) >= rate_to_ms(rate as f32 + 1.0));
        }
    }

    #[test]
    fn max_toggle_restores_rate_and_field_rates_leave_max() {
        let mut app = GolApp::new(8, 8);
        app.set_tick_rate_ms(rate_to_ms(10.0));
        apply_action(Action::MaxSpeed, &mut app);
        assert!(app.is_max_speed());
        assert_eq!(fixed_rate(&app), 10.0);
        assert!(!app.speed_up());
        assert_eq!(fixed_rate(&app), 10.0);
        apply_action(Action::MaxSpeed, &mut app);
        assert!(!app.is_max_speed());
        assert_eq!(app.tick_rate_ms(), 100);

        apply_action(Action::MaxSpeed, &mut app);
        app.set_tick_rate_ms(rate_to_ms(MAX_FIXED_RATE));
        assert!(!app.is_max_speed());
        assert_eq!(fixed_rate(&app), MAX_FIXED_RATE);

        // Keyboard shortcuts retain their existing Max behavior.
        app.speed_up();
        assert!(app.is_max_speed());
        assert_eq!(fixed_rate(&app), MAX_FIXED_RATE);
        app.set_tick_rate_ms(rate_to_ms(MAX_FIXED_RATE + 1.0));
        assert!(!app.is_max_speed());
    }

    #[test]
    fn canvas_guidance_follows_the_tool_and_actual_stamp_orientation() {
        let mut app = GolApp::new(8, 8);
        assert!(tool_hint(&app).starts_with("Draw ·"));
        app.select_pattern(crate::app::PatternKind::Glider);
        assert!(tool_hint(&app).contains("Stamp: Glider · 0°"));
        app.rotate_stamp_clockwise();
        app.toggle_stamp_flip();
        assert!(tool_hint(&app).contains("90° · flipped"));
        assert!(tool_hint(&app).contains("Z rotate · F flip"));
        app.set_tool(EditTool::Erase);
        assert_eq!(tool_hint(&app), "Erase · drag to erase cells");
        app.set_tool(EditTool::Draw);
        assert!(tool_hint(&app).starts_with("Draw ·"));
    }
}
