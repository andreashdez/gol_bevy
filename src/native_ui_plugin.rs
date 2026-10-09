//! Native Bevy interface for the simulation.
mod accessibility;
mod controls;
mod dialogs;
mod palette;
mod toolbar;
use crate::{
    app::{App as GolApp, EditTool},
    shared::{EditFeedback, GridOverlay, Simulation, StartupSet, UiLayout, UpdateSet},
    ui_input::{UiInputRegions, UiInputSet},
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

const PANEL: Color = Color::srgb(0.055, 0.075, 0.095);
const SURFACE: Color = Color::srgb(0.095, 0.12, 0.145);
const LINE: Color = Color::srgb(0.18, 0.23, 0.27);
const INK: Color = Color::srgb(0.88, 0.93, 0.94);
const MUTED: Color = Color::srgb(0.52, 0.63, 0.68);
const ACCENT: Color = Color::srgb(0.22, 0.85, 0.56);

fn bottom_height(size: Vec2) -> f32 {
    if size.x < 1000.0 { 88.0 } else { 48.0 }
}
const SIDE: f32 = 256.0;
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
            .add_plugins(TabNavigationPlugin)
            .add_observer(activate)
            .add_observer(controls::speed_changed)
            .add_observer(controls::navigate)
            .add_plugins(dialogs::NativeDialogsPlugin)
            .add_plugins(accessibility::NativeAccessibilityPlugin)
            .add_systems(
                PostUpdate,
                palette::sync_scrollbar.after(bevy::ui::UiSystems::Layout),
            )
            .add_systems(Startup, setup.after(StartupSet::BuildWorld))
            .add_systems(
                Update,
                (
                    dialogs::update_dialogs,
                    sync,
                    sync_palette,
                    palette::filter,
                    palette::sync_preview,
                    controls::sync_speed,
                    controls::sync_overlays,
                    controls::sync_footer,
                    toolbar::sync_layout,
                    controls::sync_feedback,
                    publish_input_regions,
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
struct Palette;
#[derive(Component)]
struct Footer;
#[derive(Component)]
struct FooterStatus;
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
    FitBoard,
    FitLiveCells,
    ZoomIn,
    ZoomOut,
    MaxSpeed,
    Sidebar,
    Import,
    Help,
    Settings,
    Diagnostics,
    CloseDialog,
    ImportSubmit,
    ResizeBoard,
    BoardPreset(usize),
    /// Index into the rule presets of the settings dialog.
    RulePreset(usize),
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

fn label(value: impl Into<String>, size: f32, color: Color) -> impl Bundle {
    (
        Text::new(value),
        TextFont {
            font: UI_FONT.into(),
            font_size: FontSize::Px(size),
            ..default()
        },
        TextColor(color),
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
            BackgroundColor(SURFACE),
            BorderColor::all(LINE),
            Name::new(text.to_owned()),
            controls::Tooltip(controls::hint(action).to_owned()),
        ))
        .with_children(|p| {
            p.spawn(label(text, 14.0, INK));
        })
        .id()
}

fn setup(mut commands: Commands, mut fonts: ResMut<Assets<Font>>) {
    // Embed the licensed font so launching a desktop bundle works from any directory.
    fonts
        .insert(
            UI_FONT.id(),
            Font::from_bytes(include_bytes!("../assets/fonts/IBMPlexSans-Regular.ttf").to_vec()),
        )
        .expect("static UI font handle is valid");
    toolbar::spawn(&mut commands);
    commands
        .spawn((
            NativeRoot,
            Sidebar,
            TabGroup::default(),
            Node {
                position_type: PositionType::Absolute,
                left: px(16),
                top: px(toolbar::height(1200.0) + 16.0),
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
            BackgroundColor(PANEL),
            BorderColor::all(LINE),
            GlobalZIndex(10),
        ))
        .with_children(|p| {
            p.spawn(row()).with_children(|p| {
                button(p, "Draw", Action::Tool(EditTool::Draw));
                button(p, "Erase", Action::Tool(EditTool::Erase));
                button(p, "Stamp", Action::Tool(EditTool::Stamp));
            });
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
            BackgroundColor(PANEL),
            BorderColor::all(LINE),
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
                        p.spawn((label("", 13.0, INK), kind))
                            .insert(controls::numeric_font(13.0));
                    });
                }
                p.spawn((label("", 12.0, MUTED), Readout::Board));
            });
            p.spawn(row()).with_children(|p| {
                button(p, "−", Action::ZoomOut);
                p.spawn((
                    controls::ZoomLabel,
                    label("100%", 13.0, INK),
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
    controls::overlays(&mut commands);
    controls::feedback_overlay(&mut commands);
}

/// Logical rectangles shared by layout and input routing; gaps remain canvas.
pub fn panel_rects(size: Vec2, layout: &UiLayout) -> Vec<Rect> {
    let top = toolbar::height(size.x);
    let bottom = bottom_height(size);
    let mut rects = vec![
        Rect::new(0.0, 0.0, size.x, top),
        Rect::new(0.0, (size.y - bottom).max(0.0), size.x, size.y),
    ];
    if layout.control_deck_visible && size.y > top + bottom + 32.0 {
        rects.push(Rect::new(
            16.0,
            top + 16.0,
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
        16.0
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
    mut regions: ResMut<UiInputRegions>,
) {
    regions.modal = state.dialog.is_some();
    regions.panels = windows
        .single()
        .map(|window| panel_rects(window.size(), &layout))
        .unwrap_or_default();
}

fn activate(
    event: On<Activate>,
    actions: Query<&Action>,
    mut sim: ResMut<Simulation>,
    mut grid: ResMut<GridOverlay>,
    mut layout: ResMut<UiLayout>,
    mut feedback: ResMut<EditFeedback>,
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
        Action::Grid => grid.enabled = !grid.enabled,
        Action::Sidebar => layout.toggle_control_deck(),
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
    &'a mut BackgroundColor,
    &'a mut BorderColor,
    &'a Children,
    &'a Name,
    &'a mut bevy::a11y::AccessibilityNode,
);

#[derive(SystemParam)]
struct ViewState<'w> {
    mode: Res<'w, NativeUi>,
    layout: Res<'w, UiLayout>,
    grid: Res<'w, GridOverlay>,
    focus: Res<'w, InputFocus>,
    focus_visible: Res<'w, InputFocusVisible>,
}

type RootNode<'a> = (&'a mut Node, Has<Sidebar>, Has<toolbar::Toolbar>);

fn sync(
    mut commands: Commands,
    sim: Res<Simulation>,
    view: ViewState,
    mut roots: Query<RootNode, With<NativeRoot>>,
    mut buttons: Query<ButtonState>,
    mut texts: Query<(&mut Text, &mut TextColor, Option<&Readout>)>,
) {
    let ViewState {
        mode: _,
        layout,
        grid,
        focus,
        focus_visible: _,
    } = view;
    for (mut node, sidebar, toolbar) in &mut roots {
        let display = if sidebar && !layout.control_deck_visible {
            Display::None
        } else if toolbar {
            Display::Grid
        } else {
            Display::Flex
        };
        if node.display != display {
            node.display = display;
        }
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
            Action::Sidebar => layout.control_deck_visible,
            Action::MaxSpeed => sim.app.is_max_speed(),
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
                | Action::Sidebar
                | Action::MaxSpeed
                | Action::RulePreset(_)
        ) {
            accessible.set_toggled(if selected {
                accesskit::Toggled::True
            } else {
                accesskit::Toggled::False
            });
        }
        bg.0 = if disabled {
            PANEL
        } else if *action == Action::Play {
            if pressed {
                Color::srgb(0.15, 0.65, 0.43)
            } else {
                ACCENT
            }
        } else if pressed || hovered.get() {
            Color::srgb(0.16, 0.23, 0.26)
        } else if selected {
            Color::srgb(0.08, 0.21, 0.17)
        } else {
            SURFACE
        };
        *border = BorderColor::all(if focus.get() == Some(entity) {
            INK
        } else if selected {
            ACCENT
        } else {
            LINE
        });
        for child in children.iter() {
            if let Ok((mut text, mut color, _)) = texts.get_mut(child) {
                color.0 = if disabled {
                    MUTED
                } else if *action == Action::Play {
                    PANEL
                } else {
                    INK
                };
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
    palette: Query<Entity, With<Palette>>,
    mut images: ResMut<Assets<Image>>,
    mut count: Local<usize>,
) {
    if *count == sim.app.pattern_count() {
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
                        label(if index == 0 { "BUILTIN" } else { "IMPORTED" }, 11.0, MUTED),
                    ));
                }
                let Some(pattern) = sim.app.pattern_at(index) else {
                    continue;
                };
                let preview = images.add(thumbnail(&pattern.cells, pattern.width, pattern.height));
                p.spawn((
                    WidgetButton,
                    Action::Pattern(index),
                    controls::Tooltip(controls::pattern_hint(&pattern.name, index)),
                    Hovered::default(),
                    TabIndex(0),
                    Name::new(pattern.name.clone()),
                    BackgroundColor(SURFACE),
                    BorderColor::all(LINE),
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
                        p.spawn(label(&pattern.name, 15.0, INK));
                        p.spawn(label(
                            format!(
                                "{} × {} · {} cells",
                                pattern.width,
                                pattern.height,
                                pattern.cells.len()
                            ),
                            11.0,
                            MUTED,
                        ));
                    });
                });
            }
        });
    *count = sim.app.pattern_count();
}

/// A bounded thumbnail keeps large imported patterns from spawning thousands of UI entities.
fn thumbnail(cells: &[(usize, usize)], width: usize, height: usize) -> Image {
    let mut image = Image::new_fill(
        Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        &[18, 29, 35, 255],
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
                    data[(py * 64 + px) * 4..(py * 64 + px) * 4 + 4]
                        .copy_from_slice(&[56, 217, 143, 255]);
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
    fn native_widget_bundles_spawn_together() {
        use bevy::ecs::system::RunSystemOnce;
        let mut world = World::new();
        world.init_resource::<Assets<Font>>();
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
                .query_filtered::<Entity, With<controls::SpeedSlider>>()
                .iter(&world)
                .count(),
            1
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
    fn compact_toolbar_returns_space_to_the_canvas_and_input_regions_follow_it() {
        let layout = UiLayout::default();
        let narrow = Vec2::new(1199.0, 800.0);
        let wide = Vec2::new(1200.0, 800.0);
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
        for width in [680.0, 999.0, 1000.0, 1199.0, 1200.0, 1600.0] {
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
        assert!(contains(true, Vec2::new(40.0, 180.0)));
        assert!(contains(true, Vec2::new(500.0, 590.0)));
        assert!(!contains(true, Vec2::new(400.0, 300.0)));
        assert!(!contains(false, Vec2::new(40.0, 180.0)));
    }
}
