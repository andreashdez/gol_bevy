//! Generations of a rectangle of the board as an animated GIF, drawn from the
//! cells as [`CellImage`] draws a still: in one colour each for live and dead
//! cells, or by age, as the board shows them with cell age on. Each frame after
//! the first covers only the box of cells whose colour changed, with the cells
//! in it that didn't left transparent, and a generation that changes nothing
//! lengthens the frame before it. Plain Rust, so a recording runs off the main
//! thread.
use crate::{
    cell_image::CellImage,
    sparse_board::{CellRect, PLANE_CHUNKS_MAX},
    universe::Universe,
};
use std::{
    io,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

/// Most cells along either side of a recording: a frame of a busy board this
/// size is 4 MiB to encode, once for every generation.
pub const MAX_GIF_SIDE: usize = 2048;

/// Most generations a recording holds.
pub const MAX_GIF_GENERATIONS: usize = 1000;

/// Cells are drawn as squares of as many pixels as keep the longer side within
/// this, smaller than a still image, as every frame is drawn at this size.
const GIF_SIDE: usize = 1024;

/// Most pixels across a cell, however small the area.
const MAX_SCALE: usize = 8;

/// How long a generation shows at maximum speed. GIF delays count hundredths
/// of a second, and browsers slow any delay under 2 to 10, so 50 generations a
/// second is the fastest a GIF plays everywhere.
pub const FASTEST_MS_PER_GENERATION: u64 = 20;

/// How many generations to record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GifLength {
    /// This many, the current one first.
    Generations(usize),
    /// From the current generation until the board comes back to it, so the GIF
    /// loops without a jump. Up to [`MAX_GIF_GENERATIONS`].
    Period,
}

/// The colours cells are drawn in, dead and live, each by age: the generations
/// since the cell last changed. Without cell age each has a single colour.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GifColors {
    /// Dead colours, then live ones, by age; the last of each holds for any
    /// older cell.
    by_age: [Vec<[u8; 3]>; 2],
    /// The age dead and live cells start the recording at, as cells already on
    /// the board look as if they had been there a while.
    start: [u8; 2],
}

impl GifColors {
    /// One colour for dead cells and one for live ones.
    pub fn plain(dead: [u8; 3], live: [u8; 3]) -> Self {
        Self {
            by_age: [vec![dead], vec![live]],
            start: [0, 0],
        }
    }

    /// Colours by age, from 0 for a cell that just changed. Dead cells start
    /// the recording at their last colour, and live ones at `live_start`.
    pub fn by_age(dead: Vec<[u8; 3]>, live: Vec<[u8; 3]>, live_start: u8) -> Self {
        assert!(!dead.is_empty() && !live.is_empty() && dead.len() + live.len() < 256);
        let dead_start = u8::try_from(dead.len() - 1).unwrap_or(u8::MAX);
        Self {
            by_age: [dead, live],
            start: [dead_start, live_start],
        }
    }

    /// Whether a cell's colour changes as it ages.
    fn ageing(&self) -> bool {
        self.by_age.iter().any(|colors| colors.len() > 1)
    }

    /// Generations after which no cell's colour changes any more.
    fn settles_after(&self) -> usize {
        self.by_age.iter().map(Vec::len).max().unwrap_or(1) - 1
    }

    /// The palette index of a cell's colour.
    fn index(&self, alive: bool, age: u8) -> u8 {
        let [dead, live] = &self.by_age;
        let (first, colors) = if alive { (dead.len(), live) } else { (0, dead) };
        (first + usize::from(age).min(colors.len() - 1)) as u8
    }

    /// The index of cells a frame leaves as the frame before drew them.
    fn unchanged(&self) -> u8 {
        (self.by_age[0].len() + self.by_age[1].len()) as u8
    }

    /// Every colour, then the unchanged cells' one, which is never shown.
    fn palette(&self) -> Vec<u8> {
        let [dead, live] = &self.by_age;
        dead.iter()
            .chain(live)
            .chain(&dead[..1])
            .flatten()
            .copied()
            .collect()
    }
}

/// Where a recording draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GifArea {
    /// This rectangle, whatever passes through it.
    Fixed(CellRect),
    /// The box around the live cells over the whole recording, found by
    /// stepping a copy of the board through it first, so a growing pattern
    /// fits from the first frame to the last.
    Run,
}

/// What a recording starts from: a copy of the board, stepped on its own so
/// the board itself plays on, and the area to draw.
#[derive(Debug, Clone)]
pub struct GifRequest {
    pub(crate) board: Universe,
    pub(crate) area: GifArea,
    pub(crate) length: GifLength,
    pub(crate) ms_per_generation: u64,
}

/// A finished recording.
#[derive(Debug)]
pub struct GifRecording {
    pub file: Vec<u8>,
    /// Generations drawn: a period, or fewer than asked for if an unbounded
    /// board reached its memory limit first.
    pub generations: usize,
    /// Width and height in cells.
    pub size: (usize, usize),
    /// Pixels across each cell.
    pub scale: usize,
}

/// How far a recording running on another thread has got, and a way to stop it.
#[derive(Debug, Default)]
pub struct GifProgress {
    generations: AtomicUsize,
    measuring: AtomicBool,
    cancelled: AtomicBool,
}

impl GifProgress {
    /// Generations measured, or stepped through or drawn, so far.
    pub fn generations(&self) -> usize {
        self.generations.load(Ordering::Relaxed)
    }

    /// Whether the recording is still finding the area the run fills.
    pub fn measuring(&self) -> bool {
        self.measuring.load(Ordering::Relaxed)
    }

    /// Stops the recording before its next generation.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }

    pub fn cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Relaxed)
    }
}

impl GifRequest {
    pub fn length(&self) -> GifLength {
        self.length
    }

    /// Draws the current generation and the ones after it as a GIF that loops
    /// forever, reporting to `progress` and stopping if it's cancelled. Fails
    /// for a period if the board doesn't come back to the current generation
    /// in time.
    pub fn record(self, colors: GifColors, progress: &GifProgress) -> io::Result<GifRecording> {
        let mut recorder = self.recorder(colors)?;
        loop {
            if progress.cancelled() {
                return Err(io::Error::new(
                    io::ErrorKind::Interrupted,
                    "recording cancelled",
                ));
            }
            let over = recorder.step()?;
            progress
                .generations
                .store(recorder.progress(), Ordering::Relaxed);
            progress
                .measuring
                .store(recorder.measuring(), Ordering::Relaxed);
            if over {
                break;
            }
        }
        recorder.finish()
    }

    /// A recording to draw a generation at a time, as [`record`](Self::record)
    /// does all at once.
    pub fn recorder(self, colors: GifColors) -> io::Result<GifRecorder> {
        let stage = match self.area {
            GifArea::Fixed(rect) => Stage::Recording(Box::new(Recording::new(self, rect, colors)?)),
            GifArea::Run => Stage::Measuring(Box::new(Measuring {
                scout: self.board.clone(),
                start: self.board.fingerprint(),
                bounds: live_bounds(&self.board),
                stepped: 0,
                request: self,
                colors,
            })),
        };
        Ok(GifRecorder { stage: Some(stage) })
    }
}

/// A recording under way. Drawn a generation at a time, it can run in short
/// slices, as it does between frames in the web version, which has one thread.
pub struct GifRecorder {
    /// Only ever empty while moving from measuring to recording.
    stage: Option<Stage>,
}

enum Stage {
    Measuring(Box<Measuring>),
    Recording(Box<Recording>),
}

/// Stepping a copy of the board through the recording, to find the box its
/// live cells fill, before recording from the start over that box.
struct Measuring {
    request: GifRequest,
    colors: GifColors,
    scout: Universe,
    /// The board's fingerprint at the start, which a period comes back to.
    start: u64,
    /// The corners of the box around every live cell so far.
    bounds: Option<((i64, i64), (i64, i64))>,
    stepped: usize,
}

impl Measuring {
    /// Steps one generation further, and returns the box once the run is
    /// measured.
    fn step(&mut self) -> io::Result<Option<CellRect>> {
        let done = match self.request.length {
            GifLength::Generations(generations) => self.stepped + 1 >= generations,
            GifLength::Period => false,
        };
        if !done {
            let full = self.scout.step_within(PLANE_CHUNKS_MAX).is_err();
            self.stepped += 1;
            match self.request.length {
                GifLength::Period if full => {
                    return Err(io::Error::other(
                        "the board reached its memory limit before it came back round",
                    ));
                }
                GifLength::Period if self.scout.fingerprint() == self.start => {}
                GifLength::Period if self.stepped == MAX_GIF_GENERATIONS => {
                    return Err(no_period());
                }
                // The recording stops where the board fills up, too.
                GifLength::Generations(_) if full => {}
                _ => {
                    self.include(live_bounds(&self.scout))?;
                    return Ok(None);
                }
            }
        }
        let (min, max) = self
            .bounds
            .ok_or_else(|| io::Error::other("the board is empty, so there is nothing to record"))?;
        let rect = CellRect::spanning(min, max);
        fits(rect)?;
        Ok(Some(rect))
    }

    /// Widens the box to take in `bounds`, failing once it can't be drawn.
    fn include(&mut self, bounds: Option<((i64, i64), (i64, i64))>) -> io::Result<()> {
        let Some((low, high)) = bounds else {
            return Ok(());
        };
        let (min, max) = self.bounds.map_or((low, high), |(min, max)| {
            (
                (min.0.min(low.0), min.1.min(low.1)),
                (max.0.max(high.0), max.1.max(high.1)),
            )
        });
        self.bounds = Some((min, max));
        fits(CellRect::spanning(min, max))
    }
}

/// Fails for a box too large to record.
fn fits(rect: CellRect) -> io::Result<()> {
    if rect.width > MAX_GIF_SIDE || rect.height > MAX_GIF_SIDE {
        return Err(io::Error::other(format!(
            "the pattern grows past {MAX_GIF_SIDE} cells on a side · select an area to record \
             part of it"
        )));
    }
    Ok(())
}

/// The corners of the box around the live cells: on a finite board, the board.
fn live_bounds(board: &Universe) -> Option<((i64, i64), (i64, i64))> {
    match board {
        Universe::Finite(board) => {
            let (cols, rows) = board.size();
            Some(((0, 0), (cols as i64 - 1, rows as i64 - 1)))
        }
        Universe::Unbounded(plane) => plane.live_bounds(),
    }
}

fn no_period() -> io::Error {
    io::Error::other(format!(
        "the board doesn't come back to this generation within {MAX_GIF_GENERATIONS} \
         generations · run it until the status bar shows its period"
    ))
}

impl GifRecorder {
    /// Generations measured, or stepped through or drawn, so far.
    pub fn progress(&self) -> usize {
        match &self.stage {
            Some(Stage::Measuring(measuring)) => measuring.stepped,
            Some(Stage::Recording(recording)) => recording.progress(),
            None => 0,
        }
    }

    /// Whether the recording is still finding the area the run fills.
    pub fn measuring(&self) -> bool {
        matches!(self.stage, Some(Stage::Measuring(_)))
    }

    /// Takes the recording a generation further, and returns true once it's
    /// over.
    pub fn step(&mut self) -> io::Result<bool> {
        match self.stage.as_mut().expect("a stage") {
            Stage::Recording(recording) => recording.step(),
            Stage::Measuring(measuring) => {
                let Some(rect) = measuring.step()? else {
                    return Ok(false);
                };
                let Some(Stage::Measuring(measuring)) = self.stage.take() else {
                    unreachable!("measuring");
                };
                let Measuring {
                    request, colors, ..
                } = *measuring;
                self.stage = Some(Stage::Recording(Box::new(Recording::new(
                    request, rect, colors,
                )?)));
                Ok(false)
            }
        }
    }

    /// The finished GIF, once [`step`](Self::step) has returned true.
    pub fn finish(self) -> io::Result<GifRecording> {
        match self.stage {
            Some(Stage::Recording(recording)) => recording.finish(),
            _ => unreachable!("finished before recording"),
        }
    }
}

/// Drawing the recording over a known area.
struct Recording {
    board: Universe,
    rect: CellRect,
    ms_per_generation: u64,
    colors: GifColors,
    phase: Phase,
    /// The board's fingerprint at the start, which it comes back to once round
    /// a period.
    start: u64,
    scale: usize,
    encoder: gif::Encoder<Vec<u8>>,
    /// The cells of the current generation, and how many generations since
    /// each last changed, top row first.
    cells: Option<CellImage>,
    ages: Vec<u8>,
    /// Each cell's colour as the frames so far show it.
    shown: Vec<u8>,
    /// The latest frame, written once the next change shows how long it lasts.
    pending: Option<gif::Frame<'static>>,
    clock: Clock,
    /// Generations stepped through before drawing, and drawn.
    stepped: usize,
    drawn: usize,
    /// Why the recording stopped, once it has.
    end: Option<End>,
}

impl Recording {
    fn new(request: GifRequest, rect: CellRect, colors: GifColors) -> io::Result<Self> {
        let (width, height) = (rect.width, rect.height);
        let scale = (GIF_SIDE / width.max(height)).clamp(1, MAX_SCALE);
        let pixels = |cells: usize| u16::try_from(cells * scale).map_err(io::Error::other);
        let mut encoder = gif::Encoder::new(
            Vec::new(),
            pixels(width)?,
            pixels(height)?,
            &colors.palette(),
        )
        .map_err(io::Error::other)?;
        encoder
            .set_repeat(gif::Repeat::Infinite)
            .map_err(io::Error::other)?;
        let phase = match request.length {
            GifLength::Generations(generations) => Phase::Drawing { limit: generations },
            GifLength::Period => Phase::Searching,
        };
        Ok(Self {
            // Generations of one run match exactly when their fingerprints do.
            start: request.board.fingerprint(),
            board: request.board,
            rect,
            ms_per_generation: request.ms_per_generation,
            colors,
            phase,
            scale,
            encoder,
            cells: None,
            ages: vec![0; width * height],
            shown: Vec::new(),
            pending: None,
            clock: Clock::default(),
            stepped: 0,
            drawn: 0,
            end: None,
        })
    }
}

#[derive(Clone, Copy)]
enum Phase {
    /// Stepping until the board comes back to the start, to learn the period.
    Searching,
    /// Stepping round the period again, `remaining` generations more, until
    /// every cell's colour is the one it has each time round.
    WarmingUp { period: usize, remaining: usize },
    /// Drawing `limit` generations.
    Drawing { limit: usize },
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
    Drawn,
    /// The board didn't come back round within the most generations a GIF holds.
    NoPeriod,
    /// The unbounded board reached its memory limit.
    Full,
}

impl Recording {
    fn progress(&self) -> usize {
        self.stepped + self.drawn
    }

    fn step(&mut self) -> io::Result<bool> {
        if self.end.is_some() {
            return Ok(true);
        }
        match self.phase {
            Phase::Searching => {
                let first = self.cells.is_none();
                if self.advance() && !first {
                    self.stepped += 1;
                    if self.board.fingerprint() == self.start {
                        // Round the period as many times as it takes every
                        // colour to settle into the loop.
                        let period = self.stepped;
                        let laps = self.colors.settles_after().div_ceil(period).max(1);
                        self.phase = match (laps - 1) * period {
                            0 => Phase::Drawing { limit: period },
                            remaining => Phase::WarmingUp { period, remaining },
                        };
                    } else if self.stepped == MAX_GIF_GENERATIONS {
                        self.end = Some(End::NoPeriod);
                    }
                }
            }
            Phase::WarmingUp { period, remaining } => {
                if self.advance() {
                    self.stepped += 1;
                    self.phase = match remaining - 1 {
                        0 => Phase::Drawing { limit: period },
                        remaining => Phase::WarmingUp { period, remaining },
                    };
                }
            }
            Phase::Drawing { limit } => {
                if self.drawn == limit {
                    self.end = Some(End::Drawn);
                } else if (self.drawn == 0 && self.cells.is_some()) || self.advance() {
                    self.draw()?;
                }
            }
        }
        Ok(self.end.is_some())
    }

    /// Moves to the next generation, keeping the cells of the area and their
    /// ages; the first call takes the board as it is. False, and over, if the
    /// board can't step any further.
    fn advance(&mut self) -> bool {
        if self.cells.is_some() && self.board.step_within(PLANE_CHUNKS_MAX).is_err() {
            self.end = Some(End::Full);
            return false;
        }
        let now = CellImage::capture(&self.board, self.rect);
        let width = self.rect.width;
        match &self.cells {
            None => {
                for (index, age) in self.ages.iter_mut().enumerate() {
                    let alive = now.is_alive(index % width, index / width);
                    *age = self.colors.start[usize::from(alive)];
                }
            }
            Some(before) if self.colors.ageing() => {
                for (index, age) in self.ages.iter_mut().enumerate() {
                    let (x, y) = (index % width, index / width);
                    *age = if now.is_alive(x, y) == before.is_alive(x, y) {
                        age.saturating_add(1)
                    } else {
                        0
                    };
                }
            }
            Some(_) => {}
        }
        self.cells = Some(now);
        true
    }

    /// Draws the current generation: the whole area first, then the box of
    /// cells whose colour changed.
    fn draw(&mut self) -> io::Result<()> {
        let cells = self.cells.as_ref().expect("a generation to draw");
        let width = self.rect.width;
        let colors: Vec<u8> = (self.ages.iter().enumerate())
            .map(|(index, &age)| {
                let alive = cells.is_alive(index % width, index / width);
                self.colors.index(alive, age)
            })
            .collect();
        let (area, shown) = if self.drawn == 0 {
            (Some((0, 0, width, self.rect.height)), None)
        } else {
            (
                changed_box(&colors, &self.shown, width),
                Some(&self.shown[..]),
            )
        };
        if let Some(area) = area {
            let frame = draw(
                &colors,
                shown,
                width,
                area,
                self.scale,
                self.colors.unchanged(),
            );
            if let Some(mut done) = self.pending.replace(frame) {
                done.delay = self.clock.lap();
                self.encoder.write_frame(&done).map_err(io::Error::other)?;
            }
        }
        self.shown = colors;
        self.clock.run(self.ms_per_generation);
        self.drawn += 1;
        Ok(())
    }

    fn finish(mut self) -> io::Result<GifRecording> {
        debug_assert!(self.end.is_some(), "finished before the last generation");
        let looking = matches!(self.phase, Phase::Searching | Phase::WarmingUp { .. });
        match self.end {
            Some(End::Full) if looking => {
                return Err(io::Error::other(
                    "the board reached its memory limit before it came back round",
                ));
            }
            Some(End::NoPeriod) => return Err(no_period()),
            _ => {}
        }
        if let Some(mut last) = self.pending.take() {
            last.delay = self.clock.lap();
            self.encoder.write_frame(&last).map_err(io::Error::other)?;
        }
        Ok(GifRecording {
            file: self.encoder.into_inner().map_err(io::Error::other)?,
            generations: self.drawn,
            size: (self.rect.width, self.rect.height),
            scale: self.scale,
        })
    }
}

/// The smallest box, as `(left, top, width, height)`, holding every cell of
/// `now` that differs from `before`, both in rows `width` cells long.
fn changed_box(now: &[u8], before: &[u8], width: usize) -> Option<(usize, usize, usize, usize)> {
    let (mut left, mut right, mut rows) = (usize::MAX, 0, None);
    for (y, (row, was)) in now
        .chunks_exact(width)
        .zip(before.chunks_exact(width))
        .enumerate()
    {
        let differs = |(a, b): (&u8, &u8)| a != b;
        let Some(first) = row.iter().zip(was).position(differs) else {
            continue;
        };
        let last = row.iter().zip(was).rposition(differs).unwrap_or(first);
        (left, right) = (left.min(first), right.max(last));
        rows = Some(rows.map_or((y, y), |(top, _)| (top, y)));
    }
    let (top, bottom) = rows?;
    Some((left, top, right - left + 1, bottom - top + 1))
}

/// Time in milliseconds, handed out to frames in hundredths of a second. Each
/// frame ends on the hundredth nearest its true end, so rounding never adds up
/// over a recording: generations of 25 ms alternate frames of 3 and 2.
#[derive(Default)]
struct Clock {
    /// When the frame being timed started, and how long it has run.
    start: u64,
    running: u64,
}

impl Clock {
    fn run(&mut self, ms: u64) {
        self.running += ms;
    }

    /// The running frame's delay, starting the next one.
    fn lap(&mut self) -> u16 {
        let hundredths = |ms: u64| (ms + 5) / 10;
        let end = self.start + self.running;
        let delay = hundredths(end) - hundredths(self.start);
        (self.start, self.running) = (end, 0);
        u16::try_from(delay).unwrap_or(u16::MAX)
    }
}

/// A frame of the `area` of `colors`, the cells' palette indices in rows
/// `width` long, each cell a square `scale` pixels across. Over the `shown`
/// colours of the frames before, the cells that kept theirs are `unchanged`,
/// the transparent index.
fn draw(
    colors: &[u8],
    shown: Option<&[u8]>,
    width: usize,
    (left, top, area_width, area_height): (usize, usize, usize, usize),
    scale: usize,
    unchanged: u8,
) -> gif::Frame<'static> {
    let row_pixels = area_width * scale;
    let mut buffer = Vec::with_capacity(row_pixels * area_height * scale);
    for y in top..top + area_height {
        let start = buffer.len();
        for index in y * width + left..y * width + left + area_width {
            let color = match shown {
                Some(shown) if shown[index] == colors[index] => unchanged,
                _ => colors[index],
            };
            buffer.extend(std::iter::repeat_n(color, scale));
        }
        for _ in 1..scale {
            buffer.extend_from_within(start..start + row_pixels);
        }
    }
    // Within the whole image, whose sides were checked to fit.
    let pixels = |cells: usize| (cells * scale) as u16;
    gif::Frame {
        left: pixels(left),
        top: pixels(top),
        width: pixels(area_width),
        height: pixels(area_height),
        buffer: buffer.into(),
        dispose: gif::DisposalMethod::Keep,
        transparent: shown.map(|_| unchanged),
        ..gif::Frame::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, GifArea, GifColors, GifLength, GifProgress, GifRequest, MAX_GIF_SIDE};
    use crate::{board::Board, sparse_board::CellRect, universe::Universe};

    const COLORS: [[u8; 3]; 2] = [[10, 20, 30], [200, 210, 220]];

    fn plain() -> GifColors {
        GifColors::plain(COLORS[0], COLORS[1])
    }

    fn request(board: Universe, rect: CellRect, length: GifLength, ms: u64) -> GifRequest {
        GifRequest {
            board,
            area: GifArea::Fixed(rect),
            length,
            ms_per_generation: ms,
        }
    }

    /// A glider and a blinker on a 20 × 12 board that wraps, and all of it.
    fn glider_and_blinker() -> (Universe, CellRect) {
        let mut board = Universe::Finite(Board::new(20, 12));
        let cells = [
            (1, 10),
            (2, 9),
            (0, 8),
            (1, 8),
            (2, 8),
            (15, 3),
            (16, 3),
            (17, 3),
        ];
        for (x, y) in cells {
            board.populate(x, y);
        }
        let rect = CellRect {
            left: 0,
            bottom: 0,
            width: 20,
            height: 12,
        };
        (board, rect)
    }

    /// Each frame's delay and its pixels as RGBA once composed over the ones
    /// before, as a viewer shows them.
    fn play(file: &[u8]) -> Vec<(u16, Vec<u8>)> {
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(file).unwrap();
        let (width, height) = (decoder.width() as usize, decoder.height() as usize);
        let mut screen = vec![0u8; width * height * 4];
        let mut frames = Vec::new();
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            assert_eq!(frame.dispose, gif::DisposalMethod::Keep);
            let (left, top) = (frame.left as usize, frame.top as usize);
            for (index, pixel) in frame.buffer.as_chunks::<4>().0.iter().enumerate() {
                let (x, y) = (
                    left + index % frame.width as usize,
                    top + index / frame.width as usize,
                );
                if pixel[3] != 0 {
                    screen[(y * width + x) * 4..][..4].copy_from_slice(pixel);
                }
            }
            frames.push((frame.delay, screen.clone()));
        }
        frames
    }

    /// The RGBA pixels of `board`'s `rect` at `scale`, top row first.
    fn expected(board: &Universe, rect: CellRect, scale: usize) -> Vec<u8> {
        let mut pixels = Vec::new();
        for y in (0..rect.height * scale).rev() {
            for x in 0..rect.width * scale {
                let (x, y) = (
                    rect.left + (x / scale) as i64,
                    rect.bottom + (y / scale) as i64,
                );
                pixels.extend(COLORS[usize::from(board.is_alive(x, y))]);
                pixels.push(255);
            }
        }
        pixels
    }

    /// Colours by age with a shade of their own for every age, so a frame
    /// shows each cell's age: dead cells settle after 3 generations, and live
    /// ones after 5, from a start at 2.
    fn aged() -> GifColors {
        let shades = |count: u8, color: fn(u8) -> [u8; 3]| (0..count).map(color).collect();
        GifColors::by_age(
            shades(4, |age| [10 + age, 0, 0]),
            shades(6, |age| [0, 100 + age, 0]),
            2,
        )
    }

    /// The RGBA pixels of every generation of `rect` at `scale`, from the
    /// start, with each cell's age kept one cell at a time as the board keeps
    /// them: back to 0 when it changes, and up by one when it doesn't.
    fn aged_frames(
        mut board: Universe,
        rect: CellRect,
        scale: usize,
        generations: usize,
    ) -> Vec<Vec<u8>> {
        let cells: Vec<(i64, i64)> = (0..rect.height)
            .flat_map(|y| (0..rect.width).map(move |x| (x, y)))
            .map(|(x, y)| {
                (
                    rect.left + x as i64,
                    rect.bottom + (rect.height - 1 - y) as i64,
                )
            })
            .collect();
        let mut ages: Vec<u8> = cells
            .iter()
            .map(|&(x, y)| if board.is_alive(x, y) { 2 } else { 3 })
            .collect();
        let mut frames = Vec::new();
        for _ in 0..generations {
            let mut pixels = Vec::new();
            for y in 0..rect.height * scale {
                for x in 0..rect.width * scale {
                    let index = y / scale * rect.width + x / scale;
                    let (cell, age) = (cells[index], ages[index]);
                    pixels.extend(if board.is_alive(cell.0, cell.1) {
                        [0, 100 + age.min(5), 0]
                    } else {
                        [10 + age.min(3), 0, 0]
                    });
                    pixels.push(255);
                }
            }
            frames.push(pixels);
            let before: Vec<bool> = cells.iter().map(|&(x, y)| board.is_alive(x, y)).collect();
            board.step_within(usize::MAX).unwrap();
            for ((age, &(x, y)), was) in ages.iter_mut().zip(&cells).zip(before) {
                *age = if board.is_alive(x, y) == was {
                    age.saturating_add(1)
                } else {
                    0
                };
            }
        }
        frames
    }

    /// A blinker beside a block, on an 8 × 8 board, and all of it.
    fn blinker_and_block() -> (Universe, CellRect) {
        let mut board = Universe::Finite(Board::new(8, 8));
        for (x, y) in [(1, 5), (2, 5), (3, 5), (5, 1), (6, 1), (5, 2), (6, 2)] {
            board.populate(x, y);
        }
        let rect = CellRect {
            left: 0,
            bottom: 0,
            width: 8,
            height: 8,
        };
        (board, rect)
    }

    #[test]
    fn cells_take_the_colour_of_their_age_from_settled_colours_at_the_start() {
        let (board, rect) = blinker_and_block();
        let recording = request(board.clone(), rect, GifLength::Generations(8), 10)
            .record(aged(), &GifProgress::default())
            .unwrap();
        let frames = play(&recording.file);
        let expected = aged_frames(board, rect, 8, 8);
        assert_eq!(frames.len(), 8);
        for (generation, ((_, pixels), expected)) in frames.iter().zip(&expected).enumerate() {
            assert!(pixels == expected, "generation {generation}");
        }
    }

    #[test]
    fn one_period_goes_round_until_the_colours_loop_too() {
        // The blinker repeats every 2 generations, but colours take up to 5 to
        // settle, so the recording goes round 3 times and draws from generation 6.
        let (board, rect) = blinker_and_block();
        let progress = GifProgress::default();
        let recording = request(board.clone(), rect, GifLength::Period, 10)
            .record(aged(), &progress)
            .unwrap();
        assert_eq!(recording.generations, 2);
        assert_eq!(progress.generations(), 6 + 2);
        let frames = play(&recording.file);
        let expected = aged_frames(board, rect, 8, 9);
        assert_eq!(frames.len(), 2);
        assert!(frames[0].1 == expected[6] && frames[1].1 == expected[7]);
        // Round the loop, the next generation looks like the first.
        assert!(expected[8] == expected[6]);
    }

    #[test]
    fn every_frame_shows_its_generation_as_the_board_steps_it() {
        // Drawn as the glider crosses the wrap.
        let (mut board, rect) = glider_and_blinker();
        let progress = GifProgress::default();
        let recording = request(board.clone(), rect, GifLength::Generations(30), 100)
            .record(plain(), &progress)
            .unwrap();
        assert_eq!(recording.generations, 30);
        assert_eq!(progress.generations(), 30);
        assert_eq!((recording.size, recording.scale), ((20, 12), 8));

        let frames = play(&recording.file);
        assert_eq!(frames.len(), 30);
        for (generation, (delay, pixels)) in frames.iter().enumerate() {
            assert_eq!(*delay, 10, "generation {generation}");
            assert!(
                *pixels == expected(&board, rect, 8),
                "generation {generation}"
            );
            board.step_within(usize::MAX).unwrap();
        }
    }

    #[test]
    fn one_period_ends_on_the_generation_before_the_first_comes_back() {
        // A glider crosses the whole of a 20 × 12 board that wraps every 240
        // generations: 60 cells along its diagonal, 4 generations a cell.
        let (mut board, rect) = glider_and_blinker();
        board.clear();
        for (x, y) in [(1, 10), (2, 9), (0, 8), (1, 8), (2, 8)] {
            board.populate(x, y);
        }
        let recording = request(board.clone(), rect, GifLength::Period, 50)
            .record(plain(), &GifProgress::default())
            .unwrap();
        assert_eq!(recording.generations, 240);
        let frames = play(&recording.file);
        assert_eq!(frames.len(), 240);
        let first = expected(&board, rect, 8);
        for _ in 0..239 {
            board.step_within(usize::MAX).unwrap();
        }
        assert!(frames.last().unwrap().1 == expected(&board, rect, 8));
        board.step_within(usize::MAX).unwrap();
        assert!(expected(&board, rect, 8) == first);
    }

    #[test]
    fn a_board_that_never_comes_back_has_no_period_to_record() {
        let mut plane = Universe::Unbounded(Default::default());
        for (x, y) in [(1, 2), (2, 1), (0, 0), (1, 0), (2, 0)] {
            plane.populate(x, y);
        }
        let rect = CellRect {
            left: 0,
            bottom: 0,
            width: 3,
            height: 3,
        };
        let progress = GifProgress::default();
        let error = request(plane, rect, GifLength::Period, 20)
            .record(plain(), &progress)
            .unwrap_err();
        assert!(
            error.to_string().contains("within 1000 generations"),
            "{error}"
        );
        assert_eq!(progress.generations(), 1000);
    }

    #[test]
    fn a_recording_cancelled_from_another_thread_stops_part_way() {
        // A busy 1024 × 1024 board takes a while a generation, so the recording
        // is still under way when the cancel comes.
        let mut board = Universe::Finite(Board::new(1024, 1024));
        board.finite_mut().unwrap().populate_random();
        let rect = CellRect {
            left: 0,
            bottom: 0,
            width: 1024,
            height: 1024,
        };
        let progress = GifProgress::default();
        let result = std::thread::scope(|scope| {
            let recording = scope.spawn(|| {
                request(board, rect, GifLength::Generations(1000), 20).record(plain(), &progress)
            });
            while progress.generations() == 0 {
                std::thread::yield_now();
            }
            progress.cancel();
            recording.join().unwrap()
        });
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Interrupted);
        assert!(progress.generations() < 1000, "{}", progress.generations());
    }

    #[test]
    fn generations_that_change_nothing_lengthen_the_frame_before() {
        // A block never changes: one frame for the whole recording.
        let mut board = Universe::Finite(Board::new(8, 8));
        for (x, y) in [(3, 3), (4, 3), (3, 4), (4, 4)] {
            board.populate(x, y);
        }
        let rect = CellRect {
            left: 0,
            bottom: 0,
            width: 8,
            height: 8,
        };
        let recording = request(board, rect, GifLength::Generations(7), 250)
            .record(plain(), &GifProgress::default())
            .unwrap();
        let frames = play(&recording.file);
        assert_eq!(
            frames.iter().map(|frame| frame.0).collect::<Vec<_>>(),
            [175]
        );
    }

    #[test]
    fn delays_round_to_hundredths_without_drifting() {
        let mut clock = Clock::default();
        let laps: Vec<_> = (0..4)
            .map(|_| {
                clock.run(25);
                clock.lap()
            })
            .collect();
        assert_eq!(laps, [3, 2, 3, 2]);
        clock.run(20);
        assert_eq!(clock.lap(), 2);
    }

    #[test]
    fn an_unbounded_board_draws_what_passes_through_its_area() {
        let mut plane = Universe::Unbounded(Default::default());
        for (x, y) in [(1, 2), (2, 1), (0, 0), (1, 0), (2, 0)] {
            plane.populate(x - 100, y - 100);
        }
        let rect = CellRect {
            left: -102,
            bottom: -102,
            width: MAX_GIF_SIDE,
            height: 6,
        };
        let recording = request(plane, rect, GifLength::Generations(12), 40)
            .record(plain(), &GifProgress::default())
            .unwrap();
        assert_eq!(recording.generations, 12);
        assert_eq!(recording.scale, 1);
        // The glider heads down and right, out through the area's bottom.
        let frames = play(&recording.file);
        let live = |pixels: &Vec<u8>| {
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| pixel[..3] == COLORS[1])
                .count()
        };
        assert_eq!(live(&frames[0].1), 5);
        assert!(live(&frames.last().unwrap().1) < 5);
    }

    fn fitting_the_run(board: Universe, length: GifLength) -> std::io::Result<super::GifRecording> {
        GifRequest {
            board,
            area: GifArea::Run,
            length,
            ms_per_generation: 40,
        }
        .record(plain(), &GifProgress::default())
    }

    #[test]
    fn fitting_the_run_keeps_a_moving_pattern_whole_in_every_frame() {
        // In 40 generations a glider moves 10 cells each way, so the run fills
        // 13 × 13 cells, though the glider only fills 3 × 3 at the start.
        let mut plane = Universe::Unbounded(Default::default());
        for (x, y) in [(1, 2), (2, 1), (0, 0), (1, 0), (2, 0)] {
            plane.populate(x, y);
        }
        let recording = fitting_the_run(plane, GifLength::Generations(41)).unwrap();
        assert_eq!((recording.size, recording.generations), ((13, 13), 41));
        let frames = play(&recording.file);
        let live = |pixels: &Vec<u8>| {
            let cell = recording.scale * recording.scale;
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| pixel[..3] == COLORS[1])
                .count()
                / cell
        };
        assert!(frames.iter().all(|(_, pixels)| live(pixels) == 5));
    }

    #[test]
    fn fitting_the_run_covers_a_period_and_refuses_what_wont_fit() {
        // A blinker turns between a row and a column, so its period fills 3 × 3.
        let mut plane = Universe::Unbounded(Default::default());
        for x in 0..3 {
            plane.populate(x, 0);
        }
        let recording = fitting_the_run(plane, GifLength::Period).unwrap();
        assert_eq!((recording.size, recording.generations), ((3, 3), 2));

        // Two blocks further apart than a GIF can be wide.
        let mut plane = Universe::Unbounded(Default::default());
        for x in [0, 3000] {
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                plane.populate(x + dx, dy);
            }
        }
        let error = fitting_the_run(plane, GifLength::Generations(1)).unwrap_err();
        assert!(error.to_string().contains("past 2048 cells"), "{error}");
    }
}
