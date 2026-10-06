//! Persisted workspace settings, independent of the UI implementation.
use crate::{
    board::{BoundaryMode, Rule},
    shared::{BoardSettings, GridOverlay, Simulation, StartupSet, UiLayout, UiLayoutPreset},
};
use bevy::prelude::*;
use std::path::PathBuf;

pub struct SettingsPlugin;
impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiLayout>()
            .init_resource::<BoardSettings>()
            .add_systems(
                Startup,
                (
                    setup_layout_persistence.in_set(StartupSet::LoadSettings),
                    apply_simulation_settings.after(StartupSet::BuildWorld),
                ),
            )
            .add_systems(Update, persist_settings);
    }
}

#[derive(Resource)]
struct UiLayoutPersistence {
    path: PathBuf,
    last_saved: Option<PersistedSettings>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PersistedSettings {
    preset: UiLayoutPreset,
    board: (usize, usize),
    simulation: SimulationSettings,
    grid: bool,
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
    preset: Option<UiLayoutPreset>,
    board: Option<(usize, usize)>,
    simulation: SimulationSettings,
    grid: Option<bool>,
}

fn setup_layout_persistence(
    mut commands: Commands,
    mut ui_layout: ResMut<UiLayout>,
    mut board_settings: ResMut<BoardSettings>,
    mut grid: ResMut<GridOverlay>,
) {
    let path = ui_layout_config_path();

    let simulation = load_settings(&path, &mut ui_layout, &mut board_settings, &mut grid);

    commands.insert_resource(simulation);
    commands.insert_resource(UiLayoutPersistence {
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
) -> SimulationSettings {
    match std::fs::read_to_string(path) {
        Ok(contents) => {
            let settings = parse_settings_from_toml(&contents);
            match settings.preset {
                Some(preset) => ui_layout.preset = preset,
                None => warn!(
                    "Ignoring invalid persisted layout preset in {}",
                    path.display()
                ),
            }
            // Left unset, startup falls back to a board that fills the window.
            if let Some((cols, rows)) = settings.board {
                board_settings.set(cols, rows);
            }
            if let Some(enabled) = settings.grid {
                grid.enabled = enabled;
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
    sim: Option<Res<Simulation>>,
    mut persistence: ResMut<UiLayoutPersistence>,
) {
    // Saving before the world is built would overwrite the file with defaults.
    let Some(sim) = sim else {
        return;
    };
    if !board_settings.is_set() {
        return;
    }
    let current = PersistedSettings {
        preset: ui_layout.preset,
        board: (board_settings.cols, board_settings.rows),
        simulation: SimulationSettings::of(&sim),
        grid: grid.enabled,
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
        "layout_preset = \"{}\"\nboard_cols = {}\nboard_rows = {}\ngrid = {}\n",
        current.preset.as_storage_str(),
        current.board.0,
        current.board.1,
        current.grid,
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
            "layout_preset" => parsed.preset = UiLayoutPreset::from_storage_str(value),
            "board_cols" => cols = value.parse::<usize>().ok(),
            "board_rows" => rows = value.parse::<usize>().ok(),
            "grid" => parsed.grid = value.parse::<bool>().ok(),
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

fn ui_layout_config_path() -> PathBuf {
    ui_layout_config_path_with_file("ui_layout.toml")
}

fn ui_layout_config_path_with_file(file_name: &str) -> PathBuf {
    if let Some(config_dir) = std::env::var_os("GOL_BEVY_CONFIG_DIR") {
        return PathBuf::from(config_dir).join(file_name);
    }

    if let Some(config_dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(config_dir).join("gol-bevy").join(file_name);
    }

    if let Some(home_dir) = std::env::var_os("HOME") {
        return PathBuf::from(home_dir)
            .join(".config")
            .join("gol-bevy")
            .join(file_name);
    }

    PathBuf::from(".gol-bevy").join("config").join(file_name)
}

#[cfg(test)]
mod persistence_tests {
    use super::{parse_settings_from_toml, unquote};
    use crate::shared::UiLayoutPreset;

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
        let path = directory.join("ui_layout.toml");
        std::fs::write(
            &path,
            "layout_preset = \"compact\"\nboard_cols = 512\nboard_rows = 256\n",
        )
        .unwrap();
        let mut layout = UiLayout::default();
        let mut board = BoardSettings::default();
        let mut grid = GridOverlay { enabled: false };
        // A file from before the simulation settings were saved leaves them alone.
        let legacy = load_settings(&path, &mut layout, &mut board, &mut grid);
        assert_eq!(layout.preset, UiLayoutPreset::Compact);
        assert_eq!((board.cols, board.rows), (512, 256));
        assert_eq!(legacy, SimulationSettings::default());
        assert!(!grid.enabled);

        let mut app = crate::app::App::new(128, 64);
        let highlife = Rule::parse("B36/S23").unwrap();
        app.set_rule(highlife);
        app.toggle_boundary_mode();
        app.set_tick_rate_ms(100);
        app.set_max_speed();
        let mut world = World::new();
        layout.preset = UiLayoutPreset::Minimal;
        board.set(128, 64);
        world.insert_resource(layout);
        world.insert_resource(board);
        world.insert_resource(GridOverlay { enabled: true });
        world.insert_resource(Simulation::new(app));
        world.insert_resource(UiLayoutPersistence {
            path: path.clone(),
            last_saved: None,
        });
        world.run_system_once(persist_settings).unwrap();

        let mut loaded_layout = UiLayout::default();
        let mut loaded_board = BoardSettings::default();
        let mut loaded_grid = GridOverlay { enabled: false };
        let simulation = load_settings(
            &path,
            &mut loaded_layout,
            &mut loaded_board,
            &mut loaded_grid,
        );
        assert_eq!(loaded_layout.preset, UiLayoutPreset::Minimal);
        assert_eq!((loaded_board.cols, loaded_board.rows), (128, 64));
        assert!(loaded_grid.enabled);

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
    fn reads_layout_preset_and_board_size() {
        let parsed = parse_settings_from_toml(
            "# saved by gol-bevy\nlayout_preset = \"compact\"\nboard_cols = 512\nboard_rows = 256\n",
        );
        assert_eq!(parsed.preset, Some(UiLayoutPreset::Compact));
        assert_eq!(parsed.board, Some((512, 256)));
    }

    #[test]
    fn a_partial_or_invalid_file_leaves_fields_unset() {
        // Only one of the two dimensions is useless on its own.
        let half = parse_settings_from_toml("layout_preset = 'minimal'\nboard_cols = 512\n");
        assert_eq!(half.preset, Some(UiLayoutPreset::Minimal));
        assert_eq!(half.board, None);

        let junk = parse_settings_from_toml(
            "layout_preset = \"nope\"\nboard_cols = wide\nrule = \"B9\"\nboundary_mode = \"round\"\n\
             tick_rate_ms = fast\nmax_speed = yes\ngrid = 1\n",
        );
        assert_eq!(junk.preset, None);
        assert_eq!(junk.board, None);
        assert_eq!(junk.simulation, super::SimulationSettings::default());
        assert_eq!(junk.grid, None);

        assert_eq!(parse_settings_from_toml("").board, None);
    }

    #[test]
    fn unquote_handles_both_quote_styles_and_bare_values() {
        assert_eq!(unquote("\"full\""), "full");
        assert_eq!(unquote("'full'"), "full");
        assert_eq!(unquote("512"), "512");
    }
}
