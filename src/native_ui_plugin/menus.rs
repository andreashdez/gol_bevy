//! Compact workspace menus using Bevy's menu navigation and focus handling.
use super::*;
use bevy::{
    a11y::AccessibilityNode,
    input_focus::{FocusCause, tab_navigation::NavAction},
    ui::OverrideClip,
    ui_widgets::{
        MenuAction, MenuButton, MenuEvent, MenuFocusState, MenuItem, MenuPopup,
        popover::{Popover, PopoverAlign, PopoverPlacement, PopoverSide},
    },
};

#[derive(Component)]
pub(super) struct WorkspacePopup(pub Entity);

/// Accessibility actions emit Activate directly, bypassing pointer/key handlers.
pub(super) fn item_activated(
    event: On<Activate>,
    items: Query<(), With<MenuItem>>,
    mut commands: Commands,
) {
    if items.contains(event.entity) {
        commands.trigger(MenuEvent {
            source: event.entity,
            action: MenuAction::FocusRoot,
        });
        commands.trigger(MenuEvent {
            source: event.entity,
            action: MenuAction::CloseAll,
        });
    }
}

pub(super) fn spawn(parent: &mut ChildSpawnerCommands, title: &str, action: Action) -> Entity {
    let owner = super::button(parent, title, action);
    parent
        .commands()
        .entity(owner)
        .insert((MenuButton, toolbar::ToolbarButton))
        .with_children(|p| {
            p.spawn((
                WorkspacePopup(owner),
                MenuPopup::default(),
                Node {
                    display: Display::None,
                    position_type: PositionType::Absolute,
                    width: px(200),
                    padding: UiRect::all(px(6)),
                    row_gap: px(4),
                    flex_direction: FlexDirection::Column,
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(8)),
                    ..default()
                },
                Popover {
                    positions: vec![PopoverPlacement {
                        side: PopoverSide::Bottom,
                        align: PopoverAlign::End,
                        gap: 6.0,
                    }],
                    window_margin: 8.0,
                },
                OverrideClip,
                ThemedBackground(Token::Panel),
                ThemedBorder(Token::Line),
                GlobalZIndex(50),
            ))
            .with_children(|p| {
                let entries = match action {
                    Action::ViewMenu => {
                        vec![
                            ("Grid", Action::Grid),
                            ("Cell age", Action::CellAge),
                            ("Pattern palette", Action::Sidebar),
                        ]
                    }
                    Action::MoreMenu => {
                        vec![("Settings", Action::Settings), ("Help", Action::Help)]
                    }
                    _ => unreachable!("workspace menu action"),
                };
                for (title, action) in entries {
                    let item = super::button(p, title, action);
                    // MenuItem emits Activate itself; a Button would activate twice.
                    p.commands().entity(item).remove::<WidgetButton>().insert((
                        MenuItem,
                        TabIndex(-1),
                        accessibility::named_node(accesskit::Role::MenuItem, title),
                        Node {
                            width: percent(100),
                            padding: UiRect::axes(px(10), px(9)),
                            border: UiRect::all(px(1)),
                            border_radius: BorderRadius::all(px(6)),
                            align_items: AlignItems::Center,
                            ..default()
                        },
                    ));
                }
            });
        });
    owner
}

pub(super) fn menu_event(
    mut event: On<MenuEvent>,
    parents: Query<&ChildOf>,
    mut owners: Query<&mut AccessibilityNode, With<MenuButton>>,
    mut popups: Query<(Entity, &WorkspacePopup, &mut Node, &mut MenuFocusState)>,
    mut items: Query<(Entity, &mut TabIndex), With<MenuItem>>,
    mut focus: ResMut<InputFocus>,
    mut capture: ResMut<crate::ui_input::UiInputCapture>,
) {
    let Some(owner) = std::iter::once(event.source)
        .chain(parents.iter_ancestors(event.source))
        .find(|entity| owners.contains(*entity))
    else {
        return;
    };
    let Some((popup, _, mut node, mut state)) =
        popups.iter_mut().find(|(_, popup, _, _)| popup.0 == owner)
    else {
        return;
    };
    event.propagate(false);
    capture.block_modal_frame();
    let open = match event.action {
        MenuAction::Open(nav) => Some(nav),
        MenuAction::Toggle if node.display == Display::None => Some(NavAction::First),
        MenuAction::Toggle | MenuAction::CloseAll => None,
        MenuAction::FocusRoot => {
            focus.set(owner, FocusCause::Navigated);
            return;
        }
    };
    node.display = if open.is_some() {
        Display::Flex
    } else {
        Display::None
    };
    *state = open.map_or(MenuFocusState::Closed, MenuFocusState::Opening);
    owners.get_mut(owner).unwrap().set_expanded(open.is_some());
    // Opening must restore tab indices before Bevy acquires menu focus in PostUpdate.
    for (entity, mut index) in &mut items {
        if parents
            .iter_ancestors(entity)
            .any(|ancestor| ancestor == popup)
        {
            index.0 = if open.is_some() { 0 } else { -1 };
        }
    }
}

/// Canvas clicks dismiss menus without painting; the closing frame stays captured.
#[derive(SystemParam)]
pub(super) struct MenuInput<'w, 's> {
    mode: Res<'w, NativeUi>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    focus: ResMut<'w, InputFocus>,
    capture: ResMut<'w, crate::ui_input::UiInputCapture>,
}

pub(super) fn dismiss(
    mut input: MenuInput,
    mut popups: Query<(
        &WorkspacePopup,
        &mut Node,
        &mut MenuFocusState,
        &ComputedNode,
        &UiGlobalTransform,
    )>,
    mut owners: Query<
        (&ComputedNode, &UiGlobalTransform, &mut AccessibilityNode),
        With<MenuButton>,
    >,
) {
    let Ok(window) = input.windows.single() else {
        return;
    };
    let rect = |node: &ComputedNode, transform: &UiGlobalTransform| {
        let scale = node.inverse_scale_factor();
        Rect::from_center_size(transform.translation * scale, node.size() * scale)
    };
    for (popup, mut node, mut state, computed, transform) in &mut popups {
        if node.display == Display::None {
            continue;
        }
        let Ok((owner_node, owner_transform, mut accessible)) = owners.get_mut(popup.0) else {
            continue;
        };
        let outside = input.mouse.any_just_pressed([
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
        ]) && window.cursor_position().is_some_and(|cursor| {
            !rect(computed, transform).contains(cursor)
                && !rect(owner_node, owner_transform).contains(cursor)
        });
        if input.mode.dialog.is_some() || input.keys.just_pressed(KeyCode::Escape) || outside {
            node.display = Display::None;
            *state = MenuFocusState::Closed;
            accessible.set_expanded(false);
            input.capture.block_modal_frame();
            if input.mode.dialog.is_none() {
                input.focus.set(popup.0, FocusCause::Navigated);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::{
        ecs::system::RunSystemOnce,
        input::{
            ButtonState,
            keyboard::{Key, KeyboardInput},
        },
        input_focus::InputDispatchPlugin,
        ui_widgets::{ButtonPlugin, MenuPlugin},
    };

    #[test]
    fn keyboard_menu_navigation_toggles_once_and_escape_restores_focus() {
        let mut app = App::new();
        app.init_resource::<Assets<Font>>()
            .init_resource::<InputFocus>()
            .init_resource::<InputFocusVisible>()
            .init_resource::<NativeUi>()
            .init_resource::<UiLayout>()
            .init_resource::<Theme>()
            .init_resource::<EditFeedback>()
            .init_resource::<crate::ui_input::UiInputCapture>()
            .insert_resource(GridOverlay { enabled: false })
            .init_resource::<crate::shared::CellAge>()
            .insert_resource(Simulation::new(GolApp::new(8, 8)))
            .add_plugins((
                bevy::input::InputPlugin,
                InputDispatchPlugin,
                TabNavigationPlugin,
                ButtonPlugin,
                MenuPlugin,
            ))
            .add_observer(menu_event)
            .add_observer(item_activated)
            .add_observer(activate);
        app.world_mut().run_system_once(setup).unwrap();
        // This headless harness has no renderer to propagate UI visibility.
        for mut visibility in app
            .world_mut()
            .query::<&mut InheritedVisibility>()
            .iter_mut(app.world_mut())
        {
            *visibility = InheritedVisibility::VISIBLE;
        }
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let owner = app
            .world_mut()
            .query::<(Entity, &Action)>()
            .iter(app.world())
            .find(|(_, action)| **action == Action::ViewMenu)
            .unwrap()
            .0;
        let popup = app
            .world_mut()
            .query::<(Entity, &WorkspacePopup)>()
            .iter(app.world())
            .find(|(_, popup)| popup.0 == owner)
            .unwrap()
            .0;
        let send = |app: &mut App, key_code, logical_key: Key| {
            for state in [ButtonState::Pressed, ButtonState::Released] {
                app.world_mut().write_message(KeyboardInput {
                    key_code,
                    logical_key: logical_key.clone(),
                    state,
                    text: None,
                    repeat: false,
                    window,
                });
                app.world_mut().run_schedule(PreUpdate);
            }
        };
        app.world_mut()
            .resource_mut::<InputFocus>()
            .set(owner, FocusCause::Navigated);
        send(&mut app, KeyCode::ArrowDown, Key::ArrowDown);
        app.world_mut().run_schedule(PostUpdate);
        let focused = app.world().resource::<InputFocus>().get().unwrap();
        assert_eq!(app.world().get::<Action>(focused), Some(&Action::Grid));
        send(&mut app, KeyCode::ArrowDown, Key::ArrowDown);
        let focused = app.world().resource::<InputFocus>().get().unwrap();
        assert_eq!(app.world().get::<Action>(focused), Some(&Action::CellAge));
        send(&mut app, KeyCode::ArrowDown, Key::ArrowDown);
        let focused = app.world().resource::<InputFocus>().get().unwrap();
        assert_eq!(app.world().get::<Action>(focused), Some(&Action::Sidebar));
        send(&mut app, KeyCode::Enter, Key::Enter);
        assert!(!app.world().resource::<UiLayout>().control_deck_visible);
        assert_eq!(
            app.world().get::<Node>(popup).unwrap().display,
            Display::None
        );
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(owner));
        assert!(
            app.world()
                .resource::<crate::ui_input::UiInputCapture>()
                .pointer
        );
        send(&mut app, KeyCode::ArrowDown, Key::ArrowDown);
        app.world_mut().run_schedule(PostUpdate);
        send(&mut app, KeyCode::Escape, Key::Escape);
        assert_eq!(
            app.world().get::<Node>(popup).unwrap().display,
            Display::None
        );
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(owner));
        assert!(!app.world().resource::<Simulation>().app.should_quit);
        // Accessible clicks use Activate directly, and must close the menu too.
        send(&mut app, KeyCode::ArrowDown, Key::ArrowDown);
        app.world_mut().run_schedule(PostUpdate);
        let grid = app.world().resource::<InputFocus>().get().unwrap();
        app.world_mut().trigger(Activate { entity: grid });
        app.world_mut().flush();
        assert!(app.world().resource::<GridOverlay>().enabled);
        assert_eq!(
            app.world().get::<Node>(popup).unwrap().display,
            Display::None
        );
        assert_eq!(app.world().resource::<InputFocus>().get(), Some(owner));
    }
}
