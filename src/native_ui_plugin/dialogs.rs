//! Native editors and modal input routing. Drafts survive closing and reopening.
use super::*;
use crate::workspace::{DecodedWorkspace, Workspace};
use crate::{
    board::{Rule, frontier_dense_threshold_percent_for_diagnostics},
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
    ui_widgets::{SelectAllOnFocus, TextInput},
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
    density: String,
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
            density: "25".into(),
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
    Density,
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
            .add_observer(dialog_action);
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
            Field::Density => &mut drafts.density,
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
            state.drafts.density = random.density_percent().to_string();
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
            match parse_randomize(&state.drafts.density, &state.drafts.seed) {
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

fn parse_randomize(
    density: &str,
    seed: &str,
) -> Result<crate::board::RandomizeSettings, &'static str> {
    let density = density
        .trim()
        .parse::<u8>()
        .ok()
        .filter(|&value| value <= 100)
        .ok_or("Enter a whole density percentage from 0 to 100.")?;
    let seed = if seed.trim().is_empty() {
        None
    } else {
        Some(seed.trim().parse::<u64>().map_err(|_| "Seed must be a whole number from 0 to 18446744073709551615, or blank for a fresh seed.")?)
    };
    Ok(crate::board::RandomizeSettings::new(density, seed).expect("validated density"))
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
            p.spawn((ScrollArea, super::accessibility::named_node(accesskit::Role::GenericContainer, ""), Node { flex_direction: FlexDirection::Column, row_gap: px(12),
                overflow: Overflow::scroll_y(), min_height: px(0), ..default() })).with_children(|p| {
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
                        p.spawn(label("Board dimensions", 19.0, Token::Ink));
                        p.spawn(label("Resizing preserves cells that fit and clears undo history. Window resizing leaves the board size unchanged, unless Fill canvas is on.", 14.0, Token::Muted));
                        p.spawn(row()).with_children(|p| {
                            for (size, _) in BOARD_SIZE_PRESETS { button(p, &format!("{size}²"), Action::BoardPreset(size)); }
                            button(p, "Fill canvas", Action::FillCanvas);
                        });
                        p.spawn(Node { column_gap: px(12), flex_shrink: 0.0, ..default() }).with_children(|p| {
                            for (title, value, kind) in [("Width (cells)", &drafts.width, Field::Width), ("Height (cells)", &drafts.height, Field::Height)] {
                                p.spawn(Node { flex_direction: FlexDirection::Column, row_gap: px(12), flex_grow: 1.0, flex_basis: px(0), min_width: px(0), ..default() })
                                    .with_children(|p| field(p, title, value, kind, false, matches!(kind, Field::Width) && !drafts.rule_first));
                            }
                        });
                        button(p, "Apply board size", Action::ResizeBoard);
                        p.spawn(label("Rule", 19.0, Token::Ink));
                        p.spawn(label("Neighbour counts that give birth (B) and let a cell survive (S). Changing the rule can be undone.", 14.0, Token::Muted));
                        p.spawn(row()).with_children(|p| {
                            for (index, (name, _)) in RULE_PRESETS.iter().enumerate() { button(p, name, Action::RulePreset(index)); }
                        });
                        field(p, "Rulestring", &drafts.rule, Field::Rule, false, drafts.rule_first);
                        button(p, "Apply rule", Action::ApplyRule);
                        p.spawn(label("Randomization", 19.0, Token::Ink));
                        p.spawn(label("Each cell has this chance of being alive. The same density, seed, and board dimensions repeat the same starting board. Leave seed blank for a fresh board each time.", 14.0, Token::Muted));
                        field(p, "Density (whole %, 0–100)", &drafts.density, Field::Density, false, false);
                        field(p, "Seed (optional whole number)", &drafts.seed, Field::Seed, false, false);
                        p.spawn(row()).with_children(|p| {
                            button(p, "Apply randomization", Action::ApplyRandomize);
                            button(p, "Randomize now", Action::RandomizeNow);
                        });
                        p.spawn(label("Theme", 19.0, Token::Ink));
                        p.spawn(label("Colours of the board, the pattern library, and the interface.", 14.0, Token::Muted));
                        p.spawn(row()).with_children(|p| {
                            for theme in Theme::ALL { button(p, theme.label(), Action::Theme(theme)); }
                        });
                        p.spawn(row()).with_children(|p| {
                            button(p, "Grid", Action::Grid);
                            button(p, "Cell age", Action::CellAge);
                            button(p, "Edges", Action::Boundary);
                            button(p, "Diagnostics", Action::Diagnostics);
                        });
                        button(p, "Reset generation counter", Action::ResetEpoch);
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
                if matches!(kind, DialogKind::Import | DialogKind::Settings) {
                    p.spawn((label("", 14.0, Token::Accent), DialogReadout::Feedback, super::accessibility::feedback_node()));
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
    fn randomization_fields_validate_before_applying_and_randomize_repeatably() {
        let mut world = test_world(DialogKind::Settings);
        let density = world.spawn((Field::Density, EditableText::new("70"))).id();
        let seed = world.spawn((Field::Seed, EditableText::new("42"))).id();
        let apply = world.spawn(Action::ApplyRandomize).id();
        let randomize = world.spawn(Action::RandomizeNow).id();
        world.trigger(Activate { entity: apply });
        assert!(!world.resource::<Drafts>().error);
        assert_eq!(
            world.resource::<Simulation>().app.board.live_cells_count(),
            0
        );
        assert!(!world.resource::<Simulation>().app.can_undo());
        let settings = crate::board::RandomizeSettings::new(70, Some(42)).unwrap();
        assert_eq!(
            world.resource::<Simulation>().app.randomize_settings(),
            settings
        );
        world.trigger(Activate { entity: randomize });
        let original = world.resource::<Simulation>().app.board.snapshot_cells();
        world.trigger(Activate { entity: randomize });
        assert_eq!(
            world.resource::<Simulation>().app.board.snapshot_cells(),
            original
        );
        world.entity_mut(seed).insert(EditableText::new("-1"));
        world.trigger(Activate { entity: randomize });
        assert!(world.resource::<Drafts>().error);
        assert_eq!(
            world.resource::<Simulation>().app.board.snapshot_cells(),
            original
        );
        assert_eq!(
            world.resource::<Simulation>().app.randomize_settings(),
            settings
        );
        world.entity_mut(density).insert(EditableText::new("101"));
        world.entity_mut(seed).insert(EditableText::new(""));
        world.trigger(Activate { entity: apply });
        assert!(world.resource::<Drafts>().error);
        assert_eq!(
            world.resource::<Simulation>().app.randomize_settings(),
            settings
        );
        world.entity_mut(density).insert(EditableText::new("0"));
        world.trigger(Activate { entity: randomize });
        assert!(!world.resource::<Drafts>().error);
        assert_eq!(
            world.resource::<Simulation>().app.board.live_cells_count(),
            0
        );
        assert_eq!(
            world
                .resource::<Simulation>()
                .app
                .randomize_settings()
                .seed(),
            None
        );
        assert!(world.resource_mut::<Simulation>().app.undo());
        assert_eq!(
            world.resource::<Simulation>().app.board.snapshot_cells(),
            original
        );
    }

    #[test]
    fn randomization_fields_accept_seed_endpoints_and_reject_invalid_numbers() {
        assert_eq!(parse_randomize(" 100 ", " 0 ").unwrap().seed(), Some(0));
        assert_eq!(
            parse_randomize("0", "18446744073709551615").unwrap().seed(),
            Some(u64::MAX)
        );
        assert_eq!(parse_randomize("25", "  ").unwrap().seed(), None);
        for (density, seed) in [
            ("", ""),
            ("101", ""),
            ("-1", ""),
            ("25.5", ""),
            ("25", "1.5"),
            ("25", "-1"),
            ("25", "18446744073709551616"),
        ] {
            assert!(parse_randomize(density, seed).is_err(), "{density}, {seed}");
        }
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
