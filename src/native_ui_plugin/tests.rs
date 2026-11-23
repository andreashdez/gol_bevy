use super::*;

#[test]
fn opening_a_workspace_rebuilds_a_library_with_the_same_pattern_count() {
    let mut world = World::new();
    let mut app = GolApp::new(16, 16);
    app.import_rle_pattern("Old import", "x = 1, y = 1\no!")
        .unwrap();
    world.insert_resource(Simulation::new(app));
    world.insert_resource(Theme::Dark);
    world.init_resource::<Assets<Image>>();
    world.spawn(Palette);
    let sync = world.register_system(sync_palette);
    world.run_system(sync).unwrap();
    world.clear_trackers();
    let mut other = GolApp::new(16, 16);
    other
        .import_rle_pattern("New import", "x = 2, y = 1\n2o!")
        .unwrap();
    world
        .resource_mut::<Simulation>()
        .app
        .restore_workspace(other.workspace().unwrap())
        .unwrap();
    world.run_system(sync).unwrap();
    let names: Vec<_> = world
        .query::<&Name>()
        .iter(&world)
        .map(|name| name.as_str())
        .collect();
    assert!(names.contains(&"New import"));
    assert!(!names.contains(&"Old import"));
}

#[test]
fn clicked_buttons_release_focus_but_keyboard_and_modal_focus_stay() {
    use bevy::{ecs::system::RunSystemOnce, input_focus::FocusCause};
    let mut world = World::new();
    world.init_resource::<InputFocus>();
    world.init_resource::<InputFocusVisible>();
    world.init_resource::<UiInputRegions>();
    let button = world.spawn(Action::Randomize).id();
    let field = world.spawn_empty().id();
    let focused = |world: &mut World, entity, visible, modal| {
        world
            .resource_mut::<InputFocus>()
            .set(entity, FocusCause::Navigated);
        world.resource_mut::<InputFocusVisible>().0 = visible;
        world.resource_mut::<UiInputRegions>().modal = modal;
        world.run_system_once(release_pointer_focus).unwrap();
        world.resource::<InputFocus>().get()
    };
    assert_eq!(focused(&mut world, button, false, false), None);
    assert_eq!(focused(&mut world, button, true, false), Some(button));
    assert_eq!(focused(&mut world, button, false, true), Some(button));
    assert_eq!(focused(&mut world, field, false, false), Some(field));
}

#[test]
fn tab_follows_the_layout_from_the_toolbar_to_the_board() {
    use bevy::{
        ecs::system::RunSystemOnce,
        input_focus::{
            FocusCause,
            tab_navigation::{NavAction, TabNavigation},
        },
    };
    let mut world = World::new();
    world.init_resource::<Assets<Font>>();
    world.init_resource::<Assets<Image>>();
    world.init_resource::<InputFocus>();
    world.spawn((Window::default(), PrimaryWindow));
    world.run_system_once(setup).unwrap();
    // Visibility propagation does not run here; Bevy's tab walker skips hidden nodes.
    for mut visibility in world
        .query::<&mut InheritedVisibility>()
        .iter_mut(&mut world)
    {
        *visibility = InheritedVisibility::VISIBLE;
    }
    let area = |world: &World, entity: Entity| {
        let mut current = Some(entity);
        while let Some(e) = current {
            let found = if world.get::<toolbar::Group>(e).is_some() {
                Some("toolbar")
            } else if world.get::<Sidebar>(e).is_some() {
                Some("palette")
            } else if world.get::<Footer>(e).is_some() {
                Some("status bar")
            } else if world.get::<BoardArea>(e).is_some() {
                Some("board")
            } else {
                None
            };
            if found.is_some() {
                return found;
            }
            current = world.get::<ChildOf>(e).map(ChildOf::parent);
        }
        None
    };
    let play = world
        .query::<(Entity, &Action)>()
        .iter(&world)
        .find_map(|(e, a)| (*a == Action::Play).then_some(e))
        .unwrap();
    world
        .resource_mut::<InputFocus>()
        .set(play, FocusCause::Navigated);
    let mut visited = vec!["toolbar"];
    while *visited.last().unwrap() != "board" {
        let next = world
            .run_system_once(|nav: TabNavigation, focus: Res<InputFocus>| {
                nav.navigate(&focus, NavAction::Next).unwrap()
            })
            .unwrap();
        world
            .resource_mut::<InputFocus>()
            .set(next, FocusCause::Navigated);
        let here = area(&world, next).expect("every Tab stop is in a known area");
        if *visited.last().unwrap() != here {
            assert!(!visited.contains(&here), "Tab returned to the {here}");
            visited.push(here);
        }
    }
    assert_eq!(visited, ["toolbar", "palette", "status bar", "board"]);
}

#[test]
fn board_outline_follows_keyboard_focus_on_the_board() {
    use bevy::{
        ecs::system::RunSystemOnce,
        input_focus::{
            FocusCause,
            tab_navigation::{NavAction, TabNavigation},
        },
    };
    let mut world = World::new();
    world.init_resource::<Assets<Font>>();
    world.init_resource::<Assets<Image>>();
    world.init_resource::<InputFocus>();
    world.init_resource::<InputFocusVisible>();
    world.init_resource::<UiInputRegions>();
    let window = world.spawn((Window::default(), PrimaryWindow)).id();
    world.run_system_once(setup).unwrap();
    let board = world
        .query_filtered::<Entity, With<BoardFocus>>()
        .single(&world)
        .unwrap();
    let button = world
        .query::<(Entity, &Action)>()
        .iter(&world)
        .find_map(|(e, a)| (*a == Action::Play).then_some(e))
        .unwrap();
    let outlined = |world: &mut World, focus: Option<Entity>, visible, modal| {
        match focus {
            Some(e) => world
                .resource_mut::<InputFocus>()
                .set(e, FocusCause::Navigated),
            None => world.resource_mut::<InputFocus>().clear(),
        }
        world.resource_mut::<InputFocusVisible>().0 = visible;
        world.resource_mut::<UiInputRegions>().modal = modal;
        world.run_system_once(sync_board_focus).unwrap();
        let mut rings = world.query_filtered::<&Visibility, With<BoardFocusRing>>();
        *rings.single(world).unwrap() == Visibility::Inherited
    };
    // Visibility propagation does not run here; Bevy's tab walker skips hidden nodes.
    for mut visibility in world
        .query::<&mut InheritedVisibility>()
        .iter_mut(&mut world)
    {
        *visibility = InheritedVisibility::VISIBLE;
    }
    let last = world
        .run_system_once(|nav: TabNavigation, focus: Res<InputFocus>| {
            nav.navigate(&focus, NavAction::Previous).unwrap()
        })
        .unwrap();
    assert_eq!(
        last, board,
        "Shift+Tab from the board's start reaches the board"
    );
    assert!(outlined(&mut world, Some(board), true, false));
    assert!(outlined(&mut world, Some(window), true, false));
    assert!(outlined(&mut world, None, true, false));
    assert!(!outlined(&mut world, Some(board), false, false));
    assert!(!outlined(&mut world, Some(board), true, true));
    assert!(!outlined(&mut world, Some(button), true, false));
    let rect = canvas_rect(world.get::<Window>(window).unwrap().size());
    let mut areas = world.query_filtered::<&Node, With<BoardArea>>();
    let node = areas.single(&world).unwrap();
    assert_eq!((node.left, node.width), (px(rect.min.x), px(rect.width())));
}

#[test]
fn native_widget_bundles_spawn_together() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    world.init_resource::<Assets<Font>>();
    world.init_resource::<Assets<Image>>();
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
            .query_filtered::<Entity, With<speed::SpeedEditor>>()
            .iter(&world)
            .count(),
        1
    );
}

#[test]
fn unchanged_frames_leave_button_render_state_untouched() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    world.init_resource::<Assets<Font>>();
    world.init_resource::<Assets<Image>>();
    world.insert_resource(Simulation::new(GolApp::new(8, 8)));
    world.init_resource::<NativeUi>();
    world.insert_resource(GridOverlay { enabled: false });
    world.init_resource::<CellAge>();
    world.init_resource::<crate::shared::BoardSettings>();
    world.init_resource::<InputFocus>();
    world.init_resource::<InputFocusVisible>();
    world.init_resource::<Theme>();
    world.run_system_once(setup).unwrap();
    world.run_system_once(sync).unwrap();
    world.run_system_once(theme::apply).unwrap();
    let ticks = |world: &mut World| {
        let mut ticks = Vec::new();
        ticks.extend(
            world
                .query_filtered::<(Ref<ThemedBackground>, Ref<ThemedBorder>), With<Action>>()
                .iter(world)
                .flat_map(|(bg, border)| [bg.last_changed(), border.last_changed()]),
        );
        ticks.extend(
            world
                .query_filtered::<(Ref<BackgroundColor>, Ref<BorderColor>), With<Action>>()
                .iter(world)
                .flat_map(|(bg, border)| [bg.last_changed(), border.last_changed()]),
        );
        ticks.extend(
            world
                .query::<(Ref<ThemedText>, Ref<TextColor>)>()
                .iter(world)
                .flat_map(|(token, color)| [token.last_changed(), color.last_changed()]),
        );
        ticks
    };
    let before = ticks(&mut world);
    assert!(!before.is_empty());
    world.run_system_once(sync).unwrap();
    world.run_system_once(theme::apply).unwrap();
    assert_eq!(ticks(&mut world), before);
}

#[test]
fn only_keyboard_focus_outlines_a_button() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    world.init_resource::<Assets<Font>>();
    world.init_resource::<Assets<Image>>();
    world.insert_resource(Simulation::new(GolApp::new(8, 8)));
    world.init_resource::<NativeUi>();
    world.insert_resource(GridOverlay { enabled: false });
    world.init_resource::<CellAge>();
    world.init_resource::<crate::shared::BoardSettings>();
    world.init_resource::<InputFocus>();
    world.init_resource::<InputFocusVisible>();
    world.init_resource::<Theme>();
    world.run_system_once(setup).unwrap();
    let step = world
        .query::<(Entity, &Action)>()
        .iter(&world)
        .find_map(|(entity, action)| (*action == Action::Step).then_some(entity))
        .unwrap();
    world
        .resource_mut::<InputFocus>()
        .set(step, bevy::input_focus::FocusCause::Navigated);
    let border = |world: &mut World, visible: bool| {
        world.resource_mut::<InputFocusVisible>().0 = visible;
        world.run_system_once(sync).unwrap();
        world.get::<ThemedBorder>(step).unwrap().0
    };
    // A click focuses the button for a frame without showing focus.
    assert_eq!(border(&mut world, false), Token::Line);
    assert_eq!(border(&mut world, true), Token::Focus);
}

#[test]
fn the_palette_shows_only_while_stamping_and_leaves_the_toolbar_alone() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    world.init_resource::<Assets<Font>>();
    world.init_resource::<Assets<Image>>();
    world.insert_resource(Simulation::new(GolApp::new(8, 8)));
    world.init_resource::<NativeUi>();
    world.insert_resource(GridOverlay { enabled: false });
    world.init_resource::<CellAge>();
    world.init_resource::<crate::shared::BoardSettings>();
    world.init_resource::<InputFocus>();
    world.init_resource::<InputFocusVisible>();
    world.init_resource::<Theme>();
    world.run_system_once(setup).unwrap();
    let shown = |world: &mut World| {
        world.run_system_once(sync).unwrap();
        let (node, visibility) = world
            .query_filtered::<(&Node, &Visibility), With<Sidebar>>()
            .single(world)
            .unwrap();
        // Tab skips hidden nodes, but not ones merely out of the layout.
        assert_eq!(
            node.display != Display::None,
            *visibility != Visibility::Hidden
        );
        node.display != Display::None
    };
    let change = |world: &mut World, edit: fn(&mut GolApp)| {
        edit(&mut world.resource_mut::<Simulation>().app);
    };
    assert!(!shown(&mut world));
    change(&mut world, |app| app.set_tool(EditTool::Stamp));
    assert!(shown(&mut world));
    change(&mut world, GolApp::toggle_palette);
    assert!(!shown(&mut world));
    change(&mut world, |app| app.set_tool(EditTool::Erase));
    assert!(!shown(&mut world));

    // Every tool is a toolbar button, so the palette can close without hiding one.
    let groups = world
        .query_filtered::<&ChildOf, With<Action>>()
        .iter(&world)
        .map(ChildOf::parent)
        .collect::<Vec<_>>();
    let tools = world
        .query::<(Entity, &toolbar::Group)>()
        .iter(&world)
        .find_map(|(e, group)| matches!(group, toolbar::Group::Tools).then_some(e))
        .unwrap();
    assert_eq!(groups.iter().filter(|&&e| e == tools).count(), 4);
}

#[test]
fn a_closing_palette_gives_up_keyboard_focus() {
    use bevy::{ecs::system::RunSystemOnce, input_focus::FocusCause};
    let mut world = World::new();
    let mut app = GolApp::new(8, 8);
    app.set_tool(EditTool::Stamp);
    world.insert_resource(Simulation::new(app));
    world.init_resource::<InputFocus>();
    let palette = world.spawn(Sidebar).id();
    let row = world.spawn(ChildOf(palette)).id();
    let other = world.spawn_empty().id();
    let focused = |world: &mut World, entity| {
        world
            .resource_mut::<InputFocus>()
            .set(entity, FocusCause::Navigated);
        world.run_system_once(release_palette_focus).unwrap();
        world.resource::<InputFocus>().get()
    };
    assert_eq!(focused(&mut world, row), Some(row));
    world
        .resource_mut::<Simulation>()
        .app
        .set_tool(EditTool::Draw);
    assert_eq!(focused(&mut world, other), Some(other));
    assert_eq!(focused(&mut world, row), None);
}

#[test]
fn narrow_windows_move_workspace_controls_into_a_menu() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    world.init_resource::<Assets<Font>>();
    world.init_resource::<Assets<Image>>();
    world.insert_resource(Simulation::new(GolApp::new(8, 8)));
    world.init_resource::<NativeUi>();
    world.init_resource::<toolbar::WorkspaceMenu>();
    world.init_resource::<InputFocus>();
    world.init_resource::<InputFocusVisible>();
    world.init_resource::<ButtonInput<KeyCode>>();
    world.init_resource::<ButtonInput<MouseButton>>();
    world.init_resource::<UiInputRegions>();
    world.init_resource::<minimap::MinimapArea>();
    world.add_observer(toolbar::menu_action);
    let window = world
        .spawn((
            Window {
                resolution: bevy::window::WindowResolution::new(
                    toolbar::COLLAPSE_BELOW_WIDTH as u32 - 1,
                    600,
                ),
                ..default()
            },
            PrimaryWindow,
        ))
        .id();
    world.run_system_once(setup).unwrap();
    let mut schedule = Schedule::default();
    schedule.add_systems((toolbar::sync_layout, toolbar::sync_menu).chain());
    let find = |world: &mut World, action: Action| {
        world
            .query::<(Entity, &Action)>()
            .iter(world)
            .find_map(|(e, a)| (*a == action).then_some(e))
            .unwrap()
    };
    let more = find(&mut world, Action::More);
    let grid = find(&mut world, Action::Grid);
    let panel_shown = |world: &mut World| {
        let (node, visibility) = world
            .query_filtered::<(&Node, &Visibility), With<toolbar::MenuPanel>>()
            .single(world)
            .unwrap();
        assert_eq!(
            node.display != Display::None,
            *visibility != Visibility::Hidden
        );
        node.display != Display::None
    };
    schedule.run(&mut world);
    assert!(world.resource::<toolbar::WorkspaceMenu>().collapsed);
    assert_ne!(world.get::<Node>(more).unwrap().display, Display::None);
    assert!(!panel_shown(&mut world));

    // From the keyboard, More opens the menu on its first item.
    world.resource_mut::<InputFocusVisible>().0 = true;
    world.trigger(Activate { entity: more });
    schedule.run(&mut world);
    assert!(panel_shown(&mut world));
    world.run_system_once(publish_input_regions).unwrap();
    assert!(world.resource::<UiInputRegions>().modal);
    assert_eq!(world.resource::<InputFocus>().get(), Some(grid));

    // Escape closes it and returns focus to More.
    world
        .resource_mut::<ButtonInput<KeyCode>>()
        .press(KeyCode::Escape);
    schedule.run(&mut world);
    world.resource_mut::<ButtonInput<KeyCode>>().clear();
    assert!(!panel_shown(&mut world));
    assert_eq!(world.resource::<InputFocus>().get(), Some(more));

    // Choosing an item closes the menu.
    world.trigger(Activate { entity: more });
    schedule.run(&mut world);
    assert!(panel_shown(&mut world));
    world.trigger(Activate { entity: grid });
    schedule.run(&mut world);
    assert!(!panel_shown(&mut world));

    // Wide windows show the controls in the toolbar and hide More.
    world.trigger(Activate { entity: more });
    world
        .get_mut::<Window>(window)
        .unwrap()
        .resolution
        .set(toolbar::COLLAPSE_BELOW_WIDTH, 600.0);
    schedule.run(&mut world);
    let menu = world.resource::<toolbar::WorkspaceMenu>();
    assert!(!menu.collapsed && !menu.open);
    assert!(panel_shown(&mut world));
    assert_eq!(world.get::<Node>(more).unwrap().display, Display::None);
}

#[test]
fn row_thumbnails_scale_wide_patterns_to_the_wide_texture() {
    let colors = Theme::Dark.board();
    let live = crate::render_plugin::color_to_texel(colors.live);
    // A 36 × 9 row of live cells, as wide as the Gosper glider gun.
    let cells: Vec<_> = (0..36).map(|x| (x, 4)).collect();
    let image = thumbnail_sized(&cells, 36, 9, colors, ROW_THUMBNAIL_TEXELS, 4.0, 14.0);
    let data = image.data.unwrap();
    let width = ROW_THUMBNAIL_TEXELS.x as usize;
    let lit = |x: usize, y: usize| data[(y * width + x) * 4..(y * width + x) * 4 + 4] == live;
    // The cells' texel row, rather than a gap between rows of cells.
    let row = (0..ROW_THUMBNAIL_TEXELS.y as usize)
        .max_by_key(|&y| (0..width).filter(|&x| lit(x, y)).count())
        .unwrap();
    let first = (0..width).find(|&x| lit(x, row)).unwrap();
    let last = (0..width).rfind(|&x| lit(x, row)).unwrap();
    let span = last - first + 1;
    // Width bounds the scale, so the pattern spans most of the texture.
    assert!(span > width * 3 / 4, "{span}");
    assert!(
        !lit(0, row) && !lit(width - 1, row),
        "the margin stays clear"
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
fn speed_buttons_step_the_displayed_rate_with_limits_and_infinity_toggles() {
    let mut app = GolApp::new(8, 8);
    app.set_tick_rate_ms(100);
    apply_action(Action::SpeedAdjust(1), &mut app);
    assert_eq!(app.tick_rate_ms(), controls::rate_to_ms(11.0));
    apply_action(Action::SpeedAdjust(-1), &mut app);
    assert_eq!(app.tick_rate_ms(), 100);
    app.set_tick_rate_ms(80);
    apply_action(Action::SpeedAdjust(1), &mut app);
    assert_eq!(app.tick_rate_ms(), controls::rate_to_ms(13.5));
    app.set_tick_rate_ms(1000);
    apply_action(Action::SpeedAdjust(-1), &mut app);
    assert_eq!(app.tick_rate_ms(), 1000);
    app.set_tick_rate_ms(25);
    apply_action(Action::SpeedAdjust(1), &mut app);
    assert!(!app.is_max_speed());
    apply_action(Action::MaxSpeed, &mut app);
    assert!(app.is_max_speed());
    apply_action(Action::SpeedAdjust(-1), &mut app);
    assert!(!app.is_max_speed());
    assert_eq!(app.tick_rate_ms(), 25);
    assert_eq!(app.board.live_cells_count(), 0);
}

#[test]
fn the_status_bar_edges_button_steps_through_every_mode() {
    use crate::board::BoundaryMode;
    let mut app = GolApp::new(8, 8);
    assert_eq!(app.board.boundary_mode(), Some(BoundaryMode::Wrap));
    apply_action(Action::ToggleEdges, &mut app);
    assert_eq!(app.board.boundary_mode(), Some(BoundaryMode::Fixed));
    apply_action(Action::ToggleEdges, &mut app);
    assert_eq!(app.board.boundary_mode(), None);
    apply_action(Action::ToggleEdges, &mut app);
    assert_eq!(app.board.boundary_mode(), Some(BoundaryMode::Wrap));
}

#[test]
fn the_unbounded_edges_action_moves_onto_the_plane_unless_the_rule_refuses() {
    let mut app = GolApp::new(8, 8);
    apply_action(Action::Unbounded, &mut app);
    assert_eq!(app.board.boundary_mode(), None);

    let mut app = GolApp::new(8, 8);
    app.set_rule(crate::board::Rule::parse("B0/S").unwrap());
    assert!(!app.board.allows_unbounded());
    apply_action(Action::Unbounded, &mut app);
    assert!(app.board.finite().is_some());
}

#[test]
fn the_palette_floats_over_the_canvas_without_changing_it() {
    for size in [Vec2::new(800.0, 420.0), Vec2::new(1600.0, 900.0)] {
        let canvas = canvas_rect(size);
        // One status bar row at every width, down to the minimum window.
        assert_eq!(canvas.max.y, size.y - STATUS_BAR_HEIGHT - 8.0);
        for panel in panel_rects(size, false) {
            assert!(canvas.intersect(panel).is_empty());
        }
        let palette = palette_rect(size);
        assert!(!canvas.intersect(palette).is_empty());
        assert!(palette.height() > 0.0);
    }
}

#[test]
fn the_canvas_takes_clicks_except_under_an_open_palette() {
    let size = Vec2::new(800.0, 600.0);
    let captured = |palette, pos| panel_rects(size, palette).iter().any(|r| r.contains(pos));
    let toolbar = Vec2::new(300.0, 40.0);
    let status_bar = Vec2::new(500.0, 590.0);
    let under_palette = Vec2::new(40.0, 210.0);
    let beside_palette = Vec2::new(400.0, 300.0);
    for palette in [true, false] {
        assert!(captured(palette, toolbar));
        assert!(captured(palette, status_bar));
        assert!(!captured(palette, beside_palette));
        assert_eq!(captured(palette, under_palette), palette);
    }
    // The size of a very short window cannot turn the rectangle inside out.
    assert!(palette_rect(Vec2::new(800.0, 10.0)).height() == 0.0);
}

#[test]
fn an_edit_notice_moves_beside_an_open_palette() {
    use bevy::ecs::system::RunSystemOnce;
    let mut world = World::new();
    world.insert_resource(Simulation::new(GolApp::new(8, 8)));
    world.init_resource::<EditFeedback>();
    world.init_resource::<Time>();
    world.init_resource::<NativeUi>();
    world.insert_resource(GridOverlay { enabled: false });
    world.init_resource::<CellAge>();
    world.init_resource::<crate::shared::BoardSettings>();
    world.init_resource::<InputFocus>();
    world.init_resource::<InputFocusVisible>();
    world.init_resource::<Theme>();
    let window = world.spawn((Window::default(), PrimaryWindow)).id();
    let notice = world
        .spawn((controls::FeedbackOverlay, Node::default()))
        .id();
    let left = |world: &mut World| {
        world.run_system_once(controls::sync_feedback).unwrap();
        let Val::Px(left) = world.get::<Node>(notice).unwrap().left else {
            panic!("the notice is placed in pixels");
        };
        left
    };
    let size = world.get::<Window>(window).unwrap().size();
    assert_eq!(left(&mut world), canvas_rect(size).min.x + 12.0);
    world
        .resource_mut::<Simulation>()
        .app
        .set_tool(EditTool::Stamp);
    assert_eq!(left(&mut world), palette_rect(size).max.x + 12.0);
}
