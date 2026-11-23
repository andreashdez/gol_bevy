//! Spawning the root, toolbar, palette, canvas, and status bar, plus focus and input-region bookkeeping
use super::*;

pub(super) fn label(value: impl Into<String>, size: f32, color: Token) -> impl Bundle {
    (
        Text::new(value),
        TextFont {
            font: UI_FONT.into(),
            font_size: FontSize::Px(size),
            ..default()
        },
        ThemedText(color),
        Pickable::IGNORE,
    )
}

pub(super) fn row() -> impl Bundle {
    (
        Node {
            align_items: AlignItems::Center,
            column_gap: px(8),
            flex_shrink: 0.0,
            ..default()
        },
        accessibility::named_node(accesskit::Role::GenericContainer, ""),
    )
}

pub(super) fn button(parent: &mut ChildSpawnerCommands, text: &str, action: Action) -> Entity {
    let entity = button_frame(parent, text, action);
    parent
        .commands()
        .entity(entity)
        .with_child(label(text, 14.0, Token::Ink));
    entity
}

/// A button that shows an icon; `name` still labels it for tooltips and screen readers.
pub(super) fn icon_button(
    parent: &mut ChildSpawnerCommands,
    name: &str,
    icon: Icon,
    action: Action,
) -> Entity {
    let entity = button_frame(parent, name, action);
    parent
        .commands()
        .entity(entity)
        .with_child(icons::node(icon));
    entity
}

pub(super) fn button_frame(
    parent: &mut ChildSpawnerCommands,
    name: &str,
    action: Action,
) -> Entity {
    parent
        .spawn((
            Node {
                padding: UiRect::axes(px(12), px(9)),
                min_width: if action == Action::Play {
                    px(72)
                } else {
                    px(0)
                },
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(7)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                flex_shrink: 0.0,
                ..default()
            },
            WidgetButton,
            action,
            Hovered::default(),
            TabIndex(0),
            ThemedBackground(Token::Surface),
            ThemedBorder(Token::Line),
            Name::new(name.to_owned()),
            controls::Tooltip(controls::hint(action)),
        ))
        .id()
}

pub(super) fn setup(
    mut commands: Commands,
    mut fonts: ResMut<Assets<Font>>,
    mut images: ResMut<Assets<Image>>,
) {
    // Embed the licensed font so launching a desktop bundle works from any directory.
    fonts
        .insert(
            UI_FONT.id(),
            Font::from_bytes(include_bytes!("../../assets/fonts/IBMPlexSans-Regular.ttf").to_vec()),
        )
        .expect("static UI font handle is valid");
    icons::insert_all(&mut images);
    toolbar::spawn(&mut commands);
    commands
        .spawn((
            NativeRoot,
            Sidebar,
            TabGroup {
                order: TAB_ORDER_PALETTE,
                ..default()
            },
            Node {
                position_type: PositionType::Absolute,
                left: px(16),
                top: px(toolbar::HEIGHT + 16.0),
                bottom: px(STATUS_BAR_HEIGHT + 16.0),
                width: px(SIDE),
                padding: UiRect::all(px(10)),
                flex_direction: FlexDirection::Column,
                row_gap: px(8),
                overflow: Overflow::clip(),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(12)),
                ..default()
            },
            ThemedBackground(Token::Panel),
            ThemedBorder(Token::Line),
            GlobalZIndex(10),
        ))
        .with_children(|p| {
            // Search stays at the top; the stamp card appears under it, so
            // choosing a stamp never moves the field.
            palette::search(p);
            palette::preview(p);
            palette::summary(p);
            palette::library(p);
        });
    commands
        .spawn((
            NativeRoot,
            Footer,
            TabGroup {
                order: TAB_ORDER_STATUS_BAR,
                ..default()
            },
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                bottom: px(0),
                height: px(STATUS_BAR_HEIGHT),
                padding: UiRect::axes(px(16), px(4)),
                align_items: AlignItems::Center,
                column_gap: px(16),
                row_gap: px(8),
                border: UiRect::top(px(1)),
                ..default()
            },
            ThemedBackground(Token::Panel),
            ThemedBorder(Token::Line),
            GlobalZIndex(10),
        ))
        .with_children(|p| {
            p.spawn((
                FooterStatus,
                Node {
                    align_items: AlignItems::Center,
                    column_gap: px(14),
                    height: px(34),
                    flex_grow: 1.0,
                    flex_shrink: 0.0,
                    ..default()
                },
            ))
            .with_children(|p| {
                p.spawn((
                    StatusPill,
                    // Wide enough for either word, so Play never shifts the readouts.
                    Node {
                        min_width: px(76),
                        justify_content: JustifyContent::Center,
                        padding: UiRect::axes(px(10), px(3)),
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(px(10)),
                        flex_shrink: 0.0,
                        ..default()
                    },
                    ThemedBackground(Token::Surface),
                    ThemedBorder(Token::Line),
                ))
                .with_child((label("", 12.0, Token::Muted), Readout::Status));
                // Muted words around ink numbers. Numbers keep the widest size
                // they have reached, so their neighbours do not shift as they tick.
                p.spawn(controls::readout_group()).with_children(|p| {
                    p.spawn(label("Gen", 13.0, Token::Muted));
                    p.spawn(controls::number(Readout::Generation, 13.0, Token::Ink));
                });
                p.spawn((
                    controls::readout_group(),
                    controls::PopulationReadout,
                    controls::Tooltip(String::new()),
                    Hovered::default(),
                    accessibility::named_node(accesskit::Role::Group, "Population"),
                ))
                .with_children(|p| {
                    p.spawn(controls::number(Readout::Population, 13.0, Token::Ink));
                    p.spawn(label("alive", 13.0, Token::Muted));
                    p.spawn(controls::number(Readout::Change, 12.0, Token::Muted));
                    p.spawn(sparkline::node());
                });
                button(p, "Rule", Action::EditRule);
                button(p, "Edges", Action::ToggleEdges);
                p.spawn(controls::number(Readout::Pointer, 12.0, Token::Muted));
                p.spawn((label("", 12.0, Token::Muted), Readout::Board));
            });
            p.spawn(row()).with_children(|p| {
                controls::zoom_stepper(p);
                icon_button(p, "Fit board", Icon::Fit, Action::FitBoard);
            });
        });
    // Last in the tab order, so Tab and Shift+Tab can return to the board.
    // Bevy only tabs to entities inside a group, and skips invisible ones,
    // so the group, the focus target, and the ring are separate nodes.
    commands
        .spawn((
            NativeRoot,
            BoardArea,
            TabGroup {
                order: TAB_ORDER_BOARD,
                ..default()
            },
            Node {
                position_type: PositionType::Absolute,
                ..default()
            },
            Pickable::IGNORE,
        ))
        .with_children(|p| {
            p.spawn((
                BoardFocus,
                TabIndex(0),
                Node {
                    width: percent(100),
                    height: percent(100),
                    ..default()
                },
                Pickable::IGNORE,
                accessibility::named_node(accesskit::Role::Canvas, "Board"),
            ))
            .with_child((
                BoardFocusRing,
                Node {
                    width: percent(100),
                    height: percent(100),
                    border: UiRect::all(px(2)),
                    border_radius: BorderRadius::all(px(4)),
                    ..default()
                },
                ThemedBorder(Token::Focus),
                Visibility::Hidden,
                Pickable::IGNORE,
            ));
            minimap::spawn(p);
        });
    controls::overlays(&mut commands);
    controls::feedback_overlay(&mut commands);
}

/// A clicked button gives the keyboard back to the board, so Space keeps
/// meaning play/pause. Tab still focuses buttons, with a visible outline.
pub(super) fn release_pointer_focus(
    mut focus: ResMut<InputFocus>,
    focus_visible: Res<InputFocusVisible>,
    regions: Res<UiInputRegions>,
    buttons: Query<(), With<Action>>,
) {
    if !focus_visible.0 && !regions.modal && focus.get().is_some_and(|e| buttons.contains(e)) {
        focus.clear();
    }
}

/// A palette that closes gives up the keyboard, so nothing hidden keeps focus.
pub(super) fn release_palette_focus(
    sim: Res<Simulation>,
    mut focus: ResMut<InputFocus>,
    palettes: Query<(), With<Sidebar>>,
    parents: Query<&ChildOf>,
) {
    if !sim.app.palette_shown()
        && focus
            .get()
            .is_some_and(|e| parents.iter_ancestors(e).any(|a| palettes.contains(a)))
    {
        focus.clear();
    }
}

/// Outlines the canvas while keyboard navigation leaves the keyboard on the board.
pub(super) fn sync_board_focus(
    windows: Query<&Window, With<PrimaryWindow>>,
    board_focus: BoardFocusVisible,
    mut areas: Query<&mut Node, With<BoardArea>>,
    mut rings: Query<&mut Visibility, With<BoardFocusRing>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let rect = canvas_rect(window.size());
    for mut node in &mut areas {
        set_if_neq!(node.left, px(rect.min.x));
        set_if_neq!(node.top, px(rect.min.y));
        set_if_neq!(node.width, px(rect.width()));
        set_if_neq!(node.height, px(rect.height()));
    }
    let ring = if board_focus.get() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut rings {
        visibility.set_if_neq(ring);
    }
}

/// Where the palette floats over the canvas.
pub(super) fn palette_rect(size: Vec2) -> Rect {
    let min = Vec2::new(16.0, toolbar::HEIGHT + 16.0);
    let max = Vec2::new(16.0 + SIDE, size.y - STATUS_BAR_HEIGHT - 16.0);
    Rect::from_corners(min, max.max(min))
}

/// Logical rectangles shared by layout and input routing; gaps remain canvas.
pub(super) fn panel_rects(size: Vec2, palette: bool) -> Vec<Rect> {
    let mut rects = vec![
        Rect::new(0.0, 0.0, size.x, toolbar::HEIGHT),
        Rect::new(0.0, (size.y - STATUS_BAR_HEIGHT).max(0.0), size.x, size.y),
    ];
    if palette {
        rects.push(palette_rect(size));
    }
    rects
}

/// The board is clipped to this workspace; the UI camera still covers the window.
/// The palette floats over it rather than beside it, because with Fill canvas on,
/// a canvas that changes size resizes the board, cropping its cells.
pub fn canvas_rect(size: Vec2) -> Rect {
    const GAP: f32 = 8.0;
    let max = (size - Vec2::new(GAP, STATUS_BAR_HEIGHT + GAP)).max(Vec2::ONE);
    let min = Vec2::new(GAP, toolbar::HEIGHT + GAP)
        .min(max - Vec2::ONE)
        .max(Vec2::ZERO);
    Rect::from_corners(min, max)
}

pub(super) fn publish_input_regions(
    windows: Query<&Window, With<PrimaryWindow>>,
    sim: Res<Simulation>,
    state: Res<NativeUi>,
    menu: Res<toolbar::WorkspaceMenu>,
    minimap: Res<minimap::MinimapArea>,
    mut regions: ResMut<UiInputRegions>,
) {
    // An open menu holds the pointer and keyboard like a dialog, so the click
    // that dismisses it does not also draw on the board.
    regions.modal = state.dialog.is_some() || menu.open;
    regions.panels = windows
        .single()
        .map(|window| panel_rects(window.size(), sim.app.palette_shown()))
        .unwrap_or_default();
    regions.panels.extend(minimap.0);
}
