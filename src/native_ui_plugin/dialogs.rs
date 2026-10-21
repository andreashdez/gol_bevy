//! Native editors and modal input routing. Drafts survive closing and reopening.
use super::*;
use crate::workspace::{DecodedWorkspace, Workspace};
use crate::{
    board::{
        BoundaryMode, RandomizeSettings, Rule, frontier_dense_threshold_percent_for_diagnostics,
    },
    shared::{
        BOARD_SIZE_MAX, BOARD_SIZE_MIN, BOARD_SIZE_PRESETS, BoardSettings, DiagnosticsOverlay,
        HelpSection, command_key, help_sections,
    },
};
use bevy::{
    clipboard::Clipboard,
    input_focus::AutoFocus,
    tasks::{IoTaskPool, Task, futures::check_ready},
    text::{EditableText, TextEdit},
    ui_widgets::{
        SelectAllOnFocus, Slider, SliderPrecision, SliderRange, SliderStep, SliderThumb,
        SliderValue, TextInput, TrackClick, ValueChange,
    },
};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DialogKind {
    Import,
    Settings,
    Help,
    Diagnostics,
    /// Asks before a validated workspace file replaces the current one.
    ConfirmOpen,
}

impl DialogKind {
    fn title(self) -> &'static str {
        match self {
            Self::Import => "Import / Export and workspaces",
            Self::Settings => "Workspace settings",
            Self::Help => "Controls and tips",
            Self::Diagnostics => "Simulation diagnostics",
            Self::ConfirmOpen => "Open workspace?",
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
        Action::Settings | Action::EditRule => {
            state.drafts.rule_first = *action == Action::EditRule;
            let (width, height) = state.sim.app.board.size();
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
                state.drafts.feedback = format!(
                    "Board size set to {width} × {height}. Your choice is saved automatically."
                );
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
                            "Board randomized at {}% with {seed}. U to undo.",
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

enum FileRequest {
    /// Captured at the click, so the file holds what was on screen then, even
    /// if playback continues while the user browses.
    Save(Box<Workspace>),
    Open,
}

/// The native file dialog in progress, if any. It runs off the main thread so
/// the app keeps drawing while the user browses.
#[derive(Resource, Default)]
pub(super) struct FileDialog(Option<(FileRequest, Task<Option<PathBuf>>)>);

fn request_save(state: &mut DialogState) {
    if file_dialog_busy(state) {
        return;
    }
    match state.sim.app.workspace() {
        Ok(workspace) => request_file(state, FileRequest::Save(Box::new(workspace))),
        Err(error) => {
            state.drafts.error = true;
            state.drafts.feedback = format!("Could not save the workspace: {error}");
        }
    }
}

fn request_open(state: &mut DialogState) {
    if !file_dialog_busy(state) {
        request_file(state, FileRequest::Open);
    }
}

fn file_dialog_busy(state: &mut DialogState) -> bool {
    let busy = state.file_dialog.0.is_some();
    if busy {
        state.drafts.error = false;
        state.drafts.feedback = "A file dialog is already open.".into();
    }
    busy
}

fn request_file(state: &mut DialogState, request: FileRequest) {
    let previous = state.drafts.workspace_path.as_deref();
    let directory = previous
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(dirs::document_dir);
    let file_name = previous
        .and_then(Path::file_name)
        .map_or_else(|| "workspace.golw".into(), |name| name.to_string_lossy());
    let mut dialog = rfd::AsyncFileDialog::new().add_filter("Game of Life workspace", &["golw"]);
    if let Some(directory) = directory {
        dialog = dialog.set_directory(directory);
    }
    // The system dialog asks before replacing an existing file.
    let chosen: std::pin::Pin<Box<dyn Future<Output = Option<rfd::FileHandle>> + Send>> =
        match request {
            FileRequest::Save(_) => Box::pin(dialog.set_file_name(file_name).save_file()),
            FileRequest::Open => Box::pin(dialog.pick_file()),
        };
    let task =
        IoTaskPool::get().spawn(async move { chosen.await.map(|file| file.path().to_owned()) });
    state.drafts.error = false;
    state.drafts.feedback = match request {
        FileRequest::Save(_) => "Choose where to save the workspace.",
        FileRequest::Open => "Choose a workspace file to open.",
    }
    .into();
    state.file_dialog.0 = Some((request, task));
}

pub(super) fn poll_file_dialog(mut state: DialogState) {
    let Some((_, task)) = state.file_dialog.0.as_mut() else {
        return;
    };
    let Some(chosen) = check_ready(task) else {
        return;
    };
    let Some((request, _)) = state.file_dialog.0.take() else {
        return;
    };
    finish_file_request(&mut state, request, chosen);
}

fn finish_file_request(state: &mut DialogState, request: FileRequest, chosen: Option<PathBuf>) {
    let Some(path) = chosen else {
        state.drafts.error = false;
        state.drafts.feedback = "No file was chosen.".into();
        return;
    };
    let saving = matches!(request, FileRequest::Save(_));
    let result = match request {
        FileRequest::Save(workspace) => workspace
            .write(&path)
            .map(|()| format!("Workspace saved to {}.", path.display())),
        // Validate the whole file first, so only a usable workspace asks to replace yours.
        FileRequest::Open => Workspace::read(&path)
            .and_then(Workspace::decode)
            .map(|decoded| {
                state.drafts.pending_open = Some((path.clone(), decoded));
                state.mode.dialog = Some(DialogKind::ConfirmOpen);
                String::new()
            }),
    };
    match result {
        Ok(feedback) => {
            state.drafts.workspace_path = Some(path);
            state.drafts.error = false;
            state.drafts.feedback = feedback;
        }
        Err(error) => {
            let verb = if saving { "save" } else { "open" };
            state.drafts.error = true;
            state.drafts.feedback = format!("Could not {verb} {}: {error}", path.display());
        }
    }
}

fn confirm_open(state: &mut DialogState) {
    let Some((path, decoded)) = state.drafts.pending_open.take() else {
        return;
    };
    state.sim.app.restore_decoded(decoded);
    let (width, height) = state.sim.app.board.size();
    state.board.set(width, height);
    state.board.fill_canvas = false;
    state.sim.accumulator = std::time::Duration::ZERO;
    state.sim.avg_step_ms = 0.0;
    state.sim.step_samples = 0;
    state.sim.rate = Default::default();
    state.mode.dialog = Some(DialogKind::Import);
    state.drafts.error = false;
    state.drafts.feedback = format!(
        "Opened {}. Playback is paused; undo history cleared and Fill canvas off.",
        path.display()
    );
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
fn heading(parent: &mut ChildSpawnerCommands, title: &str, description: &str) {
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
            p.spawn(label(description, 14.0, Token::Muted));
        });
}

/// A single-line field: 10 px padding, a 1 px border, and 16 px text at
/// Bevy's 1.2 line height.
const FIELD_HEIGHT: f32 = 2.0 * 10.0 + 2.0 + 16.0 * 1.2;

/// A labelled text field that shares a row with others.
fn inline_field(
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
fn apply_row(
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
fn setting(
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
pub(super) struct Segment;

/// Mutually exclusive choices joined in one box, like Wrap and Fixed.
fn segmented(parent: &mut ChildSpawnerCommands, choices: &[(&str, Action)]) {
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
pub(super) struct Switch;
#[derive(Component)]
pub(super) struct SwitchTrack(Action);
#[derive(Component)]
pub(super) struct SwitchKnob;

fn switch(parent: &mut ChildSpawnerCommands, title: &str, description: &str, action: Action) {
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
pub(super) fn sync_switches(
    view: ViewState,
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
pub(super) struct DensitySlider;
#[derive(Component)]
pub(super) struct DensityFill;
#[derive(Component)]
pub(super) struct DensityReadout;

const SLIDER_THUMB: f32 = 18.0;

/// Density from 0 to 100%, with the value beside it. Arrow keys step by 1%.
fn density_slider(parent: &mut ChildSpawnerCommands, density: u8) {
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
fn density_changed(
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
pub(super) fn sync_density(
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

/// Width of the key caps beside each action, so the actions line up.
const HELP_KEY_COLUMN: f32 = 132.0;

/// Shortcut groups in two columns, which stack when the window is narrow:
/// Simulation and Editing, then Stamps, Camera, and Interface.
fn help_columns(parent: &mut ChildSpawnerCommands) {
    let mut sections = help_sections().into_iter();
    parent
        .spawn(Node {
            flex_wrap: FlexWrap::Wrap,
            column_gap: px(32),
            row_gap: px(20),
            flex_shrink: 0.0,
            ..default()
        })
        .with_children(|p| {
            for count in [2, 3] {
                p.spawn(Node {
                    flex_direction: FlexDirection::Column,
                    row_gap: px(20),
                    flex_grow: 1.0,
                    flex_basis: px(320),
                    min_width: px(0),
                    ..default()
                })
                .with_children(|p| {
                    for section in sections.by_ref().take(count) {
                        help_section(p, section);
                    }
                });
            }
        });
}

fn help_section(parent: &mut ChildSpawnerCommands, section: HelpSection) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(7),
            ..default()
        })
        .with_children(|p| {
            p.spawn(label(section.title, 16.0, Token::Ink));
            for entry in section.entries {
                p.spawn(Node {
                    align_items: AlignItems::Center,
                    column_gap: px(12),
                    ..default()
                })
                .with_children(|p| {
                    p.spawn(Node {
                        width: px(HELP_KEY_COLUMN),
                        flex_shrink: 0.0,
                        flex_wrap: FlexWrap::Wrap,
                        column_gap: px(4),
                        row_gap: px(4),
                        ..default()
                    })
                    .with_children(|p| {
                        for cap in entry.caps {
                            key_cap(p, &cap.replace("Mod", command_key()));
                        }
                    });
                    p.spawn(label(entry.action, 14.0, Token::Ink));
                });
            }
        });
}

/// A key or gesture drawn as a small raised key.
fn key_cap(parent: &mut ChildSpawnerCommands, text: &str) {
    parent
        .spawn((
            Node {
                min_width: px(24),
                padding: UiRect::axes(px(7), px(2)),
                justify_content: JustifyContent::Center,
                border: UiRect {
                    bottom: px(2),
                    ..UiRect::all(px(1))
                },
                border_radius: BorderRadius::all(px(5)),
                ..default()
            },
            ThemedBackground(Token::Surface),
            ThemedBorder(Token::Line),
            Pickable::IGNORE,
        ))
        .with_child(label(text, 12.0, Token::Ink));
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
                if matches!(kind, DialogKind::Help | DialogKind::Diagnostics) { p.commands().entity(close_button).insert(AutoFocus); }
            });
            // The body scrolls with a visible scrollbar; the title and feedback stay put.
            p.spawn(Node { min_height: px(0), column_gap: px(10), ..default() }).with_children(|p| {
            let body = p.spawn((ScrollArea, super::accessibility::named_node(accesskit::Role::GenericContainer, ""), Node { flex_direction: FlexDirection::Column, row_gap: px(12),
                flex_grow: 1.0, min_width: px(0), overflow: Overflow::scroll_y(), min_height: px(0), ..default() })).with_children(|p| {
                match kind {
                    DialogKind::Import => {
                        p.spawn(label("Paste RLE to add a stamp to your palette. A rule in the header also updates the board rule.", 14.0, Token::Muted));
                        field(p, "Pattern name", &drafts.name, Field::Name, false, true);
                        field(p, "RLE source", &drafts.source, Field::Source, true, false);
                        button(p, "Import and select", Action::ImportSubmit);
                        p.spawn(label("Export", 19.0, Token::Ink));
                        p.spawn(label("Copy the live cells, cropped to their bounding box, as RLE with the board rule.", 14.0, Token::Muted));
                        button(p, "Copy board as RLE", Action::ExportRle);
                        p.spawn(label("Workspace files", 19.0, Token::Ink));
                        p.spawn(label("Save your board and imported patterns to a .golw file, or open one to replace them. Opening asks first.", 14.0, Token::Muted));
                        p.spawn(row()).with_children(|p| {
                            button(p, "Save workspace…", Action::SaveWorkspace);
                            button(p, "Open workspace…", Action::OpenWorkspace);
                        });
                    }
                    DialogKind::Settings => {
                        heading(p, "Board", "Resizing keeps the cells that fit and clears undo history.");
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
                        setting(p, "Edges", |p| segmented(p, &[("Wrap", Action::Edges(BoundaryMode::Wrap)), ("Fixed", Action::Edges(BoundaryMode::Fixed))]));
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
                        p.spawn(label("Import / Export adds RLE patterns to the palette, copies the board as RLE, and saves or opens workspace files.", 14.0, Token::Muted));
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
            if matches!(kind, DialogKind::Import | DialogKind::Settings) {
                p.spawn((label("", 14.0, Token::Accent), DialogReadout::Feedback, super::accessibility::feedback_node()));
            }
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
        world.insert_resource(BoardSettings {
            cols: 32,
            rows: 32,
            fill_canvas: false,
        });
        world.insert_resource(DiagnosticsOverlay { enabled: false });
        world.insert_resource(InputFocus::default());
        world.insert_resource(ButtonInput::<KeyCode>::default());
        world.init_resource::<FileDialog>();
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
    fn the_status_bar_rule_opens_settings_with_the_rulestring_first() {
        let mut world = test_world(DialogKind::Settings);
        world.resource_mut::<NativeUi>().dialog = None;
        let rule = world.spawn(Action::EditRule).id();
        world.trigger(Activate { entity: rule });
        assert_eq!(
            world.resource::<NativeUi>().dialog,
            Some(DialogKind::Settings)
        );
        assert!(world.resource::<Drafts>().rule_first);
        assert_eq!(world.resource::<Drafts>().return_focus, Some(rule));
        // The gear opens the same dialog at the top.
        world.resource_mut::<NativeUi>().dialog = None;
        let gear = world.spawn(Action::Settings).id();
        world.trigger(Activate { entity: gear });
        assert!(!world.resource::<Drafts>().rule_first);
    }

    #[test]
    fn fill_canvas_toggles_and_an_explicit_size_switches_it_off() {
        let mut world = test_world(DialogKind::Settings);
        let fill = world.spawn(Action::FillCanvas).id();
        world.trigger(Activate { entity: fill });
        assert!(world.resource::<BoardSettings>().fill_canvas);
        world.trigger(Activate { entity: fill });
        assert!(!world.resource::<BoardSettings>().fill_canvas);

        world.trigger(Activate { entity: fill });
        world.spawn((Field::Width, EditableText::new("64")));
        world.spawn((Field::Height, EditableText::new("48")));
        let apply = world.spawn(Action::ResizeBoard).id();
        world.trigger(Activate { entity: apply });
        let board = *world.resource::<BoardSettings>();
        assert_eq!((board.cols, board.rows, board.fill_canvas), (64, 48, false));
        assert!(
            world
                .resource::<Drafts>()
                .feedback
                .contains("Fill canvas is now off")
        );
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

    /// Completes a file dialog as if the user had chosen `path`.
    fn choose(world: &mut World, request: FileRequest, path: Option<&Path>) {
        let path = path.map(Path::to_path_buf);
        let mut request = Some(request);
        world
            .run_system_once(move |mut state: DialogState| {
                let request = request.take().expect("the system runs once");
                finish_file_request(&mut state, request, path.clone());
            })
            .unwrap();
    }

    /// A save request as the Save button captures it from the current app.
    fn save_request(world: &World) -> FileRequest {
        let workspace = world.resource::<Simulation>().app.workspace().unwrap();
        FileRequest::Save(Box::new(workspace))
    }

    #[test]
    fn opening_a_workspace_validates_first_then_asks_before_replacing() {
        let directory = crate::test_support::TestDirectory::new();
        let path = directory.0.join("workspace.golw");
        let mut world = test_world(DialogKind::Import);
        let sim = |world: &World| world.resource::<Simulation>().app.board.is_alive(31, 30);
        world
            .resource_mut::<Simulation>()
            .app
            .populate_board(31, 30);
        world.resource_mut::<Simulation>().app.epoch = 42;
        let request = save_request(&world);
        choose(&mut world, request, Some(&path));
        assert!(!world.resource::<Drafts>().error);
        assert_eq!(
            world.resource::<Drafts>().workspace_path.as_deref(),
            Some(path.as_path())
        );

        // A cancelled system dialog changes nothing.
        world.resource_mut::<Simulation>().app.resize_board(8, 8);
        world.resource_mut::<Simulation>().app.toggle_running();
        choose(&mut world, FileRequest::Open, None);
        assert_eq!(
            world.resource::<NativeUi>().dialog,
            Some(DialogKind::Import)
        );

        // Choosing a valid file asks first, and Cancel keeps the current workspace.
        choose(&mut world, FileRequest::Open, Some(&path));
        assert_eq!(
            world.resource::<NativeUi>().dialog,
            Some(DialogKind::ConfirmOpen)
        );
        assert_eq!(world.resource::<Simulation>().app.board.size(), (8, 8));
        let cancel = world.spawn(Action::CancelOpenWorkspace).id();
        world.trigger(Activate { entity: cancel });
        assert_eq!(
            world.resource::<NativeUi>().dialog,
            Some(DialogKind::Import)
        );
        assert!(world.resource::<Drafts>().pending_open.is_none());
        assert_eq!(world.resource::<Simulation>().app.board.size(), (8, 8));
        assert!(world.resource::<Simulation>().app.is_running());

        // Closing the prompt (Escape or Close) also drops the pending file.
        choose(&mut world, FileRequest::Open, Some(&path));
        let close_button = world.spawn(Action::CloseDialog).id();
        world.trigger(Activate {
            entity: close_button,
        });
        assert!(world.resource::<Drafts>().pending_open.is_none());
        assert_eq!(world.resource::<Simulation>().app.board.size(), (8, 8));

        // Confirming replaces the workspace and resets playback state.
        world.resource_mut::<NativeUi>().dialog = Some(DialogKind::Import);
        world.resource_mut::<Simulation>().accumulator = std::time::Duration::from_secs(2);
        world.resource_mut::<BoardSettings>().fill_canvas = true;
        choose(&mut world, FileRequest::Open, Some(&path));
        let replace = world.spawn(Action::ConfirmOpenWorkspace).id();
        world.trigger(Activate { entity: replace });
        assert_eq!(
            world.resource::<NativeUi>().dialog,
            Some(DialogKind::Import)
        );
        assert!(!world.resource::<Drafts>().error);
        assert!(sim(&world));
        let board = *world.resource::<BoardSettings>();
        assert_eq!((board.cols, board.rows, board.fill_canvas), (32, 32, false));
        let state = world.resource::<Simulation>();
        assert!(!state.app.is_running() && !state.app.can_undo());
        assert_eq!(state.app.epoch, 42);
        assert_eq!(state.accumulator, std::time::Duration::ZERO);

        // An invalid file reports an error without asking or changing anything.
        std::fs::write(&path, "invalid").unwrap();
        choose(&mut world, FileRequest::Open, Some(&path));
        assert!(world.resource::<Drafts>().error);
        assert_eq!(
            world.resource::<NativeUi>().dialog,
            Some(DialogKind::Import)
        );
        assert!(world.resource::<Drafts>().pending_open.is_none());
        assert!(sim(&world));

        // The system dialog confirms replacement, so saving over a file just writes it.
        let request = save_request(&world);
        choose(&mut world, request, Some(&path));
        assert!(!world.resource::<Drafts>().error);
        assert!(crate::workspace::Workspace::read(&path).is_ok());
        let request = save_request(&world);
        choose(
            &mut world,
            request,
            Some(&directory.0.join("missing/w.golw")),
        );
        assert!(world.resource::<Drafts>().error);
    }

    #[test]
    fn saving_writes_the_workspace_as_it_was_when_save_was_clicked() {
        let directory = crate::test_support::TestDirectory::new();
        let path = directory.0.join("workspace.golw");
        let mut world = test_world(DialogKind::Import);
        world.resource_mut::<Simulation>().app.populate_board(3, 4);
        world.resource_mut::<Simulation>().app.epoch = 7;
        let request = save_request(&world);
        // Playback and edits continue while the user browses for a file.
        world.resource_mut::<Simulation>().app.clear_board();
        world.resource_mut::<Simulation>().app.epoch = 99;
        choose(&mut world, request, Some(&path));
        let mut restored = GolApp::new(8, 8);
        restored
            .restore_workspace(crate::workspace::Workspace::read(&path).unwrap())
            .unwrap();
        assert!(restored.board.is_alive(3, 4));
        assert_eq!(restored.epoch, 7);
    }

    #[test]
    fn density_applies_as_the_slider_moves_and_the_seed_validates_before_applying() {
        let mut world = test_world(DialogKind::Settings);
        world.add_observer(density_changed);
        let slider = world.spawn((DensitySlider, SliderValue(25.0))).id();
        let seed = world.spawn((Field::Seed, EditableText::new("42"))).id();
        let apply = world.spawn(Action::ApplyRandomize).id();
        let randomize = world.spawn(Action::RandomizeNow).id();
        let settings = |world: &World| world.resource::<Simulation>().app.randomize_settings();
        let slide = |world: &mut World, value: f32| {
            world.trigger(ValueChange {
                source: slider,
                value,
                is_final: true,
            });
            world.flush();
        };

        // Moving the slider applies the density at once, without the pending seed.
        slide(&mut world, 70.0);
        assert_eq!(settings(&world), RandomizeSettings::new(70, None).unwrap());
        assert_eq!(world.get::<SliderValue>(slider), Some(&SliderValue(70.0)));
        world.trigger(Activate { entity: apply });
        assert!(!world.resource::<Drafts>().error);
        assert_eq!(
            settings(&world),
            RandomizeSettings::new(70, Some(42)).unwrap()
        );
        // Applying leaves the board and history alone.
        assert_eq!(
            world.resource::<Simulation>().app.board.live_cells_count(),
            0
        );
        assert!(!world.resource::<Simulation>().app.can_undo());

        // The same density and seed repeat the same board.
        world.trigger(Activate { entity: randomize });
        let original = world.resource::<Simulation>().app.board.snapshot_cells();
        world.trigger(Activate { entity: randomize });
        assert_eq!(
            world.resource::<Simulation>().app.board.snapshot_cells(),
            original
        );

        // An invalid seed changes nothing.
        world.entity_mut(seed).insert(EditableText::new("-1"));
        world.trigger(Activate { entity: randomize });
        assert!(world.resource::<Drafts>().error);
        assert_eq!(
            world.resource::<Simulation>().app.board.snapshot_cells(),
            original
        );
        assert_eq!(
            settings(&world),
            RandomizeSettings::new(70, Some(42)).unwrap()
        );

        // The slider keeps the applied seed and stays within 0–100%.
        slide(&mut world, 130.0);
        assert_eq!(
            settings(&world),
            RandomizeSettings::new(100, Some(42)).unwrap()
        );
        slide(&mut world, 0.0);
        world.entity_mut(seed).insert(EditableText::new(""));
        world.trigger(Activate { entity: randomize });
        assert!(!world.resource::<Drafts>().error);
        assert_eq!(
            world.resource::<Simulation>().app.board.live_cells_count(),
            0
        );
        assert_eq!(settings(&world).seed(), None);
        assert!(world.resource_mut::<Simulation>().app.undo());
        assert_eq!(
            world.resource::<Simulation>().app.board.snapshot_cells(),
            original
        );
    }

    #[test]
    fn seeds_accept_their_endpoints_and_reject_invalid_numbers() {
        assert_eq!(parse_randomize(100, " 0 ").unwrap().seed(), Some(0));
        assert_eq!(
            parse_randomize(0, "18446744073709551615").unwrap().seed(),
            Some(u64::MAX)
        );
        assert_eq!(parse_randomize(25, "  ").unwrap().seed(), None);
        for seed in ["1.5", "-1", "18446744073709551616", "abc"] {
            assert!(parse_randomize(25, seed).is_err(), "{seed}");
        }
        assert!(parse_randomize(101, "").is_err());
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
