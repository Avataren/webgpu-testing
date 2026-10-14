//! Post-processing settings: what a [`PostProcessVolume`](crate::scene::PostProcessVolume)
//! stores and what the renderer consumes after volumes are blended at the camera.
//!
//! Every effect keeps a neutral value so two settings can be blended smoothly: an effect that
//! is off on one side fades in through its intensity instead of popping on.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

/// The full post-processing stack applied to the HDR scene.
///
/// # Examples
///
/// ```
/// use wgpu_cube::renderer::postprocess::{PostProcessSettings, Tonemapper};
///
/// let mut settings = PostProcessSettings::default();
/// settings.tonemapping = Tonemapper::AgX;
/// settings.exposure.compensation = 1.0;
/// let halfway = PostProcessSettings::default().blend(&settings, 0.5);
/// assert_eq!(halfway.exposure.compensation, 0.5);
/// ```
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PostProcessSettings {
    pub exposure: ExposureSettings,
    pub tonemapping: Tonemapper,
    pub color: ColorAdjustments,
    pub bloom: BloomSettings,
    pub ssao: SsaoSettings,
    pub vignette: VignetteSettings,
    pub chromatic_aberration: ChromaticAberrationSettings,
    pub film_grain: FilmGrainSettings,
    pub sharpen: SharpenSettings,
    pub anti_aliasing: AntiAliasing,
}

impl PostProcessSettings {
    /// Blend toward `other` by `t` (0 keeps `self`, 1 gives `other`). Numbers interpolate,
    /// modes switch halfway, and an effect enabled on only one side fades through its
    /// intensity.
    pub fn blend(&self, other: &Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        if t <= 0.0 {
            return self.clone();
        }
        if t >= 1.0 {
            return other.clone();
        }
        Self {
            exposure: self.exposure.blend(&other.exposure, t),
            tonemapping: pick(self.tonemapping, other.tonemapping, t),
            color: self.color.blend(&other.color, t),
            bloom: self.bloom.blend(&other.bloom, t),
            ssao: self.ssao.blend(&other.ssao, t),
            vignette: self.vignette.blend(&other.vignette, t),
            chromatic_aberration: self
                .chromatic_aberration
                .blend(&other.chromatic_aberration, t),
            film_grain: self.film_grain.blend(&other.film_grain, t),
            sharpen: self.sharpen.blend(&other.sharpen, t),
            anti_aliasing: pick(self.anti_aliasing, other.anti_aliasing, t),
        }
    }

    /// Settings with every adjustment at its neutral value and every effect off: what the
    /// scene looks like with no post-processing beyond tonemapping.
    pub fn neutral() -> Self {
        Self {
            bloom: BloomSettings {
                enabled: false,
                ..BloomSettings::default()
            },
            ssao: SsaoSettings {
                enabled: false,
                ..SsaoSettings::default()
            },
            anti_aliasing: AntiAliasing::None,
            ..Self::default()
        }
    }

    /// Clamp every value into the range the shaders expect.
    pub fn sanitized(mut self) -> Self {
        self.exposure = self.exposure.sanitized();
        self.color = self.color.sanitized();
        let bloom = &mut self.bloom;
        bloom.intensity = finite_or(bloom.intensity, 0.0).max(0.0);
        bloom.threshold = finite_or(bloom.threshold, 0.0).max(0.0);
        bloom.knee = finite_or(bloom.knee, 0.0).clamp(0.0, 1.0);
        bloom.scatter = finite_or(bloom.scatter, 0.7).clamp(0.0, 1.0);
        bloom.tint = bloom.tint.map(|c| finite_or(c, 1.0).max(0.0));
        let ssao = &mut self.ssao;
        ssao.radius = finite_or(ssao.radius, 0.2).max(0.001);
        ssao.bias = finite_or(ssao.bias, 0.0).max(0.0);
        ssao.intensity = finite_or(ssao.intensity, 0.0).max(0.0);
        ssao.power = finite_or(ssao.power, 1.0).max(0.01);
        let vignette = &mut self.vignette;
        vignette.intensity = finite_or(vignette.intensity, 0.0).clamp(0.0, 1.0);
        vignette.smoothness = finite_or(vignette.smoothness, 0.2).clamp(0.01, 1.0);
        vignette.roundness = finite_or(vignette.roundness, 1.0).clamp(0.0, 1.0);
        vignette.color = vignette.color.map(|c| finite_or(c, 0.0).clamp(0.0, 1.0));
        self.chromatic_aberration.intensity =
            finite_or(self.chromatic_aberration.intensity, 0.0).clamp(0.0, 1.0);
        let grain = &mut self.film_grain;
        grain.intensity = finite_or(grain.intensity, 0.0).clamp(0.0, 1.0);
        grain.response = finite_or(grain.response, 0.8).clamp(0.0, 1.0);
        self.sharpen.strength = finite_or(self.sharpen.strength, 0.0).clamp(0.0, 1.0);
        self
    }

    /// Whether the scene needs the SSAO passes this frame.
    pub fn ssao_active(&self) -> bool {
        self.ssao.enabled && self.ssao.intensity > 0.0
    }

    /// Whether the scene needs the bloom passes this frame.
    pub fn bloom_active(&self) -> bool {
        self.bloom.enabled && self.bloom.intensity > 0.0
    }

    /// Whether exposure follows the scene's brightness.
    pub fn auto_exposure_active(&self) -> bool {
        self.exposure.mode == ExposureMode::Auto
    }
}

/// How scene brightness is mapped before tonemapping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExposureMode {
    /// A fixed exposure set by `compensation`.
    #[default]
    Manual,
    /// Exposure adapts so the scene's average brightness lands on middle grey, like an eye
    /// or a camera adjusting to light.
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ExposureSettings {
    pub mode: ExposureMode,
    /// Exposure offset in stops (EV): +1 doubles the brightness. In auto mode this shifts
    /// the brightness the adaptation aims for.
    pub compensation: f32,
    /// Darkest average scene luminance auto exposure adapts to, in EV (log2 luminance).
    pub min_ev: f32,
    /// Brightest average scene luminance auto exposure adapts to, in EV.
    pub max_ev: f32,
    /// How fast auto exposure darkens when the scene gets brighter, in EV per second.
    pub speed_up: f32,
    /// How fast auto exposure brightens when the scene gets darker, in EV per second.
    pub speed_down: f32,
    /// Share of the darkest pixels auto exposure ignores, 0-1.
    pub low_percent: f32,
    /// Share of the brightest pixels auto exposure ignores, 0-1.
    pub high_percent: f32,
}

impl Default for ExposureSettings {
    fn default() -> Self {
        Self {
            mode: ExposureMode::Manual,
            compensation: 0.0,
            min_ev: -6.0,
            max_ev: 6.0,
            speed_up: 3.0,
            speed_down: 1.0,
            low_percent: 0.1,
            high_percent: 0.9,
        }
    }
}

impl ExposureSettings {
    fn blend(&self, other: &Self, t: f32) -> Self {
        Self {
            mode: pick(self.mode, other.mode, t),
            compensation: lerp(self.compensation, other.compensation, t),
            min_ev: lerp(self.min_ev, other.min_ev, t),
            max_ev: lerp(self.max_ev, other.max_ev, t),
            speed_up: lerp(self.speed_up, other.speed_up, t),
            speed_down: lerp(self.speed_down, other.speed_down, t),
            low_percent: lerp(self.low_percent, other.low_percent, t),
            high_percent: lerp(self.high_percent, other.high_percent, t),
        }
    }

    fn sanitized(mut self) -> Self {
        self.compensation = finite_or(self.compensation, 0.0).clamp(-16.0, 16.0);
        self.min_ev = finite_or(self.min_ev, -6.0).clamp(-16.0, 16.0);
        self.max_ev = finite_or(self.max_ev, 6.0).clamp(self.min_ev, 16.0);
        self.speed_up = finite_or(self.speed_up, 3.0).max(0.0);
        self.speed_down = finite_or(self.speed_down, 1.0).max(0.0);
        self.low_percent = finite_or(self.low_percent, 0.1).clamp(0.0, 0.99);
        self.high_percent = finite_or(self.high_percent, 0.9).clamp(self.low_percent, 1.0);
        self
    }

    /// The fixed exposure multiplier for manual mode.
    pub fn manual_multiplier(&self) -> f32 {
        2f32.powf(self.compensation)
    }
}

/// The curve that maps HDR scene colour to the display.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tonemapper {
    /// Clip at 1.0, as the engine did before it rendered in HDR.
    None,
    /// Simple Reinhard on luminance: soft, keeps hues, dulls bright colours.
    Reinhard,
    /// ACES filmic fit (Stephen Hill): contrasty, highlights desaturate toward white.
    Aces,
    /// AgX: smooth, natural highlight roll-off without hue skews.
    #[serde(rename = "agx")]
    AgX,
    /// Khronos PBR Neutral: compresses highlights and also subtracts a small "Fresnel"
    /// toe (up to 0.04) from every colour, which suits physically based content but
    /// noticeably darkens dark scenes.
    Neutral,
    /// Leaves everything below 0.76 exactly as authored and rolls highlights off smoothly
    /// toward white (Khronos PBR Neutral's shoulder without its toe). Scenes made before
    /// the HDR pipeline look the same, minus the hard clipping.
    #[default]
    Rolloff,
}

impl Tonemapper {
    pub const ALL: [Tonemapper; 6] = [
        Tonemapper::Rolloff,
        Tonemapper::Neutral,
        Tonemapper::Aces,
        Tonemapper::AgX,
        Tonemapper::Reinhard,
        Tonemapper::None,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tonemapper::None => "None (clip)",
            Tonemapper::Reinhard => "Reinhard",
            Tonemapper::Aces => "ACES",
            Tonemapper::AgX => "AgX",
            Tonemapper::Neutral => "Khronos PBR Neutral",
            Tonemapper::Rolloff => "Highlight roll-off",
        }
    }

    pub(crate) fn shader_index(self) -> f32 {
        match self {
            Tonemapper::None => 0.0,
            Tonemapper::Reinhard => 1.0,
            Tonemapper::Aces => 2.0,
            Tonemapper::AgX => 3.0,
            Tonemapper::Neutral => 4.0,
            Tonemapper::Rolloff => 5.0,
        }
    }
}

/// Colour correction applied in HDR before tonemapping (white balance, filter, hue,
/// saturation, contrast) and to the display image after it (lift, gamma, gain).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorAdjustments {
    /// Exposure-independent brightness change in stops, applied after exposure.
    pub post_exposure: f32,
    /// White balance: negative is cooler (bluer), positive warmer. -100..100.
    pub temperature: f32,
    /// White balance: negative toward green, positive toward magenta. -100..100.
    pub tint: f32,
    /// Multiplies the image by this colour (white leaves it unchanged).
    pub color_filter: [f32; 3],
    /// Rotates every hue by this many degrees.
    pub hue_shift: f32,
    /// 1 leaves colours as they are, 0 is greyscale, 2 doubles saturation.
    pub saturation: f32,
    /// Boosts the saturation of dull colours more than already-saturated ones. -1..1.
    pub vibrance: f32,
    /// 1 leaves contrast as is. Applied around middle grey in log space.
    pub contrast: f32,
    /// Shadows: tint colour (white is neutral) in rgb and an offset in w.
    pub lift: [f32; 4],
    /// Midtones: tint colour (white is neutral) in rgb and an offset in w.
    pub gamma: [f32; 4],
    /// Highlights: tint colour (white is neutral) in rgb and an offset in w.
    pub gain: [f32; 4],
}

impl Default for ColorAdjustments {
    fn default() -> Self {
        Self {
            post_exposure: 0.0,
            temperature: 0.0,
            tint: 0.0,
            color_filter: [1.0; 3],
            hue_shift: 0.0,
            saturation: 1.0,
            vibrance: 0.0,
            contrast: 1.0,
            lift: TRACKBALL_NEUTRAL,
            gamma: TRACKBALL_NEUTRAL,
            gain: TRACKBALL_NEUTRAL,
        }
    }
}

/// A lift/gamma/gain control with no effect.
pub const TRACKBALL_NEUTRAL: [f32; 4] = [1.0, 1.0, 1.0, 0.0];

impl ColorAdjustments {
    fn blend(&self, other: &Self, t: f32) -> Self {
        Self {
            post_exposure: lerp(self.post_exposure, other.post_exposure, t),
            temperature: lerp(self.temperature, other.temperature, t),
            tint: lerp(self.tint, other.tint, t),
            color_filter: lerp_array(self.color_filter, other.color_filter, t),
            hue_shift: lerp(self.hue_shift, other.hue_shift, t),
            saturation: lerp(self.saturation, other.saturation, t),
            vibrance: lerp(self.vibrance, other.vibrance, t),
            contrast: lerp(self.contrast, other.contrast, t),
            lift: lerp_array(self.lift, other.lift, t),
            gamma: lerp_array(self.gamma, other.gamma, t),
            gain: lerp_array(self.gain, other.gain, t),
        }
    }

    fn sanitized(mut self) -> Self {
        self.post_exposure = finite_or(self.post_exposure, 0.0).clamp(-16.0, 16.0);
        self.temperature = finite_or(self.temperature, 0.0).clamp(-100.0, 100.0);
        self.tint = finite_or(self.tint, 0.0).clamp(-100.0, 100.0);
        self.color_filter = self.color_filter.map(|c| finite_or(c, 1.0).max(0.0));
        self.hue_shift = finite_or(self.hue_shift, 0.0).clamp(-180.0, 180.0);
        self.saturation = finite_or(self.saturation, 1.0).clamp(0.0, 4.0);
        self.vibrance = finite_or(self.vibrance, 0.0).clamp(-1.0, 1.0);
        self.contrast = finite_or(self.contrast, 1.0).clamp(0.0, 4.0);
        for control in [&mut self.lift, &mut self.gamma, &mut self.gain] {
            for (index, value) in control.iter_mut().enumerate() {
                let fallback = TRACKBALL_NEUTRAL[index];
                *value = finite_or(*value, fallback).clamp(-2.0, 2.0);
            }
        }
        self
    }

    /// White-balance multipliers in LMS cone space for `temperature` and `tint`.
    pub(crate) fn white_balance_lms(&self) -> [f32; 3] {
        white_balance_coefficients(self.temperature, self.tint)
    }

    /// Lift, gamma and gain reduced to per-channel shader values: an offset for the
    /// shadows, an inverse exponent for the midtones and a multiplier for the highlights.
    pub(crate) fn lift_gamma_gain(&self) -> ([f32; 3], [f32; 3], [f32; 3]) {
        let prepare = |control: [f32; 4], base: f32| {
            let luma = control[0] * 0.2126 + control[1] * 0.7152 + control[2] * 0.0722;
            [0, 1, 2].map(|i| control[i] - luma + control[3] + base)
        };
        let lift = prepare(self.lift, 0.0);
        let gamma = prepare(self.gamma, 1.0).map(|g| 1.0 / g.max(1e-3));
        let gain = prepare(self.gain, 1.0);
        (lift, gamma, gain)
    }
}

/// Glow around bright parts of the image.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BloomSettings {
    pub enabled: bool,
    /// How much of the blurred highlights is added to the image.
    pub intensity: f32,
    /// Brightness (in HDR scene units, before exposure) where bloom starts. 0 makes
    /// everything glow a little, which is the physically based look.
    pub threshold: f32,
    /// Softens the threshold over this fraction of it, 0-1.
    pub knee: f32,
    /// How far the glow spreads, 0-1.
    pub scatter: f32,
    /// Colours the glow.
    pub tint: [f32; 3],
}

impl Default for BloomSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            intensity: 1.0,
            threshold: 0.9,
            knee: 0.5,
            scatter: 0.7,
            tint: [1.0; 3],
        }
    }
}

impl BloomSettings {
    /// The old bloom added all five upsampled mips at `scatter^level` strength, while
    /// the current chain is normalized, so the old glow's energy becomes the intensity
    /// (scaled to match side by side), the knee becomes a fraction of the threshold and
    /// the spread takes the scatter that gives the same falloff.
    fn from_legacy(enabled: bool, legacy: &LegacyBloomSettings) -> Self {
        let scatter = legacy.scatter.clamp(0.0, 1.0);
        let energy: f32 = (0..5).map(|level| scatter.powi(level)).sum();
        let threshold = legacy.threshold.max(0.0);
        Self {
            enabled,
            intensity: energy * 1.33,
            threshold,
            knee: if threshold > 0.0 {
                (legacy.knee / threshold).clamp(0.0, 1.0)
            } else {
                0.5
            },
            scatter: 0.75,
            tint: [1.0; 3],
        }
    }

    fn blend(&self, other: &Self, t: f32) -> Self {
        let (from, to, enabled) = fade_pair(self, other, self.enabled, other.enabled, |s| {
            BloomSettings {
                intensity: 0.0,
                ..*s
            }
        });
        Self {
            enabled,
            intensity: lerp(from.intensity, to.intensity, t),
            threshold: lerp(from.threshold, to.threshold, t),
            knee: lerp(from.knee, to.knee, t),
            scatter: lerp(from.scatter, to.scatter, t),
            tint: lerp_array(from.tint, to.tint, t),
        }
    }
}

/// Screen-space ambient occlusion: darkens creases and contact areas.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SsaoSettings {
    pub enabled: bool,
    /// Sampling radius in world units.
    pub radius: f32,
    /// Depth bias that keeps flat surfaces from shading themselves.
    pub bias: f32,
    /// Strength of the darkening.
    pub intensity: f32,
    /// Exponent applied to the occlusion; higher gives darker, tighter contact shadows.
    pub power: f32,
}

impl Default for SsaoSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            radius: 0.2,
            bias: 0.05,
            intensity: 0.75,
            power: 1.25,
        }
    }
}

impl SsaoSettings {
    fn blend(&self, other: &Self, t: f32) -> Self {
        let (from, to, enabled) =
            fade_pair(self, other, self.enabled, other.enabled, |s| SsaoSettings {
                intensity: 0.0,
                ..*s
            });
        Self {
            enabled,
            radius: lerp(from.radius, to.radius, t),
            bias: lerp(from.bias, to.bias, t),
            intensity: lerp(from.intensity, to.intensity, t),
            power: lerp(from.power, to.power, t),
        }
    }
}

/// Darkens (or tints) the edges of the frame.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VignetteSettings {
    pub enabled: bool,
    /// How dark the edges get, 0-1.
    pub intensity: f32,
    /// Width of the falloff, 0-1.
    pub smoothness: f32,
    /// 1 gives round corners, 0 square ones.
    pub roundness: f32,
    /// Keep the vignette circular on screen instead of following the frame's shape.
    pub rounded: bool,
    /// Colour of the edges.
    pub color: [f32; 3],
    /// Centre in screen UV (0.5, 0.5 is the middle).
    pub center: [f32; 2],
}

impl Default for VignetteSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            intensity: 0.3,
            smoothness: 0.2,
            roundness: 1.0,
            rounded: false,
            color: [0.0; 3],
            center: [0.5, 0.5],
        }
    }
}

impl VignetteSettings {
    fn blend(&self, other: &Self, t: f32) -> Self {
        let (from, to, enabled) = fade_pair(self, other, self.enabled, other.enabled, |s| {
            VignetteSettings {
                intensity: 0.0,
                ..*s
            }
        });
        Self {
            enabled,
            intensity: lerp(from.intensity, to.intensity, t),
            smoothness: lerp(from.smoothness, to.smoothness, t),
            roundness: lerp(from.roundness, to.roundness, t),
            rounded: pick(from.rounded, to.rounded, t),
            color: lerp_array(from.color, to.color, t),
            center: lerp_array(from.center, to.center, t),
        }
    }
}

/// Splits colour channels toward the frame edges, like a cheap lens.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChromaticAberrationSettings {
    pub enabled: bool,
    /// Strength of the fringe, 0-1.
    pub intensity: f32,
}

impl Default for ChromaticAberrationSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            intensity: 0.5,
        }
    }
}

impl ChromaticAberrationSettings {
    fn blend(&self, other: &Self, t: f32) -> Self {
        let (from, to, enabled) = fade_pair(self, other, self.enabled, other.enabled, |s| {
            ChromaticAberrationSettings {
                intensity: 0.0,
                ..*s
            }
        });
        Self {
            enabled,
            intensity: lerp(from.intensity, to.intensity, t),
        }
    }
}

/// Animated film grain.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilmGrainSettings {
    pub enabled: bool,
    /// Grain strength, 0-1.
    pub intensity: f32,
    /// How much grain fades out in bright areas, 0-1.
    pub response: f32,
}

impl Default for FilmGrainSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            intensity: 0.2,
            response: 0.8,
        }
    }
}

impl FilmGrainSettings {
    fn blend(&self, other: &Self, t: f32) -> Self {
        let (from, to, enabled) = fade_pair(self, other, self.enabled, other.enabled, |s| {
            FilmGrainSettings {
                intensity: 0.0,
                ..*s
            }
        });
        Self {
            enabled,
            intensity: lerp(from.intensity, to.intensity, t),
            response: lerp(from.response, to.response, t),
        }
    }
}

/// Contrast-adaptive sharpening (AMD FidelityFX CAS style).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SharpenSettings {
    pub enabled: bool,
    /// Sharpening strength, 0-1.
    pub strength: f32,
}

impl Default for SharpenSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            strength: 0.4,
        }
    }
}

impl SharpenSettings {
    fn blend(&self, other: &Self, t: f32) -> Self {
        let (from, to, enabled) = fade_pair(self, other, self.enabled, other.enabled, |s| {
            SharpenSettings {
                strength: 0.0,
                ..*s
            }
        });
        Self {
            enabled,
            strength: lerp(from.strength, to.strength, t),
        }
    }
}

/// Post-process anti-aliasing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AntiAliasing {
    None,
    /// Fast approximate anti-aliasing on the tonemapped image.
    #[default]
    Fxaa,
}

/// Which post effects ran and how they were tuned, as projects stored them before
/// post-processing moved into volumes. Only read to migrate old projects.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PostProcessEffects {
    pub ssao: bool,
    pub bloom: bool,
    pub fxaa: bool,
    pub ssao_settings: LegacySsaoSettings,
    pub bloom_settings: LegacyBloomSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LegacySsaoSettings {
    pub radius: f32,
    pub bias: f32,
    pub intensity: f32,
    pub power: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LegacyBloomSettings {
    pub threshold: f32,
    pub knee: f32,
    pub scatter: f32,
}

impl Default for PostProcessEffects {
    fn default() -> Self {
        Self {
            ssao: true,
            bloom: true,
            fxaa: true,
            ssao_settings: LegacySsaoSettings::default(),
            bloom_settings: LegacyBloomSettings::default(),
        }
    }
}

impl Default for LegacySsaoSettings {
    fn default() -> Self {
        let current = SsaoSettings::default();
        Self {
            radius: current.radius,
            bias: current.bias,
            intensity: current.intensity,
            power: current.power,
        }
    }
}

impl Default for LegacyBloomSettings {
    fn default() -> Self {
        Self {
            threshold: 0.8,
            knee: 0.4,
            scatter: 0.95,
        }
    }
}

/// The colour grading environments carried before it moved into volumes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct LegacyColorGrading {
    pub exposure: f32,
    pub saturation: f32,
    pub contrast: f32,
}

impl Default for LegacyColorGrading {
    fn default() -> Self {
        Self {
            exposure: 1.0,
            saturation: 1.0,
            contrast: 1.0,
        }
    }
}

impl PostProcessSettings {
    /// Settings that reproduce an old project's effects and environment grading.
    pub fn from_legacy(
        effects: Option<&PostProcessEffects>,
        grading: Option<&LegacyColorGrading>,
    ) -> Self {
        let mut settings = Self::default();
        if let Some(effects) = effects {
            settings.ssao = SsaoSettings {
                enabled: effects.ssao,
                radius: effects.ssao_settings.radius,
                bias: effects.ssao_settings.bias,
                intensity: effects.ssao_settings.intensity,
                power: effects.ssao_settings.power,
            };
            settings.bloom = BloomSettings::from_legacy(effects.bloom, &effects.bloom_settings);
            settings.anti_aliasing = if effects.fxaa {
                AntiAliasing::Fxaa
            } else {
                AntiAliasing::None
            };
        }
        if let Some(grading) = grading {
            settings.exposure.compensation = grading.exposure.max(1e-4).log2().clamp(-16.0, 16.0);
            settings.color.saturation = grading.saturation;
            // The old contrast pivoted at 0.5 in linear light: c * x - 0.5 * (c - 1), which
            // crushes dark values to black. Lift and gain reproduce that exactly
            // (x * gain + lift * (1 - x)) wherever the tonemapper leaves colours alone.
            let lift = -0.5 * (grading.contrast - 1.0);
            settings.color.lift[3] = lift;
            settings.color.gain[3] = grading.contrast + lift - 1.0;
        }
        settings.sanitized()
    }
}

/// Fields that make an effect fade in or out when its `enabled` flag differs between
/// two blended settings.
const FADE_FIELDS: [&str; 2] = ["intensity", "strength"];

impl PostProcessSettings {
    /// Every setting's path, such as `"bloom.intensity"` or `"tonemapping"`, in
    /// declaration order. Volumes list the paths they override.
    pub fn setting_paths() -> Vec<String> {
        let value = serde_json::to_value(Self::default()).unwrap_or(Value::Null);
        let mut paths = Vec::new();
        collect_leaf_paths(&value, String::new(), &mut paths);
        paths
    }

    /// The setting paths a partial settings value (such as a JSON patch) contains.
    pub fn paths_in(value: &Value) -> Vec<String> {
        let mut paths = Vec::new();
        collect_leaf_paths(value, String::new(), &mut paths);
        paths
    }

    /// Blend toward `other` by `t`, but only the settings named in `overrides`: leaf paths
    /// (`"bloom.intensity"`) or whole groups (`"bloom"`). Everything else keeps `self`.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::collections::BTreeSet;
    /// use wgpu_cube::renderer::postprocess::PostProcessSettings;
    ///
    /// let base = PostProcessSettings::default();
    /// let mut volume = base.clone();
    /// volume.exposure.compensation = 2.0;
    /// volume.color.saturation = 0.0;
    /// let only: BTreeSet<String> = ["exposure.compensation".to_string()].into();
    /// let result = base.blend_overridden(&volume, 1.0, &only);
    /// assert_eq!(result.exposure.compensation, 2.0);
    /// assert_eq!(result.color.saturation, base.color.saturation);
    /// ```
    pub fn blend_overridden(&self, other: &Self, t: f32, overrides: &BTreeSet<String>) -> Self {
        if overrides.is_empty() || t <= 0.0 {
            return self.clone();
        }
        let blended = self.blend(other, t);
        let (Ok(original), Ok(blended), Ok(target)) = (
            serde_json::to_value(self),
            serde_json::to_value(&blended),
            serde_json::to_value(other),
        ) else {
            return self.clone();
        };
        let mut base = original.clone();
        let mut leaves = Vec::new();
        collect_leaf_paths(&original, String::new(), &mut leaves);
        for path in leaves {
            if !setting_overridden(&path, overrides) {
                continue;
            }
            copy_path(&blended, &mut base, &path);
            // Switching an effect on or off fades it through its strength, so that
            // comes along even when only the switch is overridden.
            if let Some(group) = path.strip_suffix(".enabled") {
                let pointer = json_pointer(&path);
                if original.pointer(&pointer) != target.pointer(&pointer) {
                    for field in FADE_FIELDS {
                        copy_path(&blended, &mut base, &format!("{group}.{field}"));
                    }
                }
            }
        }
        serde_json::from_value(base).unwrap_or_else(|_| self.clone())
    }
}

/// Whether `path` (a leaf such as `"bloom.intensity"`) is covered by `overrides`, which
/// may name it or any of its parent groups.
pub fn setting_overridden(path: &str, overrides: &BTreeSet<String>) -> bool {
    if overrides.contains(path) {
        return true;
    }
    path.match_indices('.')
        .any(|(index, _)| overrides.contains(&path[..index]))
}

fn collect_leaf_paths(value: &Value, prefix: String, out: &mut Vec<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = if prefix.is_empty() {
                    key.clone()
                } else {
                    format!("{prefix}.{key}")
                };
                collect_leaf_paths(child, path, out);
            }
        }
        _ if !prefix.is_empty() => out.push(prefix),
        _ => {}
    }
}

fn json_pointer(path: &str) -> String {
    format!("/{}", path.replace('.', "/"))
}

fn copy_path(from: &Value, to: &mut Value, path: &str) {
    let pointer = json_pointer(path);
    if let (Some(source), Some(slot)) = (from.pointer(&pointer), to.pointer_mut(&pointer)) {
        *slot = source.clone();
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn lerp_array<const N: usize>(a: [f32; N], b: [f32; N], t: f32) -> [f32; N] {
    std::array::from_fn(|i| lerp(a[i], b[i], t))
}

fn pick<T>(a: T, b: T, t: f32) -> T {
    if t >= 0.5 {
        b
    } else {
        a
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

/// The two endpoints to interpolate for an effect that may be off on either side: a
/// disabled side is replaced by the enabled side's settings at zero strength, so the
/// effect fades instead of switching.
fn fade_pair<T: Copy>(
    a: &T,
    b: &T,
    a_enabled: bool,
    b_enabled: bool,
    silenced: impl Fn(&T) -> T,
) -> (T, T, bool) {
    match (a_enabled, b_enabled) {
        (true, true) => (*a, *b, true),
        (false, true) => (silenced(b), *b, true),
        (true, false) => (*a, silenced(a), true),
        (false, false) => (*a, *b, false),
    }
}

/// LMS multipliers for a white balance shift (the approach Unity and Unreal use: move the
/// D65 white point along the Planckian locus by `temperature` and across it by `tint`).
fn white_balance_coefficients(temperature: f32, tint: f32) -> [f32; 3] {
    let t1 = temperature / 65.0;
    let t2 = tint / 65.0;

    // Reference white point chromaticity (CIE xy).
    let x = 0.31271 - t1 * if t1 < 0.0 { 0.1 } else { 0.05 };
    let standard_illuminant_y = 2.87 * x - 3.0 * x * x - 0.275_095_08;
    let y = standard_illuminant_y + t2 * 0.05;

    let w1 = cie_xy_to_lms(0.31271, 0.32902);
    let w2 = cie_xy_to_lms(x, y);
    [w1[0] / w2[0], w1[1] / w2[1], w1[2] / w2[2]]
}

fn cie_xy_to_lms(x: f32, y: f32) -> [f32; 3] {
    let big_y = 1.0;
    let big_x = big_y * x / y;
    let big_z = big_y * (1.0 - x - y) / y;
    let l = 0.7328 * big_x + 0.4296 * big_y - 0.1624 * big_z;
    let m = -0.7036 * big_x + 1.6975 * big_y + 0.0061 * big_z;
    let s = 0.0030 * big_x + 0.0136 * big_y + 0.9834 * big_z;
    [l, m, s]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blend_endpoints_return_the_inputs() {
        let a = PostProcessSettings::default();
        let mut b = PostProcessSettings::default();
        b.exposure.compensation = 2.0;
        b.tonemapping = Tonemapper::Aces;
        assert_eq!(a.blend(&b, 0.0), a);
        assert_eq!(a.blend(&b, 1.0), b);
    }

    #[test]
    fn blend_interpolates_numbers_and_switches_modes_halfway() {
        let a = PostProcessSettings::default();
        let mut b = a.clone();
        b.exposure.compensation = 2.0;
        b.tonemapping = Tonemapper::Aces;
        let early = a.blend(&b, 0.25);
        assert_eq!(early.exposure.compensation, 0.5);
        assert_eq!(early.tonemapping, a.tonemapping);
        assert_eq!(a.blend(&b, 0.75).tonemapping, Tonemapper::Aces);
    }

    #[test]
    fn effect_enabled_on_one_side_fades_through_intensity() {
        let mut a = PostProcessSettings::default();
        a.vignette.enabled = false;
        let mut b = a.clone();
        b.vignette.enabled = true;
        b.vignette.intensity = 0.8;
        let blended = a.blend(&b, 0.5);
        assert!(blended.vignette.enabled);
        assert!((blended.vignette.intensity - 0.4).abs() < 1e-6);

        let fading_out = b.blend(&a, 0.25);
        assert!(fading_out.vignette.enabled);
        assert!((fading_out.vignette.intensity - 0.6).abs() < 1e-6);
    }

    #[test]
    fn missing_fields_take_defaults() {
        let settings: PostProcessSettings =
            serde_json::from_str(r#"{"bloom": {"intensity": 2.0}}"#).unwrap();
        assert_eq!(settings.bloom.intensity, 2.0);
        assert_eq!(settings.bloom.threshold, BloomSettings::default().threshold);
        assert_eq!(settings.tonemapping, Tonemapper::Rolloff);
    }

    #[test]
    fn tonemapper_names_round_trip() {
        for mapper in Tonemapper::ALL {
            let text = serde_json::to_string(&mapper).unwrap();
            let back: Tonemapper = serde_json::from_str(&text).unwrap();
            assert_eq!(back, mapper);
        }
        assert_eq!(serde_json::to_string(&Tonemapper::AgX).unwrap(), "\"agx\"");
    }

    #[test]
    fn neutral_white_balance_is_identity() {
        let lms = ColorAdjustments::default().white_balance_lms();
        for value in lms {
            assert!((value - 1.0).abs() < 1e-3, "{lms:?}");
        }
        let warm = white_balance_coefficients(50.0, 0.0);
        assert!(warm[2] < warm[0], "warming should cut blue cones: {warm:?}");
    }

    #[test]
    fn neutral_lift_gamma_gain_do_nothing() {
        let (lift, gamma, gain) = ColorAdjustments::default().lift_gamma_gain();
        assert_eq!(lift, [0.0; 3]);
        for value in gamma.into_iter().chain(gain) {
            assert!((value - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn legacy_settings_convert() {
        let effects = PostProcessEffects {
            ssao: false,
            fxaa: false,
            ..PostProcessEffects::default()
        };
        let grading = LegacyColorGrading {
            exposure: 2.0,
            saturation: 1.1,
            contrast: 1.05,
        };
        let settings = PostProcessSettings::from_legacy(Some(&effects), Some(&grading));
        assert!(!settings.ssao.enabled);
        assert_eq!(settings.anti_aliasing, AntiAliasing::None);
        assert!((settings.exposure.compensation - 1.0).abs() < 1e-6);
        assert_eq!(settings.color.saturation, 1.1);
        // Contrast becomes lift/gain matching the old linear-pivot formula.
        let (lift, _, gain) = settings.color.lift_gamma_gain();
        for x in [0.02f32, 0.2, 0.6] {
            let old = 1.05 * x - 0.5 * 0.05;
            let new = x * gain[0] + lift[0] * (1.0 - x);
            assert!((old - new).abs() < 1e-5, "{x}: {old} vs {new}");
        }
        assert_eq!(settings.color.contrast, 1.0);
        assert_eq!(settings.bloom.threshold, 0.8);
        assert!((settings.bloom.knee - 0.5).abs() < 1e-6);
        assert!((settings.bloom.intensity - 6.0).abs() < 0.05);
    }

    #[test]
    fn overrides_blend_only_named_settings() {
        let base = PostProcessSettings::default();
        let mut volume = base.clone();
        volume.exposure.compensation = 2.0;
        volume.bloom.threshold = 3.0;
        volume.bloom.scatter = 0.1;
        volume.tonemapping = Tonemapper::Aces;

        let group: BTreeSet<String> = ["bloom".to_string()].into();
        let result = base.blend_overridden(&volume, 0.5, &group);
        assert!((result.bloom.threshold - (base.bloom.threshold + 3.0) * 0.5).abs() < 1e-5);
        assert_eq!(result.exposure.compensation, 0.0);
        assert_eq!(result.tonemapping, base.tonemapping);

        let leaves: BTreeSet<String> =
            ["tonemapping".to_string(), "bloom.scatter".to_string()].into();
        let result = base.blend_overridden(&volume, 1.0, &leaves);
        assert_eq!(result.tonemapping, Tonemapper::Aces);
        assert_eq!(result.bloom.scatter, 0.1);
        assert_eq!(result.bloom.threshold, base.bloom.threshold);

        assert_eq!(base.blend_overridden(&volume, 1.0, &BTreeSet::new()), base);
    }

    #[test]
    fn overriding_enabled_fades_the_effect_in() {
        let base = PostProcessSettings::default();
        let mut volume = base.clone();
        volume.vignette.enabled = true;
        volume.vignette.intensity = 0.8;
        let only: BTreeSet<String> = ["vignette.enabled".to_string()].into();
        let half = base.blend_overridden(&volume, 0.5, &only);
        assert!(half.vignette.enabled);
        assert!((half.vignette.intensity - 0.4).abs() < 1e-6);
    }

    #[test]
    fn setting_paths_cover_every_leaf() {
        let paths = PostProcessSettings::setting_paths();
        assert!(paths.contains(&"bloom.intensity".to_string()));
        assert!(paths.contains(&"tonemapping".to_string()));
        assert!(paths.contains(&"color.lift".to_string()));
        assert!(setting_overridden(
            "color.lift",
            &["color".to_string()].into()
        ));
        assert!(!setting_overridden(
            "colorful",
            &["color".to_string()].into()
        ));
    }

    #[test]
    fn sanitized_repairs_bad_values() {
        let mut settings = PostProcessSettings::default();
        settings.bloom.intensity = f32::NAN;
        settings.exposure.min_ev = 4.0;
        settings.exposure.max_ev = -4.0;
        let fixed = settings.sanitized();
        assert_eq!(fixed.bloom.intensity, 0.0);
        assert!(fixed.exposure.max_ev >= fixed.exposure.min_ev);
    }
}
