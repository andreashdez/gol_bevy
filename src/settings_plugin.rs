//! Persisted workspace settings, independent of the UI implementation.
use crate::{
    board::{BoundaryMode, Rule},
    shared::{BoardSettings, CellAge, GridOverlay, Simulation, StartupSet, Theme, UiLayout},
};
use bevy::prelude::*;
use std::path::PathBuf;

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
            .add_systems(Update, persist_settings);
    }
}

#[derive(Resource)]
struct SettingsPersistence {
    path: PathBuf,
    last_saved: Option<PersistedSettings>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PersistedSettings {
    palette: bool,
    board: (usize, usize),
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

fn persist_settings(
    ui_layout: Res<UiLayout>,
    board_settings: Res<BoardSettings>,
    grid: Res<GridOverlay>,
    cell_age: Res<CellAge>,
    theme: Res<Theme>,
    sim: Option<Res<Simulation>>,
    mut persistence: ResMut<SettingsPersistence>,
) {
    // Saving before the world is built would overwrite the file with defaults.
    let Some(sim) = sim else {
        return;
    };
    if !board_settings.is_set() {
        return;
    }
    let current = PersistedSettings {
        palette: ui_layout.control_deck_visible,
        board: (board_settings.cols, board_settings.rows),
        simulation: SimulationSettings::of(&sim),
        grid: grid.enabled,
        cell_age: cell_age.enabled,
        theme: *theme,
    };
    if persistence.last_saved == Some(current) {
        return;
    }

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
        "palette = {}\nboard_cols = {}\nboard_rows = {}\ngrid = {}\ncell_age = {}\ntheme = \"{}\"\n",
        current.palette,
        current.board.0,
        current.board.1,
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
    if let Err(err) = std::fs::write(&persistence.path, contents) {
        warn!(
            "Failed to persist settings to {}: {}",
            persistence.path.display(),
            err
        );
        return;
    }

    persistence.last_saved = Some(current);
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
    use super::{parse_settings_from_toml, settings_source, unquote};
    use std::path::PathBuf;

    #[test]
    fn legacy_settings_load_save_and_reload_without_a_ui() {
        use super::*;
        use bevy::ecs::system::RunSystemOnce;
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
        world.insert_resource(layout);
        world.insert_resource(board);
        world.insert_resource(GridOverlay { enabled: true });
        world.insert_resource(CellAge { enabled: false });
        world.insert_resource(Theme::HighContrast);
        world.insert_resource(Simulation::new(app));
        world.insert_resource(SettingsPersistence {
            path: path.clone(),
            last_saved: None,
        });
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
