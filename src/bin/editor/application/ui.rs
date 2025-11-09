use wgpu_cube::app::RuntimeMode;
use wgpu_cube::scene::TransformGizmoSpace;
use wgpu_cube::scene::workspace::SceneDocumentId;

use super::core::{EditorApplication, GameViewDisplayMode};
use super::EditorCommand;

impl EditorApplication {
    pub(super) fn show_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("editor_top_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    self.project_system_mut().menu_contents(ui);

                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if ui.button("Import glTF...").clicked() {
                            ui.close();
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("glTF", &["gltf", "glb"])
                                .pick_file()
                            {
                                self.enqueue_command(EditorCommand::ImportPath(path));
                            }
                        }
                    }

                    #[cfg(target_arch = "wasm32")]
                    {
                        ui.add_enabled(false, egui::Button::new("Import glTF..."));
                    }

                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        ui.close();
                    }
                });

                ui.menu_button("Build", |ui| {
                    self.project_system_mut().build_menu_contents(ui);
                });

                ui.menu_button("Window", |ui| {
                    self.shared.windows.window_menu(ui);
                });
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Toolbar").strong());
                ui.separator();
                let desired = self.shared.runtime_state.desired_mode();
                let active = self.shared.runtime_state.active_mode();
                let requesting_play = matches!(desired, RuntimeMode::Playing);
                let active_play = matches!(active, RuntimeMode::Playing);

                if ui
                    .add_enabled(!requesting_play, egui::Button::new("▶ Play"))
                    .clicked()
                {
                    self.shared.runtime_state.request_mode(RuntimeMode::Playing);
                }

                ui.add_enabled(false, egui::Button::new("Pause"));

                if ui
                    .add_enabled(requesting_play || active_play, egui::Button::new("⏹ Stop"))
                    .clicked()
                {
                    self.shared.runtime_state.request_mode(RuntimeMode::Editor);
                }

                ui.separator();

                let (status_text, status_color) = match (active_play, requesting_play) {
                    (true, true) => ("▶ Playing", egui::Color32::from_rgb(120, 200, 120)),
                    (true, false) => ("Stopping...", egui::Color32::from_rgb(220, 190, 0)),
                    (false, true) => (
                        "Starting Play Mode...",
                        egui::Color32::from_rgb(220, 190, 0),
                    ),
                    (false, false) => ("Editor Mode", egui::Color32::from_gray(180)),
                };

                ui.label(egui::RichText::new(status_text).color(status_color));

                ui.separator();
                ui.label("Space:");
                {
                    let transform_tool = self.history_system_mut().transform_tool_mut();
                    ui.selectable_value(
                        &mut transform_tool.gizmo_space,
                        TransformGizmoSpace::Local,
                        "Local",
                    );
                    ui.selectable_value(
                        &mut transform_tool.gizmo_space,
                        TransformGizmoSpace::World,
                        "World",
                    );
                }

                ui.separator();
                ui.label("Game View:");
                ui.selectable_value(
                    &mut self.shared.viewports.game_view_display,
                    GameViewDisplayMode::Viewport,
                    "Viewport",
                );
                ui.selectable_value(
                    &mut self.shared.viewports.game_view_display,
                    GameViewDisplayMode::Fullscreen,
                    "Fullscreen",
                );
            });
        });
    }

    pub(super) fn show_rename_scene_dialog(&mut self, ctx: &egui::Context) {
        let Some(document_id) = self.shared.pending_scene_rename.clone() else {
            return;
        };

        // Initialize dialog text if empty
        if self.shared.rename_dialog_text.is_empty() {
            // Extract the current scene name from document_id
            self.shared.rename_dialog_text = document_id
                .trim_end_matches(".scene")
                .replace('_', " ")
                .to_string();
        }

        let mut open = true;
        let mut confirmed = false;
        let mut cancelled = false;

        egui::Window::new("Rename Scene")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Enter new scene name:");
                let response = ui.text_edit_singleline(&mut self.shared.rename_dialog_text);

                // Auto-focus the text field when dialog opens
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    confirmed = true;
                } else {
                    response.request_focus();
                }

                ui.horizontal(|ui| {
                    if ui.button("OK").clicked() {
                        confirmed = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancelled = true;
                    }
                });
            });

        if confirmed && !self.shared.rename_dialog_text.trim().is_empty() {
            self.perform_scene_rename(document_id, self.shared.rename_dialog_text.clone());
            self.shared.pending_scene_rename = None;
            self.shared.rename_dialog_text.clear();
        } else if cancelled || !open {
            self.shared.pending_scene_rename = None;
            self.shared.rename_dialog_text.clear();
        }
    }

    fn perform_scene_rename(&mut self, old_document_id: SceneDocumentId, new_name: String) {
        // Sanitize the new name to create a valid document ID
        let sanitized = new_name
            .trim()
            .replace(' ', "_")
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '_' || *c == '-')
            .collect::<String>();

        if sanitized.is_empty() {
            return;
        }

        let new_document_id = format!("{}.scene", sanitized);

        // Check if the new name is different from the old one
        if new_document_id == old_document_id {
            return;
        }

        // Check if a scene with the new name already exists
        let existing_ids: std::collections::HashSet<_> = self
            .project_system()
            .scene_documents()
            .iter()
            .map(|doc| doc.id.as_str())
            .collect();

        if existing_ids.contains(new_document_id.as_str()) {
            log::warn!("Scene with name '{}' already exists", new_document_id);
            return;
        }

        // Get the current project directory to rename the file
        if let Some(project_dir) = self.project_system().controller().current_dir() {
            let old_path = project_dir
                .join(wgpu_cube::project::CONTENT_DIR)
                .join("scenes")
                .join(format!("{}.json", old_document_id));
            let new_path = project_dir
                .join(wgpu_cube::project::CONTENT_DIR)
                .join("scenes")
                .join(format!("{}.json", new_document_id));

            // Rename the file on disk
            if let Err(e) = std::fs::rename(&old_path, &new_path) {
                log::error!("Failed to rename scene file: {}", e);
                return;
            }
        }

        // Update scene document ID in the project controller
        let documents = self.project_system().scene_documents().to_vec();
        let mut updated_documents = Vec::new();

        for doc in documents {
            if doc.id == old_document_id {
                let mut new_doc = doc.clone();
                new_doc.id = new_document_id.clone();
                new_doc.name = new_name.clone();
                // Update the relative path
                new_doc.relative_path = std::path::PathBuf::from(wgpu_cube::project::CONTENT_DIR)
                    .join("scenes")
                    .join(format!("{}.json", new_document_id));
                updated_documents.push(new_doc);
            } else {
                updated_documents.push(doc);
            }
        }

        self.project_system_mut().controller_mut().set_scene_documents(updated_documents);

        log::info!("Renamed scene from '{}' to '{}'", old_document_id, new_document_id);
    }
}
