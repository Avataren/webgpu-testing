//! The `AiAgent` component: which brain an entity runs and how it perceives. Plain
//! data, saved with the scene; [`super::SceneAi`] runs it in play mode.

use hecs::{Entity, World};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use super::perception::PerceptionSettings;
use crate::remote::json::patch_serde;

/// An entity with a brain: a behaviour tree or state machine by name (defined by a
/// script with `ai_tree` / `ai_machine`, or a JSON file in the project such as
/// `content/ai/guard.json`). Walking needs a `NavAgent` too; without one the brain
/// still thinks, plays animations and sends events.
///
/// ```
/// use wgpu_cube::ai::AiAgent;
///
/// let guard = AiAgent {
///     brain: "guard".into(),
///     tags: vec!["guard".into()],
///     ..Default::default()
/// };
/// assert!(guard.enabled);
/// assert_eq!(guard.tick_rate, 10.0);
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AiAgent {
    /// The brain's name, or a project path to a JSON brain.
    pub brain: String,
    pub enabled: bool,
    /// Thinks per second (0: every frame). Agents are spread over frames.
    pub tick_rate: f32,
    /// What others perceive it as (`guard`, `child`, `thief`).
    pub tags: Vec<String>,
    /// Agents of a group share the `group.` blackboard and `to = "group"` events.
    pub group: String,
    /// Speed of `run = true` moves (m/s).
    pub run_speed: f32,
    /// Starting blackboard values (`[x, y, z]` arrays become positions).
    pub blackboard: Json,
    pub perception: PerceptionSettings,
}

impl Default for AiAgent {
    fn default() -> Self {
        Self {
            brain: String::new(),
            enabled: true,
            tick_rate: 10.0,
            tags: Vec::new(),
            group: String::new(),
            run_speed: 3.2,
            blackboard: Json::Object(Default::default()),
            perception: PerceptionSettings::default(),
        }
    }
}

/// The component's name in saved scenes and the remote API.
pub const COMPONENT_NAME: &str = "ai_agent";

/// `"ai_agent"` from that or its type name (`"AiAgent"`).
pub fn canonical_name(name: &str) -> Option<&'static str> {
    match name {
        "ai_agent" | "AiAgent" => Some(COMPONENT_NAME),
        _ => None,
    }
}

pub fn component_json(world: &World, entity: Entity) -> Option<Json> {
    let component = world.get::<&AiAgent>(entity).ok()?;
    serde_json::to_value(&*component).ok()
}

pub fn has_component(world: &World, entity: Entity) -> bool {
    world.satisfies::<&AiAgent>(entity).unwrap_or(false)
}

/// Add the component or change the fields `value` gives; `null` removes it.
///
/// ```
/// use wgpu_cube::ai::{edit, AiAgent};
///
/// let mut world = hecs::World::new();
/// let e = world.spawn(());
/// edit::patch_component(&mut world, e, &serde_json::json!({"brain": "guard"})).unwrap();
/// edit::patch_component(&mut world, e, &serde_json::json!({"perception": {"sight_range": 12}})).unwrap();
/// let agent = world.get::<&AiAgent>(e).unwrap();
/// assert_eq!(agent.brain, "guard");
/// assert_eq!(agent.perception.sight_range, 12.0);
/// ```
pub fn patch_component(world: &mut World, entity: Entity, value: &Json) -> Result<(), String> {
    if value.is_null() {
        return world
            .remove_one::<AiAgent>(entity)
            .map(|_| ())
            .map_err(|err| err.to_string());
    }
    if !value.is_object() {
        return Err(format!("expected an object or null, got {value}"));
    }
    let current = world
        .get::<&AiAgent>(entity)
        .map(|component| (*component).clone())
        .unwrap_or_default();
    let mut patched: AiAgent = patch_serde(&current, value)?;
    // The blackboard merges key by key rather than being replaced.
    if let (Json::Object(old), Some(Json::Object(new))) =
        (&current.blackboard, value.get("blackboard"))
    {
        let mut merged = old.clone();
        for (k, v) in new {
            if v.is_null() {
                merged.remove(k);
            } else {
                merged.insert(k.clone(), v.clone());
            }
        }
        patched.blackboard = Json::Object(merged);
    }
    world
        .insert_one(entity, patched)
        .map_err(|err| err.to_string())
}

/// Copy the component of `source` into `builder` (entity duplication).
pub fn clone_components(world: &World, source: Entity, builder: &mut hecs::EntityBuilder) {
    if let Ok(component) = world.get::<&AiAgent>(source) {
        builder.add((*component).clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn blackboard_patches_merge_and_null_removes() {
        let mut world = World::new();
        let e = world.spawn(());
        patch_component(&mut world, e, &json!({"blackboard": {"a": 1, "b": 2}})).unwrap();
        patch_component(&mut world, e, &json!({"blackboard": {"b": null, "c": 3}})).unwrap();
        let bb = world.get::<&AiAgent>(e).unwrap().blackboard.clone();
        assert_eq!(bb, json!({"a": 1, "c": 3}));
        patch_component(&mut world, e, &Json::Null).unwrap();
        assert!(!has_component(&world, e));
        assert!(patch_component(&mut world, e, &json!(3)).is_err());
    }

    #[test]
    fn names() {
        assert_eq!(canonical_name("AiAgent"), Some("ai_agent"));
        assert_eq!(canonical_name("nav_agent"), None);
    }
}
