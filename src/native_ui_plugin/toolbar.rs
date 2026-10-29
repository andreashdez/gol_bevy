//! One row at every width. Where the workspace controls do not fit, they move
//! into a menu behind a More button at the right end.
use super::*;
use bevy::{ecs::system::SystemParam, input_focus::FocusCause};

pub(super) const HEIGHT: f32 = 64.0;
/// Below this width the workspace controls collapse into the menu.
pub(super) const COLLAPSE_BELOW_WIDTH: f32 = 1000.0;
/// Below this width the palette folds into its rail, so the board keeps most of
/// the window. It concerns the canvas, not the toolbar, so it has its own width.
pub(super) const PALETTE_COLLAPSE_BELOW_WIDTH: f32 = 900.0;

/// Column budgets for the playback and board edit groups, with a little slack.
/// With the workspace controls, one row needs 1000 px; collapsed, it fits the
/// 680 px minimum window.
const PLAYBACK_WIDTH: f32 = 336.0;
const EDITING_WIDTH: f32 = 252.0;
const MENU_WIDTH: f32 = 220.0;

#[derive(Component)]
pub(super) struct Toolbar;

#[derive(Component, Clone, Copy)]
pub(super) enum Group {
    Playback,
    Editing,
    Workspace,
}

#[derive(Component)]
pub(super) struct ToolbarButton;

/// Whether the workspace controls are collapsed into the menu, and whether it is open.
#[derive(Resource, Default)]
pub(crate) struct WorkspaceMenu {
    pub collapsed: bool,
    pub open: bool,
}

#[derive(Component)]
pub(super) struct MoreButton;
/// Holds the workspace controls: a row in the toolbar, or the open menu.
#[derive(Component)]
pub(super) struct MenuPanel;
#[derive(Component)]
pub(super) struct MenuItem;
/// Shown inside the menu only: names beside icons, and an icon-sized gap
/// that lines up a label without an icon.
#[derive(Component)]
pub(super) struct MenuLabel;

pub(super) fn button(parent: &mut ChildSpawnerCommands, text: &str, action: Action) {
    let entity = super::button(parent, text, action);
    parent.commands().entity(entity).insert(ToolbarButton);
}

fn icon_button(
    parent: &mut ChildSpawnerCommands,
    name: &str,
    icon: Icon,
    action: Action,
) -> Entity {
    let entity = super::icon_button(parent, name, icon, action);
    parent.commands().entity(entity).insert(ToolbarButton);
    entity
}

fn menu_item(parent: &mut ChildSpawnerCommands, name: &str, icon: Option<Icon>, action: Action) {
    let entity = match icon {
        Some(icon) => {
            let entity = icon_button(parent, name, icon, action);
            parent.commands().entity(entity).with_child((
                MenuLabel,
                label(name, 14.0, Token::Ink),
                Node {
                    display: Display::None,
                    ..default()
                },
            ));
            entity
        }
        None => {
            let entity = button_frame(parent, name, action);
            parent
                .commands()
                .entity(entity)
                .insert(ToolbarButton)
                .with_children(|p| {
                    p.spawn((
                        MenuLabel,
                        Node {
                            display: Display::None,
                            width: px(icons::DISPLAY_SIZE),
                            ..default()
                        },
                    ));
                    p.spawn(label(name, 14.0, Token::Ink));
                });
            entity
        }
    };
    parent.commands().entity(entity).insert(MenuItem);
}

pub(super) fn spawn(commands: &mut Commands) {
    commands
        .spawn((
            NativeRoot,
            Toolbar,
            Node {
                display: Display::Grid,
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                top: px(0),
                height: px(HEIGHT),
                padding: UiRect::all(px(12)),
                column_gap: px(12),
                row_gap: px(8),
                align_items: AlignItems::Center,
                border: UiRect::bottom(px(1)),
                ..default()
            },
            ThemedBackground(Token::Panel),
            ThemedBorder(Token::Line),
            GlobalZIndex(10),
        ))
        .with_children(|p| {
            for (group, name) in [
                (Group::Playback, "Playback"),
                (Group::Editing, "Board edits"),
                (Group::Workspace, "Workspace"),
            ] {
                p.spawn((
                    group,
                    TabGroup::default(),
                    Node {
                        align_items: AlignItems::Center,
                        column_gap: px(6),
                        min_width: px(0),
                        ..default()
                    },
                    ThemedBorder(Token::Line),
                    accessibility::named_node(accesskit::Role::Toolbar, name),
                ))
                .with_children(|p| match group {
                    Group::Playback => {
                        icon_button(p, "Play", Icon::Play, Action::Play);
                        icon_button(p, "Step", Icon::Step, Action::Step);
                        speed::spawn(p);
                    }
                    Group::Editing => {
                        button(p, "Randomize", Action::Randomize);
                        button(p, "Clear", Action::Clear);
                        icon_button(p, "Undo", Icon::Undo, Action::Undo);
                        icon_button(p, "Redo", Icon::Redo, Action::Redo);
                    }
                    // View toggles show their state. The More button comes first
                    // so Tab reaches the open menu right after it.
                    Group::Workspace => {
                        let more = icon_button(p, "More", Icon::More, Action::More);
                        p.commands().entity(more).insert(MoreButton);
                        p.spawn((
                            MenuPanel,
                            Node {
                                align_items: AlignItems::Center,
                                column_gap: px(6),
                                row_gap: px(4),
                                border_radius: BorderRadius::all(px(10)),
                                ..default()
                            },
                            ThemedBackground(Token::Panel),
                            ThemedBorder(Token::Line),
                            GlobalZIndex(40),
                        ))
                        .with_children(|p| {
                            menu_item(p, "Grid", Some(Icon::Grid), Action::Grid);
                            menu_item(p, "Cell age", Some(Icon::CellAge), Action::CellAge);
                            menu_item(p, "Pattern palette", Some(Icon::Palette), Action::Sidebar);
                            menu_item(p, "Import / Export", None, Action::Import);
                            menu_item(p, "Settings", Some(Icon::Gear), Action::Settings);
                            menu_item(p, "Help", Some(Icon::Help), Action::Help);
                        });
                    }
                });
            }
        });
}

type LayoutNode<'a> = (
    &'a mut Node,
    Has<Toolbar>,
    Option<&'a Group>,
    Option<&'a mut TabGroup>,
    Has<ToolbarButton>,
    Option<&'a Action>,
    Has<Sidebar>,
    Has<ToolStrip>,
    (
        Has<MenuPanel>,
        Has<MenuItem>,
        Has<MenuLabel>,
        Has<MoreButton>,
    ),
    (Has<ToolIcon>, Has<ToolLabel>),
);

/// Narrow windows hide the palette, so the board keeps most of the width;
/// widening brings it back. Acts only as the width
/// crosses the breakpoint, so the palette can still be shown while narrow.
pub(super) fn collapse_palette(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut layout: ResMut<UiLayout>,
    mut was_narrow: Local<Option<bool>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let narrow = window.width() < PALETTE_COLLAPSE_BELOW_WIDTH;
    if *was_narrow != Some(narrow) {
        // Startup counts as a crossing, so a narrow first window starts collapsed.
        *was_narrow = Some(narrow);
        layout.set_narrow(narrow);
    }
}

pub(super) fn sync_layout(
    windows: Query<&Window, With<PrimaryWindow>>,
    layout: Res<UiLayout>,
    mut menu: ResMut<WorkspaceMenu>,
    mut nodes: Query<LayoutNode>,
    mut previous_layout: Local<Option<(f32, bool)>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let current = (window.width(), layout.control_deck_visible);
    if *previous_layout == Some(current) {
        return;
    }
    *previous_layout = Some(current);
    let collapsed = window.width() < COLLAPSE_BELOW_WIDTH;
    set_if_neq!(menu.collapsed, collapsed);
    if !collapsed {
        set_if_neq!(menu.open, false);
    }
    for (mut node, toolbar, group, tabs, button, action, sidebar, tools, menu_parts, tool_parts) in
        &mut nodes
    {
        let (panel, item, menu_label, more) = menu_parts;
        let (tool_icon, tool_label) = tool_parts;
        // Beside the palette the tools are named; in the rail they are icons.
        let rail = !layout.control_deck_visible;
        if tool_icon || tool_label {
            node.display = if tool_icon == rail {
                Display::Flex
            } else {
                Display::None
            };
        }
        if matches!(action, Some(Action::Tool(_))) {
            node.width = if rail { Val::Auto } else { px(68) };
        }
        if toolbar {
            node.display = Display::Grid;
            node.height = px(HEIGHT);
            // Explicit group budgets leave space for all toolbar controls.
            let columns = vec![
                GridTrack::px(PLAYBACK_WIDTH),
                GridTrack::px(EDITING_WIDTH),
                GridTrack::flex(1.0),
            ];
            if node.grid_template_columns != columns {
                node.grid_template_columns = columns;
            }
        }
        if let Some(group) = group {
            let (column, order) = match group {
                Group::Playback => (1, -3),
                Group::Editing => (2, -2),
                Group::Workspace => (3, -1),
            };
            node.grid_column = GridPlacement::start(column);
            node.grid_row = GridPlacement::start(1);
            // Only the edits are set apart; the workspace controls sit at the far end.
            let divider = matches!(group, Group::Editing);
            node.border = UiRect::left(px(if divider { 1.0 } else { 0.0 }));
            node.padding = UiRect::left(px(if divider { 12.0 } else { 0.0 }));
            node.justify_content = if matches!(group, Group::Workspace) {
                JustifyContent::FlexEnd
            } else {
                JustifyContent::FlexStart
            };
            if let Some(mut tabs) = tabs {
                tabs.order = order;
            }
        }
        if button {
            node.padding = UiRect::axes(px(8), px(9));
            if action == Some(&Action::Play) {
                node.min_width = px(64);
            }
        }
        if sidebar {
            node.top = px(HEIGHT + 16.0 + TOOL_STRIP_SPACE);
        }
        if tools {
            node.top = px(HEIGHT + 16.0);
            node.width = px(if layout.control_deck_visible {
                SIDE
            } else {
                TOOL_RAIL_WIDTH
            });
            node.height = px(if layout.control_deck_visible {
                TOOL_STRIP_HEIGHT
            } else {
                TOOL_RAIL_HEIGHT
            });
            node.flex_direction = if layout.control_deck_visible {
                FlexDirection::Row
            } else {
                FlexDirection::Column
            };
            node.row_gap = px(6);
        }
        if panel {
            // A dropdown under the More button, right-aligned with it.
            node.position_type = if collapsed {
                PositionType::Absolute
            } else {
                PositionType::Relative
            };
            node.top = if collapsed { percent(100) } else { Val::Auto };
            node.right = if collapsed { px(0) } else { Val::Auto };
            node.margin = UiRect::top(px(if collapsed { 10.0 } else { 0.0 }));
            node.width = if collapsed { px(MENU_WIDTH) } else { Val::Auto };
            node.flex_direction = if collapsed {
                FlexDirection::Column
            } else {
                FlexDirection::Row
            };
            node.align_items = if collapsed {
                AlignItems::Stretch
            } else {
                AlignItems::Center
            };
            node.padding = UiRect::all(px(if collapsed { 6.0 } else { 0.0 }));
            node.border = UiRect::all(px(if collapsed { 1.0 } else { 0.0 }));
        }
        if item {
            node.justify_content = if collapsed {
                JustifyContent::FlexStart
            } else {
                JustifyContent::Center
            };
            node.column_gap = px(10);
        }
        if menu_label {
            node.display = if collapsed {
                Display::Flex
            } else {
                Display::None
            };
        }
        if more {
            node.display = if collapsed {
                Display::Flex
            } else {
                Display::None
            };
        }
    }
}

/// Opens and closes the menu. It closes on Escape, a click outside it, Tab
/// past its items, choosing an item, or a dialog opening.
/// What closes the menu: keys, clicks, and where keyboard focus goes.
#[derive(SystemParam)]
pub(super) struct MenuInput<'w, 's> {
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    focus: ResMut<'w, InputFocus>,
    focus_visible: Res<'w, InputFocusVisible>,
}

/// Opens and closes the menu. It closes on Escape, a click outside it, Tab
/// past its items, choosing an item, or a dialog opening.
pub(super) fn sync_menu(
    mut menu: ResMut<WorkspaceMenu>,
    mode: Res<NativeUi>,
    mut input: MenuInput,
    more: Query<(Entity, &ComputedNode, &UiGlobalTransform), With<MoreButton>>,
    mut panels: Query<
        (
            &mut Node,
            &mut Visibility,
            &ComputedNode,
            &UiGlobalTransform,
        ),
        With<MenuPanel>,
    >,
    items: Query<(), With<MenuItem>>,
) {
    let Ok((more, more_node, more_transform)) = more.single() else {
        return;
    };
    let rect = |node: &ComputedNode, transform: &UiGlobalTransform| {
        let scale = node.inverse_scale_factor();
        Rect::from_center_size(transform.translation * scale, node.size() * scale)
    };
    let focused = input.focus.get();
    let in_menu = focused.is_some_and(|e| items.contains(e));
    if menu.open {
        let cursor = input
            .windows
            .single()
            .ok()
            .and_then(Window::cursor_position);
        let mut rects = vec![rect(more_node, more_transform)];
        rects.extend(
            panels
                .iter()
                .map(|(_, _, node, transform)| rect(node, transform)),
        );
        let escape = input.keys.just_pressed(KeyCode::Escape);
        let clicked_outside = input.mouse.any_just_pressed([
            MouseButton::Left,
            MouseButton::Middle,
            MouseButton::Right,
        ]) && cursor.is_none_or(|p| !rects.iter().any(|r| r.contains(p)));
        let tabbed_away = input.focus_visible.0 && focused.is_some_and(|e| e != more && !in_menu);
        if escape || clicked_outside || tabbed_away || mode.dialog.is_some() || !menu.collapsed {
            menu.open = false;
            if escape && in_menu {
                input.focus.set(more, FocusCause::Navigated);
            }
        }
    } else if menu.collapsed && in_menu {
        // A dialog opened from the menu hands focus back to its item once closed.
        input.focus.set(more, FocusCause::Navigated);
    }
    let shown = !menu.collapsed || menu.open;
    for (mut node, mut visibility, _, _) in &mut panels {
        set_if_neq!(
            node.display,
            if shown { Display::Flex } else { Display::None }
        );
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}

/// More toggles the menu; choosing an item closes it. From the keyboard, the
/// menu opens with its first item focused.
pub(super) fn menu_action(
    event: On<Activate>,
    mut menu: ResMut<WorkspaceMenu>,
    more: Query<(), With<MoreButton>>,
    items: Query<(), With<MenuItem>>,
    panels: Query<&Children, With<MenuPanel>>,
    mut focus: ResMut<InputFocus>,
    focus_visible: Res<InputFocusVisible>,
) {
    if more.contains(event.entity) {
        menu.open = !menu.open;
        if menu.open
            && focus_visible.0
            && let Some(first) = panels.iter().flat_map(|c| c.iter()).next()
        {
            focus.set(first, FocusCause::Navigated);
        }
    } else if menu.collapsed && items.contains(event.entity) {
        menu.open = false;
    }
}
