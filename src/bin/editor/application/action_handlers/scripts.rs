use hecs::Entity;
use std::path::PathBuf;
use wgpu_cube::scripting::{LuaScriptComponent, LuaScriptSource};

use super::{ActionContext, ActionResult};

/// Handle AddScript action
pub fn handle_add_script(ctx: &mut ActionContext, entity: Entity) -> ActionResult {
    let world = ctx.scene.main_world_mut();

    // Check if entity already has a script component
    if world.get::<&LuaScriptComponent>(entity).is_ok() {
        log::warn!("Entity {:?} already has a script component", entity);
        ActionResult::no_change()
    } else {
        // Create a default inline Lua script
        let default_script = LuaScriptComponent::new(LuaScriptSource::inline(
            "new_script",
            "-- New Lua script\n\nfunction on_created(self_entity)\n    -- Initialization code here\nend\n\nfunction update(self_entity, dt)\n    -- Update code here\nend\n",
        ));

        match world.insert(entity, (default_script,)) {
            Ok(_) => {
                log::info!("Added Lua script component to entity {:?}", entity);
                ctx.app.record_scene_change(ctx.scene);
                ActionResult::scene_changed()
            }
            Err(err) => {
                log::warn!("Failed to add script component to {:?}: {}", entity, err);
                ActionResult::no_change()
            }
        }
    }
}

/// Handle ChangeScriptSource action
pub fn handle_change_script_source(
    ctx: &mut ActionContext,
    entity: Entity,
    script_path: PathBuf,
) -> ActionResult {
    let updated = {
        let world = ctx.scene.main_world_mut();

        // Check if entity has a script component
        if let Ok(mut script) = world.get::<&mut LuaScriptComponent>(entity) {
            // Create new script source from file
            let new_source = LuaScriptSource::File {
                path: script_path.clone(),
            };
            *script = LuaScriptComponent::new(new_source);
            log::info!(
                "Changed script source for entity {:?} to {:?}",
                entity,
                script_path
            );
            true
        } else {
            log::warn!("Entity {:?} does not have a script component", entity);
            false
        }
    };

    if updated {
        ctx.app.record_scene_change(ctx.scene);
        // Reload the script runtime to pick up the new script
        ctx.scene.reset_script_runtime();
        ActionResult::scene_changed()
    } else {
        ActionResult::no_change()
    }
}

/// Handle EditScript action
/// Note: Script edits are handled immediately in the UI stage via open_script_editor()
/// This handler is a no-op as the action is processed synchronously
pub fn handle_edit_script(
    _ctx: &mut ActionContext,
    _entity: Entity,
    _component: LuaScriptComponent,
) -> ActionResult {
    // Script edits are handled immediately in the UI stage
    ActionResult::no_change()
}
