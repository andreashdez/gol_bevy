//! Native editors and modal input routing. Drafts survive closing and reopening.
use super::*;
use crate::{
    board::{Rule, frontier_dense_threshold_percent_for_diagnostics},
    shared::{
        BOARD_SIZE_MAX, BOARD_SIZE_MIN, BOARD_SIZE_PRESETS, BoardSettings, CONTROLS_TEXT,
        DiagnosticsOverlay,
    },
};
use bevy::{
    clipboard::Clipboard,
    input_focus::AutoFocus,
    text::{EditableText, TextCursorStyle, TextEdit},
    ui_widgets::{SelectAllOnFocus, TextInput},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogKind {
    Import,
    Settings,
    Help,
    Diagnostics,
}

impl DialogKind {
    fn title(self) -> &'static str {
        match self {
            Self::Import => "Import and export RLE",
            Self::Settings => "Workspace settings",
            Self::Help => "Controls and tips",
            Self::Diagnostics => "Simulation diagnostics",
        }
    }
}

/// Well-known rules offered as one-click choices in the settings dialog.
pub(super) const RULE_PRESETS: [(&str, &str); 4] = [
    ("Life", "B3/S23"),
    ("HighLife", "B36/S23"),
    ("Day & Night", "B3678/S34678"),
    ("Seeds", "B2/S"),
];

#[derive(Resource)]
struct Drafts {
    name: String,
    source: String,
    width: String,
    height: String,
    rule: String,
    feedback: String,
    error: bool,
    return_focus: Option<Entity>,
}

impl Default for Drafts {
    fn default() -> Self {
        Self {
            name: "Imported Pattern".into(),
            source: "x = 3, y = 3\nbo$2bo$3o!".into(),
            width: String::new(),
            height: String::new(),
            rule: String::new(),
            feedback: String::new(),
            error: false,
            return_focus: None,
        }
    }
}

#[derive(Component)]
pub(super) struct DialogRoot;
#[derive(Component, Clone, Copy)]
pub(super) enum Field {
    Name,
    Source,
    Width,
    Height,
    Rule,
}
#[derive(Component)]
pub(super) enum DialogReadout {
    Feedback,
    Diagnostics,
}

pub(super) struct NativeDialogsPlugin;
impl Plugin for NativeDialogsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Drafts>().add_observer(dialog_action);
    }
}

#[derive(SystemParam)]
pub(super) struct DialogState<'w> {
    mode: ResMut<'w, NativeUi>,
    capture: ResMut<'w, crate::ui_input::UiInputCapture>,
    drafts: ResMut<'w, Drafts>,
    sim: ResMut<'w, Simulation>,
    diagnostics: ResMut<'w, DiagnosticsOverlay>,
    board: ResMut<'w, BoardSettings>,
    focus: ResMut<'w, InputFocus>,
    /// Absent in tests, which must not touch the system clipboard.
    clipboard: Option<ResMut<'w, Clipboard>>,
}

fn save_fields<F: bevy::ecs::query::QueryFilter>(
    drafts: &mut Drafts,
    fields: &Query<(&Field, &mut EditableText), F>,
) {
    for (field, input) in fields.iter() {
        let value = input.value().into_iter().collect::<String>();
        let target = match field {
            Field::Name => &mut drafts.name,
            Field::Source => &mut drafts.source,
            Field::Width => &mut drafts.width,
            Field::Height => &mut drafts.height,
            Field::Rule => &mut drafts.rule,
        };
        if *target != value {
            *target = value;
            drafts.feedback.clear();
        }
    }
}

fn close(state: &mut DialogState) {
    state.capture.block_modal_frame();
    match state.mode.dialog {
        Some(DialogKind::Help) => state.sim.app.set_help_visible(false),
        Some(DialogKind::Diagnostics) => state.diagnostics.enabled = false,
        _ => {}
    }
    state.mode.dialog = None;
    if let Some(entity) = state.drafts.return_focus.take() {
        state
            .focus
            .set(entity, bevy::input_focus::FocusCause::Navigated);
    } else {
        state.focus.clear();
    }
}

fn dialog_action(
    event: On<Activate>,
    actions: Query<&Action>,
    mut state: DialogState,
    mut fields: Query<(&Field, &mut EditableText)>,
) {
    let Ok(action) = actions.get(event.entity) else {
        return;
    };
    save_fields(&mut state.drafts, &fields);
    if state.mode.dialog.is_none()
        && matches!(
            action,
            Action::Import | Action::Settings | Action::Help | Action::Diagnostics
        )
    {
        state.drafts.return_focus = Some(event.entity);
    }
    match action {
        Action::Import => {
            state.mode.dialog = Some(DialogKind::Import);
            state.drafts.feedback.clear();
        }
        Action::Settings => {
            let (width, height) = state.sim.app.board.size();
            state.drafts.width = width.to_string();
            state.drafts.height = height.to_string();
            state.drafts.rule = state.sim.app.rule_label();
            state.drafts.feedback.clear();
            state.mode.dialog = Some(DialogKind::Settings);
        }
        Action::Help => {
            state.sim.app.set_help_visible(true);
            state.mode.dialog = Some(DialogKind::Help);
        }
        Action::Diagnostics => {
            state.diagnostics.enabled = true;
            state.mode.dialog = Some(DialogKind::Diagnostics);
        }
        Action::CloseDialog => close(&mut state),
        Action::ImportSubmit => {
            let name = if state.drafts.name.trim().is_empty() {
                format!("Imported {}", state.sim.app.pattern_count() + 1)
            } else {
                state.drafts.name.trim().to_owned()
            };
            let source = state.drafts.source.clone();
            match state.sim.app.import_rle_pattern(name.clone(), &source) {
                Ok(import) => {
                    state.drafts.error = false;
                    state.drafts.feedback = format!(
                        "Imported {name}. Close this dialog and click the board to place it."
                    );
                    if let Some(rule) = import.applied_rule {
                        state
                            .drafts
                            .feedback
                            .push_str(&format!(" Board rule set to {rule}."));
                    }
                }
                Err(error) => {
                    state.drafts.error = true;
                    state.drafts.feedback = format!("Could not import: {error}");
                }
            }
        }
        Action::BoardPreset(size) => {
            for (field, mut input) in &mut fields {
                if matches!(field, Field::Width | Field::Height) {
                    input.queue_edit(TextEdit::SelectAll);
                    input.queue_edit(TextEdit::Insert(size.to_string().into()));
                }
            }
            state.drafts.feedback.clear();
        }
        Action::ResizeBoard => match parse_size(&state.drafts.width, &state.drafts.height) {
            Ok((width, height)) => {
                state.board.set(width, height);
                state.drafts.error = false;
                state.drafts.feedback = format!(
                    "Board size set to {width} × {height}. Your choice is saved automatically."
                );
            }
            Err(error) => {
                state.drafts.error = true;
                state.drafts.feedback = error.into();
            }
        },
        Action::RulePreset(index) => {
            let Some(&(name, rulestring)) = RULE_PRESETS.get(*index) else {
                return;
            };
            show_rule(&mut fields, rulestring);
            state.drafts.rule = rulestring.to_owned();
            let rule = Rule::parse(rulestring).expect("rule presets are valid");
            apply_rule(&mut state, rule, Some(name));
        }
        Action::ApplyRule => match Rule::parse(&state.drafts.rule) {
            Ok(rule) => {
                // Show the canonical form, so `23/3` reads back as `B3/S23`.
                show_rule(&mut fields, &rule.to_string());
                apply_rule(&mut state, rule, None);
            }
            Err(error) => {
                state.drafts.error = true;
                state.drafts.feedback =
                    format!("Could not read the rule: {error}. Try B/S notation such as B3/S23.");
            }
        },
        Action::ExportRle => export_rle(&mut state),
        _ => {}
    }
}

fn show_rule(fields: &mut Query<(&Field, &mut EditableText)>, rulestring: &str) {
    for (field, mut input) in fields {
        if matches!(field, Field::Rule) {
            input.queue_edit(TextEdit::SelectAll);
            input.queue_edit(TextEdit::Insert(rulestring.into()));
        }
    }
}

fn apply_rule(state: &mut DialogState, rule: Rule, name: Option<&str>) {
    let described = match name {
        Some(name) => format!("{name} ({rule})"),
        None => rule.to_string(),
    };
    state.drafts.error = false;
    state.drafts.feedback = if state.sim.app.set_rule(rule) {
        format!("Rule set to {described}. U undoes the change.")
    } else {
        format!("The board already uses {described}.")
    };
}

fn export_rle(state: &mut DialogState) {
    let Some(export) = state.sim.app.board_to_rle() else {
        state.drafts.error = true;
        state.drafts.feedback = "The board is empty, so there is nothing to export.".into();
        return;
    };
    let copied = match state.clipboard.as_mut() {
        Some(clipboard) => clipboard
            .set_text(export.rle)
            .map_err(|err| err.to_string()),
        None => Err("no clipboard is available".to_owned()),
    };
    match copied {
        Ok(()) => {
            state.drafts.error = false;
            state.drafts.feedback = format!(
                "Copied {} live cells ({} × {}) to the clipboard as RLE.",
                export.cells, export.width, export.height
            );
        }
        Err(error) => {
            state.drafts.error = true;
            state.drafts.feedback = format!("Could not copy to the clipboard: {error}.");
        }
    }
}

fn parse_size(width: &str, height: &str) -> Result<(usize, usize), &'static str> {
    let parse = |value: &str| {
        value
            .trim()
            .parse::<usize>()
            .ok()
            .filter(|value| (BOARD_SIZE_MIN..=BOARD_SIZE_MAX).contains(value))
    };
    match (parse(width), parse(height)) {
        (Some(width), Some(height)) => Ok((width, height)),
        _ => Err("Enter whole numbers from 8 to 2048 for both width and height."),
    }
}

pub(super) fn update_dialogs(
    mut commands: Commands,
    mut state: DialogState,
    keys: Res<ButtonInput<KeyCode>>,
    roots: Query<Entity, With<DialogRoot>>,
    mut rendered: Local<Option<DialogKind>>,
    fields: Query<(&Field, &mut EditableText), Changed<EditableText>>,
    mut readouts: Query<(&DialogReadout, &mut Text, &mut TextColor)>,
) {
    save_fields(&mut state.drafts, &fields);
    if state.mode.dialog.is_none() {
        if state.sim.app.is_help_visible() {
            state.mode.dialog = Some(DialogKind::Help);
        } else if state.diagnostics.enabled {
            state.mode.dialog = Some(DialogKind::Diagnostics);
        }
    }
    if state.mode.dialog.is_some()
        && (keys.just_pressed(KeyCode::Escape)
            || (state.mode.dialog == Some(DialogKind::Diagnostics)
                && keys.just_pressed(KeyCode::F3))
            || (state.mode.dialog == Some(DialogKind::Help) && keys.just_pressed(KeyCode::Slash)))
    {
        close(&mut state);
    }
    let desired = state.mode.dialog;
    if desired != *rendered {
        for root in &roots {
            commands.entity(root).despawn();
        }
        if desired.is_some() {
            state.focus.clear();
        }
        if let Some(kind) = desired {
            build_dialog(&mut commands, kind, &state.drafts);
        }
        *rendered = desired;
    }
    for (kind, mut text, mut color) in &mut readouts {
        let value = match kind {
            DialogReadout::Feedback => {
                color.0 = if state.drafts.error {
                    Color::srgb(1.0, 0.55, 0.5)
                } else {
                    ACCENT
                };
                state.drafts.feedback.clone()
            }
            DialogReadout::Diagnostics => {
                let (width, height) = state.sim.app.board.size();
                format!(
                    "Board                 {width} x {height}\nGeneration            {}\nLiving cells          {}\nRule                  {}\n\nAverage step          {:.3} ms\nTick interval         {}\nMeasured rate         {:.1} gen/s\nSamples               {}\nDense fallback        {}%\n\nPlayback continues while this view is open.",
                    state.sim.app.epoch,
                    state.sim.app.board.live_cells_count(),
                    state.sim.app.rule_label(),
                    state.sim.avg_step_ms,
                    if state.sim.app.is_max_speed() {
                        "max speed".to_owned()
                    } else {
                        format!("{} ms", state.sim.app.tick_rate_ms())
                    },
                    state.sim.rate.per_second(),
                    state.sim.step_samples,
                    frontier_dense_threshold_percent_for_diagnostics()
                )
            }
        };
        if text.0 != value {
            text.0 = value;
        }
    }
}

fn field(
    parent: &mut ChildSpawnerCommands,
    title: &str,
    value: &str,
    field: Field,
    multiline: bool,
) {
    parent.spawn(label(title, 13.0, MUTED));
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
        TextColor(INK),
        TextCursorStyle {
            color: ACCENT,
            ..default()
        },
        BackgroundColor(Color::srgb(0.025, 0.04, 0.055)),
        BorderColor::all(LINE),
    ));
    if !multiline {
        entity.insert(SelectAllOnFocus);
    }
    if matches!(field, Field::Name | Field::Width) {
        entity.insert(AutoFocus);
    }
}

fn build_dialog(commands: &mut Commands, kind: DialogKind, drafts: &Drafts) {
    commands.spawn((DialogRoot, Node {
        position_type: PositionType::Absolute, width: percent(100), height: percent(100),
        align_items: AlignItems::Center, justify_content: JustifyContent::Center,
        padding: UiRect::all(px(20)), ..default()
    }, BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.65)), GlobalZIndex(50)))
    .with_children(|p| {
        p.spawn((TabGroup::modal(), super::accessibility::dialog_node(kind.title()), Node {
            width: px(540), max_width: percent(100), max_height: percent(100),
            flex_direction: FlexDirection::Column, padding: UiRect::all(px(22)), row_gap: px(16),
            border: UiRect::all(px(1)), border_radius: BorderRadius::all(px(14)), ..default()
        }, BackgroundColor(PANEL), BorderColor::all(LINE))).with_children(|p| {
            p.spawn(row()).with_children(|p| {
                p.spawn(label(kind.title(), 24.0, INK));
                p.spawn(Node { flex_grow: 1.0, ..default() });
                let close_button = button(p, "Close", Action::CloseDialog);
                if matches!(kind, DialogKind::Help | DialogKind::Diagnostics) { p.commands().entity(close_button).insert(AutoFocus); }
            });
            p.spawn((ScrollArea, super::accessibility::named_node(accesskit::Role::GenericContainer, ""), Node { flex_direction: FlexDirection::Column, row_gap: px(12),
                overflow: Overflow::scroll_y(), min_height: px(0), ..default() })).with_children(|p| {
                match kind {
                    DialogKind::Import => {
                        p.spawn(label("Paste RLE to add a stamp to your palette. A rule in the header also updates the board rule.", 14.0, MUTED));
                        field(p, "Pattern name", &drafts.name, Field::Name, false);
                        field(p, "RLE source", &drafts.source, Field::Source, true);
                        button(p, "Import and select", Action::ImportSubmit);
                        p.spawn(label("Export", 19.0, INK));
                        p.spawn(label("Copy the live cells, cropped to their bounding box, as RLE with the board rule.", 14.0, MUTED));
                        button(p, "Copy board as RLE", Action::ExportRle);
                    }
                    DialogKind::Settings => {
                        p.spawn(label("Board dimensions", 19.0, INK));
                        p.spawn(label("Resizing preserves cells that fit and clears undo history. Window resizing leaves the board size unchanged.", 14.0, MUTED));
                        p.spawn(row()).with_children(|p| {
                            for (size, _) in BOARD_SIZE_PRESETS { button(p, &format!("{size}²"), Action::BoardPreset(size)); }
                        });
                        p.spawn(Node { column_gap: px(12), flex_shrink: 0.0, ..default() }).with_children(|p| {
                            for (title, value, kind) in [("Width (cells)", &drafts.width, Field::Width), ("Height (cells)", &drafts.height, Field::Height)] {
                                p.spawn(Node { flex_direction: FlexDirection::Column, row_gap: px(12), flex_grow: 1.0, flex_basis: px(0), min_width: px(0), ..default() })
                                    .with_children(|p| field(p, title, value, kind, false));
                            }
                        });
                        button(p, "Apply board size", Action::ResizeBoard);
                        p.spawn(label("Rule", 19.0, INK));
                        p.spawn(label("Neighbour counts that give birth (B) and let a cell survive (S). Changing the rule can be undone.", 14.0, MUTED));
                        p.spawn(row()).with_children(|p| {
                            for (index, (name, _)) in RULE_PRESETS.iter().enumerate() { button(p, name, Action::RulePreset(index)); }
                        });
                        field(p, "Rulestring", &drafts.rule, Field::Rule, false);
                        button(p, "Apply rule", Action::ApplyRule);
                        p.spawn(row()).with_children(|p| {
                            button(p, "Grid", Action::Grid);
                            button(p, "Edges", Action::Boundary);
                            button(p, "Diagnostics", Action::Diagnostics);
                        });
                        button(p, "Reset generation counter", Action::ResetEpoch);
                    }
                    DialogKind::Help => {
                        p.spawn(label("Choose a stamp or draw cells, then press Space to play. U undoes an edit and pauses; one drag counts as one edit.", 16.0, INK));
                        p.spawn(label("Tab moves between controls. Escape closes a dialog. Simulation shortcuts are disabled while a dialog is open.", 14.0, MUTED));
                        p.spawn((label(CONTROLS_TEXT.as_str(), 14.0, INK), super::accessibility::named_node(accesskit::Role::Label, CONTROLS_TEXT.as_str())));
                    }
                    DialogKind::Diagnostics => {
                        p.spawn((Text::default(), TextFont { font_size: FontSize::Px(14.0), ..default() },
                            TextColor(INK), DialogReadout::Diagnostics, super::accessibility::named_node(accesskit::Role::Label, "Diagnostics"), Pickable::IGNORE));
                    }
                }
                if matches!(kind, DialogKind::Import | DialogKind::Settings) {
                    p.spawn((label("", 14.0, ACCENT), DialogReadout::Feedback, super::accessibility::feedback_node()));
                }
            });
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    fn test_world(dialog: DialogKind) -> World {
        let mut world = World::new();
        world.insert_resource(NativeUi {
            dialog: Some(dialog),
        });
        world.insert_resource(Drafts::default());
        world.init_resource::<crate::ui_input::UiInputCapture>();
        world.insert_resource(Simulation::new(GolApp::new(32, 32)));
        world.insert_resource(BoardSettings { cols: 32, rows: 32 });
        world.insert_resource(DiagnosticsOverlay { enabled: false });
        world.insert_resource(InputFocus::default());
        world.insert_resource(ButtonInput::<KeyCode>::default());
        world.add_observer(dialog_action);
        world
    }

    #[test]
    fn closing_dialog_restores_its_opener_even_after_the_dialog_is_despawned() {
        let mut world = test_world(DialogKind::Settings);
        world.resource_mut::<NativeUi>().dialog = None;
        let opener = world.spawn(Action::Import).id();
        world.trigger(Activate { entity: opener });
        assert_eq!(world.resource::<Drafts>().return_focus, Some(opener));
        let close_button = world.spawn(Action::CloseDialog).id();
        world
            .resource_mut::<InputFocus>()
            .set(close_button, bevy::input_focus::FocusCause::Navigated);
        world.trigger(Activate {
            entity: close_button,
        });
        world.run_system_once(update_dialogs).unwrap();
        assert_eq!(world.resource::<InputFocus>().get(), Some(opener));
    }

    #[test]
    fn native_import_errors_preserve_state_and_valid_headers_apply_rules() {
        let mut world = test_world(DialogKind::Import);
        world.spawn((Field::Name, EditableText::new("Native import")));
        let source = world
            .spawn((Field::Source, EditableText::new("invalid RLE")))
            .id();
        let submit = world.spawn(Action::ImportSubmit).id();
        let count = world.resource::<Simulation>().app.pattern_count();
        world.trigger(Activate { entity: submit });
        assert!(world.resource::<Drafts>().error);
        assert_eq!(world.resource::<Simulation>().app.pattern_count(), count);
        world
            .entity_mut(source)
            .insert(EditableText::new("x = 3, y = 1, rule = B36/S23\n3o!"));
        world.trigger(Activate { entity: submit });
        assert!(!world.resource::<Drafts>().error);
        let app = &world.resource::<Simulation>().app;
        assert_eq!(app.pattern_count(), count + 1);
        assert_eq!(app.rule_label(), "B36/S23");
        assert_eq!(app.selected_pattern_name(), Some("Native import"));
        assert_eq!(app.board.live_cells_count(), 0);
    }

    #[test]
    fn escape_closes_native_dialog_without_reaching_canvas_shortcuts() {
        let mut world = test_world(DialogKind::Settings);
        world
            .resource_mut::<ButtonInput<KeyCode>>()
            .press(KeyCode::Escape);
        world.run_system_once(update_dialogs).unwrap();
        assert!(world.resource::<NativeUi>().dialog.is_none());
        assert!(
            !world
                .run_system_once(crate::ui_input::canvas_keyboard_available)
                .unwrap()
        );
        assert!(!world.resource::<Simulation>().app.should_quit);
    }

    #[test]
    fn rule_presets_and_rulestrings_change_the_rule_undoably() {
        let mut world = test_world(DialogKind::Settings);
        let rule_field = world.spawn((Field::Rule, EditableText::new("B3/S23"))).id();
        let highlife = RULE_PRESETS
            .iter()
            .position(|&(name, _)| name == "HighLife")
            .unwrap();
        let preset = world.spawn(Action::RulePreset(highlife)).id();
        world.trigger(Activate { entity: preset });
        assert_eq!(world.resource::<Simulation>().app.rule_label(), "B36/S23");
        assert!(!world.resource::<Drafts>().error);

        // Applying the same rule again is not a second edit.
        world.trigger(Activate { entity: preset });
        world.resource_mut::<Simulation>().app.undo();
        assert_eq!(world.resource::<Simulation>().app.rule_label(), "B3/S23");

        let apply = world.spawn(Action::ApplyRule).id();
        world
            .entity_mut(rule_field)
            .insert(EditableText::new("B3/S2x"));
        world.trigger(Activate { entity: apply });
        assert!(world.resource::<Drafts>().error);
        assert_eq!(world.resource::<Simulation>().app.rule_label(), "B3/S23");

        // The legacy survival-first form is accepted too.
        world
            .entity_mut(rule_field)
            .insert(EditableText::new("23/36"));
        world.trigger(Activate { entity: apply });
        assert!(!world.resource::<Drafts>().error);
        assert_eq!(world.resource::<Simulation>().app.rule_label(), "B36/S23");
    }

    #[test]
    fn every_rule_preset_parses() {
        for (name, rulestring) in RULE_PRESETS {
            let rule = Rule::parse(rulestring).unwrap_or_else(|err| panic!("{name}: {err}"));
            assert_eq!(
                rule.to_string(),
                rulestring,
                "{name} is not in canonical form"
            );
        }
    }

    #[test]
    fn export_reports_an_empty_board_or_a_missing_clipboard() {
        let mut world = test_world(DialogKind::Import);
        let export = world.spawn(Action::ExportRle).id();
        world.trigger(Activate { entity: export });
        assert!(world.resource::<Drafts>().error);
        assert!(world.resource::<Drafts>().feedback.contains("empty"));

        world.resource_mut::<Simulation>().app.populate_board(3, 3);
        world.trigger(Activate { entity: export });
        assert!(world.resource::<Drafts>().error);
        assert!(world.resource::<Drafts>().feedback.contains("clipboard"));
    }

    #[test]
    fn board_dimensions_reject_invalid_input_instead_of_silently_clamping() {
        assert_eq!(parse_size(" 128 ", "2048"), Ok((128, 2048)));
        for (width, height) in [
            ("7", "128"),
            ("128", "2049"),
            ("", "128"),
            ("2.5", "128"),
            ("-1", "128"),
        ] {
            assert!(parse_size(width, height).is_err());
        }
    }
}
