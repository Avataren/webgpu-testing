use wgpu_cube::DefaultUI;

#[derive(Debug, Clone)]
pub struct WindowToggles {
    pub stats: bool,
    pub environment: bool,
    pub postprocess: bool,
    pub plugin_manager: bool,
}

impl WindowToggles {
    pub fn new() -> Self {
        Self {
            stats: false,
            environment: false,
            postprocess: false,
            plugin_manager: false,
        }
    }

    pub fn window_menu(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.stats, "Statistics");
        ui.checkbox(&mut self.postprocess, "Post-processing");
        ui.checkbox(&mut self.environment, "Environment");
        ui.checkbox(&mut self.plugin_manager, "Plugin Manager");
    }

    pub fn show(&mut self, ctx: &egui::Context, default_ui: &mut DefaultUI) {
        default_ui
            .stats_window_mut()
            .show(ctx, Some(&mut self.stats));
        default_ui
            .postprocess_window_mut()
            .show(ctx, Some(&mut self.postprocess));
        default_ui
            .environment_window_mut()
            .show(ctx, Some(&mut self.environment));
    }
}
