#[cfg(feature = "egui")]
use crate::renderer::postprocess::PostProcessEffects;
#[cfg(feature = "egui")]
use egui::{Context, Window};
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

        Window::new(&self.title)
            .open(open)
            .resizable(false)
            .show(ctx, |ui| {
                ui.heading("Post-processing effects");
                ui.separator();

                ui.vertical(|ui| {
                    changed |= ui
                        .checkbox(&mut effects.ssao, "Screen-space ambient occlusion")
                        .changed();
                    changed |= ui.checkbox(&mut effects.bloom, "Bloom").changed();
                    changed |= ui.checkbox(&mut effects.fxaa, "FXAA").changed();
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
