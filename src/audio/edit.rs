//! Reading and changing the audio components by name, as JSON: shared by the
//! remote API (`entity_get` / `entity_update`), Lua (`get_component` /
//! `set_component`) and the inspector.

use hecs::{Entity, World};
use serde_json::Value;

use super::components::{AudioListener, AudioSource, AudioVolume};
use crate::remote::json::patch_serde;

/// The audio components' field names in saved scenes and the remote API.
pub const COMPONENT_NAMES: [&str; 3] = ["audio_source", "audio_listener", "audio_volume"];

/// The snake_case name of an audio component, from that or its type name
/// (`"AudioSource"`).
pub fn canonical_name(name: &str) -> Option<&'static str> {
    Some(match name {
        "audio_source" | "AudioSource" => "audio_source",
        "audio_listener" | "AudioListener" => "audio_listener",
        "audio_volume" | "AudioVolume" => "audio_volume",
        _ => return None,
    })
}

fn get<T: hecs::Component + serde::Serialize>(world: &World, entity: Entity) -> Option<Value> {
    let component = world.get::<&T>(entity).ok()?;
    serde_json::to_value(&*component).ok()
}

pub fn component_json(world: &World, entity: Entity, name: &str) -> Option<Value> {
    match canonical_name(name)? {
        "audio_source" => get::<AudioSource>(world, entity),
        "audio_listener" => get::<AudioListener>(world, entity),
        "audio_volume" => get::<AudioVolume>(world, entity),
        _ => None,
    }
}

pub fn has_component(world: &World, entity: Entity, name: &str) -> bool {
    match canonical_name(name) {
        Some("audio_source") => world.satisfies::<&AudioSource>(entity),
        Some("audio_listener") => world.satisfies::<&AudioListener>(entity),
        Some("audio_volume") => world.satisfies::<&AudioVolume>(entity),
        _ => Ok(false),
    }
    .unwrap_or(false)
}

fn patch<T>(world: &mut World, entity: Entity, value: &Value) -> Result<(), String>
where
    T: hecs::Component + Clone + Default + serde::Serialize + serde::de::DeserializeOwned,
{
    if value.is_null() {
        return world
            .remove_one::<T>(entity)
            .map(|_| ())
            .map_err(|err| err.to_string());
    }
    if !value.is_object() {
        return Err(format!("expected an object or null, got {value}"));
    }
    let current = world
        .get::<&T>(entity)
        .map(|component| (*component).clone())
        .unwrap_or_default();
    let patched = patch_serde(&current, value)?;
    world
        .insert_one(entity, patched)
        .map_err(|err| err.to_string())
}

/// Add the named component or change the fields `value` gives (the rest keep
/// their values, or the defaults for a new component); `null` removes it.
pub fn patch_component(
    world: &mut World,
    entity: Entity,
    name: &str,
    value: &Value,
) -> Result<(), String> {
    match canonical_name(name) {
        Some("audio_source") => patch::<AudioSource>(world, entity, value),
        Some("audio_listener") => patch::<AudioListener>(world, entity, value),
        Some("audio_volume") => patch::<AudioVolume>(world, entity, value),
        _ => Err(format!("{name:?} is not an audio component")),
    }
}

/// Copy the audio components of `source` into `builder` (entity duplication).
pub fn clone_components(world: &World, source: Entity, builder: &mut hecs::EntityBuilder) {
    if let Ok(component) = world.get::<&AudioSource>(source) {
        builder.add((*component).clone());
    }
    if let Ok(component) = world.get::<&AudioListener>(source) {
        builder.add((*component).clone());
    }
    if let Ok(component) = world.get::<&AudioVolume>(source) {
        builder.add((*component).clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn patches_merge_and_null_removes() {
        let mut world = World::new();
        let entity = world.spawn(());
        patch_component(
            &mut world,
            entity,
            "AudioSource",
            &json!({"clip": "content/audio/hum.wav"}),
        )
        .unwrap();
        patch_component(
            &mut world,
            entity,
            "audio_source",
            &json!({"spatial": "2d", "loop": true}),
        )
        .unwrap();
        {
            let source = world.get::<&AudioSource>(entity).unwrap();
            assert_eq!(source.clip, "content/audio/hum.wav");
            assert!(source.looped);
            assert_eq!(
                source.spatial,
                super::super::components::SpatialSetting::Planar
            );
            assert_eq!(source.bus, "sfx");
        }
        assert_eq!(
            component_json(&world, entity, "audio_source").unwrap()["spatial"],
            "2d"
        );
        patch_component(&mut world, entity, "audio_source", &Value::Null).unwrap();
        assert!(!has_component(&world, entity, "audio_source"));
        assert!(patch_component(
            &mut world,
            entity,
            "audio_source",
            &json!({"spatial": "4d"})
        )
        .is_err());
    }
}
