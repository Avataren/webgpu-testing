//! Scene-only transactions run against a speculative snapshot and commit once.

use super::{RemoteContext, RemoteResult};
use crate::app::RuntimeMode;
use crate::scene::SceneStateSnapshot;
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub(super) fn allowed(method: &str) -> bool {
    matches!(
        method,
        "entity.spawn"
            | "entity.update"
            | "entity.delete"
            | "entity.duplicate"
            | "entity.remove_component"
            | "entity.get"
            | "scene.entities"
            | "scene.tree"
            | "scene.query"
            | "scene.raycast"
            | "entity.place"
            | "scene.scatter"
            | "mesh.lathe"
            | "mesh.extrude"
            | "mesh.compose"
            | "mesh.sweep"
            | "mesh.heightfield"
            | "camera.set"
            | "camera.get"
            | "environment.set"
            | "environment.get"
            | "postprocess.set"
            | "postprocess.get"
    )
}

/// References have the form {"$ref":"alias/id"}; aliases name prior results.
pub(super) fn resolve(value: &Value, aliases: &BTreeMap<String, Value>) -> Result<Value, String> {
    match value {
        Value::Object(object) if object.contains_key("$ref") => {
            if object.len() != 1 {
                return Err("$ref objects cannot contain other fields".into());
            }
            let reference = object["$ref"].as_str().ok_or("$ref must be a string")?;
            let (alias, pointer) = reference.split_once('/').unwrap_or((reference, ""));
            let result = aliases
                .get(alias)
                .ok_or_else(|| format!("unknown or forward alias {alias:?}"))?;
            if pointer.is_empty() {
                return Ok(result.clone());
            }
            result
                .pointer(&format!("/{pointer}"))
                .cloned()
                .ok_or_else(|| format!("reference {reference:?} does not exist"))
        }
        Value::Object(object) => object
            .iter()
            .map(|(k, v)| Ok((k.clone(), resolve(v, aliases)?)))
            .collect::<Result<serde_json::Map<_, _>, String>>()
            .map(Value::Object),
        Value::Array(array) => array
            .iter()
            .map(|v| resolve(v, aliases))
            .collect::<Result<Vec<_>, _>>()
            .map(Value::Array),
        _ => Ok(value.clone()),
    }
}

pub(super) fn run(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    if ctx.runtime_mode() != Some(RuntimeMode::Editor) {
        return Err("scene transactions require editor mode".into());
    }
    let calls = params
        .get("calls")
        .and_then(Value::as_array)
        .ok_or("calls must be an array")?;
    if calls.is_empty() || calls.len() > 1000 {
        return Err("a transaction needs 1..=1000 calls".into());
    }
    let mut names = std::collections::BTreeSet::new();
    for call in calls {
        let method = call["method"]
            .as_str()
            .ok_or("every transaction call needs method")?;
        if !allowed(method) {
            return Err(format!(
                "{method:?} is not a scene-only transactional method"
            ));
        }
        if let Some(alias) = call.get("as") {
            let alias = alias
                .as_str()
                .filter(|s| !s.is_empty() && !s.contains('/'))
                .ok_or("aliases must be nonempty strings without / ")?;
            if !names.insert(alias) {
                return Err(format!("duplicate alias {alias:?}"));
            }
        }
    }
    let trial = SceneStateSnapshot::capture(&ctx.gpu.scene).into_scene();
    let original = std::mem::replace(&mut *ctx.gpu.scene, trial);
    let prior_modified = ctx.take_modified();
    let mut aliases = BTreeMap::new();
    let mut results = Vec::new();
    let mut failure = None;
    for (index, call) in calls.iter().enumerate() {
        let method = call["method"].as_str().unwrap();
        let result = resolve(call.get("params").unwrap_or(&json!({})), &aliases)
            .and_then(|params| super::handle(method, &params, ctx));
        match result {
            Ok(result) => {
                if let Some(alias) = call["as"].as_str() {
                    aliases.insert(alias.to_string(), result.clone());
                }
                results.push(json!({"method":method,"result":result}));
            }
            Err(error) => {
                failure = Some(json!({"index":index,"method":method,"message":error}));
                break;
            }
        }
    }
    let dry_run = params
        .get("dry_run")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let changed = ctx.take_modified();
    let committed = failure.is_none() && !dry_run;
    if !committed {
        *ctx.gpu.scene = original;
    }
    ctx.gpu
        .renderer
        .update_texture_bind_group(&ctx.gpu.scene.assets);
    if prior_modified || committed && changed {
        ctx.mark_modified();
    }
    Ok(
        json!({"ok":failure.is_none(),"committed":committed,"dry_run":dry_run,
        "rolled_back":!committed,"failure":failure,"results":results,
        "note":if dry_run {"Returned entity ids are speculative and cannot be used after this call"} else {"Scene edits commit together as one undo step; file/process/audio/Lua operations are excluded"}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aliases_resolve_nested_values_and_reject_forward_references() {
        let aliases = BTreeMap::from([("root".into(), json!({"id":42,"children":[7]}))]);
        assert_eq!(
            resolve(
                &json!({"parent":{"$ref":"root/id"},"entities":[{"$ref":"root/children/0"}]}),
                &aliases
            )
            .unwrap(),
            json!({"parent":42,"entities":[7]})
        );
        assert!(resolve(&json!({"$ref":"missing/id"}), &aliases).is_err());
        assert!(!allowed("lua.eval"));
        assert!(!allowed("texture.generate"));
        assert!(!allowed("project.save"));
    }
}
