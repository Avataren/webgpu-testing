//! Measure a sound file, for agents that cannot listen to it: length, level, clipping,
//! silence at the ends, how well it loops, and a waveform-plus-spectrogram picture.

use std::f64::consts::PI;
use std::io::Cursor;
use std::path::Path;

use serde_json::{json, Value};

/// A sample at or above this (absolute) counts as clipped.
const CLIP_LEVEL: f64 = 0.999;
/// Quieter than this (-60 dBFS) counts as silence.
const SILENCE_LEVEL: f64 = 0.001;
const IMAGE_WIDTH: u32 = 640;
const WAVE_HEIGHT: u32 = 96;
const SPECTRUM_HEIGHT: u32 = 192;
const FFT_SIZE: usize = 1024;

fn db(level: f64) -> Value {
    if level <= 1e-9 {
        json!("-inf")
    } else {
        json!((20.0 * level.log10() * 10.0).round() / 10.0)
    }
}

fn round(value: f64, places: i32) -> f64 {
    let scale = 10f64.powi(places);
    (value * scale).round() / scale
}

/// Decoded audio: interleaved samples in -1..1.
struct Clip {
    channels: usize,
    sample_rate: u32,
    samples: Vec<f32>,
}

impl Clip {
    fn frames(&self) -> usize {
        self.samples.len() / self.channels
    }

    /// The mean of the channels, one value per frame.
    fn mono(&self) -> Vec<f32> {
        self.samples
            .chunks_exact(self.channels)
            .map(|frame| frame.iter().sum::<f32>() / self.channels as f32)
            .collect()
    }
}

fn decode(path: &Path) -> Result<Clip, String> {
    use rodio::Source;
    let bytes =
        std::fs::read(path).map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    let decoder = rodio::Decoder::try_from(Cursor::new(bytes))
        .map_err(|err| format!("cannot decode {}: {err}", path.display()))?;
    let channels = usize::from(decoder.channels().get());
    let sample_rate = decoder.sample_rate().get();
    let samples: Vec<f32> = decoder.collect();
    if samples.is_empty() {
        return Err(format!("{} has no samples", path.display()));
    }
    Ok(Clip {
        channels,
        sample_rate,
        samples,
    })
}

/// In-place radix-2 FFT of `re`/`im` (length a power of two).
fn fft(re: &mut [f64], im: &mut [f64]) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let angle = -2.0 * PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (sin, cos) = (angle * k as f64).sin_cos();
                let (a, b) = (start + k, start + k + len / 2);
                let (tr, ti) = (re[b] * cos - im[b] * sin, re[b] * sin + im[b] * cos);
                re[b] = re[a] - tr;
                im[b] = im[a] - ti;
                re[a] += tr;
                im[a] += ti;
            }
        }
        len <<= 1;
    }
}

/// Magnitudes (dB) of `FFT_SIZE`-sample Hann windows centred on `columns` points.
fn spectrogram(mono: &[f32], columns: usize) -> Vec<Vec<f64>> {
    let window: Vec<f64> = (0..FFT_SIZE)
        .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / FFT_SIZE as f64).cos())
        .collect();
    (0..columns)
        .map(|column| {
            let centre = (column as f64 + 0.5) / columns as f64 * mono.len() as f64;
            let start = centre as isize - FFT_SIZE as isize / 2;
            let mut re: Vec<f64> = (0..FFT_SIZE)
                .map(|i| {
                    let index = start + i as isize;
                    let sample = usize::try_from(index)
                        .ok()
                        .and_then(|index| mono.get(index))
                        .copied()
                        .unwrap_or(0.0);
                    f64::from(sample) * window[i]
                })
                .collect();
            let mut im = vec![0.0; FFT_SIZE];
            fft(&mut re, &mut im);
            (0..FFT_SIZE / 2)
                .map(|bin| {
                    let magnitude =
                        (re[bin] * re[bin] + im[bin] * im[bin]).sqrt() / (FFT_SIZE as f64 / 4.0);
                    20.0 * magnitude.max(1e-6).log10()
                })
                .collect()
        })
        .collect()
}

/// Dark blue -> purple -> orange -> yellow for 0..1.
fn heat(t: f64) -> [u8; 3] {
    let stops = [
        (0.0, [8.0, 8.0, 24.0]),
        (0.35, [90.0, 20.0, 120.0]),
        (0.7, [230.0, 90.0, 40.0]),
        (1.0, [255.0, 240.0, 120.0]),
    ];
    let t = t.clamp(0.0, 1.0);
    for pair in stops.windows(2) {
        let ((t0, c0), (t1, c1)) = (pair[0], pair[1]);
        if t <= t1 {
            let f = (t - t0) / (t1 - t0);
            return [0, 1, 2].map(|i| (c0[i] + (c1[i] - c0[i]) * f) as u8);
        }
    }
    [255, 240, 120]
}

/// Waveform envelope on top (red where it clips), spectrogram below (log frequency,
/// 30 Hz to Nyquist, -90..0 dB).
fn picture(clip: &Clip, mono: &[f32]) -> Result<Vec<u8>, String> {
    let width = IMAGE_WIDTH;
    let height = WAVE_HEIGHT + SPECTRUM_HEIGHT;
    let mut image = image::RgbImage::from_pixel(width, height, image::Rgb([16, 16, 20]));

    let frames = clip.frames();
    let mid = WAVE_HEIGHT as f64 / 2.0;
    for x in 0..width {
        let from = x as usize * frames / width as usize;
        let to = ((x as usize + 1) * frames / width as usize)
            .max(from + 1)
            .min(frames);
        let (mut low, mut high) = (0.0f32, 0.0f32);
        for frame in from..to {
            for channel in 0..clip.channels {
                let sample = clip.samples[frame * clip.channels + channel];
                low = low.min(sample);
                high = high.max(sample);
            }
        }
        let clipped = f64::from(high) >= CLIP_LEVEL || f64::from(-low) >= CLIP_LEVEL;
        let colour = if clipped {
            [235, 60, 60]
        } else {
            [90, 200, 140]
        };
        let top = (mid - f64::from(high) * (mid - 1.0)).round() as u32;
        let bottom = (mid - f64::from(low) * (mid - 1.0)).round() as u32;
        for y in top.min(WAVE_HEIGHT - 1)..=bottom.min(WAVE_HEIGHT - 1) {
            image.put_pixel(x, y, image::Rgb(colour));
        }
        image.put_pixel(x, mid as u32, image::Rgb([60, 60, 70]));
    }

    let columns = spectrogram(mono, width as usize);
    let nyquist = f64::from(clip.sample_rate) / 2.0;
    let (low_hz, high_hz) = (30.0f64, nyquist);
    for (x, column) in columns.iter().enumerate() {
        for row in 0..SPECTRUM_HEIGHT {
            // Row 0 is the top (highest frequency).
            let t = 1.0 - (f64::from(row) + 0.5) / f64::from(SPECTRUM_HEIGHT);
            let hz = low_hz * (high_hz / low_hz).powf(t);
            let bin = ((hz / nyquist) * (FFT_SIZE / 2) as f64) as usize;
            let level = column[bin.min(FFT_SIZE / 2 - 1)];
            let colour = heat((level + 90.0) / 90.0);
            image.put_pixel(x as u32, WAVE_HEIGHT + row, image::Rgb(colour));
        }
    }

    let mut png = Vec::new();
    image::DynamicImage::ImageRgb8(image)
        .write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|err| format!("cannot encode the picture: {err}"))?;
    Ok(png)
}

/// Measurements of the sound at `path`, and optionally a PNG picture of it.
pub fn inspect(path: &Path, with_picture: bool) -> Result<(Value, Option<Vec<u8>>), String> {
    let clip = decode(path)?;
    let frames = clip.frames();
    let rate = f64::from(clip.sample_rate);
    let seconds = |frames: usize| round(frames as f64 / rate, 3);

    let peak = clip
        .samples
        .iter()
        .fold(0.0f64, |peak, s| peak.max(f64::from(s.abs())));
    let rms = (clip
        .samples
        .iter()
        .map(|s| f64::from(*s).powi(2))
        .sum::<f64>()
        / clip.samples.len() as f64)
        .sqrt();
    let clipped = clip
        .samples
        .iter()
        .filter(|s| f64::from(s.abs()) >= CLIP_LEVEL)
        .count();
    let dc_offset =
        clip.samples.iter().map(|s| f64::from(*s)).sum::<f64>() / clip.samples.len() as f64;
    let loud = |frame: &usize| {
        (0..clip.channels).any(|channel| {
            f64::from(clip.samples[frame * clip.channels + channel].abs()) > SILENCE_LEVEL
        })
    };
    let first_sound = (0..frames).find(loud);
    let last_sound = (0..frames).rev().find(loud);

    // Looping: how far the last frame is from the first (a click if large), compared
    // with the typical step between neighbouring frames.
    let mono = clip.mono();
    let seam = f64::from((mono[frames - 1] - mono[0]).abs());
    let typical_step = mono
        .windows(2)
        .map(|pair| f64::from((pair[1] - pair[0]).abs()))
        .sum::<f64>()
        / (frames.max(2) - 1) as f64;

    // Loudness over time, in tenths of the clip, to see the envelope at a glance.
    let envelope: Vec<Value> = (0..10)
        .map(|part| {
            let from = part * frames / 10;
            let to = ((part + 1) * frames / 10).max(from + 1).min(frames);
            let rms = (mono[from..to]
                .iter()
                .map(|s| f64::from(*s).powi(2))
                .sum::<f64>()
                / (to - from) as f64)
                .sqrt();
            db(rms)
        })
        .collect();

    let mut notes = Vec::new();
    if clipped > 0 {
        notes.push(format!("{clipped} samples clip; lower the gain"));
    }
    if peak < 0.1 {
        notes.push("very quiet (peak below -20 dBFS)".to_owned());
    }
    if first_sound.is_none() {
        notes.push("the whole file is silent".to_owned());
    }
    if dc_offset.abs() > 0.01 {
        notes.push("DC offset above 1%: the waveform is not centred".to_owned());
    }
    if seam > 0.05 && seam > typical_step * 8.0 {
        notes.push("if looped, the jump from the last sample to the first will click".to_owned());
    }
    if first_sound.is_some_and(|frame| frame as f64 / rate > 0.05) {
        notes.push("more than 50 ms of silence before the sound starts (late effect)".to_owned());
    }

    let report = json!({
        "path": path.display().to_string(),
        "duration_s": seconds(frames),
        "sample_rate": clip.sample_rate,
        "channels": clip.channels,
        "peak_dbfs": db(peak),
        "rms_dbfs": db(rms),
        "clipped_samples": clipped,
        "dc_offset": round(dc_offset, 4),
        "leading_silence_s": first_sound.map(seconds),
        "trailing_silence_s": last_sound.map(|frame| seconds(frames - 1 - frame)),
        "loop_seam": {
            "jump": round(seam, 4),
            "typical_step": round(typical_step, 4),
        },
        "envelope_rms_dbfs_by_tenth": envelope,
        "notes": notes,
    });
    let png = if with_picture {
        Some(picture(&clip, &mono)?)
    } else {
        None
    };
    Ok((report, png))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_wav(path: &Path, rate: u32, samples: &[f32]) {
        let mut bytes = Vec::new();
        let data_len = (samples.len() * 2) as u32;
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&rate.to_le_bytes());
        bytes.extend_from_slice(&(rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&data_len.to_le_bytes());
        for sample in samples {
            bytes.extend_from_slice(&((sample * 32767.0) as i16).to_le_bytes());
        }
        std::fs::write(path, bytes).unwrap();
    }

    #[test]
    fn measures_level_silence_clipping_and_seam() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tone.wav");
        let rate = 44100;
        // 0.1 s silence, then 0.4 s of a loud 440 Hz tone that clips, cut off at a peak.
        let mut samples = vec![0.0; 4410];
        samples.extend((0..17615).map(|i| {
            (1.3 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin())
                .clamp(-1.0, 1.0)
        }));
        write_wav(&path, rate, &samples);

        let (report, png) = inspect(&path, true).unwrap();
        assert_eq!(report["channels"], 1);
        assert!((report["duration_s"].as_f64().unwrap() - 0.5).abs() < 0.01);
        assert!(report["clipped_samples"].as_u64().unwrap() > 100);
        assert!((report["leading_silence_s"].as_f64().unwrap() - 0.1).abs() < 0.005);
        let notes = report["notes"].to_string();
        assert!(notes.contains("clip"), "{notes}");
        assert!(notes.contains("late effect"), "{notes}");
        assert!(notes.contains("click"), "{notes}");
        let png = png.unwrap();
        assert_eq!(&png[1..4], b"PNG");
    }

    #[test]
    fn fft_finds_a_pure_tone() {
        let mut re: Vec<f64> = (0..64)
            .map(|i| (2.0 * PI * 8.0 * i as f64 / 64.0).cos())
            .collect();
        let mut im = vec![0.0; 64];
        fft(&mut re, &mut im);
        let peak = (0..32)
            .max_by(|a, b| re[*a].hypot(im[*a]).total_cmp(&re[*b].hypot(im[*b])))
            .unwrap();
        assert_eq!(peak, 8);
    }
}
