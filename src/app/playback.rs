//! Running generations: playback and its speed, stepping the board, noticing a
//! board that settled or filled up, and the population of the run so far.

use super::App;
use crate::{settle::Settled, universe::Universe};
use std::{collections::VecDeque, fmt};

/// Generations of population kept for the status bar's line.
pub const POPULATION_HISTORY: usize = 256;

/// Why playback stopped by itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutoPause {
    Settled(Settled),
    /// The unbounded board reached its memory budget.
    BoardFull,
}

impl fmt::Display for AutoPause {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Settled(settled) => settled.fmt(f),
            Self::BoardFull => f.write_str("the board reached its memory limit"),
        }
    }
}

impl App {
    const TICK_RATE_MIN_MS: u64 = 25;
    const TICK_RATE_MAX_MS: u64 = 1000;
    const TICK_RATE_STEP_MS: u64 = 25;

    pub fn toggle_running(&mut self) {
        self.running = !self.running;
        self.needs_redraw = true;
    }

    pub fn is_running(&self) -> bool {
        self.running
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

    /// Whether the unbounded board has outgrown what a run may use. Anything that
    /// frees chunks lets playback go on.
    fn board_is_full(&self) -> bool {
        matches!(&self.board, Universe::Unbounded(plane) if plane.chunk_count() >= self.chunk_budget)
    }

    pub fn increment_epoch(&mut self) {
        if self.board_is_full() {
            self.running = false;
            self.auto_paused = Some(AutoPause::BoardFull);
            return;
        }
        if let Some(res) = self.epoch.checked_add(1) {
            if self.run_start.is_none() {
                self.run_start = Some(self.current_snapshot());
            }
            self.epoch = res;
            let before = self.board.live_cells_count();
            let fingerprint = self.board.fingerprint();
            self.board.step();
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
            if self.running && self.pause_when_settled && settled.is_some() {
                self.running = false;
                self.auto_paused = settled.map(AutoPause::Settled);
            }
            self.needs_redraw = true;
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
