//! One row when there is room; the same controls reflow without losing focus.
use super::*;

const SINGLE_ROW_MIN_WIDTH: f32 = 1200.0;

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
    Brand,
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
            BackgroundColor(PANEL),
            BorderColor::all(LINE),
            GlobalZIndex(10),
        ))
        .with_children(|p| {
            p.spawn((Group::Brand, row())).with_children(|p| {
                p.spawn(label("LIFE", 21.0, ACCENT));
            });
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
                    BorderColor::all(LINE),
                    accessibility::named_node(accesskit::Role::Toolbar, name),
                ))
                .with_children(|p| match group {
                    Group::Playback => {
                        button(p, "Play", Action::Play);
                        button(p, "Step", Action::Step);
                        controls::speed_slider(p);
                    }
                    Group::Editing => {
                        button(p, "Randomize", Action::Randomize);
                        button(p, "Clear", Action::Clear);
                        button(p, "Undo", Action::Undo);
                        button(p, "Redo", Action::Redo);
                    }
                    Group::Workspace => {
                        button(p, "Grid", Action::Grid);
                        button(p, "Palette", Action::Sidebar);
                        button(p, "Import / Export", Action::Import);
                        button(p, "Settings", Action::Settings);
                        button(p, "Help", Action::Help);
                    }
                    Group::Brand => unreachable!(),
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
    Has<controls::SpeedSlider>,
    Has<Sidebar>,
);

pub(super) fn sync_layout(
    windows: Query<&Window, With<PrimaryWindow>>,
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
    let single = window.width() >= SINGLE_ROW_MIN_WIDTH;
    for (mut node, toolbar, group, tabs, button, action, slider, sidebar) in &mut nodes {
        if toolbar {
            node.display = Display::Grid;
            node.height = px(height(window.width()));
            let columns = if single {
                // Explicit group budgets keep the flexible speed slider from
                // consuming space needed by the edit and workspace buttons.
                vec![
                    GridTrack::px(44.0),
                    GridTrack::px(368.0),
                    GridTrack::px(272.0),
                    GridTrack::flex(1.0),
                ]
            } else {
                vec![
                    GridTrack::px(44.0),
                    GridTrack::flex(1.0),
                    GridTrack::px(272.0),
                ]
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
                (true, Group::Brand) => (1, 1, 1, 0),
                (true, Group::Playback) => (2, 1, 1, -3),
                (true, Group::Editing) => (3, 1, 1, -2),
                (true, Group::Workspace) => (4, 1, 1, -1),
                (false, Group::Brand) => (1, 1, 1, 0),
                (false, Group::Workspace) => (2, 2, 1, -3),
                (false, Group::Playback) => (1, 2, 2, -2),
                (false, Group::Editing) => (3, 1, 2, -1),
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
        if slider {
            node.width = px(if single { 100.0 } else { 150.0 });
            node.max_width = px(if single { 120.0 } else { 200.0 });
        }
        if sidebar {
            node.top = px(height(window.width()) + 16.0);
        }
    }
}
