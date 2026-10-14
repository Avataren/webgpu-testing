//! Conversions between engine types and the JSON used by the remote protocol.
//!
//! Conventions:
//! - Vectors are `[x, y, z]` arrays (objects with `x`/`y`/`z` are accepted too).
//! - Rotations are Euler angles in degrees, `[x, y, z]`, applied in YXZ order (yaw
//!   around Y, then pitch around X, then roll around Z), the same order as Lua's
//!   `set_rotation`. Quaternions are `[x, y, z, w]`.
//! - Colors are `[r, g, b]` or `[r, g, b, a]` in 0..1, or `"#rrggbb"` / `"#rrggbbaa"`.
//! - Entities are identified by their 64-bit id (the same value Lua scripts see) or by
//!   name.

use glam::{EulerRot, Quat, Vec3};
use hecs::{Entity, World};
use serde_json::{json, Value};

pub(crate) use crate::scene::transform::look_rotation;
use crate::scene::{Name, Transform};

pub(crate) fn opt_str<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

pub(crate) fn opt_f64(params: &Value, key: &str) -> Option<f64> {
    params.get(key).and_then(Value::as_f64)
}

pub(crate) fn opt_u64(params: &Value, key: &str) -> Option<u64> {
    params.get(key).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_f64().filter(|v| *v >= 0.0).map(|v| v as u64))
    })
}

pub(crate) fn opt_bool(params: &Value, key: &str) -> Option<bool> {
    params.get(key).and_then(Value::as_bool)
}

pub(crate) fn req_str<'a>(params: &'a Value, key: &str) -> Result<&'a str, String> {
    opt_str(params, key).ok_or_else(|| format!("missing string parameter \"{key}\""))
}

/// Round for display so JSON stays readable (float noise like 0.30000001 is dropped).
pub(crate) fn round(value: f32) -> f64 {
    let value = f64::from(value);
    (value * 10_000.0).round() / 10_000.0
}

pub(crate) fn vec3_json(v: Vec3) -> Value {
    json!([round(v.x), round(v.y), round(v.z)])
}

pub(crate) fn vec3_from_json(value: &Value) -> Result<Vec3, String> {
    match value {
        Value::Array(items) if items.len() == 3 => {
            let mut out = [0.0f32; 3];
            for (slot, item) in out.iter_mut().zip(items) {
                *slot = item
                    .as_f64()
                    .ok_or_else(|| format!("expected a number in {value}"))?
                    as f32;
            }
            Ok(Vec3::from(out))
        }
        Value::Object(map) => {
            let component = |key: &str| {
                map.get(key)
                    .and_then(Value::as_f64)
                    .map(|v| v as f32)
                    .ok_or_else(|| format!("missing \"{key}\" in {value}"))
            };
            Ok(Vec3::new(component("x")?, component("y")?, component("z")?))
        }
        Value::Number(n) => {
            let v = n.as_f64().unwrap_or(0.0) as f32;
            Ok(Vec3::splat(v))
        }
        _ => Err(format!("expected [x, y, z], got {value}")),
    }
}

/// Euler degrees `[x, y, z]` (YXZ order) to a quaternion.
pub(crate) fn euler_deg_to_quat(degrees: Vec3) -> Quat {
    Quat::from_euler(
        EulerRot::YXZ,
        degrees.y.to_radians(),
        degrees.x.to_radians(),
        degrees.z.to_radians(),
    )
}

/// A quaternion as Euler degrees `[x, y, z]` (YXZ order).
pub(crate) fn quat_to_euler_deg(rotation: Quat) -> Vec3 {
    let (yaw, pitch, roll) = rotation.to_euler(EulerRot::YXZ);
    Vec3::new(pitch.to_degrees(), yaw.to_degrees(), roll.to_degrees())
}

pub(crate) fn quat_from_json(value: &Value) -> Result<Quat, String> {
    match value {
        Value::Array(items) if items.len() == 4 => {
            let mut out = [0.0f32; 4];
            for (slot, item) in out.iter_mut().zip(items) {
                *slot = item
                    .as_f64()
                    .ok_or_else(|| format!("expected a number in {value}"))?
                    as f32;
            }
            let quat = Quat::from_array(out);
            if quat.length_squared() < 1e-12 {
                return Err("quaternion has zero length".into());
            }
            Ok(quat.normalize())
        }
        _ => Err(format!("expected [x, y, z, w], got {value}")),
    }
}

pub fn transform_to_json(transform: &Transform) -> Value {
    let rotation = transform.rotation;
    json!({
        "translation": vec3_json(transform.translation),
        "rotation": vec3_json(quat_to_euler_deg(rotation)),
        "rotation_quat": [round(rotation.x), round(rotation.y), round(rotation.z), round(rotation.w)],
        "scale": vec3_json(transform.scale),
    })
}

/// Apply the transform fields present in `params` to `transform`.
///
/// Recognised keys: `translation` (alias `position`), `rotation` (Euler degrees),
/// `rotation_quat`, `scale` (vector or uniform number), `look_at` (world point, using
/// the resulting translation; assumes no rotated parent). Returns whether any key was
/// present.
pub(crate) fn apply_transform_params(
    transform: &mut Transform,
    params: &Value,
) -> Result<bool, String> {
    let mut changed = false;
    if let Some(value) = params.get("translation").or_else(|| params.get("position")) {
        transform.translation = vec3_from_json(value)?;
        changed = true;
    }
    if let Some(value) = params.get("rotation") {
        transform.rotation = euler_deg_to_quat(vec3_from_json(value)?);
        changed = true;
    }
    if let Some(value) = params.get("rotation_quat") {
        transform.rotation = quat_from_json(value)?;
        changed = true;
    }
    if let Some(value) = params.get("scale") {
        transform.scale = vec3_from_json(value)?;
        changed = true;
    }
    if let Some(value) = params.get("look_at") {
        let target = vec3_from_json(value)?;
        transform.rotation = look_rotation(transform.translation, target)
            .ok_or("look_at target is at the entity's position")?;
        changed = true;
    }
    Ok(changed)
}

/// Parse a color into linear-agnostic RGBA floats in 0..1.
pub(crate) fn color_from_json(value: &Value) -> Result<[f32; 4], String> {
    match value {
        Value::String(text) => {
            let hex = text.trim().trim_start_matches('#');
            let byte = |index: usize| {
                hex.get(index..index + 2)
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok())
                    .map(|b| f32::from(b) / 255.0)
                    .ok_or_else(|| format!("invalid hex color {text:?}"))
            };
            match hex.len() {
                6 => Ok([byte(0)?, byte(2)?, byte(4)?, 1.0]),
                8 => Ok([byte(0)?, byte(2)?, byte(4)?, byte(6)?]),
                _ => Err(format!(
                    "invalid hex color {text:?}; use #rrggbb or #rrggbbaa"
                )),
            }
        }
        Value::Array(items) if items.len() == 3 || items.len() == 4 => {
            let mut out = [1.0f32; 4];
            for (slot, item) in out.iter_mut().zip(items) {
                *slot = item
                    .as_f64()
                    .ok_or_else(|| format!("expected numbers in color {value}"))?
                    as f32;
            }
            // Accept 0..255 colors too.
            if out.iter().any(|channel| *channel > 1.0) {
                for channel in &mut out {
                    *channel /= 255.0;
                }
                if items.len() == 3 {
                    out[3] = 1.0;
                }
            }
            Ok(out.map(|channel| channel.clamp(0.0, 1.0)))
        }
        _ => Err(format!(
            "expected a color like [r, g, b] (0..1) or \"#rrggbb\", got {value}"
        )),
    }
}

pub(crate) fn color_json(color: [f32; 4]) -> Value {
    json!(color.map(round))
}

/// The id remote clients and Lua scripts use for `entity`.
pub fn entity_id(entity: Entity) -> u64 {
    entity.to_bits().get()
}

pub(crate) fn entity_name(world: &World, entity: Entity) -> Option<String> {
    world.get::<&Name>(entity).ok().map(|name| name.0.clone())
}

/// Resolve an entity reference: a numeric id, a numeric string, or a name.
///
/// Names match exactly first, then case-insensitively; an ambiguous name is an error
/// that lists the candidates.
pub fn entity_from_json(world: &World, value: &Value) -> Result<Entity, String> {
    let by_id = |id: u64| {
        Entity::from_bits(id)
            .filter(|entity| world.contains(*entity))
            .ok_or_else(|| format!("no entity with id {id}"))
    };

    match value {
        Value::Number(number) => {
            let id = number
                .as_u64()
                .ok_or_else(|| format!("invalid entity id {number}"))?;
            by_id(id)
        }
        Value::String(text) => {
            if let Ok(id) = text.trim().parse::<u64>() {
                if let Ok(entity) = by_id(id) {
                    return Ok(entity);
                }
            }
            let named: Vec<(Entity, String)> = world
                .query::<&Name>()
                .iter()
                .map(|(entity, name)| (entity, name.0.clone()))
                .collect();
            let exact: Vec<Entity> = named
                .iter()
                .filter(|(_, name)| name == text)
                .map(|(entity, _)| *entity)
                .collect();
            let candidates = if exact.is_empty() {
                named
                    .iter()
                    .filter(|(_, name)| name.eq_ignore_ascii_case(text))
                    .map(|(entity, _)| *entity)
                    .collect()
            } else {
                exact
            };
            match candidates.as_slice() {
                [entity] => Ok(*entity),
                [] => Err(format!("no entity named {text:?}")),
                many => Err(format!(
                    "{} entities are named {text:?}; use an id: {}",
                    many.len(),
                    many.iter()
                        .map(|entity| entity_id(*entity).to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            }
        }
        _ => Err(format!("expected an entity id or name, got {value}")),
    }
}

/// Resolve the required `entity` parameter.
pub(crate) fn req_entity(world: &World, params: &Value) -> Result<Entity, String> {
    let value = params
        .get("entity")
        .ok_or("missing parameter \"entity\" (an id or a name)")?;
    entity_from_json(world, value)
}

/// Deep-merge `patch` into `target`: objects merge key by key, anything else replaces.
pub(crate) fn merge_json(target: &mut Value, patch: &Value) {
    match (target, patch) {
        (Value::Object(target), Value::Object(patch)) => {
            for (key, value) in patch {
                merge_json(target.entry(key.clone()).or_insert(Value::Null), value);
            }
        }
        (target, patch) => *target = patch.clone(),
    }
}

/// Serialize `current`, merge `patch` into it, and deserialize the result.
pub(crate) fn patch_serde<T>(current: &T, patch: &Value) -> Result<T, String>
where
    T: serde::Serialize + serde::de::DeserializeOwned,
{
    let mut value = serde_json::to_value(current).map_err(|err| err.to_string())?;
    merge_json(&mut value, patch);
    serde_json::from_value(value).map_err(|err| format!("invalid value: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_round_trips() {
        let degrees = Vec3::new(30.0, 45.0, -10.0);
        let back = quat_to_euler_deg(euler_deg_to_quat(degrees));
        assert!((back - degrees).abs().max_element() < 1e-3, "{back:?}");
    }

    #[test]
    fn colors_parse_from_hex_and_arrays() {
        assert_eq!(
            color_from_json(&json!("#ff0000")).unwrap(),
            [1.0, 0.0, 0.0, 1.0]
        );
        assert_eq!(
            color_from_json(&json!([0.5, 0.25, 1.0])).unwrap(),
            [0.5, 0.25, 1.0, 1.0]
        );
        let bytes = color_from_json(&json!([255, 0, 0])).unwrap();
        assert_eq!(bytes, [1.0, 0.0, 0.0, 1.0]);
        assert!(color_from_json(&json!("red")).is_err());
    }

    #[test]
    fn look_rotation_points_forward_at_target() {
        let rotation = look_rotation(Vec3::ZERO, Vec3::new(3.0, 0.0, 0.0)).unwrap();
        let forward = rotation * Vec3::NEG_Z;
        assert!((forward - Vec3::X).length() < 1e-5, "{forward:?}");

        let rotation = look_rotation(Vec3::ZERO, Vec3::new(0.0, -1.0, -1.0)).unwrap();
        let forward = rotation * Vec3::NEG_Z;
        assert!((forward - Vec3::new(0.0, -1.0, -1.0).normalize()).length() < 1e-5);
    }

    #[test]
    fn entities_resolve_by_id_and_name() {
        let mut world = World::new();
        let a = world.spawn((Name::new("Player"),));
        let b = world.spawn((Name::new("Enemy"),));
        world.spawn((Name::new("Enemy"),));

        assert_eq!(entity_from_json(&world, &json!(entity_id(a))).unwrap(), a);
        assert_eq!(entity_from_json(&world, &json!("player")).unwrap(), a);
        assert_eq!(
            entity_from_json(&world, &json!(entity_id(b).to_string())).unwrap(),
            b
        );
        let err = entity_from_json(&world, &json!("Enemy")).unwrap_err();
        assert!(err.contains("2 entities"), "{err}");
        assert!(entity_from_json(&world, &json!("Nobody")).is_err());
    }

    #[test]
    fn patch_serde_merges_nested_fields() {
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Inner {
            a: f32,
            b: f32,
        }
        #[derive(serde::Serialize, serde::Deserialize, PartialEq, Debug)]
        struct Outer {
            inner: Inner,
            flag: bool,
        }
        let current = Outer {
            inner: Inner { a: 1.0, b: 2.0 },
            flag: false,
        };
        let patched = patch_serde(&current, &json!({ "inner": { "b": 5.0 } })).unwrap();
        assert_eq!(
            patched,
            Outer {
                inner: Inner { a: 1.0, b: 5.0 },
                flag: false
            }
        );
    }
}
