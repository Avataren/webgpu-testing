//! Navigation over remote control: bake navmeshes, draw them, test paths and watch
//! agents. The components themselves go through `entity.get` / `entity.update`
//! (`nav_mesh_surface`, `nav_agent`, `nav_obstacle`, `nav_modifier`).

use glam::Vec3;
use hecs::Entity;
use serde_json::{json, Value};

use crate::navigation::{NavDebugDraw, QueryFilter, SceneNavigation};

use super::super::json::{
    entity_from_json, entity_id, opt_bool, opt_f64, round, vec3_from_json, vec3_json,
};
use super::{RemoteContext, RemoteResult};

fn name_of(world: &hecs::World, entity: Entity) -> String {
    world
        .get::<&crate::scene::Name>(entity)
        .map(|name| name.0.clone())
        .unwrap_or_default()
}

fn status_json(navigation: &SceneNavigation, world: &hecs::World) -> Value {
    let surfaces: Vec<Value> = navigation
        .surface_statuses(world)
        .into_iter()
        .map(|s| {
            json!({
                "entity": entity_id(s.entity),
                "name": s.name,
                "tiles": s.tiles,
                "polys": s.polys,
                "source": s.source,
                "baked": s.baked,
                "stale": s.stale,
                "error": s.error,
                "agents": s.agents,
                "build_ms": s.build_millis.map(|ms| round(ms as f32)),
            })
        })
        .collect();
    let (obstacles, carving) = navigation.obstacle_count();
    let bake = navigation.bake_progress().map(|p| {
        json!({
            "surface": entity_id(p.surface),
            "tiles_done": p.tiles_done,
            "tiles_total": p.tiles_total,
            "seconds": round(p.seconds),
        })
    });
    let last_bake = navigation.last_bake().map(|outcome| {
        json!({
            "surface": entity_id(outcome.surface),
            "saved": outcome.result.as_ref().ok(),
            "error": outcome.result.as_ref().err(),
            "tiles": outcome.stats.as_ref().map(|s| s.tiles),
            "polys": outcome.stats.as_ref().map(|s| s.polys),
            "triangles": outcome.stats.as_ref().map(|s| s.triangles),
            "ms": outcome.stats.as_ref().map(|s| round(s.millis as f32)),
        })
    });
    json!({
        "surfaces": surfaces,
        "agents": navigation.agent_count(),
        "obstacles": obstacles,
        "carving": carving,
        "debug": serde_json::to_value(navigation.debug_draw()).unwrap_or(Value::Null),
        "bake": bake,
        "last_bake": last_bake,
        "test_path": navigation.test_path().map(|(a, b)| json!([vec3_json(a), vec3_json(b)])),
    })
}

/// `navigation.status`: navmeshes, agents, obstacles, bakes.
pub(super) fn status(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let scene = &ctx.gpu.scene;
    Ok(status_json(scene.navigation(), scene.main_world()))
}

/// `navigation.bake {entity}`: bake a surface (or the first) in the background.
pub(super) fn bake(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let surface = match params.get("entity") {
        None | Some(Value::Null) => None,
        Some(value) => Some(entity_from_json(ctx.gpu.scene.main_world(), value)?),
    };
    let entity = ctx.gpu.scene.start_navmesh_bake(surface)?;
    Ok(json!({
        "started": entity_id(entity),
        "note": "The bake runs in the background; navigation_status shows its progress and, once done, last_bake (saved into content/navigation/ when a project is open; project_save keeps the surface's baked path).",
    }))
}

/// `navigation.debug {mesh, agents, paths, obstacles, x_ray, all}`.
pub(super) fn debug(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let navigation = ctx.gpu.scene.navigation_mut();
    let mut debug = navigation.debug_draw();
    if let Some(all) = opt_bool(params, "all") {
        debug = if all {
            NavDebugDraw::all()
        } else {
            NavDebugDraw::default()
        };
    }
    if let Some(v) = opt_bool(params, "mesh") {
        debug.mesh = v;
    }
    if let Some(v) = opt_bool(params, "agents") {
        debug.agents = v;
    }
    if let Some(v) = opt_bool(params, "paths") {
        debug.paths = v;
    }
    if let Some(v) = opt_bool(params, "obstacles") {
        debug.obstacles = v;
    }
    if let Some(v) = opt_bool(params, "x_ray") {
        debug.x_ray = v;
    }
    navigation.set_debug_draw(debug);
    Ok(serde_json::to_value(debug).unwrap_or(Value::Null))
}

/// `navigation.path {from, to, show}`: a path between two points.
pub(super) fn path(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    if opt_bool(params, "clear") == Some(true) {
        ctx.gpu.scene.navigation_mut().set_test_path(None);
        return Ok(json!({ "cleared": true }));
    }
    let world = ctx.gpu.scene.main_world();
    let point = |key: &str| -> Result<Vec3, String> {
        let value = params
            .get(key)
            .ok_or(format!("missing parameter {key:?}"))?;
        // An entity stands for where it is.
        if value.is_array() {
            vec3_from_json(value)
        } else {
            let entity = entity_from_json(world, value)?;
            Ok(crate::physics::world_transform(world, entity).translation)
        }
    };
    let from = point("from")?;
    let to = point("to")?;
    let navigation = ctx.gpu.scene.navigation();
    let found = navigation.find_path(from, to, "", &QueryFilter::default());
    if opt_bool(params, "show").unwrap_or(true) {
        ctx.gpu
            .scene
            .navigation_mut()
            .set_test_path(Some((from, to)));
    }
    match found {
        Some((points, complete, length)) => Ok(json!({
            "complete": complete,
            "length": round(length),
            "corners": points.len(),
            "points": points.into_iter().map(vec3_json).collect::<Vec<_>>(),
        })),
        None => Err(
            "no path: one of the points is not near the navmesh (or there is no navmesh; bake one)"
                .into(),
        ),
    }
}

/// `navigation.closest {point, radius}`: the nearest navmesh point.
pub(super) fn closest(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let point = vec3_from_json(params.get("point").ok_or("missing parameter \"point\"")?)?;
    let r = opt_f64(params, "radius").unwrap_or(2.0).max(0.01) as f32;
    let found = ctx
        .gpu
        .scene
        .navigation()
        .closest_point(point, Vec3::new(r, r.max(2.0), r), "");
    Ok(json!({ "point": found.map(vec3_json) }))
}

/// `navigation.agents`: every agent's state.
pub(super) fn agents(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let scene = &ctx.gpu.scene;
    let world = scene.main_world();
    let navigation = scene.navigation();
    let mut list = Vec::new();
    for (entity, _) in world.query::<&crate::navigation::NavAgent>().iter() {
        let status = navigation.agent_status(entity);
        list.push(json!({
            "entity": entity_id(entity),
            "name": name_of(world, entity),
            "state": status.as_ref().map(|s| s.state.name()),
            "position": status.as_ref().map(|s| vec3_json(s.position)),
            "speed": status.as_ref().map(|s| round(Vec3::new(s.velocity.x, 0.0, s.velocity.z).length())),
            "target": status.as_ref().and_then(|s| s.target).map(vec3_json),
            "remaining": status.as_ref().and_then(|s| s.remaining_distance).map(round),
            "partial": status.as_ref().map(|s| s.partial),
            "steering": status.as_ref().map(|s| s.steering),
        }));
    }
    Ok(json!({
        "agents": list,
        "note": "Agents walk only in play mode; while editing state is null.",
    }))
}
