//! Where the sun, the moon and the stars stand in the sky for a date, a time and a place
//! on Earth, and where that is in the engine's world.
//!
//! The sun comes from the Astronomical Almanac's low-precision formulas (about 0.01°),
//! the moon from the largest terms of Meeus' lunar theory ("Astronomical Algorithms",
//! ch. 47; a few tenths of a degree), corrected for parallax and refraction, which is
//! plenty for a sky.
//!
//! The engine's compass: north is -Z, east +X, up +Y. A scene turns it with its north
//! heading (degrees from -Z toward +X where north lies).

use glam::{DMat3, DVec3, Vec3};

/// A calendar date (proleptic Gregorian).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Date {
    pub const fn new(year: i32, month: u32, day: u32) -> Self {
        Self { year, month, day }
    }

    /// Parses `YYYY-MM-DD`.
    pub fn parse(text: &str) -> Result<Self, String> {
        let invalid = || format!("{text:?} is not a date (YYYY-MM-DD)");
        let mut parts = text.trim().splitn(3, '-');
        let year = parts.next().and_then(|part| part.parse::<i32>().ok());
        let month = parts.next().and_then(|part| part.parse::<u32>().ok());
        let day = parts.next().and_then(|part| part.parse::<u32>().ok());
        let (Some(year), Some(month), Some(day)) = (year, month, day) else {
            return Err(invalid());
        };
        if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
            return Err(invalid());
        }
        Ok(Self { year, month, day })
    }

    /// The Julian day at 0h UT of this date.
    pub fn julian_day(&self) -> f64 {
        // Meeus ch. 7.
        let (mut year, mut month) = (self.year as f64, self.month as f64);
        if month <= 2.0 {
            year -= 1.0;
            month += 12.0;
        }
        let a = (year / 100.0).floor();
        let b = 2.0 - a + (a / 4.0).floor();
        (365.25 * (year + 4716.0)).floor() + (30.6001 * (month + 1.0)).floor() + self.day as f64 + b
            - 1524.5
    }

    /// The date `days` days later (or earlier).
    pub fn add_days(&self, days: i64) -> Self {
        Self::from_julian_day(self.julian_day() + days as f64)
    }

    /// The date of the Julian day `jd` (UT).
    pub fn from_julian_day(jd: f64) -> Self {
        // Meeus ch. 7.
        let z = (jd + 0.5).floor();
        let alpha = ((z - 1_867_216.25) / 36_524.25).floor();
        let a = z + 1.0 + alpha - (alpha / 4.0).floor();
        let b = a + 1524.0;
        let c = ((b - 122.1) / 365.25).floor();
        let d = (365.25 * c).floor();
        let e = ((b - d) / 30.6001).floor();
        let day = (b - d - (30.6001 * e).floor()) as u32;
        let month = if e < 14.0 { e - 1.0 } else { e - 13.0 } as u32;
        let year = if month > 2 { c - 4716.0 } else { c - 4715.0 } as i32;
        Self { year, month, day }
    }
}

impl std::fmt::Display for Date {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        2 if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// A place on Earth and how its clocks run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Observer {
    /// Degrees, north positive.
    pub latitude: f64,
    /// Degrees, east positive.
    pub longitude: f64,
    /// Hours the local clock is ahead of UT.
    pub utc_offset: f64,
}

/// A direction in the local sky.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Horizontal {
    /// Degrees above the horizon.
    pub altitude: f64,
    /// Degrees from north toward east.
    pub azimuth: f64,
}

impl Horizontal {
    /// The direction in the engine's world (unit vector toward it), with north
    /// `north_heading` degrees from -Z toward +X.
    pub fn to_world(&self, north_heading: f64) -> Vec3 {
        let (alt, az) = (self.altitude.to_radians(), self.azimuth.to_radians());
        let local = DVec3::new(az.sin() * alt.cos(), alt.sin(), -az.cos() * alt.cos());
        (heading_rotation(north_heading) * local).as_vec3()
    }
}

/// Turns the compass so north lies `north_heading` degrees from -Z toward +X.
fn heading_rotation(north_heading: f64) -> DMat3 {
    DMat3::from_rotation_y(-north_heading.to_radians())
}

/// Right ascension and declination (radians), and the distance (km; the sun's in km
/// too).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Equatorial {
    pub right_ascension: f64,
    pub declination: f64,
    pub distance_km: f64,
}

impl Equatorial {
    /// The unit vector in the equatorial frame: x toward the vernal equinox, z toward
    /// the north celestial pole.
    pub fn unit(&self) -> DVec3 {
        let (ra, dec) = (self.right_ascension, self.declination);
        DVec3::new(dec.cos() * ra.cos(), dec.cos() * ra.sin(), dec.sin())
    }
}

const AU_KM: f64 = 149_597_870.7;
const EARTH_RADIUS_KM: f64 = 6378.14;

/// Days since J2000.0 (2000-01-01 12:00 UT).
fn days_since_j2000(jd: f64) -> f64 {
    jd - 2_451_545.0
}

fn wrap_degrees(degrees: f64) -> f64 {
    degrees.rem_euclid(360.0)
}

/// The sun's place (Astronomical Almanac, low precision).
pub fn sun(jd: f64) -> Equatorial {
    let n = days_since_j2000(jd);
    let mean_anomaly = wrap_degrees(357.529 + 0.985_600_28 * n).to_radians();
    let mean_longitude = wrap_degrees(280.459 + 0.985_647_36 * n);
    let longitude =
        (mean_longitude + 1.915 * mean_anomaly.sin() + 0.020 * (2.0 * mean_anomaly).sin())
            .to_radians();
    let distance_au =
        1.000_14 - 0.016_71 * mean_anomaly.cos() - 0.000_14 * (2.0 * mean_anomaly).cos();
    let obliquity = (23.439 - 0.000_000_36 * n).to_radians();
    Equatorial {
        right_ascension: (obliquity.cos() * longitude.sin()).atan2(longitude.cos()),
        declination: (obliquity.sin() * longitude.sin()).asin(),
        distance_km: distance_au * AU_KM,
    }
}

/// The moon's place (the largest terms of Meeus ch. 47).
pub fn moon(jd: f64) -> Equatorial {
    let t = days_since_j2000(jd) / 36_525.0;
    let mean_longitude = 218.316_447_7 + 481_267.881_234_21 * t;
    let elongation = (297.850_192_1 + 445_267.111_403_4 * t).to_radians();
    let sun_anomaly = (357.529_109_2 + 35_999.050_290_9 * t).to_radians();
    let anomaly = (134.963_396_4 + 477_198.867_505_5 * t).to_radians();
    let latitude_argument = (93.272_095_0 + 483_202.017_523_3 * t).to_radians();
    // The terms with the sun's anomaly shrink with the Earth's orbit's eccentricity.
    let e = 1.0 - 0.002_516 * t;
    let (d, m, mp, f) = (elongation, sun_anomaly, anomaly, latitude_argument);
    let longitude_terms = 6.288_774 * mp.sin()
        + 1.274_027 * (2.0 * d - mp).sin()
        + 0.658_314 * (2.0 * d).sin()
        + 0.213_618 * (2.0 * mp).sin()
        - 0.185_116 * e * m.sin()
        - 0.114_332 * (2.0 * f).sin()
        + 0.058_793 * (2.0 * d - 2.0 * mp).sin()
        + 0.057_066 * e * (2.0 * d - m - mp).sin()
        + 0.053_322 * (2.0 * d + mp).sin()
        + 0.045_758 * e * (2.0 * d - m).sin()
        - 0.040_923 * e * (m - mp).sin()
        - 0.034_720 * d.sin()
        - 0.030_383 * e * (m + mp).sin()
        + 0.015_327 * (2.0 * d - 2.0 * f).sin()
        - 0.012_528 * (mp + 2.0 * f).sin()
        + 0.010_980 * (mp - 2.0 * f).sin();
    let latitude_terms = 5.128_122 * f.sin()
        + 0.280_602 * (mp + f).sin()
        + 0.277_693 * (mp - f).sin()
        + 0.173_237 * (2.0 * d - f).sin()
        + 0.055_413 * (2.0 * d - mp + f).sin()
        + 0.046_271 * (2.0 * d - mp - f).sin()
        + 0.032_573 * (2.0 * d + f).sin()
        + 0.017_198 * (2.0 * mp + f).sin()
        + 0.009_266 * (2.0 * d + mp - f).sin()
        + 0.008_822 * (2.0 * mp - f).sin();
    let distance_km = 385_000.56
        - 20_905.355 * mp.cos()
        - 3_699.111 * (2.0 * d - mp).cos()
        - 2_955.968 * (2.0 * d).cos()
        - 569.925 * (2.0 * mp).cos()
        + 48.888 * e * m.cos()
        - 3.149 * (2.0 * f).cos()
        + 246.158 * (2.0 * d - 2.0 * mp).cos()
        - 152.138 * e * (2.0 * d - m - mp).cos()
        - 170.733 * (2.0 * d + mp).cos()
        - 204.586 * e * (2.0 * d - m).cos()
        - 129.620 * e * (m - mp).cos();
    let longitude = wrap_degrees(mean_longitude + longitude_terms).to_radians();
    let latitude = latitude_terms.to_radians();
    let obliquity = (23.439_291 - 0.013_004_2 * t).to_radians();
    Equatorial {
        right_ascension: (longitude.sin() * obliquity.cos() - latitude.tan() * obliquity.sin())
            .atan2(longitude.cos()),
        declination: (latitude.sin() * obliquity.cos()
            + latitude.cos() * obliquity.sin() * longitude.sin())
        .asin(),
        distance_km,
    }
}

/// Greenwich mean sidereal time (radians) at Julian day `jd` (UT).
pub fn greenwich_sidereal_time(jd: f64) -> f64 {
    let hours = 18.697_374_558 + 24.065_709_824_419_08 * days_since_j2000(jd);
    (hours.rem_euclid(24.0) * 15.0).to_radians()
}

/// The rotation from the equatorial frame (see [`Equatorial::unit`]) to the engine's
/// world, for an observer at `latitude` (degrees) under local sidereal time `lst`
/// (radians), north `north_heading` degrees from -Z toward +X.
pub fn world_from_equatorial(latitude: f64, lst: f64, north_heading: f64) -> DMat3 {
    let (sin_lat, cos_lat) = latitude.to_radians().sin_cos();
    let (sin_lst, cos_lst) = lst.sin_cos();
    // Rows: east, up, south (-north), each over the equatorial x, y, z.
    let east = DVec3::new(-sin_lst, cos_lst, 0.0);
    let up = DVec3::new(cos_lat * cos_lst, cos_lat * sin_lst, sin_lat);
    let north = DVec3::new(-sin_lat * cos_lst, -sin_lat * sin_lst, cos_lat);
    let local = DMat3::from_cols(east, up, -north).transpose();
    heading_rotation(north_heading) * local
}

/// The altitude and azimuth of `place` for an observer at `latitude` under local
/// sidereal time `lst` (radians), geometrically (no refraction or parallax).
pub fn horizontal(place: &Equatorial, latitude: f64, lst: f64) -> Horizontal {
    let v = world_from_equatorial(latitude, lst, 0.0) * place.unit();
    Horizontal {
        altitude: v.y.clamp(-1.0, 1.0).asin().to_degrees(),
        azimuth: wrap_degrees(v.x.atan2(-v.z).to_degrees()),
    }
}

/// How much the air lifts something seen at `altitude` degrees (Bennett's formula,
/// degrees), fading out below the horizon.
pub fn refraction(altitude: f64) -> f64 {
    let h = altitude.max(-1.9);
    let lift = 1.02 / (h + 10.3 / (h + 5.11)).to_radians().tan() / 60.0;
    if altitude < -1.0 {
        lift * ((altitude + 3.0) / 2.0).clamp(0.0, 1.0)
    } else {
        lift
    }
}

/// The sun and moon as an observer sees them at one moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkyPositions {
    /// Julian day (UT).
    pub julian_day: f64,
    /// Local sidereal time (radians).
    pub local_sidereal_time: f64,
    pub sun: Horizontal,
    pub moon: Horizontal,
    /// The share of the moon's disc the sun lights (0 new, 1 full).
    pub moon_illumination: f64,
    /// Whether the moon is waxing (its lit side toward the sun's west of it).
    pub moon_waxing: bool,
    /// Angle between the sun and the moon seen from Earth (degrees).
    pub moon_elongation: f64,
}

/// Where the sun and moon are for `observer` on `date` at `hours` of its local clock
/// (refraction applied; the moon also corrected for parallax).
pub fn positions(observer: &Observer, date: Date, hours: f64) -> SkyPositions {
    let jd = date.julian_day() + (hours - observer.utc_offset) / 24.0;
    let lst = greenwich_sidereal_time(jd) + observer.longitude.to_radians();
    let sun_place = sun(jd);
    let moon_place = moon(jd);

    let mut sun_sky = horizontal(&sun_place, observer.latitude, lst);
    sun_sky.altitude += refraction(sun_sky.altitude);

    let mut moon_sky = horizontal(&moon_place, observer.latitude, lst);
    // Seen from the surface rather than the Earth's centre, the moon stands lower by up
    // to its horizontal parallax (about a degree).
    let parallax = (EARTH_RADIUS_KM / moon_place.distance_km)
        .asin()
        .to_degrees();
    moon_sky.altitude -= parallax * moon_sky.altitude.to_radians().cos();
    moon_sky.altitude += refraction(moon_sky.altitude);

    let elongation = sun_place
        .unit()
        .dot(moon_place.unit())
        .clamp(-1.0, 1.0)
        .acos();
    // The phase angle, at the moon between the sun and the Earth.
    let phase_angle = (sun_place.distance_km * elongation.sin())
        .atan2(moon_place.distance_km - sun_place.distance_km * elongation.cos());
    let waxing = (moon_place.right_ascension - sun_place.right_ascension)
        .sin()
        .is_sign_positive();
    SkyPositions {
        julian_day: jd,
        local_sidereal_time: lst,
        sun: sun_sky,
        moon: moon_sky,
        moon_illumination: (1.0 + phase_angle.cos()) / 2.0,
        moon_waxing: waxing,
        moon_elongation: elongation.to_degrees(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance
    }

    #[test]
    fn julian_days_round_trip() {
        // Meeus example 7.a: 1957-10-04.81 is JD 2436116.31.
        let date = Date::new(1957, 10, 4);
        assert!(close(date.julian_day() + 0.81, 2_436_116.31, 1e-6));
        assert_eq!(Date::new(2000, 1, 1).julian_day(), 2_451_544.5);
        for date in [
            Date::new(2024, 2, 29),
            Date::new(1999, 12, 31),
            Date::new(2026, 10, 17),
        ] {
            assert_eq!(Date::from_julian_day(date.julian_day()), date);
        }
        assert_eq!(Date::new(2026, 12, 31).add_days(1), Date::new(2027, 1, 1));
        assert_eq!(Date::new(2024, 3, 1).add_days(-1), Date::new(2024, 2, 29));
    }

    #[test]
    fn dates_parse_and_print() {
        let date = Date::parse("2026-09-22").unwrap();
        assert_eq!(date, Date::new(2026, 9, 22));
        assert_eq!(date.to_string(), "2026-09-22");
        assert!(Date::parse("2026-02-30").is_err());
        assert!(Date::parse("noon").is_err());
    }

    #[test]
    fn the_sun_matches_the_almanac() {
        // Meeus example 25.a: 1992-10-13 0h TD, RA 13h13m31.4s, Dec -7°47'06".
        let place = sun(2_448_908.5);
        let ra_hours = place
            .right_ascension
            .rem_euclid(std::f64::consts::TAU)
            .to_degrees()
            / 15.0;
        assert!(close(ra_hours, 13.0 + 13.0 / 60.0 + 31.4 / 3600.0, 0.002));
        assert!(close(
            place.declination.to_degrees(),
            -(7.0 + 47.0 / 60.0 + 6.0 / 3600.0),
            0.01
        ));
    }

    #[test]
    fn the_moon_matches_meeus() {
        // Meeus example 47.a: 1992-04-12 0h TD, RA 134.688470°, Dec 13.768368°, 368409.7 km.
        let place = moon(2_448_724.5);
        let ra = place.right_ascension.to_degrees().rem_euclid(360.0);
        assert!(close(ra, 134.688_470, 0.1), "{ra}");
        assert!(close(place.declination.to_degrees(), 13.768_368, 0.1));
        assert!(close(place.distance_km, 368_409.7, 300.0));
    }

    #[test]
    fn sidereal_time_matches_meeus() {
        // Meeus example 12.a: 1987-04-10 0h UT, GMST 13h10m46.3668s.
        let gmst = greenwich_sidereal_time(2_446_895.5).to_degrees() / 15.0;
        assert!(close(gmst, 13.0 + 10.0 / 60.0 + 46.3668 / 3600.0, 1e-4));
    }

    #[test]
    fn the_equinox_sun_stands_where_it_should() {
        // At the equinox at 60°N on the Greenwich meridian the noon sun stands about
        // 30° up in the south; it rises in the east and sets in the west.
        let observer = Observer {
            latitude: 60.0,
            longitude: 0.0,
            utc_offset: 0.0,
        };
        let date = Date::new(2026, 3, 20);
        let noon = positions(&observer, date, 12.1).sun;
        assert!(close(noon.altitude, 30.0, 0.8), "{noon:?}");
        assert!(close(noon.azimuth, 180.0, 3.0), "{noon:?}");
        let morning = positions(&observer, date, 6.0).sun;
        assert!(close(morning.azimuth, 90.0, 6.0), "{morning:?}");
        let evening = positions(&observer, date, 18.2).sun;
        assert!(close(evening.azimuth, 270.0, 6.0), "{evening:?}");
        // Midnight: well below the horizon in the north.
        let midnight = positions(&observer, date, 0.0).sun;
        assert!(midnight.altitude < -25.0, "{midnight:?}");
        let from_north = (midnight.azimuth + 180.0).rem_euclid(360.0) - 180.0;
        assert!(close(from_north, 0.0, 5.0), "{midnight:?}");
    }

    #[test]
    fn north_lies_along_negative_z_and_turns_with_the_heading() {
        let north = Horizontal {
            altitude: 0.0,
            azimuth: 0.0,
        };
        let east = Horizontal {
            altitude: 0.0,
            azimuth: 90.0,
        };
        assert!(north.to_world(0.0).distance(Vec3::NEG_Z) < 1e-6);
        assert!(east.to_world(0.0).distance(Vec3::X) < 1e-6);
        assert!(north.to_world(90.0).distance(Vec3::X) < 1e-6);
        let zenith = Horizontal {
            altitude: 90.0,
            azimuth: 0.0,
        };
        assert!(zenith.to_world(37.0).distance(Vec3::Y) < 1e-6);
    }

    #[test]
    fn the_celestial_pole_stands_over_north_at_the_latitude() {
        let pole = world_from_equatorial(52.0, 1.234, 0.0) * DVec3::Z;
        let altitude = pole.y.asin().to_degrees();
        assert!(close(altitude, 52.0, 1e-9));
        assert!(pole.x.abs() < 1e-9 && pole.z < 0.0);
    }

    #[test]
    fn full_and_new_moons_are_lit_accordingly() {
        let observer = Observer {
            latitude: 48.0,
            longitude: 0.0,
            utc_offset: 0.0,
        };
        // A full moon on 2026-10-26 (about 04:12 UT) and a new moon on 2026-11-09.
        let full = positions(&observer, Date::new(2026, 10, 26), 4.2);
        assert!(full.moon_illumination > 0.99, "{full:?}");
        let new = positions(&observer, Date::new(2026, 11, 9), 7.0);
        assert!(new.moon_illumination < 0.02, "{new:?}");
        // Waxing a week after the new moon.
        let waxing = positions(&observer, Date::new(2026, 11, 16), 7.0);
        assert!(waxing.moon_waxing && (0.3..0.8).contains(&waxing.moon_illumination));
    }

    #[test]
    fn refraction_lifts_the_horizon_by_half_a_degree() {
        assert!(close(refraction(0.0), 0.48, 0.02));
        assert!(refraction(45.0) < 0.02);
        assert_eq!(refraction(-5.0), 0.0);
    }
}
