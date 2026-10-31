//! Notices when a board stops evolving, by comparing generation fingerprints.
use std::{collections::VecDeque, fmt};

/// How many generations back each one is compared, which bounds the periods
/// noticed. Periods of separate oscillators combine: 2, 3, 8, and 15 make 120.
const WINDOW: usize = 128;

/// A board that repeats. Each generation follows only from the one before it, so
/// once a generation comes round again the board repeats until it is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settled {
    /// The first generation of the repeating cycle.
    pub since: usize,
    /// Generations in the cycle; 1 for a board that no longer changes.
    pub period: usize,
}

impl Settled {
    /// A short form for the status bar, where a plain space could wrap.
    pub fn label(self) -> String {
        match self.period {
            1 => "stable".to_owned(),
            period => format!("period\u{a0}{period}"),
        }
    }
}

impl fmt::Display for Settled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.period {
            1 => write!(f, "stable since generation {}", self.since),
            period => write!(
                f,
                "repeating every {period} generations since generation {}",
                self.since
            ),
        }
    }
}

/// The fingerprints of recent generations. Fingerprints only compare within one
/// uninterrupted run, so anything else that changes the board must [`reset`] this.
///
/// [`reset`]: Self::reset
#[derive(Debug, Default)]
pub struct SettleWatch {
    recent: VecDeque<u64>,
    settled: Option<Settled>,
}

impl SettleWatch {
    pub fn reset(&mut self) {
        self.recent.clear();
        self.settled = None;
    }

    pub fn settled(&self) -> Option<Settled> {
        self.settled
    }

    /// Records a step from fingerprint `before` to `after`, which reached
    /// `generation`. Returns the result on the step that settles the board.
    pub fn record(&mut self, before: u64, after: u64, generation: usize) -> Option<Settled> {
        if self.settled.is_some() {
            return None;
        }
        if self.recent.is_empty() {
            self.recent.push_back(before);
        }
        if let Some(back) = self.recent.iter().rev().position(|&seen| seen == after) {
            let period = back + 1;
            self.settled = Some(Settled {
                since: generation.saturating_sub(period),
                period,
            });
            return self.settled;
        }
        if self.recent.len() == WINDOW {
            self.recent.pop_front();
        }
        self.recent.push_back(after);
        None
    }

    /// The generation counter went back to 0, renumbering the cycle if there is one.
    pub fn renumber(&mut self) {
        if let Some(settled) = &mut self.settled {
            settled.since = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Records a run of fingerprints from generation 0, returning what settled it.
    fn watch(fingerprints: impl IntoIterator<Item = u64>) -> Option<Settled> {
        let mut watch = SettleWatch::default();
        let mut fingerprints = fingerprints.into_iter();
        let mut before = fingerprints.next().expect("a first generation");
        let mut settled = None;
        for (generation, after) in (1..).zip(fingerprints) {
            settled = settled.or(watch.record(before, after, generation));
            before = after;
        }
        settled
    }

    #[test]
    fn a_board_that_stops_changing_has_period_one() {
        assert_eq!(
            watch([1, 2, 3, 3, 3]),
            Some(Settled {
                since: 2,
                period: 1
            })
        );
    }

    #[test]
    fn the_cycle_starts_at_its_first_generation() {
        assert_eq!(
            watch([1, 2, 3, 4, 3, 4]),
            Some(Settled {
                since: 2,
                period: 2
            })
        );
    }

    #[test]
    fn a_cycle_longer_than_the_window_goes_unnoticed() {
        let cycle = |period: u64| (0..period * 3).map(move |generation| generation % period);
        assert_eq!(watch(cycle(WINDOW as u64)).unwrap().period, WINDOW);
        assert_eq!(watch(cycle(WINDOW as u64 + 1)), None);
    }

    #[test]
    fn only_the_settling_step_reports_and_reset_starts_over() {
        let mut watch = SettleWatch::default();
        assert_eq!(watch.record(5, 6, 1), None);
        let settled = watch.record(6, 6, 2);
        assert!(settled.is_some());
        assert_eq!(watch.record(6, 6, 3), None);
        assert_eq!(watch.settled(), settled);

        watch.reset();
        assert_eq!(watch.settled(), None);
        assert_eq!(watch.record(9, 8, 4), None);
    }

    #[test]
    fn renumbering_moves_the_cycle_to_generation_zero() {
        let mut watch = SettleWatch::default();
        watch.record(5, 5, 40);
        watch.renumber();
        assert_eq!(
            watch.settled(),
            Some(Settled {
                since: 0,
                period: 1
            })
        );

        // A step just after a renumbering cannot reach back before zero.
        let mut watch = SettleWatch::default();
        watch.record(1, 2, 40);
        assert_eq!(watch.record(2, 1, 1).map(|settled| settled.since), Some(0));
    }

    #[test]
    fn labels_and_descriptions_name_the_period() {
        let stable = Settled {
            since: 7,
            period: 1,
        };
        assert_eq!(stable.label(), "stable");
        assert_eq!(stable.to_string(), "stable since generation 7");
        let oscillating = Settled {
            since: 7,
            period: 15,
        };
        assert_eq!(oscillating.label(), "period\u{a0}15");
        assert_eq!(
            oscillating.to_string(),
            "repeating every 15 generations since generation 7"
        );
    }
}
