//! Read-only queries: engine state, entity listings, assets and logs.

use hecs::Entity;
use serde_json::{json, Value};

use crate::scene::{Children, Name, Parent, WorldTransform};

use super::super::json::{entity_id, opt_bool, opt_str, opt_u64, round, vec3_json};
use super::super::log_tap;
use super::entity::{component_names, has_component, COMPONENT_NAMES};
use super::{RemoteContext, RemoteResult};

pub(super) fn engine_info(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let size = ctx.gpu.renderer.surface_size();
    let mode = ctx.runtime_mode().map(runtime_mode_name);
    let scene = &ctx.gpu.scene;
    let entity_count = scene.main_world().len();
    Ok(json!({
        "frame": ctx.hub.frame(),
        "uptime_seconds": round(ctx.hub.uptime() as f32),
        "fps": fps(ctx.hub.last_dt()),
        "runtime_mode": mode,
        "scene": {
            "document_id": scene.document_id(),
            "entity_count": entity_count,
            "active_camera": scene.active_camera_entity().map(entity_id),
            "time_seconds": round(scene.time() as f32),
        },
        "surface": {
            "width": size.width,
            "height": size.height,
            "format": format!("{:?}", ctx.gpu.renderer.surface_format()),
            "hdr_output": ctx.gpu.renderer.hdr_display().mode.label(),
            "sample_count": ctx.gpu.renderer.sample_count(),
            "ray_tracing": ctx.gpu.renderer.ray_tracing_support().to_string(),
        },
        "project_root": crate::project::active_project_root().map(|path| path.display().to_string()),
        "working_directory": std::env::current_dir().ok().map(|path| path.display().to_string()),
        "remote_port": ctx.hub.port(),
        "limits": {
            "lights": {
                "directional": crate::renderer::lights::MAX_DIRECTIONAL_LIGHTS,
                "point": crate::renderer::lights::MAX_POINT_LIGHTS,
                "spot": crate::renderer::lights::MAX_SPOT_LIGHTS,
                "selection": "First lights in scene/world iteration order; no distance or influence ranking. Limit is shared across scene instances.",
            },
            "geometry": {
                "compose_parts": crate::procgen::geometry::MAX_COMPOSE_PARTS,
                "vertices": crate::procgen::geometry::MAX_VERTICES,
                "indices": crate::procgen::geometry::MAX_VERTICES * 6,
            },
        },
        "conventions": {
            "units": "meters; Y is up, -Z is forward, right-handed",
            "rotation": "Euler degrees [x, y, z], applied yaw(Y) then pitch(X) then roll(Z)",
            "colors": "[r, g, b(, a)] in 0..1 or \"#rrggbb\"",
            "entities": "64-bit ids (same as Lua) or unique names",
        },
    }))
}

pub(super) fn runtime_mode_name(mode: crate::app::RuntimeMode) -> &'static str {
    match mode {
        crate::app::RuntimeMode::Editor => "editor",
        crate::app::RuntimeMode::Playing => "playing",
    }
}

fn fps(dt: f64) -> Value {
    if dt > 0.0 {
        json!((1.0 / dt * 10.0).round() / 10.0)
    } else {
        Value::Null
    }
}

pub(super) fn stats(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let stats = ctx.gpu.renderer.last_frame_stats();
    let dt = ctx.hub.last_dt();
    let scene = &ctx.gpu.scene;
    let stages = crate::app::core::frame_timings::snapshot();
    let accounted: f64 = stages.iter().map(|(_, _, average)| average).sum();
    let breakdown: serde_json::Map<String, Value> = stages
        .iter()
        .map(|(stage, last, average)| {
            (
                (*stage).to_owned(),
                json!({ "last_ms": round(*last as f32), "avg_ms": round(*average as f32) }),
            )
        })
        .collect();
    Ok(json!({
        "frame": ctx.hub.frame(),
        "frame_time_ms": round((dt * 1000.0) as f32),
        "fps": fps(dt),
        "frame_breakdown": breakdown,
        "script_timings": scene.lua_scripting().script_timings(),
        "frame_breakdown_note": format!(
            "Time per stage of the engine's frame loop (avg is a running average). The stages sum to {:.1} ms; the rest of frame_time_ms is waiting for the display (vsync) and window events. scripts = Lua update/on_created; render_collect = gathering visible meshes; render_gpu = batching, uploads and recording/submitting GPU passes; present may block on vsync; ui_build/ui_render = egui; gpu_systems = editor systems, particles and remote commands. Measure game performance in the player: the editor's UI and systems add their own cost.",
            accounted
        ),
        "entities": scene.main_world().len(),
        "scripts": scene.lua_scripting().script_entities().len(),
        "renderer": {
            "batches": stats.batch_count,
            "instances": stats.instance_count,
            "draw_calls": stats.total_draw_calls(),
            "opaque_draw_calls": stats.opaque_draw_calls,
            "transparent_draw_calls": stats.transparent_draw_calls,
            "shadow_draw_calls": stats.shadow_draw_calls,
            "ray_tracing": stats.ray_tracing.map(|rt| json!({
                "tlas_instances": rt.instances,
                "meshes": rt.meshes,
                "blas_builds": rt.blas_builds,
                "pool_vertices": rt.pool_vertices,
            })),
        },
        "assets": {
            "meshes": scene.assets.meshes.len(),
            "materials": scene.assets.materials.live_count(),
            "textures": scene.assets.textures.len(),
        },
    }))
}

/// One-line description of an entity used by listings.
pub(super) fn entity_summary(world: &hecs::World, entity: Entity) -> Value {
    let name = world.get::<&Name>(entity).ok().map(|name| name.0.clone());
    let parent = world
        .get::<&Parent>(entity)
        .ok()
        .map(|parent| entity_id(parent.0));
    let children = world
        .get::<&Children>(entity)
        .map(|children| children.0.len())
        .unwrap_or(0);
    let position = world
        .get::<&WorldTransform>(entity)
        .ok()
        .map(|transform| vec3_json(transform.0.translation));
    json!({
        "id": entity_id(entity),
        "name": name,
        "parent": parent,
        "children": children,
        "components": component_names(world, entity),
        "world_position": position,
    })
}

pub(super) fn scene_entities(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let name_filter = opt_str(params, "name_contains").map(str::to_lowercase);
    let component = opt_str(params, "component");
    if let Some(component) = component {
        if !COMPONENT_NAMES.contains(&component) {
            return Err(format!(
                "unknown component {component:?}; known: {}",
                COMPONENT_NAMES.join(", ")
            ));
        }
    }
    let roots_only = opt_bool(params, "roots_only").unwrap_or(false);
    let limit = opt_u64(params, "limit").unwrap_or(200) as usize;
    let offset = opt_u64(params, "offset").unwrap_or(0) as usize;

    let world = ctx.gpu.scene.main_world();
    let mut entities: Vec<Entity> = world.iter().map(|entity_ref| entity_ref.entity()).collect();
    entities.sort_unstable_by_key(|entity| entity.to_bits());

    let matching: Vec<Entity> = entities
        .into_iter()
        .filter(|entity| {
            if roots_only && world.get::<&Parent>(*entity).is_ok() {
                return false;
            }
            if let Some(filter) = &name_filter {
                let matches = world
                    .get::<&Name>(*entity)
                    .is_ok_and(|name| name.0.to_lowercase().contains(filter));
                if !matches {
                    return false;
                }
            }
            component.is_none_or(|component| has_component(world, *entity, component))
        })
        .collect();

    let total = matching.len();
    let listed: Vec<Value> = matching
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|entity| entity_summary(world, entity))
        .collect();
    Ok(json!({ "total": total, "offset": offset, "entities": listed }))
}

pub(super) fn scene_tree(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let max_depth = opt_u64(params, "max_depth").unwrap_or(32) as usize;
    let world = ctx.gpu.scene.main_world();

    let mut roots: Vec<Entity> = world
        .iter()
        .map(|entity_ref| entity_ref.entity())
        .filter(|entity| world.get::<&Parent>(*entity).is_err())
        .collect();
    roots.sort_unstable_by_key(|entity| entity.to_bits());

    fn node(world: &hecs::World, entity: Entity, depth: usize, max_depth: usize) -> Value {
        let name = world.get::<&Name>(entity).ok().map(|name| name.0.clone());
        let children: Vec<Entity> = world
            .get::<&Children>(entity)
            .map(|children| children.0.clone())
            .unwrap_or_default();
        let mut value = json!({
            "id": entity_id(entity),
            "name": name,
            "components": component_names(world, entity),
        });
        if !children.is_empty() {
            value["children"] = if depth >= max_depth {
                json!(format!("{} children (max_depth reached)", children.len()))
            } else {
                Value::Array(
                    children
                        .into_iter()
                        .filter(|child| world.contains(*child))
                        .map(|child| node(world, child, depth + 1, max_depth))
                        .collect(),
                )
            };
        }
        value
    }

    let tree: Vec<Value> = roots
        .into_iter()
        .map(|root| node(world, root, 0, max_depth))
        .collect();
    Ok(json!({ "entity_count": world.len(), "roots": tree }))
}

pub(super) fn assets_list(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let kind = opt_str(params, "kind").unwrap_or("materials");
    let assets = &ctx.gpu.scene.assets;
    match kind {
        "materials" => {
            let materials: Vec<Value> = assets
                .materials
                .iter()
                .map(|(handle, asset)| {
                    let path = asset.canonical_path();
                    json!({
                        "handle": handle.index(),
                        "path": (!path.as_os_str().is_empty()).then(|| path.display().to_string()),
                        "material": super::entity::material_json(asset),
                    })
                })
                .collect();
            Ok(json!({ "kind": kind, "count": materials.len(), "materials": materials }))
        }
        "meshes" => Ok(json!({ "kind": kind, "count": assets.meshes.len() })),
        "textures" => Ok(json!({ "kind": kind, "count": assets.textures.len() })),
        other => Err(format!(
            "unknown asset kind {other:?}; use materials, meshes or textures"
        )),
    }
}

pub(super) fn logs(params: &Value) -> RemoteResult {
    let mut after = opt_u64(params, "after").unwrap_or(0);
    if opt_bool(params, "since_play_start").unwrap_or(false) {
        after = after.max(log_tap::play_start_seq());
    }
    let min_level = match opt_str(params, "min_level").unwrap_or("info") {
        "error" => log::Level::Error,
        "warn" | "warning" => log::Level::Warn,
        "info" => log::Level::Info,
        "debug" => log::Level::Debug,
        "trace" => log::Level::Trace,
        other => return Err(format!("unknown log level {other:?}")),
    };
    let contains = opt_str(params, "contains").map(str::to_lowercase);
    let target = opt_str(params, "target");
    let limit = opt_u64(params, "limit").unwrap_or(100).clamp(1, 2000) as usize;

    let lines = log_tap::entries_since(after, min_level, |line| {
        target.is_none_or(|target| line.target.starts_with(target))
            && contains
                .as_ref()
                .is_none_or(|text| line.message.to_lowercase().contains(text))
    });
    let truncated = lines.len() > limit;
    let entries: Vec<Value> = lines
        .iter()
        .skip(lines.len().saturating_sub(limit))
        .map(|line| {
            json!({
                "seq": line.seq,
                "timestamp_ms": line.timestamp_ms,
                "level": line.level.as_str(),
                "target": line.target,
                "message": line.message,
            })
        })
        .collect();
    Ok(json!({
        "latest_seq": log_tap::latest_seq(),
        "play_start_seq": log_tap::play_start_seq(),
        "truncated": truncated,
        "entries": entries,
    }))
}
