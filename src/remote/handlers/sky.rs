//! `sky.get` and `sky.set`: the procedural sky and its day/night clock (`crate::sky`).

use serde_json::{json, Map, Value};

use super::{RemoteContext, RemoteResult};
use crate::app::RuntimeMode;
use crate::remote::json::entity_from_json;
use crate::scene::Name;
use crate::sky::{ProceduralSky, SkyFrame};

/// How the engine's compass lies, for callers placing things by it.
const COMPASS: &str = "North is -Z and east +X (y up) at north_heading 0; north_heading turns \
                       north toward +X by that many degrees.";

pub(super) fn get(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let playing = ctx.runtime_mode() == Some(RuntimeMode::Playing);
    let scene = &ctx.gpu.scene;
    let Some(sky) = scene.environment().sky().cloned() else {
        return Ok(json!({
            "enabled": false,
            "note": "The scene has no procedural sky; sky_set {enabled: true} adds one \
                     (it replaces the HDR sky while enabled).",
            "compass": COMPASS,
        }));
    };
    let clock = scene.sky_clock();
    let frame = sky.frame(clock.elapsed_hours(&sky));
    let world = scene.main_world();
    let following: Vec<String> = crate::scene::sky_following_lights(world)
        .into_iter()
        .map(|entity| {
            world
                .get::<&Name>(entity)
                .map(|name| name.0.clone())
                .unwrap_or_else(|_| entity.to_bits().get().to_string())
        })
        .collect();
    let running = sky.enabled && sky.day_length > 0.0 && (playing || clock.preview());
    let day_bake = scene.day_bake().map(|bake| {
        json!({
            "hours": bake.hours,
            "baking_hour": bake.hour(),
            "hours_done": bake.current,
        })
    });
    Ok(json!({
        "enabled": sky.enabled,
        "settings": sky,
        "now": now_json(&frame),
        "clock": {
            "running": running,
            "preview": clock.preview(),
            "elapsed_hours": round(clock.elapsed_hours(&sky), 4),
            "held_at": clock.hold(),
            "day_length": sky.day_length,
            "note": "The clock runs in play mode, and in the editor while preview is on, at \
                     day_length real seconds per day; it starts over from the sky's date and \
                     time whenever they are set.",
        },
        "sun": {
            "altitude": round(frame.positions.sun.altitude, 3),
            "azimuth": round(frame.positions.sun.azimuth, 3),
            "direction": vec_json(frame.sun_direction),
        },
        "moon": {
            "altitude": round(frame.positions.moon.altitude, 3),
            "azimuth": round(frame.positions.moon.azimuth, 3),
            "direction": vec_json(frame.moon_direction),
            "illumination": round(frame.positions.moon_illumination, 4),
            "phase": moon_phase(&frame),
        },
        "light": frame.light.map(|light| json!({
            "source": if light.moon { "moon" } else { "sun" },
            "intensity": round(light.intensity as f64, 4),
            "color": vec_json(light.color),
            "direction": vec_json(light.direction),
        })),
        "night": frame.is_night(),
        "following_lights": following,
        "day_bake": day_bake,
        "compass": COMPASS,
    }))
}

pub(super) fn set(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let fields = params.as_object().ok_or("sky settings must be an object")?;
    let known = serde_json::to_value(ProceduralSky::default()).map_err(|err| err.to_string())?;
    let mut patch = Map::new();
    for (key, value) in fields {
        match key.as_str() {
            "preview" | "reset_clock" | "follow" => {}
            "time" => {
                patch.insert(key.clone(), json!(parse_time(value)?));
            }
            "date" => {
                let text = value.as_str().ok_or("date must be text: YYYY-MM-DD")?;
                crate::sky::Date::parse(text)?;
                patch.insert(key.clone(), value.clone());
            }
            _ if known.get(key).is_some() => {
                patch.insert(key.clone(), value.clone());
            }
            _ => {
                return Err(format!(
                    "unknown sky setting {key:?}; use the fields sky_get shows under settings, \
                     preview, reset_clock or follow"
                ))
            }
        }
    }

    let scene = &mut ctx.gpu.scene;
    let created = scene.environment().sky().is_none();
    let mut modified = false;
    if created || !patch.is_empty() {
        let patch = json!({ "sky": Value::Object(patch) });
        super::rendering::patch_environment(scene, &patch)?;
        modified = true;
    }
    if let Some(preview) = params.get("preview").and_then(Value::as_bool) {
        scene.sky_clock_mut().set_preview(preview);
    }
    if params.get("reset_clock").and_then(Value::as_bool) == Some(true) {
        scene.sky_clock_mut().reset();
    }
    // A new sky lights the scene through a directional light from the start.
    let follow = match params.get("follow") {
        Some(value) => Some(value.clone()),
        None if created => Some(json!("auto")),
        None => None,
    };
    let mut note = None;
    if let Some(follow) = follow {
        note = Some(set_follow(scene, &follow)?);
        modified = true;
    }
    if modified {
        ctx.mark_modified();
    }
    let mut result = get(ctx)?;
    if let Some(note) = note {
        result["note"] = json!(note);
    }
    Ok(result)
}

/// Make a directional light follow the sky: `"auto"` the scene's first (a new "Sun" when
/// it has none), an entity that one, `false` none.
fn set_follow(scene: &mut crate::scene::Scene, follow: &Value) -> Result<String, String> {
    let world = scene.main_world_mut();
    if follow == &Value::Bool(false) {
        crate::scene::stop_following_sky(world);
        return Ok("No directional light follows the sky now.".into());
    }
    let light = if follow.as_str() == Some("auto") {
        None
    } else {
        Some(entity_from_json(world, follow)?)
    };
    let entity = crate::scene::follow_sky(world, light)?;
    let name = world
        .get::<&Name>(entity)
        .map(|name| name.0.clone())
        .unwrap_or_else(|_| entity.to_bits().get().to_string());
    Ok(format!(
        "{name} follows the sky: the sun's light by day, the moon's by night (its own colour, \
         intensity and rotation apply again without the sky)."
    ))
}

/// Hours from a number or `"HH:MM"` text.
fn parse_time(value: &Value) -> Result<f64, String> {
    let hours = match value {
        Value::Number(number) => number.as_f64().ok_or("time must be a number")?,
        Value::String(text) => {
            let (h, m) = text
                .split_once(':')
                .ok_or_else(|| format!("{text:?} is not a time (HH:MM or hours)"))?;
            let h: f64 = h
                .trim()
                .parse()
                .map_err(|_| format!("{text:?} is not a time"))?;
            let m: f64 = m
                .trim()
                .parse()
                .map_err(|_| format!("{text:?} is not a time"))?;
            h + m / 60.0
        }
        _ => return Err("time must be hours (a number) or \"HH:MM\"".into()),
    };
    if !(0.0..=24.0).contains(&hours) {
        return Err(format!("time {hours} is not between 0 and 24 hours"));
    }
    Ok(hours)
}

fn now_json(frame: &SkyFrame) -> Value {
    let minutes = (frame.hours * 60.0).round() as i64;
    json!({
        "date": frame.date.to_string(),
        "time": round(frame.hours, 4),
        "clock": format!("{:02}:{:02}", (minutes / 60) % 24, minutes % 60),
    })
}

fn moon_phase(frame: &SkyFrame) -> &'static str {
    let lit = frame.positions.moon_illumination;
    let waxing = frame.positions.moon_waxing;
    match (lit, waxing) {
        (l, _) if l < 0.03 => "new",
        (l, _) if l > 0.97 => "full",
        (l, true) if (0.45..=0.55).contains(&l) => "first quarter",
        (l, false) if (0.45..=0.55).contains(&l) => "last quarter",
        (l, true) if l < 0.5 => "waxing crescent",
        (_, true) => "waxing gibbous",
        (l, false) if l < 0.5 => "waning crescent",
        (_, false) => "waning gibbous",
    }
}

fn vec_json(v: glam::Vec3) -> Value {
    json!([
        round(v.x as f64, 4),
        round(v.y as f64, 4),
        round(v.z as f64, 4)
    ])
}

fn round(value: f64, digits: i32) -> f64 {
    let scale = 10f64.powi(digits);
    (value * scale).round() / scale
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{DirectionalLight, EnvironmentComponent};

    #[test]
    fn times_parse_as_hours_or_clock_text() {
        assert_eq!(parse_time(&json!(6.5)).unwrap(), 6.5);
        assert_eq!(parse_time(&json!("18:45")).unwrap(), 18.75);
        assert!(parse_time(&json!("dawn")).is_err());
        assert!(parse_time(&json!(25)).is_err());
    }

    #[test]
    fn a_new_sky_follows_the_first_light_or_makes_a_sun() {
        let mut scene = crate::scene::Scene::new();
        assert!(set_follow(&mut scene, &json!("auto"))
            .unwrap()
            .contains("Sun"));
        let following = scene
            .main_world()
            .query::<&DirectionalLight>()
            .iter()
            .filter(|(_, light)| light.follow_sky)
            .count();
        assert_eq!(following, 1);
        set_follow(&mut scene, &json!(false)).unwrap();
        assert!(scene
            .main_world()
            .query::<&DirectionalLight>()
            .iter()
            .all(|(_, light)| !light.follow_sky));
    }

    #[test]
    fn an_environment_patch_creates_and_updates_the_sky() {
        let mut scene = crate::scene::Scene::new();
        super::super::rendering::patch_environment(&mut scene, &json!({ "sky": {} })).unwrap();
        assert!(scene.environment().active_sky().is_some());
        super::super::rendering::patch_environment(
            &mut scene,
            &json!({ "sky": { "time": 19.5, "latitude": 60.0 } }),
        )
        .unwrap();
        let sky = scene.environment().sky().unwrap();
        assert_eq!((sky.time, sky.latitude), (19.5, 60.0));
        let component = EnvironmentComponent::from_environment(scene.environment());
        assert_eq!(component.sky.as_ref().unwrap().time, 19.5);
    }
}
