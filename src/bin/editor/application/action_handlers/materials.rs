use hecs::Entity;
use std::path::PathBuf;
use wgpu_cube::asset::{AssetTypeTag, Handle, MaterialAsset, MaterialKind, ShaderMaterialMetadata};
use wgpu_cube::renderer::Material;
use wgpu_cube::scene::MaterialComponent;

#[cfg(not(target_arch = "wasm32"))]
use std::fs;

use super::{ActionContext, ActionResult};

/// Handle UpdateMaterial action
pub fn handle_update_material(
    ctx: &mut ActionContext,
    entity: Entity,
    handle: Handle<MaterialAsset>,
    material: Material,
) -> ActionResult {
    // Validate that entity's material handle matches
    let can_update = {
        let world = ctx.scene.main_world_mut();
        match world.get::<&MaterialComponent>(entity) {
            Ok(component) => {
                if component.0 == handle {
                    true
                } else {
                    log::warn!(
                        "Inspector material handle mismatch for {:?}: {:?} != {:?}",
                        entity,
                        component.0.index(),
                        handle.index()
                    );
                    false
                }
            }
            Err(err) => {
                log::warn!("Failed to access material for {:?}: {}", entity, err);
                false
            }
        }
    };

    if can_update {
        if let Some(asset) = ctx.scene.assets.material_mut(handle) {
            *asset.material_mut() = material;
            ctx.app.record_scene_change(ctx.scene);
            ActionResult::scene_changed()
        } else {
            log::warn!(
                "Inspector material asset missing for handle {:?}",
                handle.index()
            );
            ActionResult::no_change()
        }
    } else {
        ActionResult::no_change()
    }
}

/// Handle SetMaterialKind action
pub fn handle_set_material_kind(
    ctx: &mut ActionContext,
    entity: Entity,
    handle: Handle<MaterialAsset>,
    kind: MaterialKind,
) -> ActionResult {
    // Validate that entity's material handle matches
    let can_update = {
        let world = ctx.scene.main_world_mut();
        match world.get::<&MaterialComponent>(entity) {
            Ok(component) => {
                if component.0 == handle {
                    true
                } else {
                    log::warn!(
                        "Inspector material handle mismatch for {:?}: {:?} != {:?}",
                        entity,
                        component.0.index(),
                        handle.index()
                    );
                    false
                }
            }
            Err(err) => {
                log::warn!("Failed to access material for {:?}: {}", entity, err);
                false
            }
        }
    };

    if !can_update {
        return ActionResult::no_change();
    }

    if let Some(asset) = ctx.scene.assets.material_mut(handle) {
        asset.set_kind(kind.clone());
        match kind {
            MaterialKind::Shader(_) => {
                asset.set_asset_type(AssetTypeTag::new("shader_material"));
            }
            MaterialKind::Pbr => {
                asset.set_asset_type(AssetTypeTag::default());
            }
        }
        ctx.app.record_scene_change(ctx.scene);
        ActionResult::scene_changed()
    } else {
        log::warn!(
            "Inspector material asset missing for handle {:?}",
            handle.index()
        );
        ActionResult::no_change()
    }
}

/// Handle AssignShaderSource action
pub fn handle_assign_shader_source(
    ctx: &mut ActionContext,
    entity: Entity,
    handle: Handle<MaterialAsset>,
    shader_path: PathBuf,
) -> ActionResult {
    // Validate that entity's material handle matches
    let can_update = {
        let world = ctx.scene.main_world_mut();
        match world.get::<&MaterialComponent>(entity) {
            Ok(component) => {
                if component.0 == handle {
                    true
                } else {
                    log::warn!(
                        "Inspector material handle mismatch for {:?}: {:?} != {:?}",
                        entity,
                        component.0.index(),
                        handle.index()
                    );
                    false
                }
            }
            Err(err) => {
                log::warn!("Failed to access material for {:?}: {}", entity, err);
                false
            }
        }
    };

    if !can_update {
        return ActionResult::no_change();
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(content_root) = ctx.app.project_system().content_root() else {
            log::warn!(
                "Cannot assign shader without an open project for handle {:?}",
                handle.index()
            );
            return ActionResult::no_change();
        };

        let canonical = shader_path.canonicalize().unwrap_or(shader_path.clone());

        if !canonical.starts_with(&content_root) {
            log::warn!(
                "Shader {:?} is outside of the project content directory",
                canonical
            );
            return ActionResult::no_change();
        }

        let source = match fs::read_to_string(&canonical) {
            Ok(contents) => contents,
            Err(err) => {
                log::warn!("Failed to read shader source {:?}: {}", canonical, err);
                return ActionResult::no_change();
            }
        };

        if let Some(asset) = ctx.scene.assets.material_mut(handle) {
            let mut metadata = match asset.kind().clone() {
                MaterialKind::Shader(existing) => existing,
                MaterialKind::Pbr => ShaderMaterialMetadata::default_template(),
            };
            metadata.set_wgsl_source(source);
            metadata.set_source_path(Some(canonical));
            asset.set_kind(MaterialKind::Shader(metadata));
            asset.set_asset_type(AssetTypeTag::new("shader_material"));
            ctx.app.record_scene_change(ctx.scene);
            ActionResult::scene_changed()
        } else {
            log::warn!(
                "Inspector material asset missing for handle {:?}",
                handle.index()
            );
            ActionResult::no_change()
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        let _ = (entity, handle, shader_path);
        log::warn!("Assigning shader files is unavailable on WebAssembly builds.");
        ActionResult::no_change()
    }
}

/// Handle CreateShaderSource action
pub fn handle_create_shader_source(
    ctx: &mut ActionContext,
    entity: Entity,
    handle: Handle<MaterialAsset>,
    suggested_stem: String,
) -> ActionResult {
    // Validate that entity's material handle matches
    let can_update = {
        let world = ctx.scene.main_world_mut();
        match world.get::<&MaterialComponent>(entity) {
            Ok(component) => {
                if component.0 == handle {
                    true
                } else {
                    log::warn!(
                        "Inspector material handle mismatch for {:?}: {:?} != {:?}",
                        entity,
                        component.0.index(),
                        handle.index()
                    );
                    false
                }
            }
            Err(err) => {
                log::warn!("Failed to access material for {:?}: {}", entity, err);
                false
            }
        }
    };

    if !can_update {
        return ActionResult::no_change();
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let Some(content_root) = ctx.app.project_system().content_root() else {
            log::warn!(
                "Cannot create shader without an open project for handle {:?}",
                handle.index()
            );
            return ActionResult::no_change();
        };

        let shader_dir = content_root.join("shaders");
        if let Err(err) = fs::create_dir_all(&shader_dir) {
            log::warn!(
                "Failed to create shader directory {:?}: {}",
                shader_dir,
                err
            );
            return ActionResult::no_change();
        }

        let base_stem = if suggested_stem.trim().is_empty() {
            format!("material_{}", handle.index())
        } else {
            suggested_stem.clone()
        };

        let mut candidate = shader_dir.join(format!("{base_stem}.wgsl"));
        let mut counter = 1usize;
        while candidate.exists() {
            candidate = shader_dir.join(format!("{base_stem}_{counter:02}.wgsl"));
            counter += 1;
        }

        if let Err(err) = fs::write(&candidate, ShaderMaterialMetadata::DEFAULT_TEMPLATE) {
            log::warn!("Failed to write shader template {:?}: {}", candidate, err);
            return ActionResult::no_change();
        }

        let canonical = candidate.canonicalize().unwrap_or(candidate.clone());

        if let Some(asset) = ctx.scene.assets.material_mut(handle) {
            let mut metadata = match asset.kind().clone() {
                MaterialKind::Shader(existing) => existing,
                MaterialKind::Pbr => ShaderMaterialMetadata::default_template(),
            };
            metadata.set_wgsl_source(ShaderMaterialMetadata::DEFAULT_TEMPLATE);
            metadata.set_source_path(Some(canonical));
            asset.set_kind(MaterialKind::Shader(metadata));
            asset.set_asset_type(AssetTypeTag::new("shader_material"));
            ctx.app.record_scene_change(ctx.scene);
            ActionResult::scene_changed()
        } else {
            log::warn!(
                "Inspector material asset missing for handle {:?}",
                handle.index()
            );
            ActionResult::no_change()
        }
    }

    #[cfg(target_arch = "wasm32")]
    {
        let _ = (entity, handle, suggested_stem);
        log::warn!("Creating shader files is unavailable on WebAssembly builds.");
        ActionResult::no_change()
    }
}

/// Handle CreateShaderMaterial action
pub fn handle_create_shader_material(
    ctx: &mut ActionContext,
    entity: Entity,
    source: Handle<MaterialAsset>,
) -> ActionResult {
    let Some(source_asset) = ctx.scene.assets.material(source).cloned() else {
        log::warn!(
            "Inspector shader material source missing for handle {:?}",
            source.index()
        );
        return ActionResult::no_change();
    };

    let mut shader_asset = source_asset;
    shader_asset.set_kind(MaterialKind::Shader(
        ShaderMaterialMetadata::default_template(),
    ));
    shader_asset.set_asset_type(AssetTypeTag::new("shader_material"));
    shader_asset.set_canonical_path(PathBuf::new());

    let new_handle = ctx.scene.assets.insert_material_asset(shader_asset);

    match ctx
        .scene
        .main_world_mut()
        .insert_one(entity, MaterialComponent(new_handle))
    {
        Ok(_) => {
            ctx.app.record_scene_change(ctx.scene);
            ActionResult::scene_changed()
        }
        Err(err) => {
            log::warn!("Failed to assign shader material to {:?}: {}", entity, err);
            ActionResult::no_change()
        }
    }
}
