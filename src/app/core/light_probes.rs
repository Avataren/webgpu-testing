//! Per frame: hand the scene's light probe volumes to the renderer, load their baked data
//! where the renderer does not show it yet, and save a finished bake into the project.

use crate::renderer::LightProbeGrid;
use crate::renderer::Renderer;
use crate::scene::Scene;
use std::collections::HashMap;

/// Where bakes are saved, relative to the project.
#[cfg(not(target_arch = "wasm32"))]
const BAKE_DIR: &str = "content/lighting";

/// Set the renderer's probe volumes from `scene`, load each volume's baked data when the
/// renderer does not show it yet, and save a bake that finished. `failed_loads` holds the
/// files that failed to load and the grid they failed for: they are read again only once
/// that volume's grid changes (moved back to where the bake fits, say).
pub(super) fn sync(
    renderer: &mut Renderer,
    scene: &mut Scene,
    failed_loads: &mut HashMap<String, LightProbeGrid>,
) {
    let volumes = crate::scene::light_probe_volumes(scene.main_world());
    // With ray tracing off in the graphics settings the probes show their bakes.
    let trace = crate::graphics_settings::graphics_settings()
        .settings
        .ray_tracing;
    let grids: Vec<_> = volumes
        .iter()
        .map(|volume| LightProbeGrid {
            live: volume.grid.live && trace,
            ..volume.grid
        })
        .collect();
    renderer.set_light_probe_grids(&grids);

    for volume in &volumes {
        let Some(path) = volume.baked.as_deref() else {
            continue;
        };
        if renderer.light_probe_baked_source(volume.grid.id) == Some(path)
            || failed_loads.get(path) == Some(&volume.grid)
        {
            continue;
        }
        let loaded = crate::io::load_binary(&crate::project::resolve_project_path(path))
            .and_then(|bytes| renderer.load_light_probe_bake(volume.grid.id, &bytes, path));
        match loaded {
            Ok(()) => {
                log::info!("Light probes: showing the bake in {path}");
                failed_loads.remove(path);
            }
            Err(error) => {
                log::warn!("Light probes: {error}");
                failed_loads.insert(path.to_owned(), volume.grid);
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    if scene.day_bake().is_some() {
        step_day_bake(renderer, scene, &volumes, failed_loads);
        return;
    }

    #[cfg(not(target_arch = "wasm32"))]
    if let Some((id, result)) = renderer.take_finished_light_probe_bake() {
        let volume = volumes.iter().find(|volume| volume.grid.id == id);
        let outcome = match (result, volume) {
            (Ok(bytes), Some(volume)) => match save_bake(scene, volume.entity, &bytes) {
                Ok(path) => {
                    log::info!(
                        "Light probes: baked {} KB into {path}",
                        bytes.len().div_ceil(1024)
                    );
                    if let Err(error) = renderer.load_light_probe_bake(id, &bytes, &path) {
                        log::warn!("Light probes: {error}");
                    }
                    failed_loads.remove(&path);
                    Ok(path)
                }
                Err(error) => Err(format!("saving the bake failed: {error}")),
            },
            (Ok(_), None) => Err("the volume went away during the bake".to_owned()),
            (Err(error), _) => Err(format!("the bake failed: {error}")),
        };
        if let Err(error) = &outcome {
            log::error!("Light probes: {error}");
        }
        renderer.set_light_probe_bake_outcome(id, outcome);
    }
}

/// One frame of a bake at several hours of the day (`crate::sky::DayBake`): let the sky
/// settle at the hour, bake the volumes there, keep each volume's bake, and on to the
/// next hour; after the last, save each volume's bakes as one file whose light follows
/// the time of day, and show it.
#[cfg(not(target_arch = "wasm32"))]
fn step_day_bake(
    renderer: &mut Renderer,
    scene: &mut Scene,
    volumes: &[crate::scene::ActiveLightProbeVolume],
    failed_loads: &mut HashMap<String, LightProbeGrid>,
) {
    let Some(mut bake) = scene.take_day_bake() else {
        return;
    };
    let fail = |renderer: &mut Renderer, scene: &mut Scene, volume: u64, error: String| {
        log::error!("Light probes: the bake of the day's hours stopped: {error}");
        renderer.set_light_probe_bake_outcome(volume, Err(error));
        scene.end_day_bake();
    };
    if let Some((id, result)) = renderer.take_finished_light_probe_bake() {
        match result {
            Ok(bytes) => {
                let hour = bake.hour().unwrap_or_default();
                bake.results.push((id, hour, bytes));
            }
            Err(error) => return fail(renderer, scene, id, error),
        }
    }
    if bake.settle > 0 {
        bake.settle -= 1;
        if bake.settle == 0 {
            if let Err(error) = renderer.start_light_probe_bake(
                bake.volumes.as_deref(),
                bake.warmup,
                bake.sweeps,
                bake.rays,
            ) {
                return fail(renderer, scene, 0, error);
            }
            log::info!(
                "Light probes: baking the day's hour {} of {} ({:.2} h)",
                bake.current + 1,
                bake.hours.len(),
                bake.hour().unwrap_or_default()
            );
        }
        scene.put_day_bake(bake);
        return;
    }
    if renderer.light_probe_bake_progress().is_some() {
        scene.put_day_bake(bake);
        return;
    }
    // Every volume baked this hour.
    bake.current += 1;
    if bake.current < bake.hours.len() {
        bake.settle = crate::sky::DayBake::SETTLE_FRAMES;
        scene.put_day_bake(bake);
        return;
    }
    scene.end_day_bake();
    let mut ids: Vec<u64> = bake.results.iter().map(|(id, _, _)| *id).collect();
    ids.sort_unstable();
    ids.dedup();
    for id in ids {
        let outcome = save_day_bake(renderer, scene, volumes, &bake, id, failed_loads);
        if let Err(error) = &outcome {
            log::error!("Light probes: {error}");
        }
        renderer.set_light_probe_bake_outcome(id, outcome);
    }
}

/// Save volume `id`'s bakes of the day's hours as one keyframed file, point the volume at
/// it and show it. Returns the project path.
#[cfg(not(target_arch = "wasm32"))]
fn save_day_bake(
    renderer: &mut Renderer,
    scene: &mut Scene,
    volumes: &[crate::scene::ActiveLightProbeVolume],
    bake: &crate::sky::DayBake,
    id: u64,
    failed_loads: &mut HashMap<String, LightProbeGrid>,
) -> Result<String, String> {
    use crate::renderer::light_probes::{decode_bake, encode_keyframed_bake};
    let volume = volumes
        .iter()
        .find(|volume| volume.grid.id == id)
        .ok_or("the volume went away during the bake")?;
    let decoded: Vec<(f32, _)> = bake
        .results
        .iter()
        .filter(|(volume, _, _)| *volume == id)
        .map(|(_, hour, bytes)| decode_bake(bytes).map(|decoded| (*hour, decoded)))
        .collect::<Result<_, _>>()?;
    // The light shown without a sky: the hour nearest noon.
    let base = decoded
        .iter()
        .min_by(|a, b| (a.0 - 12.0).abs().total_cmp(&(b.0 - 12.0).abs()))
        .ok_or("nothing was baked")?;
    let keyframes: Vec<(f32, _)> = decoded.iter().map(|(hour, bake)| (*hour, bake)).collect();
    let bytes = encode_keyframed_bake(&base.1, &keyframes)?;
    let path = save_bake(scene, volume.entity, &bytes)
        .map_err(|error| format!("saving the bake failed: {error}"))?;
    log::info!(
        "Light probes: baked {} hours of the day, {} KB, into {path}",
        keyframes.len(),
        bytes.len().div_ceil(1024)
    );
    renderer.load_light_probe_bake(id, &bytes, &path)?;
    failed_loads.remove(&path);
    Ok(path)
}

/// Write `bytes` to `content/lighting/<volume name>.probes` in the project (with the
/// entity's id added when another volume has the same name) and point the volume at it.
/// Returns the project path.
#[cfg(not(target_arch = "wasm32"))]
fn save_bake(scene: &mut Scene, entity: hecs::Entity, bytes: &[u8]) -> Result<String, String> {
    let root =
        crate::project::active_project_root().ok_or("no project is open to save the bake into")?;
    let world = scene.main_world_mut();
    let stem_of = |entity: hecs::Entity| {
        file_stem(
            &world
                .get::<&crate::scene::Name>(entity)
                .map(|name| name.0.clone())
                .unwrap_or_else(|_| "light_probes".into()),
        )
    };
    let mut stem = stem_of(entity);
    let shared = world
        .query::<&crate::scene::LightProbeVolume>()
        .iter()
        .any(|(other, _)| other != entity && stem_of(other) == stem);
    if shared {
        stem = format!("{stem}_{}", entity.id());
    }
    let path = format!("{BAKE_DIR}/{stem}.probes");
    let file = root.join(&path);
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(|err| format!("{}: {err}", dir.display()))?;
    }
    std::fs::write(&file, bytes).map_err(|err| format!("{}: {err}", file.display()))?;
    let mut volume = world
        .get::<&mut crate::scene::LightProbeVolume>(entity)
        .map_err(|_| "the volume entity has no light probe volume any more")?;
    volume.baked = Some(path.clone());
    Ok(path)
}

/// `name` as a file name: letters, digits, '-' and '_' kept, the rest '_'.
#[cfg(not(target_arch = "wasm32"))]
fn file_stem(name: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if stem.trim_matches('_').is_empty() {
        "light_probes".into()
    } else {
        stem
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #[test]
    fn bake_files_are_named_after_the_volume() {
        assert_eq!(super::file_stem("Light Probes"), "light_probes");
        assert_eq!(super::file_stem("Hall/East 2"), "hall_east_2");
        assert_eq!(super::file_stem("   "), "light_probes");
    }
}
