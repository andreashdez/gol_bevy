//! A rectangle of the board as a picture: its cells a bit each, top row first
//! as images go, and the PNG file drawn from them. Plain Rust, so an export can
//! be encoded and written off the main thread.
use crate::{chunk::bits, sparse_board::CellRect, universe::Universe};
use std::io::{self, Write};

/// Most cells along either side of an image, which keeps its cells within 32 MiB.
pub const MAX_IMAGE_SIDE: usize = 16384;

/// Cells are drawn as squares of as many pixels as keep the image's longer side
/// within this, so a small pattern comes out large enough to see.
const SCALED_SIDE: usize = 4096;

/// Most pixels across a cell, however small the pattern.
const MAX_SCALE: usize = 8;

/// The live cells of a rectangle, a bit each.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CellImage {
    width: usize,
    height: usize,
    /// Words in each row: cell `x` of a row is bit `x % 64` of its word `x / 64`.
    row_words: usize,
    /// The rows, top row first.
    words: Vec<u64>,
}

impl CellImage {
    /// The cells of `rect`, which is at least a cell and at most
    /// [`MAX_IMAGE_SIDE`] on each side.
    pub fn capture(board: &Universe, rect: CellRect) -> Self {
        assert!(
            (1..=MAX_IMAGE_SIDE).contains(&rect.width)
                && (1..=MAX_IMAGE_SIDE).contains(&rect.height),
            "an image of {} × {} cells",
            rect.width,
            rect.height
        );
        let row_words = rect.width.div_ceil(64);
        let mut words = vec![0; row_words * rect.height];
        let top = rect.bottom + rect.height as i64 - 1;
        board.for_each_live_run_in(rect, |x, y, length| {
            let row = (top - y) as usize * row_words;
            set_run(
                &mut words[row..row + row_words],
                (x - rect.left) as usize,
                length,
            );
        });
        Self {
            width: rect.width,
            height: rect.height,
            row_words,
            words,
        }
    }

    /// Width and height in cells.
    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Whether the cell `x` from the left and `y` from the top is alive.
    pub fn is_alive(&self, x: usize, y: usize) -> bool {
        self.words[y * self.row_words + x / 64] >> (x % 64) & 1 != 0
    }

    /// The smallest box holding every cell that differs from `earlier`, an
    /// image of the same size, as `(left, top, width, height)`; `None` if no
    /// cell does.
    pub fn changed_since(&self, earlier: &Self) -> Option<(usize, usize, usize, usize)> {
        assert_eq!(self.size(), earlier.size());
        // Every row's changes together, to find the leftmost and rightmost.
        let mut columns = vec![0u64; self.row_words];
        let mut rows = None;
        let pairs = self
            .words
            .chunks_exact(self.row_words)
            .zip(earlier.words.chunks_exact(self.row_words));
        for (y, (row, before)) in pairs.enumerate() {
            let mut changed = false;
            for ((column, now), then) in columns.iter_mut().zip(row).zip(before) {
                *column |= now ^ then;
                changed |= now != then;
            }
            if changed {
                rows = Some(rows.map_or((y, y), |(top, _)| (top, y)));
            }
        }
        let (top, bottom) = rows?;
        let first = columns.iter().position(|&word| word != 0)?;
        let last = columns.iter().rposition(|&word| word != 0)?;
        let left = first * 64 + columns[first].trailing_zeros() as usize;
        let right = last * 64 + 63 - columns[last].leading_zeros() as usize;
        Some((left, top, right - left + 1, bottom - top + 1))
    }

    /// Pixels across each cell: as many as keep the longer side within
    /// [`SCALED_SIDE`] pixels, from 1 to [`MAX_SCALE`].
    pub fn scale(&self) -> usize {
        (SCALED_SIDE / self.width.max(self.height)).clamp(1, MAX_SCALE)
    }

    /// A PNG of the cells, each a square `scale` pixels across, dead cells in
    /// `colors[0]` and live ones in `colors[1]`. It takes a bit a pixel, as an
    /// index into those two colours.
    pub fn to_png(&self, scale: usize, colors: [[u8; 3]; 2]) -> io::Result<Vec<u8>> {
        let width = self.width * scale;
        let pixels = |side: usize| u32::try_from(side).map_err(io::Error::other);
        let mut file = Vec::new();
        let mut encoder =
            png::Encoder::new(&mut file, pixels(width)?, pixels(self.height * scale)?);
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_depth(png::BitDepth::One);
        encoder.set_palette(colors.concat());
        let mut writer = encoder.write_header()?;
        let mut stream = writer.stream_writer()?;
        // A pixel row, leftmost pixel in the highest bit of the first byte.
        let mut line = vec![0u8; width.div_ceil(8)];
        for row in self.words.chunks_exact(self.row_words) {
            line.fill(0);
            for (index, &word) in row.iter().enumerate() {
                for bit in bits(word) {
                    let first = (index * 64 + bit) * scale;
                    for pixel in first..first + scale {
                        line[pixel / 8] |= 0x80 >> (pixel % 8);
                    }
                }
            }
            for _ in 0..scale {
                stream.write_all(&line)?;
            }
        }
        stream.finish()?;
        writer.finish()?;
        Ok(file)
    }
}

/// Sets bits `first..first + length` of `row`.
fn set_run(row: &mut [u64], first: usize, length: usize) {
    let (mut at, end) = (first, first + length);
    while at < end {
        let bit = at % 64;
        let count = (64 - bit).min(end - at);
        row[at / 64] |= (!0 >> (64 - count)) << bit;
        at += count;
    }
}

#[cfg(test)]
mod tests {
    use super::{CellImage, MAX_IMAGE_SIDE, set_run};
    use crate::{board::Board, sparse_board::CellRect, universe::Universe};

    const COLORS: [[u8; 3]; 2] = [[10, 20, 30], [200, 210, 220]];

    /// The pixels of a PNG as RGB, row by row from the top.
    fn decode(file: &[u8]) -> (usize, usize, Vec<[u8; 3]>) {
        let mut decoder = png::Decoder::new(std::io::Cursor::new(file));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info().unwrap();
        let mut data = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut data).unwrap();
        assert_eq!(info.color_type, png::ColorType::Rgb);
        let pixels = data[..info.buffer_size()].as_chunks::<3>().0.to_vec();
        (info.width as usize, info.height as usize, pixels)
    }

    #[test]
    fn runs_set_exactly_their_bits_across_words() {
        for (first, length) in [(0, 1), (3, 61), (60, 8), (64, 64), (1, 190), (127, 1)] {
            let mut row = [0u64; 3];
            set_run(&mut row, first, length);
            for bit in 0..192 {
                let set = row[bit / 64] >> (bit % 64) & 1 != 0;
                assert_eq!(
                    set,
                    (first..first + length).contains(&bit),
                    "bit {bit} of {first}+{length}"
                );
            }
        }
    }

    #[test]
    fn a_capture_puts_the_top_row_first_on_either_board() {
        // An L wider than a word, with its foot at the bottom left.
        let cells = [(0, 0), (1, 0), (2, 0), (0, 1), (0, 2), (70, 2)];
        let mut finite = Universe::Finite(Board::new(80, 4));
        let mut plane = Universe::Unbounded(Default::default());
        for &(x, y) in &cells {
            finite.populate(x, y);
            plane.populate(x - 40, y - 7);
        }
        let rect = |left, bottom| CellRect {
            left,
            bottom,
            width: 71,
            height: 3,
        };
        for image in [
            CellImage::capture(&finite, rect(0, 0)),
            CellImage::capture(&plane, rect(-40, -7)),
        ] {
            assert_eq!(image.size(), (71, 3));
            for y in 0..3 {
                for x in 0..71 {
                    let alive = cells.contains(&(x as i64, 2 - y as i64));
                    assert_eq!(image.is_alive(x, y), alive, "({x}, {y})");
                }
            }
        }
    }

    #[test]
    fn the_png_draws_each_cell_as_a_square_of_its_colour() {
        let mut board = Universe::Finite(Board::new(9, 2));
        board.populate(0, 1);
        board.populate(8, 0);
        let image = CellImage::capture(
            &board,
            CellRect {
                left: 0,
                bottom: 0,
                width: 9,
                height: 2,
            },
        );
        for scale in [1, 3] {
            let (width, height, pixels) = decode(&image.to_png(scale, COLORS).unwrap());
            assert_eq!((width, height), (9 * scale, 2 * scale));
            for (index, pixel) in pixels.iter().enumerate() {
                let (x, y) = (index % width / scale, index / width / scale);
                assert_eq!(
                    *pixel,
                    COLORS[usize::from(image.is_alive(x, y))],
                    "pixel {index}"
                );
            }
        }
    }

    #[test]
    fn small_patterns_draw_larger_and_large_boards_a_pixel_a_cell() {
        let image = |width, height| {
            let board = Universe::Unbounded(Default::default());
            CellImage::capture(
                &board,
                CellRect {
                    left: 0,
                    bottom: 0,
                    width,
                    height,
                },
            )
        };
        assert_eq!(image(3, 3).scale(), 8);
        assert_eq!(image(1024, 100).scale(), 4);
        assert_eq!(image(2048, 2048).scale(), 2);
        assert_eq!(image(MAX_IMAGE_SIDE, 1).scale(), 1);
    }
}
