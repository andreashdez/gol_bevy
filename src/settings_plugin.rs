//! Persisted workspace settings, independent of the UI implementation.
use crate::shared::{BoardSettings, StartupSet, UiLayout, UiLayoutPreset};
use bevy::prelude::*;
use std::path::PathBuf;

pub struct SettingsPlugin;
impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<UiLayout>()
            .init_resource::<BoardSettings>()
            .add_systems(
                Startup,
                setup_layout_persistence.in_set(StartupSet::LoadSettings),
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
}

#[derive(Default)]
struct ParsedSettings {
    preset: Option<UiLayoutPreset>,
    board: Option<(usize, usize)>,
}

fn setup_layout_persistence(
    mut commands: Commands,
    mut ui_layout: ResMut<UiLayout>,
    mut board_settings: ResMut<BoardSettings>,
) {
    let path = ui_layout_config_path();

    load_settings(&path, &mut ui_layout, &mut board_settings);

    commands.insert_resource(UiLayoutPersistence {
        path,
        last_saved: None,
    });
}

fn load_settings(
    path: &std::path::Path,
    ui_layout: &mut UiLayout,
    board_settings: &mut BoardSettings,
) {
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
        }
        Err(err) => {
            if err.kind() != std::io::ErrorKind::NotFound {
                warn!(
                    "Failed to read persisted settings from {}: {}",
                    path.display(),
                    err
                );
            }
        }
    }
}

fn persist_settings(
    ui_layout: Res<UiLayout>,
    board_settings: Res<BoardSettings>,
    mut persistence: ResMut<UiLayoutPersistence>,
) {
    if !board_settings.is_set() {
        return;
    }
    let current = PersistedSettings {
        preset: ui_layout.preset,
        board: (board_settings.cols, board_settings.rows),
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

    let contents = format!(
        "layout_preset = \"{}\"\nboard_cols = {}\nboard_rows = {}\n",
        current.preset.as_storage_str(),
        current.board.0,
        current.board.1
    );
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
            _ => {}
        }
    }

    if let (Some(cols), Some(rows)) = (cols, rows) {
        parsed.board = Some((cols, rows));
    }
    parsed
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
        load_settings(&path, &mut layout, &mut board);
        assert_eq!(layout.preset, UiLayoutPreset::Compact);
        assert_eq!((board.cols, board.rows), (512, 256));
        let mut world = World::new();
        layout.preset = UiLayoutPreset::Minimal;
        board.set(128, 64);
        world.insert_resource(layout);
        world.insert_resource(board);
        world.insert_resource(UiLayoutPersistence {
            path: path.clone(),
            last_saved: None,
        });
        world.run_system_once(persist_settings).unwrap();
        let mut loaded_layout = UiLayout::default();
        let mut loaded_board = BoardSettings::default();
        load_settings(&path, &mut loaded_layout, &mut loaded_board);
        assert_eq!(loaded_layout.preset, UiLayoutPreset::Minimal);
        assert_eq!((loaded_board.cols, loaded_board.rows), (128, 64));
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

        let junk = parse_settings_from_toml("layout_preset = \"nope\"\nboard_cols = wide\n");
        assert_eq!(junk.preset, None);
        assert_eq!(junk.board, None);

        assert_eq!(parse_settings_from_toml("").board, None);
    }

    #[test]
    fn unquote_handles_both_quote_styles_and_bare_values() {
        assert_eq!(unquote("\"full\""), "full");
        assert_eq!(unquote("'full'"), "full");
        assert_eq!(unquote("512"), "512");
    }
}
