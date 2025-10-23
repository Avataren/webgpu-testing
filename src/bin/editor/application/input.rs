use glam::Vec2;
use wgpu_cube::app::RuntimeMode;
use wgpu_cube::scene::TransformGizmoMode;

use super::core::{EditorApplication, ViewportPick};
use super::EditorCommand;

impl EditorApplication {
    pub(super) fn capture_viewport_pick_input(&mut self, ctx: &egui::Context) {
        if matches!(self.runtime_state.active_mode(), RuntimeMode::Playing) {
            let selection = self.selection_system_mut();
            selection.clear_pending_pick();
            selection.reset_pointer_press();
            return;
        }

        if self.camera_system().is_looking() {
            self.selection_system_mut().reset_pointer_press();
            return;
        }

        let Some(rect) = self.viewports.scene_viewport.rect() else {
            self.selection_system_mut().reset_pointer_press();
            return;
        };
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            self.selection_system_mut().reset_pointer_press();
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

        let gizmo_idle = self.history_system().gizmo_drag().is_none();
        let selection = self.selection_system_mut();
        selection.set_pointer_primary_down(pointer_down);

        if let Some(uv) = pressed_uv {
            selection.set_pointer_press_uv(Some(uv));
            selection.set_selection_press_uv(Some(uv));
        }

        if let Some(uv) = released_uv {
            if gizmo_idle && selection.take_selection_press_uv().is_some() {
                selection.set_pending_pick(ViewportPick { uv });
            }
        } else if !selection.pointer_primary_down() {
            selection.set_selection_press_uv(None);
        }
    }

    pub(super) fn handle_gizmo_shortcuts(&mut self, ctx: &egui::Context) {
        if self.camera_system().is_looking() {
            return;
        }
        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::W) {
                self.history_system_mut().transform_tool_mut().gizmo_mode =
                    TransformGizmoMode::Translate;
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::E) {
                self.history_system_mut().transform_tool_mut().gizmo_mode =
                    TransformGizmoMode::Rotate;
            } else if input.consume_key(egui::Modifiers::NONE, egui::Key::R) {
                self.history_system_mut().transform_tool_mut().gizmo_mode =
                    TransformGizmoMode::Scale;
            }
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Delete) {
                if let Some(entity) = self.selection_system().selected() {
                    self.enqueue_command(EditorCommand::DeleteEntity(entity));
                    self.history_system_mut().clear_gizmo_drag();
                }
            }
        });
    }

    pub(super) fn handle_general_shortcuts(&mut self, ctx: &egui::Context) {
        if self.camera_system().is_looking() {
            return;
        }

        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                self.selection_system_mut().clear_selection();
            }
        });
    }

    pub(super) fn handle_history_shortcuts(&mut self, ctx: &egui::Context) {
        if self.camera_system().is_looking() {
            return;
        }

        ctx.input_mut(|input| {
            let undo_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z);
            if input.consume_shortcut(&undo_shortcut) && self.history().can_undo() {
                self.history_system_mut().request_undo();
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
                && self.history().can_redo()
            {
                self.history_system_mut().request_redo();
            }
        });
    }
}
