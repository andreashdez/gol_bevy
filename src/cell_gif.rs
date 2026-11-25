//! Generations of a rectangle of the board as an animated GIF, drawn from the
//! cells as [`CellImage`] draws a still. Each frame after the first covers only
//! the box of cells that changed, with the cells in it that didn't left
//! transparent, and a generation that changes nothing lengthens the frame
//! before it. Plain Rust, so a recording runs off the main thread.
use crate::{
    cell_image::CellImage,
    sparse_board::{CellRect, PLANE_CHUNKS_MAX},
    universe::Universe,
};
use std::{
    io,
    sync::atomic::{AtomicUsize, Ordering},
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

/// Palette indices: the two cell colours, and the transparent one for cells
/// that a frame leaves as the frame before drew them.
const LIVE: u8 = 1;
const UNCHANGED: u8 = 2;

/// How many generations to record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GifLength {
    /// This many, the current one first.
    Generations(usize),
    /// From the current generation until the board comes back to it, so the GIF
    /// loops without a jump. Up to [`MAX_GIF_GENERATIONS`].
    Period,
}

/// What a recording starts from: a copy of the board, stepped on its own so
/// the board itself plays on, and the area to draw.
#[derive(Debug, Clone)]
pub struct GifRequest {
    pub(crate) board: Universe,
    pub(crate) rect: CellRect,
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

impl GifRequest {
    pub fn length(&self) -> GifLength {
        self.length
    }

    /// Draws the current generation and the ones after it, dead cells in
    /// `colors[0]` and live ones in `colors[1]`, as a GIF that loops forever.
    /// `progress` counts the generations drawn so far. Fails for a period if
    /// the board doesn't come back to the current generation in time.
    pub fn record(self, colors: [[u8; 3]; 2], progress: &AtomicUsize) -> io::Result<GifRecording> {
        let mut recorder = self.recorder(colors)?;
        while !recorder.step()? {
            progress.store(recorder.drawn(), Ordering::Relaxed);
        }
        recorder.finish()
    }

    /// A recording to draw a generation at a time, as [`record`](Self::record)
    /// does all at once.
    pub fn recorder(self, colors: [[u8; 3]; 2]) -> io::Result<GifRecorder> {
        let (width, height) = (self.rect.width, self.rect.height);
        let scale = (GIF_SIDE / width.max(height)).clamp(1, MAX_SCALE);
        let pixels = |cells: usize| u16::try_from(cells * scale).map_err(io::Error::other);
        let [dead, live] = colors;
        let mut encoder = gif::Encoder::new(
            Vec::new(),
            pixels(width)?,
            pixels(height)?,
            &[dead, live, dead].concat(),
        )
        .map_err(io::Error::other)?;
        encoder
            .set_repeat(gif::Repeat::Infinite)
            .map_err(io::Error::other)?;
        let (limit, find_period) = match self.length {
            GifLength::Generations(generations) => (generations, false),
            GifLength::Period => (MAX_GIF_GENERATIONS, true),
        };
        Ok(GifRecorder {
            // Generations of one run match exactly when their fingerprints do.
            start: self.board.fingerprint(),
            board: self.board,
            rect: self.rect,
            ms_per_generation: self.ms_per_generation,
            limit,
            find_period,
            scale,
            encoder,
            previous: None,
            pending: None,
            clock: Clock::default(),
            drawn: 0,
            end: None,
        })
    }
}

/// A recording under way. Drawn a generation at a time, it can run in short
/// slices, as it does between frames in the web version, which has one thread.
pub struct GifRecorder {
    board: Universe,
    rect: CellRect,
    ms_per_generation: u64,
    /// Most generations to draw, and whether to stop sooner, once the board
    /// comes back to its fingerprint at the `start`.
    limit: usize,
    find_period: bool,
    start: u64,
    scale: usize,
    encoder: gif::Encoder<Vec<u8>>,
    previous: Option<CellImage>,
    /// The latest frame, written once the next change shows how long it lasts.
    pending: Option<gif::Frame<'static>>,
    clock: Clock,
    drawn: usize,
    /// Why the recording stopped, once it has.
    end: Option<End>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
    /// The limit, which for a period means it never came back.
    Limit,
    CameBack,
    /// The unbounded board reached its memory limit.
    Full,
}

impl GifRecorder {
    /// Generations drawn so far.
    pub fn drawn(&self) -> usize {
        self.drawn
    }

    /// Draws the next generation, or returns true once there are no more.
    pub fn step(&mut self) -> io::Result<bool> {
        if self.end.is_some() {
            return Ok(true);
        }
        if self.drawn > 0 {
            // A period is only known once the step after the last frame.
            self.end = if self.drawn == self.limit && !self.find_period {
                Some(End::Limit)
            } else if self.board.step_within(PLANE_CHUNKS_MAX).is_err() {
                Some(End::Full)
            } else if self.find_period && self.board.fingerprint() == self.start {
                Some(End::CameBack)
            } else if self.drawn == self.limit {
                Some(End::Limit)
            } else {
                None
            };
            if self.end.is_some() {
                return Ok(true);
            }
        }
        let image = CellImage::capture(&self.board, self.rect);
        let changed = match &self.previous {
            None => Some((0, 0, self.rect.width, self.rect.height)),
            Some(previous) => image.changed_since(previous),
        };
        if let Some(area) = changed {
            let frame = draw(&image, self.previous.as_ref(), area, self.scale);
            if let Some(mut done) = self.pending.replace(frame) {
                done.delay = self.clock.lap();
                self.encoder.write_frame(&done).map_err(io::Error::other)?;
            }
        }
        self.clock.run(self.ms_per_generation);
        self.previous = Some(image);
        self.drawn += 1;
        Ok(false)
    }

    /// The finished GIF, once [`step`](Self::step) has returned true.
    pub fn finish(mut self) -> io::Result<GifRecording> {
        debug_assert!(self.end.is_some(), "finished before the last generation");
        if self.find_period && self.end != Some(End::CameBack) {
            return Err(io::Error::other(if self.end == Some(End::Full) {
                "the board reached its memory limit before it came back round".to_owned()
            } else {
                format!(
                    "the board doesn't come back to this generation within \
                     {MAX_GIF_GENERATIONS} generations · run it until the status bar \
                     shows its period"
                )
            }));
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

/// A frame of the cells of `area` of `image`, each a square `scale` pixels
/// across. With a `previous` image the frame lies over it, and the cells that
/// haven't changed since are transparent.
fn draw(
    image: &CellImage,
    previous: Option<&CellImage>,
    (left, top, width, height): (usize, usize, usize, usize),
    scale: usize,
) -> gif::Frame<'static> {
    let row_pixels = width * scale;
    let mut buffer = Vec::with_capacity(row_pixels * height * scale);
    for y in top..top + height {
        let start = buffer.len();
        for x in left..left + width {
            let alive = image.is_alive(x, y);
            let index = if previous.is_some_and(|previous| previous.is_alive(x, y) == alive) {
                UNCHANGED
            } else {
                u8::from(alive) * LIVE
            };
            buffer.extend(std::iter::repeat_n(index, scale));
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
        width: pixels(width),
        height: pixels(height),
        buffer: buffer.into(),
        dispose: gif::DisposalMethod::Keep,
        transparent: previous.map(|_| UNCHANGED),
        ..gif::Frame::default()
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, GifLength, GifRequest, MAX_GIF_SIDE};
    use crate::{board::Board, sparse_board::CellRect, universe::Universe};
    use std::sync::atomic::{AtomicUsize, Ordering};

    const COLORS: [[u8; 3]; 2] = [[10, 20, 30], [200, 210, 220]];

    fn request(board: Universe, rect: CellRect, length: GifLength, ms: u64) -> GifRequest {
        GifRequest {
            board,
            rect,
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

    #[test]
    fn every_frame_shows_its_generation_as_the_board_steps_it() {
        // Drawn as the glider crosses the wrap.
        let (mut board, rect) = glider_and_blinker();
        let progress = AtomicUsize::new(0);
        let recording = request(board.clone(), rect, GifLength::Generations(30), 100)
            .record(COLORS, &progress)
            .unwrap();
        assert_eq!(recording.generations, 30);
        assert_eq!(progress.load(Ordering::Relaxed), 30);
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
            .record(COLORS, &AtomicUsize::new(0))
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
        let progress = AtomicUsize::new(0);
        let error = request(plane, rect, GifLength::Period, 20)
            .record(COLORS, &progress)
            .unwrap_err();
        assert!(
            error.to_string().contains("within 1000 generations"),
            "{error}"
        );
        assert_eq!(progress.load(Ordering::Relaxed), 1000);
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
            .record(COLORS, &AtomicUsize::new(0))
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
            .record(COLORS, &AtomicUsize::new(0))
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
}
