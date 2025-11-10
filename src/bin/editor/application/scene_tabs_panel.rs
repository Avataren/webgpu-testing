use wgpu_cube::scene::workspace::SceneDocumentId;
use wgpu_cube::SceneTabDescriptor;

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
    pub fn show(
        &mut self,
        tabs: &[SceneTabDescriptor],
        ctx: &egui::Context,
    ) -> Vec<SceneTabAction> {
        let mut actions = Vec::new();

        egui::TopBottomPanel::top("scene_tabs_panel")
            .frame(egui::Frame::side_top_panel(&ctx.style()))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    for tab in tabs {
                        actions.extend(Self::show_tab(ui, tab));
                    }

                    ui.add_space(8.0);
                    if ui.button("+").clicked() {
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
                let label_response = ui.selectable_label(tab.active, &tab.title);

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

                let close = egui::Button::new("✕").small();
                if ui.add(close).clicked() {
                    close_clicked = true;
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
