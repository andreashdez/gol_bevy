//! One row when there is room; the same controls reflow without losing focus.
use super::*;

pub(super) const SINGLE_ROW_MIN_WIDTH: f32 = 900.0;

/// Column budgets for the playback and board edit groups, with a little slack.
/// With the workspace controls, one row needs 900 px.
const PLAYBACK_WIDTH: f32 = 232.0;
const EDITING_WIDTH: f32 = 252.0;

pub(super) fn height(width: f32) -> f32 {
    if width >= SINGLE_ROW_MIN_WIDTH {
        64.0
    } else {
        112.0
    }
}

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

pub(super) fn button(parent: &mut ChildSpawnerCommands, text: &str, action: Action) {
    let entity = super::button(parent, text, action);
    parent.commands().entity(entity).insert(ToolbarButton);
}

fn icon_button(parent: &mut ChildSpawnerCommands, name: &str, icon: Icon, action: Action) {
    let entity = super::icon_button(parent, name, icon, action);
    parent.commands().entity(entity).insert(ToolbarButton);
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
                height: px(height(1200.0)),
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
                    // View toggles show their state; every workspace control is one click.
                    Group::Workspace => {
                        icon_button(p, "Grid", Icon::Grid, Action::Grid);
                        icon_button(p, "Cell age", Icon::CellAge, Action::CellAge);
                        icon_button(p, "Pattern palette", Icon::Palette, Action::Sidebar);
                        button(p, "Import / Export", Action::Import);
                        icon_button(p, "Settings", Icon::Gear, Action::Settings);
                        icon_button(p, "Help", Icon::Help, Action::Help);
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
);

pub(super) fn sync_layout(
    windows: Query<&Window, With<PrimaryWindow>>,
    layout: Res<UiLayout>,
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
    let single = window.width() >= SINGLE_ROW_MIN_WIDTH;
    for (mut node, toolbar, group, tabs, button, action, sidebar, tools) in &mut nodes {
        if toolbar {
            node.display = Display::Grid;
            node.height = px(height(window.width()));
            let columns = if single {
                // Explicit group budgets leave space for all toolbar controls.
                vec![
                    GridTrack::px(PLAYBACK_WIDTH),
                    GridTrack::px(EDITING_WIDTH),
                    GridTrack::flex(1.0),
                ]
            } else {
                vec![GridTrack::flex(1.0), GridTrack::px(EDITING_WIDTH)]
            };
            let rows = vec![GridTrack::flex(1.0); if single { 1 } else { 2 }];
            if node.grid_template_columns != columns {
                node.grid_template_columns = columns;
            }
            if node.grid_template_rows != rows {
                node.grid_template_rows = rows;
            }
        }
        if let Some(group) = group {
            let (column, span, row, order) = match (single, group) {
                (true, Group::Playback) => (1, 1, 1, -3),
                (true, Group::Editing) => (2, 1, 1, -2),
                (true, Group::Workspace) => (3, 1, 1, -1),
                (false, Group::Workspace) => (1, 2, 1, -3),
                (false, Group::Playback) => (1, 1, 2, -2),
                (false, Group::Editing) => (2, 1, 2, -1),
            };
            node.grid_column = GridPlacement::start_span(column, span);
            node.grid_row = GridPlacement::start(row);
            let divider =
                matches!(group, Group::Editing) || (single && matches!(group, Group::Workspace));
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
            node.top = px(height(window.width()) + 16.0 + TOOL_STRIP_SPACE);
        }
        if tools {
            node.top = px(height(window.width()) + 16.0);
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
    }
}
