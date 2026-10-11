//! AccessKit metadata, actions, and keyboard navigation for the native interface.
use super::*;
use accesskit::{Action as AccessibleAction, ActionData, Live, Node as AccessibleNode, Role};
use bevy::{
    a11y::{AccessibilityNode, AccessibilitySystems, ActionRequest},
    input_focus::{FocusCause, FocusGained},
    text::{EditableText, TextEdit},
    ui::UiSystems,
    ui_widgets::ScrollIntoView,
};

pub(super) struct NativeAccessibilityPlugin;
impl Plugin for NativeAccessibilityPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, handle_actions.before(super::NativeLayout))
            .add_systems(
                PostUpdate,
                sync_accessibility
                    .after(UiSystems::PostLayout)
                    .before(AccessibilitySystems::Update),
            )
            .add_observer(reveal_focus);
    }
}

pub(super) fn named_node(role: Role, label: &str) -> AccessibilityNode {
    let mut node = AccessibleNode::new(role);
    node.set_label(label);
    AccessibilityNode(node)
}

pub(super) fn dialog_node(title: &str) -> AccessibilityNode {
    let mut node = named_node(Role::Dialog, title);
    node.set_modal();
    node
}

pub(super) fn feedback_node() -> AccessibilityNode {
    let mut node = named_node(Role::Status, "");
    node.set_live(Live::Polite);
    node.set_live_atomic();
    node
}

type NativeRoots = Or<(With<NativeRoot>, With<dialogs::DialogRoot>)>;

#[derive(SystemParam)]
struct NativeTree<'w, 's> {
    mode: Res<'w, NativeUi>,
    parents: Query<'w, 's, &'static ChildOf>,
    nodes: Query<'w, 's, &'static Node>,
    roots: Query<'w, 's, (), NativeRoots>,
    dialogs: Query<'w, 's, (), With<dialogs::DialogRoot>>,
}

impl NativeTree<'_, '_> {
    /// None means this belongs to another frontend. Hidden controls must neither
    /// appear to a screen reader nor remain in Bevy's tab order.
    fn visible(&self, entity: Entity) -> Option<bool> {
        let ancestors: Vec<_> = std::iter::once(entity)
            .chain(self.parents.iter_ancestors(entity))
            .collect();
        if !ancestors.iter().any(|e| self.roots.contains(*e)) {
            return None;
        }
        Some(
            (self.mode.dialog.is_none() || ancestors.iter().any(|e| self.dialogs.contains(*e)))
                && !ancestors.iter().any(|e| {
                    self.nodes
                        .get(*e)
                        .is_ok_and(|node| node.display == Display::None)
                }),
        )
    }
}

type AccessibleControl<'a> = (
    Entity,
    &'a mut AccessibilityNode,
    Option<&'a mut TabIndex>,
    Option<&'a EditableText>,
    Option<&'a Text>,
    Option<&'a Action>,
    Has<InteractionDisabled>,
    Has<ImageNode>,
    Option<&'a mut ThemedBorder>,
);

fn sync_accessibility(
    tree: NativeTree,
    mut focus: ResMut<InputFocus>,
    mut controls: Query<AccessibleControl>,
) {
    // Hiding the palette can hide the focused button. Restore focus to its
    // visible equivalent, or return keyboard control to the canvas.
    if tree.mode.dialog.is_none()
        && let Some(entity) = focus.get()
        && let Ok((_, _, _, _, _, action, disabled, _, _)) = controls.get(entity)
        && (tree.visible(entity) == Some(false) || disabled)
    {
        let replacement = action.copied().and_then(|action| {
            controls
                .iter()
                .find_map(|(candidate, _, tab, _, _, other, disabled, _, _)| {
                    (other == Some(&action)
                        && tab.is_some()
                        && !disabled
                        && tree.visible(candidate) == Some(true))
                    .then_some(candidate)
                })
        });
        if let Some(entity) = replacement {
            focus.set(entity, FocusCause::Navigated);
        } else {
            focus.clear();
        }
    }
    for (entity, mut node, tab_index, input, text, action, disabled, image, border) in &mut controls
    {
        let Some(visible) = tree.visible(entity) else {
            continue;
        };
        // Pattern images are decorative; the card already announces the pattern.
        if visible && !image {
            node.clear_hidden();
        } else {
            node.set_hidden();
        }
        if disabled {
            node.set_disabled();
        } else {
            node.clear_disabled();
        }
        if let Some(mut index) = tab_index {
            // Bevy's tab walker does not skip Display::None or disabled widgets.
            index.set_if_neq(TabIndex(if visible && !disabled { 0 } else { -1 }));
            node.add_action(AccessibleAction::Focus);
        }
        if action.is_some() {
            node.add_action(AccessibleAction::Click);
        }
        if let Some(input) = input {
            let value = input.value().into_iter().collect::<String>();
            if node.value() != Some(value.as_str()) {
                node.set_value(value);
            }
            node.add_action(AccessibleAction::SetValue);
            if input.allow_newlines {
                node.set_role(Role::MultilineTextInput);
            }
            if let Some(mut border) = border
                && border.0 != Token::Error
            {
                border.set_if_neq(ThemedBorder(if focus.get() == Some(entity) {
                    Token::Accent
                } else {
                    Token::Line
                }));
            }
        } else if let Some(text) = text
            && node.label() != Some(text.0.as_str())
        {
            node.set_label(text.0.as_str());
        }
    }
}

fn handle_actions(
    mut requests: MessageReader<ActionRequest>,
    tree: NativeTree,
    targets: Query<(Option<&Action>, Has<InteractionDisabled>)>,
    mut fields: Query<&mut EditableText>,
    sliders: Query<&bevy::ui_widgets::SliderValue>,
    mut focus: ResMut<InputFocus>,
    mut commands: Commands,
) {
    for request in requests.read() {
        let Some(entity) = Entity::try_from_bits(request.target_node.0) else {
            continue;
        };
        if tree.visible(entity) != Some(true) {
            continue;
        }
        let Ok((action, disabled)) = targets.get(entity) else {
            continue;
        };
        if disabled {
            continue;
        }
        match request.action {
            AccessibleAction::Click if action.is_some() => {
                focus.set(entity, FocusCause::Navigated);
                commands.trigger(Activate { entity });
            }
            AccessibleAction::Focus => focus.set(entity, FocusCause::Navigated),
            AccessibleAction::SetValue => {
                if let Some(ActionData::Value(value)) = &request.data
                    && let Ok(mut input) = fields.get_mut(entity)
                {
                    input.queue_edit(TextEdit::SelectAll);
                    input.queue_edit(TextEdit::Insert(value.as_ref().into()));
                    focus.set(entity, FocusCause::Navigated);
                } else if sliders.contains(entity) {
                    let value = match &request.data {
                        Some(ActionData::NumericValue(value)) => Some(*value as f32),
                        Some(ActionData::Value(value)) => value.parse::<f32>().ok(),
                        _ => None,
                    };
                    if let Some(value) = value.filter(|v| v.is_finite()) {
                        commands.trigger(bevy::ui_widgets::SetSliderValue {
                            entity,
                            change: bevy::ui_widgets::SliderValueChange::Absolute(value),
                        });
                        focus.set(entity, FocusCause::Navigated);
                    }
                }
            }
            AccessibleAction::Increment | AccessibleAction::Decrement
                if sliders.contains(entity) =>
            {
                commands.trigger(bevy::ui_widgets::SetSliderValue {
                    entity,
                    change: bevy::ui_widgets::SliderValueChange::Relative(
                        if request.action == AccessibleAction::Increment {
                            1.0
                        } else {
                            -1.0
                        },
                    ),
                });
                focus.set(entity, FocusCause::Navigated);
            }
            _ => {}
        }
    }
}

fn reveal_focus(event: On<FocusGained>, tree: NativeTree, mut commands: Commands) {
    let entity = event.event_target();
    if tree.visible(entity) == Some(true) {
        commands.trigger(ScrollIntoView { entity });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    };
    use bevy::input_focus::tab_navigation::{NavAction, TabNavigation};

    fn world() -> World {
        let mut world = World::new();
        world.insert_resource(NativeUi { dialog: None });
        world.insert_resource(InputFocus::default());
        world.init_resource::<Messages<ActionRequest>>();
        world
    }

    #[test]
    fn hidden_opener_returns_focus_to_a_visible_equivalent() {
        let mut world = world();
        let root = world.spawn((NativeRoot, Node::default())).id();
        let mut button = |display| {
            world
                .spawn((
                    Node {
                        display,
                        ..default()
                    },
                    Action::Settings,
                    named_node(Role::Button, "Settings"),
                    TabIndex(0),
                    ChildOf(root),
                ))
                .id()
        };
        let hidden = button(Display::None);
        let visible = button(Display::Flex);
        world
            .resource_mut::<InputFocus>()
            .set(hidden, FocusCause::Navigated);
        world.run_system_once(sync_accessibility).unwrap();
        assert_eq!(world.resource::<InputFocus>().get(), Some(visible));
    }

    #[test]
    fn disabled_and_hidden_controls_leave_tab_order_and_modal_background_is_hidden() {
        let mut world = world();
        let root = world
            .spawn((NativeRoot, Node::default(), TabGroup::default()))
            .id();
        let hidden = world
            .spawn((
                Node {
                    display: Display::None,
                    ..default()
                },
                ChildOf(root),
            ))
            .id();
        let button = |world: &mut World, parent| {
            world
                .spawn((
                    Node::default(),
                    Action::Step,
                    named_node(Role::Button, "Step"),
                    TabIndex(0),
                    // Tab navigation skips invisible entities, and this bare
                    // world never runs visibility propagation.
                    InheritedVisibility::VISIBLE,
                    ChildOf(parent),
                ))
                .id()
        };
        let offscreen = button(&mut world, hidden);
        let disabled = button(&mut world, root);
        world.entity_mut(disabled).insert(InteractionDisabled);
        let active = button(&mut world, root);
        world.run_system_once(sync_accessibility).unwrap();
        assert!(
            world
                .get::<AccessibilityNode>(offscreen)
                .unwrap()
                .is_hidden()
        );
        assert_eq!(world.get::<TabIndex>(disabled).unwrap().0, -1);
        let next = world
            .run_system_once(|nav: TabNavigation, focus: Res<InputFocus>| {
                nav.navigate(&focus, NavAction::Next)
            })
            .unwrap()
            .unwrap();
        assert_eq!(next, active);
        world.resource_mut::<NativeUi>().dialog = Some(DialogKind::Import);
        world.run_system_once(sync_accessibility).unwrap();
        assert!(world.get::<AccessibilityNode>(active).unwrap().is_hidden());
        assert_eq!(world.get::<TabIndex>(active).unwrap().0, -1);
    }

    #[test]
    fn accessible_set_value_edits_the_field_and_rejects_modal_background_actions() {
        let mut world = world();
        let root = world.spawn((NativeRoot, Node::default())).id();
        let field = world
            .spawn((Node::default(), EditableText::new("old"), ChildOf(root)))
            .id();
        let request = |world: &mut World, value: &str| {
            world.write_message(ActionRequest(accesskit::ActionRequest {
                action: AccessibleAction::SetValue,
                target_tree: accesskit::TreeId::ROOT,
                target_node: accesskit::NodeId(field.to_bits()),
                data: Some(ActionData::Value(value.into())),
            }));
        };
        request(&mut world, "new\nvalue");
        world.run_system_once(handle_actions).unwrap();
        let input = world.get::<EditableText>(field).unwrap();
        assert!(
            input
                .pending_edits
                .iter()
                .any(|edit| matches!(edit, TextEdit::Insert(value) if value == "new\nvalue"))
        );
        world
            .get_mut::<EditableText>(field)
            .unwrap()
            .pending_edits
            .clear();
        world.resource_mut::<NativeUi>().dialog = Some(DialogKind::Settings);
        request(&mut world, "blocked");
        world.run_system_once(handle_actions).unwrap();
        assert!(
            world
                .get::<EditableText>(field)
                .unwrap()
                .pending_edits
                .is_empty()
        );
    }

    /// Exercise Bevy's real input dispatch without touching the system clipboard:
    /// OS clipboard I/O occurs later, when the queued edit is applied.
    #[test]
    fn platform_clipboard_chords_and_shift_selection_reach_the_focused_editor() {
        let mut app = App::new();
        app.init_resource::<InputFocus>();
        app.add_message::<bevy::window::Ime>();
        app.add_plugins((
            bevy::input::InputPlugin,
            bevy::input_focus::InputDispatchPlugin,
            bevy::ui_widgets::TextInputPlugin,
        ));
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let field = app
            .world_mut()
            .spawn((
                EditableText::new("two lines\nof text"),
                bevy::ui_widgets::TextInput,
            ))
            .id();
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(field, FocusCause::Navigated);
        let send = |app: &mut App, key_code, logical_key: Key, state| {
            let text = if let Key::Character(value) = &logical_key {
                Some(value.clone())
            } else {
                None
            };
            app.world_mut().write_message(KeyboardInput {
                key_code,
                logical_key,
                state,
                text,
                repeat: false,
                window,
            });
            app.world_mut().run_schedule(PreUpdate);
        };
        let (modifier_code, modifier) = if cfg!(target_os = "macos") {
            (KeyCode::SuperLeft, Key::Super)
        } else {
            (KeyCode::ControlLeft, Key::Control)
        };
        send(
            &mut app,
            modifier_code,
            modifier.clone(),
            ButtonState::Pressed,
        );
        for (code, key, expected) in [
            (KeyCode::KeyA, "a", 0),
            (KeyCode::KeyC, "c", 1),
            (KeyCode::KeyX, "x", 2),
            (KeyCode::KeyV, "v", 3),
        ] {
            app.world_mut()
                .get_mut::<EditableText>(field)
                .unwrap()
                .pending_edits
                .clear();
            send(
                &mut app,
                code,
                Key::Character(key.into()),
                ButtonState::Pressed,
            );
            let input = app.world().get::<EditableText>(field).unwrap();
            assert!(input.pending_edits.iter().any(|edit| matches!(
                (expected, edit),
                (0, TextEdit::SelectAll)
                    | (1, TextEdit::Copy)
                    | (2, TextEdit::Cut)
                    | (3, TextEdit::Paste)
            )));
        }
        send(&mut app, modifier_code, modifier, ButtonState::Released);
        send(
            &mut app,
            KeyCode::ShiftLeft,
            Key::Shift,
            ButtonState::Pressed,
        );
        send(
            &mut app,
            KeyCode::ArrowLeft,
            Key::ArrowLeft,
            ButtonState::Pressed,
        );
        assert!(
            app.world()
                .get::<EditableText>(field)
                .unwrap()
                .pending_edits
                .iter()
                .any(|edit| matches!(edit, TextEdit::Left(true)))
        );
    }
}
