//! The app as `main.rs` builds it, run headless for a few frames: each plugin's
//! systems are tested on their own elsewhere, but only here do they run in the
//! order `GolPlugin` gives them, with the UI, input, camera, and drawing
//! handing work to one another from frame to frame.
use bevy::{
    input::{ButtonState, mouse::MouseButtonInput},
    prelude::*,
    render::{RenderPlugin, settings::WgpuSettings},
    window::{CursorMoved, PrimaryWindow, WindowEvent, WindowResolution},
    winit::WinitPlugin,
};
use gol_bevy::{
    gol_plugin::GolPlugin,
    native_ui_plugin::canvas_rect,
    shared::{RedrawRequested, Simulation},
};

/// The default plugins with a window but no event loop or GPU, then the app's.
fn headless_app() -> App {
    // Settings are read and saved as the app runs, so they go to a folder of
    // their own rather than the user's. The first test to get here sets it
    // before any app runs, and the others wait, so nothing reads the
    // environment while it changes.
    static SETTINGS: std::sync::Once = std::sync::Once::new();
    SETTINGS.call_once(|| {
        let settings =
            std::env::temp_dir().join(format!("gol-bevy-schedule-{}", std::process::id()));
        unsafe { std::env::set_var("GOL_BEVY_CONFIG_DIR", &settings) };
    });

    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .build()
            .disable::<WinitPlugin>()
            .set(RenderPlugin {
                render_creation: WgpuSettings {
                    backends: None,
                    ..default()
                }
                .into(),
                ..default()
            })
            .set(WindowPlugin {
                primary_window: Some(Window {
                    resolution: WindowResolution::new(1200, 800),
                    ..default()
                }),
                ..default()
            }),
    )
    .add_plugins(GolPlugin);
    // WinitPlugin registers the requests of assistive technology, which the
    // accessibility systems read.
    app.add_message::<bevy::a11y::ActionRequest>();
    app.finish();
    app.cleanup();
    app
}

fn window(app: &mut App) -> Entity {
    app.world_mut()
        .query_filtered::<Entity, With<PrimaryWindow>>()
        .single(app.world())
        .unwrap()
}

/// Moves the pointer to `position`, as the window's events would.
fn move_pointer(app: &mut App, position: Vec2) {
    let window = window(app);
    app.world_mut()
        .get_mut::<Window>(window)
        .unwrap()
        .set_cursor_position(Some(position));
    let moved = CursorMoved {
        window,
        position,
        delta: None,
    };
    app.world_mut().write_message(moved.clone());
    app.world_mut()
        .write_message(WindowEvent::CursorMoved(moved));
}

/// Presses or releases the left button, as the window's events would.
fn press(app: &mut App, state: ButtonState) {
    let input = MouseButtonInput {
        button: MouseButton::Left,
        state,
        window: window(app),
    };
    app.world_mut().write_message(input);
    app.world_mut()
        .write_message(WindowEvent::MouseButtonInput(input));
}

/// Puts the pointer at `position` and presses or releases the left button,
/// then runs a frame.
fn click_frame(app: &mut App, position: Vec2, state: ButtonState) {
    move_pointer(app, position);
    press(app, state);
    app.update();
}

fn live_cells(app: &App) -> usize {
    app.world()
        .resource::<Simulation>()
        .app
        .board
        .live_cells_count()
}

#[test]
fn a_click_edits_the_canvas_but_not_through_the_palette_and_is_redrawn_next_frame() {
    let mut app = headless_app();
    // Startup builds the board, and the UI lays itself out and says where it is.
    for _ in 0..3 {
        app.update();
    }
    assert_eq!(live_cells(&app), 0);
    let canvas = canvas_rect(Vec2::new(1200.0, 800.0));

    // The edit happens after the frame is drawn, so the next frame draws it.
    click_frame(&mut app, canvas.center(), ButtonState::Pressed);
    assert_eq!(live_cells(&app), 1, "a click on the canvas draws a cell");
    click_frame(&mut app, canvas.center(), ButtonState::Released);
    assert!(app.world().resource::<RedrawRequested>().0);
    app.update();
    assert!(
        !app.world().resource::<RedrawRequested>().0,
        "nothing left to draw"
    );

    // With a stamp picked, the palette floats over the board's left side.
    app.world_mut()
        .resource_mut::<Simulation>()
        .app
        .select_pattern_index(0);
    for _ in 0..2 {
        app.update();
    }
    let palette = Vec2::new(canvas.min.x + 100.0, canvas.max.y - 40.0);
    click_frame(&mut app, palette, ButtonState::Pressed);
    click_frame(&mut app, palette, ButtonState::Released);
    assert_eq!(live_cells(&app), 1, "a click on the palette stamps nothing");

    let right = Vec2::new(canvas.max.x - 100.0, canvas.center().y);
    click_frame(&mut app, right, ButtonState::Pressed);
    assert!(live_cells(&app) > 1, "a click beside it stamps");
}

#[test]
fn a_tap_pressed_and_released_within_one_frame_still_clicks_a_button() {
    let mut app = headless_app();
    for _ in 0..3 {
        app.update();
    }
    let running = |app: &App| app.world().resource::<Simulation>().app.is_running();
    let play = Vec2::new(44.0, 32.0);
    click_frame(&mut app, play, ButtonState::Pressed);
    click_frame(&mut app, play, ButtonState::Released);
    assert!(running(&app), "a click over two frames plays");

    // A quick tap, or a click in a slow frame: the pointer comes from the
    // board, presses Play, and lifts before the app sees another frame.
    move_pointer(&mut app, canvas_rect(Vec2::new(1200.0, 800.0)).center());
    app.update();
    move_pointer(&mut app, play);
    press(&mut app, ButtonState::Pressed);
    press(&mut app, ButtonState::Released);
    app.update();
    assert!(!running(&app), "the tap pauses");
}
