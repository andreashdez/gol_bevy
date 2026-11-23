//! Tests of the app's actions, through its public methods and its fields.

use super::*;
use crate::{
    patterns::PatternParseError, sparse_board::PLANE_EXPORT_CELLS_MAX, universe::CellSnapshot,
};

fn stamp_centered(app: &mut App, pattern: PatternKind) {
    app.select_pattern(pattern);
    let (cols, rows) = app.finite_size();
    app.place_selected_pattern_at(cols as i64 / 2, rows as i64 / 2);
}

fn unbounded_app() -> App {
    let mut app = App::new(16, 16);
    assert_eq!(app.set_unbounded(), Ok(true));
    app
}

/// A board with a blinker, and its cells before any generation has run.
fn blinker_board() -> (App, CellSnapshot) {
    let mut app = App::new(16, 16);
    stamp_centered(&mut app, PatternKind::Blinker);
    let cells = app.board.snapshot_cells();
    (app, cells)
}

#[test]
fn the_unbounded_board_edits_and_runs_past_where_a_finite_one_stops() {
    let mut app = unbounded_app();
    assert_eq!(app.board.size(), None);
    for x in -1..=1 {
        app.populate_board(x - 5000, -5000);
    }
    assert_eq!(app.board.live_cells_count(), 3);

    app.increment_epoch();
    assert!(app.board.is_alive(-5000, -5001));
    assert!(app.board.is_alive(-5000, -4999));
    assert_eq!(app.population_change(), Some(0));

    app.depopulate_board(-5000, -5000);
    assert_eq!(app.board.live_cells_count(), 2);
}

#[test]
fn undo_and_reset_work_on_the_unbounded_board() {
    let mut app = unbounded_app();
    app.populate_board(-3, -3);
    let drawn = app.board.snapshot_cells();
    app.populate_board(-4, -3);
    assert!(app.undo());
    assert_eq!(app.board.snapshot_cells(), drawn);
    assert!(app.redo());
    assert_eq!(app.board.live_cells_count(), 2);

    let edited = app.board.snapshot_cells();
    app.increment_epoch();
    assert_eq!(app.board.live_cells_count(), 0);
    assert!(app.reset_to_start());
    assert_eq!(app.board.snapshot_cells(), edited);
}

#[test]
fn stamps_can_land_past_the_origin_only_on_the_unbounded_board() {
    let mut app = unbounded_app();
    app.select_pattern(PatternKind::Blinker);
    app.place_selected_pattern_at(-40, -40);
    assert_eq!(app.board.live_cells_count(), 3);

    // The same cursor is off a finite board, where stamps start at its edge.
    let mut app = App::new(16, 16);
    app.select_pattern(PatternKind::Blinker);
    app.place_selected_pattern_at(-40, -40);
    assert_eq!(app.board.live_cells_count(), 3);
    assert!(app.board.is_alive(0, 0));
}

#[test]
fn a_finite_board_ignores_edits_off_its_edges() {
    let mut app = App::new(16, 16);
    app.populate_board(-1, 0);
    app.populate_board(0, 16);
    app.depopulate_board(-1, 0);
    assert_eq!(app.board.live_cells_count(), 0);
    assert!(!app.can_undo());
}

#[test]
fn a_blinker_settles_on_the_unbounded_board_and_a_glider_does_not() {
    let mut app = unbounded_app();
    stamp_centered(&mut app, PatternKind::Blinker);
    for _ in 0..4 {
        app.increment_epoch();
    }
    assert!(app.settled().is_some());

    let mut app = unbounded_app();
    stamp_centered(&mut app, PatternKind::Glider);
    for _ in 0..200 {
        app.increment_epoch();
    }
    assert_eq!(app.settled(), None, "it never returns to the same cells");
}

#[test]
fn b0_rules_stay_off_the_unbounded_board_without_touching_history() {
    let b0 = Rule::parse("B0/S").unwrap();
    let mut app = unbounded_app();
    assert!(!app.set_rule(b0));
    assert_eq!(app.board.rule(), Rule::LIFE);
    assert!(app.undo());
    assert_eq!(app.board.size(), Some((16, 16)), "the refusal left no edit");

    // A finite board may hold one, and then can't move to the plane.
    let mut app = App::new(16, 16);
    assert!(app.set_rule(b0));
    assert_eq!(app.set_unbounded(), Err(UnboundedRuleError));
    assert_eq!(app.board.size(), Some((16, 16)));
    assert!(app.can_undo(), "a refused move forgets nothing");
}

#[test]
fn moving_onto_the_plane_keeps_the_cells_and_the_run_counter() {
    let mut app = App::new(16, 16);
    app.populate_board(8, 8);
    app.populate_board(0, 0);
    app.increment_epoch();
    app.populate_board(8, 8);
    let epoch = app.epoch;

    assert_eq!(app.set_unbounded(), Ok(true));
    assert_eq!(app.set_unbounded(), Ok(false), "it is already there");
    assert!(app.board.is_alive(0, 0));
    assert_eq!(app.board.live_cells_count(), 1);
    assert_eq!(app.epoch, epoch);
    assert!(!app.can_reset_to_start());
    assert!(app.take_redraw());
}

#[test]
fn switching_kinds_of_board_can_be_undone_and_redone() {
    let mut app = App::new(16, 16);
    app.set_boundary_mode(BoundaryMode::Fixed);
    app.populate_board(8, 8);
    let finite = app.board.snapshot_cells();

    app.set_unbounded().unwrap();
    app.populate_board(500, 500);
    let plane = app.board.snapshot_cells();

    // Leaving drops the far cell, and undo has to bring it back.
    app.set_boundary_mode(BoundaryMode::Wrap);
    assert_eq!(app.board.live_cells_count(), 1);
    assert!(app.undo());
    assert_eq!(app.board.snapshot_cells(), plane);
    assert!(app.board.is_alive(500, 500));

    assert!(app.undo(), "the cell drawn on the plane");
    assert!(app.undo(), "the move onto the plane");
    assert_eq!(app.board.snapshot_cells(), finite);
    assert_eq!(app.board.boundary_mode(), Some(BoundaryMode::Fixed));

    assert!(app.redo());
    assert_eq!(app.board.size(), None);
    assert!(app.redo());
    assert_eq!(app.board.snapshot_cells(), plane);
}

#[test]
fn undo_across_kinds_of_board_keeps_the_rule() {
    let highlife = Rule::parse("B36/S23").unwrap();
    let mut app = App::new(16, 16);
    app.set_rule(highlife);
    app.set_unbounded().unwrap();
    assert_eq!(app.board.rule(), highlife);

    assert!(app.undo());
    assert_eq!(app.board.size(), Some((16, 16)));
    assert_eq!(app.board.rule(), highlife);
    assert!(app.redo());
    assert_eq!(app.board.size(), None);
    assert_eq!(app.board.rule(), highlife);
}

#[test]
fn a_finite_rule_the_plane_refuses_comes_back_with_its_board() {
    let b0 = Rule::parse("B0/S").unwrap();
    let mut app = App::new(16, 16);
    // A rule the plane takes, swapped for B0 only after a snapshot of the
    // plane exists to undo onto and leave again.
    app.set_unbounded().unwrap();
    app.set_boundary_mode(BoundaryMode::Wrap);
    assert!(app.set_rule(b0));
    assert!(app.undo(), "the rule");
    assert!(app.undo(), "back onto the plane");
    assert_eq!(app.board.size(), None);
    assert!(app.redo(), "off the plane");
    assert!(app.redo(), "the rule again");
    assert_eq!(app.board.rule(), b0);
    assert_eq!(app.board.size(), Some((16, 16)));
}

#[test]
fn choosing_edges_returns_to_a_finite_board_of_the_old_size() {
    let mut app = App::new(16, 8);
    assert_eq!(app.set_unbounded(), Ok(true));
    app.populate_board(0, 0);
    app.populate_board(100, 100);

    app.set_boundary_mode(BoundaryMode::Fixed);
    assert_eq!(app.board.size(), Some((16, 8)));
    assert_eq!(app.board.boundary_mode(), Some(BoundaryMode::Fixed));
    assert_eq!(app.board.live_cells_count(), 1);
    assert!(app.board.is_alive(8, 4));
}

#[test]
fn the_edges_shortcut_steps_through_all_three_modes() {
    let mut app = App::new(16, 16);
    assert_eq!(app.board.boundary_mode(), Some(BoundaryMode::Wrap));
    app.toggle_boundary_mode();
    assert_eq!(app.board.boundary_mode(), Some(BoundaryMode::Fixed));
    app.toggle_boundary_mode();
    assert_eq!(app.board.boundary_mode(), None);
    assert_eq!(app.boundary_mode_label(), "Unbounded");
    app.toggle_boundary_mode();
    assert_eq!(app.board.boundary_mode(), Some(BoundaryMode::Wrap));
}

#[test]
fn the_edges_shortcut_skips_unbounded_under_a_b0_rule() {
    let mut app = App::new(16, 16);
    app.set_rule(Rule::parse("B0/S").unwrap());
    app.toggle_boundary_mode();
    app.toggle_boundary_mode();
    assert_eq!(app.board.boundary_mode(), Some(BoundaryMode::Wrap));
}

#[test]
fn resizing_the_unbounded_board_only_changes_the_size_to_return_to() {
    let mut app = unbounded_app();
    app.populate_board(0, 0);
    let drawn = app.board.snapshot_cells();
    app.populate_board(1, 0);
    app.resize_board(32, 64);
    assert_eq!(app.finite_size(), (32, 64));
    assert_eq!(app.board.size(), None);
    assert!(app.undo(), "nothing about the plane changed");
    assert_eq!(app.board.snapshot_cells(), drawn);
    assert!(app.undo());
    assert_eq!(app.board.live_cells_count(), 0);

    // History ends there: before it is a board of a size no longer in use.
    assert!(!app.can_undo());
    assert_eq!(app.board.size(), None);

    app.set_boundary_mode(BoundaryMode::Wrap);
    assert_eq!(app.board.size(), Some((32, 64)));
}

/// Cells in chunks of their own, far enough apart to have nothing to do.
fn scattered_app(chunks: i64) -> App {
    let mut app = unbounded_app();
    for chunk in 0..chunks {
        app.populate_board(chunk * 1000, 0);
    }
    app
}

/// A blinker standing on the east edge of a chunk, whose next generation reaches
/// into the chunk beside it.
fn blinker_on_a_chunk_edge(app: &mut App) {
    for y in 1..=3 {
        app.populate_board(63, y);
    }
}

#[test]
fn a_generation_past_the_budget_stops_the_run_until_something_frees_chunks() {
    let mut app = unbounded_app();
    blinker_on_a_chunk_edge(&mut app);
    app.chunk_budget = 1;
    app.toggle_running();
    app.increment_epoch();
    assert_eq!(app.epoch, 0, "no generation ran");
    assert!(app.board.is_alive(63, 1) && !app.board.is_alive(64, 2));
    assert!(!app.is_running());
    assert_eq!(app.take_auto_paused(), Some(AutoPause::BoardFull));
    assert_eq!(app.take_auto_paused(), None);
    assert_eq!(
        AutoPause::BoardFull.to_string(),
        "the board reached its memory limit"
    );

    // Playing or stepping again can't get past it.
    app.toggle_running();
    app.increment_epoch();
    assert!(!app.is_running());
    app.increment_epoch();
    assert_eq!(app.epoch, 0);
    assert_eq!(app.take_auto_paused(), Some(AutoPause::BoardFull));

    // What is left dies out within its chunk.
    app.depopulate_board(63, 1);
    app.increment_epoch();
    assert_eq!(app.epoch, 1);
    assert_eq!(app.take_auto_paused(), None);
}

#[test]
fn a_board_under_budget_or_with_edges_keeps_running() {
    let mut app = scattered_app(2);
    app.chunk_budget = 3;
    app.toggle_running();
    app.increment_epoch();
    assert!(app.is_running());
    assert_eq!(app.epoch, 1);

    let mut app = App::new(16, 16);
    app.chunk_budget = 0;
    app.toggle_running();
    app.increment_epoch();
    assert!(app.is_running());
    assert_eq!(app.epoch, 1);
}

#[test]
fn randomizing_the_plane_fills_the_view_and_leaves_the_rest() {
    let mut app = unbounded_app();
    app.set_randomize_settings(RandomizeSettings::new(100, None).unwrap());
    app.populate_board(500, 500);
    let view = CellRect {
        left: -5,
        bottom: 100,
        width: 70,
        height: 3,
    };
    app.set_view_cells(view);
    assert_eq!(app.randomize_area(), Some(view));
    app.randomize_board();
    assert_eq!(app.board.live_cells_count(), 70 * 3 + 1);
    assert!(app.board.is_alive(-5, 100) && app.board.is_alive(64, 102));
    assert!(!app.board.is_alive(-6, 100) && !app.board.is_alive(65, 102));
    assert!(app.board.is_alive(500, 500), "left outside the view alone");

    // What was there goes, as on a finite board.
    app.set_randomize_settings(RandomizeSettings::new(0, None).unwrap());
    app.randomize_board();
    assert_eq!(app.board.live_cells_count(), 1);

    assert!(app.undo());
    assert_eq!(app.board.live_cells_count(), 70 * 3 + 1);
    assert!(app.undo());
    assert_eq!(app.board.live_cells_count(), 1);
}

#[test]
fn a_seed_repeats_on_the_plane() {
    let randomized = |seed| {
        let mut app = unbounded_app();
        app.set_randomize_settings(RandomizeSettings::new(50, Some(seed)).unwrap());
        app.randomize_board();
        let mut cells: Vec<_> = match &app.board {
            Universe::Unbounded(plane) => plane.live_cells().collect(),
            Universe::Finite(_) => panic!("the board is unbounded"),
        };
        cells.sort_unstable();
        cells
    };
    assert_eq!(randomized(7), randomized(7));
    assert_ne!(randomized(7), randomized(8));
}

#[test]
fn randomizing_starts_from_where_the_board_would_be_and_is_capped_at_a_board() {
    let mut app = App::new(32, 16);
    app.set_unbounded().unwrap();
    assert_eq!(app.randomize_area(), Some(CellRect::around_origin(32, 16)));

    // A view zoomed far out is bigger than anything worth filling.
    app.set_view_cells(CellRect {
        left: 1 << 40,
        bottom: -(1 << 30),
        width: 1 << 30,
        height: BOARD_SIZE_MAX - 2,
    });
    let area = app.randomize_area().unwrap();
    assert_eq!(
        (area.width, area.height),
        (BOARD_SIZE_MAX, BOARD_SIZE_MAX - 2)
    );
    assert_eq!(area.bottom, -(1 << 30));
    let middle = (1i64 << 40) + (1 << 29);
    assert_eq!(area.left, middle - BOARD_SIZE_MAX as i64 / 2);

    assert_eq!(App::new(16, 16).randomize_area(), None);
}

#[test]
fn the_plane_exports_runs_of_any_extent_from_the_corner_of_its_cells() {
    let mut app = unbounded_app();
    assert_eq!(app.board_to_rle(), Err(BoardExportError::Empty));

    let far = 1i64 << 40;
    for (x, y) in [(far, -far), (far + 1, -far), (far + 1000, -far + 2)] {
        app.populate_board(x, y);
    }
    app.epoch = 9;
    let export = app.board_to_rle().unwrap();
    assert_eq!((export.cells, export.width, export.height), (3, 1001, 3));
    assert_eq!(
        export.rle,
        "#C Generation 9 of an unbounded board\n\
             x = 1001, y = 3, rule = B3/S23\n2o2$1000bo!\n"
    );
}

#[test]
fn a_plane_too_big_to_write_out_says_so() {
    let mut app = unbounded_app();
    let Universe::Unbounded(plane) = &mut app.board else {
        panic!("the board is unbounded");
    };
    let side = 1 << 13;
    let area = CellRect::around_origin(side + 1, side);
    plane.replace_rect(area, || true);
    assert!(plane.live_cells_count() > PLANE_EXPORT_CELLS_MAX);
    assert_eq!(app.board_to_rle(), Err(BoardExportError::TooLarge));
    assert!(
        app.workspace()
            .unwrap_err()
            .to_string()
            .contains("67,108,864")
    );

    let Universe::Unbounded(plane) = &mut app.board else {
        panic!("the board is unbounded");
    };
    plane.clear();
    plane.replace_rect(CellRect::around_origin(side, side), || true);
    assert_eq!(plane.live_cells_count(), PLANE_EXPORT_CELLS_MAX);
    assert_eq!(app.board_to_rle().unwrap().cells, PLANE_EXPORT_CELLS_MAX);
}

#[test]
fn a_stamp_with_nothing_new_on_the_board_is_not_an_edit() {
    let mut app = App::new(8, 8);
    app.select_pattern(PatternKind::Blinker);
    // Its third cell is off the board, and the two it can reach are alive.
    app.board.populate(6, 0);
    app.board.populate(7, 0);
    app.place_selected_pattern_at(7, 0);
    assert!(!app.can_undo());
}

#[test]
fn the_palette_follows_the_stamp_tool_and_can_be_hidden_while_stamping() {
    let mut app = App::new(16, 16);
    assert!(!app.palette_shown());
    app.toggle_palette();
    app.set_tool(EditTool::Stamp);
    assert!(app.palette_shown(), "there was nothing to hide before");
    assert!(app.has_selected_pattern());

    // Hidden, it stays hidden through stamping and a change of stamp.
    app.toggle_palette();
    assert!(!app.palette_shown());
    app.place_selected_pattern_at(8, 8);
    app.rotate_stamp_clockwise();
    app.select_pattern_index(1);
    assert!(!app.palette_shown());
    app.toggle_palette();
    assert!(app.palette_shown());

    // Choosing Stamp again brings it back, as does importing a pattern.
    app.toggle_palette();
    app.set_tool(EditTool::Stamp);
    assert!(app.palette_shown());
    app.toggle_palette();
    app.import_rle_pattern("Dot", "x = 1, y = 1\no!").unwrap();
    assert!(app.palette_shown());

    // Leaving the tool closes it, and coming back opens it, however it was left.
    app.toggle_palette();
    app.clear_selected_pattern();
    assert!(!app.palette_shown());
    app.set_tool(EditTool::Stamp);
    assert!(app.palette_shown());
    app.toggle_palette();
    app.set_tool(EditTool::Draw);
    app.select_pattern_index(0);
    assert!(app.palette_shown());
}

#[test]
fn reset_returns_to_the_board_before_the_first_generation() {
    let (mut app, drawn) = blinker_board();
    assert!(!app.can_reset_to_start());
    assert!(!app.reset_to_start());

    // Pausing and stepping stay part of the same run.
    app.toggle_running();
    app.increment_epoch();
    app.toggle_running();
    app.increment_epoch();
    app.increment_epoch();
    assert!(app.can_reset_to_start());
    app.toggle_running();

    assert!(app.reset_to_start());
    assert_eq!(app.board.snapshot_cells(), drawn);
    assert_eq!(app.epoch, 0);
    assert!(!app.is_running());
    assert!(!app.can_reset_to_start());
}

#[test]
fn population_history_starts_with_the_run_and_keeps_the_latest_generations() {
    let (mut app, _) = blinker_board();
    assert!(app.population_history().is_empty());
    app.increment_epoch();
    assert_eq!(app.population_history().len(), 2);

    for _ in 0..POPULATION_HISTORY * 2 {
        app.increment_epoch();
    }
    assert_eq!(app.population_history().len(), POPULATION_HISTORY);
    assert_eq!(
        app.population_history().back(),
        Some(&app.board.live_cells_count())
    );

    app.populate_board(1, 1);
    assert!(app.population_history().is_empty());
    app.increment_epoch();
    assert!(app.reset_to_start());
    assert!(app.population_history().is_empty());
}

#[test]
fn an_edit_moves_the_start_of_the_run_even_within_a_stroke() {
    let (mut app, _) = blinker_board();
    app.increment_epoch();
    app.begin_stroke();
    app.populate_board(1, 1);
    assert!(!app.can_reset_to_start());
    app.increment_epoch();
    assert!(app.can_reset_to_start());
    // Later cells of the same stroke are one undo edit, but still edits.
    app.populate_board(2, 1);
    assert!(!app.can_reset_to_start());
    app.end_stroke();
    let edited = app.board.snapshot_cells();

    app.increment_epoch();
    assert!(app.reset_to_start());
    assert_eq!(app.board.snapshot_cells(), edited);
    assert_eq!(app.epoch, 2);
}

#[test]
fn reset_is_an_undoable_edit() {
    let (mut app, drawn) = blinker_board();
    for _ in 0..3 {
        app.increment_epoch();
    }
    let ran = app.board.snapshot_cells();
    assert!(app.reset_to_start());

    assert!(app.undo());
    assert_eq!(app.board.snapshot_cells(), ran);
    assert_eq!(app.epoch, 3);
    assert!(app.redo());
    assert_eq!(app.board.snapshot_cells(), drawn);
    assert_eq!(app.epoch, 0);
}

#[test]
fn undo_drops_the_start_of_the_run() {
    let (mut app, _) = blinker_board();
    app.increment_epoch();
    assert!(app.undo());
    assert!(!app.can_reset_to_start());
}

#[test]
fn resizing_drops_the_start_of_the_run() {
    let (mut app, _) = blinker_board();
    app.increment_epoch();
    app.resize_board(32, 32);
    assert!(!app.can_reset_to_start());
}

#[test]
fn undo_and_redo_survive_a_resize_and_keep_the_new_size() {
    let mut app = App::new(16, 16);
    app.populate_board(1, 1);
    app.end_stroke();
    app.populate_board(12, 12);
    app.end_stroke();

    app.resize_board(32, 32);
    assert!(app.undo(), "history outlives a resize");
    assert_eq!(app.board.size(), Some((32, 32)));
    assert!(app.board.is_alive(1, 1) && !app.board.is_alive(12, 12));
    assert!(app.redo());
    assert!(app.board.is_alive(12, 12));

    // A snapshot from a larger board brings back only the cells that fit.
    app.populate_board(20, 20);
    app.end_stroke();
    app.resize_board(8, 8);
    assert!(app.undo());
    assert_eq!(app.board.size(), Some((8, 8)));
    assert!(app.board.is_alive(1, 1));
    assert_eq!(app.board.live_cells_count(), 1);
}

#[test]
fn a_still_life_settles_on_the_first_generation_and_an_oscillator_on_its_period() {
    let mut app = App::new(16, 16);
    for (col, row) in [(4, 4), (5, 4), (4, 5), (5, 5)] {
        app.populate_board(col, row);
    }
    assert_eq!(app.settled(), None);
    app.increment_epoch();
    let settled = app.settled().unwrap();
    assert_eq!((settled.since, settled.period), (0, 1));

    let (mut app, _) = blinker_board();
    app.increment_epoch();
    assert_eq!(app.settled(), None);
    app.increment_epoch();
    let settled = app.settled().unwrap();
    assert_eq!((settled.since, settled.period), (0, 2));
}

#[test]
fn a_glider_only_settles_once_it_crashes_into_a_fixed_corner() {
    let mut app = App::new(32, 32);
    app.set_boundary_mode(crate::board::BoundaryMode::Fixed);
    stamp_centered(&mut app, PatternKind::Glider);
    let mut generation = 0;
    while app.settled().is_none() {
        assert!(generation < 200, "the glider should end in the corner");
        app.increment_epoch();
        generation += 1;
    }
    let settled = app.settled().unwrap();
    assert_eq!(settled.period, 1);
    assert!(settled.since > 20);
}

#[test]
fn anything_that_changes_the_evolution_clears_settled() {
    let settled = || {
        let (mut app, _) = blinker_board();
        app.increment_epoch();
        app.increment_epoch();
        assert!(app.settled().is_some());
        app
    };
    let mut app = settled();
    app.populate_board(1, 1);
    assert_eq!(app.settled(), None);

    let mut app = settled();
    app.undo();
    assert_eq!(app.settled(), None);

    let mut app = settled();
    app.reset_to_start();
    assert_eq!(app.settled(), None);

    let mut app = settled();
    app.set_rule(Rule::parse("B36/S23").unwrap());
    assert_eq!(app.settled(), None);

    let mut app = settled();
    app.toggle_boundary_mode();
    assert_eq!(app.settled(), None);

    let mut app = settled();
    app.resize_board(32, 32);
    assert_eq!(app.settled(), None);

    // Choosing the mode already in use changes nothing.
    let mut app = settled();
    app.set_boundary_mode(app.board.boundary_mode().unwrap());
    assert!(app.settled().is_some());
}

#[test]
fn an_edit_between_generations_does_not_read_as_a_repeat() {
    let (mut app, _) = blinker_board();
    app.increment_epoch();
    app.populate_board(1, 1);
    app.increment_epoch();
    assert_eq!(app.settled(), None);
}

#[test]
fn resetting_the_counter_renumbers_a_settled_board() {
    let (mut app, _) = blinker_board();
    for _ in 0..5 {
        app.increment_epoch();
    }
    app.reset_epoch();
    assert_eq!(app.settled().map(|settled| settled.since), Some(0));
}

#[test]
fn playback_pauses_once_when_settled_if_asked() {
    let (mut app, _) = blinker_board();
    app.set_pause_when_settled(true);
    app.toggle_running();
    app.increment_epoch();
    assert!(app.is_running());
    assert_eq!(app.take_auto_paused(), None);
    app.increment_epoch();
    assert!(!app.is_running());
    assert_eq!(
        app.take_auto_paused(),
        app.settled().map(AutoPause::Settled)
    );
    assert_eq!(app.take_auto_paused(), None);

    // Playing on from there is a choice, so it is not interrupted again.
    app.toggle_running();
    app.increment_epoch();
    assert!(app.is_running());
    assert_eq!(app.take_auto_paused(), None);
}

#[test]
fn stepping_to_a_settled_board_does_not_report_a_pause() {
    let (mut app, _) = blinker_board();
    app.set_pause_when_settled(true);
    app.increment_epoch();
    app.increment_epoch();
    assert!(app.settled().is_some());
    assert_eq!(app.take_auto_paused(), None);
}

#[test]
fn playback_keeps_running_past_a_settled_board_by_default() {
    let (mut app, _) = blinker_board();
    app.toggle_running();
    app.increment_epoch();
    app.increment_epoch();
    assert!(app.settled().is_some());
    assert!(app.is_running());
}

#[test]
fn opening_a_workspace_keeps_the_pause_preference_but_not_the_run() {
    let (mut app, _) = blinker_board();
    app.set_pause_when_settled(true);
    let workspace = app.workspace().unwrap();
    app.increment_epoch();
    app.increment_epoch();
    app.restore_workspace(workspace).unwrap();
    assert!(app.pause_when_settled());
    assert!(!app.can_reset_to_start());
    assert_eq!(app.settled(), None);
}

#[test]
fn configured_randomization_repeats_and_remains_one_undoable_edit() {
    let mut app = App::new(32, 24);
    app.populate_board(5, 6);
    app.set_rule(Rule::parse("B36/S23").unwrap());
    app.epoch = 123;
    let original = app.board.snapshot_cells();
    let settings = RandomizeSettings::new(70, Some(u64::MAX)).unwrap();
    app.set_randomize_settings(settings);
    app.randomize_board();
    let randomized = app.board.snapshot_cells();
    assert!(app.undo());
    assert_eq!(app.board.snapshot_cells(), original);
    assert_eq!(app.epoch, 123);
    assert_eq!(app.rule_label(), "B36/S23");
    assert_eq!(app.randomize_settings(), settings);
    assert!(app.redo());
    assert_eq!(app.board.snapshot_cells(), randomized);
    app.randomize_board();
    assert_eq!(app.board.snapshot_cells(), randomized);
}

#[test]
fn imports_beyond_the_library_limits_leave_the_library_and_rule_unchanged() {
    use crate::patterns::{MAX_LIBRARY_PATTERNS, MAX_PATTERN_NAME_BYTES, MAX_PATTERN_SIDE};
    let mut app = App::new(16, 16);
    for index in 0..MAX_LIBRARY_PATTERNS {
        app.import_rle_pattern(format!("dot {index}"), "x = 1, y = 1\no!")
            .unwrap();
    }
    let (count, revision) = (app.pattern_count(), app.library_revision());
    assert_eq!(
        app.import_rle_pattern("one too many", "x = 1, y = 1, rule = B36/S23\no!")
            .err(),
        Some(PatternParseError::LibraryFull)
    );
    assert_eq!(
        (app.pattern_count(), app.library_revision()),
        (count, revision)
    );
    assert_eq!(app.rule_label(), "B3/S23");

    // A maximum-size pattern uses the whole cell budget.
    let mut app = App::new(16, 16);
    let side = MAX_PATTERN_SIDE;
    let full = format!(
        "x = {side}, y = {side}\n{}{side}o!",
        format!("{side}o$").repeat(side - 1)
    );
    app.import_rle_pattern("full", &full).unwrap();
    assert_eq!(
        app.import_rle_pattern("dot", "x = 1, y = 1\no!").err(),
        Some(PatternParseError::LibraryFull)
    );
    assert_eq!(
        app.import_rle_pattern("n".repeat(MAX_PATTERN_NAME_BYTES + 1), "x = 1, y = 1\no!")
            .err(),
        Some(PatternParseError::NameTooLong)
    );
    assert_eq!(app.pattern_count(), Pattern::builtin_count() + 1);
}

#[test]
fn setting_the_rule_is_an_undoable_edit() {
    let mut app = App::new(8, 8);
    app.populate_board(2, 2);
    let seeds = Rule::parse("B2/S").unwrap();
    assert!(app.set_rule(seeds));
    assert!(!app.set_rule(seeds));
    assert_eq!(app.board.rule(), seeds);
    assert!(app.undo());
    assert_eq!(app.board.rule(), Rule::LIFE);
    // Undoing the rule leaves the earlier drawing in place.
    assert!(app.board.is_alive(2, 2));
    assert!(app.redo());
    assert_eq!(app.board.rule(), seeds);
}

#[test]
fn board_export_crops_to_the_live_cells_and_reads_back() {
    let mut app = App::new(32, 24);
    assert_eq!(app.board_to_rle(), Err(BoardExportError::Empty));

    let cells = [(10, 5), (11, 6), (9, 7), (10, 7), (11, 7), (20, 15)];
    for (x, y) in cells {
        app.populate_board(x, y);
    }
    app.set_rule(Rule::parse("B36/S23").unwrap());
    let export = app.board_to_rle().unwrap();
    assert_eq!((export.cells, export.width, export.height), (6, 12, 11));

    let pattern = Pattern::from_rle("export", &export.rle).unwrap();
    assert_eq!(pattern.rule, Some(app.board.rule()));
    let mut read_back: Vec<_> = pattern
        .cells
        .iter()
        .map(|&(x, y)| (x as i64 + 9, y as i64 + 5))
        .collect();
    read_back.sort_unstable();
    let mut expected = cells.to_vec();
    expected.sort_unstable();
    assert_eq!(read_back, expected);
}

#[test]
fn undo_groups_drag_and_ignores_noop_edits() {
    let mut app = App::new(8, 8);
    app.clear_board();
    app.depopulate_board(0, 0);
    assert!(!app.can_undo());
    app.begin_stroke();
    app.populate_board(2, 2);
    app.populate_board(3, 2);
    app.populate_board(3, 2);
    app.end_stroke();
    app.populate_board(3, 2);
    assert!(app.undo());
    assert_eq!(app.board.live_cells_count(), 0);
    assert!(!app.can_undo());
}

#[test]
fn undo_restores_clear_and_generation_then_simulation_can_continue() {
    let mut app = App::new(8, 8);
    stamp_centered(&mut app, PatternKind::Blinker);
    app.increment_epoch();
    let expected = app.board.snapshot_cells();
    let mut next = app.board.clone();
    next.step_within(usize::MAX).unwrap();
    app.clear_board();
    app.toggle_running();
    app.increment_epoch();
    assert!(app.undo());
    assert_eq!(app.epoch, 1);
    assert!(!app.is_running());
    assert_eq!(app.board.snapshot_cells(), expected);
    assert_eq!(
        app.board.finite_mut().unwrap().take_dirty_cells(),
        board::DirtyCells::Full
    );
    app.increment_epoch();
    assert_eq!(app.board.snapshot_cells(), next.snapshot_cells());
}

#[test]
fn undo_restores_erased_cells_randomization_and_stamps() {
    let mut app = App::new(8, 8);
    stamp_centered(&mut app, PatternKind::Glider);
    let original = app.board.snapshot_cells();
    app.begin_stroke();
    for row in 0..8 {
        for col in 0..8 {
            app.depopulate_board(col, row);
        }
    }
    app.end_stroke();
    assert_eq!(app.board.live_cells_count(), 0);
    assert!(app.undo());
    assert_eq!(app.board.snapshot_cells(), original);
    app.randomize_board();
    assert!(app.undo());
    assert_eq!(app.board.snapshot_cells(), original);
    assert!(app.undo());
    assert_eq!(app.board.live_cells_count(), 0);
}

#[test]
fn redo_reapplies_undone_edits_until_a_new_edit_discards_the_chain() {
    let mut app = App::new(8, 8);
    assert!(!app.can_redo());
    assert!(!app.redo());

    app.populate_board(1, 1);
    app.populate_board(2, 2);
    let two_edits = app.board.snapshot_cells();

    assert!(app.undo());
    assert!(app.can_redo());
    assert_eq!(app.board.live_cells_count(), 1);

    assert!(app.redo());
    assert_eq!(app.board.snapshot_cells(), two_edits);
    assert!(!app.can_redo());

    // Undo twice, then make a different edit: the redo chain must be dropped.
    assert!(app.undo());
    assert!(app.undo());
    assert_eq!(app.board.live_cells_count(), 0);
    assert!(app.can_redo());
    app.populate_board(5, 5);
    assert!(!app.can_redo());
    assert!(!app.redo());
}

#[test]
fn redo_restores_generation_and_leaves_playback_paused() {
    let mut app = App::new(8, 8);
    stamp_centered(&mut app, PatternKind::Blinker);
    app.increment_epoch();
    app.increment_epoch();
    let epoch_before_clear = app.epoch;

    app.clear_board();
    assert!(app.undo());
    assert_eq!(app.epoch, epoch_before_clear);

    app.toggle_running();
    assert!(app.redo());
    assert_eq!(app.board.live_cells_count(), 0);
    assert_eq!(app.epoch, epoch_before_clear);
    assert!(!app.is_running());
}

#[test]
fn importing_rle_applies_a_declared_rule() {
    let mut app = App::new(12, 12);
    assert_eq!(app.rule_label(), "B3/S23");

    // HighLife: the same survival counts plus birth on six neighbours.
    let import = app
        .import_rle_pattern("HighLife", "x = 3, y = 1, rule = B36/S23\n3o!")
        .expect("RLE with a rule should import");
    assert_eq!(
        import.applied_rule.map(|rule| rule.to_string()),
        Some("B36/S23".to_string())
    );
    assert_eq!(app.rule_label(), "B36/S23");

    // A header without a rule leaves the board rule alone.
    let plain = app
        .import_rle_pattern("Plain", "x = 3, y = 1\n3o!")
        .expect("RLE without a rule should import");
    assert!(plain.applied_rule.is_none());
    assert_eq!(app.rule_label(), "B36/S23");
}

#[test]
fn the_unbounded_board_refuses_a_pattern_whose_rule_it_cannot_run() {
    let mut app = unbounded_app();
    let patterns = app.pattern_count();
    let source = "x = 3, y = 1, rule = B0/S8\n3o!";
    let b0 = Rule::parse("B0/S8").unwrap();
    assert_eq!(
        app.import_rle_pattern("Flash", source).unwrap_err(),
        PatternParseError::UnboundedRule(b0)
    );
    assert_eq!(app.pattern_count(), patterns, "nothing was added");
    assert_eq!(app.rule_label(), "B3/S23");
}

#[test]
fn undo_and_redo_carry_the_rule_an_import_changed() {
    let mut app = App::new(12, 12);
    app.populate_board(1, 1);

    app.import_rle_pattern("HighLife", "x = 3, y = 1, rule = B36/S23\n3o!")
        .expect("RLE with a rule should import");
    assert_eq!(app.rule_label(), "B36/S23");

    // The rule change is its own edit: undo reverts it and keeps the cells.
    assert!(app.undo());
    assert_eq!(app.rule_label(), "B3/S23");
    assert_eq!(app.board.live_cells_count(), 1);

    assert!(app.redo());
    assert_eq!(app.rule_label(), "B36/S23");

    // Undoing past the import also lands on the rule of that earlier state.
    assert!(app.undo());
    assert!(app.undo());
    assert_eq!(app.rule_label(), "B3/S23");
    assert_eq!(app.board.live_cells_count(), 0);

    // An import that leaves the rule alone adds no undo step.
    let mut plain = App::new(12, 12);
    plain
        .import_rle_pattern("Plain", "x = 3, y = 1, rule = B3/S23\n3o!")
        .expect("RLE should import");
    assert!(!plain.can_undo());
}

#[test]
fn switching_tools_preserves_stamp_selection_but_hides_ghost() {
    let mut app = App::new(8, 8);
    app.select_pattern(PatternKind::Blinker);
    app.set_tool(EditTool::Erase);
    assert!(!app.has_selected_pattern());
    assert!(app.selected_pattern_layout().is_none());
    app.set_tool(EditTool::Stamp);
    assert!(app.has_selected_pattern());
    assert_eq!(app.selected_pattern_name(), Some("Blinker"));
    app.set_tool(EditTool::Draw);
    assert!(app.select_pattern_index(1));
    assert_eq!(app.tool(), EditTool::Stamp);
}

#[test]
fn speed_accepts_intervals_between_keyboard_steps_and_clamps_bounds() {
    let mut app = App::new(8, 8);
    app.set_tick_rate_ms(100);
    assert_eq!(1000 / app.tick_rate_ms(), 10);
    app.set_tick_rate_ms(33);
    assert_eq!(app.tick_rate_ms(), 33);
    app.set_tick_rate_ms(0);
    assert_eq!(app.tick_rate_ms(), 25);
    app.set_tick_rate_ms(2000);
    assert_eq!(app.tick_rate_ms(), 1000);
}

#[test]
fn speeding_up_past_the_fastest_interval_reaches_max_speed_and_back() {
    let mut app = App::new(8, 8);
    app.set_tick_rate_ms(50);
    assert!(app.speed_up());
    assert!(!app.is_max_speed());
    assert_eq!(app.tick_rate_ms(), 25);

    assert!(app.speed_up());
    assert!(app.is_max_speed());
    assert!(!app.speed_up(), "max speed is the ceiling");

    assert!(app.slow_down());
    assert!(!app.is_max_speed());
    assert_eq!(app.tick_rate_ms(), 25);

    // Picking any interval, even the current one, leaves max speed.
    app.set_max_speed();
    assert!(app.set_tick_rate_ms(25));
    assert!(!app.is_max_speed());
    assert!(!app.set_tick_rate_ms(25));
}

#[test]
fn population_change_describes_the_latest_generation_until_an_edit() {
    let mut app = App::new(8, 8);
    assert_eq!(app.population_change(), None);
    // A blinker keeps its three cells; a lone cell dies.
    for col in 1..=3 {
        app.populate_board(col, 4);
    }
    app.increment_epoch();
    assert_eq!(app.population_change(), Some(0));
    // A rule change keeps the population, but the change no longer follows.
    assert!(app.set_rule(Rule::parse("B36/S23").unwrap()));
    assert_eq!(app.population_change(), None);
    app.increment_epoch();
    assert_eq!(app.population_change(), Some(0));
    app.populate_board(6, 6);
    assert_eq!(app.population_change(), None, "an edit is not a generation");
    app.increment_epoch();
    assert_eq!(app.population_change(), Some(-1));
    app.reset_epoch();
    assert_eq!(app.population_change(), None);
}

#[test]
fn test_app_increment_epoch() {
    let mut app = App::new(4, 4);
    app.increment_epoch();
    assert_eq!(app.epoch, 1);
}

#[test]
fn test_reset_epoch() {
    let mut app = App::new(4, 4);
    app.increment_epoch();
    app.reset_epoch();
    assert_eq!(app.epoch, 0);
}

#[test]
fn test_toggle_help() {
    let mut app = App::new(4, 4);
    assert!(!app.is_help_visible());
    app.toggle_help();
    assert!(app.is_help_visible());
}

#[test]
fn test_place_selected_pattern_at_cursor() {
    let mut app = App::new(8, 8);
    app.select_pattern(PatternKind::Blinker);
    app.place_selected_pattern_at(4, 4);
    assert!(app.board.is_alive(3, 4));
    assert!(app.board.is_alive(4, 4));
    assert!(app.board.is_alive(5, 4));
}

#[test]
fn test_toggle_boundary_mode() {
    let mut app = App::new(8, 8);
    assert_eq!(app.boundary_mode_label(), "Wrap");
    app.toggle_boundary_mode();
    assert_eq!(app.boundary_mode_label(), "Fixed");
    app.toggle_boundary_mode();
    assert_eq!(app.boundary_mode_label(), "Unbounded");
    app.toggle_boundary_mode();
    assert_eq!(app.boundary_mode_label(), "Wrap");
}

#[test]
fn test_rotate_stamp_changes_placement_orientation() {
    let mut app = App::new(8, 8);
    app.select_pattern(PatternKind::Blinker);
    app.rotate_stamp_clockwise();
    app.place_selected_pattern_at(4, 4);

    assert!(app.board.is_alive(4, 3));
    assert!(app.board.is_alive(4, 4));
    assert!(app.board.is_alive(4, 5));
}

#[test]
fn test_flip_stamp_changes_status_label() {
    let mut app = App::new(8, 8);
    app.select_pattern(PatternKind::Glider);
    app.toggle_stamp_flip();
    assert_eq!(app.selected_pattern_status_label(), "Glider (FlipH)");
}

fn selecting(app: &mut App, from: (i64, i64), to: (i64, i64)) {
    app.set_tool(EditTool::Select);
    app.begin_selection(from.0, from.1);
    app.extend_selection(to.0, to.1);
}

#[test]
fn a_selection_spans_its_corners_whichever_way_it_was_dragged() {
    let mut app = App::new(16, 16);
    selecting(&mut app, (9, 2), (4, 7));
    let expected = CellRect {
        left: 4,
        bottom: 2,
        width: 6,
        height: 6,
    };
    assert_eq!(app.selection(), Some(expected));

    // Only the Select tool shows one, and leaving it drops it.
    app.set_tool(EditTool::Draw);
    assert_eq!(app.selection(), None);
    app.set_tool(EditTool::Select);
    assert_eq!(app.selection(), None);
}

#[test]
fn a_selection_stays_within_a_board_that_shrank() {
    let mut app = App::new(16, 16);
    selecting(&mut app, (4, 4), (14, 14));
    app.resize_board(10, 12);
    assert_eq!(
        app.selection(),
        Some(CellRect {
            left: 4,
            bottom: 4,
            width: 6,
            height: 8,
        })
    );
    app.resize_board(8, 8);
    selecting(&mut app, (9, 9), (12, 12));
    assert_eq!(app.selection(), None);
}

#[test]
fn erasing_a_selection_kills_only_its_cells_as_one_edit() {
    for mut app in [App::new(16, 16), unbounded_app()] {
        for cell in [(2, 2), (3, 3), (4, 4), (8, 8)] {
            app.populate_board(cell.0, cell.1);
        }
        let before = app.board.snapshot_cells();
        selecting(&mut app, (3, 3), (8, 4));
        assert_eq!(app.erase_selection(), 2);
        assert!(app.board.is_alive(2, 2) && app.board.is_alive(8, 8));
        assert!(!app.board.is_alive(3, 3) && !app.board.is_alive(4, 4));
        // Nothing left to erase is no edit.
        assert_eq!(app.erase_selection(), 0);
        assert!(app.undo());
        assert_eq!(app.board.snapshot_cells(), before);
    }
}

#[test]
fn a_selection_copies_as_rle_that_keeps_its_margins() {
    for mut app in [App::new(16, 16), unbounded_app()] {
        assert_eq!(
            app.selection_to_rle(),
            Err(SelectionExportError::NothingSelected)
        );
        selecting(&mut app, (5, 5), (9, 7));
        assert_eq!(app.selection_to_rle(), Err(SelectionExportError::Empty));
        // An L in the selection's upper right.
        for cell in [(7, 6), (7, 7), (8, 7)] {
            app.populate_board(cell.0, cell.1);
        }
        let export = app.selection_to_rle().unwrap();
        assert_eq!((export.cells, export.width, export.height), (3, 5, 3));
        let pattern = Pattern::from_rle("copy", &export.rle).unwrap();
        assert_eq!((pattern.width, pattern.height), (5, 3));
        let mut cells = pattern.cells.clone();
        cells.sort_unstable();
        assert_eq!(cells, vec![(2, 1), (2, 2), (3, 2)]);
        assert_eq!(pattern.rule, Some(app.board.rule()));
    }
}

#[test]
fn selecting_the_live_cells_boxes_them_with_the_select_tool() {
    for mut app in [App::new(16, 16), unbounded_app()] {
        assert!(!app.select_live_cells());
        assert_eq!(app.tool(), EditTool::Draw);
        app.populate_board(3, 9);
        app.populate_board(6, 2);
        assert!(app.select_live_cells());
        assert_eq!(app.tool(), EditTool::Select);
        assert_eq!(
            app.selection(),
            Some(CellRect {
                left: 3,
                bottom: 2,
                width: 4,
                height: 8,
            })
        );
    }
}

#[test]
fn moving_between_the_board_and_the_plane_drops_the_selection() {
    let mut app = App::new(16, 16);
    selecting(&mut app, (1, 1), (4, 4));
    assert_eq!(app.set_unbounded(), Ok(true));
    assert_eq!(app.selection(), None);

    // Undo brings the board back, but not a plane selection onto it.
    selecting(&mut app, (1, 1), (4, 4));
    assert!(app.undo());
    assert!(app.board.finite().is_some());
    assert_eq!(app.selection(), None);
}

#[test]
fn a_copied_selection_pastes_as_a_stamp_that_places_the_same_cells() {
    for mut app in [App::new(32, 32), unbounded_app()] {
        // An L with an empty column on its left, kept by the copy.
        for cell in [(4, 4), (4, 5), (5, 5)] {
            app.populate_board(cell.0, cell.1);
        }
        selecting(&mut app, (3, 4), (5, 6));
        let rle = app.selection_to_rle().unwrap().rle;
        let library = app.pattern_count();
        app.rotate_stamp_clockwise();

        let pasted = app.paste_rle(&rle).unwrap();
        assert_eq!((pasted.width, pasted.height), (3, 3));
        assert_eq!(app.tool(), EditTool::Stamp);
        assert_eq!(app.selection(), None);
        assert!(app.has_selected_pattern());
        assert_eq!(app.selected_pattern_name(), Some("Pasted"));
        assert_eq!(app.stamp_orientation(), (0, false), "as it was copied");
        assert_eq!(app.selected_pattern_index(), None);
        assert_eq!(app.pattern_count(), library, "the library is unchanged");

        // Centred on (21, 21), the 3 × 3 stamp starts at (20, 20).
        app.place_selected_pattern_at(21, 21);
        for cell in [(21, 20), (21, 21), (22, 21)] {
            assert!(app.board.is_alive(cell.0, cell.1), "{cell:?}");
        }
        assert_eq!(app.board.live_cells_count(), 6);
    }
}

#[test]
fn a_paste_stays_the_stamp_until_another_is_chosen_or_put_away() {
    let mut app = App::new(16, 16);
    app.paste_rle("x = 2, y = 1\n2o!").unwrap();
    let revision = app.paste_revision();

    // Drawing and coming back keeps the paste.
    app.set_tool(EditTool::Draw);
    assert!(!app.has_selected_pattern());
    app.set_tool(EditTool::Stamp);
    assert_eq!(app.selected_pattern_name(), Some("Pasted"));

    // A library pattern replaces it, and so does a later paste.
    app.select_pattern(PatternKind::Glider);
    assert_eq!(app.selected_pattern_name(), Some("Glider"));
    app.paste_rle("#N Domino\nx = 1, y = 2\no$o!").unwrap();
    assert_eq!(app.selected_pattern_name(), Some("Domino"));
    assert_ne!(app.paste_revision(), revision);
    // Choosing the library pattern again works while a paste is the stamp.
    assert!(app.select_pattern_index(PatternKind::Glider.builtin_index().unwrap()));

    app.paste_rle("x = 1, y = 1\no!").unwrap();
    app.clear_selected_pattern();
    assert!(!app.has_selected_pattern());
    assert_eq!(app.selected_pattern_name(), None);
}

#[test]
fn a_paste_that_is_not_a_pattern_leaves_the_stamp_and_rule_alone() {
    let mut app = App::new(16, 16);
    app.select_pattern(PatternKind::Glider);
    for text in ["", "hello"] {
        let error = app.paste_rle(text).unwrap_err();
        assert_eq!(error.to_string(), "The clipboard holds no RLE pattern");
    }
    assert_eq!(
        app.paste_rle("x = 3, y = 3\n!").err(),
        Some(PasteError::Empty)
    );
    assert_eq!(app.selected_pattern_name(), Some("Glider"));

    let rule = app.board.rule();
    app.paste_rle("x = 1, y = 1, rule = B36/S23\no!").unwrap();
    assert_eq!(app.board.rule(), rule);
    assert!(!app.can_undo(), "pasting is no edit");
}
