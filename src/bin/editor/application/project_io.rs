use std::path::PathBuf;

use log::{error, info, warn};
use wgpu_cube::app::{GpuUpdateContext, RuntimeMode, UpdateContext};
use wgpu_cube::project::{ProjectError, ProjectManifest};
use wgpu_cube::scene::EntityBuilder;
use wgpu_cube::scene::Transform;

use super::core::{EditorApplication, UndoRedoState};
use crate::history::EditorHistory;
use crate::project::{BuildPlatform, ProjectBuildRequest};

impl EditorApplication {
    pub(super) fn process_pending_imports(&mut self, ctx: &mut UpdateContext) {
        if self.pending_imports.is_empty() {
            return;
        }

        let imports = std::mem::take(&mut self.pending_imports);
        let mut any_spawned = false;
        for path in imports {
            let Some(script_source) = Self::create_import_script(&path) else {
                continue;
            };

            let entity_name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Imported glTF".to_string());

            let mut builder = EntityBuilder::new(ctx.scene.main_world_mut())
                .with_name(format!("{entity_name} (glTF)"))
                .with_transform(Transform::default())
                .with_script(script_source);
            builder.spawn();
            any_spawned = true;
        }

        if matches!(ctx.runtime, RuntimeMode::Editor) {
            ctx.scene.set_animation_playback(false);
            ctx.scene.update(0.0);
        }

        if any_spawned {
            self.record_scene_change(ctx.scene);
        }
    }

    pub(super) fn handle_project_save(&mut self, ctx: &mut GpuUpdateContext, dir: PathBuf) {
        match ProjectManifest::capture(ctx.scene, self.project.metadata().clone()) {
            Ok(manifest) => {
                if let Err(err) = manifest.save_to_dir(&dir) {
                    error!("Failed to save project to {:?}: {err}", dir);
                } else {
                    self.project.set_current_dir(dir);
                }
            }
            Err(ProjectError::EmptyScene) => {
                warn!("Skipping project save: no exportable scene data available");
            }
            Err(err) => {
                error!("Failed to prepare project for saving: {err}");
            }
        }
    }

    pub(super) fn handle_project_load(&mut self, ctx: &mut GpuUpdateContext, dir: PathBuf) {
        match ProjectManifest::load_from_dir(&dir) {
            Ok(manifest) => {
                let metadata = manifest.metadata.clone();
                match manifest.instantiate_into(ctx.scene, ctx.renderer, &dir) {
                    Ok(textures_changed) => {
                        if textures_changed {
                            ctx.renderer.update_texture_bind_group(&ctx.scene.assets);
                        }

                        self.project.set_current_dir(dir);
                        self.project.set_metadata(metadata);
                        self.pending_imports.clear();
                        self.pending_entity_deletions.clear();
                        self.selection.set_selected(None);
                        self.selection.set_highlighted(None);
                        self.selection.clear_pending_pick();
                        self.undo_redo = UndoRedoState::default();
                        self.history = EditorHistory::new();
                        self.initialize_history_state(ctx.scene);
                        self.selection.request_override(None);
                        self.runtime_state.request_mode(RuntimeMode::Editor);
                    }
                    Err(err) => {
                        error!("Failed to instantiate project scene: {err}");
                    }
                }
            }
            Err(err) => {
                error!("Failed to load project from {:?}: {err}", dir);
            }
        }
    }

    pub(super) fn handle_project_build(
        &mut self,
        ctx: &mut GpuUpdateContext,
        request: ProjectBuildRequest,
    ) {
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            let _ = request;
            warn!("Project builds are not supported when running inside the browser editor");
            return;
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            use std::fs;

            if let Err(err) = fs::create_dir_all(&request.output_dir) {
                error!(
                    "Failed to prepare build output directory {:?}: {err}",
                    request.output_dir
                );
                return;
            }

            match ProjectManifest::capture(ctx.scene, self.project.metadata().clone()) {
                Ok(manifest) => {
                    if let Err(err) = manifest.save_to_dir(&request.output_dir) {
                        error!(
                            "Failed to save project manifest to {:?}: {err}",
                            request.output_dir
                        );
                        return;
                    }

                    match request.platform {
                        BuildPlatform::Desktop => {
                            info!(
                                "Saved build manifest for desktop target at {:?}",
                                request.output_dir
                            );
                            info!("Desktop build command execution is not yet implemented");
                        }
                        BuildPlatform::Web => {
                            info!(
                                "Saved build manifest for web target at {:?}",
                                request.output_dir
                            );
                            info!("Web build command execution is not yet implemented");
                        }
                    }
                }
                Err(ProjectError::EmptyScene) => {
                    warn!(
                        "Skipping project build: no exportable scene data available for {:?}",
                        request.platform
                    );
                }
                Err(err) => {
                    error!("Failed to prepare project for building: {err}");
                }
            }
        }
    }
}
