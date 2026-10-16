//! Entity inspection and editing.

use std::path::{Path, PathBuf};

use glam::{Mat4, Quat, Vec3};
use hecs::{Entity, World};
use serde_json::{json, Value};

use crate::asset::{MaterialAsset, MaterialKind, MaterialTextureReference, MaterialTextureSlot};
use crate::procgen::GeneratedMesh;
use crate::renderer::material::MaterialFlags;
use crate::renderer::primitives::PrimitiveMeshDescriptor;
use crate::renderer::{Material, Renderer};
use crate::scene::components::{Billboard, BillboardOrientation, DepthState};
use crate::scene::skeletal::{Animator, SkinnedMesh};
use crate::scene::{
    CameraComponent, CameraProjection, CanCastShadow, Children, DirectionalLight,
    EnvironmentComponent, GltfSource, LightProbeVolume, MaterialComponent, MeshBounds,
    MeshComponent, Name, OrbitAnimation, Parent, ParticleBehaviorConfig, ParticleBehaviorPreset,
    ParticleEmitterComponent, ParticleSystemComponent, PointLight, PostProcessVolume,
    PrimitiveMeshComponent, RotateAnimation, Scene, SceneImportDevice, SpotLight, Transform,
    TransformComponent, Visible, WorldTransform,
};
use crate::scripting::lua::ScriptMode;
use crate::scripting::{LuaScriptComponent, LuaScriptSource};

use super::super::json::{
    apply_transform_params, color_from_json, color_json, entity_from_json, entity_id, entity_name,
    opt_bool, opt_f64, opt_str, opt_u64, patch_serde, req_entity, round, transform_to_json,
    vec3_from_json, vec3_json,
};
use super::info::entity_summary;
use super::{RemoteContext, RemoteResult};

/// Component names accepted by filters, `entity.remove_component` and reported by
/// listings.
pub(super) const COMPONENT_NAMES: &[&str] = &[
    "transform",
    "mesh",
    "material",
    "visible",
    "point_light",
    "directional_light",
    "spot_light",
    "cast_shadow",
    "camera",
    "script",
    "particle_system",
    "particle_emitter",
    "environment",
    "post_process_volume",
    "light_probe_volume",
    "rigid_body",
    "collider",
    "character_controller",
    "spring_camera",
    "billboard",
    "depth_state",
    "rotate_animation",
    "orbit_animation",
    "gltf",
    "skinned_mesh",
    "animator",
];

pub(super) fn has_component(world: &World, entity: Entity, name: &str) -> bool {
    fn has<T: hecs::Component>(world: &World, entity: Entity) -> bool {
        world.satisfies::<&T>(entity).unwrap_or(false)
    }
    match name {
        "transform" => has::<TransformComponent>(world, entity),
        "mesh" => has::<MeshComponent>(world, entity),
        "material" => has::<MaterialComponent>(world, entity),
        "visible" => has::<Visible>(world, entity),
        "point_light" => has::<PointLight>(world, entity),
        "directional_light" => has::<DirectionalLight>(world, entity),
        "spot_light" => has::<SpotLight>(world, entity),
        "cast_shadow" => has::<CanCastShadow>(world, entity),
        "camera" => has::<CameraComponent>(world, entity),
        "script" => has::<LuaScriptComponent>(world, entity),
        "particle_system" => has::<ParticleSystemComponent>(world, entity),
        "particle_emitter" => has::<ParticleEmitterComponent>(world, entity),
        "environment" => has::<EnvironmentComponent>(world, entity),
        "post_process_volume" => has::<PostProcessVolume>(world, entity),
        "light_probe_volume" => has::<LightProbeVolume>(world, entity),
        "rigid_body" | "collider" | "character_controller" | "spring_camera" => {
            crate::physics::edit::has_component(world, entity, name)
        }
        "billboard" => has::<Billboard>(world, entity),
        "depth_state" => has::<DepthState>(world, entity),
        "rotate_animation" => has::<RotateAnimation>(world, entity),
        "orbit_animation" => has::<OrbitAnimation>(world, entity),
        "gltf" => has::<GltfSource>(world, entity),
        "skinned_mesh" => has::<SkinnedMesh>(world, entity),
        "animator" => has::<Animator>(world, entity),
        _ => false,
    }
}

pub(super) fn component_names(world: &World, entity: Entity) -> Vec<&'static str> {
    COMPONENT_NAMES
        .iter()
        .copied()
        .filter(|name| has_component(world, entity, name))
        .collect()
}

fn unit_to_u8(value: f64) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

const TEXTURE_SLOTS: [(&str, MaterialTextureSlot, MaterialFlags, bool); 5] = [
    (
        "base_color_texture",
        MaterialTextureSlot::BaseColor,
        MaterialFlags::USE_BASE_COLOR_TEXTURE,
        true,
    ),
    (
        "metallic_roughness_texture",
        MaterialTextureSlot::MetallicRoughness,
        MaterialFlags::USE_METALLIC_ROUGHNESS_TEXTURE,
        false,
    ),
    (
        "normal_texture",
        MaterialTextureSlot::Normal,
        MaterialFlags::USE_NORMAL_TEXTURE,
        false,
    ),
    (
        "emissive_texture",
        MaterialTextureSlot::Emissive,
        MaterialFlags::USE_EMISSIVE_TEXTURE,
        true,
    ),
    (
        "occlusion_texture",
        MaterialTextureSlot::Occlusion,
        MaterialFlags::USE_OCCLUSION_TEXTURE,
        false,
    ),
];

pub(super) fn material_json(asset: &MaterialAsset) -> Value {
    let material = asset.material();
    let flags = material.flags;
    let mut textures = serde_json::Map::new();
    for (key, slot, flag, _) in TEXTURE_SLOTS {
        if flags.contains(flag) {
            let path = asset
                .texture_references()
                .find(|(reference_slot, _)| *reference_slot == slot)
                .and_then(|(_, reference)| reference.canonical_path())
                .map(|path| path.display().to_string());
            textures.insert(
                key.to_owned(),
                json!(path.unwrap_or_else(|| "(embedded)".into())),
            );
        }
    }
    let kind = match asset.kind() {
        MaterialKind::Pbr => "pbr",
        MaterialKind::Shader(_) => "shader",
    };
    json!({
        "kind": kind,
        "color": color_json(material.color_f32()),
        "metallic": round(material.metallic_f32()),
        "roughness": round(material.roughness_f32()),
        "emissive": round(material.emissive_f32()),
        "transmission": round(material.transmission_f32()),
        "ior": round(material.ior_f32()),
        "dispersion": round(material.dispersion_f32()),
        "unlit": flags.contains(MaterialFlags::UNLIT),
        "transparent": flags.contains(MaterialFlags::ALPHA_BLEND),
        "alpha_cutoff": round(material.alpha_cutoff_f32()),
        "double_sided": flags.contains(MaterialFlags::DOUBLE_SIDED),
        "nearest_filtering": flags.contains(MaterialFlags::USE_NEAREST_FILTERING),
        "billboarded": flags.contains(MaterialFlags::BILLBOARDED),
        "textures": textures,
    })
}

fn script_mode_name(mode: ScriptMode) -> &'static str {
    match mode {
        ScriptMode::RuntimeOnly => "play (no annotation)",
        ScriptMode::EditorOnly => "editor (@editor)",
        ScriptMode::Both => "both (@tool)",
    }
}

fn primitive_name(descriptor: PrimitiveMeshDescriptor) -> &'static str {
    match descriptor {
        PrimitiveMeshDescriptor::Cube => "cube",
        PrimitiveMeshDescriptor::Sphere => "sphere",
        PrimitiveMeshDescriptor::Plane => "plane",
        PrimitiveMeshDescriptor::Cylinder => "cylinder",
        PrimitiveMeshDescriptor::Cone => "cone",
        PrimitiveMeshDescriptor::Torus => "torus",
    }
}

fn primitive_from_name(name: &str) -> Option<PrimitiveMeshDescriptor> {
    Some(match name {
        "cube" | "box" => PrimitiveMeshDescriptor::Cube,
        "sphere" => PrimitiveMeshDescriptor::Sphere,
        "plane" | "quad" => PrimitiveMeshDescriptor::Plane,
        "cylinder" => PrimitiveMeshDescriptor::Cylinder,
        "cone" => PrimitiveMeshDescriptor::Cone,
        "torus" => PrimitiveMeshDescriptor::Torus,
        _ => return None,
    })
}

fn particle_preset_from_name(name: &str) -> Result<ParticleBehaviorPreset, String> {
    Ok(match name {
        "physics" => ParticleBehaviorPreset::Physics,
        "starfield" => ParticleBehaviorPreset::Starfield,
        "boids" => ParticleBehaviorPreset::Boids,
        "optimized_boids" => ParticleBehaviorPreset::OptimizedBoids,
        other => {
            return Err(format!(
            "unknown particle preset {other:?}; use physics, starfield, boids or optimized_boids"
        ))
        }
    })
}

fn point_light_json(light: &PointLight) -> Value {
    json!({ "color": vec3_json(light.color), "intensity": round(light.intensity), "range": round(light.range) })
}

fn directional_light_json(light: &DirectionalLight) -> Value {
    json!({ "color": vec3_json(light.color), "intensity": round(light.intensity), "shadow_size": round(light.shadow_size) })
}

fn spot_light_json(light: &SpotLight) -> Value {
    json!({
        "color": vec3_json(light.color),
        "intensity": round(light.intensity),
        "range": round(light.range),
        "inner_angle_deg": round(light.inner_angle.to_degrees()),
        "outer_angle_deg": round(light.outer_angle.to_degrees()),
    })
}

fn camera_json(camera: &CameraComponent) -> Value {
    match camera.projection {
        CameraProjection::Perspective {
            fov_y_radians,
            near,
            far,
        } => json!({
            "projection": "perspective",
            "fov_deg": round(fov_y_radians.to_degrees()),
            "near": round(near),
            "far": round(far),
        }),
        CameraProjection::Orthographic {
            left,
            right,
            bottom,
            top,
            near,
            far,
        } => json!({
            "projection": "orthographic",
            "left": round(left), "right": round(right),
            "bottom": round(bottom), "top": round(top),
            "near": round(near), "far": round(far),
        }),
    }
}

pub(super) fn get(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let entity = req_entity(ctx.gpu.scene.main_world(), params)?;
    // Binding clips may load their files, which needs the scene mutably.
    let animator = super::animation::animator_json(&mut ctx.gpu.scene, entity);
    let scene: &Scene = &ctx.gpu.scene;
    let world = scene.main_world();

    let mut out = entity_summary(world, entity);
    out["children"] = json!(world
        .get::<&Children>(entity)
        .map(|children| children
            .0
            .iter()
            .copied()
            .map(entity_id)
            .collect::<Vec<_>>())
        .unwrap_or_default());

    if let Ok(transform) = world.get::<&TransformComponent>(entity) {
        out["transform"] = transform_to_json(&transform.0);
    }
    if let Ok(transform) = world.get::<&WorldTransform>(entity) {
        out["world_transform"] = transform_to_json(&transform.0);
    }
    if let Ok(visible) = world.get::<&Visible>(entity) {
        out["visible"] = json!(visible.0);
    }
    if let Ok(mesh) = world.get::<&MeshComponent>(entity) {
        let primitive = world
            .get::<&PrimitiveMeshComponent>(entity)
            .ok()
            .map(|primitive| primitive_name(primitive.descriptor));
        let bounds = world
            .get::<&MeshBounds>(entity)
            .ok()
            .map(|bounds| json!({ "min": vec3_json(bounds.min), "max": vec3_json(bounds.max) }));
        out["mesh"] =
            json!({ "handle": mesh.0.index(), "primitive": primitive, "local_bounds": bounds });
    }
    if let Ok(material) = world.get::<&MaterialComponent>(entity) {
        let handle = material.0;
        if let Some(asset) = scene.assets.materials.get(handle) {
            let mut value = material_json(asset);
            let shared_by = world
                .query::<&MaterialComponent>()
                .iter()
                .filter(|(_, other)| other.0 == handle)
                .count();
            value["handle"] = json!(handle.index());
            value["shared_by"] = json!(shared_by);
            out["material"] = value;
        }
    }
    if let Ok(light) = world.get::<&PointLight>(entity) {
        out["point_light"] = point_light_json(&light);
    }
    if let Ok(light) = world.get::<&DirectionalLight>(entity) {
        out["directional_light"] = directional_light_json(&light);
    }
    if let Ok(light) = world.get::<&SpotLight>(entity) {
        out["spot_light"] = spot_light_json(&light);
    }
    if let Ok(shadow) = world.get::<&CanCastShadow>(entity) {
        out["cast_shadow"] = json!(shadow.0);
    }
    if let Ok(camera) = world.get::<&CameraComponent>(entity) {
        let mut value = camera_json(&camera);
        value["active"] = json!(scene.active_camera_entity() == Some(entity));
        out["camera"] = value;
    }
    if let Ok(script) = world.get::<&LuaScriptComponent>(entity) {
        let mut value = match script.source() {
            LuaScriptSource::File { path } => json!({
                "kind": "file",
                "path": path.display().to_string(),
                "exists": path.exists(),
            }),
            LuaScriptSource::Inline { name, source } => json!({
                "kind": "inline",
                "name": name,
                "source": source,
            }),
        };
        value["mode"] = json!(script_mode_name(script.script_mode()));
        value["last_error"] = json!(scene.lua_scripting().last_error(entity));
        out["script"] = value;
    }
    if let Ok(system) = world.get::<&ParticleSystemComponent>(entity) {
        out["particle_system"] = serde_json::to_value(&*system).unwrap_or(Value::Null);
    }
    if let Ok(emitter) = world.get::<&ParticleEmitterComponent>(entity) {
        out["particle_emitter"] = serde_json::to_value(&*emitter).unwrap_or(Value::Null);
    }
    if let Ok(environment) = world.get::<&EnvironmentComponent>(entity) {
        out["environment"] = serde_json::to_value(&*environment).unwrap_or(Value::Null);
    }
    if let Ok(volume) = world.get::<&PostProcessVolume>(entity) {
        out["post_process_volume"] = serde_json::to_value(&*volume).unwrap_or(Value::Null);
    }
    if let Ok(volume) = world.get::<&LightProbeVolume>(entity) {
        out["light_probe_volume"] = serde_json::to_value(&*volume).unwrap_or(Value::Null);
    }
    for name in crate::physics::edit::COMPONENT_NAMES {
        if let Some(value) = crate::physics::edit::component_json(world, entity, name) {
            out[name] = value;
        }
    }
    if let Some(status) = scene.physics().character_status(entity) {
        out["character_state"] = json!({
            "grounded": status.grounded,
            "velocity": status.velocity.to_array(),
            "air_time": status.air_time,
            "sliding": status.sliding,
            "running": status.running,
        });
    }
    if let Ok(billboard) = world.get::<&Billboard>(entity) {
        let orientation = match billboard.orientation {
            BillboardOrientation::FaceCamera => "face_camera",
            BillboardOrientation::FaceCameraYAxis => "face_camera_y_axis",
        };
        out["billboard"] = json!({ "orientation": orientation, "lit": billboard.lit });
    }
    if let Ok(depth) = world.get::<&DepthState>(entity) {
        out["depth_state"] =
            json!({ "depth_test": depth.depth_test, "depth_write": depth.depth_write });
    }
    if let Ok(animation) = world.get::<&RotateAnimation>(entity) {
        out["rotate_animation"] =
            json!({ "axis": vec3_json(animation.axis), "speed": round(animation.speed) });
    }
    if let Ok(animation) = world.get::<&OrbitAnimation>(entity) {
        out["orbit_animation"] = json!({
            "center": vec3_json(animation.center),
            "radius": round(animation.radius),
            "speed": round(animation.speed),
            "offset": round(animation.offset),
        });
    }
    if let Ok(source) = world.get::<&GltfSource>(entity) {
        out["gltf"] = json!({ "source": source.0.display().to_string() });
    }
    if let Some(skin) = super::animation::skinned_mesh_json(scene, entity) {
        out["skinned_mesh"] = skin;
    }
    if let Some(animator) = animator {
        out["animator"] = animator;
    }
    Ok(out)
}

/// Default transform for a new entity of `kind`, matching the editor's create menu.
fn default_transform(kind: &str) -> Transform {
    match kind {
        "point_light" => Transform::from_translation(Vec3::new(3.0, 3.0, 2.0)),
        "directional_light" => {
            let direction = Vec3::new(0.35, -1.0, -0.85).normalize();
            Transform::from_trs(
                Vec3::new(0.0, 6.0, 0.0),
                Quat::from_rotation_arc(Vec3::NEG_Z, direction),
                Vec3::ONE,
            )
        }
        "spot_light" => {
            let position = Vec3::new(-4.0, 5.0, -3.0);
            let direction = (Vec3::new(0.0, 1.0, 0.0) - position).normalize();
            Transform::from_trs(
                position,
                Quat::from_rotation_arc(Vec3::NEG_Z, direction),
                Vec3::ONE,
            )
        }
        "camera" => {
            let position = Vec3::new(0.0, 3.0, 6.0);
            Transform::from_trs(
                position,
                Quat::from_rotation_arc(Vec3::NEG_Z, (-position).normalize()),
                Vec3::ONE,
            )
        }
        _ => Transform::default(),
    }
}

fn default_name(kind: &str) -> String {
    match kind {
        "empty" => "Empty".into(),
        "point_light" => "Point Light".into(),
        "directional_light" => "Directional Light".into(),
        "spot_light" => "Spot Light".into(),
        "camera" => "Camera".into(),
        "particles" => "Particle System".into(),
        "environment" => "Environment".into(),
        "post_process_volume" => "Post Process Volume".into(),
        "light_probe_volume" => "Light Probe Volume".into(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_else(|| "Entity".into())
        }
    }
}

pub(super) fn spawn(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let kind = opt_str(params, "kind")
        .or_else(|| opt_str(params, "primitive"))
        .unwrap_or("empty")
        .to_ascii_lowercase();
    let primitive = primitive_from_name(&kind);
    let known = matches!(
        kind.as_str(),
        "empty"
            | "point_light"
            | "directional_light"
            | "spot_light"
            | "camera"
            | "particles"
            | "environment"
            | "post_process_volume"
            | "light_probe_volume"
    );
    if primitive.is_none() && !known {
        return Err(format!(
            "unknown kind {kind:?}; use empty, cube, sphere, plane, cylinder, cone, torus, \
             point_light, directional_light, spot_light, camera, particles, environment, \
             post_process_volume or light_probe_volume"
        ));
    }

    let name = opt_str(params, "name")
        .map(str::to_owned)
        .unwrap_or_else(|| default_name(&kind));
    let mut transform = default_transform(&kind);
    apply_transform_params(&mut transform, params)?;

    let mut builder = hecs::EntityBuilder::new();
    builder
        .add(Name::new(name))
        .add(TransformComponent(transform))
        .add(Visible(true));

    let scene = &mut *ctx.gpu.scene;
    if let Some(descriptor) = primitive {
        add_primitive_mesh(scene, &mut *ctx.gpu.renderer, &mut builder, descriptor);
    }
    match kind.as_str() {
        "point_light" => {
            builder.add(PointLight::default()).add(CanCastShadow(true));
        }
        "directional_light" => {
            builder
                .add(DirectionalLight::default())
                .add(CanCastShadow(true));
        }
        "spot_light" => {
            builder.add(SpotLight::default()).add(CanCastShadow(true));
        }
        "camera" => {
            builder.add(CameraComponent::from(*scene.camera()));
        }
        "environment" => {
            builder.add(EnvironmentComponent::from_environment(scene.environment()));
        }
        "light_probe_volume" => {
            builder.add(LightProbeVolume::default());
        }
        "post_process_volume" => {
            // Start from what is in effect, so the new volume changes nothing until edited.
            let settings =
                crate::scene::blend_post_process_volumes(scene.main_world(), scene.camera().eye);
            builder.add(match settings {
                None => PostProcessVolume::default(),
                // Other volumes exist: start from what they give and override only what
                // this call sets (apply_patch ticks those).
                Some(settings) => PostProcessVolume::additional(settings),
            });
        }
        "particles" => {
            let preset = particle_preset_from_name(
                params
                    .get("particle_system")
                    .and_then(|system| opt_str(system, "preset"))
                    .or_else(|| opt_str(params, "preset"))
                    .unwrap_or("physics"),
            )?;
            add_particle_components(scene, &mut *ctx.gpu.renderer, &mut builder, preset);
        }
        _ => {}
    }

    let entity = scene.main_world_mut().spawn(builder.build());
    let make_active = kind == "camera" && opt_bool(params, "active_camera").unwrap_or(true);

    // Everything else (material, lights, script, parent, ...) uses the update path.
    let mut patch = params.clone();
    if let Some(object) = patch.as_object_mut() {
        for key in [
            "kind",
            "primitive",
            "name",
            "translation",
            "position",
            "rotation",
            "rotation_quat",
            "scale",
            "look_at",
            "preset",
            "active_camera",
            "quiet",
        ] {
            object.remove(key);
        }
        if kind == "particles" {
            if let Some(system) = object
                .get_mut("particle_system")
                .and_then(Value::as_object_mut)
            {
                system.remove("preset");
            }
        }
    }
    if let Err(err) = apply_patch(ctx, entity, &patch) {
        let _ = ctx.gpu.scene.main_world_mut().despawn(entity);
        return Err(err);
    }
    if make_active {
        ctx.gpu.scene.set_active_camera_entity(Some(entity));
    }

    ctx.gpu.scene.propagate_transforms();
    ctx.mark_modified();
    if kind == "environment" {
        sync_environment_from_entity(ctx, entity);
    }
    if opt_bool(params, "quiet").unwrap_or(false) {
        return Ok(brief(ctx.gpu.scene.main_world(), entity));
    }
    Ok(entity_summary(ctx.gpu.scene.main_world(), entity))
}

/// `{id, name}` of an entity: the result of calls made with `quiet: true`.
fn brief(world: &World, entity: Entity) -> Value {
    json!({ "id": entity_id(entity), "name": entity_name(world, entity) })
}

/// The shared mesh for a primitive and its bounds, created on first use.
fn primitive_mesh(
    scene: &mut Scene,
    renderer: &mut dyn SceneImportDevice,
    descriptor: PrimitiveMeshDescriptor,
) -> (MeshComponent, Option<MeshBounds>) {
    let mesh = scene.assets.ensure_primitive_mesh(renderer, descriptor);
    let bounds = scene
        .assets
        .meshes
        .get(mesh)
        .and_then(|mesh| MeshBounds::from_vertices(&mesh.data().vertices));
    (MeshComponent(mesh), bounds)
}

fn new_pbr_material(scene: &mut Scene) -> MaterialComponent {
    MaterialComponent(scene.assets.materials.insert(MaterialAsset::from_material(
        Material::pbr(),
        PathBuf::new(),
    )))
}

fn add_primitive_mesh(
    scene: &mut Scene,
    renderer: &mut dyn SceneImportDevice,
    builder: &mut hecs::EntityBuilder,
    descriptor: PrimitiveMeshDescriptor,
) {
    let (mesh, bounds) = primitive_mesh(scene, renderer, descriptor);
    builder
        .add(mesh)
        .add(new_pbr_material(scene))
        .add(PrimitiveMeshComponent { descriptor });
    if let Some(bounds) = bounds {
        builder.add(bounds);
    }
}

fn add_particle_components(
    scene: &mut Scene,
    renderer: &mut dyn SceneImportDevice,
    builder: &mut hecs::EntityBuilder,
    preset: ParticleBehaviorPreset,
) {
    let mesh = scene
        .assets
        .ensure_primitive_mesh(renderer, PrimitiveMeshDescriptor::Plane);
    let bounds = scene
        .assets
        .meshes
        .get(mesh)
        .and_then(|mesh| MeshBounds::from_vertices(&mesh.data().vertices));
    let material = match preset {
        ParticleBehaviorPreset::Starfield => Material::new([255, 255, 255, 255])
            .with_unlit()
            .with_billboarding(),
        _ => Material::pbr(),
    };
    let material = scene
        .assets
        .materials
        .insert(MaterialAsset::from_material(material, PathBuf::new()));
    let spawn_rate = 120.0;
    let emitter = crate::gpu_particles::ParticleEmitter::new(Vec3::ZERO, spawn_rate);
    builder
        .add(MeshComponent(mesh))
        .add(MaterialComponent(material))
        .add(ParticleSystemComponent::new(spawn_rate, preset))
        .add(ParticleEmitterComponent::from(&emitter));
    if let Some(bounds) = bounds {
        builder.add(bounds);
    }
    if preset == ParticleBehaviorPreset::Starfield {
        builder
            .add(Billboard::new(BillboardOrientation::FaceCamera))
            .add(DepthState::new(true, false));
    }
}

pub(super) fn update(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let quiet = opt_bool(params, "quiet").unwrap_or(false);
    let mut patch = params.clone();
    if let Some(object) = patch.as_object_mut() {
        for key in ["entity", "entities", "name_contains", "quiet"] {
            object.remove(key);
        }
    }
    let nothing_to_change = || {
        "nothing to change; pass fields like name, translation, rotation, scale, parent, \
         visible, material, point_light, script, ..."
            .to_string()
    };

    let Some(targets) = bulk_targets(ctx.gpu.scene.main_world(), params)? else {
        let entity = req_entity(ctx.gpu.scene.main_world(), params)?;
        let changed = apply_patch(ctx, entity, &patch)?;
        if changed.is_empty() {
            return Err(nothing_to_change());
        }
        ctx.gpu.scene.propagate_transforms();
        ctx.mark_modified();
        let mut summary = if quiet {
            brief(ctx.gpu.scene.main_world(), entity)
        } else {
            entity_summary(ctx.gpu.scene.main_world(), entity)
        };
        summary["changed"] = json!(changed);
        return Ok(summary);
    };

    if patch.get("name").is_some() && targets.len() > 1 {
        return Err("name cannot be given to several entities; names must be unique".into());
    }
    let mut changed = Vec::new();
    for (index, &entity) in targets.iter().enumerate() {
        match apply_patch(ctx, entity, &patch) {
            Ok(fields) => changed = fields,
            Err(err) => {
                if index > 0 {
                    ctx.gpu.scene.propagate_transforms();
                    ctx.mark_modified();
                }
                return Err(format!(
                    "{err} (at entity {}; the {index} entities before it were changed)",
                    entity_id(entity)
                ));
            }
        }
        if changed.is_empty() {
            return Err(nothing_to_change());
        }
    }
    ctx.gpu.scene.propagate_transforms();
    ctx.mark_modified();
    Ok(json!({
        "updated": targets.len(),
        "changed": changed,
        "ids": targets.into_iter().map(entity_id).collect::<Vec<_>>(),
    }))
}

/// The entities a bulk update applies to: an `entities` list of ids/names, or every
/// entity whose name contains `name_contains` (case-insensitive). `None` when the call
/// names a single `entity` instead.
fn bulk_targets(world: &World, params: &Value) -> Result<Option<Vec<Entity>>, String> {
    if let Some(list) = params.get("entities") {
        let list = list
            .as_array()
            .ok_or("entities must be an array of ids or names")?;
        let entities = list
            .iter()
            .map(|value| entity_from_json(world, value))
            .collect::<Result<Vec<_>, _>>()?;
        if entities.is_empty() {
            return Err("entities is empty".into());
        }
        return Ok(Some(entities));
    }
    let Some(filter) = opt_str(params, "name_contains") else {
        return Ok(None);
    };
    let filter = filter.to_lowercase();
    let mut entities: Vec<Entity> = world
        .query::<&Name>()
        .iter()
        .filter(|(_, name)| name.0.to_lowercase().contains(&filter))
        .map(|(entity, _)| entity)
        .collect();
    if entities.is_empty() {
        return Err(format!("no entity name contains {filter:?}"));
    }
    entities.sort_by_key(|entity| entity.id());
    Ok(Some(entities))
}

/// Apply every recognised field of `patch` to `entity`. Returns the names of the
/// fields that were applied; unknown fields are an error so typos are not silent.
pub(super) fn apply_patch(
    ctx: &mut RemoteContext<'_, '_>,
    entity: Entity,
    patch: &Value,
) -> Result<Vec<String>, String> {
    let Some(fields) = patch.as_object() else {
        return Err("expected an object of fields".into());
    };
    const TRANSFORM_KEYS: [&str; 6] = [
        "translation",
        "position",
        "rotation",
        "rotation_quat",
        "scale",
        "look_at",
    ];

    let mut changed = Vec::new();
    if fields
        .keys()
        .any(|key| TRANSFORM_KEYS.contains(&key.as_str()))
    {
        let world = ctx.gpu.scene.main_world_mut();
        let mut transform = world
            .get::<&TransformComponent>(entity)
            .map(|component| component.0)
            .unwrap_or_default();
        apply_transform_params(&mut transform, patch)?;
        world
            .insert_one(entity, TransformComponent(transform))
            .map_err(|err| err.to_string())?;
        changed.push("transform".to_owned());
    }

    for (key, value) in fields {
        if TRANSFORM_KEYS.contains(&key.as_str()) {
            continue;
        }
        match key.as_str() {
            "name" => {
                let name = value.as_str().ok_or("name must be a string")?;
                insert(ctx, entity, Name::new(name))?;
            }
            "parent" => set_parent_field(ctx, entity, value)?,
            "visible" => {
                let visible = value.as_bool().ok_or("visible must be true or false")?;
                insert(ctx, entity, Visible(visible))?;
            }
            "cast_shadow" => {
                let cast = value.as_bool().ok_or("cast_shadow must be true or false")?;
                insert(ctx, entity, CanCastShadow(cast))?;
            }
            "mesh" => set_mesh(&mut ctx.gpu.scene, &mut *ctx.gpu.renderer, entity, value)?,
            "material" => {
                if patch_material(&mut ctx.gpu.scene, &mut *ctx.gpu.renderer, entity, value)? {
                    ctx.gpu
                        .renderer
                        .update_texture_bind_group(&ctx.gpu.scene.assets);
                }
            }
            "point_light" | "directional_light" | "spot_light" | "camera" => {
                patch_light_or_camera(ctx.gpu.scene.main_world_mut(), entity, key, value)?
            }
            "active_camera" => {
                if value.as_bool() == Some(true) {
                    if !has_component(ctx.gpu.scene.main_world(), entity, "camera") {
                        return Err("active_camera needs a camera component".into());
                    }
                    ctx.gpu.scene.set_active_camera_entity(Some(entity));
                }
            }
            "script" => set_script(ctx, entity, value)?,
            "animator" => {
                for note in super::animation::patch_animator(ctx, entity, value)? {
                    changed.push(format!("animator ({note})"));
                }
            }
            "particle_system" => patch_particle_system(ctx, entity, value)?,
            "particle_emitter" => patch_component(
                ctx.gpu.scene.main_world_mut(),
                entity,
                value,
                |emitter: &mut ParticleEmitterComponent, v| {
                    *emitter = patch_serde(emitter, v)?;
                    Ok(())
                },
            )?,
            "environment" => {
                patch_component(
                    ctx.gpu.scene.main_world_mut(),
                    entity,
                    value,
                    |environment: &mut EnvironmentComponent, v| {
                        *environment = patch_serde(environment, v)?;
                        Ok(())
                    },
                )?;
                sync_environment_from_entity(ctx, entity);
            }
            "post_process_volume" => patch_component(
                ctx.gpu.scene.main_world_mut(),
                entity,
                value,
                |volume: &mut PostProcessVolume, v| {
                    *volume = patch_serde(volume, v)?;
                    if v.get("overrides").is_none() {
                        if let Some(settings) = v.get("settings") {
                            volume.tick_overrides_for_patch(settings);
                        }
                    }
                    Ok(())
                },
            )?,
            "light_probe_volume" => patch_component(
                ctx.gpu.scene.main_world_mut(),
                entity,
                value,
                |volume: &mut LightProbeVolume, v| {
                    *volume = patch_serde(volume, v)?;
                    Ok(())
                },
            )?,
            "rigid_body" | "collider" | "character_controller" | "spring_camera" => {
                crate::physics::edit::patch_component(
                    ctx.gpu.scene.main_world_mut(),
                    entity,
                    key,
                    value,
                )?
            }
            "rotate_animation" => patch_component(
                ctx.gpu.scene.main_world_mut(),
                entity,
                value,
                |animation: &mut RotateAnimation, v| {
                    if let Some(axis) = v.get("axis") {
                        animation.axis = vec3_from_json(axis)?.normalize_or_zero();
                    }
                    if let Some(speed) = opt_f64(v, "speed") {
                        animation.speed = speed as f32;
                    }
                    Ok(())
                },
            )?,
            "orbit_animation" => patch_component(
                ctx.gpu.scene.main_world_mut(),
                entity,
                value,
                |animation: &mut OrbitAnimation, v| {
                    if let Some(center) = v.get("center") {
                        animation.center = vec3_from_json(center)?;
                    }
                    if let Some(radius) = opt_f64(v, "radius") {
                        animation.radius = radius as f32;
                    }
                    if let Some(speed) = opt_f64(v, "speed") {
                        animation.speed = speed as f32;
                    }
                    if let Some(offset) = opt_f64(v, "offset") {
                        animation.offset = offset as f32;
                    }
                    Ok(())
                },
            )?,
            "billboard" => patch_component(
                ctx.gpu.scene.main_world_mut(),
                entity,
                value,
                |billboard: &mut Billboard, v| {
                    match opt_str(v, "orientation") {
                        Some("face_camera") => {
                            billboard.orientation = BillboardOrientation::FaceCamera
                        }
                        Some("face_camera_y_axis") => {
                            billboard.orientation = BillboardOrientation::FaceCameraYAxis
                        }
                        Some(other) => {
                            return Err(format!(
                            "unknown orientation {other:?}; use face_camera or face_camera_y_axis"
                        ))
                        }
                        None => {}
                    }
                    if let Some(lit) = opt_bool(v, "lit") {
                        billboard.lit = lit;
                    }
                    Ok(())
                },
            )?,
            other => {
                return Err(format!(
                    "unknown field {other:?}; see the entity_update tool description for the fields"
                ))
            }
        }
        changed.push(key.clone());
    }
    Ok(changed)
}

fn insert<T: hecs::Component>(
    ctx: &mut RemoteContext<'_, '_>,
    entity: Entity,
    component: T,
) -> Result<(), String> {
    ctx.gpu
        .scene
        .main_world_mut()
        .insert_one(entity, component)
        .map_err(|err| err.to_string())
}

/// Insert or patch a component that has a `Default`, or remove it when `value` is null.
fn patch_component<T, F>(
    world: &mut World,
    entity: Entity,
    value: &Value,
    apply: F,
) -> Result<(), String>
where
    T: hecs::Component + Clone + DefaultForRemote,
    F: FnOnce(&mut T, &Value) -> Result<(), String>,
{
    if value.is_null() {
        world
            .remove_one::<T>(entity)
            .map(|_| ())
            .map_err(|err| err.to_string())?;
        return Ok(());
    }
    if !value.is_object() {
        return Err(format!("expected an object or null, got {value}"));
    }
    let mut component = world
        .get::<&T>(entity)
        .map(|component| (*component).clone())
        .unwrap_or_else(|_| T::default_for_remote());
    apply(&mut component, value)?;
    world
        .insert_one(entity, component)
        .map_err(|err| err.to_string())
}

/// Add, change or (with `null`) remove a light or camera component. `field` names it:
/// `point_light`, `directional_light`, `spot_light` or `camera`.
pub(crate) fn patch_light_or_camera(
    world: &mut World,
    entity: Entity,
    field: &str,
    value: &Value,
) -> Result<(), String> {
    match field {
        "point_light" => patch_component(world, entity, value, |light: &mut PointLight, v| {
            patch_light_common(&mut light.color, &mut light.intensity, v)?;
            if let Some(range) = opt_f64(v, "range") {
                light.range = range as f32;
            }
            Ok(())
        })?,
        "directional_light" => {
            patch_component(world, entity, value, |light: &mut DirectionalLight, v| {
                patch_light_common(&mut light.color, &mut light.intensity, v)?;
                if let Some(size) = opt_f64(v, "shadow_size") {
                    light.shadow_size = size as f32;
                }
                Ok(())
            })?
        }
        "spot_light" => patch_component(world, entity, value, |light: &mut SpotLight, v| {
            patch_light_common(&mut light.color, &mut light.intensity, v)?;
            if let Some(range) = opt_f64(v, "range") {
                light.range = range as f32;
            }
            if let Some(angle) = opt_f64(v, "inner_angle_deg") {
                light.inner_angle = (angle as f32).to_radians();
            }
            if let Some(angle) = opt_f64(v, "outer_angle_deg") {
                light.outer_angle = (angle as f32).to_radians();
            }
            Ok(())
        })?,
        "camera" => patch_component(world, entity, value, patch_camera)?,
        other => return Err(format!("{other:?} is not a light or camera")),
    }
    Ok(())
}

/// Light kinds as scripts name them, and the component field of each.
const LIGHT_KINDS: [(&str, &str); 3] = [
    ("point", "point_light"),
    ("directional", "directional_light"),
    ("spot", "spot_light"),
];

/// Change `entity`'s light (for scripts). `patch` holds light fields plus optional
/// `kind` (point, directional or spot), which adds that light or replaces another
/// kind, and `cast_shadow`.
pub(crate) fn set_light(world: &mut World, entity: Entity, patch: &Value) -> Result<(), String> {
    let mut fields = patch
        .as_object()
        .cloned()
        .ok_or("expected a table of light fields")?;
    let kind = fields.remove("kind");
    let cast_shadow = fields.remove("cast_shadow");
    let field = match kind {
        Some(kind) => {
            let kind = kind.as_str().ok_or("light kind must be a string")?;
            let (_, field) = LIGHT_KINDS
                .iter()
                .find(|(name, _)| *name == kind)
                .ok_or_else(|| {
                    format!("unknown light kind {kind:?}; use point, directional or spot")
                })?;
            for (_, other) in LIGHT_KINDS {
                if other != *field && has_component(world, entity, other) {
                    patch_light_or_camera(world, entity, other, &Value::Null)?;
                }
            }
            *field
        }
        None => LIGHT_KINDS
            .iter()
            .map(|(_, field)| *field)
            .find(|field| has_component(world, entity, field))
            .ok_or("the entity has no light; pass a kind to add one")?,
    };
    patch_light_or_camera(world, entity, field, &Value::Object(fields))?;
    if let Some(cast) = cast_shadow {
        let cast = cast.as_bool().ok_or("cast_shadow must be true or false")?;
        world
            .insert_one(entity, CanCastShadow(cast))
            .map_err(|err| err.to_string())?;
    }
    ensure_transform(world, entity)
}

/// Give `entity` a camera or change it (for scripts). `active: true` also makes it the
/// camera the game is viewed through.
pub(crate) fn set_camera(scene: &mut Scene, entity: Entity, patch: &Value) -> Result<(), String> {
    let mut fields = patch
        .as_object()
        .cloned()
        .ok_or("expected a table of camera fields")?;
    let active = match fields.remove("active") {
        Some(active) => active.as_bool().ok_or("active must be true or false")?,
        None => false,
    };
    let world = scene.main_world_mut();
    patch_light_or_camera(world, entity, "camera", &Value::Object(fields))?;
    ensure_transform(world, entity)?;
    if active {
        scene.set_active_camera_entity(Some(entity));
    }
    Ok(())
}

/// View the game through `entity`'s camera.
pub(crate) fn set_active_camera(scene: &mut Scene, entity: Entity) -> Result<(), String> {
    if !has_component(scene.main_world(), entity, "camera") {
        return Err("the entity has no camera".into());
    }
    scene.set_active_camera_entity(Some(entity));
    Ok(())
}

/// Lights and cameras are placed by their transform; entities spawned bare by a script
/// have none.
fn ensure_transform(world: &mut World, entity: Entity) -> Result<(), String> {
    if has_component(world, entity, "transform") {
        return Ok(());
    }
    world
        .insert_one(entity, TransformComponent(Transform::default()))
        .map_err(|err| err.to_string())
}

/// Starting value for a component added through the remote API.
trait DefaultForRemote {
    fn default_for_remote() -> Self;
}

macro_rules! default_for_remote_from_default {
    ($($ty:ty),* $(,)?) => {
        $(impl DefaultForRemote for $ty {
            fn default_for_remote() -> Self {
                <$ty>::default()
            }
        })*
    };
}

default_for_remote_from_default!(
    PointLight,
    DirectionalLight,
    SpotLight,
    CameraComponent,
    ParticleEmitterComponent,
    EnvironmentComponent,
    PostProcessVolume,
    LightProbeVolume,
);

impl DefaultForRemote for RotateAnimation {
    fn default_for_remote() -> Self {
        Self {
            axis: Vec3::Y,
            speed: 1.0,
        }
    }
}

impl DefaultForRemote for OrbitAnimation {
    fn default_for_remote() -> Self {
        Self {
            center: Vec3::ZERO,
            radius: 3.0,
            speed: 1.0,
            offset: 0.0,
        }
    }
}

impl DefaultForRemote for Billboard {
    fn default_for_remote() -> Self {
        Billboard::new(BillboardOrientation::FaceCamera)
    }
}

fn patch_light_common(color: &mut Vec3, intensity: &mut f32, value: &Value) -> Result<(), String> {
    if let Some(c) = value.get("color") {
        let [r, g, b, _] = color_from_json(c)?;
        *color = Vec3::new(r, g, b);
    }
    if let Some(i) = opt_f64(value, "intensity") {
        *intensity = i as f32;
    }
    Ok(())
}

fn patch_camera(camera: &mut CameraComponent, value: &Value) -> Result<(), String> {
    if let Some(projection) = value.get("projection").filter(|p| p.is_object()) {
        camera.projection = serde_json::from_value(projection.clone())
            .map_err(|err| format!("invalid projection: {err}"))?;
    }
    if opt_str(value, "projection") == Some("orthographic") {
        if let CameraProjection::Perspective { near, far, .. } = camera.projection {
            camera.projection = CameraProjection::orthographic(-5.0, 5.0, -5.0, 5.0, near, far);
        }
    }
    if opt_str(value, "projection") == Some("perspective") {
        if let CameraProjection::Orthographic { near, far, .. } = camera.projection {
            camera.projection = CameraProjection::perspective(60f32.to_radians(), near, far);
        }
    }
    let fov = opt_f64(value, "fov_deg").map(|fov| (fov as f32).to_radians());
    let new_near = opt_f64(value, "near").map(|v| v as f32);
    let new_far = opt_f64(value, "far").map(|v| v as f32);
    camera.projection = match camera.projection {
        CameraProjection::Perspective {
            fov_y_radians,
            near,
            far,
        } => CameraProjection::perspective(
            fov.unwrap_or(fov_y_radians),
            new_near.unwrap_or(near),
            new_far.unwrap_or(far),
        ),
        CameraProjection::Orthographic {
            left,
            right,
            bottom,
            top,
            near,
            far,
        } => {
            let field = |key: &str, current: f32| opt_f64(value, key).map_or(current, |v| v as f32);
            CameraProjection::orthographic(
                field("left", left),
                field("right", right),
                field("bottom", bottom),
                field("top", top),
                new_near.unwrap_or(near),
                new_far.unwrap_or(far),
            )
        }
    };
    Ok(())
}

/// Give `entity` a primitive mesh (`value` is its name), or remove its mesh (`null`).
pub(crate) fn set_mesh(
    scene: &mut Scene,
    renderer: &mut Renderer,
    entity: Entity,
    value: &Value,
) -> Result<(), String> {
    let world = scene.main_world_mut();
    if value.is_null() {
        let _ = world.remove_one::<MeshComponent>(entity);
        let _ = world.remove_one::<PrimitiveMeshComponent>(entity);
        let _ = world.remove_one::<MeshBounds>(entity);
        return Ok(());
    }
    let name = value.as_str().ok_or(
        "mesh must be a primitive name (cube, sphere, plane, cylinder, cone, torus) or null",
    )?;
    let descriptor =
        primitive_from_name(name).ok_or_else(|| format!("unknown primitive {name:?}"))?;
    let (mesh, bounds) = primitive_mesh(scene, renderer, descriptor);
    attach_mesh(scene, entity, mesh, bounds)?;
    scene
        .main_world_mut()
        .insert_one(entity, PrimitiveMeshComponent { descriptor })
        .map_err(|err| err.to_string())
}

/// Give `entity` a mesh built at runtime (e.g. one colour of a voxel model).
pub(crate) fn set_generated_mesh(
    scene: &mut Scene,
    renderer: &mut Renderer,
    entity: Entity,
    mesh: &GeneratedMesh,
) -> Result<(), String> {
    let handle = scene.assets.ensure_generated_mesh(renderer, mesh);
    let bounds = (!mesh.data.vertices.is_empty()).then(|| MeshBounds::new(mesh.min, mesh.max));
    attach_mesh(scene, entity, MeshComponent(handle), bounds)?;
    let _ = scene
        .main_world_mut()
        .remove_one::<PrimitiveMeshComponent>(entity);
    Ok(())
}

/// Replace `entity`'s geometry, keeping its material (or giving it a new one) and making
/// sure it has what drawing needs.
fn attach_mesh(
    scene: &mut Scene,
    entity: Entity,
    mesh: MeshComponent,
    bounds: Option<MeshBounds>,
) -> Result<(), String> {
    let has_material = scene
        .main_world()
        .satisfies::<&MaterialComponent>(entity)
        .unwrap_or(false);
    // Keep an existing material; only the geometry changes.
    let material = (!has_material).then(|| new_pbr_material(scene));
    let world = scene.main_world_mut();
    world
        .insert_one(entity, mesh)
        .map_err(|err| err.to_string())?;
    if let Some(material) = material {
        world
            .insert_one(entity, material)
            .map_err(|err| err.to_string())?;
    }
    // Only visible entities with a transform are drawn; entities spawned bare (e.g. by
    // a script) have neither.
    if !world.satisfies::<&Visible>(entity).unwrap_or(true) {
        let _ = world.insert_one(entity, Visible(true));
    }
    if !world
        .satisfies::<&TransformComponent>(entity)
        .unwrap_or(true)
    {
        let _ = world.insert_one(entity, TransformComponent(Transform::default()));
    }
    match bounds {
        Some(bounds) => {
            let _ = world.insert_one(entity, bounds);
        }
        None => {
            let _ = world.remove_one::<MeshBounds>(entity);
        }
    }
    Ok(())
}

/// Change the given fields of `entity`'s material, copying a shared material first
/// unless `edit_shared` is set. Returns whether new textures were loaded, in which case
/// the renderer's texture bind group must be rebuilt.
pub(crate) fn patch_material(
    scene: &mut Scene,
    renderer: &mut Renderer,
    entity: Entity,
    patch: &Value,
) -> Result<bool, String> {
    if !patch.is_object() {
        return Err("material must be an object".into());
    }
    let edit_shared = opt_bool(patch, "edit_shared").unwrap_or(false);

    let current = scene
        .main_world()
        .get::<&MaterialComponent>(entity)
        .ok()
        .map(|component| component.0);
    let handle = match current {
        Some(handle) => {
            let users = scene
                .main_world()
                .query::<&MaterialComponent>()
                .iter()
                .filter(|(_, other)| other.0 == handle)
                .count();
            let is_default = handle == scene.assets.default_material_handle();
            if (users > 1 || is_default) && !edit_shared {
                // Copy on write so changing one entity does not recolor the others.
                let mut copy = scene
                    .assets
                    .materials
                    .get(handle)
                    .cloned()
                    .ok_or("material asset missing")?;
                if !copy.canonical_path().as_os_str().is_empty() {
                    copy = MaterialAsset::from_material(*copy.material(), PathBuf::new());
                }
                let new_handle = scene.assets.materials.insert(copy);
                scene
                    .main_world_mut()
                    .insert_one(entity, MaterialComponent(new_handle))
                    .map_err(|err| err.to_string())?;
                new_handle
            } else {
                handle
            }
        }
        None => {
            let handle = scene.assets.materials.insert(MaterialAsset::from_material(
                Material::pbr(),
                PathBuf::new(),
            ));
            scene
                .main_world_mut()
                .insert_one(entity, MaterialComponent(handle))
                .map_err(|err| err.to_string())?;
            handle
        }
    };

    let mut material = *scene
        .assets
        .materials
        .get(handle)
        .ok_or("material asset missing")?
        .material();

    if let Some(color) = patch.get("color").or_else(|| patch.get("base_color")) {
        let rgba = color_from_json(color)?;
        material.base_color = rgba.map(|channel| unit_to_u8(f64::from(channel)));
    }
    if let Some(metallic) = opt_f64(patch, "metallic") {
        material.metallic_factor = unit_to_u8(metallic);
    }
    if let Some(roughness) = opt_f64(patch, "roughness") {
        material.roughness_factor = unit_to_u8(roughness);
    }
    if let Some(emissive) = opt_f64(patch, "emissive") {
        material.emissive_strength = Material::encode_emissive(emissive as f32);
    }
    if let Some(transmission) = opt_f64(patch, "transmission") {
        material.transmission_factor = unit_to_u8(transmission);
    }
    if let Some(ior) = opt_f64(patch, "ior") {
        material.ior = Material::encode_ior(ior as f32);
    }
    if let Some(dispersion) = opt_f64(patch, "dispersion") {
        material.dispersion = Material::encode_dispersion(dispersion as f32);
    }
    if let Some(cutoff) = opt_f64(patch, "alpha_cutoff") {
        material.alpha_cutoff = Material::encode_alpha_cutoff(cutoff as f32);
    }
    for (key, flag) in [
        ("unlit", MaterialFlags::UNLIT),
        ("transparent", MaterialFlags::ALPHA_BLEND),
        ("double_sided", MaterialFlags::DOUBLE_SIDED),
        ("nearest_filtering", MaterialFlags::USE_NEAREST_FILTERING),
    ] {
        match opt_bool(patch, key) {
            Some(true) => material.flags.insert(flag),
            Some(false) => material.flags.remove(flag),
            None => {}
        }
    }

    let mut references = Vec::new();
    let mut textures_loaded = false;
    for (key, slot, flag, srgb) in TEXTURE_SLOTS {
        let Some(value) = patch.get(key) else {
            continue;
        };
        let index = if value.is_null() {
            material.flags.remove(flag);
            references.push((slot, MaterialTextureReference::new(None, None)));
            crate::asset::Assets::default_texture_index(slot)
        } else {
            let path = resolve_existing_path(
                value
                    .as_str()
                    .ok_or_else(|| format!("{key} must be an image path or null"))?,
            )?;
            let before = scene.assets.textures.len();
            let texture = scene.assets.get_or_load_texture(renderer, &path, srgb)?;
            textures_loaded |= scene.assets.textures.len() != before;
            material.flags.insert(flag);
            let display = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned());
            references.push((slot, MaterialTextureReference::new(Some(path), display)));
            texture.index() as u32
        };
        match slot {
            MaterialTextureSlot::BaseColor => material.base_color_texture = index,
            MaterialTextureSlot::MetallicRoughness => material.metallic_roughness_texture = index,
            MaterialTextureSlot::Normal => material.normal_texture = index,
            MaterialTextureSlot::Emissive => material.emissive_texture = index,
            MaterialTextureSlot::Occlusion => material.occlusion_texture = index,
        }
    }

    let asset = scene
        .assets
        .materials
        .get_mut(handle)
        .ok_or("material asset missing")?;
    *asset.material_mut() = material;
    for (slot, reference) in references {
        asset.set_texture_reference(slot, reference);
    }
    Ok(textures_loaded)
}

fn patch_particle_system(
    ctx: &mut RemoteContext<'_, '_>,
    entity: Entity,
    value: &Value,
) -> Result<(), String> {
    if value.is_null() {
        let world = ctx.gpu.scene.main_world_mut();
        let _ = world.remove_one::<ParticleSystemComponent>(entity);
        let _ = world.remove_one::<ParticleEmitterComponent>(entity);
        return Ok(());
    }
    let existing = ctx
        .gpu
        .scene
        .main_world()
        .get::<&ParticleSystemComponent>(entity)
        .map(|system| (*system).clone())
        .ok();
    let mut patch = value.clone();
    let preset = match patch
        .as_object_mut()
        .and_then(|object| object.remove("preset"))
    {
        Some(Value::String(name)) => Some(particle_preset_from_name(&name)?),
        Some(other) => return Err(format!("preset must be a string, got {other}")),
        None => None,
    };

    let mut system = match existing {
        Some(system) => system,
        None => {
            // A new particle system needs the quad mesh and emitter the renderer expects.
            let preset = preset.unwrap_or_default();
            let mut builder = hecs::EntityBuilder::new();
            add_particle_components(
                &mut ctx.gpu.scene,
                &mut *ctx.gpu.renderer,
                &mut builder,
                preset,
            );
            ctx.gpu
                .scene
                .main_world_mut()
                .insert(entity, builder.build())
                .map_err(|err| err.to_string())?;
            ParticleSystemComponent::new(120.0, preset)
        }
    };
    if let Some(preset) = preset {
        if preset != system.behavior {
            system.set_behavior(preset);
        }
    }
    let mut system = patch_serde(&system, &patch)?;
    system.behavior_config = std::mem::replace(
        &mut system.behavior_config,
        ParticleBehaviorConfig::from_preset(system.behavior),
    )
    .ensure_variant(system.behavior);
    insert(ctx, entity, system)
}

fn set_script(
    ctx: &mut RemoteContext<'_, '_>,
    entity: Entity,
    value: &Value,
) -> Result<(), String> {
    let world = ctx.gpu.scene.main_world_mut();
    if value.is_null() {
        world
            .remove_one::<LuaScriptComponent>(entity)
            .map(|_| ())
            .map_err(|err| err.to_string())?;
        return Ok(());
    }
    let source = if let Some(path) = opt_str(value, "path") {
        LuaScriptSource::file(resolve_existing_path(path)?)
    } else if let Some(source) = opt_str(value, "source") {
        let name = opt_str(value, "name").unwrap_or("remote_script");
        LuaScriptSource::inline(name, source)
    } else {
        return Err(
            "script needs \"path\" (a .lua file) or \"source\" (inline Lua), or null to remove"
                .into(),
        );
    };
    // A new component always gets a fresh instance, compiled on the next update.
    world
        .insert_one(entity, LuaScriptComponent::new(source))
        .map_err(|err| err.to_string())
}

/// Resolve a path given by a client to an absolute path that exists.
///
/// Relative paths are tried against the working directory, then the active project
/// root and its content folder.
pub(super) fn resolve_existing_path(path: &str) -> Result<PathBuf, String> {
    let raw = Path::new(path);
    let mut candidates = vec![raw.to_path_buf()];
    if raw.is_relative() {
        if let Some(root) = crate::project::active_project_root() {
            candidates.push(root.join(raw));
            candidates.push(root.join(crate::project::CONTENT_DIR).join(raw));
        }
    }
    candidates
        .iter()
        .find(|candidate| candidate.exists())
        .map(|found| found.canonicalize().unwrap_or_else(|_| found.clone()))
        .ok_or_else(|| format!("file not found: {path}"))
}

fn set_parent_field(
    ctx: &mut RemoteContext<'_, '_>,
    entity: Entity,
    value: &Value,
) -> Result<(), String> {
    let world = ctx.gpu.scene.main_world_mut();
    let parent = if value.is_null() {
        None
    } else {
        Some(entity_from_json(world, value)?)
    };
    set_parent(world, entity, parent)
}

/// Re-parent `entity`, keeping its local transform. Rejects cycles.
pub(super) fn set_parent(
    world: &mut World,
    entity: Entity,
    parent: Option<Entity>,
) -> Result<(), String> {
    if let Some(parent) = parent {
        let mut cursor = Some(parent);
        while let Some(current) = cursor {
            if current == entity {
                return Err("cannot parent an entity to itself or one of its descendants".into());
            }
            cursor = world.get::<&Parent>(current).ok().map(|p| p.0);
        }
    }

    if let Ok(old_parent) = world.get::<&Parent>(entity).map(|p| p.0) {
        let mut now_empty = false;
        if let Ok(mut children) = world.get::<&mut Children>(old_parent) {
            children.0.retain(|child| *child != entity);
            now_empty = children.0.is_empty();
        }
        if now_empty {
            let _ = world.remove_one::<Children>(old_parent);
        }
    }

    match parent {
        Some(parent) => {
            world
                .insert_one(entity, Parent(parent))
                .map_err(|err| err.to_string())?;
            let appended = world
                .get::<&mut Children>(parent)
                .map(|mut children| {
                    if !children.0.contains(&entity) {
                        children.0.push(entity);
                    }
                })
                .is_ok();
            if !appended {
                world
                    .insert_one(parent, Children(vec![entity]))
                    .map_err(|err| err.to_string())?;
            }
        }
        None => {
            let _ = world.remove_one::<Parent>(entity);
        }
    }
    Ok(())
}

/// Push an environment entity's settings to the scene.
fn sync_environment_from_entity(ctx: &mut RemoteContext<'_, '_>, entity: Entity) {
    let component = ctx
        .gpu
        .scene
        .main_world()
        .get::<&EnvironmentComponent>(entity)
        .ok()
        .map(|component| (*component).clone());
    if let Some(component) = component {
        ctx.gpu.scene.set_environment(component.to_environment());
    }
}

pub(super) fn remove_component(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let entity = req_entity(ctx.gpu.scene.main_world(), params)?;
    let component = opt_str(params, "component").ok_or("missing parameter \"component\"")?;
    if !COMPONENT_NAMES.contains(&component) {
        return Err(format!(
            "unknown component {component:?}; known: {}",
            COMPONENT_NAMES.join(", ")
        ));
    }
    if !has_component(ctx.gpu.scene.main_world(), entity, component) {
        return Err(format!("entity has no {component} component"));
    }
    let world = ctx.gpu.scene.main_world_mut();
    fn remove<T: hecs::Component>(world: &mut World, entity: Entity) {
        let _ = world.remove_one::<T>(entity);
    }
    match component {
        "transform" => remove::<TransformComponent>(world, entity),
        "mesh" => {
            remove::<MeshComponent>(world, entity);
            remove::<PrimitiveMeshComponent>(world, entity);
            remove::<MeshBounds>(world, entity);
        }
        "material" => remove::<MaterialComponent>(world, entity),
        "visible" => remove::<Visible>(world, entity),
        "point_light" => remove::<PointLight>(world, entity),
        "directional_light" => remove::<DirectionalLight>(world, entity),
        "spot_light" => remove::<SpotLight>(world, entity),
        "cast_shadow" => remove::<CanCastShadow>(world, entity),
        "camera" => remove::<CameraComponent>(world, entity),
        "script" => remove::<LuaScriptComponent>(world, entity),
        "particle_system" => {
            remove::<ParticleSystemComponent>(world, entity);
            remove::<ParticleEmitterComponent>(world, entity);
        }
        "particle_emitter" => remove::<ParticleEmitterComponent>(world, entity),
        "environment" => remove::<EnvironmentComponent>(world, entity),
        "post_process_volume" => remove::<PostProcessVolume>(world, entity),
        "light_probe_volume" => remove::<LightProbeVolume>(world, entity),
        "rigid_body" | "collider" | "character_controller" | "spring_camera" => {
            let _ = crate::physics::edit::patch_component(world, entity, component, &Value::Null);
        }
        "billboard" => remove::<Billboard>(world, entity),
        "depth_state" => remove::<DepthState>(world, entity),
        "rotate_animation" => remove::<RotateAnimation>(world, entity),
        "orbit_animation" => remove::<OrbitAnimation>(world, entity),
        "gltf" => remove::<GltfSource>(world, entity),
        "skinned_mesh" => remove::<SkinnedMesh>(world, entity),
        "animator" => remove::<Animator>(world, entity),
        _ => unreachable!("validated against COMPONENT_NAMES"),
    }
    if component == "camera" && ctx.gpu.scene.active_camera_entity() == Some(entity) {
        ctx.gpu.scene.set_active_camera_entity(None);
    }
    ctx.mark_modified();
    Ok(entity_summary(ctx.gpu.scene.main_world(), entity))
}

/// `root` and all its descendants, parents before children.
fn subtree(world: &World, root: Entity) -> Vec<Entity> {
    let mut order = Vec::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if !world.contains(entity) || order.contains(&entity) {
            continue;
        }
        order.push(entity);
        if let Ok(children) = world.get::<&Children>(entity) {
            stack.extend(children.0.iter().rev().copied());
        }
    }
    order
}

pub(super) fn delete(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let world = ctx.gpu.scene.main_world_mut();
    let targets = match bulk_targets(world, params)? {
        Some(targets) => targets,
        None => vec![req_entity(world, params)?],
    };
    let mut removed = Vec::new();
    for entity in targets {
        // A target may already be gone as a descendant of an earlier one.
        if !world.contains(entity) {
            continue;
        }
        set_parent(world, entity, None)?;
        let subtree = subtree(world, entity);
        for entity in subtree.iter().rev() {
            let _ = world.despawn(*entity);
        }
        removed.extend(subtree);
    }
    if ctx
        .gpu
        .scene
        .active_camera_entity()
        .is_some_and(|camera| removed.contains(&camera))
    {
        ctx.gpu.scene.set_active_camera_entity(None);
    }
    ctx.gpu.scene.propagate_transforms();
    ctx.mark_modified();
    ctx.gpu.scene.collect_unused_materials();
    Ok(json!({ "deleted": removed.into_iter().map(entity_id).collect::<Vec<_>>() }))
}

/// Copy the components the engine knows about from `source` into a builder.
fn clone_components(scene: &mut Scene, source: Entity) -> hecs::EntityBuilder {
    fn copy<T: hecs::Component + Clone>(
        world: &World,
        entity: Entity,
        builder: &mut hecs::EntityBuilder,
    ) {
        if let Ok(component) = world.get::<&T>(entity) {
            builder.add((*component).clone());
        }
    }

    let mut builder = hecs::EntityBuilder::new();
    let world = scene.main_world();
    copy::<Name>(world, source, &mut builder);
    copy::<TransformComponent>(world, source, &mut builder);
    copy::<Visible>(world, source, &mut builder);
    copy::<MeshComponent>(world, source, &mut builder);
    copy::<PrimitiveMeshComponent>(world, source, &mut builder);
    copy::<MeshBounds>(world, source, &mut builder);
    copy::<PointLight>(world, source, &mut builder);
    copy::<DirectionalLight>(world, source, &mut builder);
    copy::<SpotLight>(world, source, &mut builder);
    copy::<CanCastShadow>(world, source, &mut builder);
    copy::<CameraComponent>(world, source, &mut builder);
    copy::<LuaScriptComponent>(world, source, &mut builder);
    copy::<ParticleSystemComponent>(world, source, &mut builder);
    copy::<ParticleEmitterComponent>(world, source, &mut builder);
    copy::<EnvironmentComponent>(world, source, &mut builder);
    copy::<PostProcessVolume>(world, source, &mut builder);
    copy::<LightProbeVolume>(world, source, &mut builder);
    crate::physics::edit::clone_components(world, source, &mut builder);
    copy::<Billboard>(world, source, &mut builder);
    copy::<DepthState>(world, source, &mut builder);
    copy::<RotateAnimation>(world, source, &mut builder);
    copy::<OrbitAnimation>(world, source, &mut builder);
    copy::<GltfSource>(world, source, &mut builder);
    copy::<SkinnedMesh>(world, source, &mut builder);
    copy::<Animator>(world, source, &mut builder);

    let material = world.get::<&MaterialComponent>(source).ok().map(|m| m.0);
    if let Some(handle) = material {
        // Materials with a file path are shared assets; inline ones get their own copy.
        let copied = scene.assets.materials.get(handle).cloned();
        let handle = match copied {
            Some(asset) if asset.canonical_path().as_os_str().is_empty() => {
                scene.assets.materials.insert(asset)
            }
            _ => handle,
        };
        builder.add(MaterialComponent(handle));
    }
    builder
}

pub(super) fn duplicate(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let source = req_entity(ctx.gpu.scene.main_world(), params)?;
    let quiet = opt_bool(params, "quiet").unwrap_or(false);
    let count = opt_u64(params, "count").unwrap_or(1);
    if !(1..=1000).contains(&count) {
        return Err("count must be between 1 and 1000".into());
    }
    let vec3_param = |key: &str| {
        params
            .get(key)
            .map(vec3_from_json)
            .transpose()
            .map(Option::unwrap_or_default)
    };
    let step = vec3_param("step")?;
    let row_step = vec3_param("row_step")?;
    let columns = opt_u64(params, "columns").filter(|&columns| columns > 0);
    let arrayed = params.get("step").is_some() || params.get("row_step").is_some();

    let mut patch = params.clone();
    let mut base = None;
    if let Some(object) = patch.as_object_mut() {
        for key in ["entity", "quiet", "count", "step", "row_step", "columns"] {
            object.remove(key);
        }
        if arrayed {
            // Copies are placed relative to this position (the source's by default).
            base = object
                .remove("translation")
                .or_else(|| object.remove("position"))
                .map(|value| vec3_from_json(&value))
                .transpose()?;
        }
    }
    let base = base.unwrap_or_else(|| {
        ctx.gpu
            .scene
            .main_world()
            .get::<&TransformComponent>(source)
            .map(|transform| transform.0.translation)
            .unwrap_or_default()
    });
    let source_name =
        entity_name(ctx.gpu.scene.main_world(), source).unwrap_or_else(|| "Copy".into());
    let name = opt_str(params, "name").map(str::to_owned);

    let mut copies = Vec::with_capacity(count as usize);
    for index in 1..=count {
        let mut copy_patch = patch.clone();
        let copy_name = match &name {
            Some(name) if name.contains("{i}") => name.replace("{i}", &index.to_string()),
            Some(name) if count > 1 => format!("{name} {index}"),
            Some(name) => name.clone(),
            None if count > 1 => format!("{source_name} {index}"),
            None => format!("{source_name} (copy)"),
        };
        copy_patch["name"] = json!(copy_name);
        if arrayed {
            // Copy `index` fills grid cell `index`; the source occupies cell 0.
            let (column, row) = match columns {
                Some(columns) => (index % columns, index / columns),
                None => (index, 0),
            };
            let offset = step * column as f32 + row_step * row as f32;
            copy_patch["translation"] = vec3_json(base + offset);
        }
        match duplicate_once(ctx, source, &copy_patch) {
            Ok(copy) => copies.push(copy),
            Err(err) => {
                ctx.gpu.scene.propagate_transforms();
                ctx.mark_modified();
                return Err(format!("{err} (copy {index}; earlier copies were kept)"));
            }
        }
    }
    ctx.gpu.scene.propagate_transforms();
    ctx.mark_modified();

    let world = ctx.gpu.scene.main_world();
    if count == 1 {
        let (root, copied_entities) = copies[0];
        if quiet {
            return Ok(brief(world, root));
        }
        let mut summary = entity_summary(world, root);
        summary["copied_entities"] = json!(copied_entities);
        return Ok(summary);
    }
    if quiet {
        // Ids need not be consecutive (freed slots are reused), so point at names instead.
        let (first, _) = copies[0];
        let (last, _) = copies[copies.len() - 1];
        return Ok(json!({
            "count": copies.len(),
            "first": brief(world, first),
            "last": brief(world, last),
            "note": "Address the copies by name (entity_update/entity_delete name_contains, find_entity_by_name in Lua).",
        }));
    }
    Ok(json!({
        "copies": copies
            .iter()
            .map(|&(root, _)| brief(world, root))
            .collect::<Vec<_>>(),
    }))
}

/// Copy `source` and its children once and apply `patch` to the copy. Returns the
/// copy and how many entities were copied.
fn duplicate_once(
    ctx: &mut RemoteContext<'_, '_>,
    source: Entity,
    patch: &Value,
) -> Result<(Entity, usize), String> {
    let originals = subtree(ctx.gpu.scene.main_world(), source);
    let mut copies: Vec<(Entity, Entity)> = Vec::with_capacity(originals.len());

    for original in &originals {
        let mut builder = clone_components(&mut ctx.gpu.scene, *original);
        let copy = ctx.gpu.scene.main_world_mut().spawn(builder.build());
        copies.push((*original, copy));
    }

    let world = ctx.gpu.scene.main_world_mut();
    let mapped = |entity: Entity| {
        copies
            .iter()
            .find(|(original, _)| *original == entity)
            .map(|(_, copy)| *copy)
    };
    let source_parent = world.get::<&Parent>(source).ok().map(|p| p.0);
    for (original, copy) in &copies {
        let parent = if *original == source {
            source_parent
        } else {
            world
                .get::<&Parent>(*original)
                .ok()
                .and_then(|p| mapped(p.0))
        };
        set_parent(world, *copy, parent)?;
    }

    // A copied character deforms with its copied skeleton, not the original's.
    for (_, copy) in &copies {
        if let Ok(mut skin) = world.get::<&mut SkinnedMesh>(*copy) {
            skin.remap(mapped);
        }
    }

    let root = copies[0].1;
    apply_patch(ctx, root, patch)?;
    Ok((root, copies.len()))
}

/// World matrix → local transform under `parent_world` (if any).
pub(super) fn world_to_local(world_matrix: Mat4, parent_world: Option<&Transform>) -> Transform {
    let local = match parent_world {
        Some(parent) => parent.matrix().inverse() * world_matrix,
        None => world_matrix,
    };
    let (scale, rotation, translation) = local.to_scale_rotation_translation();
    Transform::from_trs(translation, rotation, scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_light_adds_switches_and_patches_lights() {
        let mut world = World::new();
        let entity = world.spawn((Name::new("Flash"),));

        let err = set_light(&mut world, entity, &json!({ "intensity": 2.0 })).unwrap_err();
        assert!(err.contains("no light"), "{err}");

        set_light(
            &mut world,
            entity,
            &json!({ "kind": "point", "color": [1.0, 0.5, 0.0], "range": 7.0, "cast_shadow": true }),
        )
        .unwrap();
        let light = *world.get::<&PointLight>(entity).unwrap();
        assert_eq!(light.color, Vec3::new(1.0, 0.5, 0.0));
        assert_eq!(light.range, 7.0);
        assert!(world.get::<&CanCastShadow>(entity).unwrap().0);
        assert!(has_component(&world, entity, "transform"));

        // Without a kind, the existing light is changed.
        set_light(&mut world, entity, &json!({ "intensity": 9.0 })).unwrap();
        assert_eq!(world.get::<&PointLight>(entity).unwrap().intensity, 9.0);

        set_light(
            &mut world,
            entity,
            &json!({ "kind": "spot", "outer_angle_deg": 30.0 }),
        )
        .unwrap();
        assert!(!has_component(&world, entity, "point_light"));
        let spot = *world.get::<&SpotLight>(entity).unwrap();
        assert!((spot.outer_angle - 30f32.to_radians()).abs() < 1e-6);

        assert!(set_light(&mut world, entity, &json!({ "kind": "area" })).is_err());
    }

    #[test]
    fn set_camera_adds_a_camera_and_activates_it() {
        let mut scene = Scene::new();
        let entity = scene.main_world_mut().spawn((Name::new("Cut"),));

        assert!(set_active_camera(&mut scene, entity).is_err());

        set_camera(
            &mut scene,
            entity,
            &json!({ "fov_deg": 40.0, "active": true }),
        )
        .unwrap();
        let camera = *scene.main_world().get::<&CameraComponent>(entity).unwrap();
        match camera.projection {
            CameraProjection::Perspective { fov_y_radians, .. } => {
                assert!((fov_y_radians - 40f32.to_radians()).abs() < 1e-6)
            }
            other => panic!("expected a perspective camera, got {other:?}"),
        }
        assert_eq!(scene.active_camera_entity(), Some(entity));
    }
}
