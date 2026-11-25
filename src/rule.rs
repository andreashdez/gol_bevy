//! Life-like rules: the neighbour counts at which a dead cell is born and a live
//! one survives, and the rulestrings that write them.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleParseError {
    Empty,
    Malformed,
    InvalidDigit(char),
    NeighborOutOfRange(u32),
}

impl std::fmt::Display for RuleParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => write!(f, "empty rulestring"),
            Self::Malformed => write!(f, "rulestring must look like 'B3/S23'"),
            Self::InvalidDigit(ch) => write!(f, "invalid neighbor count '{ch}' in rulestring"),
            Self::NeighborOutOfRange(n) => write!(f, "neighbor count {n} exceeds the maximum of 8"),
        }
    }
}

impl std::error::Error for RuleParseError {}

/// Birth and survival neighbor counts, as written in `B3/S23` notation. Each is a
/// bitmask over the nine possible neighbor counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rule {
    birth: u16,
    survive: u16,
}

impl Rule {
    /// Conway's Game of Life.
    pub const LIFE: Self = Self {
        birth: 1 << 3,
        survive: (1 << 2) | (1 << 3),
    };

    /// Accepts `B3/S23`, the equivalent `S23/B3`, `B3S23` without the slash, and
    /// the older `23/3` survival-first ordering. Golly's bounded-grid suffix, as
    /// in `B3/S23:T64,64`, is ignored: boards here have their own edges.
    pub fn parse(source: &str) -> Result<Self, RuleParseError> {
        let text = source.split_once(':').map_or(source, |(rule, _)| rule);
        let text = text.trim().to_ascii_lowercase();
        if text.is_empty() {
            return Err(RuleParseError::Empty);
        }
        let (left, right) = match text.split_once('/') {
            Some(halves) => halves,
            // Without the slash, the second letter starts the second half.
            None if text.starts_with(['b', 's']) => match text[1..].find(['b', 's']) {
                Some(at) => text.split_at(at + 1),
                None => return Err(RuleParseError::Malformed),
            },
            None => return Err(RuleParseError::Malformed),
        };
        let (left, right) = (left.trim(), right.trim());

        let (birth, survive) = match (
            left.strip_prefix('b'),
            right.strip_prefix('s'),
            left.strip_prefix('s'),
            right.strip_prefix('b'),
        ) {
            (Some(birth), Some(survive), _, _) => (birth, survive),
            (_, _, Some(survive), Some(birth)) => (birth, survive),
            _ if left
                .chars()
                .chain(right.chars())
                .all(|c| c.is_ascii_digit())
                && !(left.is_empty() && right.is_empty()) =>
            {
                // Legacy ordering puts survival counts first, e.g. `23/3`.
                (right, left)
            }
            _ => return Err(RuleParseError::Malformed),
        };

        Ok(Self {
            birth: counts_to_mask(birth)?,
            survive: counts_to_mask(survive)?,
        })
    }

    /// Whether a dead cell with no live neighbours is born, which on an unbounded
    /// board would light every cell of the plane.
    pub(crate) fn births_from_nothing(self) -> bool {
        self.allows_birth(0)
    }

    fn allows_birth(self, neighbors: u8) -> bool {
        self.birth & (1 << neighbors) != 0
    }

    fn allows_survival(self, neighbors: u8) -> bool {
        self.survive & (1 << neighbors) != 0
    }

    /// Birth counts in bits 0..=8 and survival counts in bits 9..=17, so the next
    /// state is bit `neighbors + 9 * alive` without branching on `alive`.
    pub(crate) fn transition_mask(self) -> u32 {
        u32::from(self.birth) | (u32::from(self.survive) << 9)
    }
}

impl Default for Rule {
    fn default() -> Self {
        Self::LIFE
    }
}

impl std::fmt::Display for Rule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "B")?;
        for n in 0..=8u8 {
            if self.allows_birth(n) {
                write!(f, "{n}")?;
            }
        }
        write!(f, "/S")?;
        for n in 0..=8u8 {
            if self.allows_survival(n) {
                write!(f, "{n}")?;
            }
        }
        Ok(())
    }
}

fn counts_to_mask(counts: &str) -> Result<u16, RuleParseError> {
    let mut mask = 0u16;
    for ch in counts.chars() {
        let digit = ch.to_digit(10).ok_or(RuleParseError::InvalidDigit(ch))?;
        if digit > 8 {
            return Err(RuleParseError::NeighborOutOfRange(digit));
        }
        mask |= 1 << digit;
    }
    Ok(mask)
}

#[cfg(test)]
mod tests {
    use super::{Rule, RuleParseError};

    #[test]
    fn rulestrings_parse_in_every_accepted_ordering() {
        let life = Rule::LIFE;
        assert_eq!(Rule::parse("B3/S23"), Ok(life));
        assert_eq!(Rule::parse("b3/s23"), Ok(life));
        assert_eq!(Rule::parse(" B3 / S23 "), Ok(life));
        assert_eq!(Rule::parse("S23/B3"), Ok(life));
        // Legacy ordering lists survival counts first.
        assert_eq!(Rule::parse("23/3"), Ok(life));
        assert_eq!(life.to_string(), "B3/S23");

        let highlife = Rule::parse("B36/S23").expect("HighLife should parse");
        assert_ne!(highlife, life);
        assert_eq!(highlife.to_string(), "B36/S23");

        assert_eq!(Rule::parse(""), Err(RuleParseError::Empty));
        assert_eq!(Rule::parse("B3S23"), Ok(life));
        assert_eq!(Rule::parse("s23b3"), Ok(life));
        assert_eq!(Rule::parse("B3/S23:T64,64"), Ok(life));
        assert_eq!(Rule::parse("B/S").unwrap().to_string(), "B/S");
        assert_eq!(Rule::parse(":T64,64"), Err(RuleParseError::Empty));
        for malformed in ["/", "B3", "B3B3", "323"] {
            assert_eq!(Rule::parse(malformed), Err(RuleParseError::Malformed));
        }
        assert_eq!(
            Rule::parse("B3/S2x"),
            Err(RuleParseError::InvalidDigit('x'))
        );
        assert_eq!(
            Rule::parse("B9/S23"),
            Err(RuleParseError::NeighborOutOfRange(9))
        );
    }
}
