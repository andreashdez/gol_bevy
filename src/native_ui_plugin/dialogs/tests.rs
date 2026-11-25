use super::files::{
    FileShortcut, FileWork, file_shortcut, finish_file_request, finish_file_work, request_save,
};
use super::import_preview::import_summary;
use super::widgets::DensitySlider;
use super::*;
use crate::{cell_gif::GifLength, workspace::Workspace};
use bevy::ecs::system::RunSystemOnce;
use bevy::ui_widgets::{SliderValue, ValueChange};
use std::path::Path;

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
    world.init_resource::<crate::keybindings::TypedKeys>();
    world.init_resource::<FileDialog>();
    world.init_resource::<crate::feedback::EditFeedback>();
    world.init_resource::<Theme>();
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
fn help_closes_on_the_typed_slash_not_the_physical_slash_key() {
    let mut world = test_world(DialogKind::Help);
    world
        .resource_mut::<Simulation>()
        .app
        .set_help_visible(true);
    // German layout: the physical Slash key types `-`.
    world
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Slash);
    world
        .resource_mut::<crate::keybindings::TypedKeys>()
        .press("-");
    world.run_system_once(update_dialogs).unwrap();
    assert_eq!(world.resource::<NativeUi>().dialog, Some(DialogKind::Help));

    // Shift+7 types `/` there.
    world.insert_resource(ButtonInput::<KeyCode>::default());
    let mut typed = crate::keybindings::TypedKeys::default();
    typed.press("/");
    world.insert_resource(typed);
    world.run_system_once(update_dialogs).unwrap();
    assert!(world.resource::<NativeUi>().dialog.is_none());
    assert!(!world.resource::<Simulation>().app.is_help_visible());
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
fn import_and_export_open_their_own_dialogs_and_return_focus() {
    let mut world = test_world(DialogKind::Settings);
    world.resource_mut::<NativeUi>().dialog = None;
    let close_button = world.spawn(Action::CloseDialog).id();
    for (action, kind) in [
        (Action::Import, DialogKind::Import),
        (Action::Export, DialogKind::Export),
    ] {
        let opener = world.spawn(action).id();
        world.resource_mut::<Drafts>().feedback = "stale".into();
        world.trigger(Activate { entity: opener });
        assert_eq!(world.resource::<NativeUi>().dialog, Some(kind));
        assert!(world.resource::<Drafts>().feedback.is_empty());

        world.trigger(Activate {
            entity: close_button,
        });
        assert_eq!(world.resource::<NativeUi>().dialog, None);
        assert_eq!(world.resource::<InputFocus>().get(), Some(opener));
    }
}

#[test]
fn file_results_stay_in_the_export_dialog_instead_of_covering_the_board() {
    let mut world = test_world(DialogKind::Export);
    let request = save_request(&world);
    choose(&mut world, request, None);
    assert_eq!(world.resource::<Drafts>().feedback, "No file was chosen.");
    assert!(
        world
            .resource::<crate::feedback::EditFeedback>()
            .message
            .is_empty()
    );
}

#[test]
fn export_reports_an_empty_board_or_a_missing_clipboard() {
    let mut world = test_world(DialogKind::Export);
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
fn export_reaches_the_clipboard_from_an_unbounded_board() {
    let mut world = test_world(DialogKind::Export);
    let app = &mut world.resource_mut::<Simulation>().into_inner().app;
    app.set_unbounded().unwrap();
    let export = world.spawn(Action::ExportRle).id();
    world.trigger(Activate { entity: export });
    assert!(world.resource::<Drafts>().feedback.contains("empty"));

    world.resource_mut::<Simulation>().app.populate_board(-3, 3);
    world.trigger(Activate { entity: export });
    assert!(world.resource::<Drafts>().feedback.contains("clipboard"));
}

#[test]
fn the_board_section_says_what_its_size_means_on_each_board() {
    let mut world = test_world(DialogKind::Settings);
    world.init_resource::<Theme>();
    let note = world
        .spawn((
            DialogReadout::BoardNote,
            Text::default(),
            ThemedText(Token::Muted),
        ))
        .id();
    let text = |world: &mut World| {
        world.run_system_once(update_dialogs).unwrap();
        world.get::<Text>(note).unwrap().0.clone()
    };
    assert!(text(&mut world).contains("keeps the cells that fit"));
    let app = &mut world.resource_mut::<Simulation>().into_inner().app;
    app.set_unbounded().unwrap();
    assert!(text(&mut world).contains("has no size"));
}

#[test]
fn a_size_set_on_the_plane_is_the_size_to_return_to() {
    let mut world = test_world(DialogKind::Settings);
    let app = &mut world.resource_mut::<Simulation>().into_inner().app;
    app.set_unbounded().unwrap();
    let mut drafts = world.resource_mut::<Drafts>();
    drafts.width = "64".into();
    drafts.height = "48".into();
    let resize = world.spawn(Action::ResizeBoard).id();
    world.trigger(Activate { entity: resize });
    let feedback = &world.resource::<Drafts>().feedback;
    assert!(
        feedback.starts_with("Size to return to set to 64 × 48"),
        "{feedback}"
    );
}

#[test]
fn randomizing_the_plane_says_how_much_it_filled() {
    let mut world = test_world(DialogKind::Settings);
    let app = &mut world.resource_mut::<Simulation>().into_inner().app;
    app.set_unbounded().unwrap();
    let randomize = world.spawn(Action::RandomizeNow).id();
    world.trigger(Activate { entity: randomize });
    let feedback = &world.resource::<Drafts>().feedback;
    assert!(
        feedback.starts_with("32 × 32 cells randomized at 25%"),
        "{feedback}"
    );
}

#[test]
fn opening_an_unbounded_workspace_frames_its_cells_and_a_finite_one_does_not() {
    let open = |world: &mut World, workspace: Workspace| {
        let decoded = workspace.decode().unwrap();
        world.resource_mut::<Drafts>().pending_open = Some(("plane.golw".into(), decoded));
        let replace = world.spawn(Action::ConfirmOpenWorkspace).id();
        world.trigger(Activate { entity: replace });
    };
    let mut world = test_world(DialogKind::ConfirmOpen);
    let mut app = GolApp::new(32, 32);
    open(&mut world, app.workspace().unwrap());
    assert!(!world.resource::<Simulation>().fit_view);

    app.set_unbounded().unwrap();
    open(&mut world, app.workspace().unwrap());
    assert!(world.resource::<Simulation>().fit_view);
    assert_eq!(world.resource::<Simulation>().app.board.size(), None);
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
    finish_work(world);
}

/// Waits for the file task `choose` started and applies its result.
fn finish_work(world: &mut World) {
    let Some(FileWork::Working(path, task, _)) = world.resource_mut::<FileDialog>().0.take() else {
        return;
    };
    let mut outcome = Some((path, bevy::tasks::block_on(task)));
    world
        .run_system_once(move |mut state: DialogState| {
            let (path, outcome) = outcome.take().expect("the system runs once");
            finish_file_work(&mut state, path, outcome);
        })
        .unwrap();
}

/// A save request as the Save button captures it from the current app.
fn save_request(world: &World) -> FileRequest {
    let workspace = world.resource::<Simulation>().app.workspace().unwrap();
    FileRequest::Save(Box::new(workspace))
}

#[test]
fn an_image_saves_in_the_theme_colours_without_moving_the_workspace_folder() {
    let directory = crate::test_support::TestDirectory::new();
    let path = directory.0.join("board.png");
    let mut world = test_world(DialogKind::Export);
    world.resource_mut::<Simulation>().app.populate_board(0, 31);
    let image = world.resource::<Simulation>().app.board_image().unwrap();
    let colors = [[1, 2, 3], [4, 5, 6]];
    choose(
        &mut world,
        FileRequest::SaveImage(Box::new(image), colors),
        Some(&path),
    );

    let drafts = world.resource::<Drafts>();
    assert!(!drafts.error, "{}", drafts.feedback);
    assert!(
        drafts
            .feedback
            .starts_with("Saved 32 × 32 cells, 8 pixels across each"),
        "{}",
        drafts.feedback
    );
    assert_eq!(drafts.workspace_path, None);
    let file = std::fs::read(&path).unwrap();
    let mut reader = png::Decoder::new(std::io::Cursor::new(file))
        .read_info()
        .unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let frame = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((frame.width, frame.height), (256, 256));
    assert_eq!(
        reader.info().palette.as_deref(),
        Some(&[1, 2, 3, 4, 5, 6][..])
    );
    // Cell (0, 31) is the top left: the first pixel row starts with live ones.
    assert_eq!(pixels[0], 0xff);
    assert_eq!(pixels[1], 0);
}

#[test]
fn a_gif_records_the_generations_asked_for_and_refuses_a_count_out_of_range() {
    let mut world = test_world(DialogKind::Export);
    for generations in ["0", "1001", "ten"] {
        world.resource_mut::<Drafts>().generations = generations.into();
        let save = world.spawn(Action::SaveGif).id();
        world.trigger(Activate { entity: save });
        let drafts = world.resource::<Drafts>();
        assert!(drafts.error, "{generations}");
        assert_eq!(drafts.feedback, "A GIF records 1 to 1000 generations.");
        assert!(world.resource::<FileDialog>().0.is_none());
    }

    let directory = crate::test_support::TestDirectory::new();
    let path = directory.0.join("board.gif");
    world.resource_mut::<Simulation>().app.populate_board(4, 4);
    let gif = world
        .resource::<Simulation>()
        .app
        .board_gif(GifLength::Generations(5))
        .unwrap();
    choose(
        &mut world,
        FileRequest::SaveGif(Box::new(gif), [[0; 3], [255; 3]]),
        Some(&path),
    );
    let drafts = world.resource::<Drafts>();
    assert!(!drafts.error, "{}", drafts.feedback);
    assert!(
        drafts
            .feedback
            .starts_with("Saved 5 generations of 32 × 32 cells, 8 pixels across each"),
        "{}",
        drafts.feedback
    );
    assert_eq!(drafts.workspace_path, None);
    assert!(std::fs::read(&path).unwrap().starts_with(b"GIF89a"));
}

#[test]
fn one_period_reports_its_length_or_why_there_is_none() {
    let directory = crate::test_support::TestDirectory::new();
    let path = directory.0.join("board.gif");
    let mut world = test_world(DialogKind::Export);
    let save_period = |world: &mut World| {
        let gif = world
            .resource::<Simulation>()
            .app
            .board_gif(GifLength::Period)
            .unwrap();
        choose(
            world,
            FileRequest::SaveGif(Box::new(gif), [[0; 3], [255; 3]]),
            Some(&path),
        );
        let drafts = world.resource::<Drafts>();
        (drafts.error, drafts.feedback.clone())
    };
    // A blinker comes back every 2 generations.
    for x in 4..7 {
        world.resource_mut::<Simulation>().app.populate_board(x, 4);
    }
    let (error, feedback) = save_period(&mut world);
    assert!(!error, "{feedback}");
    assert!(
        feedback.starts_with("Saved one period, 2 generations, of 32 × 32 cells"),
        "{feedback}"
    );

    // An R-pentomino never comes back to its first generation, but settles into
    // debris instead, so nothing is saved.
    std::fs::remove_file(&path).unwrap();
    let app = &mut world.resource_mut::<Simulation>().into_inner().app;
    app.clear_board();
    for (x, y) in [(15, 16), (16, 16), (14, 15), (15, 15), (15, 14)] {
        app.populate_board(x, y);
    }
    let (error, feedback) = save_period(&mut world);
    assert!(error);
    assert!(
        feedback.contains("doesn't come back to this generation within 1000"),
        "{feedback}"
    );
    assert!(!path.exists());
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
    assert_eq!(
        world.resource::<Simulation>().app.board.size(),
        Some((8, 8))
    );
    let cancel = world.spawn(Action::CancelOpenWorkspace).id();
    world.trigger(Activate { entity: cancel });
    assert_eq!(
        world.resource::<NativeUi>().dialog,
        Some(DialogKind::Import)
    );
    assert!(world.resource::<Drafts>().pending_open.is_none());
    assert_eq!(
        world.resource::<Simulation>().app.board.size(),
        Some((8, 8))
    );
    assert!(world.resource::<Simulation>().app.is_running());

    // Closing the prompt (Escape or Close) also drops the pending file.
    choose(&mut world, FileRequest::Open, Some(&path));
    let close_button = world.spawn(Action::CloseDialog).id();
    world.trigger(Activate {
        entity: close_button,
    });
    assert!(world.resource::<Drafts>().pending_open.is_none());
    assert_eq!(
        world.resource::<Simulation>().app.board.size(),
        Some((8, 8))
    );

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
fn the_import_preview_summarises_the_pasted_rle_or_says_what_is_wrong() {
    let (pattern, summary, error) = import_summary("#N Glider\nx = 3, y = 3\nbo$2bo$3o!");
    assert_eq!(pattern.map(|p| p.cells.len()), Some(5));
    assert_eq!(summary, "Glider · 3 × 3 · 5 cells");
    assert!(!error);
    let (_, summary, _) = import_summary("x = 3, y = 1, rule = B36/S23\n3o!");
    assert_eq!(summary, "3 × 1 · 3 cells · rule B36/S23");
    let (pattern, _, error) = import_summary("x = 3, y = 3\nbo$2bo$4o!");
    assert!(pattern.is_none() && error);
    let (pattern, summary, error) = import_summary("  \n");
    assert!(pattern.is_none() && !error);
    assert_eq!(summary, "Paste RLE above to see a preview.");
}

#[test]
fn an_opened_rle_file_fills_the_import_fields_without_importing() {
    let directory = crate::test_support::TestDirectory::new();
    let named = directory.0.join("spaceship.rle");
    std::fs::write(&named, "#N Lightweight\nx = 5, y = 4\nbo2bo$o4b$o3bo$4o!").unwrap();
    let unnamed = directory.0.join("blinker.rle");
    std::fs::write(&unnamed, "x = 3, y = 1\n3o!").unwrap();
    let mut world = test_world(DialogKind::Import);
    let patterns = world.resource::<Simulation>().app.pattern_count();

    choose(&mut world, FileRequest::OpenRle, Some(&named));
    let drafts = world.resource::<Drafts>();
    assert!(!drafts.error);
    assert_eq!(drafts.name, "Lightweight");
    assert!(drafts.source.starts_with("#N Lightweight"));
    assert!(drafts.rebuild, "the fields are rebuilt to show the file");
    assert_eq!(drafts.workspace_path, None, "only workspace files set this");
    assert_eq!(world.resource::<Simulation>().app.pattern_count(), patterns);

    // Without a #N line, the file name names the pattern.
    choose(&mut world, FileRequest::OpenRle, Some(&unnamed));
    assert_eq!(world.resource::<Drafts>().name, "blinker");

    // A file too large to import is refused before it is read.
    let huge = directory.0.join("huge.rle");
    std::fs::write(&huge, vec![b' '; crate::patterns::MAX_RLE_BYTES + 1]).unwrap();
    choose(&mut world, FileRequest::OpenRle, Some(&huge));
    assert!(world.resource::<Drafts>().error);
    assert_eq!(world.resource::<Drafts>().name, "blinker");
}

#[test]
fn a_save_from_the_board_reports_as_a_notice() {
    let directory = crate::test_support::TestDirectory::new();
    let path = directory.0.join("workspace.golw");
    let mut world = test_world(DialogKind::Import);
    world.resource_mut::<NativeUi>().dialog = None;
    let request = save_request(&world);
    choose(&mut world, request, Some(&path));
    let notice = world.resource::<crate::feedback::EditFeedback>();
    assert!(notice.remaining > 0.0);
    assert!(notice.message.starts_with("Workspace saved to"));
    // With the dialog open, the result shows there instead.
    world
        .resource_mut::<crate::feedback::EditFeedback>()
        .remaining = 0.0;
    world.resource_mut::<NativeUi>().dialog = Some(DialogKind::Import);
    let request = save_request(&world);
    choose(&mut world, request, Some(&path));
    assert_eq!(
        world.resource::<crate::feedback::EditFeedback>().remaining,
        0.0
    );
    assert!(
        world
            .resource::<Drafts>()
            .feedback
            .starts_with("Workspace saved to")
    );
}

#[test]
fn a_second_save_is_refused_while_the_first_is_still_writing() {
    let directory = crate::test_support::TestDirectory::new();
    let path = directory.0.join("workspace.golw");
    let mut world = test_world(DialogKind::Import);
    world.resource_mut::<NativeUi>().dialog = None;
    let mut request = Some(save_request(&world));
    world
        .run_system_once(move |mut state: DialogState| {
            let request = request.take().expect("the system runs once");
            finish_file_request(&mut state, request, Some(path.clone()));
            assert_eq!(state.drafts.feedback, "Saving…");
            request_save(&mut state);
        })
        .unwrap();
    assert_eq!(
        world.resource::<crate::feedback::EditFeedback>().message,
        "Still working on the last file."
    );
    finish_work(&mut world);
    assert!(
        world
            .resource::<Drafts>()
            .feedback
            .starts_with("Workspace saved to")
    );
}

#[test]
fn cmd_or_ctrl_s_saves_and_o_opens_unless_a_dialog_without_feedback_is_open() {
    let press = |modifier: Option<KeyCode>, key: &str| {
        let mut keys = ButtonInput::<KeyCode>::default();
        if let Some(modifier) = modifier {
            keys.press(modifier);
        }
        let mut typed = crate::keybindings::TypedKeys::default();
        typed.press(key);
        (keys, typed)
    };
    for modifier in [KeyCode::SuperLeft, KeyCode::ControlRight] {
        let (keys, typed) = press(Some(modifier), "s");
        assert_eq!(file_shortcut(&keys, &typed, None), Some(FileShortcut::Save));
        let (keys, typed) = press(Some(modifier), "O");
        assert_eq!(
            file_shortcut(&keys, &typed, Some(DialogKind::Import)),
            Some(FileShortcut::Open)
        );
        assert!(
            file_shortcut(&keys, &typed, Some(DialogKind::Export)).is_some(),
            "Export shows file results too"
        );
        assert_eq!(file_shortcut(&keys, &typed, Some(DialogKind::Help)), None);
    }
    // Plain S pans the camera instead.
    let (keys, typed) = press(None, "s");
    assert_eq!(file_shortcut(&keys, &typed, None), None);
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
