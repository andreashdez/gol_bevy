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

/// The status bar fits on one row at every window width the app allows.
const STATUS_BAR_HEIGHT: f32 = 48.0;

/// Tab follows the layout: the toolbar's groups (negative orders), then the
/// tools, the palette, the status bar, and the board last.
const TAB_ORDER_TOOLS: i32 = 1;
const TAB_ORDER_PALETTE: i32 = 2;
const TAB_ORDER_STATUS_BAR: i32 = 3;
const TAB_ORDER_BOARD: i32 = 4;
const SIDE: f32 = 256.0;
const TOOL_STRIP_HEIGHT: f32 = 48.0;
const TOOL_STRIP_SPACE: f32 = TOOL_STRIP_HEIGHT + 8.0;
/// The hidden palette's rail of icon-only tools: one 44 px button wide, and
/// three 38 px buttons tall, with the strip's padding and border.
const TOOL_RAIL_WIDTH: f32 = 56.0;
const TOOL_RAIL_HEIGHT: f32 = 136.0;
/// Library rows are short, with a wide thumbnail, since most patterns are wider
/// than they are tall. The texture has two texels per point, for Retina displays.
const ROW_THUMBNAIL: Vec2 = Vec2::new(64.0, 32.0);
const ROW_THUMBNAIL_TEXELS: UVec2 = UVec2::new(128, 64);
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
            .init_resource::<toolbar::WorkspaceMenu>()
            .add_plugins((TabNavigationPlugin, bevy::picking::cursor::CursorIconPlugin))
            .add_observer(activate)
            .add_observer(toolbar::menu_action)
            .add_observer(speed::handle_key)
            .add_observer(controls::navigate)
            .add_plugins(dialogs::NativeDialogsPlugin)
            .add_plugins(accessibility::NativeAccessibilityPlugin)
            .add_systems(
                PostUpdate,
                (
                    palette::sync_scrollbar.after(bevy::ui::UiSystems::Layout),
                    controls::keep_widest.after(bevy::ui::UiSystems::Layout),
                    theme::apply.after(bevy::a11y::AccessibilitySystems::Update),
                    theme::apply_window,
                    speed::edit
                        .after(bevy::text::EditableTextSystems)
                        .before(bevy::ui::UiSystems::PostLayout),
                    speed::sync_fill.before(bevy::ui::UiSystems::Layout),
                ),
            )
            .add_systems(Startup, setup.after(StartupSet::BuildWorld))
            .add_systems(
                Update,
                (
                    dialogs::file_shortcuts,
                    dialogs::sync_switches,
                    dialogs::sync_density,
                    dialogs::sync_import_preview,
                )
                    .after(dialogs::update_dialogs)
                    .in_set(NativeLayout),
            )
            .add_systems(
                Update,
                (
                    // First, so the rest of the layout sees the palette's state this frame.
                    toolbar::collapse_palette,
                    dialogs::poll_file_dialog,
                    dialogs::update_dialogs,
                    sync,
                    sync_palette,
                    palette::filter,
                    palette::sync_preview,
                    speed::capture_scrub,
                    speed::sync_scrubber,
                    speed::sync_chevrons,
                    controls::sync_overlays,
                    controls::sync_footer,
                    controls::sync_status,
                    toolbar::sync_layout,
                    toolbar::sync_menu,
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
#[derive(Component, Clone, Copy, PartialEq, Eq)]
enum Readout {
    Status,
    Generation,
    Population,
    /// How the latest generation changed the population, or how the board settled.
    Change,
    /// The cell under the pointer.
    Pointer,
    Board,
}
/// Green while running, neutral while paused.
#[derive(Component)]
struct StatusPill;

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
enum Action {
    Play,
    /// Returns to the board before the run.
    ResetToStart,
    Step,
    Undo,
    Redo,
    Grid,
    CellAge,
    FitBoard,
    ZoomIn,
    ZoomOut,
    SpeedAdjust(i8),
    /// Toggles ∞, maximum speed.
    MaxSpeed,
    Sidebar,
    Import,
    Export,
    Help,
    Settings,
    /// Opens the menu holding the workspace controls in narrow windows.
    More,
    Diagnostics,
    CloseDialog,
    ImportSubmit,
    ResizeBoard,
    BoardPreset(usize),
    /// Keep the board size matched to the canvas.
    FillCanvas,
    /// Index into the rule presets of the settings dialog.
    RulePreset(usize),
    Theme(Theme),
    ApplyRule,
    /// Opens Settings with the rulestring field focused.
    EditRule,
    ExportRle,
    SaveWorkspace,
    OpenWorkspace,
    /// Loads an `.rle` file into the import fields.
    OpenRle,
    ConfirmOpenWorkspace,
    CancelOpenWorkspace,
    ResetEpoch,
    PauseWhenSettled,
    ApplyRandomize,
    RandomizeNow,
    Tool(EditTool),
    Pattern(usize),
    Rotate,
    Flip,
    Randomize,
    Clear,
    /// Chooses wrapping or fixed edges.
    Edges(crate::board::BoundaryMode),
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
    let entity = button_frame(parent, text, action);
    parent
        .commands()
        .entity(entity)
        .with_child(label(text, 14.0, Token::Ink));
    entity
}

/// A button that shows an icon; `name` still labels it for tooltips and screen readers.
fn icon_button(
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

/// The tool's icon, shown only in the rail the tools become when the palette is hidden.
#[derive(Component)]
struct ToolIcon;
/// The tool's name, shown only beside the palette.
#[derive(Component)]
struct ToolLabel;

fn tool_button(parent: &mut ChildSpawnerCommands, name: &str, icon: Icon, tool: EditTool) {
    let entity = button_frame(parent, name, Action::Tool(tool));
    parent.commands().entity(entity).with_children(|p| {
        p.spawn((ToolIcon, icons::node(icon)));
        p.spawn((ToolLabel, label(name, 14.0, Token::Ink)));
    });
}

fn button_frame(parent: &mut ChildSpawnerCommands, name: &str, action: Action) -> Entity {
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
            Name::new(name.to_owned()),
            controls::Tooltip(controls::hint(action).to_owned()),
        ))
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
                order: TAB_ORDER_TOOLS,
                ..default()
            },
            Node {
                position_type: PositionType::Absolute,
                left: px(16),
                top: px(toolbar::HEIGHT + 16.0),
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
            tool_button(p, "Draw", Icon::Draw, EditTool::Draw);
            tool_button(p, "Erase", Icon::Erase, EditTool::Erase);
            tool_button(p, "Stamp", Icon::Stamp, EditTool::Stamp);
        });
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
                top: px(toolbar::HEIGHT + 16.0 + TOOL_STRIP_SPACE),
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
                });
                button(p, "Rule", Action::EditRule);
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
    let top = toolbar::HEIGHT;
    let bottom = STATUS_BAR_HEIGHT;
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
    // The palette keeps its 16 px window margin; the canvas only needs a thin
    // gap, so the board gets as much of the window as possible.
    const GAP: f32 = 8.0;
    let left = 16.0
        + GAP
        + if layout.control_deck_visible {
            SIDE
        } else {
            TOOL_RAIL_WIDTH
        };
    let max = (size - Vec2::new(GAP, STATUS_BAR_HEIGHT + GAP)).max(Vec2::ONE);
    let min = Vec2::new(left, toolbar::HEIGHT + GAP)
        .min(max - Vec2::ONE)
        .max(Vec2::ZERO);
    Rect::from_corners(min, max)
}

fn publish_input_regions(
    windows: Query<&Window, With<PrimaryWindow>>,
    layout: Res<UiLayout>,
    state: Res<NativeUi>,
    menu: Res<toolbar::WorkspaceMenu>,
    mut regions: ResMut<UiInputRegions>,
) {
    // An open menu holds the pointer and keyboard like a dialog, so the click
    // that dismisses it does not also draw on the board.
    regions.modal = state.dialog.is_some() || menu.open;
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
        | Action::Export
        | Action::FitBoard
        | Action::ZoomIn
        | Action::ZoomOut
        | Action::Settings
        | Action::More
        | Action::Diagnostics
        | Action::Help
        | Action::CloseDialog
        | Action::ImportSubmit
        | Action::ResizeBoard
        | Action::BoardPreset(_)
        | Action::FillCanvas
        | Action::RulePreset(_)
        | Action::ApplyRule
        | Action::EditRule
        | Action::ExportRle
        | Action::SaveWorkspace
        | Action::OpenWorkspace
        | Action::OpenRle
        | Action::ConfirmOpenWorkspace
        | Action::CancelOpenWorkspace
        | Action::ApplyRandomize
        | Action::RandomizeNow => {}
        Action::Grid => view.grid.enabled = !view.grid.enabled,
        Action::CellAge => view.cell_age.enabled = !view.cell_age.enabled,
        Action::Sidebar => layout.toggle_control_deck(),
        Action::Theme(choice) => {
            theme.set_if_neq(*choice);
        }
        Action::ResetToStart => feedback.reset_to_start(&mut sim.app),
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
        Action::SpeedAdjust(step) => controls::step_speed(app, step),
        Action::MaxSpeed => controls::toggle_max_speed(app),
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
        Action::Edges(mode) => app.set_boundary_mode(mode),
        Action::ResetEpoch => app.reset_epoch(),
        Action::PauseWhenSettled => app.set_pause_when_settled(!app.pause_when_settled()),
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
    (Has<dialogs::Switch>, Has<dialogs::Segment>),
);

#[derive(SystemParam)]
struct ViewState<'w> {
    mode: Res<'w, NativeUi>,
    layout: Res<'w, UiLayout>,
    grid: Res<'w, GridOverlay>,
    cell_age: Res<'w, CellAge>,
    board: Res<'w, crate::shared::BoardSettings>,
    focus: Res<'w, InputFocus>,
    focus_visible: Res<'w, InputFocusVisible>,
    theme: Res<'w, Theme>,
}

type RootNode<'a> = (&'a mut Node, Has<Sidebar>, Has<toolbar::Toolbar>);

fn sync(
    mut commands: Commands,
    sim: Res<Simulation>,
    view: ViewState,
    mut roots: Query<RootNode, With<NativeRoot>>,
    mut buttons: Query<ButtonState>,
    mut texts: Query<(&mut Text, &mut ThemedText)>,
    mut icons: Query<(&mut ImageNode, &mut ThemedImage)>,
) {
    let ViewState {
        mode: _,
        layout,
        grid,
        cell_age,
        board,
        focus,
        focus_visible: _,
        theme,
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
    for (
        entity,
        action,
        hovered,
        pressed,
        mut bg,
        mut border,
        children,
        name,
        mut accessible,
        (switch, segment),
    ) in &mut buttons
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
        let accessible_label = match action {
            Action::EditRule => format!("Rule: {}", sim.app.rule_label()),
            _ => accessible_label.to_owned(),
        };
        if accessible.label() != Some(accessible_label.as_str()) {
            accessible.set_label(accessible_label);
        }
        let disabled = match action {
            Action::ResetToStart => !sim.app.can_reset_to_start(),
            Action::Undo => !sim.app.can_undo(),
            Action::Redo => !sim.app.can_redo(),
            Action::Rotate | Action::Flip => sim.app.tool() != EditTool::Stamp,
            Action::SpeedAdjust(step) if *step < 0 => {
                !sim.app.is_max_speed() && sim.app.tick_rate_ms() >= 1000
            }
            Action::SpeedAdjust(_) => {
                sim.app.is_max_speed() || controls::fixed_rate(&sim.app) >= controls::MAX_FIXED_RATE
            }
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
            Action::MaxSpeed => sim.app.is_max_speed(),
            Action::CellAge => cell_age.enabled,
            Action::FillCanvas => board.fill_canvas,
            Action::PauseWhenSettled => sim.app.pause_when_settled(),
            Action::Sidebar => layout.control_deck_visible,
            Action::Theme(choice) => *theme == *choice,
            Action::Edges(mode) => sim.app.board.boundary_mode() == *mode,
            Action::RulePreset(index) => dialogs::RULE_PRESETS
                .get(*index)
                .is_some_and(|(_, rule)| sim.app.rule_label() == *rule),
            _ => false,
        };
        if matches!(
            action,
            Action::Tool(_)
                | Action::Pattern(_)
                | Action::MaxSpeed
                | Action::Grid
                | Action::CellAge
                | Action::FillCanvas
                | Action::PauseWhenSettled
                | Action::Sidebar
                | Action::RulePreset(_)
                | Action::Theme(_)
                | Action::Edges(_)
        ) {
            accessible.set_toggled(if selected {
                accesskit::Toggled::True
            } else {
                accesskit::Toggled::False
            });
        }
        // A switch row shows its state in the switch, and a segment sits flush
        // in its group until chosen.
        let resting = if switch {
            Token::Panel
        } else if segment {
            Token::Field
        } else {
            Token::Surface
        };
        bg.set_if_neq(ThemedBackground(
            if matches!(action, Action::SpeedAdjust(_)) && (disabled || !(pressed || hovered.get()))
            {
                // The speed's steps sit on its fill, which shows through until one is hovered.
                Token::Clear
            } else if disabled {
                Token::Panel
            } else if *action == Action::Play || (*action == Action::MaxSpeed && selected) {
                if pressed {
                    Token::AccentPressed
                } else {
                    Token::Accent
                }
            } else if pressed || hovered.get() {
                Token::Hover
            } else if selected && !switch {
                Token::Selected
            } else if matches!(action, Action::ZoomIn | Action::ZoomOut) {
                Token::Field
            } else if *action == Action::MaxSpeed {
                // Off, it sits clear in the speed field.
                Token::Clear
            } else {
                resting
            },
        ));
        border.set_if_neq(ThemedBorder(if focus.get() == Some(entity) {
            Token::Focus
        } else if *action == Action::MaxSpeed {
            // Its left border divides it from the fixed rates, on or off.
            Token::Line
        } else if selected && !switch {
            Token::Accent
        } else if switch || segment {
            resting
        } else {
            Token::Line
        }));
        let ink = if disabled {
            Token::Muted
        } else if *action == Action::Play || (*action == Action::MaxSpeed && selected) {
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
            if let Ok((mut text, mut color)) = texts.get_mut(child) {
                color.set_if_neq(ThemedText(ink));
                let replacement = match action {
                    Action::Play => Some(if sim.app.is_running() {
                        "Pause".to_string()
                    } else {
                        "Play".to_string()
                    }),
                    Action::EditRule => Some(sim.app.rule_label()),
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
}

fn sync_palette(
    mut commands: Commands,
    sim: Res<Simulation>,
    theme: Res<Theme>,
    palette: Query<Entity, With<Palette>>,
    mut images: ResMut<Assets<Image>>,
    mut revision: Local<Option<u64>>,
) {
    // Thumbnails are drawn in board colours, so a new theme redraws the library.
    if *revision == Some(sim.app.library_revision()) && !theme.is_changed() {
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
                            if index == 0 { "Built-in" } else { "Imported" },
                            11.0,
                            Token::Muted,
                        ),
                    ));
                }
                let Some(pattern) = sim.app.pattern_at(index) else {
                    continue;
                };
                let preview = images.add(thumbnail_sized(
                    &pattern.cells,
                    pattern.width,
                    pattern.height,
                    theme.board(),
                    ROW_THUMBNAIL_TEXELS,
                    4.0,
                    14.0,
                ));
                p.spawn((
                    WidgetButton,
                    Action::Pattern(index),
                    controls::Tooltip(controls::pattern_hint(&pattern.name)),
                    Hovered::default(),
                    TabIndex(0),
                    Name::new(pattern.name.clone()),
                    ThemedBackground(Token::Surface),
                    ThemedBorder(Token::Line),
                    Node {
                        width: percent(100),
                        padding: UiRect::axes(px(8), px(6)),
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
                            width: px(ROW_THUMBNAIL.x),
                            height: px(ROW_THUMBNAIL.y),
                            flex_shrink: 0.0,
                            ..default()
                        },
                        Pickable::IGNORE,
                    ));
                    p.spawn((
                        Node {
                            flex_direction: FlexDirection::Column,
                            row_gap: px(3),
                            min_width: px(0),
                            ..default()
                        },
                        Pickable::IGNORE,
                    ))
                    .with_children(|p| {
                        p.spawn(label(&pattern.name, 14.0, Token::Ink));
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
    *revision = Some(sim.app.library_revision());
}

/// A bounded thumbnail keeps large imported patterns from spawning thousands of UI entities.
fn thumbnail(
    cells: &[(usize, usize)],
    width: usize,
    height: usize,
    colors: &crate::shared::BoardColors,
) -> Image {
    thumbnail_sized(cells, width, height, colors, UVec2::splat(64), 6.0, 10.0)
}

/// Draws the pattern centred in a `canvas`-sized texture, at least `margin`
/// texels from its edges, with cells at most `max_cell` texels across.
fn thumbnail_sized(
    cells: &[(usize, usize)],
    width: usize,
    height: usize,
    colors: &crate::shared::BoardColors,
    canvas: UVec2,
    margin: f32,
    max_cell: f32,
) -> Image {
    let live = crate::render_plugin::color_to_texel(colors.live);
    let (canvas_width, canvas_height) = (canvas.x as usize, canvas.y as usize);
    let mut image = Image::new_fill(
        Extent3d {
            width: canvas.x,
            height: canvas.y,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &crate::render_plugin::color_to_texel(colors.dead),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::nearest();
    let scale = ((canvas.x as f32 - 2.0 * margin) / width.max(1) as f32)
        .min((canvas.y as f32 - 2.0 * margin) / height.max(1) as f32)
        .min(max_cell);
    // Leave a one-pixel gap between cells only where cells are big enough to spare it.
    let cell = if scale >= 3.0 {
        (scale - 1.0) as usize
    } else {
        scale.ceil() as usize
    };
    let x_pad = (canvas.x as f32 - width as f32 * scale) / 2.0;
    let y_pad = (canvas.y as f32 - height as f32 * scale) / 2.0;
    if let Some(data) = &mut image.data {
        for &(x, y) in cells {
            let left = (x_pad + x as f32 * scale) as usize;
            let top = (y_pad + height.saturating_sub(y + 1) as f32 * scale) as usize;
            for py in top..(top + cell).min(canvas_height) {
                for px in left..(left + cell).min(canvas_width) {
                    let offset = (py * canvas_width + px) * 4;
                    data[offset..offset + 4].copy_from_slice(&live);
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
    fn opening_a_workspace_rebuilds_a_library_with_the_same_pattern_count() {
        let mut world = World::new();
        let mut app = GolApp::new(16, 16);
        app.import_rle_pattern("Old import", "x = 1, y = 1\no!")
            .unwrap();
        world.insert_resource(Simulation::new(app));
        world.insert_resource(Theme::Dark);
        world.init_resource::<Assets<Image>>();
        world.spawn(Palette);
        let sync = world.register_system(sync_palette);
        world.run_system(sync).unwrap();
        world.clear_trackers();
        let mut other = GolApp::new(16, 16);
        other
            .import_rle_pattern("New import", "x = 2, y = 1\n2o!")
            .unwrap();
        world
            .resource_mut::<Simulation>()
            .app
            .restore_workspace(other.workspace().unwrap())
            .unwrap();
        world.run_system(sync).unwrap();
        let names: Vec<_> = world
            .query::<&Name>()
            .iter(&world)
            .map(|name| name.as_str())
            .collect();
        assert!(names.contains(&"New import"));
        assert!(!names.contains(&"Old import"));
    }

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
    fn tab_follows_the_layout_from_the_toolbar_to_the_board() {
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
        world.spawn((Window::default(), PrimaryWindow));
        world.run_system_once(setup).unwrap();
        // Visibility propagation does not run here; Bevy's tab walker skips hidden nodes.
        for mut visibility in world
            .query::<&mut InheritedVisibility>()
            .iter_mut(&mut world)
        {
            *visibility = InheritedVisibility::VISIBLE;
        }
        let area = |world: &World, entity: Entity| {
            let mut current = Some(entity);
            while let Some(e) = current {
                let found = if world.get::<toolbar::Group>(e).is_some() {
                    Some("toolbar")
                } else if world.get::<ToolStrip>(e).is_some() {
                    Some("tools")
                } else if world.get::<Sidebar>(e).is_some() {
                    Some("palette")
                } else if world.get::<Footer>(e).is_some() {
                    Some("status bar")
                } else if world.get::<BoardArea>(e).is_some() {
                    Some("board")
                } else {
                    None
                };
                if found.is_some() {
                    return found;
                }
                current = world.get::<ChildOf>(e).map(ChildOf::parent);
            }
            None
        };
        let play = world
            .query::<(Entity, &Action)>()
            .iter(&world)
            .find_map(|(e, a)| (*a == Action::Play).then_some(e))
            .unwrap();
        world
            .resource_mut::<InputFocus>()
            .set(play, FocusCause::Navigated);
        let mut visited = vec!["toolbar"];
        while *visited.last().unwrap() != "board" {
            let next = world
                .run_system_once(|nav: TabNavigation, focus: Res<InputFocus>| {
                    nav.navigate(&focus, NavAction::Next).unwrap()
                })
                .unwrap();
            world
                .resource_mut::<InputFocus>()
                .set(next, FocusCause::Navigated);
            let here = area(&world, next).expect("every Tab stop is in a known area");
            if *visited.last().unwrap() != here {
                assert!(!visited.contains(&here), "Tab returned to the {here}");
                visited.push(here);
            }
        }
        assert_eq!(
            visited,
            ["toolbar", "tools", "palette", "status bar", "board"]
        );
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
        world.init_resource::<crate::shared::BoardSettings>();
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
        world.init_resource::<crate::shared::BoardSettings>();
        world.init_resource::<InputFocus>();
        world.init_resource::<InputFocusVisible>();
        world.init_resource::<Theme>();
        world.init_resource::<toolbar::WorkspaceMenu>();
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
    fn narrow_windows_move_workspace_controls_into_a_menu() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Assets<Font>>();
        world.init_resource::<Assets<Image>>();
        world.insert_resource(Simulation::new(GolApp::new(8, 8)));
        world.init_resource::<NativeUi>();
        world.init_resource::<UiLayout>();
        world.init_resource::<toolbar::WorkspaceMenu>();
        world.init_resource::<InputFocus>();
        world.init_resource::<InputFocusVisible>();
        world.init_resource::<ButtonInput<KeyCode>>();
        world.init_resource::<ButtonInput<MouseButton>>();
        world.init_resource::<UiInputRegions>();
        world.add_observer(toolbar::menu_action);
        let window = world
            .spawn((
                Window {
                    resolution: bevy::window::WindowResolution::new(
                        toolbar::COLLAPSE_BELOW_WIDTH as u32 - 1,
                        600,
                    ),
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();
        world.run_system_once(setup).unwrap();
        let mut schedule = Schedule::default();
        schedule.add_systems((toolbar::sync_layout, toolbar::sync_menu).chain());
        let find = |world: &mut World, action: Action| {
            world
                .query::<(Entity, &Action)>()
                .iter(world)
                .find_map(|(e, a)| (*a == action).then_some(e))
                .unwrap()
        };
        let more = find(&mut world, Action::More);
        let grid = find(&mut world, Action::Grid);
        let panel_shown = |world: &mut World| {
            let (node, visibility) = world
                .query_filtered::<(&Node, &Visibility), With<toolbar::MenuPanel>>()
                .single(world)
                .unwrap();
            assert_eq!(
                node.display != Display::None,
                *visibility != Visibility::Hidden
            );
            node.display != Display::None
        };
        schedule.run(&mut world);
        assert!(world.resource::<toolbar::WorkspaceMenu>().collapsed);
        assert_ne!(world.get::<Node>(more).unwrap().display, Display::None);
        assert!(!panel_shown(&mut world));

        // From the keyboard, More opens the menu on its first item.
        world.resource_mut::<InputFocusVisible>().0 = true;
        world.trigger(Activate { entity: more });
        schedule.run(&mut world);
        assert!(panel_shown(&mut world));
        world.run_system_once(publish_input_regions).unwrap();
        assert!(world.resource::<UiInputRegions>().modal);
        assert_eq!(world.resource::<InputFocus>().get(), Some(grid));

        // Escape closes it and returns focus to More.
        world
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        schedule.run(&mut world);
        world.resource_mut::<ButtonInput<KeyCode>>().clear();
        assert!(!panel_shown(&mut world));
        assert_eq!(world.resource::<InputFocus>().get(), Some(more));

        // Choosing an item closes the menu.
        world.trigger(Activate { entity: more });
        schedule.run(&mut world);
        assert!(panel_shown(&mut world));
        world.trigger(Activate { entity: grid });
        schedule.run(&mut world);
        assert!(!panel_shown(&mut world));

        // Wide windows show the controls in the toolbar and hide More.
        world.trigger(Activate { entity: more });
        world
            .get_mut::<Window>(window)
            .unwrap()
            .resolution
            .set(toolbar::COLLAPSE_BELOW_WIDTH, 600.0);
        schedule.run(&mut world);
        let menu = world.resource::<toolbar::WorkspaceMenu>();
        assert!(!menu.collapsed && !menu.open);
        assert!(panel_shown(&mut world));
        assert_eq!(world.get::<Node>(more).unwrap().display, Display::None);
    }

    #[test]
    fn the_palette_collapses_only_as_the_window_crosses_the_breakpoint() {
        let mut world = World::new();
        world.init_resource::<UiLayout>();
        let window = world
            .spawn((
                Window {
                    resolution: bevy::window::WindowResolution::new(
                        toolbar::PALETTE_COLLAPSE_BELOW_WIDTH as u32 - 1,
                        600,
                    ),
                    ..default()
                },
                PrimaryWindow,
            ))
            .id();
        let collapse = world.register_system(toolbar::collapse_palette);
        let visible = |world: &World| world.resource::<UiLayout>().control_deck_visible;
        world.run_system(collapse).unwrap();
        assert!(!visible(&world), "a narrow first window starts collapsed");

        // Shown by hand while narrow, it is not hidden again on the next frame.
        world.resource_mut::<UiLayout>().toggle_control_deck();
        world.run_system(collapse).unwrap();
        assert!(visible(&world));

        world.resource_mut::<UiLayout>().toggle_control_deck();
        world
            .get_mut::<Window>(window)
            .unwrap()
            .resolution
            .set(toolbar::PALETTE_COLLAPSE_BELOW_WIDTH, 600.0);
        world.run_system(collapse).unwrap();
        assert!(!visible(&world), "hidden by hand, widening keeps it hidden");
    }

    #[test]
    fn row_thumbnails_scale_wide_patterns_to_the_wide_texture() {
        let colors = Theme::Dark.board();
        let live = crate::render_plugin::color_to_texel(colors.live);
        // A 36 × 9 row of live cells, as wide as the Gosper glider gun.
        let cells: Vec<_> = (0..36).map(|x| (x, 4)).collect();
        let image = thumbnail_sized(&cells, 36, 9, colors, ROW_THUMBNAIL_TEXELS, 4.0, 14.0);
        let data = image.data.unwrap();
        let width = ROW_THUMBNAIL_TEXELS.x as usize;
        let lit = |x: usize, y: usize| data[(y * width + x) * 4..(y * width + x) * 4 + 4] == live;
        // The cells' texel row, rather than a gap between rows of cells.
        let row = (0..ROW_THUMBNAIL_TEXELS.y as usize)
            .max_by_key(|&y| (0..width).filter(|&x| lit(x, y)).count())
            .unwrap();
        let first = (0..width).find(|&x| lit(x, row)).unwrap();
        let last = (0..width).rfind(|&x| lit(x, row)).unwrap();
        let span = last - first + 1;
        // Width bounds the scale, so the pattern spans most of the texture.
        assert!(span > width * 3 / 4, "{span}");
        assert!(
            !lit(0, row) && !lit(width - 1, row),
            "the margin stays clear"
        );
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
    fn speed_buttons_step_the_displayed_rate_with_limits_and_infinity_toggles() {
        let mut app = GolApp::new(8, 8);
        app.set_tick_rate_ms(100);
        apply_action(Action::SpeedAdjust(1), &mut app);
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
        assert!(!app.is_max_speed());
        apply_action(Action::MaxSpeed, &mut app);
        assert!(app.is_max_speed());
        apply_action(Action::SpeedAdjust(-1), &mut app);
        assert!(!app.is_max_speed());
        assert_eq!(app.tick_rate_ms(), 25);
        assert_eq!(app.board.live_cells_count(), 0);
    }

    #[test]
    fn toolbar_keeps_one_row_and_input_regions_match_the_canvas() {
        let layout = UiLayout::default();
        let narrow = Vec2::new(toolbar::COLLAPSE_BELOW_WIDTH - 1.0, 800.0);
        let wide = Vec2::new(toolbar::COLLAPSE_BELOW_WIDTH, 800.0);
        assert_eq!(
            canvas_rect(narrow, &layout).min.y,
            canvas_rect(wide, &layout).min.y
        );
        let below_toolbar = Vec2::new(500.0, toolbar::HEIGHT + 20.0);
        for size in [narrow, wide] {
            assert!(
                !panel_rects(size, &layout)
                    .iter()
                    .any(|r| r.contains(below_toolbar))
            );
            assert!(canvas_rect(size, &layout).contains(below_toolbar));
        }
        for width in [680.0, 999.0, 1000.0, 1039.0, 1040.0, 1200.0, 1600.0] {
            for visible in [true, false] {
                let size = Vec2::new(width, 420.0);
                let layout = UiLayout {
                    control_deck_visible: visible,
                    ..default()
                };
                let canvas = canvas_rect(size, &layout);
                // One status bar row at every width, down to the minimum window.
                assert_eq!(canvas.max.y, size.y - STATUS_BAR_HEIGHT - 8.0);
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
                    ..default()
                },
            )
            .iter()
            .any(|r| r.contains(pos))
        };
        assert!(contains(true, Vec2::new(300.0, 40.0)));
        assert!(contains(true, Vec2::new(40.0, 150.0)));
        assert!(contains(false, Vec2::new(40.0, 150.0)));
        assert!(contains(true, Vec2::new(40.0, 210.0)));
        assert!(contains(true, Vec2::new(500.0, 590.0)));
        assert!(!contains(true, Vec2::new(400.0, 300.0)));
        assert!(contains(false, Vec2::new(40.0, 210.0)));
        assert!(!contains(false, Vec2::new(40.0, 300.0)));
    }
}
