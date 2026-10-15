#[cfg(feature = "egui")]
use crate::environment::Environment;
#[cfg(feature = "egui")]
use egui::{Context, Window};
#[cfg(feature = "egui")]
use std::sync::{Arc, Mutex};
#[cfg(feature = "egui")]
use wgpu::Color;

#[cfg(feature = "egui")]
#[derive(Clone, Debug)]
pub struct EnvironmentSettingsControls {
    pub clear_color: [f32; 3],
    pub ambient_intensity: f32,
    pub hdr_enabled: bool,
    pub hdr_available: bool,
    pub hdr_intensity: f32,
    /// Draw the image behind the scene, not only for light and reflections.
    pub hdr_background: bool,
    pub dirty: bool,
}

#[cfg(feature = "egui")]
impl EnvironmentSettingsControls {
    pub fn from_environment(environment: &Environment) -> Self {
        let color = environment.clear_color();
        let hdr_background = environment.hdr_background();
        let hdr_available = hdr_background.is_some();
        let hdr_enabled = environment.is_hdr_enabled();
        let hdr_intensity = hdr_background.map(|hdr| hdr.intensity()).unwrap_or(1.0);
        let hdr_visible = hdr_background.is_none_or(|hdr| hdr.background());

        Self {
            clear_color: [color.r as f32, color.g as f32, color.b as f32],
            ambient_intensity: environment.ambient_intensity(),
            hdr_enabled: hdr_enabled && hdr_available,
            hdr_available,
            hdr_intensity,
            hdr_background: hdr_visible,
            dirty: false,
        }
    }

    pub fn apply_to_environment(&self, environment: &mut Environment) {
        environment.set_clear_color(Color {
            r: self.clear_color[0] as f64,
            g: self.clear_color[1] as f64,
            b: self.clear_color[2] as f64,
            a: 1.0,
        });
        environment.set_ambient_intensity(self.ambient_intensity);

        if let Some(hdr) = environment.hdr_background_mut() {
            hdr.set_enabled(self.hdr_enabled && self.hdr_available);
            hdr.set_intensity(self.hdr_intensity);
            hdr.set_background(self.hdr_background);
        }
    }
}

#[cfg(feature = "egui")]
pub type EnvironmentSettingsHandle = Arc<Mutex<EnvironmentSettingsControls>>;

#[cfg(feature = "egui")]
pub struct EnvironmentWindow {
    handle: EnvironmentSettingsHandle,
    title: String,
}

#[cfg(feature = "egui")]
impl EnvironmentWindow {
    pub fn new(handle: EnvironmentSettingsHandle) -> Self {
        Self {
            handle,
            title: "Environment".to_string(),
        }
    }

    pub fn show(&mut self, ctx: &Context, open: Option<&mut bool>) {
        let controls = self
            .handle
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone());

        let mut controls = controls;
        let mut changed = false;

        let mut window = Window::new(&self.title).resizable(true);
        if let Some(open) = open {
            window = window.open(open);
        }

        window.show(ctx, |ui| {
            ui.heading("Global environment settings");
            ui.separator();

            changed |= ui
                .color_edit_button_rgb(&mut controls.clear_color)
                .changed();

            Self::slider_row(
                ui,
                &mut controls.ambient_intensity,
                0.0..=2.0,
                0.005,
                "Ambient intensity",
                &mut changed,
            );

            ui.separator();
            ui.heading("HDR background");
            if controls.hdr_available {
                changed |= ui
                    .checkbox(&mut controls.hdr_enabled, "Enable HDR background")
                    .changed();

                Self::slider_row(
                    ui,
                    &mut controls.hdr_intensity,
                    0.0..=5.0,
                    0.01,
                    "Intensity",
                    &mut changed,
                );
                changed |= ui
                    .checkbox(&mut controls.hdr_background, "Show behind the scene")
                    .on_hover_text(
                        "Off: the image only lights the scene and shows in reflections; \
                         the clear colour is drawn behind",
                    )
                    .changed();
            } else {
                ui.label("No HDR background is loaded.");
            }

            ui.separator();
            ui.label("Exposure, tonemapping and colour grading are set on Post Process Volumes.");
        });

        if changed {
            controls.dirty = true;
            if let Ok(mut guard) = self.handle.lock() {
                *guard = controls;
            }
        }
    }

    fn slider_row(
        ui: &mut egui::Ui,
        value: &mut f32,
        range: std::ops::RangeInclusive<f32>,
        step: f32,
        label: &str,
        changed: &mut bool,
    ) {
        ui.horizontal(|ui| {
            ui.label(label);
            *changed |= crate::ui::fine_slider(ui, value, range, step).changed();
        });
    }

    pub fn handle_from_environment(environment: &Environment) -> EnvironmentSettingsHandle {
        Arc::new(Mutex::new(EnvironmentSettingsControls::from_environment(
            environment,
        )))
    }

    pub fn sync_handle(handle: &EnvironmentSettingsHandle, environment: &Environment) {
        if let Ok(mut guard) = handle.lock() {
            *guard = EnvironmentSettingsControls::from_environment(environment);
        }
    }
}
