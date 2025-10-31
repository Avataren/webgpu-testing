use wgpu_cube::app::RuntimeMode;
use wgpu_cube::scene::TransformGizmoSpace;

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
}
