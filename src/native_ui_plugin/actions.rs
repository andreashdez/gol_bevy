//! Toolbar and shortcut actions and how they apply to the app
use super::*;

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Action {
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
    /// Saves the selection, or else the board, as a PNG.
    SaveImage,
    /// Records generations of the selection, or else the board, as a GIF.
    SaveGif,
    /// Records the selection, or else the board, until it comes back round.
    SaveGifPeriod,
    /// Stops the GIF being recorded, saving nothing.
    CancelRecording,
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
    Unbounded,
    /// Switches between wrapping and fixed edges, from the status bar.
    ToggleEdges,
}

/// Board display options that buttons switch on and off.
#[derive(SystemParam)]
pub(super) struct ViewToggles<'w> {
    grid: ResMut<'w, GridOverlay>,
    cell_age: ResMut<'w, CellAge>,
}

pub(super) fn activate(
    event: On<Activate>,
    actions: Query<&Action>,
    mut sim: ResMut<Simulation>,
    mut view: ViewToggles,
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
        | Action::SaveImage
        | Action::SaveGif
        | Action::SaveGifPeriod
        | Action::CancelRecording
        | Action::OpenWorkspace
        | Action::OpenRle
        | Action::ConfirmOpenWorkspace
        | Action::CancelOpenWorkspace
        | Action::ApplyRandomize
        | Action::RandomizeNow => {}
        Action::Grid => view.grid.enabled = !view.grid.enabled,
        Action::CellAge => view.cell_age.enabled = !view.cell_age.enabled,
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

pub(super) fn apply_action(action: Action, app: &mut GolApp) {
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
        // Disabled for the one rule that refuses, so the error can't reach here.
        Action::Unbounded => {
            let _ = app.set_unbounded();
        }
        Action::ToggleEdges => app.toggle_boundary_mode(),
        Action::ResetEpoch => app.reset_epoch(),
        Action::PauseWhenSettled => app.set_pause_when_settled(!app.pause_when_settled()),
        _ => {}
    }
}
