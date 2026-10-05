//! Everyday controls, shortcut hints, and first-use guidance.
use super::*;
use bevy::{
    a11y::AccessibilityNode,
    ui_widgets::{
        Slider, SliderPrecision, SliderRange, SliderStep, SliderThumb, SliderValue, TrackClick,
        ValueChange,
    },
};

#[derive(Component)]
pub(super) struct Tooltip(pub String);
#[derive(Component)]
pub(super) struct WideLabel;
#[derive(Component)]
pub(super) struct SpeedSlider;
#[derive(Component)]
pub(super) struct SpeedThumb;
#[derive(Component)]
pub(super) struct SpeedLabel;
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

pub(super) fn hint(action: Action) -> &'static str {
    match action {
        Action::Play => "Play / pause · Space",
        Action::Step => "One generation · J",
        Action::Undo => "Undo the last edit · U",
        Action::Redo => {
            if cfg!(target_os = "macos") {
                "Redo the last edit · Cmd+Shift+Z"
            } else {
                "Redo the last edit · Ctrl+Shift+Z"
            }
        }
        Action::Grid => "Show / hide the grid · T",
        Action::Sidebar => "Show / hide the palette · H",
        Action::Help => "Controls and tips · /",
        Action::Diagnostics => "Simulation diagnostics · F3",
        Action::Rotate => "Rotate stamp 90° clockwise · Z",
        Action::Flip => "Flip stamp horizontally · F",
        Action::Randomize => "Fill the board with random cells · R",
        Action::Clear => "Clear the board · X · undo with U",
        Action::Boundary => "Wrap / fixed edges · M",
        Action::Pattern(0) => "Select Glider · G · click the board to place",
        Action::Pattern(1) => "Select Blinker · B · click the board to place",
        Action::Pattern(2) => "Select LWSS · L · click the board to place",
        Action::Pattern(3) => "Select RLE 8x4 · P · click the board to place",
        Action::Pattern(_) => "Select pattern · click the board to place",
        Action::Tool(EditTool::Draw) => "Draw cells with the left mouse button",
        Action::Tool(EditTool::Erase) => "Erase cells · right-click also erases",
        Action::Tool(EditTool::Stamp) => "Place the selected pattern on the board",
        Action::Import => "Import a named RLE pattern into your library",
        Action::Settings => "Board dimensions and workspace layout",
        Action::CloseDialog => "Close this dialog · Escape",
        _ => "",
    }
}

/// The slider runs through fixed rates in generations per second, then one
/// notch past the fastest of them for max speed.
const SLIDER_FASTEST_RATE: f32 = 40.0;
const SLIDER_MAX_SPEED: f32 = SLIDER_FASTEST_RATE + 1.0;

fn rate_to_ms(rate: f32) -> u64 {
    (1000.0 / rate.clamp(1.0, SLIDER_FASTEST_RATE)).round() as u64
}

fn apply_slider_value(app: &mut GolApp, value: f32) {
    if value > SLIDER_FASTEST_RATE {
        app.set_max_speed();
    } else {
        app.set_tick_rate_ms(rate_to_ms(value));
    }
}

fn slider_value(app: &GolApp) -> f32 {
    if app.is_max_speed() {
        SLIDER_MAX_SPEED
    } else {
        1000.0 / app.tick_rate_ms() as f32
    }
}

pub(super) fn speed_slider(p: &mut ChildSpawnerCommands) {
    p.spawn((
        SpeedSlider,
        Slider {
            track_click: TrackClick::Snap,
            ..default()
        },
        SliderValue(SLIDER_FASTEST_RATE),
        SliderRange::new(1.0, SLIDER_MAX_SPEED),
        SliderStep(1.0),
        SliderPrecision(0),
        TabIndex(0),
        Hovered::default(),
        Tooltip("Speed · Left/Right adjusts · Home slowest · End max speed · +/− on canvas".into()),
        accessibility::named_node(
            accesskit::Role::Slider,
            "Simulation speed (generations per second)",
        ),
        Node {
            height: px(34),
            width: px(150),
            min_width: px(64),
            max_width: px(200),
            flex_grow: 1.0,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            align_items: AlignItems::Center,
            ..default()
        },
        BorderColor::all(LINE),
        BackgroundColor(PANEL),
    ))
    .with_children(|p| {
        p.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: px(8),
                right: px(8),
                height: px(4),
                border_radius: BorderRadius::all(px(2)),
                ..default()
            },
            BackgroundColor(LINE),
            Pickable::IGNORE,
        ));
        p.spawn((
            SpeedThumb,
            SliderThumb,
            Node {
                position_type: PositionType::Absolute,
                width: px(16),
                height: px(22),
                border_radius: BorderRadius::all(px(5)),
                ..default()
            },
            BackgroundColor(ACCENT),
            Pickable::IGNORE,
        ));
    });
    p.spawn(Node {
        width: px(84),
        height: px(34),
        align_items: AlignItems::Center,
        flex_shrink: 0.0,
        ..default()
    })
    .with_children(|p| {
        p.spawn((SpeedLabel, label("40.0 gen/s", 13.0, INK)));
    });
}

pub(super) fn speed_changed(
    event: On<ValueChange<f32>>,
    sliders: Query<(), With<SpeedSlider>>,
    mut sim: ResMut<Simulation>,
    mode: Res<NativeUi>,
) {
    if mode.dialog.is_none() && sliders.contains(event.source) && event.value.is_finite() {
        apply_slider_value(&mut sim.app, event.value);
    }
}

pub(super) fn sync_speed(
    mut commands: Commands,
    sim: Res<Simulation>,
    focus: Res<InputFocus>,
    mut sliders: Query<
        (
            Entity,
            &ComputedNode,
            &SliderValue,
            &mut AccessibilityNode,
            &mut BorderColor,
        ),
        With<SpeedSlider>,
    >,
    mut thumbs: Query<&mut Node, With<SpeedThumb>>,
    mut labels: Query<&mut Text, With<SpeedLabel>>,
) {
    let position = slider_value(&sim.app);
    let Ok((entity, computed, value, mut accessible, mut border)) = sliders.single_mut() else {
        return;
    };
    if value.0 != position {
        commands.entity(entity).insert(SliderValue(position));
    }
    let measured = sim.rate.per_second();
    let (spoken, shown) = if sim.app.is_max_speed() && sim.app.is_running() && measured > 0.0 {
        (
            format!("Maximum speed, {measured:.0} generations per second"),
            format!("Max {measured:.0}/s"),
        )
    } else if sim.app.is_max_speed() {
        // Paused, or running for less than one measuring window.
        ("Maximum speed".to_owned(), "Max".to_owned())
    } else {
        (
            format!("{position:.1} generations per second"),
            format!("{position:.1} gen/s"),
        )
    };
    accessible.set_numeric_value(position as f64);
    accessible.set_value(spoken);
    accessible.add_action(accesskit::Action::SetValue);
    accessible.add_action(accesskit::Action::Increment);
    accessible.add_action(accesskit::Action::Decrement);
    *border = BorderColor::all(if focus.get() == Some(entity) {
        INK
    } else {
        LINE
    });
    if let Ok(mut thumb) = thumbs.single_mut() {
        let travel = (computed.size().x * computed.inverse_scale_factor - 16.0).max(0.0);
        thumb.left = px(travel * (position - 1.0) / (SLIDER_MAX_SPEED - 1.0));
    }
    if let Ok(mut label) = labels.single_mut()
        && label.0 != shown
    {
        label.0 = shown;
    }
}

pub(super) fn overlays(commands: &mut Commands) {
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
                BackgroundColor(PANEL),
                BorderColor::all(LINE),
                GlobalZIndex(if hint { 100 } else { 1 }),
                Pickable::IGNORE,
            ))
            .with_children(|p| {
                p.spawn((OverlayText(hint), label("", 13.0, MUTED)));
            });
    }
}

pub(super) fn sync_overlays(
    sim: Res<Simulation>,
    view: ViewState,
    windows: Query<&Window, With<PrimaryWindow>>,
    time: Res<Time>,
    mut hints: Query<(Entity, &Tooltip, &Hovered, &mut AccessibilityNode)>,
    mut nodes: Query<(&mut Node, Option<&mut Overlay>, Has<WideLabel>)>,
    mut texts: Query<(&mut Text, &OverlayText)>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let mut hovered = None;
    let mut focused = None;
    for (entity, tooltip, hover, mut accessible) in &mut hints {
        if tooltip.0.is_empty() {
            continue;
        }
        accessible.set_description(tooltip.0.clone());
        if hover.get() {
            hovered = Some((entity, tooltip.0.clone()));
        }
        if view.focus.get() == Some(entity) {
            focused = Some((entity, tooltip.0.clone()));
        }
    }
    let candidate = hovered.or(focused);
    let canvas = canvas_rect(window.size(), &view.layout);
    for (mut node, overlay, wide) in &mut nodes {
        if wide {
            node.display = if window.width() < 850.0 {
                Display::None
            } else {
                Display::Flex
            };
        }
        let Some(mut overlay) = overlay else {
            continue;
        };
        let (show, hint, text, position, width) = match &mut *overlay {
            Overlay::Hint { target, elapsed } => {
                let next = candidate.as_ref().map(|c| c.0);
                if *target != next {
                    *target = next;
                    *elapsed = 0.0;
                }
                *elapsed += time.delta_secs();
                let show = candidate.is_some() && (*elapsed >= 0.45 || view.focus.get() == next);
                let cursor = window
                    .cursor_position()
                    .unwrap_or(Vec2::new(canvas.min.x, window.height() - BOTTOM - 60.0));
                let pos = (cursor + Vec2::splat(18.0))
                    .min(window.size() - Vec2::new(300.0, 90.0))
                    .max(Vec2::splat(8.0));
                (
                    show,
                    true,
                    candidate.as_ref().map(|c| c.1.clone()).unwrap_or_default(),
                    pos,
                    280.0,
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
                    Vec2::new(canvas.center().x - width / 2.0, canvas.min.y + 16.0),
                    width,
                )
            }
        };
        node.display = if show && view.mode.dialog.is_none() {
            Display::Flex
        } else {
            Display::None
        };
        node.left = px(position.x);
        node.top = px(position.y);
        node.width = px(width);
        for (mut label, kind) in &mut texts {
            if kind.0 == hint && label.0 != text {
                label.0 = text.clone();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn speed_slider_maps_to_simulation_limits_and_monotonic_rates() {
        assert_eq!(rate_to_ms(1.0), GolApp::tick_rate_bounds().1);
        assert_eq!(rate_to_ms(40.0), GolApp::tick_rate_bounds().0);
        for rate in 1..40 {
            assert!(rate_to_ms(rate as f32) >= rate_to_ms(rate as f32 + 1.0));
        }
    }

    #[test]
    fn the_notch_past_the_fastest_rate_is_max_speed() {
        let mut app = GolApp::new(8, 8);
        apply_slider_value(&mut app, SLIDER_MAX_SPEED);
        assert!(app.is_max_speed());
        assert_eq!(slider_value(&app), SLIDER_MAX_SPEED);

        apply_slider_value(&mut app, SLIDER_FASTEST_RATE);
        assert!(!app.is_max_speed());
        assert_eq!(slider_value(&app), SLIDER_FASTEST_RATE);

        // The + key past the fastest rate lands on the same notch.
        app.speed_up();
        assert_eq!(slider_value(&app), SLIDER_MAX_SPEED);
    }
}
