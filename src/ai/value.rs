//! What a blackboard holds: numbers, flags, text, positions, entities (as their id bits,
//! the integers Lua scripts hold), plain data and, from Lua, tables and functions kept
//! by reference.

use std::fmt;
use std::sync::Arc;

use glam::Vec3;
use hecs::Entity;

/// A Lua value (table, function) a blackboard keeps by reference: Lua code reading it
/// back gets the same table, not a copy. Rust code sees it as opaque.
pub struct LuaRef(pub mlua::RegistryKey);

impl fmt::Debug for LuaRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LuaRef({})", self.0.id())
    }
}

/// A blackboard value.
///
/// ```
/// use wgpu_cube::ai::Value;
///
/// let json = serde_json::json!({ "home": [1.0, 0.0, 2.0], "hungry": true, "coins": 3 });
/// let home = Value::from_json(&json["home"]);
/// assert_eq!(home.as_vec3(), Some(glam::Vec3::new(1.0, 0.0, 2.0)));
/// assert!(Value::from_json(&json["hungry"]).truthy());
/// assert_eq!(Value::from_json(&json["coins"]).as_f64(), Some(3.0));
/// assert!(!Value::Nil.truthy());
/// ```
#[derive(Debug, Clone, Default)]
pub enum Value {
    #[default]
    Nil,
    Bool(bool),
    /// An integer; entities are stored as their id bits.
    Int(i64),
    Num(f64),
    Str(Arc<str>),
    Vec3(Vec3),
    /// Plain data (objects, lists) from JSON.
    Json(Arc<serde_json::Value>),
    /// A Lua table or function, by reference.
    Lua(Arc<LuaRef>),
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Nil, Value::Nil) => true,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Vec3(a), Value::Vec3(b)) => a == b,
            (Value::Json(a), Value::Json(b)) => a == b,
            (Value::Lua(a), Value::Lua(b)) => Arc::ptr_eq(a, b) || a.0.id() == b.0.id(),
            (a, b) => match (a.as_f64(), b.as_f64()) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
        }
    }
}

impl Value {
    pub fn str(text: impl AsRef<str>) -> Self {
        Value::Str(Arc::from(text.as_ref()))
    }

    pub fn entity(entity: Entity) -> Self {
        Value::Int(entity.to_bits().get() as i64)
    }

    pub fn is_nil(&self) -> bool {
        matches!(self, Value::Nil)
    }

    /// Lua's truth: everything but nil and false.
    pub fn truthy(&self) -> bool {
        !matches!(self, Value::Nil | Value::Bool(false))
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(v) => Some(*v as f64),
            Value::Num(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_vec3(&self) -> Option<Vec3> {
        match self {
            Value::Vec3(v) => Some(*v),
            Value::Json(json) => vec3_from_json(json),
            _ => None,
        }
    }

    /// The entity an integer value names.
    pub fn as_entity(&self) -> Option<Entity> {
        match self {
            Value::Int(bits) if *bits > 0 => Entity::from_bits(*bits as u64),
            Value::Num(bits) if *bits > 0.0 && bits.fract() == 0.0 => {
                Entity::from_bits(*bits as u64)
            }
            _ => None,
        }
    }

    /// A short name of the kind of value, for messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Nil => "nil",
            Value::Bool(_) => "boolean",
            Value::Int(_) | Value::Num(_) => "number",
            Value::Str(_) => "string",
            Value::Vec3(_) => "vector",
            Value::Json(_) => "data",
            Value::Lua(_) => "lua",
        }
    }

    /// From JSON: arrays of three numbers become vectors, integers stay integers.
    pub fn from_json(json: &serde_json::Value) -> Self {
        match json {
            serde_json::Value::Null => Value::Nil,
            serde_json::Value::Bool(b) => Value::Bool(*b),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => Value::Int(i),
                None => Value::Num(n.as_f64().unwrap_or(0.0)),
            },
            serde_json::Value::String(s) => Value::str(s),
            // `[x, y, z]` and `{x, y, z}` are positions.
            other => match vec3_from_json(other) {
                Some(v) => Value::Vec3(v),
                None => Value::Json(Arc::new(other.clone())),
            },
        }
    }

    /// As JSON (vectors as `[x, y, z]`; Lua values by their kind only).
    pub fn to_json(&self) -> serde_json::Value {
        match self {
            Value::Nil => serde_json::Value::Null,
            Value::Bool(b) => serde_json::Value::Bool(*b),
            Value::Int(i) => serde_json::json!(i),
            Value::Num(n) => serde_json::Number::from_f64(*n)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null),
            Value::Str(s) => serde_json::Value::String(s.to_string()),
            Value::Vec3(v) => serde_json::json!([v.x, v.y, v.z]),
            Value::Json(json) => (**json).clone(),
            Value::Lua(_) => serde_json::Value::String("<lua>".into()),
        }
    }

    /// A field of a data value (`{"spot": {"x": 1}}` → `spot.x`), or nil.
    pub fn json_field(&self, name: &str) -> Value {
        match self {
            Value::Json(json) => match &**json {
                serde_json::Value::Object(map) => {
                    map.get(name).map(Value::from_json).unwrap_or_default()
                }
                serde_json::Value::Array(items) => name
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| i.checked_sub(1))
                    .and_then(|i| items.get(i))
                    .map(Value::from_json)
                    .unwrap_or_default(),
                _ => Value::Nil,
            },
            Value::Vec3(v) => match name {
                "x" => Value::Num(v.x as f64),
                "y" => Value::Num(v.y as f64),
                "z" => Value::Num(v.z as f64),
                _ => Value::Nil,
            },
            _ => Value::Nil,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Nil => write!(f, "nil"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    write!(f, "{n:.0}")
                } else {
                    write!(f, "{}", (n * 1000.0).round() / 1000.0)
                }
            }
            Value::Str(s) => write!(f, "{s}"),
            Value::Vec3(v) => write!(f, "({:.2}, {:.2}, {:.2})", v.x, v.y, v.z),
            Value::Json(json) => write!(f, "{json}"),
            Value::Lua(_) => write!(f, "<lua>"),
        }
    }
}

impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Num(v)
    }
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::str(v)
    }
}

impl From<Vec3> for Value {
    fn from(v: Vec3) -> Self {
        Value::Vec3(v)
    }
}

/// A hasher for maps keyed by entities: their ids are already well spread, so a multiply
/// is enough (the default SipHash showed up in profiles of large crowds).
#[derive(Default, Clone, Copy)]
pub struct EntityHasher(u64);

impl std::hash::Hasher for EntityHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64);
        }
    }

    fn write_u32(&mut self, n: u32) {
        self.write_u64(n as u64);
    }

    fn write_u64(&mut self, n: u64) {
        self.0 = (self.0.rotate_left(26) ^ n).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}

/// A map keyed by entities.
pub type EntityMap<V> =
    std::collections::HashMap<Entity, V, std::hash::BuildHasherDefault<EntityHasher>>;

/// A set of entities.
pub type EntitySet = std::collections::HashSet<Entity, std::hash::BuildHasherDefault<EntityHasher>>;

/// `[x, y, z]` or `{"x", "y", "z"}`.
pub(crate) fn vec3_from_json(json: &serde_json::Value) -> Option<Vec3> {
    match json {
        serde_json::Value::Array(items) if items.len() == 3 => Some(Vec3::new(
            items[0].as_f64()? as f32,
            items[1].as_f64()? as f32,
            items[2].as_f64()? as f32,
        )),
        serde_json::Value::Object(map) if map.len() == 3 => Some(Vec3::new(
            map.get("x")?.as_f64()? as f32,
            map.get("y")?.as_f64()? as f32,
            map.get("z")?.as_f64()? as f32,
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_compare_across_kinds() {
        assert_eq!(Value::Int(2), Value::Num(2.0));
        assert_ne!(Value::Int(2), Value::str("2"));
        assert_eq!(Value::Nil, Value::Nil);
    }

    #[test]
    fn json_round_trip_keeps_shapes() {
        let json = serde_json::json!({"a": [1, 2, 3], "b": {"c": 1}, "d": [1, 2]});
        assert!(matches!(Value::from_json(&json["a"]), Value::Vec3(_)));
        let b = Value::from_json(&json["b"]);
        assert_eq!(b.json_field("c"), Value::Int(1));
        let d = Value::from_json(&json["d"]);
        assert_eq!(d.json_field("2"), Value::Int(2));
        assert_eq!(Value::from_json(&json).to_json()["b"], json["b"]);
    }
}
