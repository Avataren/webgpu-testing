//! # Sky API
//!
//! The procedural sky's day and night for gameplay (lanterns lit at dusk, guards changing
//! at midnight):
//!
//! - `sky_info()` - `nil` without a procedural sky, else `{time, date, day_length,
//!   sun_altitude, sun_azimuth, moon_altitude, moon_azimuth, moon_illumination, night,
//!   daylight}`: the local clock time in hours, the date (`"YYYY-MM-DD"`), degrees above
//!   the horizon and from north toward east, the share of the moon lit, whether the sun
//!   is more than 6 degrees down, and a 0-1 daylight factor (0 at night, 1 by day,
//!   between at dusk and dawn)
//! - `set_time_of_day(hours)` - set the sky's time (its clock starts over from there);
//!   `set_environment({ sky = {...} })` changes any of its settings
//!
//! `sky_info()` reads the sky as it was when this frame's scripts started.

use mlua::{Lua, Result as LuaResult};

use crate::scripting::lua::guards::with_active_commands;

/// The sky scripts read, stored as Lua app data by the scripting state each frame.
#[derive(Clone, Debug)]
pub(crate) struct ScriptSky {
    pub time: f64,
    pub date: String,
    pub day_length: f32,
    pub sun_altitude: f64,
    pub sun_azimuth: f64,
    pub moon_altitude: f64,
    pub moon_azimuth: f64,
    pub moon_illumination: f64,
    pub night: bool,
}

impl ScriptSky {
    pub(crate) fn from_frame(frame: &crate::sky::SkyFrame) -> Self {
        Self {
            time: frame.hours,
            date: frame.date.to_string(),
            day_length: frame.settings.day_length,
            sun_altitude: frame.positions.sun.altitude,
            sun_azimuth: frame.positions.sun.azimuth,
            moon_altitude: frame.positions.moon.altitude,
            moon_azimuth: frame.positions.moon.azimuth,
            moon_illumination: frame.positions.moon_illumination,
            night: frame.is_night(),
        }
    }

    /// 0 at night, 1 by day: the sun from 6 degrees below the horizon to 6 above.
    fn daylight(&self) -> f64 {
        let t = ((self.sun_altitude + 6.0) / 12.0).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    }
}

pub(crate) fn register_sky_api(lua: &Lua) -> LuaResult<()> {
    lua.globals().set(
        "sky_info",
        lua.create_function(|lua, ()| {
            let Some(sky) = lua.app_data_ref::<ScriptSky>().map(|sky| sky.clone()) else {
                return Ok(mlua::Value::Nil);
            };
            let table = lua.create_table()?;
            table.set("time", sky.time)?;
            table.set("date", sky.date.clone())?;
            table.set("day_length", sky.day_length)?;
            table.set("sun_altitude", sky.sun_altitude)?;
            table.set("sun_azimuth", sky.sun_azimuth)?;
            table.set("moon_altitude", sky.moon_altitude)?;
            table.set("moon_azimuth", sky.moon_azimuth)?;
            table.set("moon_illumination", sky.moon_illumination)?;
            table.set("night", sky.night)?;
            table.set("daylight", sky.daylight())?;
            Ok(mlua::Value::Table(table))
        })?,
    )?;
    lua.globals().set(
        "set_time_of_day",
        lua.create_function(|_, hours: f64| {
            if !(0.0..=24.0).contains(&hours) {
                return Err(mlua::Error::RuntimeError(format!(
                    "set_time_of_day expects hours from 0 to 24, got {hours}"
                )));
            }
            let patch = serde_json::json!({ "sky": { "time": hours } });
            with_active_commands(|commands| {
                commands.environment(patch);
                Ok(())
            })
        })?,
    )
}
