//! Skinned meshes and animators over remote control: `entity.get` reports them,
//! `entity.update {animator}` changes them, `animation.inspect` lists a file's clips.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use hecs::Entity;
use serde_json::{json, Value};

use crate::app::RuntimeMode;
use crate::asset::Assets;
use crate::scene::components::MeshComponent;
use crate::scene::skeletal::{
    animator_entity, Animator, AnimatorCommand, ClipLibrary, PlayOptions, RootMotion, SkinnedMesh,
    MAX_LAYERS,
};
use crate::scene::Scene;

use super::super::json::{entity_from_json, entity_id, opt_bool, opt_f64, opt_str, round};
use super::{RemoteContext, RemoteResult};

/// Most missing bone names listed per clip.
const MISSING_SHOWN: usize = 8;

/// Load every library the animator uses, then bind a copy of it (playback state
/// included) so its clips and coverage can be reported in edit mode too.
fn bound_copy(scene: &mut Scene, entity: Entity) -> Option<Animator> {
    let mut animator = Animator::clone(&*scene.main_world().get::<&Animator>(entity).ok()?);
    let libraries: HashMap<PathBuf, Result<Arc<ClipLibrary>, String>> = animator
        .libraries
        .iter()
        .map(|path| (path.clone(), scene.assets.clip_library(path)))
        .collect();
    animator.bind_with(scene.main_world(), entity, &mut |path| {
        libraries
            .get(path)
            .cloned()
            .unwrap_or_else(|| Err(format!("{} not loaded", path.display())))
    });
    Some(animator)
}

/// `entity.get`'s `animator` section.
pub(super) fn animator_json(scene: &mut Scene, entity: Entity) -> Option<Value> {
    let animator = bound_copy(scene, entity)?;
    let clips: Vec<Value> = animator
        .clips()
        .into_iter()
        .map(|clip| {
            let mut value = json!({
                "name": clip.name,
                "duration": round(clip.duration),
                "library": clip.library.display().to_string(),
                "tracks": clip.tracks,
                "bound": clip.bound,
                "root_speed": round(clip.root_speed),
            });
            if clip.source_name != value["name"] {
                value["source_name"] = json!(clip.source_name);
            }
            if !clip.missing.is_empty() {
                value["missing_bones"] = json!(clip.missing.len());
                value["missing_examples"] =
                    json!(clip.missing.iter().take(MISSING_SHOWN).collect::<Vec<_>>());
            }
            value
        })
        .collect();
    let layers: Vec<Value> = animator
        .state
        .layers
        .iter()
        .enumerate()
        .filter(|(_, layer)| !layer.clips.is_empty() || layer.mask.is_some())
        .map(|(index, layer)| {
            json!({
                "layer": index,
                "weight": round(layer.weight),
                "mask": layer.mask,
                "clips": layer.clips.iter().map(|clip| json!({
                    "name": clip.name,
                    "time": round(clip.time),
                    "weight": round(clip.weight),
                    "speed": round(clip.speed),
                    "loop": clip.looping,
                    "finished": clip.finished,
                    "fading_out": clip.fading_out(),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let mut value = json!({
        "libraries": animator
            .libraries
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>(),
        "default_clip": animator.default_clip,
        "speed": round(animator.speed),
        "root_motion": animator.root_motion.as_str(),
        "clips": clips,
        "playing": layers,
    });
    let errors = animator.library_errors();
    if !errors.is_empty() {
        value["errors"] = json!(errors);
    }
    Some(value)
}

/// `entity.get`'s `skinned_mesh` section.
pub(super) fn skinned_mesh_json(scene: &Scene, entity: Entity) -> Option<Value> {
    let world = scene.main_world();
    let skin = world.get::<&SkinnedMesh>(entity).ok()?;
    let vertices = world
        .get::<&MeshComponent>(entity)
        .ok()
        .and_then(|mesh| scene.assets.meshes.get(mesh.0))
        .map(|mesh| mesh.vertex_count());
    let missing = skin.joints.iter().filter(|&&j| !world.contains(j)).count();
    let mut value = json!({
        "joints": skin.joints.len(),
        "vertices": vertices,
        "animator": animator_entity(world, entity).map(entity_id),
    });
    if missing > 0 {
        value["missing_joints"] = json!(missing);
    }
    Some(value)
}

fn play_options(value: &Value) -> Result<(String, PlayOptions), String> {
    if let Some(clip) = value.as_str() {
        return Ok((clip.to_string(), PlayOptions::default()));
    }
    let clip = opt_str(value, "clip")
        .ok_or("play needs a clip name, or {clip, fade, loop, speed, layer, start}")?;
    let mut options = PlayOptions::default();
    if let Some(fade) = opt_f64(value, "fade") {
        options.fade = fade.max(0.0) as f32;
    }
    if let Some(looping) = opt_bool(value, "loop") {
        options.looping = looping;
    }
    if let Some(speed) = opt_f64(value, "speed") {
        options.speed = speed as f32;
    }
    if let Some(layer) = value.get("layer").and_then(Value::as_u64) {
        if layer as usize >= MAX_LAYERS {
            return Err(format!("layer must be below {MAX_LAYERS}"));
        }
        options.layer = layer as usize;
    }
    options.start = opt_f64(value, "start").map(|s| s as f32);
    Ok((clip.to_string(), options))
}

fn paths(value: &Value) -> Result<Vec<PathBuf>, String> {
    match value {
        Value::String(path) => Ok(vec![PathBuf::from(path)]),
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(PathBuf::from)
                    .ok_or_else(|| "library paths must be strings".to_string())
            })
            .collect(),
        _ => Err("expected a path or a list of paths".into()),
    }
}

/// `entity.update {animator}`: add, change or (null) remove the animator that drives
/// `entity` (its own or the nearest ancestor's). Returns notes for the caller.
pub(super) fn patch_animator(
    ctx: &mut RemoteContext<'_, '_>,
    entity: Entity,
    value: &Value,
) -> Result<Vec<String>, String> {
    let playing = ctx.runtime_mode() == Some(RuntimeMode::Playing);
    let world = ctx.gpu.scene.main_world_mut();
    let target = animator_entity(world, entity).unwrap_or(entity);
    if value.is_null() {
        world
            .remove_one::<Animator>(target)
            .map_err(|_| "the entity has no animator".to_string())?;
        return Ok(Vec::new());
    }
    let Some(fields) = value.as_object() else {
        return Err("animator must be an object or null".into());
    };
    let mut animator = world
        .get::<&Animator>(target)
        .map(|a| (*a).clone())
        .unwrap_or_default();
    let mut notes = Vec::new();
    for (key, field) in fields {
        match key.as_str() {
            "libraries" => animator.libraries = paths(field)?,
            "add_clips" => {
                for path in paths(field)? {
                    animator.apply(AnimatorCommand::AddLibrary(path));
                }
            }
            "default_clip" => {
                animator.default_clip = match field {
                    Value::Null => None,
                    Value::String(clip) => Some(clip.clone()),
                    _ => return Err("default_clip must be a clip name or null".into()),
                }
            }
            "speed" => {
                let speed = field.as_f64().ok_or("speed must be a number")?;
                animator.apply(AnimatorCommand::SetSpeed(speed as f32));
            }
            "root_motion" => {
                let mode = field
                    .as_str()
                    .and_then(RootMotion::parse)
                    .ok_or("root_motion is \"keep\", \"in_place\" or \"apply\"")?;
                animator.apply(AnimatorCommand::SetRootMotion(mode));
            }
            "play" => {
                let (clip, options) = play_options(field)?;
                if playing {
                    animator.apply(AnimatorCommand::Play { clip, options });
                } else {
                    // Playback state does not survive entering play mode; what starts
                    // then is the default clip.
                    if options.layer == 0 {
                        animator.default_clip = Some(clip);
                        notes.push(
                            "not playing: set default_clip instead, which starts when play mode begins"
                                .to_string(),
                        );
                    } else {
                        return Err("layered playback needs play mode (runtime_set play)".into());
                    }
                }
            }
            "stop" => {
                let (layer, fade) = match field {
                    Value::Bool(true) => (None, 0.2),
                    Value::Object(_) => (
                        field
                            .get("layer")
                            .and_then(Value::as_u64)
                            .map(|l| l as usize),
                        opt_f64(field, "fade").unwrap_or(0.2) as f32,
                    ),
                    _ => return Err("stop must be true or {layer, fade}".into()),
                };
                animator.apply(AnimatorCommand::Stop { layer, fade });
            }
            "layer" => {
                let layer = field
                    .get("index")
                    .and_then(Value::as_u64)
                    .ok_or("layer needs {index, weight, fade, mask}")?
                    as usize;
                if layer >= MAX_LAYERS {
                    return Err(format!("layer index must be below {MAX_LAYERS}"));
                }
                let mask = match field.get("mask") {
                    None => None,
                    Some(Value::Null) | Some(Value::Bool(false)) => Some(None),
                    Some(Value::String(bone)) => Some(Some(bone.clone())),
                    Some(_) => return Err("mask must be a bone name or null".into()),
                };
                animator.apply(AnimatorCommand::SetLayer {
                    layer,
                    weight: opt_f64(field, "weight").unwrap_or(1.0) as f32,
                    fade: opt_f64(field, "fade").unwrap_or(0.0) as f32,
                    mask,
                });
            }
            other => {
                return Err(format!(
                    "unknown animator field {other:?}; use libraries, add_clips, default_clip, \
                     speed, root_motion, play, stop or layer"
                ))
            }
        }
    }
    world
        .insert_one(target, animator)
        .map_err(|err| err.to_string())?;
    if target != entity {
        notes.push(format!(
            "applied to the animator on ancestor {}",
            entity_id(target)
        ));
    }
    Ok(notes)
}

/// The clips of a glTF/GLB file, as an animator would see them.
pub(super) fn library_json(assets: &mut Assets, path: &Path) -> Result<Value, String> {
    let library = assets.clip_library(path)?;
    Ok(json!({
        "path": path.display().to_string(),
        "clips": library.clips.iter().map(|clip| json!({
            "name": clip.name,
            "duration": round(clip.duration),
            "tracks": clip.tracks.len(),
            "bones": clip.tracks.iter().take(80).map(|t| t.bone.clone()).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
    }))
}

/// `animation.inspect {path}`: list a file's clips without attaching them.
pub(super) fn inspect(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let path = opt_str(params, "path").ok_or("missing parameter \"path\"")?;
    let path = PathBuf::from(path);
    // A fresh read, in case the file changed since it was cached.
    ctx.gpu.scene.assets.invalidate_clip_library(&path);
    library_json(&mut ctx.gpu.scene.assets, &path)
}

/// `character.make {entity, name, camera, script}`: put an imported model under a new
/// character entity with a CharacterController, a generated AnimationTree, a spring
/// camera following it and (with `script`) the engine's third-person controls.
pub(super) fn make_character(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    if ctx.runtime_mode() == Some(RuntimeMode::Playing) {
        return Err("make characters while editing (runtime_set stop first)".into());
    }
    let model = entity_from_json(
        ctx.gpu.scene.main_world(),
        params.get("entity").ok_or("missing parameter \"entity\"")?,
    )?;
    let script = match params.get("script") {
        None | Some(Value::Bool(false)) | Some(Value::Null) => None,
        Some(Value::Bool(true)) => Some(PathBuf::new()),
        Some(Value::String(path)) => Some(PathBuf::from(path)),
        Some(_) => return Err("script is true, false or a project-relative path".into()),
    };
    let options = crate::physics::MakeCharacterOptions {
        name: opt_str(params, "name").map(str::to_owned),
        camera: opt_bool(params, "camera").unwrap_or(true),
        script,
    };
    let made = crate::physics::make_character(&mut ctx.gpu.scene, model, &options)?;
    ctx.mark_modified();
    let world = ctx.gpu.scene.main_world();
    let name_of = |entity: Entity| {
        world
            .get::<&crate::scene::Name>(entity)
            .map(|name| name.0.clone())
            .unwrap_or_default()
    };
    Ok(json!({
        "character": entity_id(made.character),
        "name": made.name,
        "model": {"id": entity_id(made.model), "name": name_of(made.model)},
        "camera": made.camera.map(|c| json!({"id": entity_id(c), "name": name_of(c)})),
        "height": round(made.height),
        "radius": round(made.radius),
        "animation_tree_states": made.tree_states,
        "script": made.script.map(|p| p.display().to_string()),
        "notes": made.notes,
    }))
}

/// `debug.skeleton {enabled}`: draw skinned meshes' bones over everything.
pub(super) fn debug_skeleton(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    if let Some(enabled) = opt_bool(params, "enabled") {
        ctx.gpu.scene.set_show_skeletons(enabled);
    }
    let bones = crate::scene::skeletal::skeleton_segments(ctx.gpu.scene.main_world()).len();
    Ok(json!({
        "enabled": ctx.gpu.scene.show_skeletons(),
        "bones": bones,
        "note": "Bones draw as orange overlays in the viewport and in screenshots.",
    }))
}

/// `debug.physics {enabled}`: draw every simulated collider (play mode), and report what
/// the simulation holds.
pub(super) fn debug_physics(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    if let Some(enabled) = opt_bool(params, "enabled") {
        ctx.gpu.scene.physics_mut().set_debug_draw(enabled);
    }
    let physics = ctx.gpu.scene.physics();
    Ok(json!({
        "enabled": physics.debug_draw(),
        "bodies": physics.body_count(),
        "steps": physics.steps(),
        "gravity": physics.gravity().to_array(),
        "note": "The simulation is built when play starts, so bodies is 0 in edit mode. \
                 Colliders draw as wireframes (green fixed, orange dynamic, blue kinematic, \
                 cyan characters, yellow sensors); the editor always outlines the selected \
                 entity's collider.",
    }))
}
