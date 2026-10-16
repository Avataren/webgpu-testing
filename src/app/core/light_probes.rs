//! Per frame: hand the scene's light probe volume to the renderer, load its baked data
//! where the probes need it, and save a finished bake into the project.

use crate::renderer::Renderer;
use crate::scene::Scene;

/// Where bakes are saved, relative to the project.
#[cfg(not(target_arch = "wasm32"))]
const BAKE_DIR: &str = "content/lighting";

/// Set the renderer's probe grid from `scene`, load the volume's baked data when the
/// renderer does not show it yet, and save a bake that finished. `failed_load` remembers
/// the file that last failed to load, so it is not read again every frame.
pub(super) fn sync(renderer: &mut Renderer, scene: &mut Scene, failed_load: &mut Option<String>) {
    let volume = crate::scene::active_light_probe_volume(scene.main_world());
    renderer.set_light_probe_grid(volume.as_ref().map(|volume| volume.grid));

    if let Some(path) = volume.as_ref().and_then(|volume| volume.baked.as_deref()) {
        if renderer.light_probe_baked_source() != Some(path) && failed_load.as_deref() != Some(path)
        {
            let loaded = crate::io::load_binary(&crate::project::resolve_project_path(path))
                .and_then(|bytes| renderer.load_light_probe_bake(&bytes, path));
            match loaded {
                Ok(()) => {
                    log::info!("Light probes: showing the bake in {path}");
                    *failed_load = None;
                }
                Err(error) => {
                    log::warn!("Light probes: {error}");
                    *failed_load = Some(path.to_owned());
                }
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    if let Some(result) = renderer.take_finished_light_probe_bake() {
        let outcome = match (result, volume) {
            (Ok(bytes), Some(volume)) => match save_bake(scene, volume.entity, &bytes) {
                Ok(path) => {
                    log::info!(
                        "Light probes: baked {} KB into {path}",
                        bytes.len().div_ceil(1024)
                    );
                    if let Err(error) = renderer.load_light_probe_bake(&bytes, &path) {
                        log::warn!("Light probes: {error}");
                    }
                    *failed_load = None;
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
        renderer.set_light_probe_bake_outcome(outcome);
    }
}

/// Write `bytes` to `content/lighting/<volume name>.probes` in the project and point the
/// volume at it. Returns the project path.
#[cfg(not(target_arch = "wasm32"))]
fn save_bake(scene: &mut Scene, entity: hecs::Entity, bytes: &[u8]) -> Result<String, String> {
    let root =
        crate::project::active_project_root().ok_or("no project is open to save the bake into")?;
    let world = scene.main_world_mut();
    let name = world
        .get::<&crate::scene::Name>(entity)
        .map(|name| name.0.clone())
        .unwrap_or_else(|_| "light_probes".into());
    let path = format!("{BAKE_DIR}/{}.probes", file_stem(&name));
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
