//! Versioned workspace files. Decode completely before touching the live app.
use crate::{
    board::{Board, BoundaryMode, RandomizeSettings, Rule},
    patterns::{MAX_LIBRARY_CELLS, MAX_LIBRARY_PATTERNS, MAX_PATTERN_NAME_BYTES, Pattern, to_rle},
    shared::{BOARD_SIZE_MAX, BOARD_SIZE_MIN},
    sparse_board::PLANE_EXPORT_CELLS_MAX,
    universe::{Universe, plane_from_rle, plane_to_rle},
};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path};

const MAX_WORKSPACE_BYTES: u64 = 32 * 1024 * 1024;
const FORMAT: &str = "gol-bevy-workspace";
const FINITE_VERSION: u32 = 1;
/// Finite boards stay at the old version, for builds that know no other.
const UNBOUNDED_VERSION: u32 = 2;
const UNBOUNDED: &str = "unbounded";

#[derive(Debug)]
pub struct WorkspaceError(String);
impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for WorkspaceError {}
impl From<std::io::Error> for WorkspaceError {
    fn from(error: std::io::Error) -> Self {
        Self(error.to_string())
    }
}
impl From<serde_json::Error> for WorkspaceError {
    fn from(error: serde_json::Error) -> Self {
        Self(error.to_string())
    }
}
fn invalid(message: &str) -> WorkspaceError {
    WorkspaceError(message.into())
}

fn default_random_density() -> u8 {
    25
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Workspace {
    format: String,
    version: u32,
    width: usize,
    height: usize,
    generation: u64,
    rule: String,
    boundary: String,
    tick_rate_ms: u64,
    max_speed: bool,
    #[serde(default = "default_random_density")]
    random_density_percent: u8,
    #[serde(default)]
    random_seed: Option<u64>,
    board_rle: String,
    /// On an unbounded board, the lowest corner of `board_rle`. `width` and
    /// `height` are then the size of the finite board it returns to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plane_origin: Option<(i64, i64)>,
    imported_patterns: Vec<StoredPattern>,
}
#[derive(Debug, Serialize, Deserialize)]
struct StoredPattern {
    name: String,
    rle: String,
}

pub(crate) struct DecodedWorkspace {
    pub width: usize,
    pub height: usize,
    pub generation: usize,
    pub tick_rate_ms: u64,
    pub max_speed: bool,
    pub randomize_settings: RandomizeSettings,
    pub board: Universe,
    pub patterns: Vec<Pattern>,
}

impl Workspace {
    pub(crate) fn capture(
        app: &crate::app::App,
        patterns: &[Pattern],
    ) -> Result<Self, WorkspaceError> {
        let (width, height) = app.finite_size();
        if !(BOARD_SIZE_MIN..=BOARD_SIZE_MAX).contains(&width)
            || !(BOARD_SIZE_MIN..=BOARD_SIZE_MAX).contains(&height)
        {
            return Err(invalid("Workspace dimensions must be between 8 and 2048."));
        }
        // Import enforces the same limits, so this only guards against drift.
        // The board is bounded by its dimensions and does not count here.
        if patterns.len() > MAX_LIBRARY_PATTERNS {
            return Err(invalid(
                "A workspace can contain at most 256 imported patterns.",
            ));
        }
        let mut total = 0usize;
        for pattern in patterns {
            total = total.saturating_add(pattern.cells.len());
            if total > MAX_LIBRARY_CELLS || pattern.name.len() > MAX_PATTERN_NAME_BYTES {
                return Err(invalid(
                    "Imported patterns exceed the 4,194,304 live-cell or 1024-byte name limit.",
                ));
            }
        }
        let rule = app.board.rule();
        let (version, boundary, board_rle, plane_origin) = match &app.board {
            Universe::Finite(board) => {
                let cells: Vec<_> = (0..height)
                    .flat_map(|y| (0..width).map(move |x| (x, y)))
                    .filter(|&(x, y)| board.is_alive(x, y))
                    .collect();
                let boundary = match board.boundary_mode() {
                    BoundaryMode::Wrap => "wrap",
                    BoundaryMode::Fixed => "fixed",
                };
                let rle = to_rle(&cells, width, height, Some(rule), "");
                (FINITE_VERSION, boundary, rle, None)
            }
            Universe::Unbounded(plane) => {
                if plane.live_cells_count() > PLANE_EXPORT_CELLS_MAX {
                    return Err(invalid(
                        "An unbounded board can be saved with at most 67,108,864 live cells.",
                    ));
                }
                let (rle, origin) = plane_to_rle(plane, "").map_or_else(
                    || (to_rle(&[], 0, 0, Some(rule), ""), (0, 0)),
                    |export| (export.rle, export.origin),
                );
                (UNBOUNDED_VERSION, UNBOUNDED, rle, Some(origin))
            }
        };
        Ok(Self {
            format: FORMAT.into(),
            version,
            width,
            height,
            generation: app.epoch as u64,
            rule: app.rule_label(),
            boundary: boundary.into(),
            tick_rate_ms: app.tick_rate_ms(),
            max_speed: app.is_max_speed(),
            random_density_percent: app.randomize_settings().density_percent(),
            random_seed: app.randomize_settings().seed(),
            board_rle,
            plane_origin,
            imported_patterns: patterns
                .iter()
                .map(|pattern| StoredPattern {
                    name: pattern.name.clone(),
                    // Preserve the absence of an imported rule as well as an explicit rule.
                    rle: to_rle(
                        &pattern.cells,
                        pattern.width,
                        pattern.height,
                        pattern.rule,
                        "",
                    ),
                })
                .collect(),
        })
    }

    pub(crate) fn decode(self) -> Result<DecodedWorkspace, WorkspaceError> {
        if self.format != FORMAT || !(FINITE_VERSION..=UNBOUNDED_VERSION).contains(&self.version) {
            return Err(invalid("Unsupported workspace format or version."));
        }
        if !(BOARD_SIZE_MIN..=BOARD_SIZE_MAX).contains(&self.width)
            || !(BOARD_SIZE_MIN..=BOARD_SIZE_MAX).contains(&self.height)
        {
            return Err(invalid("Workspace dimensions must be between 8 and 2048."));
        }
        let generation =
            usize::try_from(self.generation).map_err(|_| invalid("Generation is too large."))?;
        if !(25..=1000).contains(&self.tick_rate_ms) {
            return Err(invalid("Invalid workspace playback interval."));
        }
        let randomize_settings =
            RandomizeSettings::new(self.random_density_percent, self.random_seed)
                .ok_or_else(|| invalid("Random density must be between 0 and 100 percent."))?;
        let rule = Rule::parse(&self.rule).map_err(|e| WorkspaceError(e.to_string()))?;
        let board = match self.boundary.as_str() {
            "wrap" => self.decode_board(rule, BoundaryMode::Wrap)?,
            "fixed" => self.decode_board(rule, BoundaryMode::Fixed)?,
            UNBOUNDED if self.version == UNBOUNDED_VERSION => self.decode_plane(rule)?,
            _ => return Err(invalid("Invalid workspace boundary mode.")),
        };
        if self.imported_patterns.len() > MAX_LIBRARY_PATTERNS {
            return Err(invalid(
                "A workspace can contain at most 256 imported patterns.",
            ));
        }
        // Each pattern is bounded by the RLE limits; the running total stops a
        // file of highly compressed patterns from expanding without limit.
        let mut total = 0usize;
        let mut patterns = Vec::new();
        for stored in self.imported_patterns {
            if stored.name.len() > MAX_PATTERN_NAME_BYTES {
                return Err(invalid("Pattern names may contain at most 1024 bytes."));
            }
            let pattern = Pattern::from_rle(stored.name, &stored.rle)
                .map_err(|e| WorkspaceError(e.to_string()))?;
            total = total.saturating_add(pattern.cells.len());
            if total > MAX_LIBRARY_CELLS {
                return Err(invalid(
                    "Imported patterns exceed the 4,194,304 live-cell limit.",
                ));
            }
            patterns.push(pattern);
        }
        Ok(DecodedWorkspace {
            width: self.width,
            height: self.height,
            generation,
            tick_rate_ms: self.tick_rate_ms,
            max_speed: self.max_speed,
            randomize_settings,
            board,
            patterns,
        })
    }

    fn decode_board(&self, rule: Rule, mode: BoundaryMode) -> Result<Universe, WorkspaceError> {
        let pattern = Pattern::from_rle("Board", &self.board_rle)
            .map_err(|e| WorkspaceError(e.to_string()))?;
        if (pattern.width, pattern.height) != (self.width, self.height)
            || pattern.rule != Some(rule)
        {
            return Err(invalid(
                "Board RLE dimensions or rule disagree with workspace metadata.",
            ));
        }
        let mut cells = vec![0; self.width * self.height];
        for (x, y) in pattern.cells {
            cells[y * self.width + x] = 1;
        }
        let mut board = Board::new(self.width, self.height);
        board.set_rule(rule);
        board.set_boundary_mode(mode);
        board.restore_cells(cells);
        Ok(Universe::Finite(board))
    }

    fn decode_plane(&self, rule: Rule) -> Result<Universe, WorkspaceError> {
        let origin = self
            .plane_origin
            .ok_or_else(|| invalid("An unbounded board needs its position."))?;
        let (mut plane, declared) =
            plane_from_rle(&self.board_rle, origin).map_err(|e| WorkspaceError(e.to_string()))?;
        if declared != Some(rule) {
            return Err(invalid(
                "Board RLE dimensions or rule disagree with workspace metadata.",
            ));
        }
        plane
            .set_rule(rule)
            .map_err(|e| WorkspaceError(e.to_string()))?;
        Ok(Universe::Unbounded(plane))
    }

    pub fn read(path: &Path) -> Result<Self, WorkspaceError> {
        // Take limits the actual read too, including a file that grows after opening.
        let mut source = Vec::new();
        std::fs::File::open(path)?
            .take(MAX_WORKSPACE_BYTES + 1)
            .read_to_end(&mut source)?;
        if source.len() as u64 > MAX_WORKSPACE_BYTES {
            return Err(invalid("Workspace files may contain at most 32 MiB."));
        }
        Ok(serde_json::from_slice(&source)?)
    }

    /// Write beside the destination and replace it only after the complete file is flushed.
    pub fn write(&self, path: &Path) -> Result<(), WorkspaceError> {
        let source = serde_json::to_vec(self)?;
        if source.len() as u64 > MAX_WORKSPACE_BYTES {
            return Err(invalid("Workspace files may contain at most 32 MiB."));
        }
        Ok(crate::atomic_file::write(path, &source)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{app::App, test_support::TestDirectory};
    use std::fs;

    #[test]
    fn file_round_trip_preserves_positions_generation_rules_speed_and_imported_library() {
        let directory = TestDirectory::new();
        let path = directory.0.join("résumé.golw");
        let mut app = App::new(40, 24);
        app.import_rle_pattern("No rule", "x = 1, y = 1\no!")
            .unwrap();
        app.import_rle_pattern(
            "Named \"pattern\" 🌱\nsecond line",
            "x = 3, y = 1, rule = B36/S23\n3o!",
        )
        .unwrap();
        app.board.set_boundary_mode(BoundaryMode::Fixed);
        app.populate_board(12, 13);
        app.populate_board(39, 23);
        app.epoch = 812;
        app.set_tick_rate_ms(137);
        app.set_max_speed();
        app.set_randomize_settings(RandomizeSettings::new(75, Some(0)).unwrap());
        app.toggle_running();
        let original = app.board.snapshot_cells();
        app.workspace().unwrap().write(&path).unwrap();
        let mut restored = App::new(8, 8);
        restored.populate_board(0, 0);
        let revision = restored.library_revision();
        restored
            .restore_workspace(Workspace::read(&path).unwrap())
            .unwrap();
        assert_eq!(restored.board.size(), Some((40, 24)));
        assert_eq!(restored.board.snapshot_cells(), original);
        assert_eq!(restored.epoch, 812);
        assert_eq!(restored.rule_label(), "B36/S23");
        assert_eq!(restored.board.boundary_mode(), Some(BoundaryMode::Fixed));
        assert_eq!(restored.tick_rate_ms(), 137);
        assert!(restored.is_max_speed());
        assert_eq!(restored.randomize_settings(), app.randomize_settings());
        assert!(!restored.is_running());
        assert!(!restored.can_undo() && !restored.can_redo());
        assert!(restored.library_revision() > revision);
        assert_eq!(restored.pattern_count(), app.pattern_count());
        for index in Pattern::builtin_count()..app.pattern_count() {
            let before = app.pattern_at(index).unwrap();
            let after = restored.pattern_at(index).unwrap();
            assert_eq!(
                (after.name.as_str(), after.width, after.height, after.rule),
                (
                    before.name.as_str(),
                    before.width,
                    before.height,
                    before.rule
                )
            );
            assert_eq!(after.cells, before.cells);
        }
        // Existing files can be replaced, including with an empty fixed-speed workspace.
        let empty = App::new(16, 16);
        empty.workspace().unwrap().write(&path).unwrap();
        restored
            .restore_workspace(Workspace::read(&path).unwrap())
            .unwrap();
        assert_eq!(restored.board.live_cells_count(), 0);
        assert_eq!(restored.board.size(), Some((16, 16)));
        assert!(!restored.is_max_speed());
        assert_eq!(restored.pattern_count(), Pattern::builtin_count());
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 1);
    }

    fn plane_app() -> App {
        let mut app = App::new(40, 24);
        app.set_rule(Rule::parse("B36/S23").unwrap());
        app.set_unbounded().unwrap();
        app
    }

    fn live_cells(app: &App) -> Vec<(i64, i64)> {
        let Universe::Unbounded(plane) = &app.board else {
            panic!("the board is unbounded");
        };
        let mut cells: Vec<_> = plane.live_cells().collect();
        cells.sort_unstable();
        cells
    }

    fn plane_json(app: &App) -> serde_json::Value {
        serde_json::to_value(app.workspace().unwrap()).unwrap()
    }

    #[test]
    fn an_unbounded_board_round_trips_through_a_file() {
        let directory = TestDirectory::new();
        let path = directory.0.join("plane.golw");
        let mut app = plane_app();
        let far = 1i64 << 40;
        for (x, y) in [(far, -far), (far + 1, -far), (far + 2, -far), (-far, far)] {
            app.populate_board(x, y);
        }
        app.epoch = 812;
        app.workspace().unwrap().write(&path).unwrap();

        let mut restored = App::new(8, 8);
        restored
            .restore_workspace(Workspace::read(&path).unwrap())
            .unwrap();
        assert_eq!(live_cells(&restored), live_cells(&app));
        assert_eq!(restored.epoch, 812);
        assert_eq!(restored.rule_label(), "B36/S23");
        assert_eq!(restored.finite_size(), (40, 24));
        assert!(!restored.can_undo() && !restored.can_redo());

        // And it carries on as the original does.
        restored.increment_epoch();
        app.increment_epoch();
        assert_eq!(live_cells(&restored), live_cells(&app));

        // The size in the file is the one it goes back to.
        restored.set_boundary_mode(BoundaryMode::Wrap);
        assert_eq!(restored.board.size(), Some((40, 24)));
    }

    #[test]
    fn an_empty_unbounded_board_round_trips() {
        let mut restored = App::new(8, 8);
        restored
            .restore_workspace(plane_app().workspace().unwrap())
            .unwrap();
        assert_eq!(restored.board.size(), None);
        assert_eq!(restored.board.live_cells_count(), 0);
        assert_eq!(restored.rule_label(), "B36/S23");
    }

    #[test]
    fn only_an_unbounded_board_needs_the_newer_version() {
        let finite = serde_json::to_value(App::new(16, 16).workspace().unwrap()).unwrap();
        assert_eq!(finite["version"], 1);
        assert!(finite.get("plane_origin").is_none());

        let plane = plane_json(&plane_app());
        assert_eq!(plane["version"], 2);
        assert_eq!(plane["boundary"], "unbounded");
        assert_eq!(plane["plane_origin"], serde_json::json!([0, 0]));
    }

    #[test]
    fn invalid_unbounded_workspaces_never_replace_the_current_app() {
        let mut app = plane_app();
        app.populate_board(5, 6);
        let original = plane_json(&app);
        let rule = "rule = B36/S23";
        let bad_fields = [
            ("version", serde_json::json!(1)),
            ("plane_origin", serde_json::Value::Null),
            ("plane_origin", serde_json::json!([i64::MAX, 0])),
            ("rule", serde_json::json!("B0/S23")),
            ("rule", serde_json::json!("B3/S23")),
            ("board_rle", serde_json::json!("x = 1, y = 1\no!")),
            (
                "board_rle",
                serde_json::json!(format!("x = 9000000000, y = 1, {rule}\n9000000000o!")),
            ),
        ];
        for (key, value) in bad_fields {
            let mut broken = original.clone();
            broken[key] = value.clone();
            let workspace = serde_json::from_value(broken).unwrap();
            assert!(app.restore_workspace(workspace).is_err(), "{key}: {value}");
            assert_eq!(plane_json(&app), original);
        }

        // B0 in both places agrees, and still can't go on the plane.
        let mut broken = original.clone();
        broken["rule"] = serde_json::json!("B0/S23");
        broken["board_rle"] = serde_json::json!("x = 1, y = 1, rule = B0/S23\no!");
        let workspace = serde_json::from_value(broken).unwrap();
        let error = app.restore_workspace(workspace).unwrap_err();
        assert!(error.to_string().contains("B0"), "{error}");
    }

    #[test]
    fn a_full_maximum_board_saves_alongside_a_full_imported_library() {
        use crate::patterns::MAX_PATTERN_SIDE;
        let side = MAX_PATTERN_SIDE;
        let mut app = App::new(BOARD_SIZE_MAX, BOARD_SIZE_MAX);
        app.set_randomize_settings(RandomizeSettings::new(100, None).unwrap());
        app.randomize_board();
        let full = format!(
            "x = {side}, y = {side}\n{}{side}o!",
            format!("{side}o$").repeat(side - 1)
        );
        app.import_rle_pattern("full", &full).unwrap();
        let mut restored = App::new(8, 8);
        restored
            .restore_workspace(app.workspace().unwrap())
            .unwrap();
        assert_eq!(
            restored.board.live_cells_count(),
            BOARD_SIZE_MAX * BOARD_SIZE_MAX
        );
        let pattern = restored.pattern_at(Pattern::builtin_count()).unwrap();
        assert_eq!(pattern.cells.len(), side * side);
    }

    #[test]
    fn imported_patterns_beyond_the_cell_budget_are_rejected() {
        let app = App::new(16, 16);
        let mut value = serde_json::to_value(app.workspace().unwrap()).unwrap();
        // Two maximum-size patterns are each valid but together exceed the budget.
        let full = serde_json::json!({"name": "full", "rle": "x = 2048, y = 2048\n2048o$".to_owned()
            + &"2048o$".repeat(2046) + "2048o!"});
        value["imported_patterns"] = serde_json::json!([full.clone(), full]);
        let mut restored = App::new(16, 16);
        let error = restored
            .restore_workspace(serde_json::from_value(value).unwrap())
            .unwrap_err();
        assert!(error.to_string().contains("4,194,304"), "{error}");
        assert_eq!(restored.pattern_count(), Pattern::builtin_count());
    }

    #[test]
    fn invalid_workspaces_never_replace_the_current_app() {
        let mut app = App::new(16, 16);
        app.populate_board(4, 5);
        app.import_rle_pattern("keep", "x = 1, y = 1\no!").unwrap();
        app.epoch = 23;
        app.toggle_running();
        let original = serde_json::to_value(app.workspace().unwrap()).unwrap();
        let bad_fields = [
            ("format", serde_json::json!("other")),
            ("version", serde_json::json!(3)),
            ("width", serde_json::json!(0)),
            ("height", serde_json::json!(2049)),
            ("rule", serde_json::json!("B9/S23")),
            ("boundary", serde_json::json!("other")),
            ("tick_rate_ms", serde_json::json!(0)),
            ("random_density_percent", serde_json::json!(101)),
            (
                "board_rle",
                serde_json::json!("x = 16, y = 16, rule = B3/S23\n99999999o!"),
            ),
            (
                "board_rle",
                serde_json::json!("x = 8, y = 8, rule = B3/S23\n!"),
            ),
            (
                "imported_patterns",
                serde_json::json!([{"name":"bad", "rle":"invalid"}]),
            ),
        ];
        for (key, value) in bad_fields {
            let mut broken = original.clone();
            broken[key] = value;
            let workspace = serde_json::from_value(broken).unwrap();
            assert!(app.restore_workspace(workspace).is_err(), "{key}");
            assert_eq!(
                serde_json::to_value(app.workspace().unwrap()).unwrap(),
                original
            );
            assert!(app.is_running() && app.can_undo());
        }
    }

    #[test]
    fn older_workspaces_default_to_unseeded_twenty_five_percent_randomization() {
        let app = App::new(16, 16);
        let mut value = serde_json::to_value(app.workspace().unwrap()).unwrap();
        let fields = value.as_object_mut().unwrap();
        fields.remove("random_density_percent");
        fields.remove("random_seed");
        let mut restored = App::new(8, 8);
        restored.set_randomize_settings(RandomizeSettings::new(100, Some(42)).unwrap());
        restored
            .restore_workspace(serde_json::from_value(value).unwrap())
            .unwrap();
        assert_eq!(restored.randomize_settings(), RandomizeSettings::default());
    }

    #[test]
    fn file_errors_and_size_limits_are_reported_without_leftover_temporary_files() {
        let directory = TestDirectory::new();
        let path = directory.0.join("bad.golw");
        assert!(Workspace::read(&path).is_err());
        fs::write(&path, b"{\"format\":").unwrap();
        assert!(Workspace::read(&path).is_err());
        fs::File::create(&path)
            .unwrap()
            .set_len(MAX_WORKSPACE_BYTES + 1)
            .unwrap();
        assert!(
            Workspace::read(&path)
                .unwrap_err()
                .to_string()
                .contains("32 MiB")
        );
        let workspace = App::new(8, 8).workspace().unwrap();
        assert!(
            workspace
                .write(&directory.0.join("missing/file.golw"))
                .is_err()
        );
        let destination = directory.0.join("existing-directory");
        fs::create_dir(&destination).unwrap();
        assert!(workspace.write(&destination).is_err());
        assert!(destination.is_dir());
        assert_eq!(fs::read_dir(&directory.0).unwrap().count(), 2);
    }
}
