//! The pattern library and the stamp: choosing a pattern or pasting one,
//! importing RLE into the library, turning and flipping the stamp, and placing
//! it on the board.

use super::{App, EditTool};
use crate::{
    board::Rule,
    patterns::{Pattern, PatternParseError},
};
use std::fmt;

/// What an RLE import changed beyond adding a stamp.
#[derive(Debug, Clone, Copy, Default)]
pub struct PatternImport {
    /// Set when the pattern declared a rule that differed from the board's.
    pub applied_rule: Option<Rule>,
}

/// Why clipboard text can't become the stamp.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PasteError {
    Parse(PatternParseError),
    Empty,
}

impl fmt::Display for PasteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // Ordinary text has no header with the pattern's size.
            Self::Parse(PatternParseError::MissingHeader | PatternParseError::MissingDimension) => {
                f.write_str("The clipboard holds no RLE pattern")
            }
            Self::Parse(error) => write!(f, "Could not paste the clipboard's RLE: {error}"),
            Self::Empty => f.write_str("The pattern on the clipboard has no live cells"),
        }
    }
}

/// Built-in patterns that tests select by name.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternKind {
    Glider,
    Blinker,
    Lwss,
}

#[cfg(test)]
impl PatternKind {
    /// Name of the file under `assets/patterns/`, without `.rle`.
    fn file_stem(self) -> &'static str {
        match self {
            Self::Glider => "glider",
            Self::Blinker => "blinker",
            Self::Lwss => "lwss",
        }
    }

    /// Looked up by file name, so the order of the built-in list is free to
    /// change. `None` only if the file has been removed.
    pub fn builtin_index(self) -> Option<usize> {
        Pattern::builtin_index(self.file_stem())
    }
}

type PatternLayout = (Vec<(usize, usize)>, usize, usize);

/// How the stamp is turned before it is placed.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct StampTransform {
    rotation_quarters: u8,
    flip_horizontal: bool,
}

impl App {
    #[cfg(test)]
    pub fn select_pattern(&mut self, pattern: PatternKind) {
        let index = pattern.builtin_index().expect("the built-in exists");
        self.select_pattern_index(index);
    }

    pub fn import_rle_pattern(
        &mut self,
        name: impl Into<String>,
        source: &str,
    ) -> Result<PatternImport, PatternParseError> {
        let pattern = Pattern::from_rle(name, source)?;
        if pattern.name.len() > crate::patterns::MAX_PATTERN_NAME_BYTES {
            return Err(PatternParseError::NameTooLong);
        }
        let imported = &self.patterns[Pattern::builtin_count()..];
        let cells: usize = imported.iter().map(|p| p.cells.len()).sum();
        if imported.len() >= crate::patterns::MAX_LIBRARY_PATTERNS
            || cells + pattern.cells.len() > crate::patterns::MAX_LIBRARY_CELLS
        {
            return Err(PatternParseError::LibraryFull);
        }
        if let Some(rule) = pattern.rule
            && !self.board.allows_rule(rule)
        {
            return Err(PatternParseError::UnboundedRule(rule));
        }

        // Honour a declared rule instead of simulating the pattern as plain Life.
        let applied_rule = pattern.rule.filter(|&rule| self.set_rule(rule));

        self.patterns.push(pattern);
        self.library_revision = self.library_revision.wrapping_add(1);
        self.selected_pattern = Some(self.patterns.len() - 1);
        self.pasted = None;
        self.tool = EditTool::Stamp;
        self.selection = None;
        self.palette_hidden = false;
        self.needs_redraw = true;
        Ok(PatternImport { applied_rule })
    }

    pub fn clear_selected_pattern(&mut self) {
        self.tool = EditTool::Draw;
        self.selection = None;
        self.palette_hidden = false;
        if self.selected_pattern.take().is_some() | self.pasted.take().is_some() {
            self.needs_redraw = true;
        }
    }

    /// The library pattern that is the stamp. `None` while a paste is.
    pub fn selected_pattern_index(&self) -> Option<usize> {
        self.selected_pattern
            .filter(|&idx| self.pasted.is_none() && idx < self.patterns.len())
    }

    /// The pattern a click places: a paste, or else the library's choice.
    pub(super) fn stamp_pattern(&self) -> Option<&Pattern> {
        self.pasted
            .as_ref()
            .or_else(|| self.selected_pattern.and_then(|idx| self.patterns.get(idx)))
    }

    /// Makes RLE, such as a copied selection, the stamp, without adding it to
    /// the library. It starts unrotated, as it was copied, and the rule in its
    /// header is left alone: a paste brings cells, not a rule.
    pub fn paste_rle(&mut self, source: &str) -> Result<&Pattern, PasteError> {
        let pattern = Pattern::from_named_rle("Pasted", source).map_err(PasteError::Parse)?;
        if pattern.cells.is_empty() {
            return Err(PasteError::Empty);
        }
        self.pasted = Some(pattern);
        self.paste_revision = self.paste_revision.wrapping_add(1);
        self.stamp_transform = StampTransform::default();
        self.set_tool(EditTool::Stamp);
        Ok(self.pasted.as_ref().expect("just pasted"))
    }

    /// Changes whenever a paste replaces the stamp.
    pub fn paste_revision(&self) -> u64 {
        self.paste_revision
    }

    pub fn library_revision(&self) -> u64 {
        self.library_revision
    }

    pub fn pattern_count(&self) -> usize {
        self.patterns.len()
    }

    pub fn pattern_name_at(&self, index: usize) -> Option<&str> {
        self.patterns
            .get(index)
            .map(|pattern| pattern.name.as_str())
    }

    pub fn pattern_at(&self, index: usize) -> Option<&Pattern> {
        self.patterns.get(index)
    }

    pub fn select_pattern_index(&mut self, index: usize) -> bool {
        if index >= self.patterns.len()
            || (self.selected_pattern_index() == Some(index) && self.tool == EditTool::Stamp)
        {
            return false;
        }

        self.selected_pattern = Some(index);
        self.pasted = None;
        self.tool = EditTool::Stamp;
        self.selection = None;
        self.needs_redraw = true;
        true
    }

    pub fn has_selected_pattern(&self) -> bool {
        self.tool == EditTool::Stamp && self.stamp_pattern().is_some()
    }

    pub fn selected_pattern_name(&self) -> Option<&str> {
        self.stamp_pattern().map(|pattern| pattern.name.as_str())
    }

    pub fn selected_pattern_status_label(&self) -> String {
        let Some(name) = self.selected_pattern_name() else {
            return "Draw".to_string();
        };
        if self.stamp_transform.rotation_quarters == 0 && !self.stamp_transform.flip_horizontal {
            return name.to_string();
        }

        let mut suffix = String::new();
        if self.stamp_transform.flip_horizontal {
            suffix.push_str("FlipH");
        }
        if self.stamp_transform.rotation_quarters != 0 {
            if !suffix.is_empty() {
                suffix.push(' ');
            }
            let degrees = self.stamp_transform.rotation_quarters as u32 * 90;
            suffix.push_str(&format!("R{degrees}"));
        }
        format!("{name} ({suffix})")
    }

    pub fn stamp_orientation(&self) -> (u8, bool) {
        (
            self.stamp_transform.rotation_quarters,
            self.stamp_transform.flip_horizontal,
        )
    }

    pub fn rotate_stamp_clockwise(&mut self) {
        self.stamp_transform.rotation_quarters = (self.stamp_transform.rotation_quarters + 1) % 4;
        self.needs_redraw = true;
    }

    pub fn toggle_stamp_flip(&mut self) {
        self.stamp_transform.flip_horizontal = !self.stamp_transform.flip_horizontal;
        self.needs_redraw = true;
    }

    pub fn selected_pattern_layout(&self) -> Option<PatternLayout> {
        if self.tool != EditTool::Stamp {
            return None;
        }
        self.stamp_pattern()
            .map(|pattern| self.transform_pattern(&pattern.cells, pattern.width, pattern.height))
    }

    pub fn place_selected_pattern_at(&mut self, col: i64, row: i64) {
        let Some((pattern, width, height)) = self
            .stamp_pattern()
            .map(|pattern| self.transform_pattern(&pattern.cells, pattern.width, pattern.height))
        else {
            return;
        };
        self.place_pattern_at(&pattern, width, height, col, row);
    }

    fn place_pattern_at(
        &mut self,
        pattern: &[(usize, usize)],
        width: usize,
        height: usize,
        center_col: i64,
        center_row: i64,
    ) {
        let (origin_x, origin_y) = self
            .board
            .stamp_origin((center_col, center_row), (width, height));
        let changes_board = pattern.iter().any(|&(dx, dy)| {
            let (col, row) = (origin_x + dx as i64, origin_y + dy as i64);
            self.board.contains(col, row) && !self.board.is_alive(col, row)
        });
        if !changes_board {
            return;
        }
        self.end_stroke();
        self.save_edit();
        let mut changed = false;

        for &(dx, dy) in pattern {
            changed |= self
                .board
                .populate(origin_x + dx as i64, origin_y + dy as i64);
        }

        if changed {
            self.needs_redraw = true;
        }
    }

    fn transform_pattern(
        &self,
        pattern: &[(usize, usize)],
        width: usize,
        height: usize,
    ) -> (Vec<(usize, usize)>, usize, usize) {
        let mut transformed = Vec::with_capacity(pattern.len());
        for &(x0, y0) in pattern {
            let mut x = x0;
            let mut y = y0;
            let mut w = width;
            let mut h = height;

            if self.stamp_transform.flip_horizontal {
                x = w.saturating_sub(1).saturating_sub(x);
            }

            for _ in 0..self.stamp_transform.rotation_quarters {
                let nx = h.saturating_sub(1).saturating_sub(y);
                let ny = x;
                x = nx;
                y = ny;
                std::mem::swap(&mut w, &mut h);
            }

            transformed.push((x, y));
        }

        let (final_w, final_h) = match self.stamp_transform.rotation_quarters {
            1 | 3 => (height, width),
            _ => (width, height),
        };
        (transformed, final_w, final_h)
    }
}
