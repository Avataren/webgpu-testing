use std::collections::VecDeque;

use hecs::Entity;
use log::warn;

use super::core::PendingShaderAction;
use super::system::{EditorAppAccess, EditorCommand, EditorContext, EditorSystem};
use crate::shader_editor::{ShaderEditorEvent, ShaderEditorState};
use wgpu_cube::asset::{Handle, MaterialAsset, MaterialKind, ShaderMaterialMetadata};
use wgpu_cube::renderer::Renderer;
use wgpu_cube::scene::{Scene, SceneWorkspaceSceneMut};

#[derive(Default)]
pub(crate) struct ShaderEditorSystem {
    editor: Option<ShaderEditorState>,
    window_enabled: bool,
}

impl ShaderEditorSystem {
    pub(crate) fn open_shader_editor(
        &mut self,
        entity: Entity,
        handle: Handle<MaterialAsset>,
        metadata: &ShaderMaterialMetadata,
    ) {
        if let Some(editor) = self.editor.as_mut() {
            if editor.entity() == entity && editor.handle() == handle {
                editor.sync_with_metadata(metadata);
                return;
            }
        }
        self.editor = Some(ShaderEditorState::new(entity, handle, metadata));
    }

    pub(crate) fn set_window_enabled(&mut self, enabled: bool) {
        self.window_enabled = enabled;
    }

    fn ensure_target_valid(&mut self, scene: &Scene) {
        let Some(editor) = self.editor.as_mut() else {
            return;
        };

        let world = scene.main_world();
        if !world.contains(editor.entity()) {
            editor.mark_target_missing("Entity has been removed.");
            return;
        }

        // Check if material still exists and has shader metadata
        if let Some(material_asset) = scene.assets.material(editor.handle()) {
            if let MaterialKind::Shader(metadata) = material_asset.kind() {
                editor.clear_target_missing();
                editor.sync_with_metadata(metadata);
            } else {
                editor.mark_target_missing("Material is no longer a shader material.");
            }
        } else {
            editor.mark_target_missing("Material asset has been removed.");
        }
    }

    fn notify_saved(&mut self, handle: Handle<MaterialAsset>, message: impl Into<String>) {
        if let Some(editor) = self.editor.as_mut() {
            if editor.handle() == handle {
                editor.finish_save(message);
            }
        }
    }

    fn notify_error(&mut self, handle: Handle<MaterialAsset>, message: impl Into<String>) {
        if let Some(editor) = self.editor.as_mut() {
            if editor.handle() == handle {
                editor.fail_save(message);
            }
        }
    }

    fn drain_shader_commands(queue: &mut VecDeque<EditorCommand>) -> Vec<PendingShaderAction> {
        let mut pending = Vec::new();
        let mut remaining = VecDeque::with_capacity(queue.len());

        while let Some(command) = queue.pop_front() {
            match command {
                EditorCommand::Shader(action) => pending.push(action),
                other => remaining.push_back(other),
            }
        }

        *queue = remaining;
        pending
    }

    fn process_pending_actions(
        &mut self,
        app: &mut EditorAppAccess<'_>,
        scene: &mut SceneWorkspaceSceneMut<'_>,
        mut renderer: Option<&mut Renderer>,
        actions: Vec<PendingShaderAction>,
    ) {
        if actions.is_empty() {
            return;
        }

        let mut notifications = Vec::new();

        for action in actions {
            match action {
                PendingShaderAction::Save {
                    handle,
                    contents,
                    message,
                } => {
                    let mut updated = false;
                    let mut error_message = None;

                    if let Some(material_asset) = scene.assets.material_mut(handle) {
                        if let MaterialKind::Shader(metadata) = material_asset.kind_mut() {
                            metadata.set_wgsl_source(contents.clone());

                            // If there's a source path, write to file
                            #[cfg(not(target_arch = "wasm32"))]
                            if let Some(path) = metadata.source_path() {
                                match std::fs::write(path, &contents) {
                                    Ok(()) => {
                                        updated = true;
                                    }
                                    Err(err) => {
                                        error_message = Some(format!(
                                            "Failed to write shader file {}: {err}",
                                            path.display()
                                        ));
                                        warn!(
                                            "Failed to write shader file {} for {:?}: {err}",
                                            path.display(),
                                            handle
                                        );
                                    }
                                }
                            } else {
                                // Inline shader source (no file)
                                updated = true;
                            }

                            // Invalidate shader modules to force recompilation
                            if updated {
                                if let Some(renderer) = renderer.as_mut() {
                                    renderer.invalidate_material_shader_modules(handle, None);
                                }
                            }
                        } else {
                            error_message = Some("Material is not a shader material.".to_string());
                            warn!(
                                "Attempted to save shader for non-shader material {:?}",
                                handle
                            );
                        }
                    } else {
                        error_message = Some("Material asset not found.".to_string());
                        warn!("Material asset {:?} not found when saving shader", handle);
                    }

                    if let Some(message) = error_message {
                        self.notify_error(handle, message);
                    }

                    if updated {
                        notifications.push((handle, message));
                        app.record_scene_change(scene);
                    }
                }
            }
        }

        // Send success notifications
        for (handle, message) in notifications {
            self.notify_saved(handle, message);
        }
    }
}

impl EditorSystem for ShaderEditorSystem {
    fn gpu_update<'app, 'ctx, 'scene>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'scene>) {
        if ctx.gpu_context_mut().is_none() {
            return;
        }

        let actions = {
            let queue = ctx.command_queue();
            Self::drain_shader_commands(queue)
        };

        let _ = ctx.with_gpu_app(move |app, gpu_ctx| {
            self.ensure_target_valid(&gpu_ctx.scene);
            self.process_pending_actions(app, &mut gpu_ctx.scene, Some(gpu_ctx.renderer), actions);
        });
    }

    fn ui<'app, 'ctx>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'ctx>) {
        let egui_ctx = {
            let Some(ui_ctx) = ctx.ui_context() else {
                return;
            };
            ui_ctx.egui()
        };

        if !self.window_enabled {
            return;
        }

        let Some(editor) = self.editor.as_mut() else {
            return;
        };

        let event = editor.show(egui_ctx);

        match event {
            ShaderEditorEvent::None => {}
            ShaderEditorEvent::Closed => {
                self.editor = None;
            }
            ShaderEditorEvent::Save {
                _entity,
                handle,
                contents,
                message,
            } => {
                ctx.command_queue()
                    .push_back(EditorCommand::Shader(PendingShaderAction::Save {
                        handle,
                        contents,
                        message,
                    }));
            }
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
