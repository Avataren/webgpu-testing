//! View camera, environment and post-processing.

use glam::{Mat4, Vec3};
use serde_json::{json, Value};

use crate::scene::{
    blend_post_process_volumes, volume_influence, Camera, CameraComponent, CameraProjection,
    EnvironmentComponent, MeshBounds, Name, Parent, PostProcessVolume, Transform,
    TransformComponent, Visible, WorldTransform,
};

use super::super::json::{
    color_from_json, entity_from_json, entity_id, entity_name, look_rotation, merge_json, opt_f64,
    opt_str, patch_serde, round, vec3_from_json, vec3_json,
};
use super::entity::{resolve_existing_path, world_to_local};
use super::{RemoteContext, RemoteResult};

pub(super) fn camera_get(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let scene = &ctx.gpu.scene;
    let camera = scene.camera();
    let forward = (camera.target - camera.eye).normalize_or_zero();
    // The stored up is only a hint for the view matrix and can be stale; report the
    // up vector the view actually uses, perpendicular to forward.
    let right = forward.cross(camera.up).try_normalize().unwrap_or(Vec3::X);
    let up = right.cross(forward).normalize_or_zero();
    let active = scene.active_camera_entity();
    let projection = match camera.projection() {
        CameraProjection::Perspective {
            fov_y_radians,
            near,
            far,
        } => json!({
            "kind": "perspective",
            "fov_deg": round(fov_y_radians.to_degrees()),
            "near": round(near),
            "far": round(far),
        }),
        CameraProjection::Orthographic { near, far, .. } => json!({
            "kind": "orthographic",
            "near": round(near),
            "far": round(far),
        }),
    };
    Ok(json!({
        "position": vec3_json(camera.eye),
        "target": vec3_json(camera.target),
        "up": vec3_json(up),
        "forward": vec3_json(forward),
        "projection": projection,
        "active_camera_entity": active.map(|entity| json!({
            "id": entity_id(entity),
            "name": entity_name(scene.main_world(), entity),
        })),
        "note": "In the editor the view camera drives the active camera entity, if there is one.",
    }))
}

/// Apply the `position`/`eye`, `target`, `fov_deg`, `near` and `far` fields of `params`
/// to `camera`. Giving only a target keeps the camera's distance and direction.
pub(crate) fn adjust_camera(camera: &mut Camera, params: &Value) -> Result<(), String> {
    let offset = camera.eye - camera.target;
    if let Some(value) = params.get("target") {
        camera.target = vec3_from_json(value)?;
        if params.get("position").is_none() && params.get("focus").is_none() {
            camera.eye = camera.target + offset;
        }
    }
    if let Some(value) = params.get("position").or_else(|| params.get("eye")) {
        camera.eye = vec3_from_json(value)?;
    }
    if (camera.target - camera.eye).length_squared() < 1e-8 {
        return Err("camera position and target are the same point".into());
    }

    let fov = opt_f64(params, "fov_deg").map(|fov| (fov as f32).clamp(1.0, 170.0).to_radians());
    let near = opt_f64(params, "near").map(|v| v as f32);
    let far = opt_f64(params, "far").map(|v| v as f32);
    if fov.is_some() || near.is_some() || far.is_some() {
        if let CameraProjection::Perspective {
            fov_y_radians,
            near: current_near,
            far: current_far,
        } = camera.projection()
        {
            camera.set_projection(CameraProjection::perspective(
                fov.unwrap_or(fov_y_radians),
                near.unwrap_or(current_near),
                far.unwrap_or(current_far),
            ));
        }
    }
    Ok(())
}

pub(super) fn camera_set(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let scene = &mut *ctx.gpu.scene;
    let mut camera = *scene.camera();
    let offset = camera.eye - camera.target;

    if let Some(value) = params.get("focus") {
        let entity = entity_from_json(scene.main_world(), value)?;
        let world = scene.main_world();
        let transform = world
            .get::<&WorldTransform>(entity)
            .map(|transform| transform.0)
            .map_err(|_| "the focus entity has no transform")?;
        let radius = world
            .get::<&MeshBounds>(entity)
            .map(|bounds| ((bounds.max - bounds.min) * transform.scale).length() * 0.5)
            .unwrap_or(0.5);
        let distance = opt_f64(params, "distance").map_or(radius.max(0.5) * 3.0, |d| d as f32);
        let direction = offset
            .try_normalize()
            .unwrap_or(Vec3::new(0.0, 0.4, 1.0).normalize());
        camera.target = transform.translation;
        camera.eye = camera.target + direction * distance;
    }
    adjust_camera(&mut camera, params)?;

    scene.set_camera(camera);

    // The active camera entity drives the view, so move it too.
    if let Some(entity) = scene.active_camera_entity() {
        let rotation = look_rotation(camera.eye, camera.target).unwrap_or_default();
        let world = scene.main_world_mut();
        let parent_world = world
            .get::<&Parent>(entity)
            .ok()
            .and_then(|parent| world.get::<&WorldTransform>(parent.0).ok().map(|t| t.0));
        let scale = world
            .get::<&TransformComponent>(entity)
            .map(|transform| transform.0.scale)
            .unwrap_or(Vec3::ONE);
        let mut local = world_to_local(
            Mat4::from_rotation_translation(rotation, camera.eye),
            parent_world.as_ref(),
        );
        local.scale = scale;
        let _ = world.insert_one(entity, TransformComponent(local));
        if let Ok(mut component) = world.get::<&mut CameraComponent>(entity) {
            component.projection = camera.projection();
        }
        scene.propagate_transforms();
        ctx.mark_modified();
    }

    camera_get(ctx)
}

pub(super) fn environment_get(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let component = EnvironmentComponent::from_environment(ctx.gpu.scene.environment());
    serde_json::to_value(component).map_err(|err| err.to_string())
}

pub(super) fn environment_set(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    if params.get("color_grading").is_some() {
        return Err(
            "color grading moved to post-process volumes: use postprocess.set with \
                    exposure.compensation and color.saturation / color.contrast"
                .into(),
        );
    }
    let mut patch = params.clone();
    if let Some(color) = params.get("clear_color") {
        patch["clear_color"] = json!(color_from_json(color)?);
    }
    if let Some(path) = params
        .get("hdr")
        .and_then(|hdr| hdr.get("path"))
        .and_then(Value::as_str)
    {
        let resolved = resolve_existing_path(path)?;
        patch["hdr"]["path"] = json!(resolved.display().to_string());
        if params["hdr"].get("enabled").is_none() {
            patch["hdr"]["enabled"] = json!(true);
        }
    }

    let mut current = EnvironmentComponent::from_environment(ctx.gpu.scene.environment());
    if current.hdr.is_none() && patch.get("hdr").is_some_and(Value::is_object) {
        current.hdr = Some(Default::default());
    }
    let updated: EnvironmentComponent = patch_serde(&current, &patch)?;
    // Scenes save their environment through an environment entity, so make sure one
    // exists; set_environment then keeps every environment entity in sync.
    let world = ctx.gpu.scene.main_world_mut();
    let has_entity = world
        .query_mut::<&EnvironmentComponent>()
        .into_iter()
        .next()
        .is_some();
    if !has_entity {
        world.spawn((Name::new("Environment"), updated.clone()));
    }
    ctx.gpu.scene.set_environment(updated.to_environment());
    ctx.mark_modified();
    let mut result = environment_get(ctx)?;
    if !has_entity {
        result["note"] =
            json!("Created an Environment entity so these settings are saved with the scene.");
    }
    Ok(result)
}

/// The post-processing in effect at the view camera, and every volume that feeds it.
pub(super) fn postprocess_get(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let scene = &ctx.gpu.scene;
    let camera = scene.camera().eye;
    let world = scene.main_world();
    let mut volumes = Vec::new();
    let mut query = world.query::<(&PostProcessVolume, Option<&WorldTransform>)>();
    for (entity, (volume, transform)) in query.iter() {
        let matrix = transform.map(|t| t.0.matrix()).unwrap_or(Mat4::IDENTITY);
        volumes.push(json!({
            "id": entity_id(entity),
            "name": entity_name(world, entity),
            "enabled": volume.enabled,
            "shape": volume.shape,
            "priority": volume.priority,
            "weight": round(volume.weight),
            "blend_distance": round(volume.blend_distance),
            "influence_at_camera": round(volume_influence(volume, matrix, camera)),
        }));
    }
    let effective = blend_post_process_volumes(world, camera);
    let mut result = json!({
        "effective": serde_json::to_value(effective.clone().unwrap_or_default())
            .map_err(|err| err.to_string())?,
        "volumes": volumes,
    });
    if let Some(over) = ctx.hub.post_process_override() {
        // Only a visual capture sets one, and only while it runs.
        result["render_override"] = over.describe();
        result["rendered"] =
            serde_json::to_value(over.apply(effective.clone().unwrap_or_default()))
                .map_err(|err| err.to_string())?;
    }
    if effective.is_none() {
        result["note"] = json!(
            "No enabled post_process_volume in the scene, so the defaults apply. \
             postprocess.set creates an infinite volume."
        );
    }
    Ok(result)
}

/// Accept the pre-volume parameter names (`ssao: bool`, `bloom: bool`, `fxaa: bool`,
/// `ssao_settings`, `bloom_settings`) alongside the settings layout.
fn settings_patch(params: &Value) -> Value {
    let mut patch = params.clone();
    let Some(object) = patch.as_object_mut() else {
        return patch;
    };
    for key in ["entity", "volume"] {
        object.remove(key);
    }
    for effect in ["ssao", "bloom"] {
        if let Some(enabled) = object.get(effect).and_then(Value::as_bool) {
            object.insert(effect.into(), json!({ "enabled": enabled }));
        }
        if let Some(legacy) = object.remove(&format!("{effect}_settings")) {
            let target = object.entry(effect).or_insert_with(|| json!({}));
            merge_json(target, &legacy);
        }
    }
    if let Some(fxaa) = object.remove("fxaa").and_then(|v| v.as_bool()) {
        object.insert(
            "anti_aliasing".into(),
            json!(if fxaa { "fxaa" } else { "none" }),
        );
    }
    patch
}

/// Setting names in `patch` that `current` has no field for, as dotted paths. Settings
/// deserialize leniently, so without this a misspelt name would be dropped silently.
fn unknown_setting_paths(current: &Value, patch: &Value, prefix: &str, out: &mut Vec<String>) {
    let (Value::Object(current), Value::Object(patch)) = (current, patch) else {
        return;
    };
    for (key, value) in patch {
        let path = if prefix.is_empty() {
            key.clone()
        } else {
            format!("{prefix}.{key}")
        };
        match current.get(key) {
            Some(existing) => unknown_setting_paths(existing, value, &path, out),
            None => out.push(path),
        }
    }
}

/// The settings patch in `params` (see [`patch_post_process`]), with the pre-volume
/// parameter names translated. Rejects setting names that do not exist and values of the
/// wrong type, so scripts and tools hear about a typo instead of nothing happening.
pub(crate) fn post_process_patch(params: &Value) -> Result<Value, String> {
    let patch = settings_patch(params);
    let defaults = serde_json::to_value(PostProcessVolume::default().settings)
        .map_err(|err| err.to_string())?;
    let mut unknown = Vec::new();
    unknown_setting_paths(&defaults, &patch, "", &mut unknown);
    if !unknown.is_empty() {
        return Err(format!(
            "unknown post-process setting(s): {}; use {}",
            unknown.join(", "),
            defaults
                .as_object()
                .map(|groups| groups.keys().cloned().collect::<Vec<_>>().join(", "))
                .unwrap_or_default()
        ));
    }
    // A wrong type (a string for a number) is only found by reading the values.
    patch_serde(&PostProcessVolume::default().settings, &patch)?;
    Ok(patch)
}

/// Patch a post-process volume: `target`, else the lowest-priority infinite volume, which
/// is created when the scene has none. `params` holds settings groups (`{"bloom":
/// {"intensity": 2}}`) and may carry a `volume` object with the volume's own fields
/// (`priority`, `weight`, `shape`, ...). Returns the edited volume and whether it was
/// created; a created volume has no world transform until the caller propagates them.
pub(crate) fn patch_post_process(
    world: &mut hecs::World,
    target: Option<hecs::Entity>,
    params: &Value,
) -> Result<(hecs::Entity, bool), String> {
    let target = match target {
        Some(entity) => {
            if world.get::<&PostProcessVolume>(entity).is_err() {
                return Err(format!(
                    "entity {} has no post_process_volume",
                    entity_id(entity)
                ));
            }
            Some(entity)
        }
        None => world
            .query_mut::<&PostProcessVolume>()
            .into_iter()
            .filter(|(_, volume)| volume.shape.is_infinite())
            .min_by_key(|(_, volume)| volume.priority)
            .map(|(entity, _)| entity),
    };

    let patch = post_process_patch(params)?;

    let created = target.is_none();
    let entity = target.unwrap_or_else(|| {
        world.spawn((
            Name::new("Post Process Volume"),
            TransformComponent(Transform::IDENTITY),
            Visible(true),
            PostProcessVolume::default(),
        ))
    });

    let mut volume = world
        .get::<&PostProcessVolume>(entity)
        .map(|volume| (*volume).clone())
        .map_err(|err| err.to_string())?;
    volume.settings = patch_serde(&volume.settings, &patch)?;
    volume.tick_overrides_for_patch(&patch);
    if let Some(volume_patch) = params.get("volume") {
        let settings = volume.settings.clone();
        volume = patch_serde(&volume, volume_patch)?;
        volume.settings = settings;
    }
    world
        .insert_one(entity, volume)
        .map_err(|err| err.to_string())?;
    Ok((entity, created))
}

/// Change post-processing on a volume: the given `entity`, else the lowest-priority
/// infinite volume, which is created when the scene has none.
pub(super) fn postprocess_set(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let world = ctx.gpu.scene.main_world_mut();
    let target = params
        .get("entity")
        .map(|value| entity_from_json(world, value))
        .transpose()?;
    let (entity, created) = patch_post_process(world, target, params)?;
    ctx.gpu.scene.propagate_transforms();
    ctx.mark_modified();

    let mut result = postprocess_get(ctx)?;
    result["edited_volume"] = json!({
        "id": entity_id(entity),
        "name": entity_name(ctx.gpu.scene.main_world(), entity),
    });
    if created {
        result["note"] =
            json!("Created an infinite Post Process Volume entity for these settings.");
    }
    Ok(result)
}

/// Re-read textures (and scripts) from disk: everything, or what lives under `path`.
pub(super) fn asset_reload(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let filter = opt_str(params, "path")
        .map(resolve_existing_path)
        .transpose()?;
    let scene = &mut ctx.gpu.scene;
    let results = scene
        .assets
        .reload_textures(&mut *ctx.gpu.renderer, filter.as_deref());
    let mut reloaded = Vec::new();
    let mut failed = serde_json::Map::new();
    for (path, outcome) in results {
        match outcome {
            Ok(()) => reloaded.push(json!(path.display().to_string())),
            Err(err) => {
                failed.insert(path.display().to_string(), json!(err));
            }
        }
    }
    if !reloaded.is_empty() {
        ctx.gpu
            .renderer
            .update_texture_bind_group(&ctx.gpu.scene.assets);
    }
    let scripts = match &filter {
        Some(path) => ctx.gpu.scene.reload_script_path(path),
        None => 0,
    };
    if reloaded.is_empty() && failed.is_empty() && scripts == 0 {
        return Err(match &filter {
            Some(path) => format!("no loaded texture or script uses {}", path.display()),
            None => "no textures were loaded from files".into(),
        });
    }
    Ok(json!({
        "textures_reloaded": reloaded,
        "textures_failed": failed,
        "scripts_reloaded": scripts,
    }))
}
