//! A fixed rate with Feathers-style scrubbing, direct entry, and step buttons.
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

const INPUT_HINT: &str =
    "Speed · Drag to adjust · Shift for finer control · Click to type · 1–40 gen/s";
const INVALID_HINT: &str = "Enter a speed from 1 to 40 gen/s.";
const DRAG_THRESHOLD: f32 = 3.0;
const RATE_PER_PIXEL: f32 = 0.2;

#[derive(Component)]
pub(super) struct SpeedScrubber(Entity);

#[derive(Component)]
pub(super) struct SpeedChevron(Entity);

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
        .spawn(Node {
            width: px(118),
            height: px(34),
            align_items: AlignItems::Center,
            column_gap: px(4),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|p| {
            p.spawn(Node {
                width: px(94),
                height: px(34),
                flex_shrink: 0.0,
                ..default()
            })
            .with_children(|p| {
                spawn_step_button(p, -1);
                let field = p
                    .spawn((
                        SpeedEditor::default(),
                        EditableText::new("40"),
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
                            width: px(94),
                            height: px(34),
                            padding: UiRect::axes(px(20), px(7)),
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
                        ThemedBackground(Token::Field),
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
                        left: px(21),
                        right: px(21),
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
            });
            p.spawn((
                label("/s", 13.0, Token::Muted),
                Node {
                    width: px(20),
                    ..default()
                },
            ));
            p.spawn((
                controls::SpeedLabel,
                label("", 11.0, Token::Muted),
                Node {
                    display: Display::None,
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px(34),
                    width: px(118),
                    ..default()
                },
            ));
        });
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
                right: if step > 0 { px(1) } else { Val::Auto },
                top: px(1),
                width: px(20),
                height: px(32),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border_radius: if step < 0 {
                    BorderRadius::left(px(5))
                } else {
                    BorderRadius::right(px(5))
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

pub(super) fn sync_chevrons(
    focus: Res<InputFocus>,
    visible: Res<InputFocusVisible>,
    mut buttons: Query<(Entity, &Action, Has<InteractionDisabled>, &mut Node)>,
    mut bars: Query<(&SpeedChevron, &mut ThemedBackground)>,
) {
    for (entity, action, _, mut node) in &mut buttons {
        if matches!(action, Action::SpeedAdjust(_)) {
            set_if_neq!(
                node.border,
                UiRect::all(px(if visible.0 && focus.get() == Some(entity) {
                    1.0
                } else {
                    0.0
                }))
            );
        }
    }
    for (chevron, mut color) in &mut bars {
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
        let base = 1000.0 / scrub.interval as f32;
        let rate = ((base + scrub.offset) * 10.0).round() / 10.0;
        sim.app.set_tick_rate_ms(controls::rate_to_ms(rate));
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

fn parse_rate(value: &str) -> Option<f32> {
    value
        .trim()
        .parse::<f32>()
        .ok()
        .filter(|rate| rate.is_finite() && (1.0..=40.0).contains(rate))
}

fn formatted_rate(app: &GolApp) -> String {
    let value = format!("{:.1}", controls::fixed_rate(app));
    value.strip_suffix(".0").unwrap_or(&value).to_owned()
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
    let draft = input.value().to_string();
    if draft != state.draft {
        state.invalid = false;
    }
    if focused && !state.focused {
        state.initial = formatted_rate(&sim.app);
    }
    let rate_changed = state.last_interval.is_some_and(|interval| {
        interval != sim.app.tick_rate_ms() || state.last_max != sim.app.is_max_speed()
    });
    let request = state.request.take();
    let cancelled = matches!(request, Some(Request::Cancel));
    let apply = matches!(request, Some(Request::Apply))
        || (state.focused && !focused && draft != state.initial && !rate_changed);
    if apply && !cancelled {
        if let Some(rate) = parse_rate(&draft) {
            sim.app.set_tick_rate_ms(controls::rate_to_ms(rate));
            state.invalid = false;
            if focused {
                focus.clear();
                capture.block_modal_frame();
            }
        } else {
            state.invalid = true;
            feedback.message = INVALID_HINT;
            feedback.remaining = 4.0;
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
        let value = formatted_rate(&sim.app);
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
        // Sub-threshold pointer jitter remains a click and does not leave Max.
        drag(&mut world, layer, 1.0, 1.0);
        assert!(world.resource::<Simulation>().app.is_max_speed());
        drag(&mut world, layer, 50.0, 49.0);
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 50);
        assert!(!world.resource::<Simulation>().app.is_max_speed());
        world
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::ShiftLeft);
        drag(&mut world, layer, 100.0, 50.0);
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 48);
        world.resource_mut::<ButtonInput<KeyCode>>().reset_all();
        drag(&mut world, layer, 1000.0, 900.0);
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 25);
        drag(&mut world, layer, -1000.0, -2000.0);
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 1000);
        release(&mut world, layer);
        assert!(world.resource::<InputFocus>().get().is_none());
        assert!(world.get::<SpeedEditor>(field).unwrap().scrub.is_none());
        assert!(world.resource::<crate::ui_input::UiInputCapture>().pointer);
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
        drag(&mut world, layer, -25.0, -25.0);
        assert_eq!(world.resource::<Simulation>().app.tick_rate_ms(), 200);
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
        drag(&mut world, layer, 50.0, 50.0);
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
        assert!(world.resource::<Simulation>().app.is_max_speed());
        assert_eq!(
            world
                .get::<EditableText>(field)
                .unwrap()
                .value()
                .to_string(),
            "12.5"
        );
    }

    #[test]
    fn invalid_input_is_rejected_and_external_speed_changes_win_over_an_old_draft() {
        let (mut world, field) = world();
        world
            .resource_mut::<InputFocus>()
            .set(field, FocusCause::Navigated);
        world.run_system_once(edit).unwrap();
        for value in ["", "0", "41", "NaN", "inf", "abc"] {
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
            "10"
        );
    }
}
