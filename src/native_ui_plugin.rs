//! Native Bevy interface for the simulation.

/// Assigns only when the value differs. UI rendering is retained, so writing an
/// unchanged value every frame would still mark the node for re-extraction.
macro_rules! set_if_neq {
    ($place:expr, $value:expr) => {{
        let value = $value;
        if $place != value {
            $place = value;
        }
    }};
}

mod accessibility;
mod controls;
mod dialogs;
mod icons;
mod menus;
mod palette;
mod speed;
mod theme;
mod toolbar;
use crate::{
    app::{App as GolApp, EditTool},
    shared::{
        CellAge, EditFeedback, GridOverlay, Simulation, StartupSet, Theme, UiLayout, UpdateSet,
    },
    ui_input::{BoardFocus, KeyboardFocus, UiInputRegions, UiInputSet},
};
use bevy::{
    asset::RenderAssetUsages,
    ecs::system::SystemParam,
    image::ImageSampler,
    input_focus::{
        InputFocus, InputFocusVisible,
        tab_navigation::{TabGroup, TabIndex, TabNavigationPlugin},
    },
    picking::hover::Hovered,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
    ui::{InteractionDisabled, Pressed},
    ui_widgets::{Activate, Button as WidgetButton, ScrollArea},
    window::PrimaryWindow,
};
use dialogs::DialogKind;
use icons::Icon;
use theme::{ThemedBackground, ThemedBorder, ThemedImage, ThemedText, Token};

fn bottom_height(size: Vec2) -> f32 {
    if size.x < 1000.0 { 88.0 } else { 48.0 }
}
const SIDE: f32 = 256.0;
const TOOL_STRIP_HEIGHT: f32 = 48.0;
const TOOL_STRIP_SPACE: f32 = TOOL_STRIP_HEIGHT + 8.0;
const TOOL_RAIL_WIDTH: f32 = 80.0;
const TOOL_RAIL_HEIGHT: f32 = 144.0;
const UI_FONT: Handle<Font> = bevy::asset::uuid_handle!("67dfd7f0-b829-4a23-a5ae-3e416c5aba2d");

#[derive(Resource, Default)]
pub struct NativeUi {
    pub(crate) dialog: Option<DialogKind>,
}

#[derive(SystemSet, Debug, Hash, PartialEq, Eq, Clone)]
pub(crate) struct NativeLayout;

pub struct NativeUiPlugin;
impl Plugin for NativeUiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NativeUi>()
            .init_resource::<EditFeedback>()
            .init_resource::<Theme>()
            .add_plugins((TabNavigationPlugin, bevy::picking::cursor::CursorIconPlugin))
            .add_observer(activate)
            .add_observer(menus::menu_event)
            .add_observer(menus::item_activated)
            .add_observer(speed::handle_key)
            .add_observer(controls::navigate)
            .add_plugins(dialogs::NativeDialogsPlugin)
            .add_plugins(accessibility::NativeAccessibilityPlugin)
            .add_systems(
                PostUpdate,
                (
                    palette::sync_scrollbar.after(bevy::ui::UiSystems::Layout),
                    theme::apply.after(bevy::a11y::AccessibilitySystems::Update),
                    speed::edit
                        .after(bevy::text::EditableTextSystems)
                        .before(bevy::ui::UiSystems::PostLayout),
                ),
            )
            .add_systems(Startup, setup.after(StartupSet::BuildWorld))
            .add_systems(
                Update,
                (
                    dialogs::update_dialogs,
                    menus::dismiss,
                    sync,
                    sync_palette,
                    palette::filter,
                    palette::sync_preview,
                    controls::sync_speed,
                    speed::capture_scrub,
                    speed::sync_scrubber,
                    speed::sync_chevrons,
                    controls::sync_overlays,
                    controls::sync_tool_hint,
                    controls::sync_footer,
                    toolbar::sync_layout,
                    controls::sync_feedback,
                    publish_input_regions,
                    release_pointer_focus,
                    sync_board_focus,
                )
                    .chain()
                    .in_set(NativeLayout)
                    .before(UpdateSet::Camera)
                    .before(UiInputSet::Capture),
            );
    }
}

#[derive(Component)]
struct NativeRoot;
#[derive(Component)]
struct Sidebar;
#[derive(Component)]
struct ToolStrip;
#[derive(Component)]
struct Palette;
#[derive(Component)]
struct Footer;
#[derive(Component)]
struct FooterStatus;
#[derive(Component)]
struct BoardArea;
#[derive(Component)]
struct BoardFocusRing;
#[derive(Component)]
enum Readout {
    Status,
    Generation,
    Population,
    Rule,
    Board,
}

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Play,
    Step,
    Undo,
    Redo,
    Grid,
    CellAge,
    FitBoard,
    FitLiveCells,
    ZoomIn,
    ZoomOut,
    MaxSpeed,
    SpeedAdjust(i8),
    Sidebar,
    Import,
    Help,
    Settings,
    ViewMenu,
    MoreMenu,
    Diagnostics,
    CloseDialog,
    ImportSubmit,
    ResizeBoard,
    BoardPreset(usize),
    /// Index into the rule presets of the settings dialog.
    RulePreset(usize),
    Theme(Theme),
    ApplyRule,
    ExportRle,
    ResetEpoch,
    Tool(EditTool),
    Pattern(usize),
    Rotate,
    Flip,
    Randomize,
    Clear,
    Boundary,
}

fn label(value: impl Into<String>, size: f32, color: Token) -> impl Bundle {
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

fn row() -> impl Bundle {
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

fn button(parent: &mut ChildSpawnerCommands, text: &str, action: Action) -> Entity {
    parent
        .spawn((
            Node {
                padding: UiRect::axes(px(12), px(9)),
                width: if matches!(action, Action::Tool(_)) {
                    px(68)
                } else {
                    Val::Auto
                },
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
            Name::new(text.to_owned()),
            controls::Tooltip(controls::hint(action).to_owned()),
        ))
        .with_children(|p| match Icon::of(action) {
            // The name still labels the button for tooltips and screen readers.
            Some(icon) => {
                p.spawn(icons::node(icon));
            }
            None => {
                p.spawn(label(text, 14.0, Token::Ink));
            }
        })
        .id()
}

fn setup(
    mut commands: Commands,
    mut fonts: ResMut<Assets<Font>>,
    mut images: ResMut<Assets<Image>>,
) {
    // Embed the licensed font so launching a desktop bundle works from any directory.
    fonts
        .insert(
            UI_FONT.id(),
            Font::from_bytes(include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf").to_vec()),
        )
        .expect("static UI font handle is valid");
    icons::insert_all(&mut images);
    toolbar::spawn(&mut commands);
    commands
        .spawn((
            NativeRoot,
            ToolStrip,
            TabGroup {
                order: 1,
                ..default()
            },
            Node {
                position_type: PositionType::Absolute,
                left: px(16),
                top: px(toolbar::height(1200.0) + 16.0),
                width: px(SIDE),
                height: px(TOOL_STRIP_HEIGHT),
                padding: UiRect::all(px(4)),
                column_gap: px(6),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::Center,
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(8)),
                ..default()
            },
            ThemedBackground(Token::Panel),
            ThemedBorder(Token::Line),
            accessibility::named_node(accesskit::Role::Toolbar, "Drawing tools"),
            GlobalZIndex(10),
        ))
        .with_children(|p| {
            button(p, "Draw", Action::Tool(EditTool::Draw));
            button(p, "Erase", Action::Tool(EditTool::Erase));
            button(p, "Stamp", Action::Tool(EditTool::Stamp));
        });
    commands
        .spawn((
            NativeRoot,
            Sidebar,
            TabGroup::default(),
            Node {
                position_type: PositionType::Absolute,
                left: px(16),
                top: px(toolbar::height(1200.0) + 16.0 + TOOL_STRIP_SPACE),
                bottom: px(64),
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
            palette::preview(p);
            palette::search(p);
            palette::library(p);
        });
    commands
        .spawn((
            NativeRoot,
            Footer,
            TabGroup::default(),
            Node {
                position_type: PositionType::Absolute,
                left: px(0),
                right: px(0),
                bottom: px(0),
                height: px(48),
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
                    column_gap: px(8),
                    height: px(34),
                    flex_grow: 1.0,
                    flex_shrink: 0.0,
                    ..default()
                },
            ))
            .with_children(|p| {
                for (kind, width) in [
                    (Readout::Status, 68.0),
                    (Readout::Generation, 176.0),
                    (Readout::Population, 104.0),
                    (Readout::Rule, 150.0),
                ] {
                    p.spawn(Node {
                        width: px(width),
                        flex_shrink: 0.0,
                        ..default()
                    })
                    .with_children(|p| {
                        p.spawn((label("", 13.0, Token::Ink), kind))
                            .insert(controls::numeric_font(13.0));
                    });
                }
                p.spawn((label("", 12.0, Token::Muted), Readout::Board));
            });
            p.spawn(row()).with_children(|p| {
                button(p, "−", Action::ZoomOut);
                p.spawn((
                    controls::ZoomLabel,
                    label("100%", 13.0, Token::Ink),
                    controls::Tooltip("Zoom relative to Fit board (100%)".into()),
                    Hovered::default(),
                    Node {
                        width: px(56),
                        flex_shrink: 0.0,
                        ..default()
                    },
                ));
                button(p, "+", Action::ZoomIn);
                button(p, "Fit board", Action::FitBoard);
                button(p, "Fit live cells", Action::FitLiveCells);
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
                order: 2,
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
        });
    controls::overlays(&mut commands);
    controls::feedback_overlay(&mut commands);
}

/// A clicked button gives the keyboard back to the board, so Space keeps
/// meaning play/pause. Tab still focuses buttons, with a visible outline.
fn release_pointer_focus(
    mut focus: ResMut<InputFocus>,
    focus_visible: Res<InputFocusVisible>,
    regions: Res<UiInputRegions>,
    buttons: Query<(), With<Action>>,
) {
    if !focus_visible.0 && !regions.modal && focus.get().is_some_and(|e| buttons.contains(e)) {
        focus.clear();
    }
}

/// Outlines the canvas while keyboard navigation leaves the keyboard on the board.
fn sync_board_focus(
    windows: Query<&Window, With<PrimaryWindow>>,
    layout: Res<UiLayout>,
    keyboard: KeyboardFocus,
    focus_visible: Res<InputFocusVisible>,
    regions: Res<UiInputRegions>,
    mut areas: Query<&mut Node, With<BoardArea>>,
    mut rings: Query<&mut Visibility, With<BoardFocusRing>>,
) {
    let Ok(window) = windows.single() else {
        return;
    };
    let rect = canvas_rect(window.size(), &layout);
    for mut node in &mut areas {
        set_if_neq!(node.left, px(rect.min.x));
        set_if_neq!(node.top, px(rect.min.y));
        set_if_neq!(node.width, px(rect.width()));
        set_if_neq!(node.height, px(rect.height()));
    }
    let ring = if focus_visible.0 && keyboard.on_board() && !regions.modal {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in &mut rings {
        visibility.set_if_neq(ring);
    }
}

/// Logical rectangles shared by layout and input routing; gaps remain canvas.
pub fn panel_rects(size: Vec2, layout: &UiLayout) -> Vec<Rect> {
    let top = toolbar::height(size.x);
    let bottom = bottom_height(size);
    let mut rects = vec![
        Rect::new(0.0, 0.0, size.x, top),
        Rect::new(0.0, (size.y - bottom).max(0.0), size.x, size.y),
        Rect::new(
            16.0,
            top + 16.0,
            16.0 + if layout.control_deck_visible {
                SIDE
            } else {
                TOOL_RAIL_WIDTH
            },
            top + 16.0
                + if layout.control_deck_visible {
                    TOOL_STRIP_HEIGHT
                } else {
                    TOOL_RAIL_HEIGHT
                },
        ),
    ];
    if layout.control_deck_visible && size.y > top + bottom + 32.0 {
        rects.push(Rect::new(
            16.0,
            top + 16.0 + TOOL_STRIP_SPACE,
            SIDE + 16.0,
            size.y - bottom - 16.0,
        ));
    }
    rects
}

/// The board is clipped to this workspace; the UI camera still covers the window.
pub fn canvas_rect(size: Vec2, layout: &UiLayout) -> Rect {
    let left = if layout.control_deck_visible {
        SIDE + 32.0
    } else {
        TOOL_RAIL_WIDTH + 32.0
    };
    let max = (size - Vec2::new(16.0, bottom_height(size) + 16.0)).max(Vec2::ONE);
    let min = Vec2::new(left, toolbar::height(size.x) + 16.0)
        .min(max - Vec2::ONE)
        .max(Vec2::ZERO);
    Rect::from_corners(min, max)
}

fn publish_input_regions(
    windows: Query<&Window, With<PrimaryWindow>>,
    layout: Res<UiLayout>,
    state: Res<NativeUi>,
    menus: Query<&Node, With<menus::WorkspacePopup>>,
    mut regions: ResMut<UiInputRegions>,
) {
    regions.modal =
        state.dialog.is_some() || menus.iter().any(|node| node.display != Display::None);
    regions.panels = windows
        .single()
        .map(|window| panel_rects(window.size(), &layout))
        .unwrap_or_default();
}

/// Board display options that buttons switch on and off.
#[derive(SystemParam)]
struct ViewToggles<'w> {
    grid: ResMut<'w, GridOverlay>,
    cell_age: ResMut<'w, CellAge>,
}

fn activate(
    event: On<Activate>,
    actions: Query<&Action>,
    mut sim: ResMut<Simulation>,
    mut view: ViewToggles,
    mut layout: ResMut<UiLayout>,
    mut feedback: ResMut<EditFeedback>,
    mut theme: ResMut<Theme>,
) {
    let Ok(action) = actions.get(event.entity) else {
        return;
    };
    match action {
        Action::Import
        | Action::FitBoard
        | Action::FitLiveCells
        | Action::ZoomIn
        | Action::ZoomOut
        | Action::Settings
        | Action::Diagnostics
        | Action::Help
        | Action::CloseDialog
        | Action::ImportSubmit
        | Action::ResizeBoard
        | Action::BoardPreset(_)
        | Action::RulePreset(_)
        | Action::ApplyRule
        | Action::ExportRle => {}
        Action::Grid => view.grid.enabled = !view.grid.enabled,
        Action::CellAge => view.cell_age.enabled = !view.cell_age.enabled,
        Action::Sidebar => layout.toggle_control_deck(),
        Action::Theme(choice) => {
            theme.set_if_neq(*choice);
        }
        Action::Undo => feedback.undo(&mut sim.app),
        Action::Redo => feedback.redo(&mut sim.app),
        Action::Clear => feedback.clear(&mut sim.app),
        Action::Randomize => feedback.randomize(&mut sim.app),
        _ => apply_action(*action, &mut sim.app),
    }
}

fn apply_action(action: Action, app: &mut GolApp) {
    match action {
        Action::Play => app.toggle_running(),
        Action::MaxSpeed => {
            if app.is_max_speed() {
                app.set_tick_rate_ms(app.tick_rate_ms());
            } else {
                app.set_max_speed();
            }
        }
        Action::SpeedAdjust(step) => {
            let rate = (controls::fixed_rate(app) * 10.0).round() / 10.0;
            app.set_tick_rate_ms(controls::rate_to_ms(rate + f32::from(step)));
        }
        Action::Step => app.increment_epoch(),
        Action::Undo => {
            app.undo();
        }
        Action::Redo => {
            app.redo();
        }
        Action::Tool(tool) => app.set_tool(tool),
        Action::Pattern(index) => {
            app.select_pattern_index(index);
        }
        Action::Rotate => app.rotate_stamp_clockwise(),
        Action::Flip => app.toggle_stamp_flip(),
        Action::Randomize => app.randomize_board(),
        Action::Clear => app.clear_board(),
        Action::Boundary => app.toggle_boundary_mode(),
        Action::ResetEpoch => app.reset_epoch(),
        _ => {}
    }
}

type ButtonState<'a> = (
    Entity,
    &'a Action,
    &'a Hovered,
    Has<Pressed>,
    &'a mut ThemedBackground,
    &'a mut ThemedBorder,
    &'a Children,
    &'a Name,
    &'a mut bevy::a11y::AccessibilityNode,
);

#[derive(SystemParam)]
struct ViewState<'w, 's> {
    mode: Res<'w, NativeUi>,
    layout: Res<'w, UiLayout>,
    grid: Res<'w, GridOverlay>,
    cell_age: Res<'w, CellAge>,
    focus: Res<'w, InputFocus>,
    focus_visible: Res<'w, InputFocusVisible>,
    theme: Res<'w, Theme>,
    menus: Query<'w, 's, &'static bevy::ui_widgets::MenuFocusState, With<menus::WorkspacePopup>>,
}

type RootNode<'a> = (&'a mut Node, Has<Sidebar>, Has<toolbar::Toolbar>);

fn sync(
    mut commands: Commands,
    sim: Res<Simulation>,
    view: ViewState,
    mut roots: Query<RootNode, With<NativeRoot>>,
    mut buttons: Query<ButtonState>,
    mut texts: Query<(&mut Text, &mut ThemedText, Option<&Readout>)>,
    mut icons: Query<(&mut ImageNode, &mut ThemedImage)>,
) {
    let ViewState {
        mode: _,
        layout,
        grid,
        cell_age,
        focus,
        focus_visible: _,
        theme,
        menus: _,
    } = view;
    for (mut node, sidebar, toolbar) in &mut roots {
        let display = if sidebar && !layout.control_deck_visible {
            Display::None
        } else if toolbar {
            Display::Grid
        } else {
            Display::Flex
        };
        set_if_neq!(node.display, display);
    }
    for (entity, action, hovered, pressed, mut bg, mut border, children, name, mut accessible) in
        &mut buttons
    {
        let accessible_label = match action {
            Action::Play => {
                if sim.app.is_running() {
                    "Pause"
                } else {
                    "Play"
                }
            }
            _ => name.as_str(),
        };
        let accessible_label = if *action == Action::Boundary {
            format!("Edges: {}", sim.app.boundary_mode_label())
        } else {
            accessible_label.to_owned()
        };
        if accessible.label() != Some(accessible_label.as_str()) {
            accessible.set_label(accessible_label);
        }
        let disabled = match action {
            Action::Undo => !sim.app.can_undo(),
            Action::Redo => !sim.app.can_redo(),
            Action::Rotate | Action::Flip => sim.app.tool() != EditTool::Stamp,
            Action::SpeedAdjust(step) if *step < 0 => sim.app.tick_rate_ms() >= 1000,
            Action::SpeedAdjust(_) => sim.app.tick_rate_ms() <= 25,
            _ => false,
        };
        if disabled {
            commands.entity(entity).insert_if_new(InteractionDisabled);
        }
        if !disabled {
            commands.entity(entity).remove::<InteractionDisabled>();
        }
        let selected = match action {
            Action::Play => true,
            Action::Tool(tool) => sim.app.tool() == *tool,
            Action::Pattern(index) => {
                sim.app.tool() == EditTool::Stamp
                    && sim.app.selected_pattern_index() == Some(*index)
            }
            Action::Grid => grid.enabled,
            Action::CellAge => cell_age.enabled,
            Action::Sidebar => layout.control_deck_visible,
            Action::MaxSpeed => sim.app.is_max_speed(),
            Action::Theme(choice) => *theme == *choice,
            Action::RulePreset(index) => dialogs::RULE_PRESETS
                .get(*index)
                .is_some_and(|(_, rule)| sim.app.rule_label() == *rule),
            _ => false,
        };
        if matches!(
            action,
            Action::Tool(_)
                | Action::Pattern(_)
                | Action::Grid
                | Action::CellAge
                | Action::Sidebar
                | Action::MaxSpeed
                | Action::RulePreset(_)
                | Action::Theme(_)
        ) {
            accessible.set_toggled(if selected {
                accesskit::Toggled::True
            } else {
                accesskit::Toggled::False
            });
        }
        bg.set_if_neq(ThemedBackground(if disabled {
            Token::Panel
        } else if *action == Action::Play {
            if pressed {
                Token::AccentPressed
            } else {
                Token::Accent
            }
        } else if pressed || hovered.get() {
            Token::Hover
        } else if selected {
            Token::Selected
        } else if matches!(action, Action::SpeedAdjust(_)) {
            Token::Field
        } else {
            Token::Surface
        }));
        border.set_if_neq(ThemedBorder(if focus.get() == Some(entity) {
            Token::Focus
        } else if selected {
            Token::Accent
        } else {
            Token::Line
        }));
        let ink = if disabled {
            Token::Muted
        } else if *action == Action::Play {
            Token::OnAccent
        } else {
            Token::Ink
        };
        for child in children.iter() {
            if let Ok((mut image, mut color)) = icons.get_mut(child) {
                color.set_if_neq(ThemedImage(ink));
                if *action == Action::Play {
                    let icon = if sim.app.is_running() {
                        Icon::Pause
                    } else {
                        Icon::Play
                    };
                    set_if_neq!(image.image, icon.handle());
                }
            }
            if let Ok((mut text, mut color, _)) = texts.get_mut(child) {
                color.set_if_neq(ThemedText(ink));
                let replacement = match action {
                    Action::Play => Some(if sim.app.is_running() {
                        "Pause".to_string()
                    } else {
                        "Play".to_string()
                    }),
                    Action::Boundary => Some(format!("Edges: {}", sim.app.boundary_mode_label())),
                    _ => None,
                };
                if let Some(value) = replacement
                    && text.0 != value
                {
                    text.0 = value;
                }
            }
        }
    }
    for (mut text, _, readout) in &mut texts {
        let value = match readout {
            Some(Readout::Status) => if sim.app.is_running() {
                "RUNNING"
            } else {
                "PAUSED"
            }
            .to_owned(),
            Some(Readout::Generation) => format!("Gen {}", sim.app.epoch),
            Some(Readout::Population) => format!("{} alive", sim.app.board.live_cells_count()),
            Some(Readout::Rule) => sim.app.rule_label(),
            Some(Readout::Board) => format!(
                "{} × {} cells   ·   {} edges",
                sim.app.board.size().0,
                sim.app.board.size().1,
                sim.app.boundary_mode_label()
            ),
            None => continue,
        };
        if text.0 != value {
            text.0 = value;
        }
    }
}

fn sync_palette(
    mut commands: Commands,
    sim: Res<Simulation>,
    theme: Res<Theme>,
    palette: Query<Entity, With<Palette>>,
    mut images: ResMut<Assets<Image>>,
    mut count: Local<usize>,
) {
    // Thumbnails are drawn in board colours, so a new theme redraws the library.
    if *count == sim.app.pattern_count() && !theme.is_changed() {
        return;
    }
    let Ok(entity) = palette.single() else {
        return;
    };
    commands
        .entity(entity)
        .despawn_children()
        .with_children(|p| {
            let builtin_count = crate::patterns::Pattern::builtin_count();
            for index in 0..sim.app.pattern_count() {
                if index == 0 || index == builtin_count {
                    p.spawn((
                        palette::PatternGroup(index >= builtin_count),
                        label(
                            if index == 0 { "BUILTIN" } else { "IMPORTED" },
                            11.0,
                            Token::Muted,
                        ),
                    ));
                }
                let Some(pattern) = sim.app.pattern_at(index) else {
                    continue;
                };
                let preview = images.add(thumbnail(
                    &pattern.cells,
                    pattern.width,
                    pattern.height,
                    theme.board(),
                ));
                p.spawn((
                    WidgetButton,
                    Action::Pattern(index),
                    controls::Tooltip(controls::pattern_hint(&pattern.name, index)),
                    Hovered::default(),
                    TabIndex(0),
                    Name::new(pattern.name.clone()),
                    ThemedBackground(Token::Surface),
                    ThemedBorder(Token::Line),
                    Node {
                        width: percent(100),
                        min_height: px(66),
                        padding: UiRect::all(px(8)),
                        column_gap: px(10),
                        align_items: AlignItems::Center,
                        border: UiRect::all(px(1)),
                        border_radius: BorderRadius::all(px(8)),
                        flex_shrink: 0.0,
                        ..default()
                    },
                ))
                .with_children(|p| {
                    p.spawn((
                        ImageNode::new(preview),
                        Node {
                            width: px(44),
                            height: px(44),
                            flex_shrink: 0.0,
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    p.spawn((
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: px(5),
                            min_width: px(0),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ))
                    .with_children(|p| {
                        p.spawn(label(&pattern.name, 15.0, Token::Ink));
                        p.spawn(label(
                            format!(
                                "{} × {} · {} cells",
                                pattern.width,
                                pattern.height,
                                pattern.cells.len()
                            ),
                            11.0,
                            Token::Muted,
                        ));
                    });
                });
            }
        });
    *count = sim.app.pattern_count();
}

/// A bounded thumbnail keeps large imported patterns from spawning thousands of UI entities.
fn thumbnail(
    cells: &[(usize, usize)],
    width: usize,
    height: usize,
    colors: &crate::shared::BoardColors,
) -> Image {
    let live = crate::render_plugin::color_to_texel(colors.live);
    let mut image = Image::new_fill(
        Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &crate::render_plugin::color_to_texel(colors.dead),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    let scale = (52.0 / width.max(height).max(1) as f32).min(10.0);
    let x_pad = (64.0 - width as f32 * scale) / 2.0;
    let y_pad = (64.0 - height as f32 * scale) / 2.0;
    if let Some(data) = &mut image.data {
        for &(x, y) in cells {
            let left = (x_pad + x as f32 * scale) as usize;
            let top = (y_pad + height.saturating_sub(y + 1) as f32 * scale) as usize;
            for py in top..(top + (scale - 1.0).max(1.0) as usize).min(64) {
                for px in left..(left + (scale - 1.0).max(1.0) as usize).min(64) {
                    data[(py * 64 + px) * 4..(py * 64 + px) * 4 + 4].copy_from_slice(&live);
                }
            }
        }
    }
    image
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicked_buttons_release_focus_but_keyboard_and_modal_focus_stay() {
        use bevy::{ecs::system::RunSystemOnce, input_focus::FocusCause};
        let mut world = World::new();
        world.init_resource::<InputFocus>();
        world.init_resource::<InputFocusVisible>();
        world.init_resource::<UiInputRegions>();
        let button = world.spawn(Action::Randomize).id();
        let field = world.spawn_empty().id();
        let focused = |world: &mut World, entity, visible, modal| {
            world
                .resource_mut::<InputFocus>()
                .set(entity, FocusCause::Navigated);
            world.resource_mut::<InputFocusVisible>().0 = visible;
            world.resource_mut::<UiInputRegions>().modal = modal;
            world.run_system_once(release_pointer_focus).unwrap();
            world.resource::<InputFocus>().get()
        };
        assert_eq!(focused(&mut world, button, false, false), None);
        assert_eq!(focused(&mut world, button, true, false), Some(button));
        assert_eq!(focused(&mut world, button, false, true), Some(button));
        assert_eq!(focused(&mut world, field, false, false), Some(field));
    }

    #[test]
    fn board_outline_follows_keyboard_focus_on_the_board() {
        use bevy::{
            ecs::system::RunSystemOnce,
            input_focus::{
                FocusCause,
                tab_navigation::{NavAction, TabNavigation},
            },
        };
        let mut world = World::new();
        world.init_resource::<Assets<Font>>();
        world.init_resource::<Assets<Image>>();
        world.init_resource::<UiLayout>();
        world.init_resource::<InputFocus>();
        world.init_resource::<InputFocusVisible>();
        world.init_resource::<UiInputRegions>();
        let window = world.spawn((Window::default(), PrimaryWindow)).id();
        world.run_system_once(setup).unwrap();
        let board = world
            .query_filtered::<Entity, With<BoardFocus>>()
            .single(&world)
            .unwrap();
        let button = world
            .query::<(Entity, &Action)>()
            .iter(&world)
            .find_map(|(e, a)| (*a == Action::Play).then_some(e))
            .unwrap();
        let outlined = |world: &mut World, focus: Option<Entity>, visible, modal| {
            match focus {
                Some(e) => world
                    .resource_mut::<InputFocus>()
                    .set(e, FocusCause::Navigated),
                None => world.resource_mut::<InputFocus>().clear(),
            }
            world.resource_mut::<InputFocusVisible>().0 = visible;
            world.resource_mut::<UiInputRegions>().modal = modal;
            world.run_system_once(sync_board_focus).unwrap();
            let mut rings = world.query_filtered::<&Visibility, With<BoardFocusRing>>();
            *rings.single(world).unwrap() == Visibility::Inherited
        };
        // Visibility propagation does not run here; Bevy's tab walker skips hidden nodes.
        for mut visibility in world
            .query::<&mut InheritedVisibility>()
            .iter_mut(&mut world)
        {
            *visibility = InheritedVisibility::VISIBLE;
        }
        let last = world
            .run_system_once(|nav: TabNavigation, focus: Res<InputFocus>| {
                nav.navigate(&focus, NavAction::Previous).unwrap()
            })
            .unwrap();
        assert_eq!(
            last, board,
            "Shift+Tab from the board's start reaches the board"
        );
        assert!(outlined(&mut world, Some(board), true, false));
        assert!(outlined(&mut world, Some(window), true, false));
        assert!(outlined(&mut world, None, true, false));
        assert!(!outlined(&mut world, Some(board), false, false));
        assert!(!outlined(&mut world, Some(board), true, true));
        assert!(!outlined(&mut world, Some(button), true, false));
        let rect = canvas_rect(
            world.get::<Window>(window).unwrap().size(),
            &UiLayout::default(),
        );
        let mut areas = world.query_filtered::<&Node, With<BoardArea>>();
        let node = areas.single(&world).unwrap();
        assert_eq!((node.left, node.width), (px(rect.min.x), px(rect.width())));
    }

    #[test]
    fn native_widget_bundles_spawn_together() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Assets<Font>>();
        world.init_resource::<Assets<Image>>();
        world.run_system_once(setup).unwrap();
        assert_eq!(
            world
                .query_filtered::<Entity, With<palette::PatternSearch>>()
                .iter(&world)
                .count(),
            1
        );
        assert_eq!(
            world
                .query_filtered::<Entity, With<speed::SpeedEditor>>()
                .iter(&world)
                .count(),
            1
        );
    }

    #[test]
    fn unchanged_frames_leave_button_render_state_untouched() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Assets<Font>>();
        world.init_resource::<Assets<Image>>();
        world.insert_resource(Simulation::new(GolApp::new(8, 8)));
        world.init_resource::<NativeUi>();
        world.init_resource::<UiLayout>();
        world.insert_resource(GridOverlay { enabled: false });
        world.init_resource::<CellAge>();
        world.init_resource::<InputFocus>();
        world.init_resource::<InputFocusVisible>();
        world.init_resource::<Theme>();
        world.run_system_once(setup).unwrap();
        world.run_system_once(sync).unwrap();
        world.run_system_once(theme::apply).unwrap();
        let ticks = |world: &mut World| {
            let mut ticks = Vec::new();
            ticks.extend(
                world
                    .query_filtered::<(Ref<ThemedBackground>, Ref<ThemedBorder>), With<Action>>()
                    .iter(world)
                    .flat_map(|(bg, border)| [bg.last_changed(), border.last_changed()]),
            );
            ticks.extend(
                world
                    .query_filtered::<(Ref<BackgroundColor>, Ref<BorderColor>), With<Action>>()
                    .iter(world)
                    .flat_map(|(bg, border)| [bg.last_changed(), border.last_changed()]),
            );
            ticks.extend(
                world
                    .query::<(Ref<ThemedText>, Ref<TextColor>)>()
                    .iter(world)
                    .flat_map(|(token, color)| [token.last_changed(), color.last_changed()]),
            );
            ticks
        };
        let before = ticks(&mut world);
        assert!(!before.is_empty());
        world.run_system_once(sync).unwrap();
        world.run_system_once(theme::apply).unwrap();
        assert_eq!(ticks(&mut world), before);
    }

    #[test]
    fn hiding_the_palette_keeps_tools_visible_and_reflows_without_a_resize() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Assets<Font>>();
        world.init_resource::<Assets<Image>>();
        world.insert_resource(Simulation::new(GolApp::new(8, 8)));
        world.init_resource::<NativeUi>();
        world.init_resource::<UiLayout>();
        world.insert_resource(GridOverlay { enabled: false });
        world.init_resource::<CellAge>();
        world.init_resource::<InputFocus>();
        world.init_resource::<InputFocusVisible>();
        world.init_resource::<Theme>();
        world.spawn((Window::default(), PrimaryWindow));
        world.run_system_once(setup).unwrap();
        let mut schedule = Schedule::default();
        schedule.add_systems((sync, toolbar::sync_layout).chain());
        schedule.run(&mut world);
        world.resource_mut::<UiLayout>().control_deck_visible = false;
        schedule.run(&mut world);
        let (_, tools) = world
            .query_filtered::<(&Children, &Node), With<ToolStrip>>()
            .single(&world)
            .unwrap();
        assert_eq!(tools.display, Display::Flex);
        assert_eq!(tools.flex_direction, FlexDirection::Column);
        assert_eq!(tools.width, px(TOOL_RAIL_WIDTH));
        assert_eq!(
            world
                .query_filtered::<&Node, With<Sidebar>>()
                .single(&world)
                .unwrap()
                .display,
            Display::None
        );
        world.resource_mut::<UiLayout>().control_deck_visible = true;
        schedule.run(&mut world);
        let tools = world
            .query_filtered::<&Node, With<ToolStrip>>()
            .single(&world)
            .unwrap();
        assert_eq!(tools.flex_direction, FlexDirection::Row);
        assert_eq!(tools.width, px(SIDE));
    }

    #[test]
    fn native_actions_share_edit_history_and_playback() {
        let mut app = GolApp::new(30, 30);
        let glider = crate::app::PatternKind::Glider.builtin_index().unwrap();
        apply_action(Action::Pattern(glider), &mut app);
        app.place_selected_pattern_at(10, 10);
        assert_eq!(app.board.live_cells_count(), 5);
        apply_action(Action::Undo, &mut app);
        assert_eq!(app.board.live_cells_count(), 0);
        apply_action(Action::Redo, &mut app);
        assert_eq!(app.board.live_cells_count(), 5);
        apply_action(Action::Play, &mut app);
        assert!(app.is_running());
    }

    #[test]
    fn speed_buttons_step_the_displayed_rate_with_limits_and_leave_max_mode() {
        let mut app = GolApp::new(8, 8);
        app.set_tick_rate_ms(100);
        app.set_max_speed();
        apply_action(Action::SpeedAdjust(1), &mut app);
        assert!(!app.is_max_speed());
        assert_eq!(app.tick_rate_ms(), controls::rate_to_ms(11.0));
        apply_action(Action::SpeedAdjust(-1), &mut app);
        assert_eq!(app.tick_rate_ms(), 100);
        app.set_tick_rate_ms(80);
        apply_action(Action::SpeedAdjust(1), &mut app);
        assert_eq!(app.tick_rate_ms(), controls::rate_to_ms(13.5));
        app.set_tick_rate_ms(1000);
        apply_action(Action::SpeedAdjust(-1), &mut app);
        assert_eq!(app.tick_rate_ms(), 1000);
        app.set_tick_rate_ms(25);
        apply_action(Action::SpeedAdjust(1), &mut app);
        assert_eq!(app.tick_rate_ms(), 25);
        assert_eq!(app.board.live_cells_count(), 0);
    }

    #[test]
    fn compact_toolbar_returns_space_to_the_canvas_and_input_regions_follow_it() {
        let layout = UiLayout::default();
        let narrow = Vec2::new(toolbar::SINGLE_ROW_MIN_WIDTH - 1.0, 800.0);
        let wide = Vec2::new(toolbar::SINGLE_ROW_MIN_WIDTH, 800.0);
        assert_eq!(
            canvas_rect(narrow, &layout).min.y - canvas_rect(wide, &layout).min.y,
            48.0
        );
        let returned_space = Vec2::new(500.0, 100.0);
        assert!(
            panel_rects(narrow, &layout)
                .iter()
                .any(|r| r.contains(returned_space))
        );
        assert!(
            !panel_rects(wide, &layout)
                .iter()
                .any(|r| r.contains(returned_space))
        );
        assert!(canvas_rect(wide, &layout).contains(returned_space));
        for width in [680.0, 999.0, 1000.0, 1039.0, 1040.0, 1200.0, 1600.0] {
            for visible in [true, false] {
                let size = Vec2::new(width, 420.0);
                let layout = UiLayout {
                    control_deck_visible: visible,
                };
                let canvas = canvas_rect(size, &layout);
                for panel in panel_rects(size, &layout) {
                    assert!(canvas.intersect(panel).is_empty());
                }
            }
        }
    }

    #[test]
    fn chrome_captures_clicks_but_canvas_and_hidden_palette_do_not() {
        let size = Vec2::new(768.0, 600.0);
        let contains = |sidebar, pos| {
            panel_rects(
                size,
                &UiLayout {
                    control_deck_visible: sidebar,
                },
            )
            .iter()
            .any(|r| r.contains(pos))
        };
        assert!(contains(true, Vec2::new(300.0, 80.0)));
        assert!(contains(true, Vec2::new(40.0, 150.0)));
        assert!(contains(false, Vec2::new(40.0, 150.0)));
        assert!(contains(true, Vec2::new(40.0, 210.0)));
        assert!(contains(true, Vec2::new(500.0, 590.0)));
        assert!(!contains(true, Vec2::new(400.0, 300.0)));
        assert!(contains(false, Vec2::new(40.0, 210.0)));
        assert!(!contains(false, Vec2::new(40.0, 300.0)));
    }
}
