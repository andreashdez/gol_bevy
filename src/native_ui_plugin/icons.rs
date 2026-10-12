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
    Gear,
}

impl Icon {
    const ALL: [Icon; 6] = [
        Icon::Play,
        Icon::Pause,
        Icon::Step,
        Icon::Undo,
        Icon::Redo,
        Icon::Gear,
    ];

    /// Buttons that show an icon in place of their name.
    pub(super) fn of(action: Action) -> Option<Icon> {
        match action {
            Action::Play => Some(Icon::Play),
            Action::Step => Some(Icon::Step),
            Action::Undo => Some(Icon::Undo),
            Action::Redo => Some(Icon::Redo),
            Action::MoreMenu => Some(Icon::Gear),
            _ => None,
        }
    }

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
            Icon::Gear => gear(p),
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
            let coverage = (0.5 - icon.distance(p) / scale).clamp(0.0, 1.0);
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
            (Icon::Gear, vec2(12.0, 6.5)),
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
    }
}
