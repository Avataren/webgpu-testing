#[cfg(feature = "egui")]
use crate::environment::Environment;
#[cfg(feature = "egui")]
use egui::{Context, Slider, Window};
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
    pub exposure: f32,
    pub saturation: f32,
    pub contrast: f32,
}

#[cfg(feature = "egui")]
impl EnvironmentSettingsControls {
    pub fn from_environment(environment: &Environment) -> Self {
        let color = environment.clear_color();
        let hdr_background = environment.hdr_background();
        let hdr_available = hdr_background.is_some();
        let hdr_enabled = environment.is_hdr_enabled();
        let hdr_intensity = hdr_background.map(|hdr| hdr.intensity()).unwrap_or(1.0);
        let grading = environment.color_grading();

        Self {
            clear_color: [color.r as f32, color.g as f32, color.b as f32],
            ambient_intensity: environment.ambient_intensity(),
            hdr_enabled: hdr_enabled && hdr_available,
            hdr_available,
            hdr_intensity,
            exposure: grading.exposure(),
            saturation: grading.saturation(),
            contrast: grading.contrast(),
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

        let mut grading = environment.color_grading();
        grading.set_exposure(self.exposure);
        grading.set_saturation(self.saturation);
        grading.set_contrast(self.contrast);
        environment.set_color_grading(grading);

        if let Some(hdr) = environment.hdr_background_mut() {
            hdr.set_enabled(self.hdr_enabled && self.hdr_available);
            hdr.set_intensity(self.hdr_intensity);
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
    const SLIDER_WIDTH: f32 = 250.0;

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

        let mut window = Window::new(&self.title).resizable(false);
        if let Some(open) = open {
            window = window.open(open);
        }

        window.show(ctx, |ui| {
            ui.heading("Global environment settings");
            ui.separator();

            changed |= ui.color_edit_button_rgb(&mut controls.clear_color).changed();

            Self::slider_row(
                ui,
                &mut controls.ambient_intensity,
                0.0..=2.0,
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
                    "Intensity",
                    &mut changed,
                );
            } else {
                ui.label("No HDR background is loaded.");
            }

            ui.separator();
            ui.heading("Color grading");

            Self::slider_row(
                ui,
                &mut controls.exposure,
                0.0..=4.0,
                "Exposure",
                &mut changed,
            );
            Self::slider_row(
                ui,
                &mut controls.saturation,
                0.0..=2.0,
                "Saturation",
                &mut changed,
            );
            Self::slider_row(
                ui,
                &mut controls.contrast,
                0.0..=3.0,
                "Contrast",
                &mut changed,
            );
        });

        if changed {
            if let Ok(mut guard) = self.handle.lock() {
                *guard = controls;
            }
        }
    }

    fn slider_row<T: egui::emath::Numeric>(
        ui: &mut egui::Ui,
        value: &mut T,
        range: std::ops::RangeInclusive<T>,
        label: &str,
        changed: &mut bool,
    ) {
        ui.horizontal(|ui| {
            ui.spacing_mut().slider_width = Self::SLIDER_WIDTH;
            *changed |= ui.add(Slider::new(value, range).text(label)).changed();
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
