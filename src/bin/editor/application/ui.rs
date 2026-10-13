use wgpu_cube::app::RuntimeMode;
use wgpu_cube::scene::{TransformGizmoMode, TransformGizmoSpace};

use super::core::{EditorApplication, GameViewDisplayMode};
use super::EditorCommand;
use crate::theme::{self, colors, icons};

/// A rounded label with a colored dot, e.g. the play state.
///
/// Painted at its exact size, so it works in any layout direction.
fn status_pill(ui: &mut egui::Ui, text: &str, color: egui::Color32) {
    let galley = ui.painter().layout_no_wrap(
        text.to_owned(),
        egui::TextStyle::Body.resolve(ui.style()),
        colors::TEXT,
    );
    let padding = egui::vec2(10.0, 3.0);
    let dot_space = 14.0;
    let size = egui::vec2(
        galley.size().x + dot_space + padding.x * 2.0,
        galley.size().y + padding.y * 2.0,
    );
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, rect.height() / 2.0, colors::BG_RAISED);
    painter.circle_filled(
        egui::pos2(rect.min.x + padding.x + 4.0, rect.center().y),
        4.0,
        color,
    );
    painter.galley(
        egui::pos2(
            rect.min.x + padding.x + dot_space,
            rect.center().y - galley.size().y / 2.0,
        ),
        galley,
        colors::TEXT,
    );
}

impl EditorApplication {
    pub(super) fn show_menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = &ui.ctx().clone();
        egui::Panel::top("editor_top_bar")
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(colors::BG_DEEP)
                    .inner_margin(egui::Margin::symmetric(8, 4)),
            )
            .show(ui, |ui| {
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

                        ui.separator();
                        if ui.button("Exit").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            ui.close();
                        }
                    });

                    ui.menu_button("Edit", |ui| {
                        let can_undo = self.history().can_undo();
                        let can_redo = self.history().can_redo();
                        if ui
                            .add_enabled(
                                can_undo,
                                egui::Button::new("Undo").shortcut_text("Ctrl+Z"),
                            )
                            .clicked()
                        {
                            self.enqueue_command(EditorCommand::HistoryUndo);
                            ui.close();
                        }
                        if ui
                            .add_enabled(
                                can_redo,
                                egui::Button::new("Redo").shortcut_text("Ctrl+Shift+Z"),
                            )
                            .clicked()
                        {
                            self.enqueue_command(EditorCommand::HistoryRedo);
                            ui.close();
                        }
                        ui.separator();
                        let selected = self.selection_system().selected();
                        if ui
                            .add_enabled(
                                selected.is_some(),
                                egui::Button::new("Delete Entity").shortcut_text("Del"),
                            )
                            .clicked()
                        {
                            if let Some(entity) = selected {
                                self.enqueue_command(EditorCommand::DeleteEntity(entity));
                            }
                            ui.close();
                        }
                    });

                    ui.menu_button("Build", |ui| {
                        self.project_system_mut().build_menu_contents(ui);
                    });

                    ui.menu_button("Window", |ui| {
                        self.shared.windows.window_menu(ui);
                    });

                    let project_name = self
                        .project_system()
                        .controller()
                        .current_dir()
                        .map(|_| self.project_system().controller().metadata().name.clone());
                    if let Some(name) = project_name {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(theme::hint(name));
                        });
                    }
                });

                ui.add_space(2.0);
                self.show_toolbar(ui);
            });
    }

    fn show_toolbar(&mut self, ui: &mut egui::Ui) {
        let desired = self.shared.runtime_state.desired_mode();
        let active = self.shared.runtime_state.active_mode();
        let requesting_play = matches!(desired, RuntimeMode::Playing);
        let active_play = matches!(active, RuntimeMode::Playing);
        let paused = active_play && self.shared.runtime_state.time_scale() == 0.0;

        let bar_height = 28.0;
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), bar_height),
            egui::Sense::hover(),
        );

        // Left: transform tools.
        let mut left = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        {
            let tool = self.history_system_mut().transform_tool_mut();
            left.spacing_mut().item_spacing.x = 2.0;
            for (mode, label, key) in [
                (TransformGizmoMode::Translate, "Move", "W"),
                (TransformGizmoMode::Rotate, "Rotate", "E"),
                (TransformGizmoMode::Scale, "Scale", "R"),
            ] {
                left.selectable_value(&mut tool.gizmo_mode, mode, label)
                    .on_hover_text(format!("{label} tool ({key})"));
            }
            left.add_space(10.0);
            left.selectable_value(&mut tool.gizmo_space, TransformGizmoSpace::Local, "Local")
                .on_hover_text("Gizmo follows the entity's rotation");
            left.selectable_value(&mut tool.gizmo_space, TransformGizmoSpace::World, "World")
                .on_hover_text("Gizmo is aligned to the world axes");
        }

        // Center: play controls.
        let controls_width = 120.0;
        let center_rect =
            egui::Rect::from_center_size(rect.center(), egui::vec2(controls_width, bar_height));
        let mut center = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(center_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        center.spacing_mut().item_spacing.x = 4.0;
        let button_size = egui::vec2(34.0, 24.0);

        let play = egui::Button::new(egui::RichText::new(icons::PLAY).color(if active_play {
            colors::SUCCESS
        } else {
            colors::TEXT_STRONG
        }))
        .min_size(button_size)
        .selected(active_play);
        if center
            .add_enabled(!requesting_play, play)
            .on_hover_text("Play (Ctrl+P)")
            .clicked()
        {
            self.shared.runtime_state.request_mode(RuntimeMode::Playing);
        }

        let pause = egui::Button::new(icons::PAUSE)
            .min_size(button_size)
            .selected(paused);
        if center
            .add_enabled(active_play, pause)
            .on_hover_text(if paused { "Resume" } else { "Pause" })
            .clicked()
        {
            let scale = if paused { 1.0 } else { 0.0 };
            self.shared.runtime_state.set_time_scale(scale);
        }

        let stop = egui::Button::new(icons::STOP).min_size(button_size);
        if center
            .add_enabled(requesting_play || active_play, stop)
            .on_hover_text("Stop and restore the scene (Ctrl+P)")
            .clicked()
        {
            self.shared.runtime_state.request_mode(RuntimeMode::Editor);
        }

        // Right: mode status and game view options.
        let mut right = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
        );
        right.spacing_mut().item_spacing.x = 2.0;
        let fullscreen = &mut self.shared.viewports.game_view_display;
        let mut maximize = matches!(fullscreen, GameViewDisplayMode::Fullscreen);
        if right
            .toggle_value(&mut maximize, "Maximize on Play")
            .on_hover_text("Fill the whole window with the game while playing")
            .changed()
        {
            *fullscreen = if maximize {
                GameViewDisplayMode::Fullscreen
            } else {
                GameViewDisplayMode::Viewport
            };
        }
        right.add_space(12.0);

        let (status_text, status_color) = match (active_play, requesting_play) {
            (true, true) if paused => ("Paused", colors::WARNING),
            (true, true) => ("Playing", colors::SUCCESS),
            (true, false) => ("Stopping...", colors::WARNING),
            (false, true) => ("Starting...", colors::WARNING),
            (false, false) => ("Editing", colors::TEXT_WEAK),
        };
        status_pill(&mut right, status_text, status_color);
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
            // Enqueue the rename command to be processed during update
            self.enqueue_command(EditorCommand::RenameScene {
                old_id: document_id,
                new_name: self.shared.rename_dialog_text.clone(),
            });
            self.shared.pending_scene_rename = None;
            self.shared.rename_dialog_text.clear();
        } else if cancelled || !open {
            self.shared.pending_scene_rename = None;
            self.shared.rename_dialog_text.clear();
        }
    }
}
