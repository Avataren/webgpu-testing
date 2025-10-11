#[cfg(feature = "egui")]
use crate::renderer::postprocess::PostProcessEffects;
#[cfg(feature = "egui")]
use egui::{Context, Slider, Window};
#[cfg(feature = "egui")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "egui")]
pub type PostProcessEffectsHandle = Arc<Mutex<PostProcessEffects>>;

#[cfg(feature = "egui")]
pub struct PostProcessWindow {
    handle: PostProcessEffectsHandle,
    title: String,
}

#[cfg(feature = "egui")]
impl PostProcessWindow {
    pub fn new(handle: PostProcessEffectsHandle) -> Self {
        Self {
            handle,
            title: "Post-processing".to_string(),
        }
    }

    pub fn show(&mut self, ctx: &Context, open: Option<&mut bool>) {
        let mut effects = self
            .handle
            .lock()
            .map(|guard| *guard)
            .unwrap_or_else(|poisoned| *poisoned.into_inner());

        let mut changed = false;

        let mut window = Window::new(&self.title);
        if let Some(open) = open {
            window = window.open(open);
        }

        window.resizable(false).show(ctx, |ui| {
            ui.heading("Post-processing effects");
            ui.separator();

            ui.vertical(|ui| {
                changed |= ui
                    .checkbox(&mut effects.ssao, "Screen-space ambient occlusion")
                    .changed();
                changed |= ui.checkbox(&mut effects.bloom, "Bloom").changed();
                changed |= ui.checkbox(&mut effects.fxaa, "FXAA").changed();
            });

            ui.separator();
            ui.heading("SSAO");
            ui.add_enabled_ui(effects.ssao, |ui| {
                ui.label("Adjust the ambient occlusion contribution");
                changed |= ui
                    .add(Slider::new(&mut effects.ssao_settings.radius, 0.01..=1.0).text("Radius"))
                    .changed();
                changed |= ui
                    .add(Slider::new(&mut effects.ssao_settings.bias, 0.0..=0.2).text("Bias"))
                    .changed();
                changed |= ui
                    .add(
                        Slider::new(&mut effects.ssao_settings.intensity, 0.0..=2.0)
                            .text("Strength"),
                    )
                    .changed();
                changed |= ui
                    .add(Slider::new(&mut effects.ssao_settings.power, 0.5..=4.0).text("Power"))
                    .changed();
            });

            ui.separator();
            ui.heading("Bloom");
            ui.add_enabled_ui(effects.bloom, |ui| {
                ui.label("Control highlight extraction and scattering");
                changed |= ui
                    .add(
                        Slider::new(&mut effects.bloom_settings.threshold, 0.0..=2.0)
                            .text("Threshold"),
                    )
                    .changed();
                changed |= ui
                    .add(Slider::new(&mut effects.bloom_settings.knee, 0.0..=1.0).text("Knee"))
                    .changed();
                changed |= ui
                    .add(
                        Slider::new(&mut effects.bloom_settings.scatter, 0.0..=1.0).text("Scatter"),
                    )
                    .changed();
            });
        });

        if changed {
            if let Ok(mut guard) = self.handle.lock() {
                *guard = effects;
            }
        }
    }

    pub fn handle() -> PostProcessEffectsHandle {
        Arc::new(Mutex::new(PostProcessEffects::default()))
    }
}
