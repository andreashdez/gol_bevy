//! Running generations: playback and its speed, stepping the board, jumping to
//! a generation, noticing a board that settled or filled up, and the population
//! of the run so far.

use super::App;
use crate::settle::Settled;
use std::{collections::VecDeque, fmt};

/// Generations of population kept for the status bar's line.
pub const POPULATION_HISTORY: usize = 256;

/// Why playback stopped by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoPause {
    Settled(Settled),
    /// The unbounded board reached its memory budget.
    BoardFull,
    /// A jump reached its generation.
    Arrived(usize),
}

impl fmt::Display for AutoPause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Settled(settled) => settled.fmt(f),
            Self::BoardFull => f.write_str("the board reached its memory limit"),
            Self::Arrived(generation) => write!(f, "reached generation {generation}"),
        }
    }
}

/// Why a jump couldn't start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JumpError {
    /// Generations only run forward, and the board is already at or past it.
    Passed { current: usize },
    /// Past the last generation the counter holds.
    TooFar,
}

impl fmt::Display for JumpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Passed { current } => write!(
                f,
                "The board is already at generation {current}. Reset to start or undo to go back."
            ),
            Self::TooFar => f.write_str("That is past the last generation the counter holds."),
        }
    }
}

impl App {
    const TICK_RATE_MIN_MS: u64 = 25;
    const TICK_RATE_MAX_MS: u64 = 1000;
    const TICK_RATE_STEP_MS: u64 = 25;

    /// Pausing ends a jump, and playing again runs on at the set speed.
    pub fn toggle_running(&mut self) {
        self.running = !self.running;
        self.jump_target = None;
        self.needs_redraw = true;
    }

    pub fn is_running(&self) -> bool {
        self.running
    }

    /// Runs generations as fast as the frame allows until generation `target`,
    /// then pauses. It is one run, so Reset to start goes back to where it
    /// began; pausing ends it. Returns `target`.
    pub fn jump_to(&mut self, target: usize) -> Result<usize, JumpError> {
        if target <= self.epoch {
            return Err(JumpError::Passed {
                current: self.epoch,
            });
        }
        self.jump_target = Some(target);
        self.running = true;
        self.needs_redraw = true;
        Ok(target)
    }

    /// [`jump_to`](Self::jump_to) the generation `generations` from this one.
    pub fn jump_by(&mut self, generations: usize) -> Result<usize, JumpError> {
        let target = self
            .epoch
            .checked_add(generations)
            .ok_or(JumpError::TooFar)?;
        self.jump_to(target)
    }

    /// The generation the jump under way runs to.
    pub fn jump_target(&self) -> Option<usize> {
        self.jump_target.filter(|_| self.running)
    }

    /// Asks the UI for the Jump dialog, as its shortcut does.
    pub fn request_jump(&mut self) {
        self.jump_requested = true;
    }

    /// Whether the Jump dialog was asked for since the last call.
    pub fn take_jump_request(&mut self) -> bool {
        std::mem::take(&mut self.jump_requested)
    }

    pub fn tick_rate_ms(&self) -> u64 {
        self.tick_rate_ms
    }

    pub fn tick_rate_bounds() -> (u64, u64, u64) {
        (
            Self::TICK_RATE_MIN_MS,
            Self::TICK_RATE_MAX_MS,
            Self::TICK_RATE_STEP_MS,
        )
    }

    /// Choosing an interval also leaves max speed.
    pub fn set_tick_rate_ms(&mut self, tick_rate_ms: u64) -> bool {
        let clamped = tick_rate_ms.clamp(Self::TICK_RATE_MIN_MS, Self::TICK_RATE_MAX_MS);
        if self.tick_rate_ms == clamped && !self.max_speed {
            return false;
        }

        self.tick_rate_ms = clamped;
        self.max_speed = false;
        self.needs_redraw = true;
        true
    }

    pub fn is_max_speed(&self) -> bool {
        self.max_speed
    }

    /// Runs generations back to back instead of on the tick interval. The
    /// interval is kept so switching off Max restores the selected fixed rate.
    pub fn set_max_speed(&mut self) -> bool {
        if self.max_speed {
            return false;
        }
        self.max_speed = true;
        self.needs_redraw = true;
        true
    }

    pub fn speed_up(&mut self) -> bool {
        if self.max_speed {
            return false;
        }
        if self.tick_rate_ms <= Self::TICK_RATE_MIN_MS {
            // Past the fastest interval is max speed.
            return self.set_max_speed();
        }

        self.tick_rate_ms = self
            .tick_rate_ms
            .saturating_sub(Self::TICK_RATE_STEP_MS)
            .max(Self::TICK_RATE_MIN_MS);
        self.needs_redraw = true;
        true
    }

    pub fn slow_down(&mut self) -> bool {
        if self.max_speed {
            return self.set_tick_rate_ms(Self::TICK_RATE_MIN_MS);
        }
        if self.tick_rate_ms >= Self::TICK_RATE_MAX_MS {
            return false;
        }

        self.tick_rate_ms =
            (self.tick_rate_ms + Self::TICK_RATE_STEP_MS).min(Self::TICK_RATE_MAX_MS);
        self.needs_redraw = true;
        true
    }

    #[cfg(test)]
    pub fn set_chunk_budget(&mut self, chunks: usize) {
        self.chunk_budget = chunks;
    }

    /// Runs a generation, unless it would take the unbounded board past what a
    /// run may use: then playback pauses and the board stays as it was, until an
    /// edit makes room.
    pub fn increment_epoch(&mut self) {
        if let Some(res) = self.epoch.checked_add(1) {
            let start = self.run_start.is_none().then(|| self.current_snapshot());
            let before = self.board.live_cells_count();
            let fingerprint = self.board.fingerprint();
            if self.board.step_within(self.chunk_budget).is_err() {
                self.running = false;
                self.auto_paused = Some(AutoPause::BoardFull);
                return;
            }
            if start.is_some() {
                self.run_start = start;
            }
            self.epoch = res;
            self.last_step = Some((res, before, self.board.live_cells_count()));
            if self.population_history.is_empty() {
                self.population_history.push_back(before);
            }
            if self.population_history.len() == POPULATION_HISTORY {
                self.population_history.pop_front();
            }
            self.population_history
                .push_back(self.board.live_cells_count());
            let settled = self
                .settle
                .record(fingerprint, self.board.fingerprint(), res);
            if let Some(target) = self.jump_target() {
                if let Some(cycle) = self.settle.settled() {
                    self.skip_cycles(cycle, target);
                }
                if self.epoch == target {
                    self.running = false;
                    self.jump_target = None;
                    self.auto_paused = Some(AutoPause::Arrived(target));
                }
            } else if self.running && self.pause_when_settled && settled.is_some() {
                self.running = false;
                self.auto_paused = settled.map(AutoPause::Settled);
            }
            self.needs_redraw = true;
        }
    }

    /// Moves a jump on a settled board through whole cycles at once, as each
    /// brings back the board it started from. Only the generations left over
    /// are stepped, so a jump far past a settled board arrives at once.
    fn skip_cycles(&mut self, cycle: Settled, target: usize) {
        let skip = (target - self.epoch) / cycle.period * cycle.period;
        if skip == 0 {
            return;
        }
        self.epoch += skip;
        // The latest step's change recurs a whole number of cycles later.
        if let Some((epoch, ..)) = &mut self.last_step {
            *epoch = self.epoch;
        }
    }

    /// Live cells over the latest generations of this run, oldest first.
    pub fn population_history(&self) -> &VecDeque<usize> {
        &self.population_history
    }

    /// Set once the board repeats an earlier generation, until it is edited.
    pub fn settled(&self) -> Option<Settled> {
        self.settle.settled()
    }

    pub fn pause_when_settled(&self) -> bool {
        self.pause_when_settled
    }

    pub fn set_pause_when_settled(&mut self, pause: bool) {
        self.pause_when_settled = pause;
    }

    /// What just paused playback, once, so the UI can explain it.
    pub fn take_auto_paused(&mut self) -> Option<AutoPause> {
        self.auto_paused.take()
    }

    /// How the latest generation changed the live cell count. `None` once an
    /// edit, undo, or counter reset means it no longer describes the board.
    pub fn population_change(&self) -> Option<isize> {
        let (epoch, before, after) = self.last_step?;
        (epoch == self.epoch && after == self.board.live_cells_count())
            .then(|| after as isize - before as isize)
    }

    pub fn reset_epoch(&mut self) {
        if self.epoch != 0 {
            self.epoch = 0;
            self.settle.renumber();
            self.needs_redraw = true;
        }
    }
}
