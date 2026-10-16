//! `light_probes.bake` and `light_probes.status`: bake the scene's light probe volume and
//! see how far a bake has come, or what the grid shows.

use serde_json::{json, Value};

use super::{RemoteContext, RemoteResult};
use crate::remote::json::opt_u64;

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
    let frames = ctx
        .gpu
        .renderer
        .start_light_probe_bake(warmup, sweeps, rays)?;
    Ok(json!({
        "baking": true,
        "warmup_sweeps": warmup,
        "sweeps": sweeps,
        "rays_per_probe": rays,
        "frames": frames,
        "note": format!(
            "One update per frame: wait about {frames} frames, then light_probes.status \
             (volume.baked: the saved file, under content/lighting/). Save the project \
             afterwards to keep the volume's baked path."
        ),
    }))
}

pub(super) fn status(ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let volume = crate::scene::active_light_probe_volume(ctx.gpu.scene.main_world());
    let renderer = &*ctx.gpu.renderer;
    let progress = renderer.light_probe_bake_progress().map(|progress| {
        json!({
            "warmup_sweeps": progress.warmup_sweeps,
            "sweeps": progress.sweeps,
            "rays_per_probe": progress.rays_per_probe,
            "sweeps_done": progress.sweeps_done,
            "finished": progress.finished,
        })
    });
    Ok(json!({
        "volume": volume.as_ref().map(|volume| json!({
            "id": volume.entity.to_bits().get(),
            "probes": volume.grid.counts,
            "probe_count": volume.grid.probe_count(),
            "spacing": volume.grid.spacing,
            "origin": volume.grid.origin.to_array(),
            "live": volume.grid.live,
            "baked": volume.baked,
        })),
        "can_trace": renderer.light_probes_can_trace(),
        "showing_bake": renderer.light_probe_baked_source(),
        "bake": progress,
    }))
}
