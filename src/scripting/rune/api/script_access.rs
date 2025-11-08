/// Script access API for RuneScript
///
/// Provides functions to query script components on entities.
/// This allows scripts to build tools like script editors.
use crate::scripting::rune::component::RuneScriptComponent;
use crate::scripting::rune::guards::with_active_world;
use hecs::Entity;
use rune::runtime::VmResult;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static SCRIPT_CACHE: RefCell<HashMap<usize, (u64, String, String)>> = RefCell::new(HashMap::new());
}

/// Get count of entities that have script components
#[rune::function]
pub fn get_script_count() -> VmResult<i64> {
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

            scripts.push((entity.to_bits().get(), source_type, source_path));
        }

        // Cache the scripts
        SCRIPT_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            cache.clear();
            for (i, script) in scripts.into_iter().enumerate() {
                cache.insert(i, script);
            }
        });

        VmResult::Ok(SCRIPT_CACHE.with(|cache| cache.borrow().len() as i64))
    })
}

/// Get entity ID for script at index
#[rune::function]
pub fn get_script_entity(index: i64) -> VmResult<u64> {
    VmResult::Ok(SCRIPT_CACHE.with(|cache| {
        cache
            .borrow()
            .get(&(index as usize))
            .map(|(entity, _, _)| *entity)
            .unwrap_or(0)
    }))
}

/// Get source type for script at index
#[rune::function]
pub fn get_script_source_type(index: i64) -> VmResult<String> {
    VmResult::Ok(SCRIPT_CACHE.with(|cache| {
        cache
            .borrow()
            .get(&(index as usize))
            .map(|(_, source_type, _)| source_type.clone())
            .unwrap_or_else(|| String::new())
    }))
}

/// Get source path for script at index
#[rune::function]
pub fn get_script_source_path(index: i64) -> VmResult<String> {
    VmResult::Ok(SCRIPT_CACHE.with(|cache| {
        cache
            .borrow()
            .get(&(index as usize))
            .map(|(_, _, source_path)| source_path.clone())
            .unwrap_or_else(|| String::new())
    }))
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
