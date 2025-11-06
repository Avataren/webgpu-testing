use hecs::Entity;
use wgpu_cube::scene::EnvironmentComponent;

use super::{ActionContext, ActionResult};

/// Handle UpdateEnvironment action
pub fn handle_update_environment(
    ctx: &mut ActionContext,
    entity: Entity,
    mut component: EnvironmentComponent,
) -> ActionResult {
    // Get previous environment component to check for HDR path changes
    let previous = ctx
        .scene
        .main_world()
        .get::<&EnvironmentComponent>(entity)
        .ok()
        .map(|existing| EnvironmentComponent::clone(&*existing));

    // Determine if HDR should be auto-enabled based on path change
    let should_enable_hdr = {
        let new_path = component
            .hdr
            .as_ref()
            .and_then(|hdr| hdr.path.as_ref())
            .map(|path| path.as_path());
        let previous_path = previous
            .as_ref()
            .and_then(|prev| prev.hdr.as_ref())
            .and_then(|hdr| hdr.path.as_ref())
            .map(|path| path.as_path());

        match (previous_path, new_path) {
            (Some(prev), Some(new)) => prev != new,
            (None, Some(_)) => true,
            _ => false,
        }
    };

    // Copy environment asset if needed (non-wasm32 only)
    #[cfg(not(target_arch = "wasm32"))]
    {
        if let Err(err) = ctx
            .app
            .copy_environment_asset_if_needed(&mut component, previous.as_ref())
        {
            log::warn!("{err}");
            ctx.app.asset_browser_state_mut().report_error(err);
        }
    }

    // Auto-enable HDR if path was just set
    if let Some(hdr) = component.hdr.as_mut() {
        if should_enable_hdr && hdr.path.is_some() {
            hdr.enabled = true;
        }
    }

    // Update the environment component
    let world = ctx.scene.main_world_mut();
    match world.get::<&mut EnvironmentComponent>(entity) {
        Ok(mut existing) => {
            if *existing != component {
                *existing = component.clone();
                ctx.scene.set_environment(component.to_environment());
                ctx.app.record_scene_change(ctx.scene);
                ActionResult::scene_changed()
            } else {
                ActionResult::no_change()
            }
        }
        Err(err) => {
            log::warn!("Failed to update environment for {:?}: {}", entity, err);
            ActionResult::no_change()
        }
    }
}
