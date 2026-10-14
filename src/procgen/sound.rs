//! Seeded sound-effect synthesis into mono PCM WAV files, without an audio device.

use serde::Deserialize;
use std::f64::consts::TAU;

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Preset {
    #[default]
    Tone,
    Blip,
    Laser,
    Explosion,
    Pickup,
    Jump,
    Hit,
    Powerup,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Waveform {
    Sine,
    Square,
    Saw,
    Triangle,
    Noise,
}

#[derive(Debug, Deserialize)]
#[serde(default)]
pub(crate) struct SoundOptions {
    pub preset: Preset,
    pub seed: u64,
    pub sample_rate: u32,
    pub duration: Option<f64>,
    pub frequency: Option<f64>,
    pub end_frequency: Option<f64>,
    pub waveform: Option<Waveform>,
    pub noise_mix: Option<f64>,
    pub attack: Option<f64>,
    pub release: Option<f64>,
    pub decay: Option<f64>,
    pub volume: f64,
    pub vibrato_hz: f64,
    pub vibrato_depth: f64,
    pub lowpass_hz: Option<f64>,
    pub duty_cycle: f64,
}

impl Default for SoundOptions {
    fn default() -> Self {
        Self {
            preset: Preset::Tone,
            seed: 0,
            sample_rate: 44100,
            duration: None,
            frequency: None,
            end_frequency: None,
            waveform: None,
            noise_mix: None,
            attack: None,
            release: None,
            decay: None,
            volume: 0.8,
            vibrato_hz: 0.0,
            vibrato_depth: 0.0,
            lowpass_hz: None,
            duty_cycle: 0.5,
        }
    }
}

struct Voice {
    duration: f64,
    start: f64,
    end: f64,
    waveform: Waveform,
    noise: f64,
    attack: f64,
    release: f64,
    decay: f64,
}

impl Preset {
    fn voice(self) -> Voice {
        let (duration, start, end, waveform, noise, decay) = match self {
            Self::Tone => (0.5, 440.0, 440.0, Waveform::Sine, 0.0, 0.0),
            Self::Blip => (0.08, 880.0, 440.0, Waveform::Square, 0.0, 3.0),
            Self::Laser => (0.25, 1800.0, 80.0, Waveform::Saw, 0.1, 3.0),
            Self::Explosion => (0.9, 100.0, 25.0, Waveform::Sine, 0.85, 5.0),
            Self::Pickup => (0.18, 880.0, 1760.0, Waveform::Sine, 0.0, 1.0),
            Self::Jump => (0.25, 180.0, 700.0, Waveform::Square, 0.0, 1.5),
            Self::Hit => (0.12, 160.0, 40.0, Waveform::Triangle, 0.6, 5.0),
            Self::Powerup => (0.6, 220.0, 1320.0, Waveform::Triangle, 0.0, 0.5),
        };
        Voice {
            duration,
            start,
            end,
            waveform,
            noise,
            attack: 0.003,
            release: 0.03,
            decay,
        }
    }
}

/// PolyBLEP softens saw/square discontinuities to reduce aliasing.
fn blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

fn unit(name: &str, value: f64, min: f64, max: f64) -> Result<(), String> {
    if !value.is_finite() || !(min..=max).contains(&value) {
        return Err(format!("{name} must be in {min}..={max}"));
    }
    Ok(())
}

pub(crate) struct SynthesizedSound {
    pub sample_rate: u32,
    pub samples: Vec<f64>,
}

impl SoundOptions {
    pub fn generate(&self) -> Result<SynthesizedSound, String> {
        if ![8000, 16000, 22050, 44100, 48000, 96000].contains(&self.sample_rate) {
            return Err("sample_rate must be 8000, 16000, 22050, 44100, 48000 or 96000".into());
        }
        let mut voice = self.preset.voice();
        voice.duration = self.duration.unwrap_or(voice.duration);
        voice.start = self.frequency.unwrap_or(voice.start);
        // A custom tone frequency stays constant unless an end frequency is specified.
        voice.end = self
            .end_frequency
            .unwrap_or(if matches!(self.preset, Preset::Tone) {
                voice.start
            } else {
                voice.end
            });
        voice.waveform = self.waveform.unwrap_or(voice.waveform);
        voice.noise = self.noise_mix.unwrap_or(voice.noise);
        voice.attack = self
            .attack
            .unwrap_or(voice.attack.min(voice.duration * 0.1));
        voice.release = self
            .release
            .unwrap_or(voice.release.min(voice.duration * 0.25));
        voice.decay = self.decay.unwrap_or(voice.decay);
        let rate = f64::from(self.sample_rate);
        unit("duration", voice.duration, 0.02, 10.0)?;
        unit("frequency", voice.start, 20.0, rate * 0.45)?;
        unit("end_frequency", voice.end, 20.0, rate * 0.45)?;
        unit("noise_mix", voice.noise, 0.0, 1.0)?;
        unit("attack", voice.attack, 0.0, voice.duration)?;
        unit("release", voice.release, 0.0, voice.duration)?;
        if voice.attack + voice.release > voice.duration {
            return Err("attack + release must not exceed duration".into());
        }
        unit("decay", voice.decay, 0.0, 20.0)?;
        unit("volume", self.volume, 0.0, 1.0)?;
        unit("duty_cycle", self.duty_cycle, 0.05, 0.95)?;
        unit("vibrato_hz", self.vibrato_hz, 0.0, 40.0)?;
        unit("vibrato_depth", self.vibrato_depth, 0.0, 0.5)?;
        if voice.start.max(voice.end) * (1.0 + self.vibrato_depth) > rate * 0.49 {
            return Err("frequency including vibrato must stay below Nyquist".into());
        }
        let cutoff = self.lowpass_hz.unwrap_or(rate * 0.4);
        unit("lowpass_hz", cutoff, 20.0, rate * 0.49)?;
        let coefficient = 1.0 - (-TAU * cutoff / rate).exp();
        let frames = (voice.duration * rate).round() as usize;
        let mut phase = 0.0;
        let mut filtered = 0.0;
        let mut samples = Vec::with_capacity(frames);
        for i in 0..frames {
            let t = i as f64 / (frames - 1) as f64;
            let seconds = i as f64 / rate;
            let frequency = voice.start
                * (voice.end / voice.start).powf(t)
                * (1.0 + self.vibrato_depth * (TAU * self.vibrato_hz * seconds).sin());
            let dt = frequency / rate;
            let noise = (super::texture::hash(self.seed.wrapping_add(i as u64)) >> 11) as f64
                / (1u64 << 53) as f64
                * 2.0
                - 1.0;
            let wave = match voice.waveform {
                Waveform::Sine => (TAU * phase).sin(),
                Waveform::Triangle => 1.0 - 4.0 * (phase - 0.5).abs(),
                Waveform::Saw => 2.0 * phase - 1.0 - blep(phase, dt),
                Waveform::Square => {
                    let raw = if phase < self.duty_cycle { 1.0 } else { -1.0 };
                    raw + blep(phase, dt) - blep((phase - self.duty_cycle).rem_euclid(1.0), dt)
                }
                Waveform::Noise => noise,
            };
            phase = (phase + dt).rem_euclid(1.0);
            filtered += coefficient * (wave * (1.0 - voice.noise) + noise * voice.noise - filtered);
            samples.push(filtered);
        }
        // Remove DC before the envelope so its endpoints remain exactly zero.
        let mean = samples.iter().sum::<f64>() / frames as f64;
        let attack = voice.attack.max(0.001);
        let release = voice.release.max(0.001);
        for (i, sample) in samples.iter_mut().enumerate() {
            let seconds = i as f64 / rate;
            let remaining = (frames - 1 - i) as f64 / rate;
            let envelope = (seconds / attack).min(1.0)
                * (remaining / release).min(1.0)
                * (-voice.decay * i as f64 / (frames - 1) as f64).exp();
            *sample = (*sample - mean) * envelope;
        }
        let peak = samples.iter().copied().map(f64::abs).fold(0.0, f64::max);
        if peak > 0.0 {
            // Leave one quantization step of headroom even at volume 1.
            let gain = self.volume * (32766.0 / 32767.0) / peak;
            for sample in &mut samples {
                *sample *= gain;
            }
        }
        Ok(SynthesizedSound {
            sample_rate: self.sample_rate,
            samples,
        })
    }
}

impl SynthesizedSound {
    /// Encode little-endian mono 16-bit PCM, understood by native and browser players.
    pub fn wav(&self) -> Vec<u8> {
        let length = self.samples.len() as u32 * 2;
        let mut bytes = Vec::with_capacity(length as usize + 44);
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + length).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&self.sample_rate.to_le_bytes());
        bytes.extend_from_slice(&(self.sample_rate * 2).to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&length.to_le_bytes());
        for sample in &self.samples {
            bytes.extend_from_slice(
                &((sample.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes(),
            );
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_are_audible_bounded_and_click_free_at_the_endpoints() {
        for preset in [
            Preset::Tone,
            Preset::Blip,
            Preset::Laser,
            Preset::Explosion,
            Preset::Pickup,
            Preset::Jump,
            Preset::Hit,
            Preset::Powerup,
        ] {
            let options = SoundOptions {
                preset,
                ..Default::default()
            };
            let sound = options.generate().unwrap();
            assert_eq!(sound.samples.first(), Some(&0.0));
            assert_eq!(sound.samples.last(), Some(&0.0));
            assert!(sound.samples.iter().all(|v| v.is_finite() && v.abs() < 1.0));
            let rms = (sound.samples.iter().map(|v| v * v).sum::<f64>()
                / sound.samples.len() as f64)
                .sqrt();
            assert!(rms > 0.05, "{preset:?}: rms {rms}");
            assert_eq!(sound.wav(), options.generate().unwrap().wav());
            let wav = sound.wav();
            assert_eq!(&wav[..4], b"RIFF");
            assert_eq!(&wav[8..16], b"WAVEfmt ");
            assert_eq!(wav.len(), 44 + sound.samples.len() * 2);
        }
    }

    #[test]
    fn tone_has_the_requested_frequency_and_noise_is_seeded() {
        let sound = SoundOptions {
            frequency: Some(1000.0),
            ..Default::default()
        }
        .generate()
        .unwrap();
        let crossings = sound
            .samples
            .windows(2)
            .filter(|p| p[0] < 0.0 && p[1] >= 0.0)
            .count();
        assert!((crossings as f64 - 500.0).abs() < 2.0);
        let a = SoundOptions {
            waveform: Some(Waveform::Noise),
            ..Default::default()
        }
        .generate()
        .unwrap();
        let b = SoundOptions {
            waveform: Some(Waveform::Noise),
            seed: 1,
            ..Default::default()
        }
        .generate()
        .unwrap();
        assert_ne!(a.wav(), b.wav());
    }

    #[test]
    fn invalid_synthesis_parameters_are_rejected() {
        assert!(SoundOptions {
            duration: Some(1000.0),
            ..Default::default()
        }
        .generate()
        .is_err());
        assert!(SoundOptions {
            frequency: Some(50000.0),
            ..Default::default()
        }
        .generate()
        .is_err());
        assert!(SoundOptions {
            volume: f64::NAN,
            ..Default::default()
        }
        .generate()
        .is_err());
        assert!(SoundOptions {
            attack: Some(0.4),
            release: Some(0.4),
            ..Default::default()
        }
        .generate()
        .is_err());
    }
}
