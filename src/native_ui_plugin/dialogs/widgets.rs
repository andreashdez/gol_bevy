//! The controls dialogs are built from: fields, rows, switches, and the density slider.
use super::*;
use bevy::ui_widgets::{
    SelectAllOnFocus, Slider, SliderPrecision, SliderRange, SliderStep, SliderThumb, SliderValue,
    TextInput, TrackClick, ValueChange,
};

pub(super) fn field(
    parent: &mut ChildSpawnerCommands,
    title: &str,
    value: &str,
    field: Field,
    multiline: bool,
    autofocus: bool,
) {
    parent.spawn(label(title, 13.0, Token::Muted));
    let mut input = EditableText::new(value);
    input.allow_newlines = multiline;
    input.visible_lines = Some(if multiline { 7.0 } else { 1.0 });
    let mut entity = parent.spawn((
        input,
        TextInput,
        super::accessibility::named_node(accesskit::Role::TextInput, title),
        field,
        TabIndex(0),
        Node {
            width: percent(100),
            padding: UiRect::all(px(10)),
            flex_shrink: 0.0,
            border: UiRect::all(px(1)),
            border_radius: BorderRadius::all(px(6)),
            ..default()
        },
        TextFont {
            font: if multiline { default() } else { UI_FONT.into() },
            font_size: FontSize::Px(16.0),
            ..default()
        },
        ThemedText(Token::Ink),
        theme::ThemedCursor(Token::Accent),
        ThemedBackground(Token::Field),
        ThemedBorder(Token::Line),
    ));
    if !multiline {
        entity.insert(SelectAllOnFocus);
    }
    if autofocus {
        entity.insert(AutoFocus);
    }
}

/// A section title with a line saying what the section changes.
/// Returns the description, which something else may want to keep up to date.
pub(super) fn heading(parent: &mut ChildSpawnerCommands, title: &str, description: &str) -> Entity {
    let mut text = Entity::PLACEHOLDER;
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
            margin: UiRect::top(px(8)),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|p| {
            p.spawn(label(title, 19.0, Token::Ink));
            text = p.spawn(label(description, 14.0, Token::Muted)).id();
        });
    text
}

/// A single-line field: 10 px padding, a 1 px border, and 16 px text at
/// Bevy's 1.2 line height.
const FIELD_HEIGHT: f32 = 2.0 * 10.0 + 2.0 + 16.0 * 1.2;

/// A labelled text field that shares a row with others.
pub(super) fn inline_field(
    parent: &mut ChildSpawnerCommands,
    title: &str,
    value: &str,
    kind: Field,
    autofocus: bool,
) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(8),
            flex_grow: 1.0,
            flex_basis: px(0),
            min_width: px(0),
            ..default()
        })
        .with_children(|p| field(p, title, value, kind, false, autofocus));
}

/// Fields with an Apply button at the end of their row, the one style every
/// field that needs confirming uses.
pub(super) fn apply_row(
    parent: &mut ChildSpawnerCommands,
    action: Action,
    fields: impl FnOnce(&mut ChildSpawnerCommands),
) {
    parent
        .spawn(Node {
            align_items: AlignItems::FlexEnd,
            column_gap: px(12),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|p| {
            fields(p);
            let apply = button(p, "Apply", action);
            p.commands()
                .entity(apply)
                .insert(Name::new(match action {
                    Action::ResizeBoard => "Apply board size",
                    Action::ApplyRule => "Apply rule",
                    _ => "Apply seed",
                }))
                // As tall as the fields beside it.
                .entry::<Node>()
                .and_modify(|mut node| node.min_height = px(FIELD_HEIGHT));
        });
}

/// A setting with its name on the left and its control beside it.
pub(super) fn setting(
    parent: &mut ChildSpawnerCommands,
    title: &str,
    control: impl FnOnce(&mut ChildSpawnerCommands),
) {
    parent
        .spawn(Node {
            align_items: AlignItems::Center,
            column_gap: px(12),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|p| {
            p.spawn((
                label(title, 14.0, Token::Ink),
                Node {
                    width: px(SETTING_LABEL_WIDTH),
                    flex_shrink: 0.0,
                    ..default()
                },
            ));
            control(p);
        });
}

const SETTING_LABEL_WIDTH: f32 = 84.0;

/// A button that is one choice of several; the chosen one is highlighted.
#[derive(Component)]
pub(in crate::native_ui_plugin) struct Segment;

/// Mutually exclusive choices joined in one box, like Wrap and Fixed.
pub(super) fn segmented(parent: &mut ChildSpawnerCommands, choices: &[(&str, Action)]) {
    parent
        .spawn((
            Node {
                padding: UiRect::all(px(3)),
                column_gap: px(3),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(9)),
                flex_shrink: 0.0,
                ..default()
            },
            ThemedBackground(Token::Field),
            ThemedBorder(Token::Line),
            super::accessibility::named_node(accesskit::Role::RadioGroup, ""),
        ))
        .with_children(|p| {
            for &(text, action) in choices {
                let choice = button(p, text, action);
                p.commands()
                    .entity(choice)
                    .insert(Segment)
                    .entry::<Node>()
                    .and_modify(|mut node| {
                        node.padding = UiRect::axes(px(14), px(6));
                        node.border_radius = BorderRadius::all(px(6));
                    });
            }
        });
}

/// A setting that is on or off, as a row with a switch at the end.
#[derive(Component)]
pub(in crate::native_ui_plugin) struct Switch;
#[derive(Component)]
pub(in crate::native_ui_plugin) struct SwitchTrack(Action);
#[derive(Component)]
pub(in crate::native_ui_plugin) struct SwitchKnob;

pub(super) fn switch(
    parent: &mut ChildSpawnerCommands,
    title: &str,
    description: &str,
    action: Action,
) {
    let entity = button_frame(parent, title, action);
    parent
        .commands()
        .entity(entity)
        .insert((
            Switch,
            super::accessibility::named_node(accesskit::Role::Switch, title),
            Node {
                width: percent(100),
                padding: UiRect::axes(px(10), px(8)),
                border: UiRect::all(px(1)),
                border_radius: BorderRadius::all(px(8)),
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                column_gap: px(16),
                flex_shrink: 0.0,
                ..default()
            },
        ))
        .with_children(|p| {
            p.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(2),
                    min_width: px(0),
                    ..default()
                },
                Pickable::IGNORE,
            ))
            .with_children(|p| {
                p.spawn(label(title, 14.0, Token::Ink));
                p.spawn(label(description, 13.0, Token::Muted));
            });
            p.spawn((
                SwitchTrack(action),
                Node {
                    width: px(38),
                    height: px(22),
                    padding: UiRect::all(px(2)),
                    border: UiRect::all(px(1)),
                    border_radius: BorderRadius::all(px(11)),
                    align_items: AlignItems::Center,
                    flex_shrink: 0.0,
                    ..default()
                },
                ThemedBackground(Token::Field),
                ThemedBorder(Token::Line),
                Pickable::IGNORE,
            ))
            .with_child((
                SwitchKnob,
                Node {
                    width: px(16),
                    height: px(16),
                    border_radius: BorderRadius::all(px(8)),
                    ..default()
                },
                ThemedBackground(Token::Muted),
                Pickable::IGNORE,
            ));
        });
}

/// Slides the knob to the side matching the setting, and fills the track when on.
pub(in crate::native_ui_plugin) fn sync_switches(
    view: ViewState,
    sim: Res<Simulation>,
    mut tracks: Query<
        (
            &SwitchTrack,
            &Children,
            &mut Node,
            &mut ThemedBackground,
            &mut ThemedBorder,
        ),
        Without<SwitchKnob>,
    >,
    mut knobs: Query<&mut ThemedBackground, With<SwitchKnob>>,
) {
    for (track, children, mut node, mut background, mut border) in &mut tracks {
        let on = match track.0 {
            Action::Grid => view.grid.enabled,
            Action::CellAge => view.cell_age.enabled,
            Action::FillCanvas => view.board.fill_canvas,
            Action::PauseWhenSettled => sim.app.pause_when_settled(),
            _ => false,
        };
        set_if_neq!(
            node.justify_content,
            if on {
                JustifyContent::FlexEnd
            } else {
                JustifyContent::FlexStart
            }
        );
        background.set_if_neq(ThemedBackground(if on {
            Token::Accent
        } else {
            Token::Field
        }));
        border.set_if_neq(ThemedBorder(if on { Token::Accent } else { Token::Muted }));
        for child in children.iter() {
            if let Ok(mut knob) = knobs.get_mut(child) {
                knob.set_if_neq(ThemedBackground(if on {
                    Token::OnAccent
                } else {
                    Token::Muted
                }));
            }
        }
    }
}

#[derive(Component)]
pub(in crate::native_ui_plugin) struct DensitySlider;
#[derive(Component)]
pub(in crate::native_ui_plugin) struct DensityFill;
#[derive(Component)]
pub(in crate::native_ui_plugin) struct DensityReadout;

const SLIDER_THUMB: f32 = 18.0;

/// Density from 0 to 100%, with the value beside it. Arrow keys step by 1%.
pub(super) fn density_slider(parent: &mut ChildSpawnerCommands, density: u8) {
    parent
        .spawn(Node {
            align_items: AlignItems::Center,
            column_gap: px(12),
            flex_grow: 1.0,
            min_width: px(0),
            ..default()
        })
        .with_children(|p| {
            p.spawn((
                DensitySlider,
                Slider {
                    track_click: TrackClick::Snap,
                    ..default()
                },
                SliderValue(density as f32),
                SliderRange::new(0.0, 100.0),
                SliderStep(1.0),
                SliderPrecision(0),
                TabIndex(0),
                Hovered::default(),
                super::accessibility::named_node(accesskit::Role::Slider, "Density"),
                Node {
                    flex_grow: 1.0,
                    min_width: px(120),
                    height: px(SLIDER_THUMB + 6.0),
                    ..default()
                },
            ))
            .with_children(|p| {
                let bar = |width| Node {
                    position_type: PositionType::Absolute,
                    left: px(0),
                    top: px((SLIDER_THUMB + 6.0 - 4.0) / 2.0),
                    width,
                    height: px(4),
                    border_radius: BorderRadius::all(px(2)),
                    ..default()
                };
                p.spawn((
                    bar(percent(100)),
                    ThemedBackground(Token::Line),
                    Pickable::IGNORE,
                ));
                p.spawn((
                    DensityFill,
                    bar(px(0)),
                    ThemedBackground(Token::Accent),
                    Pickable::IGNORE,
                ));
                p.spawn((
                    SliderThumb,
                    Node {
                        position_type: PositionType::Absolute,
                        top: px(3),
                        width: px(SLIDER_THUMB),
                        height: px(SLIDER_THUMB),
                        border: UiRect::all(px(2)),
                        border_radius: BorderRadius::all(px(SLIDER_THUMB / 2.0)),
                        ..default()
                    },
                    ThemedBackground(Token::Accent),
                    ThemedBorder(Token::Panel),
                ));
            });
            p.spawn((
                DensityReadout,
                Text::new(format!("{density}%")),
                super::controls::numeric_font(14.0),
                ThemedText(Token::Ink),
                TextLayout {
                    justify: bevy::text::Justify::Right,
                    ..default()
                },
                Node {
                    min_width: px(44),
                    flex_shrink: 0.0,
                    ..default()
                },
                Pickable::IGNORE,
            ));
        });
}

/// Applies the density as the slider moves, keeping the seed.
pub(super) fn density_changed(
    change: On<ValueChange<f32>>,
    sliders: Query<(), With<DensitySlider>>,
    mut commands: Commands,
    mut drafts: ResMut<Drafts>,
    mut sim: ResMut<Simulation>,
) {
    if !sliders.contains(change.source) {
        return;
    }
    let density = change.value.round().clamp(0.0, 100.0) as u8;
    commands
        .entity(change.source)
        .insert(SliderValue(density as f32));
    drafts.density = density;
    let seed = sim.app.randomize_settings().seed();
    if let Some(settings) = RandomizeSettings::new(density, seed) {
        sim.app.set_randomize_settings(settings);
    }
}

type SliderThumbNode<'a> = (&'a mut Node, &'a mut ThemedBorder);
type ThumbOnly = (With<SliderThumb>, Without<DensityFill>);

/// Places the thumb and fill for the value, and outlines the thumb for keyboard focus.
pub(in crate::native_ui_plugin) fn sync_density(
    focus: Res<InputFocus>,
    focus_visible: Res<InputFocusVisible>,
    sliders: Query<(Entity, &SliderValue, &ComputedNode, &Children), With<DensitySlider>>,
    mut thumbs: Query<SliderThumbNode, ThumbOnly>,
    mut fills: Query<&mut Node, (With<DensityFill>, Without<SliderThumb>)>,
    mut readouts: Query<&mut Text, With<DensityReadout>>,
) {
    for (entity, value, computed, children) in &sliders {
        let width = computed.size().x * computed.inverse_scale_factor();
        let offset = value.0 / 100.0 * (width - SLIDER_THUMB).max(0.0);
        for child in children.iter() {
            if let Ok((mut node, mut border)) = thumbs.get_mut(child) {
                set_if_neq!(node.left, px(offset));
                border.set_if_neq(ThemedBorder(
                    if focus_visible.0 && focus.get() == Some(entity) {
                        Token::Focus
                    } else {
                        Token::Panel
                    },
                ));
            }
            if let Ok(mut node) = fills.get_mut(child) {
                set_if_neq!(node.width, px(offset + SLIDER_THUMB / 2.0));
            }
        }
        for mut text in &mut readouts {
            let shown = format!("{}%", value.0 as u8);
            if text.0 != shown {
                text.0 = shown;
            }
        }
    }
}
