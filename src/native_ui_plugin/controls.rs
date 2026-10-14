//! Everyday controls, shortcut hints, and first-use guidance.
use super::*;
use crate::shared::{CAMERA_ZOOM_MAX, CAMERA_ZOOM_MIN, MainCamera, PATTERN_SHORTCUTS};
use bevy::{
    a11y::AccessibilityNode,
    text::{FontFeatureTag, Justify},
};

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
        Action::ZoomIn => "Zoom in · or scroll over the canvas · middle-drag to pan",
        Action::ZoomOut => "Zoom out · or scroll over the canvas · middle-drag to pan",
        Action::SpeedAdjust(step) if step < 0 => "Decrease speed by 1 gen/s · ∞ steps back to 40",
        Action::SpeedAdjust(_) => "Increase speed by 1 gen/s · past 40 is ∞, as fast as possible",
        Action::Sidebar => "Show / hide the palette · H",
        Action::Help => "Controls and tips · /",
        Action::Diagnostics => "Simulation diagnostics · F3",
        Action::Rotate => "Rotate stamp 90° clockwise · Z",
        Action::Flip => "Flip stamp horizontally · F",
        Action::Randomize => "Randomize using the density and seed in Settings · R",
        Action::Clear => "Clear the board · X · undo with U",
        Action::Boundary => "Wrap / fixed edges · M",
        Action::Tool(EditTool::Draw) => "Draw cells with the left mouse button",
        Action::Tool(EditTool::Erase) => "Erase cells · right-click also erases",
        Action::Tool(EditTool::Stamp) => "Place the selected pattern on the board",
        Action::Import => "Import RLE into your library, or copy the board as RLE",
        Action::Settings => "Board size, rule, randomization, theme, grid, cell age, and edges",
        Action::ApplyRandomize => "Use this density and optional seed for future randomizations",
        Action::RandomizeNow => "Apply the density and seed, then replace the board · undo with U",
        Action::FillCanvas => {
            "Size the board to fill the canvas, and follow it when the window or palette changes"
        }
        Action::RulePreset(_) => "Switch the board to this rule · undo with U",
        Action::Theme(_) => "Switch the colours of the board and interface · saved automatically",
        Action::ApplyRule => "Use the rulestring above, written in B/S notation",
        Action::EditRule => "The board's rule · click to change it in Settings",
        Action::ExportRle => "Copy the live cells to the clipboard as RLE",
        Action::SaveWorkspace => {
            "Choose a file for the board, generation, rule, edges, speed, and imported patterns"
        }
        Action::OpenWorkspace => "Choose a workspace file to replace this one · asks first",
        Action::ConfirmOpenWorkspace => "Replace this workspace with the chosen file · clears undo",
        Action::CancelOpenWorkspace => "Keep the current workspace · Escape also cancels",
        Action::CloseDialog => "Close this dialog · Escape",
        _ => "",
    }
}

pub(super) fn navigate(
    event: On<Activate>,
    actions: Query<&Action>,
    view: Res<crate::canvas_view::CanvasView>,
    mut cameras: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
) {
    let Ok(action) = actions.get(event.entity) else {
        return;
    };
    if let Ok((mut transform, mut projection)) = cameras.single_mut() {
        match action {
            Action::FitBoard => view.fit_camera(&mut transform, &mut projection),
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

/// The fastest fixed rate. One step further is ∞, maximum speed.
pub(super) const MAX_FIXED_RATE: f32 = 40.0;

/// Steps the rate by 1 gen/s, with ∞ as the stop after the fastest fixed rate.
pub(super) fn step_speed(app: &mut GolApp, step: i8) {
    if app.is_max_speed() {
        if step < 0 {
            app.set_tick_rate_ms(rate_to_ms(MAX_FIXED_RATE));
        }
        return;
    }
    let rate = (fixed_rate(app) * 10.0).round() / 10.0;
    if step > 0 && rate >= MAX_FIXED_RATE {
        app.set_max_speed();
    } else {
        app.set_tick_rate_ms(rate_to_ms(rate + f32::from(step)));
    }
}

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
/// The hint and edit notices share one slot: a notice replaces the hint while it
/// shows, rather than covering it.
pub(super) fn sync_tool_hint(
    sim: Res<Simulation>,
    feedback: Res<EditFeedback>,
    view: ViewState,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut nodes: Query<&mut Node, With<CanvasToolHint>>,
    mut texts: Query<&mut Text, With<CanvasToolHintText>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let canvas = canvas_rect(window.size(), &view.layout);
    for mut node in &mut nodes {
        set_if_neq!(
            node.display,
            if view.mode.dialog.is_none() && feedback.remaining <= 0.0 {
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
            if show && view.mode.dialog.is_none() {
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
    Has<Sidebar>,
    Option<&'a Readout>,
    Option<&'a Action>,
    Has<toolbar::ToolbarButton>,
);

/// Grows a readout to the widest it has been, so its neighbours stay put as
/// digits come and go, without reserving room for numbers it never shows.
/// Holds the width it starts from, and returns to when the board jumps.
#[derive(Component)]
pub(super) struct KeepWidest(pub Val);

pub(super) fn keep_widest(mut nodes: Query<(&mut Node, &ComputedNode), With<KeepWidest>>) {
    for (mut node, computed) in &mut nodes {
        let width = computed.size().x * computed.inverse_scale_factor();
        let widest = match node.min_width {
            Val::Px(widest) => widest,
            _ => 0.0,
        };
        if width > widest + 0.5 {
            node.min_width = px(width.ceil());
        }
    }
}

/// A word and its number, kept close together.
pub(super) fn readout_group() -> impl Bundle {
    Node {
        align_items: AlignItems::Center,
        column_gap: px(5),
        flex_shrink: 0.0,
        ..default()
    }
}

/// A status bar number with tabular digits. Counts are right-aligned so the
/// word after them stays put; trailing readouts are left-aligned.
pub(super) fn number(kind: Readout, size: f32, color: Token) -> impl Bundle {
    let justify = match kind {
        Readout::Change | Readout::Pointer => Justify::Left,
        _ => Justify::Right,
    };
    // Room for the largest board's far corner from the start, so the pointer
    // readout neither wraps nor pushes its neighbours as coordinates grow.
    let min_width = if kind == Readout::Pointer {
        px(96)
    } else {
        Val::Auto
    };
    (
        Text::default(),
        numeric_font(size),
        ThemedText(color),
        Pickable::IGNORE,
        // No `LineBreak::NoWrap`: it lays text out unbounded, which ignores
        // `justify`. Not shrinking is what keeps these on one line.
        TextLayout {
            justify,
            ..default()
        },
        Node {
            min_width,
            flex_shrink: 0.0,
            ..default()
        },
        kind,
        KeepWidest(min_width),
    )
}

/// The UI font has no arrows, so the change is signed: +12, −12, or ±0.
fn change_text(change: Option<isize>) -> String {
    match change {
        None => String::new(),
        Some(0) => "±0".to_owned(),
        Some(change) if change > 0 => format!("+{change}"),
        Some(change) => format!("−{}", change.unsigned_abs()),
    }
}

type StatusText<'a> = (
    &'a mut Text,
    &'a mut ThemedText,
    &'a Readout,
    &'a mut Node,
    Option<&'a KeepWidest>,
);

/// What the board looked like last frame, to tell a jump from a tick.
type BoardShape = (usize, (usize, usize), usize);

pub(super) fn sync_status(
    sim: Res<Simulation>,
    hovered: Res<crate::render_plugin::HoveredCell>,
    mut texts: Query<StatusText>,
    mut pills: Query<(&mut ThemedBackground, &mut ThemedBorder), With<StatusPill>>,
    mut last: Local<Option<BoardShape>>,
) {
    let running = sim.app.is_running();
    for (mut background, mut border) in &mut pills {
        background.set_if_neq(ThemedBackground(if running {
            Token::Accent
        } else {
            Token::Surface
        }));
        border.set_if_neq(ThemedBorder(if running {
            Token::Accent
        } else {
            Token::Line
        }));
    }
    let (cols, rows) = sim.app.board.size();
    let live = sim.app.board.live_cells_count();
    // A cleared or resized board, or a generation counter that went back,
    // makes the readouts start over at their own widths. Each then grows to
    // its current number again; ordinary ticks still never shrink them.
    let jumped = last.is_some_and(|(epoch, size, count)| {
        sim.app.epoch < epoch || size != (cols, rows) || (live == 0 && count != 0)
    });
    *last = Some((sim.app.epoch, (cols, rows), live));
    for (mut text, mut color, readout, mut node, keep) in &mut texts {
        if jumped && let Some(keep) = keep {
            set_if_neq!(node.min_width, keep.0);
        }
        let value = match readout {
            Readout::Status => {
                color.set_if_neq(ThemedText(if running {
                    Token::OnAccent
                } else {
                    Token::Muted
                }));
                if running { "Running" } else { "Paused" }.to_owned()
            }
            Readout::Generation => sim.app.epoch.to_string(),
            Readout::Population => live.to_string(),
            Readout::Change => change_text(sim.app.population_change()),
            // Counted from the top left, like RLE and other Life tools.
            Readout::Pointer => hovered
                .0
                .map(|(col, row)| format!("({col}, {})", rows - 1 - row))
                .unwrap_or_default(),
            Readout::Board => format!(
                "{cols} × {rows} cells   ·   {} edges",
                sim.app.boundary_mode_label()
            ),
        };
        if text.0 != value {
            text.0 = value;
        }
    }
}

/// − 100% + in one box, like the speed field: the steps share its border and
/// background, and show a border of their own only for keyboard focus.
pub(super) fn zoom_stepper(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            Node {
                align_items: AlignItems::Center,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(7)),
                flex_shrink: 0.0,
                ..default()
            },
            ThemedBackground(Token::Field),
            ThemedBorder(Token::Line),
            accessibility::named_node(accesskit::Role::Group, "Zoom"),
        ))
        .with_children(|p| {
            for (name, text, action) in [
                ("Zoom out", "−", Action::ZoomOut),
                ("Zoom in", "+", Action::ZoomIn),
            ] {
                if action == Action::ZoomIn {
                    p.spawn((
                        ZoomLabel,
                        Text::new("100%"),
                        numeric_font(13.0),
                        ThemedText(Token::Ink),
                        TextLayout {
                            justify: Justify::Center,
                            ..default()
                        },
                        Tooltip("Zoom relative to Fit board (100%) · V fits the board".into()),
                        Hovered::default(),
                        Node {
                            min_width: px(48),
                            flex_shrink: 0.0,
                            ..default()
                        },
                    ));
                }
                let step = button(p, text, action);
                p.commands().entity(step).insert((
                    Name::new(name),
                    Node {
                        padding: UiRect::axes(px(10), px(9)),
                        border_radius: if action == Action::ZoomOut {
                            BorderRadius::left(px(6))
                        } else {
                            BorderRadius::right(px(6))
                        },
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::Center,
                        flex_shrink: 0.0,
                        ..default()
                    },
                ));
            }
        });
}

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
    for (mut node, sidebar, readout, action, toolbar_button) in &mut nodes {
        if sidebar {
            set_if_neq!(node.bottom, px(STATUS_BAR_HEIGHT + 16.0));
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
            && !matches!(
                action,
                Action::Pattern(_) | Action::SpeedAdjust(_) | Action::ZoomIn | Action::ZoomOut
            )
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
        // The tool hint's slot, which it gives up while the notice shows.
        set_if_neq!(node.left, px(canvas.min.x + 12.0));
        set_if_neq!(node.bottom, px(window.height() - canvas.max.y + 8.0));
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
    fn readout_widths_start_over_when_the_board_jumps_but_not_when_it_ticks() {
        let mut world = World::new();
        let mut app = GolApp::new(16, 16);
        app.randomize_board();
        world.insert_resource(Simulation::new(app));
        world.init_resource::<crate::render_plugin::HoveredCell>();
        let spawn = |world: &mut World, kind, base| {
            world
                .spawn((
                    Text::default(),
                    ThemedText(Token::Ink),
                    kind,
                    Node {
                        min_width: px(120),
                        ..default()
                    },
                    KeepWidest(base),
                ))
                .id()
        };
        let generation = spawn(&mut world, Readout::Generation, Val::Auto);
        let pointer = spawn(&mut world, Readout::Pointer, px(96));
        // A registered system remembers last frame, unlike `run_system_once`.
        let system = world.register_system(sync_status);
        let sync = |world: &mut World| {
            world.run_system(system).unwrap();
            let width = |entity| world.get::<Node>(entity).unwrap().min_width;
            (width(generation), width(pointer))
        };
        let grown = (px(120), px(120));
        assert_eq!(sync(&mut world), grown);
        world.resource_mut::<Simulation>().app.increment_epoch();
        assert_eq!(sync(&mut world), grown, "a generation is a tick");
        world.resource_mut::<Simulation>().app.clear_board();
        assert_eq!(sync(&mut world), (Val::Auto, px(96)));

        // Growing again, then resetting the counter, starts over too.
        for entity in [generation, pointer] {
            world.get_mut::<Node>(entity).unwrap().min_width = px(120);
        }
        assert_eq!(sync(&mut world), grown);
        world.resource_mut::<Simulation>().app.reset_epoch();
        assert_eq!(sync(&mut world), (Val::Auto, px(96)));
    }

    #[test]
    fn an_edit_notice_takes_the_tool_hint_slot_until_it_fades() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.insert_resource(Simulation::new(GolApp::new(8, 8)));
        world.init_resource::<EditFeedback>();
        world.init_resource::<Time>();
        world.init_resource::<NativeUi>();
        world.init_resource::<UiLayout>();
        world.insert_resource(GridOverlay { enabled: false });
        world.init_resource::<CellAge>();
        world.init_resource::<crate::shared::BoardSettings>();
        world.init_resource::<InputFocus>();
        world.init_resource::<InputFocusVisible>();
        world.init_resource::<Theme>();
        world.spawn((Window::default(), PrimaryWindow));
        let hint = world.spawn((CanvasToolHint, Node::default())).id();
        let notice = world.spawn((FeedbackOverlay, Node::default())).id();
        let shown = |world: &mut World| {
            world.run_system_once(sync_feedback).unwrap();
            world.run_system_once(sync_tool_hint).unwrap();
            let display = |entity| world.get::<Node>(entity).unwrap().display;
            (
                display(hint) != Display::None,
                display(notice) != Display::None,
            )
        };
        assert_eq!(shown(&mut world), (true, false));
        world
            .resource_mut::<EditFeedback>()
            .show("Board randomized · U to undo");
        assert_eq!(shown(&mut world), (false, true));
        // Both sit in the same place, so the notice reads as replacing the hint.
        assert_eq!(
            world.get::<Node>(hint).unwrap().bottom,
            world.get::<Node>(notice).unwrap().bottom
        );
        world.resource_mut::<EditFeedback>().remaining = 0.0;
        assert_eq!(shown(&mut world), (true, false));
    }

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
    fn population_change_reads_as_a_signed_count_and_readouts_only_grow() {
        assert_eq!(change_text(None), "");
        assert_eq!(change_text(Some(0)), "±0");
        assert_eq!(change_text(Some(12)), "+12");
        assert_eq!(change_text(Some(-7)), "−7");

        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        let readout = world
            .spawn((
                Node::default(),
                ComputedNode::default(),
                KeepWidest(Val::Auto),
            ))
            .id();
        let measure = |world: &mut World, width: f32| {
            world.get_mut::<ComputedNode>(readout).unwrap().size.x = width;
            world.run_system_once(keep_widest).unwrap();
            world.get::<Node>(readout).unwrap().min_width
        };
        assert_eq!(measure(&mut world, 30.2), px(31));
        // Fewer digits keep the widest size, so neighbours do not move back.
        assert_eq!(measure(&mut world, 20.0), px(31));
        assert_eq!(measure(&mut world, 40.0), px(40));
    }

    #[test]
    fn infinity_is_the_stop_after_the_fastest_rate_for_chevrons_and_keys_alike() {
        let mut app = GolApp::new(8, 8);
        app.set_tick_rate_ms(rate_to_ms(MAX_FIXED_RATE));
        step_speed(&mut app, 1);
        assert!(app.is_max_speed());
        // Nothing lies past ∞, and stepping down returns to the fastest rate.
        step_speed(&mut app, 1);
        assert!(app.is_max_speed());
        step_speed(&mut app, -1);
        assert!(!app.is_max_speed());
        assert_eq!(fixed_rate(&app), MAX_FIXED_RATE);

        // The + and - keys take the same path through ∞.
        app.speed_up();
        assert!(app.is_max_speed());
        app.slow_down();
        assert_eq!(fixed_rate(&app), MAX_FIXED_RATE);
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
