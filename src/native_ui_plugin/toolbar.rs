//! One row at every width. Where the workspace controls do not fit, they move
//! into a menu behind a More button at the right end.
use super::*;
use bevy::{ecs::system::SystemParam, input_focus::FocusCause};

pub(super) const HEIGHT: f32 = 64.0;
/// Below this width the workspace controls collapse into the menu.
pub(super) const COLLAPSE_BELOW_WIDTH: f32 = 1010.0;

/// Column budgets for the groups before the workspace controls, with a little
/// slack. With those controls, one row needs 1010 px; collapsed, it fits the
/// 800 px minimum window.
const PLAYBACK_WIDTH: f32 = 378.0;
const TOOLS_WIDTH: f32 = 140.0;
const EDITING_WIDTH: f32 = 180.0;
const MENU_WIDTH: f32 = 220.0;

#[derive(Component)]
pub(super) struct Toolbar;

#[derive(Component, Clone, Copy)]
pub(super) enum Group {
    Playback,
    Tools,
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
/// Shown inside the menu only: names beside icons.
#[derive(Component)]
pub(super) struct MenuLabel;

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

fn menu_item(parent: &mut ChildSpawnerCommands, name: &str, icon: Icon, action: Action) {
    let entity = icon_button(parent, name, icon, action);
    parent
        .commands()
        .entity(entity)
        .insert(MenuItem)
        .with_child((
            MenuLabel,
            label(name, 14.0, Token::Ink),
            Node {
                display: Display::None,
                ..default()
            },
        ));
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
                (Group::Tools, "Drawing tools"),
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
                        icon_button(p, "Reset to start", Icon::Reset, Action::ResetToStart);
                        icon_button(p, "Step", Icon::Step, Action::Step);
                        speed::spawn(p);
                    }
                    Group::Tools => {
                        icon_button(p, "Draw", Icon::Draw, Action::Tool(EditTool::Draw));
                        icon_button(p, "Erase", Icon::Erase, Action::Tool(EditTool::Erase));
                        icon_button(p, "Stamp", Icon::Stamp, Action::Tool(EditTool::Stamp));
                    }
                    Group::Editing => {
                        icon_button(p, "Randomize", Icon::Randomize, Action::Randomize);
                        icon_button(p, "Clear", Icon::Clear, Action::Clear);
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
                            menu_item(p, "Grid", Icon::Grid, Action::Grid);
                            menu_item(p, "Cell age", Icon::CellAge, Action::CellAge);
                            menu_item(p, "Import", Icon::Import, Action::Import);
                            menu_item(p, "Export", Icon::Export, Action::Export);
                            menu_item(p, "Settings", Icon::Gear, Action::Settings);
                            menu_item(p, "Help", Icon::Help, Action::Help);
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
    (
        Has<MenuPanel>,
        Has<MenuItem>,
        Has<MenuLabel>,
        Has<MoreButton>,
    ),
);

pub(super) fn sync_layout(
    windows: Query<&Window, With<PrimaryWindow>>,
    mut menu: ResMut<WorkspaceMenu>,
    mut nodes: Query<LayoutNode>,
    mut previous_width: Local<Option<f32>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    if *previous_width == Some(window.width()) {
        return;
    }
    *previous_width = Some(window.width());
    let collapsed = window.width() < COLLAPSE_BELOW_WIDTH;
    set_if_neq!(menu.collapsed, collapsed);
    if !collapsed {
        set_if_neq!(menu.open, false);
    }
    for (mut node, toolbar, group, tabs, button, action, menu_parts) in &mut nodes {
        let (panel, item, menu_label, more) = menu_parts;
        if toolbar {
            node.display = Display::Grid;
            node.height = px(HEIGHT);
            // Explicit group budgets leave space for all toolbar controls.
            let columns = vec![
                GridTrack::px(PLAYBACK_WIDTH),
                GridTrack::px(TOOLS_WIDTH),
                GridTrack::px(EDITING_WIDTH),
                GridTrack::flex(1.0),
            ];
            if node.grid_template_columns != columns {
                node.grid_template_columns = columns;
            }
        }
        if let Some(group) = group {
            let (column, order) = match group {
                Group::Playback => (1, -4),
                Group::Tools => (2, -3),
                Group::Editing => (3, -2),
                Group::Workspace => (4, -1),
            };
            node.grid_column = GridPlacement::start(column);
            node.grid_row = GridPlacement::start(1);
            // The workspace controls need no divider; they sit at the far end.
            let divider = matches!(group, Group::Tools | Group::Editing);
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
