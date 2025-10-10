#[cfg(feature = "egui")]
use crate::renderer::postprocess::{PostProcessSettings, PostProcessSettingsHandle};
#[cfg(feature = "egui")]
use egui::{RichText, Ui};

#[cfg(feature = "egui")]
pub struct PostProcessWindow {
    settings: PostProcessSettingsHandle,
    title: String,
}

#[cfg(feature = "egui")]
impl PostProcessWindow {
    pub fn new(settings: PostProcessSettingsHandle) -> Self {
        Self {
            settings,
            title: "Post-processing".to_string(),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, open: Option<&mut bool>) {
        let mut window = egui::Window::new(&self.title).default_width(260.0);
        if let Some(open) = open {
            window = window.open(open);
        }

        window.show(ctx, |ui| {
            if let Ok(mut settings) = self.settings.lock() {
                self.draw_contents(ui, &mut *settings);
            } else {
                ui.label("Settings unavailable");
            }
        });
    }

    pub fn settings_handle(&self) -> PostProcessSettingsHandle {
        self.settings.clone()
    }

    fn draw_contents(&self, ui: &mut Ui, settings: &mut PostProcessSettings) {
        ui.label(RichText::new("Enable or disable post-processing passes:"));
        ui.separator();

        let mut updated = false;

        let mut ssao = settings.ssao_enabled;
        updated |= ui
            .checkbox(&mut ssao, "Screen-space ambient occlusion")
            .changed();

        let mut bloom = settings.bloom_enabled;
        updated |= ui.checkbox(&mut bloom, "Bloom").changed();

        let mut fxaa = settings.fxaa_enabled;
        updated |= ui.checkbox(&mut fxaa, "FXAA anti-aliasing").changed();

        if updated {
            settings.ssao_enabled = ssao;
            settings.bloom_enabled = bloom;
            settings.fxaa_enabled = fxaa;
        }
    }
}
