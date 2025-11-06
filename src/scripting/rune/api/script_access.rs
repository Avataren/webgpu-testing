/// Script access API for RuneScript
///
/// Provides functions to query script components on entities.
/// This allows scripts to build tools like script editors.

use crate::scripting::rune::component::RuneScriptComponent;
use crate::scripting::rune::guards::with_active_world;
use hecs::Entity;
use rune::runtime::VmResult;
use rune::Any;

/// Information about a script attached to an entity
#[derive(Any, Clone)]
#[rune(item = ::script_info)]
pub struct ScriptInfo {
    /// The entity that has the script
    pub entity: u64,
    /// The script source type ("inline" or "file")
    pub source_type: String,
    /// The script source identifier (file path or inline name)
    pub source_path: String,
}

/// Get all entities that have script components.
/// Returns a list of ScriptInfo objects.
#[rune::function]
pub fn get_all_script_entities() -> VmResult<Vec<ScriptInfo>> {
    with_active_world(|world| {
        let mut scripts = Vec::new();
        for (entity, component) in world.query::<&RuneScriptComponent>().iter() {
            let source = component.source();
            let (source_type, source_path) = match source {
                crate::scripting::rune::RuneScriptSource::Inline { name, .. } => {
                    ("inline".to_string(), name.to_string())
                }
                crate::scripting::rune::RuneScriptSource::File { path } => {
                    ("file".to_string(), path.display().to_string())
                }
            };

            scripts.push(ScriptInfo {
                entity: entity.to_bits().get(),
                source_type,
                source_path,
            });
        }
        VmResult::Ok(scripts)
    })
}

/// Get information about a script on a specific entity.
/// Returns None if the entity doesn't have a script.
#[rune::function]
pub fn get_script_info(entity: u64) -> VmResult<Option<ScriptInfo>> {
    with_active_world(|world| {
        let entity_obj = match Entity::from_bits(entity) {
            Some(e) => e,
            None => return VmResult::Ok(None),
        };

        let component = match world.get::<&RuneScriptComponent>(entity_obj) {
            Ok(c) => c,
            Err(_) => return VmResult::Ok(None),
        };

        let source = component.source();
        let (source_type, source_path) = match source {
            crate::scripting::rune::RuneScriptSource::Inline { name, .. } => {
                ("inline".to_string(), name.to_string())
            }
            crate::scripting::rune::RuneScriptSource::File { path } => {
                ("file".to_string(), path.display().to_string())
            }
        };

        VmResult::Ok(Some(ScriptInfo {
            entity: entity_obj.to_bits().get(),
            source_type,
            source_path,
        }))
    })
}

/// Get the entity name if it has one.
#[rune::function]
pub fn get_entity_name(entity: u64) -> VmResult<String> {
    use crate::scene::components::Name;

    with_active_world(|world| {
        let entity_obj = match Entity::from_bits(entity) {
            Some(e) => e,
            None => return VmResult::Ok(format!("Entity {}", entity)),
        };

        let name = match world.get::<&Name>(entity_obj) {
            Ok(n) => n.0.clone(),
            Err(_) => format!("Entity {:?}", entity),
        };

        VmResult::Ok(name)
    })
}
