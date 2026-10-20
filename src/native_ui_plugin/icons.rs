//! Toolbar icons, drawn from distance functions at startup. The UI font has no
//! media or gear glyphs, and drawing them keeps the binary free of icon assets.
//! Icons are white masks; [`ThemedImage`] tints them like text.
use super::*;
use bevy::asset::uuid::Uuid;
use std::marker::PhantomData;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Icon {
    Play,
    Pause,
    Step,
    Undo,
    Redo,
    Grid,
    CellAge,
    Palette,
    Gear,
    Help,
    Fit,
    More,
}

impl Icon {
    const ALL: [Icon; 12] = [
        Icon::Play,
        Icon::Pause,
        Icon::Step,
        Icon::Undo,
        Icon::Redo,
        Icon::Grid,
        Icon::CellAge,
        Icon::Palette,
        Icon::Gear,
        Icon::Help,
        Icon::Fit,
        Icon::More,
    ];

    /// Stable handles, so icons can be spawned before or without the asset.
    pub(super) fn handle(self) -> Handle<Image> {
        const BASE: u128 = 0x3c1e_0b9a_5f2d_4e57_9a61_8d0c_47b2_0000;
        Handle::Uuid(Uuid::from_u128(BASE + self as u128), PhantomData)
    }

    /// Signed distance from `p` to the icon's edge, in units of a 24×24 grid
    /// with y pointing down. Negative inside.
    fn distance(self, p: Vec2) -> f32 {
        // Strokes are 2 units wide, with round caps and joins.
        const HALF_STROKE: f32 = 1.0;
        match self {
            Icon::Play => triangle(p, [vec2(8.0, 5.5), vec2(8.0, 18.5), vec2(18.5, 12.0)]) - 1.0,
            Icon::Pause => rounded_box(p, vec2(8.5, 12.0), vec2(2.0, 6.5), 1.0).min(rounded_box(
                p,
                vec2(15.5, 12.0),
                vec2(2.0, 6.5),
                1.0,
            )),
            Icon::Step => (triangle(p, [vec2(6.5, 6.5), vec2(6.5, 17.5), vec2(14.5, 12.0)]) - 1.0)
                .min(rounded_box(p, vec2(17.5, 12.0), vec2(1.3, 6.0), 1.0)),
            // Lucide's undo-2: a chevron, a line, and a half turn back.
            Icon::Undo => undo(p) - HALF_STROKE,
            Icon::Redo => undo(vec2(24.0 - p.x, p.y)) - HALF_STROKE,
            Icon::Grid => grid(p) - HALF_STROKE,
            Icon::CellAge => cell_age(p),
            // Lucide's panel-left: a window with a sidebar.
            Icon::Palette => {
                rounded_box(p, vec2(12.0, 12.0), vec2(9.0, 8.0), 2.0)
                    .abs()
                    .min(segment(p, vec2(9.0, 4.0), vec2(9.0, 20.0)))
                    - HALF_STROKE
            }
            Icon::Gear => gear(p),
            Icon::Help => help(p),
            Icon::Fit => fit(p) - HALF_STROKE,
            // Three dots in a row, for a menu of further controls.
            Icon::More => [5.0, 12.0, 19.0]
                .map(|x| p.distance(vec2(x, 12.0)) - 2.0)
                .into_iter()
                .fold(f32::INFINITY, f32::min),
        }
    }

    /// How opaque the shape is at `p`, so parts of an icon can be fainter.
    fn opacity(self, p: Vec2) -> f32 {
        match self {
            // The trail fades from the newest cell, top right, to the oldest.
            Icon::CellAge => match p.x - p.y {
                d if d > 6.5 => 1.0,
                d if d > -6.5 => 0.65,
                _ => 0.4,
            },
            _ => 1.0,
        }
    }
}

/// A white icon with the given alpha coverage, for tinting.
fn rasterize(icon: Icon, size: u32) -> Image {
    let scale = 24.0 / size as f32;
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let p = (vec2(x as f32, y as f32) + 0.5) * scale;
            // Distance in pixels; one pixel of falloff anti-aliases the edge.
            let coverage = (0.5 - icon.distance(p) / scale).clamp(0.0, 1.0) * icon.opacity(p);
            data.extend_from_slice(&[255, 255, 255, (coverage * 255.0).round() as u8]);
        }
    }
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    )
}

/// Drawn at 2.5× their display size so they stay crisp on HiDPI screens. At
/// 18 px, icon buttons are as tall as buttons with a line of 14 px text.
const RASTER_SIZE: u32 = 45;
pub(super) const DISPLAY_SIZE: f32 = 18.0;

pub(super) fn insert_all(images: &mut Assets<Image>) {
    for icon in Icon::ALL {
        images
            .insert(icon.handle().id(), rasterize(icon, RASTER_SIZE))
            .expect("static icon handle is valid");
    }
}

pub(super) fn node(icon: Icon) -> impl Bundle {
    (
        ImageNode::new(icon.handle()),
        Node {
            width: px(DISPLAY_SIZE),
            height: px(DISPLAY_SIZE),
            ..default()
        },
        ThemedImage(Token::Ink),
        Pickable::IGNORE,
    )
}

fn segment(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let (pa, ba) = (p - a, b - a);
    let t = (pa.dot(ba) / ba.length_squared()).clamp(0.0, 1.0);
    (pa - ba * t).length()
}

fn rounded_box(p: Vec2, center: Vec2, half: Vec2, radius: f32) -> f32 {
    let q = (p - center).abs() - half + radius;
    q.max(Vec2::ZERO).length() + q.x.max(q.y).min(0.0) - radius
}

fn triangle(p: Vec2, [a, b, c]: [Vec2; 3]) -> f32 {
    let edge = segment(p, a, b).min(segment(p, b, c)).min(segment(p, c, a));
    let side = |u: Vec2, v: Vec2| (v - u).perp_dot(p - u);
    let (s0, s1, s2) = (side(a, b), side(b, c), side(c, a));
    let inside = (s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0) || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0);
    if inside { -edge } else { edge }
}

/// Distance to the centre line of the undo arrow.
fn undo(p: Vec2) -> f32 {
    let center = vec2(14.5, 14.5);
    let radius = 5.5;
    // The half turn runs through the right side, from the top to the bottom.
    let arc = if p.x >= center.x {
        ((p - center).length() - radius).abs()
    } else {
        f32::INFINITY
    };
    segment(p, vec2(9.0, 14.0), vec2(4.0, 9.0))
        .min(segment(p, vec2(4.0, 9.0), vec2(9.0, 4.0)))
        .min(segment(p, vec2(4.0, 9.0), vec2(14.5, 9.0)))
        .min(segment(p, vec2(14.5, 20.0), vec2(11.0, 20.0)))
        .min(arc)
}

/// Distance to the centre lines of a 3×3 grid of cells.
fn grid(p: Vec2) -> f32 {
    let (a, b) = (4.0 + 16.0 / 3.0, 20.0 - 16.0 / 3.0);
    rounded_box(p, vec2(12.0, 12.0), vec2(8.0, 8.0), 2.0)
        .abs()
        .min(segment(p, vec2(a, 4.0), vec2(a, 20.0)))
        .min(segment(p, vec2(b, 4.0), vec2(b, 20.0)))
        .min(segment(p, vec2(4.0, a), vec2(20.0, a)))
        .min(segment(p, vec2(4.0, b), vec2(20.0, b)))
}

/// Three cells in a diagonal trail, like a glider leaving faded cells behind.
fn cell_age(p: Vec2) -> f32 {
    [vec2(18.5, 5.5), vec2(12.0, 12.0), vec2(5.5, 18.5)]
        .into_iter()
        .map(|center| rounded_box(p, center, vec2(2.9, 2.9), 0.9))
        .fold(f32::INFINITY, f32::min)
}

/// A question mark: a hook over a short stem, and a dot.
fn help(p: Vec2) -> f32 {
    let center = vec2(12.0, 8.5);
    let radius = 3.5;
    let q = p - center;
    // The hook runs from the left, over the top, to just past the right side.
    let angle = q.y.atan2(q.x);
    let end = center + Vec2::from_angle(std::f32::consts::FRAC_PI_4) * radius;
    // atan2 never goes below -π, so only the far end needs a bound.
    let hook = if angle <= std::f32::consts::FRAC_PI_4 {
        (q.length() - radius).abs()
    } else {
        p.distance(center - vec2(radius, 0.0))
    };
    let stem = hook.min(segment(p, end, vec2(12.0, 13.0))).min(segment(
        p,
        vec2(12.0, 13.0),
        vec2(12.0, 14.5),
    ));
    (stem - 1.0).min(p.distance(vec2(12.0, 18.5)) - 1.4)
}

/// Four corner brackets, as on a camera viewfinder: the whole board in view.
fn fit(p: Vec2) -> f32 {
    // Folding into one quadrant draws all four corners from one bracket.
    let q = vec2(12.0 - (p.x - 12.0).abs(), 12.0 - (p.y - 12.0).abs());
    segment(q, vec2(4.0, 9.0), vec2(4.0, 4.0)).min(segment(q, vec2(4.0, 4.0), vec2(9.0, 4.0)))
}

/// Eight teeth around a wheel with a hole in the middle.
fn gear(p: Vec2) -> f32 {
    let q = p - vec2(12.0, 12.0);
    let sector = std::f32::consts::TAU / 8.0;
    let angle = q.y.atan2(q.x).rem_euclid(sector) - sector / 2.0;
    let local = Vec2::from_angle(angle) * q.length();
    let tooth = rounded_box(local, vec2(8.4, 0.0), vec2(2.1, 1.7), 0.7);
    let wheel = q.length() - 7.0;
    let hole = 2.9 - q.length();
    wheel.min(tooth).max(hole)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icons_are_opaque_inside_and_clear_at_the_edges() {
        let inside = [
            (Icon::Play, vec2(11.0, 12.0)),
            (Icon::Pause, vec2(8.0, 12.0)),
            (Icon::Step, vec2(17.5, 12.0)),
            (Icon::Undo, vec2(10.0, 9.0)),
            (Icon::Redo, vec2(14.0, 9.0)),
            (Icon::Grid, vec2(4.0 + 16.0 / 3.0, 12.0)),
            (Icon::CellAge, vec2(18.5, 5.5)),
            (Icon::Palette, vec2(9.0, 12.0)),
            (Icon::Gear, vec2(12.0, 6.5)),
            (Icon::Help, vec2(12.0, 18.5)),
            (Icon::Fit, vec2(20.0, 6.0)),
            (Icon::More, vec2(19.0, 12.0)),
        ];
        for (icon, point) in inside {
            assert!(icon.distance(point) < -0.5, "{icon:?} at {point}");
            // A margin keeps every shape clear of the image edge.
            for edge in [
                vec2(0.5, 12.0),
                vec2(23.5, 12.0),
                vec2(12.0, 0.5),
                vec2(12.0, 23.5),
            ] {
                assert!(icon.distance(edge) > 0.5, "{icon:?} touches {edge}");
            }
        }
        // The gear has a hole, and redo mirrors undo.
        assert!(Icon::Gear.distance(vec2(12.0, 12.0)) > 0.5);
        assert_eq!(
            Icon::Undo.distance(vec2(6.0, 12.0)),
            Icon::Redo.distance(vec2(18.0, 12.0))
        );
        let image = rasterize(Icon::Pause, RASTER_SIZE);
        let alpha =
            |x: u32, y: u32| image.data.as_ref().unwrap()[((y * RASTER_SIZE + x) * 4 + 3) as usize];
        assert_eq!(alpha(RASTER_SIZE / 3, RASTER_SIZE / 2), 255);
        assert_eq!(alpha(RASTER_SIZE / 2, RASTER_SIZE / 2), 0);

        // The cell age trail fades: newest cell opaque, oldest faint.
        let image = rasterize(Icon::CellAge, RASTER_SIZE);
        let alpha = |p: Vec2| {
            let (x, y) = ((p * RASTER_SIZE as f32 / 24.0).as_uvec2()).into();
            image.data.as_ref().unwrap()[((y * RASTER_SIZE + x) * 4 + 3) as usize]
        };
        assert_eq!(alpha(vec2(18.5, 5.5)), 255);
        assert_eq!(alpha(vec2(5.5, 18.5)), 102);
    }
}
