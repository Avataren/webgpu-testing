//! View camera, environment and post-processing.

use glam::{Mat4, Vec3};
use serde_json::{json, Value};

use crate::renderer::postprocess::PostProcessEffects;
use crate::scene::{
    CameraComponent, CameraProjection, EnvironmentComponent, MeshBounds, Name, Parent,
    TransformComponent, WorldTransform,
};

use super::super::json::{
    color_from_json, entity_from_json, entity_id, entity_name, look_rotation, opt_bool, opt_f64,
    patch_serde, round, vec3_from_json, vec3_json,
};
use super::entity::{resolve_existing_path, world_to_local};
use super::{RemoteContext, RemoteResult};

pub(super) fn camera_get(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let scene = &ctx.gpu.scene;
    let camera = scene.camera();
    let forward = (camera.target - camera.eye).normalize_or_zero();
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
        "up": vec3_json(camera.up),
        "forward": vec3_json(forward),
        "projection": projection,
        "active_camera_entity": active.map(|entity| json!({
            "id": entity_id(entity),
            "name": entity_name(scene.main_world(), entity),
        })),
        "note": "In the editor the view camera drives the active camera entity, if there is one.",
    }))
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

pub(super) fn postprocess_json(effects: &PostProcessEffects) -> Value {
    json!({
        "ssao": effects.ssao,
        "bloom": effects.bloom,
        "fxaa": effects.fxaa,
        "ssao_settings": {
            "radius": round(effects.ssao_settings.radius),
            "bias": round(effects.ssao_settings.bias),
            "intensity": round(effects.ssao_settings.intensity),
            "power": round(effects.ssao_settings.power),
        },
        "bloom_settings": {
            "threshold": round(effects.bloom_settings.threshold),
            "knee": round(effects.bloom_settings.knee),
            "scatter": round(effects.bloom_settings.scatter),
        },
    })
}

pub(super) fn postprocess_set(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let mut effects = ctx.gpu.renderer.postprocess_effects();
    if let Some(value) = opt_bool(params, "ssao") {
        effects.ssao = value;
    }
    if let Some(value) = opt_bool(params, "bloom") {
        effects.bloom = value;
    }
    if let Some(value) = opt_bool(params, "fxaa") {
        effects.fxaa = value;
    }
    let float = |section: &str, key: &str| {
        params
            .get(section)
            .and_then(|section| opt_f64(section, key))
            .map(|v| v as f32)
    };
    let ssao = &mut effects.ssao_settings;
    for (key, slot) in [
        ("radius", &mut ssao.radius),
        ("bias", &mut ssao.bias),
        ("intensity", &mut ssao.intensity),
        ("power", &mut ssao.power),
    ] {
        if let Some(value) = float("ssao_settings", key) {
            *slot = value;
        }
    }
    let bloom = &mut effects.bloom_settings;
    for (key, slot) in [
        ("threshold", &mut bloom.threshold),
        ("knee", &mut bloom.knee),
        ("scatter", &mut bloom.scatter),
    ] {
        if let Some(value) = float("bloom_settings", key) {
            *slot = value;
        }
    }

    ctx.gpu.renderer.set_postprocess_effects(effects);
    ctx.hub.set_postprocess_override(effects);
    Ok(postprocess_json(&effects))
}
