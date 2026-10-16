//! `light_probes.bake`, `light_probes.status` and `light_probes.debug`: bake the scene's
//! light probe volumes, see how far a bake has come or what the grid shows, and show the
//! probes themselves.

use serde_json::{json, Value};

use super::{RemoteContext, RemoteResult};
use crate::remote::json::{entity_from_json, opt_bool, opt_f64, opt_u64};
use crate::renderer::{LightProbeDebugView, MAX_LIGHT_PROBE_VOLUMES};

/// Defaults: high quality, a few seconds at 60 frames per second for a room.
const WARMUP_SWEEPS: u64 = 32;
const SWEEPS: u64 = 256;
const RAYS: u64 = 256;

pub(super) fn bake(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let warmup = opt_u64(params, "warmup_sweeps")
        .unwrap_or(WARMUP_SWEEPS)
        .clamp(1, 1024) as u32;
    let sweeps = opt_u64(params, "sweeps").unwrap_or(SWEEPS).clamp(1, 4096) as u32;
    let rays = opt_u64(params, "rays_per_probe")
        .unwrap_or(RAYS)
        .clamp(32, 256) as u32;
    let world = ctx.gpu.scene.main_world();
    let volumes = match params.get("entity").filter(|value| !value.is_null()) {
        Some(value) => {
            let entity = entity_from_json(world, value)?;
            let id = entity.to_bits().get();
            if !crate::scene::light_probe_volumes(world)
                .iter()
                .any(|volume| volume.grid.id == id)
            {
                return Err(format!(
                    "entity {id} is not a light probe volume in use (enabled, among the first {MAX_LIGHT_PROBE_VOLUMES})"
                ));
            }
            Some(vec![id])
        }
        None => None,
    };
    let frames =
        ctx.gpu
            .renderer
            .start_light_probe_bake(volumes.as_deref(), warmup, sweeps, rays)?;
    let progress = ctx.gpu.renderer.light_probe_bake_progress();
    Ok(json!({
        "baking": true,
        "volumes": volumes.map_or_else(|| json!("all"), |ids| json!(ids)),
        "queued": progress.map(|progress| progress.queued),
        "warmup_sweeps": progress.map_or(warmup, |progress| progress.warmup_sweeps),
        "sweeps": progress.map_or(sweeps, |progress| progress.sweeps),
        "rays_per_probe": progress.map_or(rays, |progress| progress.rays_per_probe),
        "frames": frames,
        "note": format!(
            "One update per frame, one volume after another: wait about {frames} frames, then \
             light_probes.status (each volume's baked: its saved file, under \
             content/lighting/). Save the project afterwards to keep the volumes' baked paths."
        ),
    }))
}

pub(super) fn status(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let volumes = crate::scene::light_probe_volumes(ctx.gpu.scene.main_world());
    let renderer = &*ctx.gpu.renderer;
    let progress = renderer.light_probe_bake_progress().map(|progress| {
        json!({
            "volume": progress.volume,
            "queued": progress.queued,
            "warmup_sweeps": progress.warmup_sweeps,
            "sweeps": progress.sweeps,
            "rays_per_probe": progress.rays_per_probe,
            "sweeps_done": progress.sweeps_done,
            "finished": progress.finished,
        })
    });
    let volumes: Vec<Value> = volumes
        .iter()
        .map(|volume| {
            let id = volume.grid.id;
            let (irradiance, distance) = renderer
                .light_probe_regions(id)
                .map_or((None, None), |(a, b)| (Some(a), Some(b)));
            json!({
                "id": id,
                "probes": volume.grid.counts,
                "probe_count": volume.grid.probe_count(),
                "spacing": volume.grid.spacing,
                "origin": volume.grid.origin.to_array(),
                "rotation": volume.grid.rotation.to_array(),
                "live": volume.grid.live,
                "relocate": volume.grid.relocate,
                "baked": volume.baked,
                "showing_bake": renderer.light_probe_baked_source(id),
                "irradiance_region": irradiance,
                "distance_region": distance,
            })
        })
        .collect();
    Ok(json!({
        "volumes": volumes,
        "note": "volumes in blending order: where they overlap the first one's light wins",
        "can_trace": renderer.light_probes_can_trace(),
        "bake": progress,
        "last_bake": renderer.light_probe_bake_outcome().map(|outcome| match &outcome.result {
            Ok(path) => json!({ "volume": outcome.volume, "saved": path }),
            Err(error) => json!({ "volume": outcome.volume, "error": error }),
        }),
        "debug": debug_json(renderer.light_probe_debug()),
    }))
}

pub(super) fn debug(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let renderer = &mut *ctx.gpu.renderer;
    let current = renderer.light_probe_debug();
    let show = opt_bool(params, "show").unwrap_or(true);
    let view = show.then(|| {
        let mut view = current.unwrap_or_default();
        if let Some(radius) = opt_f64(params, "radius") {
            view.radius = (radius as f32).clamp(0.005, 1.0);
        }
        if let Some(exposure) = opt_f64(params, "exposure") {
            view.exposure = (exposure as f32).clamp(0.0, 1000.0);
        }
        view
    });
    renderer.set_light_probe_debug(view);
    Ok(json!({ "debug": debug_json(view) }))
}

fn debug_json(view: Option<LightProbeDebugView>) -> Value {
    match view {
        Some(view) => json!({ "show": true, "radius": view.radius, "exposure": view.exposure }),
        None => json!({ "show": false }),
    }
}
