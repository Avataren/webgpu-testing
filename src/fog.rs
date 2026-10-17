//! Fog: an option of the scene's environment ([`Fog`], off unless a scene enables it).
//!
//! The fog is a layer of height fog: as thick as `density` up to `height`, thinning above
//! it. Everywhere it takes light away from what lies behind it and scatters in light of
//! its own: the sky's and the flat ambient light (the light probes' inside a probe
//! volume), the directional lights' (the sun and the moon with the procedural sky) and,
//! near the camera, every point and spot light's. Near the camera the fog is volumetric
//! (the renderer gathers it in a volume of froxels that follows the view, see
//! `renderer/fog.rs`): its light is shadowed, so sunlight falls through gaps in shafts and
//! lamps glow in halos; farther away, and with the graphics setting `volumetric_fog` off,
//! it is lit without shadows.

use serde::{Deserialize, Serialize};

/// A scene's fog (part of the environment; see the module).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Fog {
    pub enabled: bool,
    /// How thick the fog is at and below `height`: the share of light it takes away per
    /// meter (extinction, 1/m). Light fades to 37% over 1 / density meters.
    pub density: f32,
    /// Height (world y, m) at and below which the fog has its full density.
    pub height: f32,
    /// Above `height` the fog thins out, to 37% every this many meters (0: as thick at
    /// every height).
    pub height_falloff: f32,
    /// The share of light the fog scatters rather than absorbs, per channel (linear).
    pub albedo: [f32; 3],
    /// How much the fog scatters light forward rather than back (Henyey-Greenstein g,
    /// -0.9 to 0.9): higher makes brighter halos round lights the camera looks toward.
    pub anisotropy: f32,
    /// Light the fog gives off by itself: the radiance it takes on where it is thick.
    pub emission: [f32; 3],
    /// Scales the ambient light the fog scatters (the sky's, the flat ambient, the light
    /// probes').
    pub ambient: f32,
    /// Scales the light of the directional, point and spot lights the fog scatters.
    pub light: f32,
    /// How far from the camera the fog is volumetric (meters of view depth): shadowed
    /// sunlight, halos round point and spot lights. Past it the fog is lit by the sky and
    /// the directional lights without shadows.
    pub volumetric_distance: f32,
    /// Fog farther away than this (m) adds nothing more, so thin fog does not hide the
    /// sky at the horizon entirely.
    pub max_distance: f32,
}

impl Fog {
    pub const MAX_DENSITY: f32 = 10.0;
    pub const MAX_ANISOTROPY: f32 = 0.9;
    pub const MIN_VOLUMETRIC_DISTANCE: f32 = 4.0;
    pub const MAX_VOLUMETRIC_DISTANCE: f32 = 300.0;

    /// The settings with every value in range (after loading or a remote change).
    pub fn sanitized(&self) -> Self {
        let finite = |value: f32, fallback: f32| if value.is_finite() { value } else { fallback };
        let color = |value: [f32; 3], fallback: f32| value.map(|c| finite(c, fallback).max(0.0));
        let defaults = Self::default();
        Self {
            enabled: self.enabled,
            density: finite(self.density, defaults.density).clamp(0.0, Self::MAX_DENSITY),
            height: finite(self.height, defaults.height),
            height_falloff: finite(self.height_falloff, defaults.height_falloff).max(0.0),
            albedo: color(self.albedo, 1.0).map(|c| c.min(1.0)),
            anisotropy: finite(self.anisotropy, defaults.anisotropy)
                .clamp(-Self::MAX_ANISOTROPY, Self::MAX_ANISOTROPY),
            emission: color(self.emission, 0.0),
            ambient: finite(self.ambient, 1.0).max(0.0),
            light: finite(self.light, 1.0).max(0.0),
            volumetric_distance: finite(self.volumetric_distance, defaults.volumetric_distance)
                .clamp(Self::MIN_VOLUMETRIC_DISTANCE, Self::MAX_VOLUMETRIC_DISTANCE),
            max_distance: finite(self.max_distance, defaults.max_distance).max(1.0),
        }
    }
}

impl Default for Fog {
    fn default() -> Self {
        Self {
            enabled: true,
            density: 0.02,
            height: 0.0,
            height_falloff: 10.0,
            albedo: [1.0, 1.0, 1.0],
            anisotropy: 0.4,
            emission: [0.0, 0.0, 0.0],
            ambient: 1.0,
            light: 1.0,
            volumetric_distance: 64.0,
            max_distance: 1000.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizing_keeps_values_in_range() {
        let wild = Fog {
            density: -1.0,
            height_falloff: f32::NAN,
            albedo: [2.0, -1.0, 0.5],
            anisotropy: 3.0,
            volumetric_distance: 10_000.0,
            max_distance: 0.0,
            ..Fog::default()
        };
        let fog = wild.sanitized();
        assert_eq!(fog.density, 0.0);
        assert_eq!(fog.height_falloff, Fog::default().height_falloff);
        assert_eq!(fog.albedo, [1.0, 0.0, 0.5]);
        assert_eq!(fog.anisotropy, Fog::MAX_ANISOTROPY);
        assert_eq!(fog.volumetric_distance, Fog::MAX_VOLUMETRIC_DISTANCE);
        assert_eq!(fog.max_distance, 1.0);
    }

    #[test]
    fn missing_fields_take_their_defaults() {
        let fog: Fog = serde_json::from_str(r#"{"density": 0.05}"#).unwrap();
        assert_eq!(fog.density, 0.05);
        assert!(fog.enabled);
        assert_eq!(fog.volumetric_distance, Fog::default().volumetric_distance);
    }
}
