//! Native editors and modal input routing. Drafts survive closing and reopening.
use super::*;
use crate::{
    board::{BoundaryMode, RandomizeSettings, Rule},
    shared::{
        BOARD_SIZE_MAX, BOARD_SIZE_MIN, BOARD_SIZE_PRESETS, BoardSettings, DiagnosticsOverlay,
        Theme, WEB_BUILD, command_key,
    },
    universe::Universe,
    workspace::DecodedWorkspace,
};
use bevy::{
    clipboard::Clipboard,
    input_focus::AutoFocus,
    text::{EditableText, TextEdit},
};
use std::path::PathBuf;

mod files;
mod help;
mod import_preview;
#[cfg(test)]
mod tests;
mod widgets;

pub(super) use files::{FileDialog, file_shortcuts, poll_file_dialog};
use files::{
    FileRequest, confirm_open, file_dialog_busy, request_file, request_open, request_save,
};
use help::help_columns;
use import_preview::import_preview;
pub(super) use import_preview::sync_import_preview;
pub(super) use widgets::{Segment, Switch, sync_density, sync_switches};
use widgets::{
    apply_row, density_changed, density_slider, field, heading, inline_field, segmented, setting,
    switch,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogKind {
    Import,
    Export,
    Settings,
    Help,
    Diagnostics,
    /// Asks before a validated workspace file replaces the current one.
    ConfirmOpen,
}

impl DialogKind {
    fn title(self) -> &'static str {
        match self {
            Self::Import => "Import",
            Self::Export => "Export",
            Self::Settings => "Workspace settings",
            Self::Help => "Controls and tips",
            Self::Diagnostics => "Simulation diagnostics",
            Self::ConfirmOpen => "Open workspace?",
        }
    }

    /// Whether results appear in the dialog, rather than as a notice over the board.
    fn shows_feedback(self) -> bool {
        matches!(self, Self::Import | Self::Export | Self::Settings)
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
pub(super) struct Drafts {
    name: String,
    /// The last workspace file saved or opened, where the next file dialog starts.
    workspace_path: Option<PathBuf>,
    /// A validated workspace waiting for the user to confirm replacing theirs.
    pending_open: Option<(PathBuf, DecodedWorkspace)>,
    source: String,
    width: String,
    height: String,
    rule: String,
    /// Percent; the slider applies it as it moves.
    density: u8,
    seed: String,
    feedback: String,
    error: bool,
    return_focus: Option<Entity>,
    /// Settings opened from the status bar's rule, so the rulestring gets focus.
    rule_first: bool,
    /// Rebuild the dialog so its fields show drafts changed from outside them,
    /// such as an opened `.rle` file.
    rebuild: bool,
}

impl Default for Drafts {
    fn default() -> Self {
        Self {
            name: "Imported Pattern".into(),
            workspace_path: None,
            pending_open: None,
            source: "x = 3, y = 3\nbo$2bo$3o!".into(),
            width: String::new(),
            height: String::new(),
            rule: String::new(),
            density: 25,
            seed: String::new(),
            feedback: String::new(),
            error: false,
            return_focus: None,
            rule_first: false,
            rebuild: false,
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
    Seed,
}
#[derive(Component)]
pub(super) enum DialogReadout {
    Feedback,
    Diagnostics,
    /// What the Board section's size is the size of.
    BoardNote,
}

pub(super) struct NativeDialogsPlugin;
impl Plugin for NativeDialogsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Drafts>()
            .init_resource::<FileDialog>()
            .add_observer(dialog_action)
            .add_observer(density_changed);
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
    file_dialog: ResMut<'w, FileDialog>,
    /// Where file results go when no dialog shows feedback, as after Cmd+S.
    notice: ResMut<'w, crate::shared::EditFeedback>,
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
            Field::Seed => &mut drafts.seed,
        };
        if *target != value {
            *target = value;
            drafts.feedback.clear();
        }
    }
}

fn close(state: &mut DialogState) {
    state.drafts.pending_open = None;
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
            Action::Import
                | Action::Export
                | Action::Settings
                | Action::EditRule
                | Action::Help
                | Action::Diagnostics
        )
    {
        state.drafts.return_focus = Some(event.entity);
    }
    match action {
        Action::Import => {
            state.mode.dialog = Some(DialogKind::Import);
            state.drafts.feedback.clear();
        }
        Action::Export => {
            state.mode.dialog = Some(DialogKind::Export);
            state.drafts.feedback.clear();
        }
        Action::Settings | Action::EditRule => {
            state.drafts.rule_first = *action == Action::EditRule;
            let (width, height) = state.sim.app.finite_size();
            state.drafts.width = width.to_string();
            state.drafts.height = height.to_string();
            state.drafts.rule = state.sim.app.rule_label();
            let random = state.sim.app.randomize_settings();
            state.drafts.density = random.density_percent();
            state.drafts.seed = random
                .seed()
                .map(|seed| seed.to_string())
                .unwrap_or_default();
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
        Action::FillCanvas => {
            state.board.fill_canvas = !state.board.fill_canvas;
            state.drafts.error = false;
            state.drafts.feedback = if state.board.fill_canvas {
                "The board now fills the canvas, and resizes with the window and palette.".into()
            } else {
                "The board keeps its current size from now on.".into()
            };
        }
        Action::ResizeBoard => match parse_size(&state.drafts.width, &state.drafts.height) {
            Ok((width, height)) => {
                // An explicit size replaces the canvas-matched one.
                let was_filling = std::mem::take(&mut state.board.fill_canvas);
                state.board.set(width, height);
                state.drafts.error = false;
                let what = if state.sim.app.board.finite().is_some() {
                    "Board size"
                } else {
                    "Size to return to"
                };
                state.drafts.feedback = format!("{what} set to {width} × {height}.");
                if !WEB_BUILD {
                    state
                        .drafts
                        .feedback
                        .push_str(" Your choice is saved automatically.");
                }
                if was_filling {
                    state.drafts.feedback.push_str(" Fill canvas is now off.");
                }
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
        Action::ApplyRandomize | Action::RandomizeNow => {
            match parse_randomize(state.drafts.density, &state.drafts.seed) {
                Ok(settings) => {
                    state.sim.app.set_randomize_settings(settings);
                    if *action == Action::RandomizeNow {
                        state.sim.app.randomize_board();
                    }
                    state.drafts.error = false;
                    let seed = settings
                        .seed()
                        .map(|seed| format!("seed {seed}"))
                        .unwrap_or_else(|| "a fresh seed each time".into());
                    state.drafts.feedback = if *action == Action::RandomizeNow {
                        format!(
                            "{} randomized at {}% with {seed}. U to undo.",
                            crate::shared::randomized_what(&state.sim.app),
                            settings.density_percent()
                        )
                    } else {
                        format!(
                            "Randomization set to {}% with {seed}.",
                            settings.density_percent()
                        )
                    };
                }
                Err(error) => {
                    state.drafts.error = true;
                    state.drafts.feedback = error.into();
                }
            }
        }
        Action::ExportRle => export_rle(&mut state),
        Action::SaveWorkspace => request_save(&mut state),
        Action::OpenWorkspace => request_open(&mut state),
        Action::OpenRle => {
            if !file_dialog_busy(&mut state) {
                request_file(&mut state, FileRequest::OpenRle);
            }
        }
        Action::ConfirmOpenWorkspace => confirm_open(&mut state),
        Action::CancelOpenWorkspace => {
            state.drafts.pending_open = None;
            state.mode.dialog = Some(DialogKind::Import);
            state.drafts.error = false;
            state.drafts.feedback = "Kept the current workspace.".into();
        }
        _ => {}
    }
}

fn parse_randomize(density: u8, seed: &str) -> Result<RandomizeSettings, &'static str> {
    let seed = if seed.trim().is_empty() {
        None
    } else {
        Some(seed.trim().parse::<u64>().map_err(|_| "Seed must be a whole number from 0 to 18446744073709551615, or blank for a fresh seed.")?)
    };
    RandomizeSettings::new(density, seed).ok_or("Density runs from 0 to 100%.")
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
    state.drafts.error = !state.sim.app.board.allows_rule(rule);
    if state.drafts.error {
        state.drafts.feedback = format!("{described} would fill the unbounded board.");
        return;
    }
    state.drafts.feedback = if state.sim.app.set_rule(rule) {
        format!("Rule set to {described}. U undoes the change.")
    } else {
        format!("The board already uses {described}.")
    };
}

fn export_rle(state: &mut DialogState) {
    let export = match state.sim.app.board_to_rle() {
        Ok(export) => export,
        Err(error) => {
            state.drafts.error = true;
            state.drafts.feedback = error.to_string();
            return;
        }
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
    mut readouts: Query<(&DialogReadout, &mut Text, &mut ThemedText)>,
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
    if std::mem::take(&mut state.drafts.rebuild) {
        *rendered = None;
    }
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
                color.set_if_neq(ThemedText(if state.drafts.error {
                    Token::Error
                } else {
                    Token::Accent
                }));
                state.drafts.feedback.clone()
            }
            DialogReadout::Diagnostics => {
                let size = match &state.sim.app.board {
                    Universe::Finite(board) => {
                        let (width, height) = board.size();
                        format!("{width} x {height}")
                    }
                    Universe::Unbounded(plane) => {
                        format!("unbounded, {} chunks", plane.chunk_count())
                    }
                };
                format!(
                    "Board                 {size}\nGeneration            {}\nLiving cells          {}\nRule                  {}\n\nAverage step          {:.3} ms\nTick interval         {}\nMeasured rate         {:.1} gen/s\nSamples               {}\n\nPlayback continues while this view is open.",
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
                )
            }
            DialogReadout::BoardNote => if state.sim.app.board.finite().is_some() {
                "Resizing keeps the cells that fit and clears undo history."
            } else {
                "The unbounded board has no size. This is the one it returns to when you choose Wrap or Fixed."
            }
            .to_owned(),
        };
        if text.0 != value {
            text.0 = value;
        }
    }
}

fn build_dialog(commands: &mut Commands, kind: DialogKind, drafts: &Drafts) {
    commands.spawn((DialogRoot, Node {
        position_type: PositionType::Absolute, width: percent(100), height: percent(100),
        align_items: AlignItems::Center, justify_content: JustifyContent::Center,
        padding: UiRect::all(px(20)), ..default()
    }, ThemedBackground(Token::Scrim), GlobalZIndex(50)))
    .with_children(|p| {
        p.spawn((TabGroup::modal(), super::accessibility::dialog_node(kind.title()), Node {
            width: px(if kind == DialogKind::Help { 760 } else { 540 }), max_width: percent(100), max_height: percent(100),
            flex_direction: FlexDirection::Column, padding: UiRect::all(px(22)), row_gap: px(16),
            border: UiRect::all(px(1)), border_radius: BorderRadius::all(px(14)), ..default()
        }, ThemedBackground(Token::Panel), ThemedBorder(Token::Line))).with_children(|p| {
            p.spawn(row()).with_children(|p| {
                p.spawn(label(kind.title(), 24.0, Token::Ink));
                p.spawn(Node { flex_grow: 1.0, ..default() });
                let close_button = button(p, "Close", Action::CloseDialog);
                if matches!(kind, DialogKind::Export | DialogKind::Help | DialogKind::Diagnostics) { p.commands().entity(close_button).insert(AutoFocus); }
            });
            // The body scrolls with a visible scrollbar; the title and feedback stay put.
            p.spawn(Node { min_height: px(0), column_gap: px(10), ..default() }).with_children(|p| {
            let body = p.spawn((ScrollArea, super::accessibility::named_node(accesskit::Role::GenericContainer, ""), Node { flex_direction: FlexDirection::Column, row_gap: px(12),
                flex_grow: 1.0, min_width: px(0), overflow: Overflow::scroll_y(), min_height: px(0), ..default() })).with_children(|p| {
                match kind {
                    DialogKind::Import => {
                        heading(p, "Pattern", if WEB_BUILD { "Paste RLE to add a stamp to your palette. A rule in the header also updates the board rule." } else { "Paste RLE or open an .rle file to add a stamp to your palette. A rule in the header also updates the board rule." });
                        if !WEB_BUILD { p.spawn(row()).with_children(|p| { button(p, "Open .rle file…", Action::OpenRle); }); }
                        field(p, "Pattern name", &drafts.name, Field::Name, false, true);
                        field(p, "RLE source", &drafts.source, Field::Source, true, false);
                        import_preview(p);
                        p.spawn(row()).with_children(|p| { button(p, "Import and select", Action::ImportSubmit); });
                        if !WEB_BUILD {
                            heading(p, "Workspace file", &format!("Open a .golw file to replace your board and imported patterns. It asks first. {}+O opens.", command_key()));
                            p.spawn(row()).with_children(|p| { button(p, "Open workspace…", Action::OpenWorkspace); });
                        }
                    }
                    DialogKind::Export => {
                        heading(p, "Pattern", "Copy the live cells, cropped to their bounding box, as RLE with the board rule.");
                        p.spawn(row()).with_children(|p| { button(p, "Copy board as RLE", Action::ExportRle); });
                        if !WEB_BUILD {
                            heading(p, "Workspace file", &format!("Save your board and imported patterns to a .golw file. {}+S saves.", command_key()));
                            p.spawn(row()).with_children(|p| { button(p, "Save workspace…", Action::SaveWorkspace); });
                        }
                    }
                    DialogKind::Settings => {
                        let note = heading(p, "Board", "");
                        p.commands().entity(note).insert(DialogReadout::BoardNote);
                        switch(p, "Fill canvas", "Size the board to the canvas, and follow the window and palette", Action::FillCanvas);
                        p.spawn(row()).with_children(|p| {
                            for (size, _) in BOARD_SIZE_PRESETS { button(p, &format!("{size}²"), Action::BoardPreset(size)); }
                        });
                        apply_row(p, Action::ResizeBoard, |p| {
                            for (title, value, kind) in [("Width (cells)", &drafts.width, Field::Width), ("Height (cells)", &drafts.height, Field::Height)] {
                                inline_field(p, title, value, kind, matches!(kind, Field::Width) && !drafts.rule_first);
                            }
                        });

                        heading(p, "Simulation", "Neighbour counts that give birth (B) and let a cell survive (S). Changing the rule can be undone.");
                        p.spawn(row()).with_children(|p| {
                            for (index, (name, _)) in RULE_PRESETS.iter().enumerate() { button(p, name, Action::RulePreset(index)); }
                        });
                        apply_row(p, Action::ApplyRule, |p| inline_field(p, "Rulestring", &drafts.rule, Field::Rule, drafts.rule_first));
                        setting(p, "Edges", |p| segmented(p, &[("Wrap", Action::Edges(BoundaryMode::Wrap)), ("Fixed", Action::Edges(BoundaryMode::Fixed)), ("Unbounded", Action::Unbounded)]));
                        switch(p, "Pause when settled", "Stop once the board stops changing or starts repeating", Action::PauseWhenSettled);
                        p.spawn(row()).with_children(|p| { button(p, "Reset generation counter", Action::ResetEpoch); });

                        heading(p, "Randomization", "Each cell has this chance of being alive. The same density, seed, and board size repeat the same board; leave the seed blank for a fresh board each time.");
                        setting(p, "Density", |p| density_slider(p, drafts.density));
                        apply_row(p, Action::ApplyRandomize, |p| inline_field(p, "Seed (optional whole number)", &drafts.seed, Field::Seed, false));
                        p.spawn(row()).with_children(|p| { button(p, "Randomize now", Action::RandomizeNow); });

                        heading(p, "Display", "Colours of the board, the pattern library, and the interface.");
                        let themes = Theme::ALL.map(|theme| (theme.label(), Action::Theme(theme)));
                        setting(p, "Theme", |p| segmented(p, &themes));
                        switch(p, "Grid", "Lines between cells · T", Action::Grid);
                        switch(p, "Cell age", "Brighten newborn cells, deepen old ones, and fade where cells died", Action::CellAge);

                        heading(p, "Developer", "Step timing and simulation counters, updated while the simulation runs.");
                        p.spawn(row()).with_children(|p| { button(p, "Show diagnostics", Action::Diagnostics); });
                    }
                    DialogKind::Help => {
                        p.spawn(label("Choose a stamp or draw cells, then press Space to play. U undoes an edit and pauses; one drag counts as one edit.", 16.0, Token::Ink));
                        p.spawn(label("Tab moves between controls; the board is the last stop, and while it is outlined Space and the other shortcuts apply to it. Escape closes a dialog. Simulation shortcuts are disabled while a dialog is open.", 14.0, Token::Muted));
                        help_columns(p);
                        p.spawn(label("Import adds RLE patterns to the palette and opens workspace files. Export copies the board as RLE and saves workspace files.", 14.0, Token::Muted));
                    }
                    DialogKind::ConfirmOpen => {
                        let file = drafts.pending_open.as_ref()
                            .and_then(|(path, _)| path.file_name())
                            .map_or_else(|| "this file".into(), |name| name.to_string_lossy());
                        p.spawn(label(&format!("Replace the current board, generation, rule, and imported patterns with {file}?"), 16.0, Token::Ink));
                        p.spawn(label("Playback pauses, undo history is cleared, and Fill canvas turns off. Unsaved changes are lost.", 14.0, Token::Muted));
                        p.spawn(row()).with_children(|p| {
                            button(p, "Replace workspace", Action::ConfirmOpenWorkspace);
                            // Cancel is the safe default for Enter.
                            let cancel = button(p, "Cancel", Action::CancelOpenWorkspace);
                            p.commands().entity(cancel).insert(AutoFocus);
                        });
                    }
                    DialogKind::Diagnostics => {
                        p.spawn((Text::default(), TextFont { font_size: FontSize::Px(14.0), ..default() },
                            ThemedText(Token::Ink), DialogReadout::Diagnostics, super::accessibility::named_node(accesskit::Role::Label, "Diagnostics"), Pickable::IGNORE));
                    }
                }
            }).id();
            super::palette::scrollbar(p, body);
            });
            if kind.shows_feedback() {
                p.spawn((label("", 14.0, Token::Accent), DialogReadout::Feedback, super::accessibility::feedback_node()));
            }
        });
    });
}
