//! Deterministic, tileable material textures and maps derived from a height field.

use image::{Rgba, RgbaImage};
use serde::Deserialize;
use std::f64::consts::TAU;

/// Largest accepted dimension and total pixel count, also used for imported height maps.
pub(crate) const MAX_SIZE: u32 = 2048;

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Pattern {
    #[default]
    Noise,
    Bricks,
    Checker,
    Wood,
    Marble,
    Voronoi,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub(crate) struct TextureOptions {
    pub pattern: Pattern,
    pub size: [u32; 2],
    pub seed: u64,
    pub scale: u32,
    pub octaves: u32,
    pub mortar: f64,
}

impl Default for TextureOptions {
    fn default() -> Self {
        Self {
            pattern: Pattern::Noise,
            size: [256, 256],
            seed: 0,
            scale: 4,
            octaves: 5,
            mortar: 0.06,
        }
    }
}

/// Data maps use linear values. Only base colour is encoded as sRGB colour bytes.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MapKind {
    BaseColor,
    Height,
    Normal,
    Roughness,
    MetallicRoughness,
    Occlusion,
    Orm,
}

impl MapKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::BaseColor => "base_color",
            Self::Height => "height",
            Self::Normal => "normal",
            Self::Roughness => "roughness",
            Self::MetallicRoughness => "metallic_roughness",
            Self::Occlusion => "occlusion",
            Self::Orm => "orm",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub(crate) struct MapOptions {
    pub normal_strength: f64,
    pub flip_y: bool,
    pub tileable: bool,
    pub roughness: f64,
    pub roughness_variation: f64,
    pub metallic: f64,
    pub ao_strength: f64,
    pub ao_radius: u32,
    pub invert: bool,
    pub channel: HeightChannel,
}

impl Default for MapOptions {
    fn default() -> Self {
        Self {
            normal_strength: 0.05,
            flip_y: false,
            tileable: true,
            roughness: 0.7,
            roughness_variation: 0.2,
            metallic: 0.0,
            ao_strength: 1.0,
            ao_radius: 4,
            invert: false,
            channel: HeightChannel::Luminance,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum HeightChannel {
    Luminance,
    Red,
    Green,
    Blue,
    Alpha,
}

pub(crate) struct HeightField {
    pub width: u32,
    pub height: u32,
    values: Vec<f64>,
}

pub(crate) fn validate_size([width, height]: [u32; 2]) -> Result<(), String> {
    if width == 0 || height == 0 || width > MAX_SIZE || height > MAX_SIZE {
        return Err(format!(
            "texture size must be in 1..={MAX_SIZE} on each axis"
        ));
    }
    Ok(())
}

/// A stable hash shared by the noise field and sound synthesis (no platform RNG state).
pub(crate) fn hash(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e3779b97f4a7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58476d1ce4e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d049bb133111eb);
    value ^ (value >> 31)
}

fn random(x: i64, y: i64, period: i64, seed: u64) -> f64 {
    let key = (x.rem_euclid(period) as u64) | ((y.rem_euclid(period) as u64) << 32);
    (hash(key ^ hash(seed)) >> 11) as f64 / (1u64 << 53) as f64
}

fn smooth(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn noise(u: f64, v: f64, period: u32, seed: u64) -> f64 {
    let (x, y) = (u * f64::from(period), v * f64::from(period));
    let (ix, iy) = (x.floor() as i64, y.floor() as i64);
    let (tx, ty) = (smooth(x - x.floor()), smooth(y - y.floor()));
    let r = |dx, dy| random(ix + dx, iy + dy, i64::from(period), seed);
    let a = r(0, 0) * (1.0 - tx) + r(1, 0) * tx;
    let b = r(0, 1) * (1.0 - tx) + r(1, 1) * tx;
    a * (1.0 - ty) + b * ty
}

impl TextureOptions {
    fn sample(&self, u: f64, v: f64) -> f64 {
        let mut n = 0.0;
        let mut weight = 1.0;
        let mut total = 0.0;
        for octave in 0..self.octaves {
            n += noise(
                u,
                v,
                self.scale << octave,
                self.seed.wrapping_add(u64::from(octave)),
            ) * weight;
            total += weight;
            weight *= 0.5;
        }
        n /= total;
        let scale = f64::from(self.scale);
        match self.pattern {
            Pattern::Noise => n,
            Pattern::Checker => {
                let parity = ((u * scale * 2.0).floor() as i64 + (v * scale * 2.0).floor() as i64)
                    .rem_euclid(2);
                0.15 + parity as f64 * 0.7 + (n - 0.5) * 0.1
            }
            Pattern::Bricks => {
                let row = (v * scale * 2.0).floor() as i64;
                let x = (u * scale * 2.0 + row.rem_euclid(2) as f64 * 0.5).rem_euclid(1.0);
                let y = (v * scale * 2.0).rem_euclid(1.0);
                let edge = x.min(1.0 - x).min(y.min(1.0 - y));
                let bevel = smooth(((edge - self.mortar * 0.5) / self.mortar).clamp(0.0, 1.0));
                0.1 + bevel * (0.65 + n * 0.2)
            }
            Pattern::Wood => {
                let grain = (TAU * (v * scale + noise(u, v, self.scale, self.seed) * 0.7)).sin();
                (0.5 + grain * 0.3 + (n - 0.5) * 0.3).clamp(0.0, 1.0)
            }
            Pattern::Marble => {
                let veins = (TAU * (u * scale + n * 2.0)).sin().abs().powf(0.3);
                (veins * 0.85 + n * 0.15).clamp(0.0, 1.0)
            }
            Pattern::Voronoi => {
                let (x, y) = (u * scale, v * scale);
                let (ix, iy) = (x.floor() as i64, y.floor() as i64);
                let mut nearest = f64::INFINITY;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let cx = ix + dx;
                        let cy = iy + dy;
                        let px = cx as f64 + random(cx, cy, i64::from(self.scale), self.seed);
                        let py =
                            cy as f64 + random(cx, cy, i64::from(self.scale), self.seed ^ 0xabcdef);
                        nearest = nearest.min((x - px).hypot(y - py));
                    }
                }
                (1.0 - nearest / 1.4).clamp(0.0, 1.0)
            }
        }
    }

    pub fn generate(&self) -> Result<HeightField, String> {
        validate_size(self.size)?;
        if !(1..=64).contains(&self.scale) || !(1..=8).contains(&self.octaves) {
            return Err("scale must be in 1..=64 and octaves in 1..=8".into());
        }
        if !self.mortar.is_finite() || !(0.001..=0.3).contains(&self.mortar) {
            return Err("mortar must be in 0.001..=0.3".into());
        }
        let [width, height] = self.size;
        let values = (0..height)
            .flat_map(|y| {
                (0..width).map(move |x| {
                    self.sample(
                        (f64::from(x) + 0.5) / f64::from(width),
                        (f64::from(y) + 0.5) / f64::from(height),
                    )
                })
            })
            .collect();
        Ok(HeightField {
            width,
            height,
            values,
        })
    }
}

impl MapOptions {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value, max) in [
            ("normal_strength", self.normal_strength, 10.0),
            ("roughness", self.roughness, 1.0),
            ("roughness_variation", self.roughness_variation, 1.0),
            ("metallic", self.metallic, 1.0),
            ("ao_strength", self.ao_strength, 10.0),
        ] {
            if !value.is_finite() || !(0.0..=max).contains(&value) {
                return Err(format!("{name} must be in 0..={max}"));
            }
        }
        if !(1..=32).contains(&self.ao_radius) {
            return Err("ao_radius must be in 1..=32 pixels".into());
        }
        Ok(())
    }
}

impl HeightField {
    pub fn from_image(image: &RgbaImage, options: &MapOptions) -> Result<Self, String> {
        validate_size([image.width(), image.height()])?;
        let values = image
            .pixels()
            .map(|p| {
                let value = match options.channel {
                    HeightChannel::Luminance => {
                        0.2126 * f64::from(p[0])
                            + 0.7152 * f64::from(p[1])
                            + 0.0722 * f64::from(p[2])
                    }
                    HeightChannel::Red => f64::from(p[0]),
                    HeightChannel::Green => f64::from(p[1]),
                    HeightChannel::Blue => f64::from(p[2]),
                    HeightChannel::Alpha => f64::from(p[3]),
                } / 255.0;
                if options.invert {
                    1.0 - value
                } else {
                    value
                }
            })
            .collect();
        Ok(Self {
            width: image.width(),
            height: image.height(),
            values,
        })
    }

    fn at(&self, x: i64, y: i64, tileable: bool) -> f64 {
        let coord = |v: i64, size: u32| {
            (if tileable {
                v.rem_euclid(i64::from(size))
            } else {
                v.clamp(0, i64::from(size) - 1)
            }) as usize
        };
        self.values[coord(x, self.width) + coord(y, self.height) * self.width as usize]
    }

    pub fn render(&self, kind: MapKind, options: &MapOptions, colors: [[f32; 4]; 2]) -> RgbaImage {
        let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        RgbaImage::from_fn(self.width, self.height, |x, y| {
            let (x, y) = (i64::from(x), i64::from(y));
            let h = self.at(x, y, options.tileable);
            let at = |dx, dy| self.at(x + dx, y + dy, options.tileable);
            let roughness = byte(options.roughness + (0.5 - h) * options.roughness_variation);
            let metallic = byte(options.metallic);
            let ao = || {
                let r = i64::from(options.ao_radius);
                let raised: f64 = [
                    (-r, 0),
                    (r, 0),
                    (0, -r),
                    (0, r),
                    (-r, -r),
                    (r, -r),
                    (-r, r),
                    (r, r),
                ]
                .into_iter()
                .map(|(dx, dy)| (at(dx, dy) - h).max(0.0))
                .sum();
                byte(1.0 - raised * options.ao_strength / 8.0)
            };
            Rgba(match kind {
                MapKind::BaseColor => std::array::from_fn(|c| {
                    byte(f64::from(colors[0][c]) * (1.0 - h) + f64::from(colors[1][c]) * h)
                }),
                MapKind::Height => [byte(h), byte(h), byte(h), 255],
                MapKind::Normal => {
                    let dx = (at(1, 0) - at(-1, 0))
                        * f64::from(self.width)
                        * 0.5
                        * options.normal_strength;
                    let dy = (at(0, 1) - at(0, -1))
                        * f64::from(self.height)
                        * 0.5
                        * options.normal_strength;
                    let length = (dx * dx + dy * dy + 1.0).sqrt();
                    let ny = if options.flip_y { dy } else { -dy };
                    [
                        byte(0.5 - dx / length * 0.5),
                        byte(0.5 + ny / length * 0.5),
                        byte(0.5 + 0.5 / length),
                        255,
                    ]
                }
                MapKind::Roughness => [roughness, roughness, roughness, 255],
                MapKind::MetallicRoughness => [255, roughness, metallic, 255],
                MapKind::Occlusion => {
                    let a = ao();
                    [a, a, a, 255]
                }
                MapKind::Orm => [ao(), roughness, metallic, 255],
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_are_periodic_and_seeded() {
        for pattern in [
            Pattern::Noise,
            Pattern::Bricks,
            Pattern::Checker,
            Pattern::Wood,
            Pattern::Marble,
            Pattern::Voronoi,
        ] {
            let options = TextureOptions {
                pattern,
                scale: 3,
                ..Default::default()
            };
            for (u, v) in [(0.123, 0.321), (0.997, 0.003)] {
                let h = options.sample(u, v);
                assert!((0.0..=1.0).contains(&h));
                assert!((h - options.sample(u + 1.0, v)).abs() < 1e-10);
                assert!((h - options.sample(u, v + 1.0)).abs() < 1e-10);
            }
        }
        let a = TextureOptions {
            size: [16, 8],
            ..Default::default()
        }
        .generate()
        .unwrap();
        let b = TextureOptions {
            size: [16, 8],
            seed: 1,
            ..Default::default()
        }
        .generate()
        .unwrap();
        assert_ne!(a.values, b.values);
        assert_eq!(
            a.values,
            TextureOptions {
                size: [16, 8],
                ..Default::default()
            }
            .generate()
            .unwrap()
            .values
        );
    }

    #[test]
    fn flat_height_produces_flat_normals_and_correct_packed_channels() {
        let field = HeightField {
            width: 4,
            height: 2,
            values: vec![0.5; 8],
        };
        let options = MapOptions {
            roughness: 0.4,
            metallic: 0.8,
            ..Default::default()
        };
        let colors = [[0.0; 4], [1.0; 4]];
        assert!(field
            .render(MapKind::Normal, &options, colors)
            .pixels()
            .all(|p| p.0 == [128, 128, 255, 255]));
        assert!(field
            .render(MapKind::Orm, &options, colors)
            .pixels()
            .all(|p| p.0 == [255, 102, 204, 255]));
    }

    #[test]
    fn normals_follow_height_and_flip_only_green() {
        let field = HeightField {
            width: 3,
            height: 3,
            values: vec![0.0, 0.1, 0.2, 0.2, 0.3, 0.4, 0.4, 0.5, 0.6],
        };
        let mut options = MapOptions {
            tileable: false,
            ..Default::default()
        };
        let colors = [[0.0; 4], [1.0; 4]];
        let a = field
            .render(MapKind::Normal, &options, colors)
            .get_pixel(1, 1)
            .0;
        options.flip_y = true;
        let b = field
            .render(MapKind::Normal, &options, colors)
            .get_pixel(1, 1)
            .0;
        assert!(a[0] < 128 && a[1] < 128 && b[1] > 128);
        assert_eq!(a[0], b[0]);
        assert_eq!(a[2], b[2]);
        assert_eq!(u16::from(a[1]) + u16::from(b[1]), 255);
        let n = a.map(|v| f64::from(v) / 255.0 * 2.0 - 1.0);
        assert!((n[0].powi(2) + n[1].powi(2) + n[2].powi(2) - 1.0).abs() < 0.02);
    }

    #[test]
    fn invalid_sizes_and_parameters_are_rejected() {
        assert!(TextureOptions {
            size: [u32::MAX, 1],
            ..Default::default()
        }
        .generate()
        .is_err());
        assert!(TextureOptions {
            octaves: 0,
            ..Default::default()
        }
        .generate()
        .is_err());
        assert!(MapOptions {
            normal_strength: f64::NAN,
            ..Default::default()
        }
        .validate()
        .is_err());
    }
}
