//! `texture.fetch`: read a render target back and describe it with numbers and a picture.
//!
//! Without `texture` it lists what can be read ([`Renderer::fetchable_textures`]). With
//! one it reads a rectangle (default: all of it) and reports, per channel, the minimum,
//! maximum, mean and how many values are NaN or infinite; the values at given `points`;
//! and an image of up to three channels: mapped linearly from a value range (by default
//! the 0.5th to 99.5th percentile, so a few extreme values do not wash it out), tonemapped
//! for HDR colour, or in false colour for one channel. NaN and infinite pixels are drawn
//! magenta.

use std::io::Cursor;

use serde_json::{json, Value};

use crate::renderer::readback::Readback;
use crate::renderer::Renderer;

const CHANNEL_NAMES: [&str; 4] = ["r", "g", "b", "a"];
const DEFAULT_MAX_SIZE: u32 = 1024;
const MAGENTA: [u8; 4] = [255, 0, 255, 255];

/// How the image maps values to colours.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Mode {
    /// `range` low to high, linearly, to black to white per channel.
    Range,
    /// HDR colour: Reinhard (x / (1 + x)), then sRGB encoding.
    Tonemap,
    /// One channel through the Turbo colour map over `range`.
    FalseColor,
}

pub(crate) fn fetch(params: &Value, renderer: &Renderer) -> Result<Value, String> {
    let targets = renderer.fetchable_textures();
    let Some(name) = params.get("texture").and_then(Value::as_str) else {
        let list: Vec<Value> = targets
            .iter()
            .map(|target| {
                let (width, height, layers) = target.size();
                json!({
                    "name": target.name,
                    "size": [width, height],
                    "layers": layers,
                    "format": target.format(),
                    "description": target.description,
                })
            })
            .collect();
        return Ok(json!({
            "textures": list,
            "note": "Pass texture (a name) to read one. They hold what the last frame left in them; path tracing targets exist only while it is on.",
        }));
    };
    let Some(target) = targets.iter().find(|target| target.name == name) else {
        let names: Vec<&str> = targets.iter().map(|target| target.name.as_str()).collect();
        return Err(format!(
            "unknown texture {name:?}; available: {}",
            names.join(", ")
        ));
    };
    let (width, height, layers) = target.size();
    let rect = match params.get("rect").filter(|value| !value.is_null()) {
        Some(value) => {
            let numbers: Vec<u32> = value
                .as_array()
                .filter(|items| items.len() == 4)
                .and_then(|items| {
                    items
                        .iter()
                        .map(|item| item.as_u64().map(|n| n as u32))
                        .collect()
                })
                .ok_or("rect must be [x, y, width, height] in pixels")?;
            [numbers[0], numbers[1], numbers[2], numbers[3]]
        }
        None => [0, 0, width, height],
    };
    let layer = params.get("layer").and_then(Value::as_u64).unwrap_or(0) as u32;

    #[cfg(target_arch = "wasm32")]
    {
        let _ = (rect, layer, layers);
        return Err("texture.fetch needs a native build (it waits for the GPU)".into());
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let readback = renderer.read_texture(target, rect, layer)?;
        let origin = [
            rect[0].min(width.saturating_sub(1)),
            rect[1].min(height.saturating_sub(1)),
        ];
        let mut report = json!({
            "texture": target.name,
            "description": target.description,
            "format": target.format(),
            "size": [width, height],
            "layers": layers,
            "layer": layer,
            "rect": [origin[0], origin[1], readback.width, readback.height],
            "integer": readback.integer,
            "stats": stats(&readback),
        });
        if let Some(points) = params.get("points").filter(|value| !value.is_null()) {
            report["points"] = point_values(&readback, origin, points)?;
        }
        if params.get("image").and_then(Value::as_bool) != Some(false) {
            let (image, description) = render_image(&readback, params)?;
            let max_size = params
                .get("max_size")
                .and_then(Value::as_u64)
                .map_or(DEFAULT_MAX_SIZE, |size| size.clamp(16, 8192) as u32);
            let image = shrink(image, max_size);
            let mut png = Vec::new();
            image
                .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
                .map_err(|err| format!("failed to encode PNG: {err}"))?;
            if let Some(path) = params.get("save_path").and_then(Value::as_str) {
                std::fs::write(path, &png)
                    .map_err(|err| format!("failed to write {path}: {err}"))?;
                report["path"] = json!(path);
            }
            report["image"] = description;
            report["image"]["width"] = json!(image.width());
            report["image"]["height"] = json!(image.height());
            report["png_base64"] = json!(base64::encode(&png));
        }
        Ok(report)
    }
}

fn round(value: f64) -> Value {
    if value.is_finite() {
        json!((value * 1e6).round() / 1e6)
    } else {
        json!(value.to_string())
    }
}

/// Per channel: minimum, maximum and mean of the finite values, and how many are NaN or
/// infinite.
fn stats(readback: &Readback) -> Value {
    let mut out = serde_json::Map::new();
    for (channel, name) in CHANNEL_NAMES.iter().enumerate().take(readback.channels) {
        let (mut min, mut max, mut sum, mut count, mut nan, mut infinite) = (
            f64::INFINITY,
            f64::NEG_INFINITY,
            0.0,
            0usize,
            0usize,
            0usize,
        );
        for value in readback
            .values
            .iter()
            .skip(channel)
            .step_by(readback.channels)
        {
            if value.is_nan() {
                nan += 1;
            } else if value.is_infinite() {
                infinite += 1;
            } else {
                min = min.min(*value);
                max = max.max(*value);
                sum += value;
                count += 1;
            }
        }
        let mean = if count > 0 {
            sum / count as f64
        } else {
            f64::NAN
        };
        out.insert(
            name.to_string(),
            json!({
                "min": if count > 0 { round(min) } else { Value::Null },
                "max": if count > 0 { round(max) } else { Value::Null },
                "mean": if count > 0 { round(mean) } else { Value::Null },
                "nan": nan,
                "infinite": infinite,
            }),
        );
    }
    Value::Object(out)
}

/// The values at `points` ([[x, y], ...] in texture pixels, inside the rectangle read).
fn point_values(readback: &Readback, origin: [u32; 2], points: &Value) -> Result<Value, String> {
    let points = points
        .as_array()
        .ok_or("points must be a list of [x, y] pixels")?;
    let mut out = Vec::with_capacity(points.len());
    for point in points {
        let xy: Vec<u32> = point
            .as_array()
            .filter(|items| items.len() == 2)
            .and_then(|items| {
                items
                    .iter()
                    .map(|item| item.as_u64().map(|n| n as u32))
                    .collect()
            })
            .ok_or("each point must be [x, y] in pixels")?;
        let (x, y) = (xy[0], xy[1]);
        if x < origin[0]
            || y < origin[1]
            || x - origin[0] >= readback.width
            || y - origin[1] >= readback.height
        {
            return Err(format!("point [{x}, {y}] is outside the rectangle read"));
        }
        let values: Vec<Value> = (0..readback.channels)
            .map(|channel| round(readback.get(x - origin[0], y - origin[1], channel)))
            .collect();
        out.push(json!({ "x": x, "y": y, "value": values }));
    }
    Ok(Value::Array(out))
}

/// Channel indices from letters ("rgb", "a", "rg"), at most three.
fn parse_channels(text: &str, available: usize) -> Result<Vec<usize>, String> {
    let mut channels = Vec::new();
    for letter in text.chars() {
        let index = CHANNEL_NAMES
            .iter()
            .position(|name| name.starts_with(letter))
            .ok_or_else(|| format!("unknown channel {letter:?}; use r, g, b or a"))?;
        if index >= available {
            return Err(format!(
                "channel {letter} is not in this texture ({available} channels)"
            ));
        }
        channels.push(index);
    }
    if channels.is_empty() || channels.len() > 3 {
        return Err(
            "pick one to three channels for the image, like \"rgb\", \"a\" or \"rg\"".into(),
        );
    }
    Ok(channels)
}

/// The value at quantile `q` (0-1) of the finite values in `channels`, from a sample of at
/// most a million values.
fn quantile(readback: &Readback, channels: &[usize], q: f64) -> Option<f64> {
    let pixels = readback.width as usize * readback.height as usize;
    let step = (pixels * channels.len() / 1_000_000).max(1);
    let mut sample: Vec<f64> = (0..pixels)
        .step_by(step)
        .flat_map(|pixel| channels.iter().map(move |channel| (pixel, *channel)))
        .map(|(pixel, channel)| readback.values[pixel * readback.channels + channel])
        .filter(|value| value.is_finite())
        .collect();
    if sample.is_empty() {
        return None;
    }
    let index = ((sample.len() - 1) as f64 * q).round() as usize;
    let (_, value, _) = sample.select_nth_unstable_by(index, |a, b| a.total_cmp(b));
    Some(*value)
}

fn linear_to_srgb(value: f64) -> f64 {
    let value = value.clamp(0.0, 1.0);
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

/// The Turbo colour map (Mikhailov 2019), polynomial fit.
fn turbo(t: f64) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let channel = |c: [f64; 6]| {
        let v = c[0] + t * (c[1] + t * (c[2] + t * (c[3] + t * (c[4] + t * c[5]))));
        (v.clamp(0.0, 1.0) * 255.0).round() as u8
    };
    [
        channel([
            0.13572138,
            4.61539260,
            -42.66032258,
            132.13108234,
            -152.94239396,
            59.28637943,
        ]),
        channel([
            0.09140261,
            2.19418839,
            4.84296658,
            -14.18503333,
            4.27729857,
            2.82956604,
        ]),
        channel([
            0.10667330,
            12.64194608,
            -60.58204836,
            110.36276771,
            -89.90310912,
            27.34824973,
        ]),
    ]
}

/// The picture: the chosen channels mapped to colours, and a description of how.
fn render_image(readback: &Readback, params: &Value) -> Result<(image::RgbaImage, Value), String> {
    let default_channels = match readback.channels {
        1 => "r",
        2 => "rg",
        _ => "rgb",
    };
    let channels_text = params
        .get("channels")
        .and_then(Value::as_str)
        .unwrap_or(default_channels);
    let channels = parse_channels(channels_text, readback.channels)?;
    let mode = match params
        .get("mode")
        .and_then(Value::as_str)
        .unwrap_or("range")
    {
        "range" => Mode::Range,
        "tonemap" => Mode::Tonemap,
        "false_color" if channels.len() == 1 => Mode::FalseColor,
        "false_color" => {
            return Err("false_color shows one channel; pass channels like \"r\"".into())
        }
        other => {
            return Err(format!(
                "unknown mode {other:?}; use range, tonemap or false_color"
            ))
        }
    };
    let range = match params.get("range").filter(|value| !value.is_null()) {
        Some(value) => {
            let pair: Vec<f64> = value
                .as_array()
                .filter(|items| items.len() == 2)
                .and_then(|items| items.iter().map(Value::as_f64).collect())
                .ok_or("range must be [low, high]")?;
            (pair[0], pair[1])
        }
        None => (
            quantile(readback, &channels, 0.005).unwrap_or(0.0),
            quantile(readback, &channels, 0.995).unwrap_or(1.0),
        ),
    };
    let (low, high) = if range.1 > range.0 {
        range
    } else {
        (range.0, range.0 + 1.0)
    };

    let mut image = image::RgbaImage::new(readback.width, readback.height);
    for (index, pixel) in image.pixels_mut().enumerate() {
        let values: Vec<f64> = channels
            .iter()
            .map(|channel| readback.values[index * readback.channels + channel])
            .collect();
        if values.iter().any(|value| !value.is_finite()) {
            pixel.0 = MAGENTA;
            continue;
        }
        let normalized = |value: f64| ((value - low) / (high - low)).clamp(0.0, 1.0);
        let byte = |value: f64| (value.clamp(0.0, 1.0) * 255.0).round() as u8;
        pixel.0 = match mode {
            Mode::FalseColor => {
                let [r, g, b] = turbo(normalized(values[0]));
                [r, g, b, 255]
            }
            Mode::Tonemap => {
                let tone =
                    |value: f64| byte(linear_to_srgb(value.max(0.0) / (1.0 + value.max(0.0))));
                match values.len() {
                    1 => {
                        let v = tone(values[0]);
                        [v, v, v, 255]
                    }
                    2 => [tone(values[0]), tone(values[1]), 0, 255],
                    _ => [tone(values[0]), tone(values[1]), tone(values[2]), 255],
                }
            }
            Mode::Range => match values.len() {
                1 => {
                    let v = byte(normalized(values[0]));
                    [v, v, v, 255]
                }
                2 => [
                    byte(normalized(values[0])),
                    byte(normalized(values[1])),
                    0,
                    255,
                ],
                _ => [
                    byte(normalized(values[0])),
                    byte(normalized(values[1])),
                    byte(normalized(values[2])),
                    255,
                ],
            },
        };
    }
    let description = json!({
        "channels": channels.iter().map(|channel| CHANNEL_NAMES[*channel]).collect::<String>(),
        "mode": match mode {
            Mode::Range => "range",
            Mode::Tonemap => "tonemap",
            Mode::FalseColor => "false_color",
        },
        "range": if mode == Mode::Tonemap { Value::Null } else { json!([round(low), round(high)]) },
        "note": "NaN and infinite pixels are magenta. Without range, the image spans the 0.5th to 99.5th percentile of the shown channels.",
    });
    Ok((image, description))
}

/// Shrink so the longest side is at most `max_size`, nearest-neighbour (values stay exact).
fn shrink(image: image::RgbaImage, max_size: u32) -> image::RgbaImage {
    let longest = image.width().max(image.height());
    if longest <= max_size {
        return image;
    }
    let scale = max_size as f32 / longest as f32;
    let width = ((image.width() as f32 * scale).round() as u32).max(1);
    let height = ((image.height() as f32 * scale).round() as u32).max(1);
    image::imageops::resize(&image, width, height, image::imageops::FilterType::Nearest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn readback(channels: usize, values: Vec<f64>, width: u32) -> Readback {
        let height = (values.len() / channels) as u32 / width;
        Readback {
            width,
            height,
            channels,
            values,
            integer: false,
        }
    }

    #[test]
    fn stats_skip_nan_and_infinity() {
        let data = readback(1, vec![1.0, f64::NAN, 3.0, f64::INFINITY], 2);
        let stats = stats(&data);
        assert_eq!(stats["r"]["min"], json!(1.0));
        assert_eq!(stats["r"]["max"], json!(3.0));
        assert_eq!(stats["r"]["mean"], json!(2.0));
        assert_eq!(stats["r"]["nan"], json!(1));
        assert_eq!(stats["r"]["infinite"], json!(1));
    }

    #[test]
    fn points_read_texture_coordinates_inside_the_rectangle() {
        let data = readback(2, vec![0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0], 2);
        let values = point_values(&data, [10, 20], &json!([[11, 21]])).unwrap();
        assert_eq!(values[0]["value"], json!([6.0, 7.0]));
        assert!(point_values(&data, [10, 20], &json!([[9, 20]])).is_err());
    }

    #[test]
    fn images_map_the_range_and_mark_bad_values() {
        let data = readback(1, vec![0.0, 0.5, 1.0, f64::NAN], 4);
        let (image, description) = render_image(&data, &json!({"range": [0.0, 1.0]})).unwrap();
        assert_eq!(image.get_pixel(0, 0).0, [0, 0, 0, 255]);
        assert_eq!(image.get_pixel(1, 0).0, [128, 128, 128, 255]);
        assert_eq!(image.get_pixel(2, 0).0, [255, 255, 255, 255]);
        assert_eq!(image.get_pixel(3, 0).0, MAGENTA);
        assert_eq!(description["channels"], json!("r"));
        assert!(render_image(&data, &json!({"channels": "g"})).is_err());
        assert!(render_image(&readback(4, vec![0.0; 4], 1), &json!({"channels": "rgba"})).is_err());
    }

    #[test]
    fn tonemapping_compresses_hdr() {
        let data = readback(3, vec![0.0, 1.0, 1000.0], 1);
        let (image, _) = render_image(&data, &json!({"mode": "tonemap"})).unwrap();
        let [r, g, b, a] = image.get_pixel(0, 0).0;
        assert_eq!((r, a), (0, 255));
        assert!(g > 150 && g < 200, "1.0 maps to about 0.5 before sRGB: {g}");
        assert!(b >= 254);
    }
}
