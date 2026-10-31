//! The population of recent generations, as a line in the status bar. It is
//! drawn as a white mask, like the icons, so [`ThemedImage`] tints it.
use super::*;
use crate::app::POPULATION_HISTORY;
use bevy::asset::uuid::Uuid;
use std::{collections::VecDeque, marker::PhantomData};

const SIZE: Vec2 = Vec2::new(56.0, 18.0);
const TEXELS_PER_POINT: usize = 2;
const WIDTH: usize = SIZE.x as usize * TEXELS_PER_POINT;
const HEIGHT: usize = SIZE.y as usize * TEXELS_PER_POINT;
/// In texels, so the line is 1.5 points thick.
const STROKE: f32 = 3.0;
/// The vertical scale spans at least this fraction of the peak, so a settled
/// board's few blinkers read as flat instead of as a violent zigzag.
const MIN_SPAN_OF_PEAK: f32 = 1.0 / 16.0;

#[derive(Component)]
pub(super) struct Sparkline;

fn handle() -> Handle<Image> {
    const ID: u128 = 0x3c1e_0b9a_5f2d_4e57_9a61_8d0c_47b2_1000;
    Handle::Uuid(Uuid::from_u128(ID), PhantomData)
}

pub(super) fn node() -> impl Bundle {
    (
        Sparkline,
        ImageNode::new(handle()),
        Node {
            width: px(SIZE.x),
            height: px(SIZE.y),
            flex_shrink: 0.0,
            ..default()
        },
        ThemedImage(Token::Accent),
        Pickable::IGNORE,
    )
}

/// Redraws only when a generation or an edit changed the history.
pub(super) fn sync(
    sim: Res<Simulation>,
    mut images: ResMut<Assets<Image>>,
    mut drawn: Local<Option<(usize, usize)>>,
) {
    let history = sim.app.population_history();
    let current = (sim.app.epoch, history.len());
    if *drawn != Some(current) {
        *drawn = Some(current);
        images
            .insert(handle().id(), image(&coverage(history)))
            .expect("the static sparkline handle is valid");
    }
}

fn image(coverage: &[u8]) -> Image {
    let data = coverage
        .iter()
        .flat_map(|&alpha| [255, 255, 255, alpha])
        .collect();
    Image::new(
        Extent3d {
            width: WIDTH as u32,
            height: HEIGHT as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Alpha for each texel, top row first. The newest generation is at the right
/// edge and each generation takes the same width, so a young run grows from
/// the right rather than stretching.
fn coverage(history: &VecDeque<usize>) -> Vec<u8> {
    let mut alpha = vec![0; WIDTH * HEIGHT];
    let (Some(&low), Some(&high)) = (history.iter().min(), history.iter().max()) else {
        return alpha;
    };
    if history.len() < 2 {
        return alpha;
    }

    let span = ((high - low) as f32)
        .max(high as f32 * MIN_SPAN_OF_PEAK)
        .max(1.0);
    let centre = (high + low) as f32 / 2.0;
    let travel = HEIGHT as f32 - STROKE - 1.0;
    let y = |value: usize| HEIGHT as f32 / 2.0 - (value as f32 - centre) / span * travel;
    let per_generation = (WIDTH - 1) as f32 / (POPULATION_HISTORY - 1) as f32;
    let newest = history.len() - 1;
    let x = |index: usize| (WIDTH - 1) as f32 - (newest - index) as f32 * per_generation + 0.5;

    // The stretch of each column the line passes through, top to bottom.
    let mut columns = vec![(f32::INFINITY, f32::NEG_INFINITY); WIDTH];
    for (index, (&from, &to)) in history.iter().zip(history.iter().skip(1)).enumerate() {
        let (x0, x1) = (x(index), x(index + 1));
        let (y0, y1) = (y(from), y(to));
        let along = |at: f32| y0 + (y1 - y0) * ((at - x0) / (x1 - x0)).clamp(0.0, 1.0);
        let (first, last) = (x0.floor() as usize, x1.ceil() as usize);
        for (column, (top, bottom)) in columns.iter_mut().enumerate().take(last).skip(first) {
            let (a, b) = (along(column as f32), along(column as f32 + 1.0));
            *top = top.min(a.min(b));
            *bottom = bottom.max(a.max(b));
        }
    }

    for (column, &(top, bottom)) in columns.iter().enumerate() {
        if top > bottom {
            continue;
        }
        for row in 0..HEIGHT {
            let centre = row as f32 + 0.5;
            let distance = (top - centre).max(centre - bottom).max(0.0);
            // One texel of falloff anti-aliases the edge, as for the icons.
            let shade = (STROKE / 2.0 + 0.5 - distance).clamp(0.0, 1.0);
            alpha[row * WIDTH + column] = (shade * 255.0).round() as u8;
        }
    }
    alpha
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(values: impl IntoIterator<Item = usize>) -> Vec<u8> {
        coverage(&values.into_iter().collect())
    }

    /// The rows the line covers in a column, as (first, last).
    fn rows_in(alpha: &[u8], column: usize) -> Option<(usize, usize)> {
        let rows: Vec<_> = (0..HEIGHT)
            .filter(|row| alpha[row * WIDTH + column] > 127)
            .collect();
        Some((*rows.first()?, *rows.last()?))
    }

    #[test]
    fn nothing_is_drawn_until_there_is_a_line() {
        assert!(drawn([]).iter().all(|&alpha| alpha == 0));
        assert!(drawn([500]).iter().all(|&alpha| alpha == 0));
    }

    #[test]
    fn growth_rises_to_the_right_edge_and_stays_inside_the_image() {
        let alpha = drawn((0..POPULATION_HISTORY).map(|generation| generation * 10));
        let (first, _) = rows_in(&alpha, WIDTH - 1).unwrap();
        let (_, last) = rows_in(&alpha, 1).unwrap();
        assert!(first < 4, "the newest value is at the top, row {first}");
        assert!(
            last >= HEIGHT - 4,
            "the oldest is at the bottom, row {last}"
        );
        for column in 0..WIDTH {
            assert!(
                rows_in(&alpha, column).is_some(),
                "a gap at column {column}"
            );
        }
    }

    #[test]
    fn a_young_run_grows_from_the_right() {
        let alpha = drawn([100, 400, 200, 300]);
        assert!(rows_in(&alpha, WIDTH - 1).is_some());
        assert!(rows_in(&alpha, WIDTH / 2).is_none());
    }

    #[test]
    fn a_constant_population_is_a_flat_line_through_the_middle() {
        for population in [0, 7, 1_000_000] {
            let alpha = drawn(vec![population; 40]);
            let (first, last) = rows_in(&alpha, WIDTH - 2).unwrap();
            assert!(first <= HEIGHT / 2 && last >= HEIGHT / 2 - 1);
            assert!(last - first <= STROKE as usize + 1);
        }
    }

    #[test]
    fn small_oscillations_stay_small_but_real_swings_fill_the_height() {
        let zigzag = |swing: usize| (0..64).map(move |generation| 1000 + generation % 2 * swing);
        let height = |alpha: &[u8]| {
            let (first, last) = rows_in(alpha, WIDTH - 8).unwrap();
            last - first
        };
        assert!(height(&drawn(zigzag(6))) < HEIGHT / 2);
        assert!(height(&drawn(zigzag(600))) >= HEIGHT - 8);
    }

    #[test]
    fn a_full_history_uses_every_column_of_a_wide_image() {
        // A spike in a single generation is not lost between columns.
        let mut values = vec![100; POPULATION_HISTORY];
        values[POPULATION_HISTORY / 2] = 5000;
        let alpha = drawn(values);
        assert!((0..WIDTH).any(|column| rows_in(&alpha, column).is_some_and(|rows| rows.0 < 4)));
    }
}
