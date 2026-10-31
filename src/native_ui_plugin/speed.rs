//! The speed: 1–40 gen/s with Feathers-style scrubbing, direct entry, step
//! buttons, and a fill showing where the speed sits in that range, plus a
//! toggle at the end for ∞.
use super::*;
use bevy::{
    input::{ButtonState, keyboard::KeyboardInput},
    input_focus::{AcquireFocus, FocusCause, FocusedInput},
    picking::{
        cursor::EntityCursor,
        events::{PointerCancel, PointerDrag, PointerPress, PointerRelease},
        pointer::PointerButton,
    },
    text::{EditableText, Justify, TextEdit},
    ui_widgets::{SelectAllOnFocus, TextInput},
    window::SystemCursorIcon,
};

const INPUT_HINT: &str = "Speed · Drag to adjust · Shift for finer control · Click to type · 1–40 gen/s, or ∞ for as fast as possible";
const INVALID_HINT: &str = "Enter a speed from 1 to 40 gen/s, or ∞ (or max).";
const DRAG_THRESHOLD: f32 = 3.0;
/// Wide, so the fill reads as a scale and a drag has room; the toolbar's
/// playback budget is sized around it.
const FIELD_WIDTH: f32 = 178.0;
const STEP_WIDTH: f32 = 20.0;
/// The ∞ toggle at the field's right end, where the fill stops.
const MAX_WIDTH: f32 = 30.0;
/// The inner corners of the field's 1 px border.
const INNER_RADIUS: f32 = 5.0;
const UNIT: &str = "gen/s";
const RATE_PER_PIXEL: f32 = 0.2;

#[derive(Component)]
pub(super) struct SpeedScrubber(Entity);

#[derive(Component)]
pub(super) struct SpeedChevron(Entity);

/// The bar behind the field's text, as wide as the speed is far along its range.
#[derive(Component)]
pub(super) struct SpeedFill;

/// The whole control, whose hover shows the steps.
#[derive(Component)]
pub(super) struct SpeedControl;

struct Scrub {
    interval: u64,
    max: bool,
    origin: Vec2,
    offset: f32,
    moved: bool,
    cancelled: bool,
}

#[derive(Clone, Copy)]
enum Request {
    Apply,
    Cancel,
}

#[derive(Component, Default)]
pub(super) struct SpeedEditor {
    scrub: Option<Scrub>,
    request: Option<Request>,
    focused: bool,
    initial: String,
    draft: String,
    last_interval: Option<u64>,
    last_max: bool,
    invalid: bool,
}

pub(super) fn spawn(parent: &mut ChildSpawnerCommands) {
    parent
        .spawn((
            SpeedControl,
            Hovered::default(),
            Node {
                width: px(FIELD_WIDTH),
                height: px(34),
                flex_shrink: 0.0,
                border_radius: BorderRadius::all(px(6)),
                ..default()
            },
            // The field itself is clear, so the fill shows behind its text.
            ThemedBackground(Token::Field),
        ))
        .with_children(|p| {
            spawn_step_button(p, -1);
            // Spans the field inside its border, under the steps, up to the
            // ∞ toggle, so 40 gen/s fills it.
            p.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: px(1),
                    right: px(1.0 + MAX_WIDTH),
                    top: px(1),
                    bottom: px(1),
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|p| {
                p.spawn((
                    SpeedFill,
                    Node {
                        width: percent(0),
                        height: percent(100),
                        border_radius: BorderRadius::left(px(INNER_RADIUS)),
                        ..default()
                    },
                    ThemedBackground(Token::Selected),
                    Pickable::IGNORE,
                ));
            });
            let field = p
                .spawn((
                    SpeedEditor::default(),
                    EditableText::new(format!("40 {UNIT}")),
                    TextInput,
                    SelectAllOnFocus,
                    TabIndex(0),
                    Hovered::default(),
                    EntityCursor::System(SystemCursorIcon::ColResize),
                    controls::Tooltip(INPUT_HINT.into()),
                    accessibility::named_node(
                        accesskit::Role::TextInput,
                        "Speed (generations per second)",
                    ),
                    Node {
                        width: px(FIELD_WIDTH),
                        height: px(34),
                        padding: UiRect {
                            left: px(STEP_WIDTH),
                            right: px(STEP_WIDTH + MAX_WIDTH),
                            top: px(7),
                            bottom: px(7),
                        },
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(px(6)),
                        overflow: Overflow::clip(),
                        flex_shrink: 0.0,
                        ..default()
                    },
                    (
                        controls::numeric_font(13.0),
                        TextLayout {
                            justify: Justify::Center,
                            ..default()
                        },
                    ),
                    ThemedText(Token::Ink),
                    theme::ThemedCursor(Token::Accent),
                    ThemedBorder(Token::Line),
                ))
                .id();
            // Intercept idle presses before TextInput can begin selecting text.
            // Focused fields ignore this layer and retain normal text selection.
            p.spawn((
                SpeedScrubber(field),
                Pickable::default(),
                ZIndex(1),
                EntityCursor::System(SystemCursorIcon::ColResize),
                Hovered::default(),
                controls::Tooltip(INPUT_HINT.into()),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(1.0 + STEP_WIDTH),
                    right: px(1.0 + MAX_WIDTH + STEP_WIDTH),
                    top: px(0),
                    bottom: px(0),
                    ..default()
                },
            ))
            .observe(scrub_focus)
            .observe(scrub_press)
            .observe(scrub_drag)
            .observe(scrub_release)
            .observe(scrub_cancel);
            spawn_step_button(p, 1);
            spawn_max_button(p);
        });
}

fn spawn_max_button(parent: &mut ChildSpawnerCommands) {
    let action = Action::MaxSpeed;
    parent
        .spawn((
            WidgetButton,
            action,
            Hovered::default(),
            TabIndex(0),
            ZIndex(2),
            EntityCursor::System(SystemCursorIcon::Pointer),
            controls::Tooltip(controls::hint(action).into()),
            accessibility::named_node(accesskit::Role::Button, "Maximum speed"),
            Name::new("Maximum speed"),
            ThemedBackground(Token::Clear),
            ThemedBorder(Token::Line),
            Node {
                position_type: PositionType::Absolute,
                right: px(1),
                top: px(1),
                width: px(MAX_WIDTH),
                height: px(32),
                border: max_border(false),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border_radius: BorderRadius::right(px(INNER_RADIUS)),
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn((
                Text::new("∞"),
                controls::numeric_font(15.0),
                ThemedText(Token::Ink),
                Pickable::IGNORE,
            ));
        });
}

/// The ∞ toggle always shows its left border, dividing it from the fixed
/// rates, and a full one only for keyboard focus.
fn max_border(focused: bool) -> UiRect {
    if focused {
        UiRect::all(px(1))
    } else {
        UiRect::left(px(1))
    }
}

fn spawn_step_button(parent: &mut ChildSpawnerCommands, step: i8) {
    let action = Action::SpeedAdjust(step);
    let name = if step < 0 {
        "Decrease speed"
    } else {
        "Increase speed"
    };
    let button = parent
        .spawn((
            WidgetButton,
            action,
            Hovered::default(),
            TabIndex(0),
            ZIndex(2),
            EntityCursor::System(SystemCursorIcon::Pointer),
            controls::Tooltip(controls::hint(action).into()),
            accessibility::named_node(accesskit::Role::Button, name),
            Name::new(name),
            ThemedBackground(Token::Field),
            ThemedBorder(Token::Line),
            Node {
                position_type: PositionType::Absolute,
                left: if step < 0 { px(1) } else { Val::Auto },
                // Increasing sits just left of the ∞ toggle.
                right: if step > 0 {
                    px(1.0 + MAX_WIDTH)
                } else {
                    Val::Auto
                },
                top: px(1),
                width: px(STEP_WIDTH),
                height: px(32),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border_radius: if step < 0 {
                    BorderRadius::left(px(INNER_RADIUS))
                } else {
                    BorderRadius::ZERO
                },
                ..default()
            },
        ))
        .id();
    parent.commands().entity(button).with_children(|p| {
        p.spawn((
            Node {
                width: px(6),
                height: px(10),
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|p| {
            for (top, rotation) in [(2.5, -45.0), (6.5, 45.0)] {
                p.spawn((
                    SpeedChevron(button),
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(0),
                        top: px(top),
                        width: px(6),
                        height: px(1.5),
                        border_radius: BorderRadius::all(px(1)),
                        ..default()
                    },
                    UiTransform::from_rotation(Rot2::degrees(rotation * -f32::from(step))),
                    ThemedBackground(Token::Ink),
                    Pickable::IGNORE,
                ));
            }
        });
    });
}

/// Like FeathersNumberInput, the steps appear only while the control is
/// hovered; they also stay while one has keyboard focus, so focus is never invisible.
pub(super) fn sync_chevrons(
    focus: Res<InputFocus>,
    visible: Res<InputFocusVisible>,
    controls: Query<&Hovered, With<SpeedControl>>,
    mut buttons: Query<(Entity, &Action, Has<InteractionDisabled>, &mut Node)>,
    mut bars: Query<(&SpeedChevron, &mut ThemedBackground, &mut Visibility)>,
) {
    let step_focused = focus.get().is_some_and(|entity| {
        buttons
            .get(entity)
            .is_ok_and(|(_, action, _, _)| matches!(action, Action::SpeedAdjust(_)))
    });
    let shown = step_focused || controls.iter().any(|hovered| hovered.get());
    for (entity, action, _, mut node) in &mut buttons {
        let keyboard_focused = visible.0 && focus.get() == Some(entity);
        if *action == Action::MaxSpeed {
            set_if_neq!(node.border, max_border(keyboard_focused));
        }
        // Steps inside a shared box show a border only for keyboard focus.
        if matches!(
            action,
            Action::SpeedAdjust(_) | Action::ZoomIn | Action::ZoomOut
        ) {
            set_if_neq!(
                node.border,
                UiRect::all(px(if keyboard_focused { 1.0 } else { 0.0 }))
            );
        }
    }
    for (chevron, mut color, mut visibility) in &mut bars {
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if let Ok((_, _, disabled, _)) = buttons.get(chevron.0) {
            color.set_if_neq(ThemedBackground(if disabled {
                Token::Muted
            } else {
                Token::Ink
            }));
        }
    }
}

fn scrub_focus(mut event: On<AcquireFocus>) {
    // Like FeathersNumberInput, a click enters editing on release, not press.
    event.propagate(false);
}

fn scrub_press(
    mut event: On<PointerPress>,
    parents: Query<&SpeedScrubber>,
    mut fields: Query<&mut SpeedEditor>,
    sim: Res<Simulation>,
    mut capture: ResMut<crate::ui_input::UiInputCapture>,
) {
    event.propagate(false);
    if event.button != PointerButton::Primary {
        return;
    }
    let Ok(parent) = parents.get(event.event_target()) else {
        return;
    };
    if let Ok(mut state) = fields.get_mut(parent.0) {
        state.scrub = Some(Scrub {
            interval: sim.app.tick_rate_ms(),
            max: sim.app.is_max_speed(),
            origin: event.pointer.position,
            offset: 0.0,
            moved: false,
            cancelled: false,
        });
        capture.block_modal_frame();
    }
}

fn scrub_drag(
    mut event: On<PointerDrag>,
    parents: Query<&SpeedScrubber>,
    mut fields: Query<&mut SpeedEditor>,
    keys: Res<ButtonInput<KeyCode>>,
    scale: Res<bevy::ui::UiScale>,
    mut sim: ResMut<Simulation>,
    mut capture: ResMut<crate::ui_input::UiInputCapture>,
) {
    event.propagate(false);
    if event.button != PointerButton::Primary {
        return;
    }
    let Ok(parent) = parents.get(event.event_target()) else {
        return;
    };
    let Ok(mut state) = fields.get_mut(parent.0) else {
        return;
    };
    let Some(scrub) = &mut state.scrub else {
        return;
    };
    capture.block_modal_frame();
    if scrub.cancelled {
        return;
    }
    let fine = keys.any_pressed([KeyCode::ShiftLeft, KeyCode::ShiftRight]);
    // Pointer positions are logical window pixels, so Retina scaling must not
    // alter sensitivity. Accumulate deltas to allow Shift to change mid-drag.
    scrub.offset += event.delta.x / scale.0 * RATE_PER_PIXEL * if fine { 0.1 } else { 1.0 };
    scrub.moved |= ((event.pointer.position.x - scrub.origin.x) / scale.0).abs() >= DRAG_THRESHOLD;
    if scrub.moved {
        // ∞ fills the field, so a drag from it starts at the fastest rate;
        // dragging left leaves it, and right stays. A drag never reaches ∞,
        // which is the toggle's.
        let base = if scrub.max {
            controls::MAX_FIXED_RATE
        } else {
            1000.0 / scrub.interval as f32
        };
        let rate = ((base + scrub.offset) * 10.0).round() / 10.0;
        if scrub.max && rate >= controls::MAX_FIXED_RATE {
            sim.app.set_max_speed();
        } else {
            sim.app.set_tick_rate_ms(controls::rate_to_ms(rate));
        }
    }
}

fn scrub_release(
    mut event: On<PointerRelease>,
    parents: Query<&SpeedScrubber>,
    mut fields: Query<(&mut SpeedEditor, &mut EditableText)>,
    mut focus: ResMut<InputFocus>,
    mut capture: ResMut<crate::ui_input::UiInputCapture>,
) {
    event.propagate(false);
    if event.button != PointerButton::Primary {
        return;
    }
    let Ok(parent) = parents.get(event.event_target()) else {
        return;
    };
    let entity = parent.0;
    if let Ok((mut state, mut input)) = fields.get_mut(entity)
        && let Some(scrub) = state.scrub.take()
    {
        capture.block_modal_frame();
        if !scrub.moved && !scrub.cancelled {
            focus.set(entity, FocusCause::Pressed);
            input.queue_edit(TextEdit::SelectAll);
        }
    }
}

fn restore_scrub(scrub: &mut Scrub, app: &mut GolApp) {
    if !scrub.cancelled {
        app.set_tick_rate_ms(scrub.interval);
        if scrub.max {
            app.set_max_speed();
        }
        scrub.cancelled = true;
    }
}

fn scrub_cancel(
    mut event: On<PointerCancel>,
    parents: Query<&SpeedScrubber>,
    mut fields: Query<&mut SpeedEditor>,
    mut sim: ResMut<Simulation>,
    mut capture: ResMut<crate::ui_input::UiInputCapture>,
) {
    event.propagate(false);
    let Ok(parent) = parents.get(event.event_target()) else {
        return;
    };
    if let Ok(mut state) = fields.get_mut(parent.0)
        && let Some(mut scrub) = state.scrub.take()
    {
        restore_scrub(&mut scrub, &mut sim.app);
        capture.block_modal_frame();
    }
}

/// Keep an active drag captured even when it leaves the toolbar or the window.
pub(super) fn capture_scrub(
    mut fields: Query<&mut SpeedEditor>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut sim: ResMut<Simulation>,
    mut capture: ResMut<crate::ui_input::UiInputCapture>,
) {
    for mut state in &mut fields {
        if let Some(scrub) = &mut state.scrub {
            capture.block_modal_frame();
            if keys.just_pressed(KeyCode::Escape) {
                restore_scrub(scrub, &mut sim.app);
            }
            // Covers releases outside the window that produce no PointerRelease.
            if !mouse.pressed(MouseButton::Left) {
                state.scrub = None;
            }
        }
    }
}

pub(super) fn sync_scrubber(
    focus: Res<InputFocus>,
    mut fields: Query<(Entity, &mut EntityCursor), With<SpeedEditor>>,
    mut layers: Query<(&SpeedScrubber, &mut Pickable)>,
) {
    for (entity, mut cursor) in &mut fields {
        cursor.set_if_neq(EntityCursor::System(if focus.get() == Some(entity) {
            SystemCursorIcon::Text
        } else {
            SystemCursorIcon::ColResize
        }));
    }
    for (parent, mut pickable) in &mut layers {
        pickable.set_if_neq(if focus.get() == Some(parent.0) {
            Pickable::IGNORE
        } else {
            Pickable::default()
        });
    }
}

pub(super) fn handle_key(
    mut event: On<FocusedInput<KeyboardInput>>,
    mut fields: Query<&mut SpeedEditor>,
) {
    if event.input.state != ButtonState::Pressed || event.input.repeat {
        return;
    }
    let Ok(mut field) = fields.get_mut(event.original_event_target()) else {
        return;
    };
    field.request = match event.input.key_code {
        KeyCode::Enter | KeyCode::NumpadEnter => Some(Request::Apply),
        KeyCode::Escape => Some(Request::Cancel),
        _ => return,
    };
    event.propagate(false);
}

#[derive(Debug, PartialEq)]
enum Speed {
    Fixed(f32),
    Max,
}

/// ∞ is hard to type, so `max`, `inf`, and `infinity` mean it too. The unit
/// the field shows at rest may be left in.
fn parse_speed(value: &str) -> Option<Speed> {
    let value = value.trim();
    let value = value.strip_suffix(UNIT).unwrap_or(value).trim_end();
    if value == "∞"
        || ["max", "inf", "infinity"]
            .iter()
            .any(|word| value.eq_ignore_ascii_case(word))
    {
        return Some(Speed::Max);
    }
    value
        .parse::<f32>()
        .ok()
        .filter(|rate| (1.0..=controls::MAX_FIXED_RATE).contains(rate))
        .map(Speed::Fixed)
}

/// The bare rate, which editing starts from.
fn formatted_rate(app: &GolApp) -> String {
    if app.is_max_speed() {
        return "∞".to_owned();
    }
    let value = format!("{:.1}", controls::fixed_rate(app));
    value.strip_suffix(".0").unwrap_or(&value).to_owned()
}

/// The rate with its unit, as the field shows it at rest. At ∞ while running,
/// that is the rate actually achieved, once one has been measured; the full
/// fill still says it is ∞.
fn displayed_rate(app: &GolApp, measured: f64) -> String {
    if app.is_max_speed() && app.is_running() && measured > 0.0 {
        return format!("{measured:.0} {UNIT}");
    }
    format!("{} {UNIT}", formatted_rate(app))
}

/// How far the fill reaches, up to the ∞ toggle: full at 40 gen/s and at ∞,
/// which the toggle itself marks.
fn fill_fraction(app: &GolApp) -> f32 {
    if app.is_max_speed() {
        1.0
    } else {
        (controls::fixed_rate(app) / controls::MAX_FIXED_RATE).clamp(0.0, 1.0)
    }
}

/// Before layout, so the fill follows a drag or step in the same frame.
pub(super) fn sync_fill(sim: Res<Simulation>, mut fills: Query<&mut Node, With<SpeedFill>>) {
    let fraction = fill_fraction(&sim.app);
    for mut node in &mut fills {
        set_if_neq!(node.width, percent(fraction * 100.0));
    }
}

type Field<'a> = (
    Entity,
    &'a mut SpeedEditor,
    &'a mut EditableText,
    &'a mut ThemedBorder,
    &'a mut controls::Tooltip,
);

/// Runs after queued text edits, so Enter also commits characters typed in this frame.
pub(super) fn edit(
    mut sim: ResMut<Simulation>,
    mut focus: ResMut<InputFocus>,
    mut capture: ResMut<crate::ui_input::UiInputCapture>,
    mut feedback: ResMut<EditFeedback>,
    mut fields: Query<Field>,
) {
    let Ok((entity, mut state, mut input, mut border, mut tooltip)) = fields.single_mut() else {
        return;
    };
    let focused = focus.get() == Some(entity);
    let mut draft = input.value().to_string();
    if draft != state.draft {
        state.invalid = false;
    }
    if focused && !state.focused {
        state.initial = formatted_rate(&sim.app);
        // The unit and a measured rate are shown only at rest, so editing
        // starts from the bare number, all selected, unless something was
        // already typed since the field last showed the rate.
        if draft == state.draft {
            input.editor_mut().set_text(&state.initial);
            input.queue_edit(TextEdit::SelectAll);
            draft = state.initial.clone();
        }
    }
    let rate_changed = state.last_interval.is_some_and(|interval| {
        interval != sim.app.tick_rate_ms() || state.last_max != sim.app.is_max_speed()
    });
    let request = state.request.take();
    let cancelled = matches!(request, Some(Request::Cancel));
    let apply = matches!(request, Some(Request::Apply))
        || (state.focused && !focused && draft != state.initial && !rate_changed);
    if apply && !cancelled {
        if let Some(speed) = parse_speed(&draft) {
            match speed {
                Speed::Fixed(rate) => sim.app.set_tick_rate_ms(controls::rate_to_ms(rate)),
                Speed::Max => sim.app.set_max_speed(),
            };
            state.invalid = false;
            if focused {
                focus.clear();
                capture.block_modal_frame();
            }
        } else {
            state.invalid = true;
            feedback.show(INVALID_HINT);
            if focused {
                focus.set(entity, FocusCause::Navigated);
            }
        }
    }
    if cancelled {
        state.invalid = false;
        focus.clear();
        capture.block_modal_frame();
    }
    if focus.get() != Some(entity) {
        let value = displayed_rate(&sim.app, sim.rate.per_second());
        if draft != value {
            input.editor_mut().set_text(&value);
        }
        state.invalid = false;
    }
    state.draft = input.value().to_string();
    state.focused = focus.get() == Some(entity);
    state.last_interval = Some(sim.app.tick_rate_ms());
    state.last_max = sim.app.is_max_speed();
    border.set_if_neq(ThemedBorder(if state.invalid {
        Token::Error
    } else if state.focused {
        Token::Accent
    } else {
        Token::Line
    }));
    set_if_neq!(
        tooltip.0,
        if state.invalid {
            INVALID_HINT
        } else {
            INPUT_HINT
        }
        .to_owned()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::{
        camera::NormalizedRenderTarget,
        picking::{
            backend::HitData,
            events::Pointer,
            pointer::{Location, PointerId},
        },
    };

    fn world() -> (World, Entity) {
        let mut world = World::new();
        world.insert_resource(Simulation::new(GolApp::new(8, 8)));
        world.init_resource::<InputFocus>();
        world.init_resource::<crate::ui_input::UiInputCapture>();
        world.init_resource::<EditFeedback>();
        world.init_resource::<ButtonInput<KeyCode>>();
        world.init_resource::<ButtonInput<MouseButton>>();
        world.init_resource::<bevy::ui::UiScale>();
        let field = world
            .spawn((
                SpeedEditor::default(),
                EditableText::new("40"),
                EntityCursor::System(SystemCursorIcon::ColResize),
                ThemedBorder(Token::Line),
                controls::Tooltip(INPUT_HINT.into()),
            ))
            .id();
        world.run_system_once(edit).unwrap();
        (world, field)
    }

    fn layer(world: &mut World, field: Entity) -> Entity {
        world
            .spawn((SpeedScrubber(field), Pickable::default()))
            .observe(scrub_focus)
            .observe(scrub_press)
            .observe(scrub_drag)
            .observe(scrub_release)
            .observe(scrub_cancel)
            .id()
    }

    fn pointer(position: Vec2) -> Pointer {
        Pointer::new(
            PointerId::Mouse,
            Location {
                target: NormalizedRenderTarget::None {
                    width: 0,
                    height: 0,
                },
                position,
            },
        )
    }

    fn press(world: &mut World, entity: Entity) {
        world.trigger(PointerPress {
            entity,
            pointer: pointer(Vec2::ZERO),
            button: PointerButton::Primary,
            hit: HitData::new(entity, 0.0, None, None),
            count: 1,
        });
    }

    fn drag(world: &mut World, entity: Entity, distance: f32, delta: f32) {
        world.trigger(PointerDrag {
            entity,
            pointer: pointer(Vec2::new(distance, 0.0)),
            button: PointerButton::Primary,
            distance: Vec2::new(distance, 0.0),
            delta: Vec2::new(delta, 0.0),
        });
    }

    fn release(world: &mut World, entity: Entity) {
        world.trigger(PointerRelease {
            entity,
            pointer: pointer(Vec2::ZERO),
            button: PointerButton::Primary,
            hit: HitData::new(entity, 0.0, None, None),
        });
    }

    #[test]
    fn scrubbing_applies_live_with_fine_control_and_limits_without_editing_the_board() {
        let (mut world, field) = world();
        let layer = layer(&mut world, field);
        world.resource_mut::<Simulation>().app.set_tick_rate_ms(100);
        world.resource_mut::<Simulation>().app.set_max_speed();
        press(&mut world, layer);
        // Sub-threshold pointer jitter remains a click and does not leave ∞.
        drag(&mut world, layer, 1.0, 1.0);
        assert!(world.resource::<Simulation>().app.is_max_speed());
        // A drag from ∞ starts at the fastest rate: 40 + 0.2 - 10.2 is 30 gen/s.
        drag(&mut world, layer, -50.0, -51.0);
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 33);
        assert!(!world.resource::<Simulation>().app.is_max_speed());
        world
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        drag(&mut world, layer, -100.0, -50.0);
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 34);
        world.resource_mut::<ButtonInput<KeyCode>>().reset_all();
        // Back past the fastest rate is ∞ again, where this drag began, and far
        // below the slowest is 1 gen/s.
        drag(&mut world, layer, 1000.0, 900.0);
        assert!(world.resource::<Simulation>().app.is_max_speed());
        drag(&mut world, layer, -1000.0, -2000.0);
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 1000);
        assert!(!world.resource::<Simulation>().app.is_max_speed());
        release(&mut world, layer);
        assert!(world.resource::<InputFocus>().get().is_none());
        assert!(world.get::<SpeedEditor>(field).unwrap().scrub.is_none());
        assert!(world.resource::<crate::ui_input::UiInputCapture>().pointer);
        // From a fixed rate, a drag stops at the fastest one; ∞ is the toggle's.
        press(&mut world, layer);
        drag(&mut world, layer, 1000.0, 1000.0);
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 25);
        assert!(!world.resource::<Simulation>().app.is_max_speed());
        release(&mut world, layer);
        assert_eq!(
            world.resource::<Simulation>().app.board.live_cells_count(),
            0
        );
    }

    #[test]
    fn a_click_enters_text_mode_but_escape_cancels_a_scrub_and_restores_max() {
        let (mut world, field) = world();
        let layer = layer(&mut world, field);
        world.resource_mut::<Simulation>().app.set_tick_rate_ms(100);
        world.resource_mut::<Simulation>().app.set_max_speed();
        press(&mut world, layer);
        release(&mut world, layer);
        assert_eq!(world.resource::<InputFocus>().get(), Some(field));
        assert!(world.resource::<Simulation>().app.is_max_speed());
        world.run_system_once(sync_scrubber).unwrap();
        assert_eq!(*world.get::<Pickable>(layer).unwrap(), Pickable::IGNORE);
        assert_eq!(
            *world.get::<EntityCursor>(field).unwrap(),
            EntityCursor::System(SystemCursorIcon::Text)
        );
        world.resource_mut::<InputFocus>().clear();
        world.run_system_once(sync_scrubber).unwrap();
        assert_eq!(*world.get::<Pickable>(layer).unwrap(), Pickable::default());
        press(&mut world, layer);
        // Dragging right from ∞ stays at ∞, and back left leaves it.
        drag(&mut world, layer, 25.0, 25.0);
        assert!(world.resource::<Simulation>().app.is_max_speed());
        drag(&mut world, layer, -25.0, -50.0);
        assert!(!world.resource::<Simulation>().app.is_max_speed());
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 29);
        world
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        world
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        world.run_system_once(capture_scrub).unwrap();
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 100);
        assert!(world.resource::<Simulation>().app.is_max_speed());
        // Moving after cancellation must neither alter speed nor enter text mode.
        drag(&mut world, layer, 50.0, 75.0);
        release(&mut world, layer);
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 100);
        assert!(world.resource::<InputFocus>().get().is_none());
        assert!(world.resource::<crate::ui_input::UiInputCapture>().keyboard);
    }

    #[test]
    fn scrubbing_keeps_input_captured_until_release_even_without_field_focus() {
        let (mut world, field) = world();
        let layer = layer(&mut world, field);
        press(&mut world, layer);
        world
            .resource_mut::<ButtonInput<MouseButton>>()
            .press(MouseButton::Left);
        *world.resource_mut::<crate::ui_input::UiInputCapture>() = default();
        world.run_system_once(capture_scrub).unwrap();
        assert!(world.resource::<crate::ui_input::UiInputCapture>().pointer);
        assert!(world.resource::<crate::ui_input::UiInputCapture>().keyboard);
        assert!(world.resource::<InputFocus>().get().is_none());
        world
            .resource_mut::<ButtonInput<MouseButton>>()
            .release(MouseButton::Left);
        world.run_system_once(capture_scrub).unwrap();
        assert!(world.get::<SpeedEditor>(field).unwrap().scrub.is_none());
        // The releasing frame remains captured, then the next frame is free.
        *world.resource_mut::<crate::ui_input::UiInputCapture>() = default();
        world.run_system_once(capture_scrub).unwrap();
        assert!(!world.resource::<crate::ui_input::UiInputCapture>().keyboard);
    }

    #[test]
    fn a_cancelled_pointer_restores_the_selected_rate_and_max_mode() {
        let (mut world, field) = world();
        let layer = layer(&mut world, field);
        world.resource_mut::<Simulation>().app.set_tick_rate_ms(100);
        world.resource_mut::<Simulation>().app.set_max_speed();
        press(&mut world, layer);
        drag(&mut world, layer, -50.0, -50.0);
        assert!(!world.resource::<Simulation>().app.is_max_speed());
        world.trigger(PointerCancel {
            entity: layer,
            pointer: pointer(Vec2::ZERO),
            hit: HitData::new(layer, 0.0, None, None),
        });
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 100);
        assert!(world.resource::<Simulation>().app.is_max_speed());
        assert!(world.get::<SpeedEditor>(field).unwrap().scrub.is_none());
    }

    #[test]
    fn entered_rates_apply_or_cancel_without_touching_the_board() {
        let (mut world, field) = world();
        world.resource_mut::<Simulation>().app.set_max_speed();
        world
            .resource_mut::<InputFocus>()
            .set(field, FocusCause::Navigated);
        world
            .get_mut::<EditableText>(field)
            .unwrap()
            .editor_mut()
            .set_text("12.5");
        world.get_mut::<SpeedEditor>(field).unwrap().request = Some(Request::Apply);
        world.run_system_once(edit).unwrap();
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 80);
        assert!(!world.resource::<Simulation>().app.is_max_speed());
        assert_eq!(
            world.resource::<Simulation>().app.board.live_cells_count(),
            0
        );
        assert!(world.resource::<InputFocus>().get().is_none());
        assert!(world.resource::<crate::ui_input::UiInputCapture>().keyboard);
        world.resource_mut::<Simulation>().app.set_max_speed();
        world
            .resource_mut::<InputFocus>()
            .set(field, FocusCause::Navigated);
        world
            .get_mut::<EditableText>(field)
            .unwrap()
            .editor_mut()
            .set_text("5");
        world.get_mut::<SpeedEditor>(field).unwrap().request = Some(Request::Cancel);
        world.run_system_once(edit).unwrap();
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 80);
        // Cancelling shows the current speed again, which is ∞, with its unit.
        assert!(world.resource::<Simulation>().app.is_max_speed());
        assert_eq!(
            world
                .get::<EditableText>(field)
                .unwrap()
                .value()
                .to_string(),
            "∞ gen/s"
        );
    }

    #[test]
    fn invalid_input_is_rejected_and_external_speed_changes_win_over_an_old_draft() {
        let (mut world, field) = world();
        world
            .resource_mut::<InputFocus>()
            .set(field, FocusCause::Navigated);
        world.run_system_once(edit).unwrap();
        for value in ["", "0", "41", "NaN", "-inf", "abc", "gen/s", "41 gen/s"] {
            world
                .get_mut::<EditableText>(field)
                .unwrap()
                .editor_mut()
                .set_text(value);
            world.get_mut::<SpeedEditor>(field).unwrap().request = Some(Request::Apply);
            world.run_system_once(edit).unwrap();
            assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 25);
            assert!(world.get::<SpeedEditor>(field).unwrap().invalid);
            assert_eq!(world.resource::<InputFocus>().get(), Some(field));
        }
        world
            .get_mut::<EditableText>(field)
            .unwrap()
            .editor_mut()
            .set_text("5");
        world.resource_mut::<Simulation>().app.set_tick_rate_ms(100);
        world.resource_mut::<InputFocus>().clear();
        world.run_system_once(edit).unwrap();
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 100);
        assert_eq!(
            world
                .get::<EditableText>(field)
                .unwrap()
                .value()
                .to_string(),
            "10 gen/s"
        );
    }

    #[test]
    fn infinity_can_be_typed_and_shows_in_the_field() {
        let (mut world, field) = world();
        for typed in ["∞", "max", "Infinity"] {
            world.resource_mut::<Simulation>().app.set_tick_rate_ms(100);
            world
                .resource_mut::<InputFocus>()
                .set(field, FocusCause::Navigated);
            world.run_system_once(edit).unwrap();
            world
                .get_mut::<EditableText>(field)
                .unwrap()
                .editor_mut()
                .set_text(typed);
            world.get_mut::<SpeedEditor>(field).unwrap().request = Some(Request::Apply);
            world.run_system_once(edit).unwrap();
            assert!(world.resource::<Simulation>().app.is_max_speed(), "{typed}");
            let shown = world
                .get::<EditableText>(field)
                .unwrap()
                .value()
                .to_string();
            assert_eq!(shown, "∞ gen/s");
        }
    }

    #[test]
    fn the_unit_shows_at_rest_and_editing_starts_from_the_bare_number() {
        let (mut world, field) = world();
        let text = |world: &World| {
            world
                .get::<EditableText>(field)
                .unwrap()
                .value()
                .to_string()
        };
        assert_eq!(text(&world), "40 gen/s");
        world
            .resource_mut::<InputFocus>()
            .set(field, FocusCause::Navigated);
        world.run_system_once(edit).unwrap();
        assert_eq!(text(&world), "40");
        // Leaving without a change keeps the rate and brings the unit back.
        world.resource_mut::<InputFocus>().clear();
        world.run_system_once(edit).unwrap();
        assert_eq!(text(&world), "40 gen/s");
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 25);
        // The unit may be typed, or left in.
        assert_eq!(parse_speed(" 20 gen/s "), Some(Speed::Fixed(20.0)));
        assert_eq!(parse_speed("∞ gen/s"), Some(Speed::Max));
    }

    #[test]
    fn infinity_shows_the_achieved_rate_while_running_and_edits_from_infinity() {
        let (mut world, field) = world();
        let text = |world: &mut World| {
            world.run_system_once(edit).unwrap();
            world
                .get::<EditableText>(field)
                .unwrap()
                .value()
                .to_string()
        };
        world.resource_mut::<Simulation>().app.set_max_speed();
        assert_eq!(text(&mut world), "∞ gen/s");
        // Until a rate has been measured, running still shows ∞.
        world.resource_mut::<Simulation>().app.toggle_running();
        assert_eq!(text(&mut world), "∞ gen/s");
        world
            .resource_mut::<Simulation>()
            .rate
            .record(20_711, std::time::Duration::from_millis(500));
        assert_eq!(text(&mut world), "41422 gen/s");
        // Editing starts from ∞, not the measured rate.
        world
            .resource_mut::<InputFocus>()
            .set(field, FocusCause::Navigated);
        assert_eq!(text(&mut world), "∞");
        world.resource_mut::<InputFocus>().clear();
        assert_eq!(text(&mut world), "41422 gen/s");
        assert!(world.resource::<Simulation>().app.is_max_speed());
        // Paused, it is ∞ again.
        world.resource_mut::<Simulation>().app.toggle_running();
        assert_eq!(text(&mut world), "∞ gen/s");
    }

    #[test]
    fn the_fill_follows_the_fixed_rates_and_fills_at_infinity() {
        let mut world = World::new();
        world.insert_resource(Simulation::new(GolApp::new(8, 8)));
        let fill = world.spawn((SpeedFill, Node::default())).id();
        let width = |world: &mut World| {
            world.run_system_once(sync_fill).unwrap();
            world.get::<Node>(fill).unwrap().width
        };
        world.resource_mut::<Simulation>().app.set_tick_rate_ms(50);
        assert_eq!(width(&mut world), percent(50));
        world
            .resource_mut::<Simulation>()
            .app
            .set_tick_rate_ms(controls::rate_to_ms(controls::MAX_FIXED_RATE));
        // 40 gen/s reaches the ∞ toggle, and ∞ is told apart by the toggle.
        assert_eq!(width(&mut world), percent(100));
        world.resource_mut::<Simulation>().app.set_max_speed();
        assert_eq!(width(&mut world), percent(100));
    }

    #[test]
    fn steps_appear_on_hover_and_while_one_has_keyboard_focus() {
        let mut world = World::new();
        world.init_resource::<InputFocus>();
        world.init_resource::<InputFocusVisible>();
        let control = world.spawn((SpeedControl, Hovered(false))).id();
        let step = world.spawn((Action::SpeedAdjust(1), Node::default())).id();
        let bar = world
            .spawn((
                SpeedChevron(step),
                ThemedBackground(Token::Ink),
                Visibility::Inherited,
            ))
            .id();
        let shown = |world: &mut World| {
            world.run_system_once(sync_chevrons).unwrap();
            *world.get::<Visibility>(bar).unwrap() == Visibility::Inherited
        };
        assert!(!shown(&mut world));
        world.entity_mut(control).insert(Hovered(true));
        assert!(shown(&mut world));
        world.entity_mut(control).insert(Hovered(false));
        world
            .resource_mut::<InputFocus>()
            .set(step, FocusCause::Navigated);
        assert!(shown(&mut world), "a focused step stays visible");
    }

    #[test]
    fn steps_in_a_shared_box_show_a_border_only_for_keyboard_focus() {
        let mut world = World::new();
        world.init_resource::<InputFocus>();
        world.init_resource::<InputFocusVisible>();
        let zoom = world.spawn((Action::ZoomIn, Node::default())).id();
        let speed = world.spawn((Action::SpeedAdjust(1), Node::default())).id();
        let border = |world: &World, entity| world.get::<Node>(entity).unwrap().border;
        for (entity, other) in [(zoom, speed), (speed, zoom)] {
            world
                .resource_mut::<InputFocus>()
                .set(entity, FocusCause::Navigated);
            // A click focuses too, but only keyboard focus is outlined.
            world.resource_mut::<InputFocusVisible>().0 = false;
            world.run_system_once(sync_chevrons).unwrap();
            assert_eq!(border(&world, entity), UiRect::all(px(0)));
            world.resource_mut::<InputFocusVisible>().0 = true;
            world.run_system_once(sync_chevrons).unwrap();
            assert_eq!(border(&world, entity), UiRect::all(px(1)));
            assert_eq!(border(&world, other), UiRect::all(px(0)));
        }
    }
}
