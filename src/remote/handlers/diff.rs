//! Scene baselines and diffs: what changed since a moment the client marked.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::super::json::{entity_id, opt_u64};
use super::{entity, RemoteContext, RemoteResult};

/// Differences below this are float noise, not changes.
const EPSILON: f64 = 1e-4;

/// Per-entity state keyed by id, as `entity.get` reports it.
pub(crate) type SceneBaseline = BTreeMap<u64, Value>;

fn capture(ctx: &mut RemoteContext<'_, '_>) -> SceneBaseline {
    let entities: Vec<u64> = ctx
        .gpu
        .scene
        .main_world()
        .iter()
        .map(|entity| entity_id(entity.entity()))
        .collect();
    entities
        .into_iter()
        .filter_map(|id| {
            let mut state = entity::get(&json!({ "entity": id }), ctx).ok()?;
            // Bookkeeping that changes whenever anything does; the real fields cover it.
            if let Some(material) = state.get_mut("material").and_then(Value::as_object_mut) {
                material.remove("shared_by");
            }
            Some((id, state))
        })
        .collect()
}

pub(super) fn mark(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let baseline = capture(ctx);
    let count = baseline.len();
    ctx.hub.set_scene_baseline(baseline);
    Ok(json!({
        "marked": count,
        "note": "scene.diff now reports changes relative to this moment. Stopping play restores the scene, so diff before stopping.",
    }))
}

pub(super) fn diff(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let limit = opt_u64(params, "limit").unwrap_or(100) as usize;
    let baseline = ctx
        .hub
        .scene_baseline()
        .ok_or("no baseline yet; call scene.mark first")?
        .clone();
    let current = capture(ctx);
    Ok(compare(&baseline, &current, limit))
}

fn summary(id: u64, state: &Value) -> Value {
    json!({ "id": id, "name": state["name"] })
}

fn compare(before: &SceneBaseline, after: &SceneBaseline, limit: usize) -> Value {
    let added: Vec<Value> = after
        .iter()
        .filter(|(id, _)| !before.contains_key(id))
        .map(|(id, state)| summary(*id, state))
        .collect();
    let removed: Vec<Value> = before
        .iter()
        .filter(|(id, _)| !after.contains_key(id))
        .map(|(id, state)| summary(*id, state))
        .collect();
    let mut changed = Vec::new();
    for (id, old) in before {
        let Some(new) = after.get(id) else {
            continue;
        };
        let mut fields = serde_json::Map::new();
        diff_values("", old, new, &mut fields);
        if !fields.is_empty() {
            let mut entry = summary(*id, new);
            entry["changes"] = Value::Object(fields);
            changed.push(entry);
        }
    }
    let changed_total = changed.len();
    changed.truncate(limit);
    json!({
        "added": added,
        "removed": removed,
        "changed_total": changed_total,
        "changed": changed,
    })
}

/// Record every leaf that differs between `old` and `new` as `path: [old, new]`.
fn diff_values(path: &str, old: &Value, new: &Value, out: &mut serde_json::Map<String, Value>) {
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => {
            for (key, value) in a {
                let child = join(path, key);
                diff_values(&child, value, b.get(key).unwrap_or(&Value::Null), out);
            }
            for (key, value) in b {
                if !a.contains_key(key) {
                    diff_values(&join(path, key), &Value::Null, value, out);
                }
            }
        }
        // Small numeric arrays (vectors, colors) read best as one value.
        (Value::Array(a), Value::Array(b))
            if a.len() == b.len() && !a.iter().all(Value::is_number) =>
        {
            for (index, (a, b)) in a.iter().zip(b).enumerate() {
                diff_values(&join(path, &index.to_string()), a, b, out);
            }
        }
        _ if same(old, new) => {}
        _ => {
            out.insert(path.to_owned(), json!([old, new]));
        }
    }
}

fn join(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_owned()
    } else {
        format!("{path}.{key}")
    }
}

fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(a), Value::Number(b)) => match (a.as_f64(), b.as_f64()) {
            (Some(a), Some(b)) => (a - b).abs() <= EPSILON,
            _ => a == b,
        },
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| same(a, b))
        }
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_added_removed_and_changed_fields() {
        let before: SceneBaseline = [
            (1, json!({ "name": "Ball", "transform": { "translation": [0.0, 0.0, 0.0] }, "visible": true })),
            (2, json!({ "name": "Brick", "visible": true })),
        ]
        .into();
        let after: SceneBaseline = [
            (1, json!({ "name": "Ball", "transform": { "translation": [1.0, 0.00001, 0.0] }, "visible": true })),
            (3, json!({ "name": "Spark" })),
        ]
        .into();
        let diff = compare(&before, &after, 10);
        assert_eq!(diff["added"][0]["id"], 3);
        assert_eq!(diff["removed"][0]["name"], "Brick");
        assert_eq!(diff["changed_total"], 1);
        let changes = &diff["changed"][0]["changes"];
        assert_eq!(changes["transform.translation"][1][0], 1.0);
        assert_eq!(changes.as_object().unwrap().len(), 1);
    }
}
