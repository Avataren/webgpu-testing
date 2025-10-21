use glam::Vec2;
use wgpu_cube::app::RuntimeMode;
use wgpu_cube::scene::TransformGizmoMode;

use super::core::{EditorApplication, ViewportPick};

impl EditorApplication {
    pub(super) fn capture_viewport_pick_input(&mut self, ctx: &egui::Context) {
        if matches!(self.runtime_state.active_mode(), RuntimeMode::Playing) {
            self.selection.clear_pending_pick();
            self.selection.pointer.reset_press();
            return;
        }

        if self.camera_controller.is_looking() {
            self.selection.pointer.reset_press();
            return;
        }

        let Some(rect) = self.viewports.scene_viewport.rect() else {
            self.selection.pointer.reset_press();
            return;
        };
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            self.selection.pointer.reset_press();
            return;
        }

        let mut pressed_uv: Option<Vec2> = None;
        let mut released_uv: Option<Vec2> = None;
        let mut pointer_down = false;

        ctx.input(|input| {
            pointer_down = input.pointer.button_down(egui::PointerButton::Primary);

            if input.pointer.button_pressed(egui::PointerButton::Primary) {
                if let Some(pos) = input.pointer.latest_pos() {
                    if rect.contains(pos) {
                        let uv = Self::viewport_uv(rect, pos);
                        if uv.is_finite() {
                            pressed_uv = Some(uv);
                        }
                    }
                }
            }

            if input.pointer.button_released(egui::PointerButton::Primary) {
                if let Some(pos) = input.pointer.latest_pos() {
                    if rect.contains(pos) {
                        let uv = Self::viewport_uv(rect, pos);
                        if uv.is_finite() {
                            released_uv = Some(uv);
                        }
                    }
                }
            }
        });

        self.selection.pointer.primary_down = pointer_down;

        if let Some(uv) = pressed_uv {
            self.selection.pointer.press_uv = Some(uv);
            self.selection.pointer.selection_press_uv = Some(uv);
        }

        if let Some(uv) = released_uv {
            if self.transform_tool.gizmo_drag.is_none()
                && self.selection.pointer.selection_press_uv.take().is_some()
            {
                self.selection.set_pending_pick(ViewportPick { uv });
            }
        } else if !self.selection.pointer.primary_down {
            self.selection.pointer.selection_press_uv = None;
        }
    }

    pub(super) fn handle_gizmo_shortcuts(&mut self, ctx: &egui::Context) {
        if self.camera_controller.is_looking() {
            return;
        }
        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::W) {
                self.transform_tool.gizmo_mode = TransformGizmoMode::Translate;
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::E) {
                self.transform_tool.gizmo_mode = TransformGizmoMode::Rotate;
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::R) {
                self.transform_tool.gizmo_mode = TransformGizmoMode::Scale;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Delete) {
                if let Some(entity) = self.selection.selected() {
                    self.pending_entity_deletions.push(entity);
                    self.transform_tool.gizmo_drag = None;
                }
            }
        });
    }

    pub(super) fn handle_general_shortcuts(&mut self, ctx: &egui::Context) {
        if self.camera_controller.is_looking() {
            return;
        }

        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                self.clear_selection();
            }
        });
    }

    pub(super) fn handle_history_shortcuts(&mut self, ctx: &egui::Context) {
        if self.camera_controller.is_looking() {
            return;
        }

        ctx.input_mut(|input| {
            let undo_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);
            if input.consume_shortcut(&undo_shortcut) && self.history.can_undo() {
                self.undo_redo.request_undo();
            }

            let mut redo_mods = egui::Modifiers::COMMAND;
            redo_mods.shift = true;
            let redo_shortcut = egui::KeyboardShortcut::new(redo_mods, egui::Key::Z);
            let redo_variants = [
                redo_shortcut,
                egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y),
            ];
            if redo_variants
                .iter()
                .any(|shortcut| input.consume_shortcut(shortcut))
                && self.history.can_redo()
            {
                self.undo_redo.request_redo();
            }
        });
    }
}
