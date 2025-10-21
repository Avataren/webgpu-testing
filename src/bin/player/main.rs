#[cfg(target_arch = "wasm32")]
use std::path::Path;
use std::path::PathBuf;

use wgpu_cube::app::{StartupContext, UpdateContext};
use wgpu_cube::project::ProjectManifest;

#[cfg(target_arch = "wasm32")]
use wgpu_cube::project::PROJECT_FILE_NAME;
use wgpu_cube::render_application::{run_application, RenderApplication};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

struct ProjectPlayer {
    manifest: ProjectManifest,
    project_root: PathBuf,
    loaded: bool,
}

impl ProjectPlayer {
    fn new(manifest: ProjectManifest, project_root: PathBuf) -> Self {
        Self {
            manifest,
            project_root,
            loaded: false,
        }
    }
}

impl RenderApplication for ProjectPlayer {
    fn name(&self) -> &str {
        &self.manifest.metadata.name
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        if self.loaded {
            return;
        }

        match self
            .manifest
            .instantiate_into(ctx.scene, ctx.renderer, &self.project_root)
        {
            Ok(textures_changed) => {
                if textures_changed {
                    ctx.renderer.update_texture_bind_group(&ctx.scene.assets);
                }

                log::info!(
                    "Loaded project '{}' from {}",
                    self.manifest.metadata.name,
                    self.project_root.display()
                );
                self.loaded = true;
            }
            Err(err) => {
                log::error!("Failed to instantiate project: {err}");
            }
        }
    }

    fn update(&mut self, _ctx: &mut UpdateContext) {}
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let project_dir = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or(std::env::current_dir()?);

    let manifest = ProjectManifest::load_from_dir(&project_dir)?;
    run_application(ProjectPlayer::new(manifest, project_dir))?;
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    if let Err(err) = start_app_internal() {
        log::error!("Failed to start project: {:?}", err);
    }
}

#[cfg(target_arch = "wasm32")]
fn start_app_internal() -> Result<(), JsValue> {
    let bytes = wgpu_cube::io::load_binary_from_str(PROJECT_FILE_NAME)
        .map_err(|err| JsValue::from_str(&format!("Failed to load manifest: {err}")))?;

    let mut manifest = ProjectManifest::from_json_bytes(&bytes)
        .map_err(|err| JsValue::from_str(&err.to_string()))?;
    manifest
        .environment
        .resolve_paths(Path::new("."))
        .map_err(|err| JsValue::from_str(&err.to_string()))?;

    run_application(ProjectPlayer::new(manifest, PathBuf::from(".")))
}
