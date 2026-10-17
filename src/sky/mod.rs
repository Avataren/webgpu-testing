//! The procedural sky and its day and night: an atmosphere that scatters the light of the
//! sun and the moon (Rayleigh and Mie single scattering with ozone, after Nishita et al.
//! and O'Neil, "Accurate Atmospheric Scattering", GPU Gems 2 ch. 16; the sky shader marches
//! the scattering integrals where O'Neil fits them, so twilight works), a starry night with
//! the Milky Way, a moon in its phase, and a clock that moves them along the paths they
//! take on Earth for a date and a place ([`celestial`]).
//!
//! It is an option of the scene's environment ([`ProceduralSky`], off by default). Each
//! frame the scene works out a [`SkyFrame`] from it and its clock: the renderer draws the
//! sky from it (and renders it into the environment map that lights and reflects the
//! scene), and directional lights with `follow_sky` take the sunlight by day and the
//! moonlight by night.

pub mod celestial;

use glam::{DVec3, Mat3, Vec3};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

pub use celestial::{Date, Horizontal, Observer, SkyPositions};

/// The atmosphere the sky shader and the sunlight's colour share (Earth's, as in Hillaire
/// 2020, "A Scalable and Production Ready Sky and Atmosphere Rendering Technique").
/// sky.wgsl has the same numbers.
pub mod atmosphere {
    use glam::Vec3;

    pub const GROUND_RADIUS: f32 = 6_360_000.0;
    pub const TOP_RADIUS: f32 = 6_460_000.0;
    /// Where the observer stands above the ground (m).
    pub const OBSERVER_HEIGHT: f32 = 200.0;
    pub const RAYLEIGH_SCATTERING: Vec3 = Vec3::new(5.802e-6, 13.558e-6, 33.1e-6);
    pub const RAYLEIGH_SCALE_HEIGHT: f32 = 8_000.0;
    pub const MIE_SCATTERING: f32 = 3.996e-6;
    pub const MIE_EXTINCTION: f32 = 4.40e-6;
    pub const MIE_SCALE_HEIGHT: f32 = 1_200.0;
    pub const OZONE_ABSORPTION: Vec3 = Vec3::new(0.650e-6, 1.881e-6, 0.085e-6);
    /// Ozone lies in a layer peaking at 25 km, thinning to nothing 15 km above and below.
    pub const OZONE_CENTER: f32 = 25_000.0;
    pub const OZONE_HALF_WIDTH: f32 = 15_000.0;
}

/// A scene's procedural sky and its clock (part of the environment; see the module).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProceduralSky {
    /// Use this sky instead of the environment's HDR image.
    pub enabled: bool,
    /// The day the sky shows (`YYYY-MM-DD`): the sun's path and the moon's phase follow
    /// it; a running clock turns it over at midnight.
    #[serde(
        serialize_with = "serialize_date",
        deserialize_with = "deserialize_date"
    )]
    pub date: Date,
    /// Hours on the local clock (0-24).
    pub time: f32,
    /// Real seconds a whole day takes while the clock runs (play mode, or the editor's
    /// preview); 0 stops it.
    pub day_length: f32,
    /// Degrees north (negative south).
    pub latitude: f32,
    /// Degrees east (negative west).
    pub longitude: f32,
    /// Hours the local clock runs ahead of UT.
    pub utc_offset: f32,
    /// Degrees from -Z toward +X where north lies (0: north is -Z, east +X).
    pub north_heading: f32,
    /// Sunlight above the atmosphere: the light's intensity with the sun overhead in a
    /// clear sky is a little less.
    pub sun_intensity: f32,
    /// Moonlight at full moon above the atmosphere. The default (1:200 of the sun's,
    /// where the real ratio is about 1:400,000) makes dark nights: enough to make out the
    /// moonlit side of things, while lamps and windows carry the scene.
    pub moon_intensity: f32,
    /// Scales the sky's own light (scattering, stars) without touching the lights.
    pub sky_intensity: f32,
    /// Air density factor (Rayleigh scattering: the blue).
    pub rayleigh: f32,
    /// Aerosol density factor (Mie scattering: haze, the glare round the sun).
    pub haze: f32,
    /// Ozone factor (the deep blue of twilight).
    pub ozone: f32,
    /// How strongly haze scatters forward (Henyey-Greenstein g, 0-0.99).
    pub mie_anisotropy: f32,
    /// The ground below the horizon (linear albedo).
    pub ground_albedo: [f32; 3],
    /// Apparent diameters (degrees).
    pub sun_size: f32,
    pub moon_size: f32,
    /// Starlight factor (0: no stars).
    pub stars: f32,
    /// Milky Way factor (0: none).
    pub milky_way: f32,
    /// The colour moonlight and the night sky take (the eye sees night bluish).
    pub night_tint: [f32; 3],
    /// The faint glow of the night sky away from the moon (airglow and starlight).
    pub night_glow: f32,
}

impl Default for ProceduralSky {
    fn default() -> Self {
        Self {
            enabled: true,
            date: Date::new(2026, 9, 26),
            time: 12.0,
            day_length: 0.0,
            latitude: 48.0,
            longitude: 0.0,
            utc_offset: 0.0,
            north_heading: 0.0,
            sun_intensity: 10.0,
            moon_intensity: 0.05,
            sky_intensity: 1.0,
            rayleigh: 1.0,
            haze: 1.0,
            ozone: 1.0,
            mie_anisotropy: 0.8,
            ground_albedo: [0.25, 0.23, 0.2],
            sun_size: 0.53,
            moon_size: 0.52,
            stars: 1.0,
            milky_way: 1.0,
            night_tint: [0.62, 0.74, 1.0],
            night_glow: 1.0,
        }
    }
}

fn serialize_date<S: Serializer>(date: &Date, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&date.to_string())
}

fn deserialize_date<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Date, D::Error> {
    let text = String::deserialize(deserializer)?;
    Date::parse(&text).map_err(serde::de::Error::custom)
}

impl ProceduralSky {
    pub fn observer(&self) -> Observer {
        Observer {
            latitude: self.latitude.clamp(-90.0, 90.0) as f64,
            longitude: self.longitude as f64,
            utc_offset: self.utc_offset as f64,
        }
    }

    /// The date and local hours `elapsed_hours` after the sky's own date and time.
    pub fn clock(&self, elapsed_hours: f64) -> (Date, f64) {
        let hours = self.time as f64 + elapsed_hours;
        let days = (hours / 24.0).floor();
        (self.date.add_days(days as i64), hours - days * 24.0)
    }

    /// What the sky looks like `elapsed_hours` after its own date and time.
    pub fn frame(&self, elapsed_hours: f64) -> SkyFrame {
        let (date, hours) = self.clock(elapsed_hours);
        SkyFrame::new(self, date, hours)
    }
}

/// A scene's sky clock: how far it has run from the sky's own date and time. It runs in
/// play mode, and in the editor while previewing; it starts over whenever the sky's date
/// or time is set.
#[derive(Clone, Debug, Default)]
pub struct SkyClock {
    elapsed_hours: f64,
    /// Run while editing too.
    preview: bool,
    /// The date and time it counts from.
    origin: Option<(Date, u32)>,
    /// Hours of the sky's own date it is held at (light probe bakes of the day's hours).
    hold: Option<f64>,
}

impl SkyClock {
    /// Move on by `dt` seconds of scene time if the clock runs (`playing`, or previewing
    /// in the editor, and a day length set).
    pub fn advance(&mut self, sky: &ProceduralSky, dt: f64, playing: bool) {
        self.sync_origin(sky);
        if self.hold.is_none() && (playing || self.preview) && sky.day_length > 0.0 && dt > 0.0 {
            self.elapsed_hours += dt * 24.0 / sky.day_length as f64;
        }
    }

    /// Hours run since the sky's own date and time.
    pub fn elapsed_hours(&self, sky: &ProceduralSky) -> f64 {
        if let Some(hours) = self.hold {
            return hours - sky.time as f64;
        }
        if self.origin == Some(Self::origin_of(sky)) {
            self.elapsed_hours
        } else {
            0.0
        }
    }

    /// Hold the sky at `hours` of its own date (None: let it go on from where it was).
    pub fn set_hold(&mut self, hours: Option<f64>) {
        self.hold = hours;
    }

    pub fn hold(&self) -> Option<f64> {
        self.hold
    }

    /// Back to the sky's own date and time.
    pub fn reset(&mut self) {
        self.elapsed_hours = 0.0;
    }

    pub fn preview(&self) -> bool {
        self.preview
    }

    pub fn set_preview(&mut self, preview: bool) {
        self.preview = preview;
    }

    fn origin_of(sky: &ProceduralSky) -> (Date, u32) {
        (sky.date, sky.time.to_bits())
    }

    fn sync_origin(&mut self, sky: &ProceduralSky) {
        let origin = Self::origin_of(sky);
        if self.origin != Some(origin) {
            self.origin = Some(origin);
            self.elapsed_hours = 0.0;
        }
    }
}

/// A bake of the light probes at several hours of the day: the sky is held at each hour
/// in turn while the volumes bake, then each volume's bakes go into one `.probes` file
/// whose light follows the time of day (the app drives it; see
/// `app::core::light_probes`).
#[derive(Clone, Debug)]
pub struct DayBake {
    /// Hours of the sky's own date, sorted.
    pub hours: Vec<f32>,
    /// The volumes baked (their grid ids; all when None).
    pub volumes: Option<Vec<u64>>,
    pub warmup: u32,
    pub sweeps: u32,
    pub rays: u32,
    /// The hour baking now (index into `hours`).
    pub current: usize,
    /// Frames to wait for the sky and its lights to show the hour before baking it (0:
    /// baking).
    pub settle: u32,
    /// Each finished bake: the volume, the hour, the `.probes` file.
    pub results: Vec<(u64, f32, Vec<u8>)>,
}

impl DayBake {
    /// Frames the sky gets to show an hour before its bake starts.
    pub const SETTLE_FRAMES: u32 = 3;

    pub fn new(
        hours: Vec<f32>,
        volumes: Option<Vec<u64>>,
        warmup: u32,
        sweeps: u32,
        rays: u32,
    ) -> Self {
        Self {
            hours,
            volumes,
            warmup,
            sweeps,
            rays,
            current: 0,
            settle: Self::SETTLE_FRAMES,
            results: Vec::new(),
        }
    }

    pub fn hour(&self) -> Option<f32> {
        self.hours.get(self.current).copied()
    }
}

/// The sky at one moment, for the renderer and the lights that follow it.
#[derive(Clone, Debug, PartialEq)]
pub struct SkyFrame {
    pub settings: ProceduralSky,
    pub date: Date,
    /// Hours on the local clock (0-24).
    pub hours: f64,
    pub positions: SkyPositions,
    /// Unit vectors toward the sun and the moon.
    pub sun_direction: Vec3,
    pub moon_direction: Vec3,
    /// Turns the equatorial frame (the stars') into the world.
    pub world_from_equatorial: Mat3,
    /// What the air lets through toward the sun and the moon from where the observer
    /// stands (per colour channel).
    pub sun_transmittance: Vec3,
    pub moon_transmittance: Vec3,
    /// The moonlight above the atmosphere: tint, intensity, phase.
    pub moon_irradiance: Vec3,
    /// What directional lights that follow the sky shine with, if anything.
    pub light: Option<SkyLight>,
}

/// The sunlight or moonlight lights following the sky take.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyLight {
    /// Unit vector the light travels along (from the sun or moon).
    pub direction: Vec3,
    /// Colour with its largest channel 1.
    pub color: Vec3,
    pub intensity: f32,
    /// The moon's light (else the sun's).
    pub moon: bool,
}

/// Sun altitude (degrees) below which the light that follows the sky turns to the moon.
const MOONLIGHT_SWITCH_ALTITUDE: f64 = -3.0;

fn smoothstep(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

impl SkyFrame {
    pub fn new(settings: &ProceduralSky, date: Date, hours: f64) -> Self {
        let observer = settings.observer();
        let positions = celestial::positions(&observer, date, hours);
        let heading = settings.north_heading as f64;
        let sun_direction = positions.sun.to_world(heading);
        let moon_direction = positions.moon.to_world(heading);
        let world_from_equatorial = celestial::world_from_equatorial(
            observer.latitude,
            positions.local_sidereal_time,
            heading,
        )
        .as_mat3();
        let sun_transmittance = transmittance(settings, positions.sun.altitude);
        let moon_transmittance = transmittance(settings, positions.moon.altitude);
        // A full moon is brighter than twice a half moon (its lit plains face us squarely
        // and cast no shadows at full).
        let phase = positions.moon_illumination.powf(1.6) as f32;
        let tint = Vec3::from_array(settings.night_tint).max(Vec3::ZERO);
        let moon_irradiance = tint * settings.moon_intensity.max(0.0) * phase;

        let mut frame = Self {
            settings: settings.clone(),
            date,
            hours,
            positions,
            sun_direction,
            moon_direction,
            world_from_equatorial,
            sun_transmittance,
            moon_transmittance,
            moon_irradiance,
            light: None,
        };
        frame.light = frame.choose_light();
        frame
    }

    /// The sun until it is a few degrees under the horizon (its light goes as it sets),
    /// then the moon when it is up (its light comes as the dusk deepens).
    fn choose_light(&self) -> Option<SkyLight> {
        let sun_altitude = self.positions.sun.altitude;
        let (direction, light, moon) = if sun_altitude > MOONLIGHT_SWITCH_ALTITUDE {
            // The disc sinks behind the horizon over its own size.
            let visible = smoothstep(-0.6, 0.6, sun_altitude) as f32;
            let light = self.sun_transmittance * self.settings.sun_intensity.max(0.0) * visible;
            (self.sun_direction, light, false)
        } else {
            let moon_altitude = self.positions.moon.altitude;
            let visible = smoothstep(-0.6, 0.6, moon_altitude)
                * smoothstep(MOONLIGHT_SWITCH_ALTITUDE, -10.0, sun_altitude);
            let light = self.moon_transmittance * self.moon_irradiance * visible as f32;
            (self.moon_direction, light, true)
        };
        let intensity = light.max_element();
        (intensity > 1e-6).then(|| SkyLight {
            direction: -direction,
            color: light / intensity,
            intensity,
            moon,
        })
    }

    /// How much of the environment's flat ambient light this hour has: all of it by day,
    /// fading through dusk, at night only the moon's share (the flat ambient stands in for
    /// the sky's light, and a dark night has little).
    pub fn ambient_scale(&self) -> f32 {
        let day = smoothstep(-12.0, 15.0, self.positions.sun.altitude) as f32;
        let moon = self.light.filter(|light| light.moon).map_or(0.0, |light| {
            light.intensity / self.settings.sun_intensity.max(1e-3)
        });
        day.max(moon).clamp(0.0, 1.0)
    }

    /// Whether the sun is down far enough for night (nautical dusk).
    pub fn is_night(&self) -> bool {
        self.positions.sun.altitude < -6.0
    }
}

/// What the air lets through from the observer toward something at `altitude` degrees
/// (0 below the horizon, where the ground is in the way).
pub fn transmittance(settings: &ProceduralSky, altitude: f64) -> Vec3 {
    use atmosphere::*;
    let origin = DVec3::new(0.0, (GROUND_RADIUS + OBSERVER_HEIGHT) as f64, 0.0);
    let alt = altitude.to_radians();
    let direction = DVec3::new(alt.cos(), alt.sin(), 0.0);
    let (ground, top) = (GROUND_RADIUS as f64, TOP_RADIUS as f64);
    if ray_sphere(origin, direction, ground).is_some_and(|(near, _)| near > 0.0) {
        return Vec3::ZERO;
    }
    let Some((_, length)) = ray_sphere(origin, direction, top) else {
        return Vec3::ONE;
    };
    const STEPS: usize = 64;
    let step = length / STEPS as f64;
    let mut depth = DVec3::ZERO;
    for i in 0..STEPS {
        let height = (origin + direction * ((i as f64 + 0.5) * step)).length() - ground;
        depth += extinction(settings, height) * step;
    }
    let t = (-depth).exp();
    Vec3::new(t.x as f32, t.y as f32, t.z as f32)
}

/// Extinction (per metre) at `height` above the ground.
fn extinction(settings: &ProceduralSky, height: f64) -> DVec3 {
    use atmosphere::*;
    let rayleigh = (-height / RAYLEIGH_SCALE_HEIGHT as f64).exp() * settings.rayleigh as f64;
    let mie = (-height / MIE_SCALE_HEIGHT as f64).exp() * settings.haze as f64;
    let ozone = (1.0 - (height - OZONE_CENTER as f64).abs() / OZONE_HALF_WIDTH as f64).max(0.0)
        * settings.ozone as f64;
    RAYLEIGH_SCATTERING.as_dvec3() * rayleigh
        + DVec3::splat(MIE_EXTINCTION as f64 * mie)
        + OZONE_ABSORPTION.as_dvec3() * ozone
}

/// Where a ray from `origin` along unit `direction` meets a sphere of `radius` round the
/// origin: (near, far) distances, if it does.
fn ray_sphere(origin: DVec3, direction: DVec3, radius: f64) -> Option<(f64, f64)> {
    let b = origin.dot(direction);
    let c = origin.length_squared() - radius * radius;
    let discriminant = b * b - c;
    if discriminant < 0.0 {
        return None;
    }
    let root = discriminant.sqrt();
    Some((-b - root, -b + root))
}

/// Hours of the day (0-24, local clock) at which the sun stands at `altitudes`
/// (degrees) on the sky's own date, rising and setting, plus local midnight and the
/// sun's highest point: good times for light probe keyframes. Altitudes the sun never
/// reaches that day are left out. Sorted.
pub fn keyframe_hours(settings: &ProceduralSky, altitudes: &[f64]) -> Vec<f32> {
    const STEPS: usize = 24 * 60;
    let observer = settings.observer();
    let altitude_at = |hours: f64| {
        celestial::positions(&observer, settings.date, hours)
            .sun
            .altitude
    };
    let samples: Vec<f64> = (0..=STEPS)
        .map(|i| altitude_at(24.0 * i as f64 / STEPS as f64))
        .collect();
    let hour = |i: usize| 24.0 * i as f64 / STEPS as f64;
    let mut hours = vec![0.0];
    let (highest, _) =
        samples.iter().enumerate().fold(
            (0, f64::MIN),
            |best, (i, &a)| if a > best.1 { (i, a) } else { best },
        );
    hours.push(hour(highest));
    for &target in altitudes {
        for i in 0..STEPS {
            let (a, b) = (samples[i], samples[i + 1]);
            if (a - target) * (b - target) <= 0.0 && a != b {
                let t = (target - a) / (b - a);
                hours.push(hour(i) + t * 24.0 / STEPS as f64);
            }
        }
    }
    let mut hours: Vec<f32> = hours.into_iter().map(|h| h as f32).collect();
    hours.sort_by(f32::total_cmp);
    hours.dedup_by(|a, b| (*a - *b).abs() < 0.25);
    hours
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noon_light_is_the_bright_white_sun() {
        let sky = ProceduralSky {
            date: Date::new(2026, 6, 21),
            ..Default::default()
        };
        let frame = sky.frame(0.0);
        let light = frame.light.expect("sunlight at noon");
        assert!(!light.moon);
        assert!(light.direction.y < -0.8, "{light:?}");
        assert!(light.intensity > 0.7 * sky.sun_intensity);
        // Nearly white high up, the blue a little dimmer than the red.
        assert!(
            light.color.z > 0.75 && light.color.z < light.color.x,
            "{light:?}"
        );
    }

    #[test]
    fn the_setting_sun_goes_red_then_the_moon_takes_over() {
        let sky = ProceduralSky {
            date: Date::new(2026, 10, 26),
            time: 0.0,
            ..Default::default()
        };
        // Find sunset: the light warms and dims as the sun nears the horizon.
        let mut low = None;
        for minute in (14 * 60)..(20 * 60) {
            let frame = sky.frame(minute as f64 / 60.0);
            let altitude = frame.positions.sun.altitude;
            if (2.0..3.0).contains(&altitude) {
                low = Some(frame);
                break;
            }
        }
        let low = low.expect("the sun sets");
        let light = low.light.unwrap();
        assert!(light.color.z < 0.5 * light.color.x, "{light:?}");
        assert!(light.intensity < 0.5 * sky.sun_intensity);
        // Near midnight at full moon the light is the moon's.
        let night = sky.frame(23.5);
        let light = night.light.expect("moonlight");
        assert!(light.moon && light.intensity < sky.moon_intensity);
    }

    #[test]
    fn the_flat_ambient_follows_the_day() {
        let sky = ProceduralSky {
            date: Date::new(2026, 9, 26),
            time: 0.0,
            ..Default::default()
        };
        assert_eq!(sky.frame(12.0).ambient_scale(), 1.0);
        let midnight = sky.frame(0.0).ambient_scale();
        assert!(midnight > 0.0 && midnight < 0.01, "{midnight}");
    }

    #[test]
    fn the_clock_turns_the_date_over() {
        let sky = ProceduralSky {
            date: Date::new(2026, 12, 31),
            time: 22.0,
            ..Default::default()
        };
        let (date, hours) = sky.clock(3.0);
        assert_eq!(date, Date::new(2027, 1, 1));
        assert!((hours - 1.0).abs() < 1e-9);
    }

    #[test]
    fn transmittance_is_whole_overhead_and_nothing_below_the_ground() {
        let sky = ProceduralSky::default();
        let up = transmittance(&sky, 90.0);
        assert!(up.min_element() > 0.6 && up.x > up.z);
        assert_eq!(transmittance(&sky, -5.0), Vec3::ZERO);
    }

    #[test]
    fn keyframes_cover_the_day() {
        let sky = ProceduralSky {
            date: Date::new(2026, 3, 20),
            ..Default::default()
        };
        let hours = keyframe_hours(&sky, &[-6.0, 0.0, 10.0]);
        // Midnight, noon, and each altitude rising and setting.
        assert_eq!(hours.len(), 8, "{hours:?}");
        assert_eq!(hours[0], 0.0);
        assert!(hours.iter().any(|h| (h - 12.1).abs() < 0.3));
    }

    #[test]
    fn settings_round_trip_with_the_date_as_text() {
        let sky = ProceduralSky::default();
        let json = serde_json::to_value(&sky).unwrap();
        assert_eq!(json["date"], "2026-09-26");
        let back: ProceduralSky = serde_json::from_value(json).unwrap();
        assert_eq!(back, sky);
        let partial: ProceduralSky = serde_json::from_str(r#"{"time": 6.5}"#).unwrap();
        assert_eq!(partial.time, 6.5);
        assert_eq!(partial.latitude, sky.latitude);
    }
}
