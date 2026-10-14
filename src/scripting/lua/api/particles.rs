//! # Particle effects API
//!
//! `emit_particles(x, y, z[, options])` starts a one-off burst of GPU particles at a
//! world position: sparks, explosions, warp-in rings, trails. Thousands of particles cost
//! almost nothing on the CPU, unlike spark entities moved from Lua.
//!
//! Options (all optional):
//!
//! - `count` (default 24, up to 4096)
//! - `color`, `color_end`: `{r, g, b[, a]}` (0..1, above 1 glows under bloom) or
//!   `"#rrggbb"`; particles fade from `color` to `color_end` (default: to transparent
//!   black, which fades out additive particles)
//! - `intensity`: multiplies both colors (HDR; try 2-4 for bloom)
//! - `speed`: outward speed in m/s, a number or `{min, max}` (default `{2, 6}`)
//! - `life`: seconds, a number or `{min, max}` (default `{0.4, 0.8}`)
//! - `size`, `size_end`: quad size in meters at spawn and at death (0.15 -> 0)
//! - `velocity`: `{x, y, z}` added to every particle (e.g. the ship's velocity)
//! - `gravity`: vertical acceleration, negative falls (default 0)
//! - `drag`: fraction of speed lost per second (default 1)
//! - `radius`: start within this distance of the point (default 0)
//! - `shape`: `"sphere"` (default, all directions) or `"ring"` (flat in XZ, for
//!   shockwaves on a top-down playfield)
//! - `blend`: `"add"` (default, glowing) or `"alpha"`

use glam::Vec3;
use mlua::{Lua, Result as LuaResult, Table, Value};

use crate::gpu_particles::ParticleBurst;
use crate::scripting::lua::guards::with_active_commands;

const MAX_COUNT: u32 = 4096;

fn error(message: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}

/// A colour from Lua: `"#rrggbb[aa]"` or `{r, g, b[, a]}` (named or positional).
pub(crate) fn color(value: &Value, key: &str) -> LuaResult<[f32; 4]> {
    match value {
        Value::String(text) => {
            let text = text.to_str()?;
            let hex = text.trim().trim_start_matches('#');
            let byte = |index: usize| {
                hex.get(index..index + 2)
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                    .map(|byte| f32::from(byte) / 255.0)
                    .ok_or_else(|| error(format!("{key}: invalid hex color {text:?}")))
            };
            match hex.len() {
                6 => Ok([byte(0)?, byte(2)?, byte(4)?, 1.0]),
                8 => Ok([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
                _ => Err(error(format!("{key}: use #rrggbb or #rrggbbaa"))),
            }
        }
        Value::Table(table) => {
            let channel = |name: &str, index: i64, default: Option<f32>| -> LuaResult<f32> {
                let named: Option<f32> = table.get(name)?;
                let indexed: Option<f32> = table.get(index)?;
                named
                    .or(indexed)
                    .or(default)
                    .ok_or_else(|| error(format!("{key} needs r, g and b")))
            };
            Ok([
                channel("r", 1, None)?,
                channel("g", 2, None)?,
                channel("b", 3, None)?,
                channel("a", 4, Some(1.0))?,
            ])
        }
        _ => Err(error(format!(
            "{key} must be {{r, g, b[, a]}} or \"#rrggbb\""
        ))),
    }
}

/// A number or a `{min, max}` pair.
fn range(value: &Value, key: &str) -> LuaResult<(f32, f32)> {
    match value {
        Value::Integer(number) => Ok((*number as f32, *number as f32)),
        Value::Number(number) => Ok((*number as f32, *number as f32)),
        Value::Table(table) => {
            let min: f32 = table.get(1)?;
            let max: f32 = table.get(2)?;
            Ok((min.min(max), min.max(max)))
        }
        _ => Err(error(format!("{key} must be a number or {{min, max}}"))),
    }
}

fn vector(value: &Value, key: &str) -> LuaResult<Vec3> {
    let Value::Table(table) = value else {
        return Err(error(format!("{key} must be {{x, y, z}}")));
    };
    let component = |name: &str, index: i64| -> LuaResult<f32> {
        let named: Option<f32> = table.get(name)?;
        let indexed: Option<f32> = table.get(index)?;
        Ok(named.or(indexed).unwrap_or(0.0))
    };
    Ok(Vec3::new(
        component("x", 1)?,
        component("y", 2)?,
        component("z", 3)?,
    ))
}

pub(crate) fn burst_from_options(
    position: Vec3,
    options: Option<&Table>,
) -> LuaResult<ParticleBurst> {
    let mut burst = ParticleBurst {
        position,
        ..ParticleBurst::default()
    };
    let Some(options) = options else {
        return Ok(burst);
    };
    let get = |key: &str| options.get::<Value>(key);
    let mut color_end_given = false;
    for pair in options.clone().pairs::<String, Value>() {
        let (key, value) = pair?;
        match key.as_str() {
            "count" => {
                let count: i64 = options.get("count")?;
                if !(1..=i64::from(MAX_COUNT)).contains(&count) {
                    return Err(error(format!("count must be 1..{MAX_COUNT}")));
                }
                burst.count = count as u32;
            }
            "color" => burst.color = color(&value, "color")?,
            "color_end" => {
                burst.color_end = color(&value, "color_end")?;
                color_end_given = true;
            }
            "speed" => burst.speed = range(&value, "speed")?,
            "life" => {
                let (min, max) = range(&value, "life")?;
                if min <= 0.0 {
                    return Err(error("life must be above 0"));
                }
                burst.life = (min, max);
            }
            "size" => burst.size = options.get("size")?,
            "size_end" => burst.size_end = options.get("size_end")?,
            "velocity" => burst.velocity = vector(&value, "velocity")?,
            "gravity" => burst.gravity = options.get("gravity")?,
            "drag" => burst.drag = options.get::<f32>("drag")?.max(0.0),
            "radius" => burst.radius = options.get::<f32>("radius")?.max(0.0),
            "intensity" | "shape" | "blend" => {}
            other => {
                return Err(error(format!(
                "unknown emit_particles option {other:?}; see the guide's Particle effects section"
            )))
            }
        }
    }
    if let Value::String(shape) = get("shape")? {
        burst.flat = match shape.to_str()?.as_ref() {
            "sphere" => false,
            "ring" => true,
            other => {
                return Err(error(format!(
                    "shape {other:?}: use \"sphere\" or \"ring\""
                )))
            }
        };
    }
    if let Value::String(blend) = get("blend")? {
        burst.additive = match blend.to_str()?.as_ref() {
            "add" | "additive" => true,
            "alpha" => false,
            other => return Err(error(format!("blend {other:?}: use \"add\" or \"alpha\""))),
        };
    }
    if !color_end_given && !burst.additive {
        // Alpha particles fade out in their own color.
        burst.color_end = [burst.color[0], burst.color[1], burst.color[2], 0.0];
    }
    if let Some(intensity) = options.get::<Option<f32>>("intensity")? {
        for channel in 0..3 {
            burst.color[channel] *= intensity;
            burst.color_end[channel] *= intensity;
        }
    }
    Ok(burst)
}

pub(crate) fn register_particles_api(lua: &Lua) -> LuaResult<()> {
    lua.globals().set(
        "emit_particles",
        lua.create_function(|_, (x, y, z, options): (f32, f32, f32, Option<Table>)| {
            let burst = burst_from_options(Vec3::new(x, y, z), options.as_ref())?;
            with_active_commands(|commands| {
                commands.particle_bursts.push(burst);
                Ok(())
            })
        })?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_map_to_a_burst() {
        let lua = Lua::new();
        let options: Table = lua
            .load(
                r##"return { count = 40, color = "#ff8000", intensity = 2, speed = { 1, 3 },
                            life = 0.5, shape = "ring", velocity = { x = 1 }, gravity = -9.8 }"##,
            )
            .eval()
            .unwrap();
        let burst = burst_from_options(Vec3::ONE, Some(&options)).unwrap();
        assert_eq!(burst.count, 40);
        assert_eq!(burst.color[0], 2.0);
        assert!((burst.color[1] - 2.0 * 128.0 / 255.0).abs() < 1e-5);
        assert_eq!(burst.speed, (1.0, 3.0));
        assert_eq!(burst.life, (0.5, 0.5));
        assert!(burst.flat);
        assert_eq!(burst.velocity, Vec3::X);
        assert_eq!(burst.gravity, -9.8);

        let typo: Table = lua.load("return { colour = '#fff' }").eval().unwrap();
        let message = burst_from_options(Vec3::ZERO, Some(&typo))
            .unwrap_err()
            .to_string();
        assert!(message.contains("colour"), "{message}");
    }
}
