//! Blackboards: the named values an agent's brain reads and writes (its target, where
//! home is, how hungry it is). Each agent has its own; agents of a group share a group
//! board, and every agent sees the scene's global board.
//!
//! In expressions and node parameters, `name` is the agent's own key, `group.name` its
//! group's and `global.name` the scene's.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use super::value::Value;

/// A blackboard's values.
///
/// ```
/// use wgpu_cube::ai::{Blackboard, Value};
///
/// let mut board = Blackboard::default();
/// board.set("hunger", Value::Num(0.4));
/// assert_eq!(board.get("hunger").as_f64(), Some(0.4));
/// board.set("hunger", Value::Nil); // nil removes the key
/// assert!(board.get("hunger").is_nil());
/// ```
#[derive(Debug, Default, Clone)]
pub struct Blackboard {
    values: HashMap<String, Value>,
    /// Counts changes, so watchers can tell something changed.
    serial: u64,
}

impl Blackboard {
    pub fn get(&self, key: &str) -> Value {
        self.values.get(key).cloned().unwrap_or_default()
    }

    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    /// Set `key` (nil removes it).
    pub fn set(&mut self, key: impl Into<String>, value: Value) {
        let key = key.into();
        if value.is_nil() {
            if self.values.remove(&key).is_some() {
                self.serial += 1;
            }
            return;
        }
        if self.values.get(&key) != Some(&value) {
            self.serial += 1;
        }
        self.values.insert(key, value);
    }

    pub fn remove(&mut self, key: &str) -> Value {
        let removed = self.values.remove(key).unwrap_or_default();
        if !removed.is_nil() {
            self.serial += 1;
        }
        removed
    }

    pub fn clear(&mut self) {
        if !self.values.is_empty() {
            self.serial += 1;
        }
        self.values.clear();
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn serial(&self) -> u64 {
        self.serial
    }

    /// The keys, sorted.
    pub fn keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.values.keys().cloned().collect();
        keys.sort();
        keys
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.values.iter()
    }

    /// Every value as JSON, keys sorted.
    pub fn to_json(&self) -> serde_json::Value {
        let mut map = serde_json::Map::new();
        for key in self.keys() {
            map.insert(key.clone(), self.values[&key].to_json());
        }
        serde_json::Value::Object(map)
    }

    /// Set every key of a JSON object.
    pub fn merge_json(&mut self, json: &serde_json::Value) {
        if let serde_json::Value::Object(map) = json {
            for (key, value) in map {
                self.set(key.clone(), Value::from_json(value));
            }
        }
    }
}

/// A blackboard shared between the brain, Lua and tools.
#[derive(Debug, Default, Clone)]
pub struct BlackboardRef(Arc<Mutex<Blackboard>>);

impl BlackboardRef {
    pub fn new(board: Blackboard) -> Self {
        Self(Arc::new(Mutex::new(board)))
    }

    pub fn lock(&self) -> MutexGuard<'_, Blackboard> {
        // A panic while holding the lock leaves the values as they were; keep using them.
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn get(&self, key: &str) -> Value {
        self.lock().get(key)
    }

    pub fn set(&self, key: impl Into<String>, value: Value) {
        self.lock().set(key, value);
    }

    pub fn ptr_eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// Which board a key names: `group.x` the agent's group's, `global.x` the scene's, `x`
/// the agent's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardScope {
    Agent,
    Group,
    Global,
}

/// Split a key into its board and its name on that board.
///
/// ```
/// use wgpu_cube::ai::{split_key, BoardScope};
///
/// assert_eq!(split_key("group.alarm"), (BoardScope::Group, "alarm"));
/// assert_eq!(split_key("target"), (BoardScope::Agent, "target"));
/// ```
pub fn split_key(key: &str) -> (BoardScope, &str) {
    if let Some(rest) = key.strip_prefix("group.") {
        (BoardScope::Group, rest)
    } else if let Some(rest) = key.strip_prefix("global.") {
        (BoardScope::Global, rest)
    } else {
        (BoardScope::Agent, key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_counts_changes_only() {
        let mut board = Blackboard::default();
        board.set("a", Value::Int(1));
        let s = board.serial();
        board.set("a", Value::Int(1));
        assert_eq!(board.serial(), s);
        board.set("a", Value::Int(2));
        assert_eq!(board.serial(), s + 1);
        board.remove("missing");
        assert_eq!(board.serial(), s + 1);
    }

    #[test]
    fn json_merge_and_dump() {
        let mut board = Blackboard::default();
        board.merge_json(&serde_json::json!({"home": [1, 2, 3], "name": "Ada"}));
        assert_eq!(board.get("name").as_str(), Some("Ada"));
        assert_eq!(board.to_json()["home"], serde_json::json!([1.0, 2.0, 3.0]));
    }
}
