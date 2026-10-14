//! Persisted workspace settings, independent of the UI implementation.
use crate::{
    board::{BoundaryMode, Rule},
    shared::{BoardSettings, CellAge, GridOverlay, Simulation, StartupSet, Theme, UiLayout},
};
use bevy::prelude::*;
use bevy::{
    app::{AppExit, OnAppExitSystems},
    ecs::system::SystemParam,
};
use std::{path::PathBuf, time::Duration};

const SAVE_DEBOUNCE: Duration = Duration::from_millis(500);

pub struct SettingsPlugin;
impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiLayout>()
            .init_resource::<BoardSettings>()
            .init_resource::<Theme>()
            .init_resource::<CellAge>()
            .add_systems(
                Startup,
                (
                    setup_settings_persistence.in_set(StartupSet::LoadSettings),
                    apply_simulation_settings.after(StartupSet::BuildWorld),
                ),
            )
            .add_systems(
                Last,
                persist_settings
                    .in_set(OnAppExitSystems)
                    .after(bevy::window::ExitSystems),
            );
    }
}

#[derive(Resource)]
struct SettingsPersistence {
    path: PathBuf,
    last_saved: Option<PersistedSettings>,
    /// The latest observed state and when it is eligible for saving (or retry).
    pending: Option<(PersistedSettings, Duration)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PersistedSettings {
    palette: bool,
    board: (usize, usize),
    fill_canvas: bool,
    simulation: SimulationSettings,
    grid: bool,
    cell_age: bool,
    theme: Theme,
}

/// What the simulation is started with. The simulation only exists once the
/// world is built, so these wait in a resource until then.
#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
struct SimulationSettings {
    rule: Option<Rule>,
    boundary: Option<BoundaryMode>,
    tick_rate_ms: Option<u64>,
    max_speed: Option<bool>,
}

impl SimulationSettings {
    fn of(sim: &Simulation) -> Self {
        Self {
            rule: Some(sim.app.board.rule()),
            boundary: Some(sim.app.board.boundary_mode()),
            tick_rate_ms: Some(sim.app.tick_rate_ms()),
            max_speed: Some(sim.app.is_max_speed()),
        }
    }

    fn apply_to(self, sim: &mut Simulation) {
        // Startup state rather than an edit, so none of this enters undo history.
        if let Some(rule) = self.rule {
            sim.app.board.set_rule(rule);
        }
        if let Some(mode) = self.boundary {
            sim.app.board.set_boundary_mode(mode);
        }
        if let Some(tick_rate_ms) = self.tick_rate_ms {
            sim.app.set_tick_rate_ms(tick_rate_ms);
        }
        if self.max_speed == Some(true) {
            sim.app.set_max_speed();
        }
    }
}

#[derive(Default)]
struct ParsedSettings {
    palette: Option<bool>,
    board: Option<(usize, usize)>,
    fill_canvas: Option<bool>,
    simulation: SimulationSettings,
    grid: Option<bool>,
    cell_age: Option<bool>,
    theme: Option<Theme>,
}

fn setup_settings_persistence(
    mut commands: Commands,
    mut ui_layout: ResMut<UiLayout>,
    mut board_settings: ResMut<BoardSettings>,
    mut grid: ResMut<GridOverlay>,
    mut cell_age: ResMut<CellAge>,
    mut theme: ResMut<Theme>,
) {
    let path = settings_path();
    let older = [
        Some(config_path(OLD_SETTINGS_FILE)),
        legacy_config_path(OLD_SETTINGS_FILE),
    ];
    let source = settings_source(&path, older.into_iter().flatten());
    if source != path {
        info!(
            "Reading settings from {}; they will be saved to {}",
            source.display(),
            path.display()
        );
    }

    let simulation = load_settings(
        &source,
        &mut ui_layout,
        &mut board_settings,
        &mut grid,
        &mut cell_age,
        &mut theme,
    );

    commands.insert_resource(simulation);
    commands.insert_resource(SettingsPersistence {
        path,
        last_saved: None,
        pending: None,
    });
}

fn apply_simulation_settings(settings: Res<SimulationSettings>, sim: Option<ResMut<Simulation>>) {
    // Without a window the world, and so the simulation, is never built.
    if let Some(mut sim) = sim {
        settings.apply_to(&mut sim);
    }
}

/// Applies what can be applied before the world is built, and returns the
/// settings that have to wait for the simulation.
fn load_settings(
    path: &std::path::Path,
    ui_layout: &mut UiLayout,
    board_settings: &mut BoardSettings,
    grid: &mut GridOverlay,
    cell_age: &mut CellAge,
    theme: &mut Theme,
) -> SimulationSettings {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let settings = parse_settings_from_toml(&contents);
            if let Some(visible) = settings.palette {
                ui_layout.control_deck_visible = visible;
            }
            // Left unset, startup falls back to a board that fills the window.
            if let Some((cols, rows)) = settings.board {
                board_settings.set(cols, rows);
            }
            if let Some(fill) = settings.fill_canvas {
                board_settings.fill_canvas = fill;
            }
            if let Some(enabled) = settings.grid {
                grid.enabled = enabled;
            }
            if let Some(enabled) = settings.cell_age {
                cell_age.enabled = enabled;
            }
            if let Some(saved) = settings.theme {
                *theme = saved;
            }
            settings.simulation
        }
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    "Failed to read persisted settings from {}: {}",
                    path.display(),
                    err
                );
            }
            SimulationSettings::default()
        }
    }
}

/// Resources used to capture a complete settings snapshot.
#[derive(SystemParam)]
struct SettingsState<'w> {
    ui_layout: Res<'w, UiLayout>,
    board_settings: Res<'w, BoardSettings>,
    grid: Res<'w, GridOverlay>,
    cell_age: Res<'w, CellAge>,
    theme: Res<'w, Theme>,
    sim: Option<Res<'w, Simulation>>,
}

impl SettingsState<'_> {
    fn snapshot(&self) -> Option<PersistedSettings> {
        // Saving before the world is built would overwrite the file with defaults.
        let sim = self.sim.as_ref()?;
        if !self.board_settings.is_set() {
            return None;
        }
        Some(PersistedSettings {
            palette: self.ui_layout.control_deck_visible,
            board: (self.board_settings.cols, self.board_settings.rows),
            fill_canvas: self.board_settings.fill_canvas,
            simulation: SimulationSettings::of(sim),
            grid: self.grid.enabled,
            cell_age: self.cell_age.enabled,
            theme: *self.theme,
        })
    }
}

fn persist_settings(
    state: SettingsState,
    mut persistence: ResMut<SettingsPersistence>,
    time: Res<Time<Real>>,
    mut exits: MessageReader<AppExit>,
) {
    let exiting = exits.read().count() != 0;
    let Some(current) = state.snapshot() else {
        return;
    };
    if persistence.last_saved == Some(current) {
        persistence.pending = None;
        return;
    }
    let now = time.elapsed();
    if !persistence
        .pending
        .is_some_and(|(pending, _)| pending == current)
    {
        persistence.pending = Some((current, now.saturating_add(SAVE_DEBOUNCE)));
    }
    if !exiting
        && persistence
            .pending
            .is_some_and(|(_, deadline)| now < deadline)
    {
        return;
    }
    // A failed save must not retry and log every frame. Keep the state pending.
    persistence.pending = Some((current, now.saturating_add(SAVE_DEBOUNCE)));

    if let Some(parent) = persistence.path.parent()
        && let Err(err) = std::fs::create_dir_all(parent)
    {
        warn!(
            "Failed to create config directory {}: {}",
            parent.display(),
            err
        );
        return;
    }

    let simulation = current.simulation;
    let mut contents = format!(
        "palette = {}\nboard_cols = {}\nboard_rows = {}\nfill_canvas = {}\ngrid = {}\ncell_age = {}\ntheme = \"{}\"\n",
        current.palette,
        current.board.0,
        current.board.1,
        current.fill_canvas,
        current.grid,
        current.cell_age,
        theme_storage_str(current.theme),
    );
    if let Some(rule) = simulation.rule {
        contents.push_str(&format!("rule = \"{rule}\"\n"));
    }
    if let Some(mode) = simulation.boundary {
        contents.push_str(&format!(
            "boundary_mode = \"{}\"\n",
            boundary_storage_str(mode)
        ));
    }
    if let Some(tick_rate_ms) = simulation.tick_rate_ms {
        contents.push_str(&format!("tick_rate_ms = {tick_rate_ms}\n"));
    }
    if let Some(max_speed) = simulation.max_speed {
        contents.push_str(&format!("max_speed = {max_speed}\n"));
    }
    if let Err(err) = crate::atomic_file::write(&persistence.path, contents.as_bytes()) {
        warn!(
            "Failed to persist settings to {}: {}",
            persistence.path.display(),
            err
        );
        return;
    }

    persistence.last_saved = Some(current);
    persistence.pending = None;
}

/// Minimal key/value reader for the handful of scalars this app persists.
fn parse_settings_from_toml(contents: &str) -> ParsedSettings {
    let mut parsed = ParsedSettings::default();
    let mut cols = None;
    let mut rows = None;

    for line in contents.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let value = unquote(raw_value.trim());

        match key.trim() {
            "palette" => parsed.palette = value.parse::<bool>().ok(),
            // Older files saved a layout preset; only "minimal" hid the palette.
            "layout_preset" => {
                let visible = match value {
                    "full" | "compact" => Some(true),
                    "minimal" => Some(false),
                    _ => None,
                };
                parsed.palette = parsed.palette.or(visible);
            }
            "board_cols" => cols = value.parse::<usize>().ok(),
            "board_rows" => rows = value.parse::<usize>().ok(),
            "fill_canvas" => parsed.fill_canvas = value.parse::<bool>().ok(),
            "grid" => parsed.grid = value.parse::<bool>().ok(),
            "cell_age" => parsed.cell_age = value.parse::<bool>().ok(),
            "theme" => parsed.theme = theme_from_storage_str(value),
            "rule" => parsed.simulation.rule = Rule::parse(value).ok(),
            "boundary_mode" => parsed.simulation.boundary = boundary_from_storage_str(value),
            "tick_rate_ms" => parsed.simulation.tick_rate_ms = value.parse::<u64>().ok(),
            "max_speed" => parsed.simulation.max_speed = value.parse::<bool>().ok(),
            _ => {}
        }
    }

    if let (Some(cols), Some(rows)) = (cols, rows) {
        parsed.board = Some((cols, rows));
    }
    parsed
}

fn boundary_storage_str(mode: BoundaryMode) -> &'static str {
    match mode {
        BoundaryMode::Wrap => "wrap",
        BoundaryMode::Fixed => "fixed",
    }
}

fn boundary_from_storage_str(value: &str) -> Option<BoundaryMode> {
    match value {
        "wrap" => Some(BoundaryMode::Wrap),
        "fixed" => Some(BoundaryMode::Fixed),
        _ => None,
    }
}

fn theme_storage_str(theme: Theme) -> &'static str {
    match theme {
        Theme::Dark => "dark",
        Theme::Light => "light",
        Theme::HighContrast => "high-contrast",
    }
}

fn theme_from_storage_str(value: &str) -> Option<Theme> {
    Theme::ALL
        .into_iter()
        .find(|theme| theme_storage_str(*theme) == value)
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|inner| inner.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|inner| inner.strip_suffix('\''))
        })
        .unwrap_or(value)
        .trim()
}

const SETTINGS_FILE: &str = "settings.toml";
/// What the settings file was called while it only held the layout preset.
const OLD_SETTINGS_FILE: &str = "ui_layout.toml";

fn settings_path() -> PathBuf {
    config_path(SETTINGS_FILE)
}

/// The platform's settings folder: `$XDG_CONFIG_HOME` or `~/.config` on Linux,
/// `~/Library/Application Support` on macOS, and `%APPDATA%` on Windows.
/// `GOL_BEVY_CONFIG_DIR` overrides it everywhere.
fn config_path(file_name: &str) -> PathBuf {
    if let Some(config_dir) = std::env::var_os("GOL_BEVY_CONFIG_DIR") {
        return PathBuf::from(config_dir).join(file_name);
    }
    if let Some(config_dir) = dirs::config_dir() {
        return config_dir.join("gol-bevy").join(file_name);
    }
    // No home directory at all, so keep the settings beside the working directory.
    PathBuf::from(".gol-bevy").join("config").join(file_name)
}

/// Where earlier versions saved settings on every platform: the XDG location,
/// even on macOS, and on Windows usually a folder relative to the working
/// directory, since `HOME` is rarely set there.
fn legacy_config_path(file_name: &str) -> Option<PathBuf> {
    if std::env::var_os("GOL_BEVY_CONFIG_DIR").is_some() {
        // An explicit folder was used then too, so there is nothing to carry over.
        return None;
    }
    let legacy = if let Some(config_dir) = std::env::var_os("XDG_CONFIG_HOME") {
        PathBuf::from(config_dir).join("gol-bevy")
    } else if let Some(home_dir) = std::env::var_os("HOME") {
        PathBuf::from(home_dir).join(".config").join("gol-bevy")
    } else {
        PathBuf::from(".gol-bevy").join("config")
    };
    Some(legacy.join(file_name))
}

/// Reads from the first older file that exists until the first save creates
/// `path`, so renaming or moving the settings file does not lose what was saved
/// before. Older files are left in place.
fn settings_source(path: &std::path::Path, older: impl IntoIterator<Item = PathBuf>) -> PathBuf {
    if path.exists() {
        return path.to_path_buf();
    }
    older
        .into_iter()
        .find(|older| older.exists())
        .unwrap_or_else(|| path.to_path_buf())
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    fn persistence_world(path: PathBuf) -> World {
        let mut world = World::new();
        populate_world(&mut world, path);
        world
    }

    fn populate_world(world: &mut World, path: PathBuf) {
        world.init_resource::<UiLayout>();
        world.insert_resource(BoardSettings {
            cols: 16,
            rows: 16,
            fill_canvas: false,
        });
        world.insert_resource(GridOverlay { enabled: false });
        world.init_resource::<CellAge>();
        world.init_resource::<Theme>();
        world.insert_resource(Simulation::new(crate::app::App::new(16, 16)));
        world.insert_resource(SettingsPersistence {
            path,
            last_saved: None,
            pending: None,
        });
        world.init_resource::<Time<Real>>();
        world.init_resource::<Messages<AppExit>>();
    }

    fn advance(world: &mut World, milliseconds: u64) {
        world
            .resource_mut::<Time<Real>>()
            .advance_by(Duration::from_millis(milliseconds));
        world.run_system_once(persist_settings).unwrap();
    }

    #[test]
    fn rapid_changes_save_only_after_settling_and_reverting_cancels_a_pending_save() {
        let directory = crate::test_support::TestDirectory::new();
        let path = directory.0.join("nested/settings.toml");
        let mut world = persistence_world(path.clone());
        advance(&mut world, 0);
        advance(&mut world, 499);
        assert!(!path.exists());
        world.resource_mut::<Simulation>().app.set_tick_rate_ms(100);
        advance(&mut world, 0);
        advance(&mut world, 499);
        assert!(!path.exists());
        advance(&mut world, 1);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            parse_settings_from_toml(&saved).simulation.tick_rate_ms,
            Some(100)
        );
        assert!(world.resource::<SettingsPersistence>().pending.is_none());
        // A marker detects unwanted rewrites without relying on filesystem timestamp resolution.
        std::fs::write(&path, "marker").unwrap();
        advance(&mut world, 1000);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "marker");
        world.resource_mut::<Simulation>().app.set_tick_rate_ms(200);
        advance(&mut world, 0);
        world.resource_mut::<Simulation>().app.set_tick_rate_ms(100);
        advance(&mut world, 1000);
        assert!(world.resource::<SettingsPersistence>().pending.is_none());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "marker");
    }

    #[test]
    fn failed_saves_keep_the_last_saved_state_and_retry_after_a_delay() {
        let directory = crate::test_support::TestDirectory::new();
        let path = directory.0.join("settings.toml");
        let mut world = persistence_world(path.clone());
        advance(&mut world, 0);
        advance(&mut world, 500);
        let last_saved = world.resource::<SettingsPersistence>().last_saved;
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();
        world.resource_mut::<GridOverlay>().enabled = true;
        advance(&mut world, 0);
        advance(&mut world, 500);
        assert!(world.resource::<SettingsPersistence>().last_saved == last_saved);
        assert!(world.resource::<SettingsPersistence>().pending.is_some());
        assert_eq!(std::fs::read_dir(&directory.0).unwrap().count(), 1);
        std::fs::remove_dir(&path).unwrap();
        std::fs::write(&path, "previous file").unwrap();
        advance(&mut world, 499);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "previous file");
        advance(&mut world, 1);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert_eq!(parse_settings_from_toml(&saved).grid, Some(true));
        assert!(world.resource::<SettingsPersistence>().pending.is_none());
    }

    #[test]
    fn an_exit_in_the_final_schedule_flushes_the_latest_settings_without_waiting() {
        let directory = crate::test_support::TestDirectory::new();
        let path = directory.0.join("settings.toml");
        let mut app = App::new();
        populate_world(app.world_mut(), path.clone());
        app.add_systems(
            Last,
            persist_settings
                .in_set(OnAppExitSystems)
                .after(bevy::window::ExitSystems),
        );
        app.world_mut().run_schedule(Last);
        assert!(!path.exists());
        // Window close emits AppExit in Last; the settings system must run afterwards.
        app.add_systems(
            Last,
            (|mut exits: MessageWriter<AppExit>| {
                exits.write(AppExit::Success);
            })
            .in_set(bevy::window::ExitSystems),
        );
        app.world_mut()
            .resource_mut::<Simulation>()
            .app
            .set_tick_rate_ms(250);
        app.world_mut().run_schedule(Last);
        let saved = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            parse_settings_from_toml(&saved).simulation.tick_rate_ms,
            Some(250)
        );
        assert!(
            app.world()
                .resource::<SettingsPersistence>()
                .pending
                .is_none()
        );
    }

    #[test]
    fn startup_without_a_simulation_or_board_size_never_overwrites_settings() {
        let directory = crate::test_support::TestDirectory::new();
        let path = directory.0.join("settings.toml");
        std::fs::write(&path, "keep existing settings").unwrap();
        let mut world = persistence_world(path.clone());
        world.resource_mut::<BoardSettings>().cols = 0;
        advance(&mut world, 1000);
        world.resource_mut::<BoardSettings>().cols = 16;
        world.remove_resource::<Simulation>();
        advance(&mut world, 1000);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "keep existing settings"
        );
    }

    #[test]
    fn legacy_settings_load_save_and_reload_without_a_ui() {
        let directory = std::env::temp_dir().join(format!(
            "gol-settings-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("settings.toml");
        std::fs::write(
            &path,
            "layout_preset = \"minimal\"\nboard_cols = 512\nboard_rows = 256\n",
        )
        .unwrap();
        let mut layout = UiLayout::default();
        let mut board = BoardSettings::default();
        let mut grid = GridOverlay { enabled: false };
        let mut cell_age = CellAge::default();
        let mut theme = Theme::default();
        // A file from before the simulation settings were saved leaves them alone.
        let legacy = load_settings(
            &path,
            &mut layout,
            &mut board,
            &mut grid,
            &mut cell_age,
            &mut theme,
        );
        assert!(!layout.control_deck_visible);
        assert_eq!((board.cols, board.rows), (512, 256));
        assert!(!board.fill_canvas);
        assert_eq!(legacy, SimulationSettings::default());
        assert!(!grid.enabled);
        assert!(cell_age.enabled);
        assert_eq!(theme, Theme::Dark);

        let mut app = crate::app::App::new(128, 64);
        let highlife = Rule::parse("B36/S23").unwrap();
        app.set_rule(highlife);
        app.toggle_boundary_mode();
        app.set_tick_rate_ms(100);
        app.set_max_speed();
        let mut world = World::new();
        layout.control_deck_visible = true;
        board.set(128, 64);
        board.fill_canvas = true;
        world.insert_resource(layout);
        world.insert_resource(board);
        world.insert_resource(GridOverlay { enabled: true });
        world.insert_resource(CellAge { enabled: false });
        world.insert_resource(Theme::HighContrast);
        world.insert_resource(Simulation::new(app));
        world.insert_resource(SettingsPersistence {
            path: path.clone(),
            last_saved: None,
            pending: None,
        });
        world.init_resource::<Time<Real>>();
        world.init_resource::<Messages<AppExit>>();
        world.run_system_once(persist_settings).unwrap();
        world.resource_mut::<Time<Real>>().advance_by(SAVE_DEBOUNCE);
        world.run_system_once(persist_settings).unwrap();

        let mut loaded_layout = UiLayout {
            control_deck_visible: false,
        };
        let mut loaded_board = BoardSettings::default();
        let mut loaded_grid = GridOverlay { enabled: false };
        let mut loaded_cell_age = CellAge::default();
        let mut loaded_theme = Theme::default();
        let simulation = load_settings(
            &path,
            &mut loaded_layout,
            &mut loaded_board,
            &mut loaded_grid,
            &mut loaded_cell_age,
            &mut loaded_theme,
        );
        assert!(loaded_layout.control_deck_visible);
        assert_eq!((loaded_board.cols, loaded_board.rows), (128, 64));
        assert!(loaded_board.fill_canvas);
        assert!(loaded_grid.enabled);
        assert!(!loaded_cell_age.enabled);
        assert_eq!(loaded_theme, Theme::HighContrast);

        let mut restored = Simulation::new(crate::app::App::new(128, 64));
        simulation.apply_to(&mut restored);
        assert_eq!(restored.app.board.rule(), highlife);
        assert_eq!(restored.app.board.boundary_mode(), BoundaryMode::Fixed);
        assert_eq!(restored.app.tick_rate_ms(), 100);
        assert!(restored.app.is_max_speed());
        // Restoring settings is not an edit the user could undo.
        assert!(!restored.app.can_undo());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn reads_palette_visibility_and_board_size() {
        let parsed = parse_settings_from_toml(
            "# saved by gol-bevy\npalette = false\nboard_cols = 512\nboard_rows = 256\n",
        );
        assert_eq!(parsed.palette, Some(false));
        assert_eq!(parsed.board, Some((512, 256)));
    }

    #[test]
    fn old_layout_presets_only_hide_the_palette_when_minimal() {
        for (preset, visible) in [("full", true), ("compact", true), ("minimal", false)] {
            let parsed = parse_settings_from_toml(&format!("layout_preset = \"{preset}\"\n"));
            assert_eq!(parsed.palette, Some(visible));
        }
        // A saved palette setting wins over a leftover preset, in either order.
        let both = parse_settings_from_toml("palette = true\nlayout_preset = \"minimal\"\n");
        assert_eq!(both.palette, Some(true));
        let both = parse_settings_from_toml("layout_preset = \"minimal\"\npalette = true\n");
        assert_eq!(both.palette, Some(true));
    }

    #[test]
    fn a_partial_or_invalid_file_leaves_fields_unset() {
        // Only one of the two dimensions is useless on its own.
        let half = parse_settings_from_toml("palette = 'false'\nboard_cols = 512\n");
        assert_eq!(half.palette, Some(false));
        assert_eq!(half.board, None);

        let junk = parse_settings_from_toml(
            "layout_preset = \"nope\"\npalette = maybe\nboard_cols = wide\nrule = \"B9\"\nboundary_mode = \"round\"\n\
             tick_rate_ms = fast\nmax_speed = yes\ngrid = 1\ncell_age = on\ntheme = \"neon\"\n",
        );
        assert_eq!(junk.palette, None);
        assert_eq!(junk.board, None);
        assert_eq!(junk.simulation, super::SimulationSettings::default());
        assert_eq!(junk.grid, None);
        assert_eq!(junk.cell_age, None);
        assert_eq!(junk.theme, None);

        assert_eq!(parse_settings_from_toml("").board, None);
    }

    #[test]
    fn settings_are_read_from_older_files_until_the_new_one_exists() {
        let directory = std::env::temp_dir().join(format!(
            "gol-settings-move-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let path = directory.join("new").join("settings.toml");
        let renamed = directory.join("new").join("ui_layout.toml");
        let legacy = directory.join("old").join("ui_layout.toml");

        let older = || [renamed.clone(), legacy.clone()];

        // Nothing saved anywhere: read, and later save, the new location.
        assert_eq!(settings_source(&path, older()), path);

        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "board_cols = 64\nboard_rows = 64\n").unwrap();
        assert_eq!(settings_source(&path, older()), legacy);
        assert_eq!(settings_source(&path, None::<PathBuf>), path);

        // The old name in the current folder is newer than the old folder.
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&renamed, "board_cols = 96\nboard_rows = 96\n").unwrap();
        assert_eq!(settings_source(&path, older()), renamed);

        // Once the new file exists it wins, even though the old ones remain.
        std::fs::write(&path, "board_cols = 128\nboard_rows = 128\n").unwrap();
        assert_eq!(settings_source(&path, older()), path);
        assert!(renamed.exists() && legacy.exists());

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unquote_handles_both_quote_styles_and_bare_values() {
        assert_eq!(unquote("\"full\""), "full");
        assert_eq!(unquote("'full'"), "full");
        assert_eq!(unquote("512"), "512");
    }
}
