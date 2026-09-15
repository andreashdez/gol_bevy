use crate::board::{Rule, RuleParseError};
use std::fmt;

#[derive(Debug, Clone)]
pub struct Pattern {
    pub name: String,
    pub cells: Vec<(usize, usize)>,
    pub width: usize,
    pub height: usize,
    /// Rule declared by the source RLE header, if it carried one.
    pub rule: Option<Rule>,
}

impl Pattern {
    pub fn new(
        name: impl Into<String>,
        cells: Vec<(usize, usize)>,
        width: usize,
        height: usize,
    ) -> Self {
        Self {
            name: name.into(),
            cells,
            width,
            height,
            rule: None,
        }
    }

    pub fn from_rle(name: impl Into<String>, source: &str) -> Result<Self, PatternParseError> {
        let name = name.into();
        let parsed = parse_rle(source)?;
        Ok(Self {
            name,
            cells: parsed.cells,
            width: parsed.width,
            height: parsed.height,
            rule: parsed.rule,
        })
    }

    pub fn builtins() -> Vec<Self> {
        vec![
            Self::new("Glider", vec![(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)], 3, 3),
            Self::new("Blinker", vec![(0, 0), (1, 0), (2, 0)], 3, 1),
            Self::new(
                "LWSS",
                vec![
                    (1, 0),
                    (4, 0),
                    (0, 1),
                    (0, 2),
                    (3, 2),
                    (0, 3),
                    (1, 3),
                    (2, 3),
                    (3, 3),
                ],
                5,
                4,
            ),
            Self::from_rle("RLE 8x4", "x = 8, y = 4\n8o$8o$8o$8o!")
                .expect("builtin RLE should parse"),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatternParseError {
    MissingHeader,
    MissingDimension,
    InvalidDimension,
    InvalidToken(char),
    MissingTerminator,
    Overflow,
    InvalidRule(RuleParseError),
}

impl fmt::Display for PatternParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingHeader => write!(f, "missing RLE header line"),
            Self::MissingDimension => write!(f, "missing x/y dimension in RLE header"),
            Self::InvalidDimension => write!(f, "invalid x/y dimension in RLE header"),
            Self::InvalidToken(ch) => write!(f, "invalid RLE token '{ch}'"),
            Self::MissingTerminator => write!(f, "RLE body missing '!' terminator"),
            Self::Overflow => write!(f, "RLE run length overflows pattern bounds"),
            Self::InvalidRule(err) => write!(f, "unsupported rule in RLE header: {err}"),
        }
    }
}

impl std::error::Error for PatternParseError {}

struct ParsedRle {
    cells: Vec<(usize, usize)>,
    width: usize,
    height: usize,
    rule: Option<Rule>,
}

fn parse_rle(source: &str) -> Result<ParsedRle, PatternParseError> {
    let mut header_line = None;
    let mut body = String::new();

    for raw_line in source.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if header_line.is_none() {
            header_line = Some(line.to_string());
        } else {
            body.push_str(line);
        }
    }

    let Some(header) = header_line else {
        return Err(PatternParseError::MissingHeader);
    };

    let RleHeader {
        width: declared_w,
        height: declared_h,
        rule,
    } = parse_header(&header)?;
    let mut cells = Vec::new();
    let mut x = 0usize;
    let mut y = 0usize;
    let mut max_x = 0usize;
    let mut max_y = 0usize;
    let mut run = 0usize;
    let mut terminated = false;

    for ch in body.chars() {
        match ch {
            '0'..='9' => {
                let digit = (ch as u8 - b'0') as usize;
                run = run
                    .checked_mul(10)
                    .and_then(|value| value.checked_add(digit))
                    .ok_or(PatternParseError::Overflow)?;
            }
            'b' => {
                let count = if run == 0 { 1 } else { run };
                run = 0;
                x = x.checked_add(count).ok_or(PatternParseError::Overflow)?;
                max_x = max_x.max(x);
            }
            'o' => {
                let count = if run == 0 { 1 } else { run };
                run = 0;
                for dx in 0..count {
                    cells.push((x + dx, y));
                }
                x = x.checked_add(count).ok_or(PatternParseError::Overflow)?;
                max_x = max_x.max(x);
                max_y = max_y.max(y + 1);
            }
            '$' => {
                let count = if run == 0 { 1 } else { run };
                run = 0;
                y = y.checked_add(count).ok_or(PatternParseError::Overflow)?;
                x = 0;
                max_y = max_y.max(y + 1);
            }
            '!' => {
                terminated = true;
                break;
            }
            ' ' | '\t' | '\r' | '\n' => {}
            _ => return Err(PatternParseError::InvalidToken(ch)),
        }
    }

    if !terminated {
        return Err(PatternParseError::MissingTerminator);
    }

    let width = declared_w.max(max_x);
    let height = declared_h.max(max_y);
    Ok(ParsedRle {
        cells,
        width,
        height,
        rule,
    })
}

struct RleHeader {
    width: usize,
    height: usize,
    rule: Option<Rule>,
}

fn parse_header(header: &str) -> Result<RleHeader, PatternParseError> {
    let mut width = None;
    let mut height = None;
    let mut rule = None;

    for entry in header.split(',') {
        let mut parts = entry.splitn(2, '=');
        let key = parts.next().map(str::trim).unwrap_or_default();
        let value = parts.next().map(str::trim).unwrap_or_default();

        match key {
            "x" => {
                width = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| PatternParseError::InvalidDimension)?,
                );
            }
            "y" => {
                height = Some(
                    value
                        .parse::<usize>()
                        .map_err(|_| PatternParseError::InvalidDimension)?,
                );
            }
            // A declared rule used to be discarded, which silently simulated
            // non-Life patterns under B3/S23.
            "rule" => {
                rule = Some(Rule::parse(value).map_err(PatternParseError::InvalidRule)?);
            }
            _ => {}
        }
    }

    match (width, height) {
        (Some(w), Some(h)) => Ok(RleHeader {
            width: w,
            height: h,
            rule,
        }),
        _ => Err(PatternParseError::MissingDimension),
    }
}

#[cfg(test)]
mod tests {
    use super::{Pattern, PatternParseError};

    #[test]
    fn parses_comments_and_run_lengths() {
        let pattern = Pattern::from_rle(
            "Three Lines",
            "#N demo\nx = 3, y = 3, rule = B3/S23\n3o$3b$3o!",
        )
        .expect("RLE should parse");

        assert_eq!(pattern.width, 3);
        assert_eq!(pattern.height, 3);
        assert_eq!(pattern.cells.len(), 6);
        assert!(pattern.cells.contains(&(0, 0)));
        assert!(pattern.cells.contains(&(2, 2)));
    }

    #[test]
    fn parses_row_break_run_counts() {
        let pattern =
            Pattern::from_rle("Row Jumps", "x = 4, y = 5\n2o2$2o!").expect("RLE should parse");

        assert!(pattern.cells.contains(&(0, 0)));
        assert!(pattern.cells.contains(&(1, 0)));
        assert!(pattern.cells.contains(&(0, 2)));
        assert!(pattern.cells.contains(&(1, 2)));
    }

    #[test]
    fn rejects_missing_terminator() {
        let result = Pattern::from_rle("Bad", "x = 3, y = 3\n3o$3o");
        assert_eq!(result.err(), Some(PatternParseError::MissingTerminator));
    }
}
