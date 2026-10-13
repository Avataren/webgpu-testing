use wgpu_cube::scene::workspace::SceneDocumentId;
use wgpu_cube::SceneTabDescriptor;

use crate::theme::{self, colors, icons};

#[derive(Debug)]
pub(super) enum SceneTabAction {
    Activate(SceneDocumentId),
    Close(SceneDocumentId),
    NewScene,
    Rename(SceneDocumentId),
}

#[derive(Default)]
pub(super) struct SceneTabsPanel;

impl SceneTabsPanel {
    pub fn show(&mut self, tabs: &[SceneTabDescriptor], ui: &mut egui::Ui) -> Vec<SceneTabAction> {
        let mut actions = Vec::new();

        egui::Panel::top("scene_tabs_panel")
            .frame(
                egui::Frame::new()
                    .fill(colors::BG_DEEP)
                    .inner_margin(egui::Margin {
                        left: 8,
                        right: 8,
                        top: 0,
                        bottom: 4,
                    }),
            )
            .show_separator_line(false)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::hint("Scenes").small());
                    ui.add_space(4.0);
                    for tab in tabs {
                        actions.extend(Self::show_tab(ui, tab));
                    }

                    ui.add_space(4.0);
                    if ui
                        .add(egui::Button::new(icons::ADD).frame(false))
                        .on_hover_text("New scene")
                        .clicked()
                    {
                        actions.push(SceneTabAction::NewScene);
                    }
                });
            });

        actions
    }

    fn show_tab(ui: &mut egui::Ui, tab: &SceneTabDescriptor) -> Vec<SceneTabAction> {
        let mut actions = Vec::new();
        let mut tab_clicked = false;
        let mut close_clicked = false;
        let mut rename_clicked = false;

        ui.scope(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            ui.spacing_mut().button_padding = egui::vec2(8.0, 4.0);
            ui.horizontal(|ui| {
                let label_response = ui
                    .selectable_label(tab.active, &tab.title)
                    .on_hover_text("Right-click to rename");

                // Add context menu for rename
                label_response.context_menu(|ui| {
                    if ui.button("Rename").clicked() {
                        rename_clicked = true;
                        ui.close();
                    }
                });

                if label_response.clicked() {
                    tab_clicked = true;
                }

                // Only show close button if this tab can be closed
                if tab.can_close {
                    let close = egui::Button::new(icons::CLOSE).frame(false).small();
                    if ui.add(close).on_hover_text("Close scene").clicked() {
                        close_clicked = true;
                    }
                }
            });
        });

        if tab_clicked && !tab.active {
            actions.push(SceneTabAction::Activate(tab.document_id.clone()));
        }

        if close_clicked {
            actions.push(SceneTabAction::Close(tab.document_id.clone()));
        }

        if rename_clicked {
            actions.push(SceneTabAction::Rename(tab.document_id.clone()));
        }

        actions
    }
}
