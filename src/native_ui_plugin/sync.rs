//! Keeping buttons, readouts, and panels in step with app state
use super::*;

pub(super) type ButtonState<'a> = (
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
pub(super) struct ViewState<'w> {
    pub(super) mode: Res<'w, NativeUi>,
    pub(super) grid: Res<'w, GridOverlay>,
    pub(super) cell_age: Res<'w, CellAge>,
    pub(super) board: Res<'w, crate::shared::BoardSettings>,
    pub(super) focus: Res<'w, InputFocus>,
    pub(super) focus_visible: Res<'w, InputFocusVisible>,
    pub(super) theme: Res<'w, Theme>,
}

pub(super) type RootNode<'a> = (
    &'a mut Node,
    &'a mut Visibility,
    Has<Sidebar>,
    Has<toolbar::Toolbar>,
);

pub(super) fn sync(
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
        grid,
        cell_age,
        board,
        focus,
        focus_visible,
        theme,
    } = view;
    let palette = sim.app.palette_shown();
    for (mut node, mut visibility, sidebar, toolbar) in &mut roots {
        let display = if sidebar && !palette {
            Display::None
        } else if toolbar {
            Display::Grid
        } else {
            Display::Flex
        };
        set_if_neq!(node.display, display);
        // Tab walks whatever is visible, even in a node removed from the layout.
        if sidebar {
            visibility.set_if_neq(if palette {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
        }
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
            Action::ToggleEdges => format!("Edges: {}", sim.app.boundary_mode_label()),
            _ => accessible_label.to_owned(),
        };
        if accessible.label() != Some(accessible_label.as_str()) {
            accessible.set_label(accessible_label);
        }
        let disabled = match action {
            Action::ResetToStart => !sim.app.can_reset_to_start(),
            Action::Undo => !sim.app.can_undo(),
            Action::Redo => !sim.app.can_redo(),
            Action::SpeedAdjust(step) if *step < 0 => {
                !sim.app.is_max_speed() && sim.app.tick_rate_ms() >= 1000
            }
            Action::SpeedAdjust(_) => {
                sim.app.is_max_speed() || controls::fixed_rate(&sim.app) >= controls::MAX_FIXED_RATE
            }
            Action::Unbounded => !sim.app.board.allows_unbounded(),
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
            Action::Theme(choice) => *theme == *choice,
            Action::Edges(mode) => sim.app.board.boundary_mode() == Some(*mode),
            Action::Unbounded => sim.app.board.finite().is_none(),
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
                | Action::RulePreset(_)
                | Action::Theme(_)
                | Action::Edges(_)
                | Action::Unbounded
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
        // Only keyboard focus is outlined: a click focuses the button for a
        // frame before `release_pointer_focus` hands the keyboard back.
        border.set_if_neq(ThemedBorder(
            if focus_visible.0 && focus.get() == Some(entity) {
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
            },
        ));
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
                    Action::ToggleEdges => Some(if sim.app.board.finite().is_some() {
                        format!("{} edges", sim.app.boundary_mode_label())
                    } else {
                        sim.app.boundary_mode_label().to_owned()
                    }),
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
