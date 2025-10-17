use wgpu_cube::DefaultUI;

#[derive(Debug, Clone)]
pub struct WindowToggles {
    pub stats: bool,
    pub environment: bool,
    pub postprocess: bool,
    pub log: bool,
}

impl WindowToggles {
    pub fn new() -> Self {
        Self {
            stats: true,
            environment: false,
            postprocess: false,
            log: false,
        }
    }

    pub fn window_menu(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.stats, "Statistics");
        ui.checkbox(&mut self.postprocess, "Post-processing");
        ui.checkbox(&mut self.environment, "Environment");
        ui.checkbox(&mut self.log, "Log");
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
        default_ui.log_window_mut().show(ctx, Some(&mut self.log));
    }
}
