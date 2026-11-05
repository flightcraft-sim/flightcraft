//! The tiny procedural atlas every corridor building tile carries: facade strips (one 8 m bay
//! wide, one strip per style and height class) and a 256-colour roof palette. Roof colours are
//! measured from NAIP; facade artwork is original and procedural, not a survey of real windows.
use image::{Rgba, RgbaImage};
use std::collections::BTreeMap;

pub(super) const STRIP_W: u32 = 32;
/// Each strip sits in a column with a replicated-edge gutter, so coarse mip levels (distant
/// buildings) blend a facade with itself rather than with the strip beside it.
const GUTTER: u32 = 12;
const COLUMN_W: u32 = STRIP_W + 2 * GUTTER;
pub(super) const PX_PER_FLOOR: u32 = 3;
pub(super) const FLOOR_M: f64 = 3.4;
/// Floors in each height-class strip.
pub(super) const CLASS_FLOORS: [u32; 5] = [4, 10, 24, 56, 120];
const BLOCK: u32 = 4;
pub(super) const PALETTE_SIDE: u32 = 16 * BLOCK;

pub(super) fn class_for(height_m: f64) -> u8 {
    let floors = (height_m / FLOOR_M).ceil().max(1.) as u32;
    CLASS_FLOORS
        .iter()
        .position(|f| *f >= floors)
        .unwrap_or(CLASS_FLOORS.len() - 1) as u8
}
/// Deterministic style from the building's position and height.
pub(super) fn style_for(height_m: f64, seed: u64) -> u8 {
    let r = ((seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 40) % 100) as u32;
    if height_m < 14. {
        [0, 0, 0, 0, 1, 1, 1, 2, 2, 3][(r / 10) as usize]
    } else if height_m < 45. {
        [0, 0, 1, 1, 2, 2, 2, 3, 3, 4][(r / 10) as usize]
    } else {
        [4, 4, 4, 4, 4, 2, 2, 3, 3, 3][(r / 10) as usize]
    }
}

fn hsv(rgb: [f32; 3]) -> (f32, f32, f32) {
    let [r, g, b] = rgb.map(|v| (v / 255.).clamp(0., 1.));
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d < 1e-6 {
        0.
    } else if max == r {
        60. * ((g - b) / d).rem_euclid(6.)
    } else if max == g {
        60. * ((b - r) / d + 2.)
    } else {
        60. * ((r - g) / d + 4.)
    };
    (h, if max <= 0. { 0. } else { d / max }, max)
}
fn from_hsv(h: f32, s: f32, v: f32) -> [u8; 3] {
    let c = v * s;
    let x = c * (1. - ((h / 60.).rem_euclid(2.) - 1.).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.) as u32 % 6 {
        0 => (c, x, 0.),
        1 => (x, c, 0.),
        2 => (0., c, x),
        3 => (0., x, c),
        4 => (x, 0., c),
        _ => (c, 0., x),
    };
    [r, g, b].map(|v| ((v + m) * 255.).round().clamp(0., 255.) as u8)
}
/// 32 greys then 8 hues x 2 saturations x 14 values.
pub(super) fn palette_index(rgb: [f32; 3]) -> u32 {
    let (h, s, v) = hsv(rgb);
    if s < 0.12 {
        return ((v - 0.06) / 0.92 * 31.).round().clamp(0., 31.) as u32;
    }
    let hue = ((h / 45.).round() as u32) % 8;
    let sat = u32::from(s >= 0.33);
    let val = ((v - 0.12) / 0.86 * 13.).round().clamp(0., 13.) as u32;
    32 + (hue * 2 + sat) * 14 + val
}
pub(super) fn palette_color(index: u32) -> [u8; 3] {
    if index < 32 {
        let v = 0.06 + 0.92 * index as f32 / 31.;
        return [(v * 255.) as u8; 3];
    }
    let i = index - 32;
    let (hs, val) = (i / 14, i % 14);
    let (hue, sat) = (hs / 2, hs % 2);
    from_hsv(
        hue as f32 * 45.,
        if sat == 0 { 0.22 } else { 0.45 },
        0.12 + 0.86 * val as f32 / 13.,
    )
}

fn strip(style: u8, floors: u32) -> RgbaImage {
    let height = floors * PX_PER_FLOOR;
    let mut image = RgbaImage::new(STRIP_W, height);
    let (wall, glass, mullion): ([f32; 3], [f32; 3], [f32; 3]) = match style {
        0 => ([150., 82., 64.], [38., 48., 58.], [90., 55., 45.]),
        1 => ([176., 150., 120.], [44., 52., 60.], [130., 110., 90.]),
        2 => ([192., 186., 168.], [58., 70., 82.], [160., 154., 138.]),
        3 => ([158., 158., 152.], [50., 62., 74.], [120., 120., 116.]),
        _ => ([62., 98., 126.], [96., 142., 176.], [48., 74., 96.]),
    };
    for y in 0..height {
        // Row 0 is the roofline; the last floor is the ground floor.
        let floor_from_bottom = (height - 1 - y) / PX_PER_FLOOR;
        let row = (height - 1 - y) % PX_PER_FLOOR; // 0 = floor slab, 1..2 = window
        for x in 0..STRIP_W {
            let mut c = wall;
            let in_window = match style {
                0..=2 => {
                    let w = if style == 2 { 4 } else { 5 };
                    row >= 1 && (3..3 + w).contains(&(x % 10)) && (2..STRIP_W - 2).contains(&x)
                }
                3 => row >= 1 && (1..STRIP_W - 1).contains(&x),
                _ => row >= 1,
            };
            if in_window {
                c = glass;
                if style == 4 && x % 4 == 0 {
                    c = mullion;
                }
                if style == 3 && x % 8 == 0 {
                    c = mullion;
                }
            } else if style == 4 {
                c = [wall[0] * 0.8, wall[1] * 0.8, wall[2] * 0.8];
            }
            if floor_from_bottom == 0 {
                // Ground floor: darker base and shopfront glazing.
                c = if row >= 1 && x % 8 != 0 {
                    [glass[0] * 0.8, glass[1] * 0.8, glass[2] * 0.85]
                } else {
                    [wall[0] * 0.62, wall[1] * 0.62, wall[2] * 0.62]
                };
            }
            // Cheap per-floor variation keeps tall faces from reading as a flat decal.
            let k = 0.94 + 0.06 * ((floor_from_bottom.wrapping_mul(2_654_435_761) >> 7) & 7) as f32 / 7.;
            image.put_pixel(
                x,
                y,
                Rgba([(c[0] * k) as u8, (c[1] * k) as u8, (c[2] * k) as u8, 255]),
            );
        }
    }
    image
}

pub(super) struct Atlas {
    pub width: u32,
    pub height: u32,
    pub palette_x: u32,
    columns: BTreeMap<(u8, u8), u32>,
}
impl Atlas {
    pub fn new(used: &std::collections::BTreeSet<(u8, u8)>) -> Self {
        let columns: BTreeMap<_, _> = used
            .iter()
            .enumerate()
            .map(|(i, k)| (*k, i as u32 * COLUMN_W))
            .collect();
        let tallest = used
            .iter()
            .map(|(_, c)| CLASS_FLOORS[usize::from(*c)] * PX_PER_FLOOR)
            .max()
            .unwrap_or(0);
        Self {
            width: columns.len() as u32 * COLUMN_W + PALETTE_SIDE,
            height: tallest.max(PALETTE_SIDE),
            palette_x: columns.len() as u32 * COLUMN_W,
            columns,
        }
    }
    /// UV of a point `u` (0..1) across a bay and `height_m` above the ground, on a strip.
    pub fn wall_uv(&self, style: u8, class: u8, u: f64, height_m: f64) -> [f32; 2] {
        let x0 = self.columns.get(&(style, class)).copied().unwrap_or(0);
        let strip_h = CLASS_FLOORS[usize::from(class)] * PX_PER_FLOOR;
        let floors = (height_m / FLOOR_M).clamp(0., f64::from(CLASS_FLOORS[usize::from(class)]));
        let y = f64::from(strip_h) - floors * f64::from(PX_PER_FLOOR);
        [
            ((f64::from(x0 + GUTTER) + 1. + u.clamp(0., 1.) * f64::from(STRIP_W - 2)) / f64::from(self.width))
                as f32,
            (y.clamp(0.5, f64::from(strip_h) - 0.5) / f64::from(self.height)) as f32,
        ]
    }
    pub fn palette_uv(&self, index: u32) -> [f32; 2] {
        let (bx, by) = (index % 16, index / 16);
        [
            ((self.palette_x + bx * BLOCK + BLOCK / 2) as f32) / self.width as f32,
            ((by * BLOCK + BLOCK / 2) as f32) / self.height as f32,
        ]
    }
    pub fn image(&self) -> RgbaImage {
        let mut image = RgbaImage::from_pixel(self.width, self.height, Rgba([128, 128, 128, 255]));
        for ((style, class), x0) in &self.columns {
            let s = strip(*style, CLASS_FLOORS[usize::from(*class)]);
            image::imageops::replace(&mut image, &s, i64::from(*x0 + GUTTER), 0);
            for y in 0..s.height() {
                let (left, right) = (*s.get_pixel(0, y), *s.get_pixel(STRIP_W - 1, y));
                for g in 0..GUTTER {
                    image.put_pixel(*x0 + g, y, left);
                    image.put_pixel(*x0 + GUTTER + STRIP_W + g, y, right);
                }
            }
        }
        for index in 0..256 {
            let c = palette_color(index);
            for dy in 0..BLOCK {
                for dx in 0..BLOCK {
                    image.put_pixel(
                        self.palette_x + (index % 16) * BLOCK + dx,
                        (index / 16) * BLOCK + dy,
                        Rgba([c[0], c[1], c[2], 255]),
                    );
                }
            }
        }
        image
    }
    pub fn webp(&self) -> Result<Vec<u8>, String> {
        let image = self.image();
        let mut bytes = Vec::new();
        image::codecs::webp::WebPEncoder::new_lossless(&mut bytes)
            .encode(
                image.as_raw(),
                image.width(),
                image.height(),
                image::ExtendedColorType::Rgba8,
            )
            .map_err(|e| e.to_string())?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_round_trips_to_a_close_colour() {
        for rgb in [
            [200., 60., 50.],
            [90., 90., 92.],
            [30., 120., 40.],
            [230., 225., 210.],
            [70., 90., 130.],
        ] {
            let c = palette_color(palette_index(rgb));
            let d: f32 = rgb.iter().zip(c).map(|(a, b)| (a - f32::from(b)).abs()).sum();
            assert!(d < 150., "{rgb:?} -> {c:?}");
        }
        assert!((0..256).all(|i| palette_index(palette_color(i).map(f32::from)) < 256));
    }
    #[test]
    fn classes_cover_the_tallest_building_and_uvs_stay_in_the_atlas() {
        assert_eq!(class_for(8.), 0);
        assert_eq!(class_for(30.), 1);
        assert_eq!(class_for(541.), 4);
        let used = std::collections::BTreeSet::from([(0, 0), (4, 4)]);
        let atlas = Atlas::new(&used);
        for (s, c) in used {
            for h in [0., 5., 50., 400.] {
                for u in [0., 0.5, 1.] {
                    let uv = atlas.wall_uv(s, c, u, h);
                    assert!((0. ..=1.).contains(&uv[0]) && (0. ..=1.).contains(&uv[1]));
                }
            }
        }
        let uv = atlas.palette_uv(255);
        assert!(uv[0] < 1. && uv[1] < 1.);
        assert!(atlas.webp().unwrap().len() > 100);
    }
}
