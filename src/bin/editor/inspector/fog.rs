//! The environment section's fog: how thick it is and where, its colour and glow, and how
//! it takes the light.

use egui::Grid;
use wgpu_cube::fog::Fog;
use wgpu_cube::scene::EnvironmentComponent;

use super::labeled_slider;

/// The fog's controls, inside the environment section. Returns whether `edited` changed.
pub(super) fn show_fog_controls(ui: &mut egui::Ui, edited: &mut EnvironmentComponent) -> bool {
    let mut changed = false;
    let mut enabled = edited.fog.as_ref().is_some_and(|fog| fog.enabled);
    if ui
        .checkbox(&mut enabled, "Fog")
        .on_hover_text(
            "Height fog lit by the sky, the sun and moon and, near the camera, every lamp, \
             with sunbeams and halos (graphics setting Volumetric fog)",
        )
        .changed()
    {
        edited.fog.get_or_insert_with(Fog::default).enabled = enabled;
        changed = true;
    }
    let Some(fog) = edited.fog.as_mut().filter(|fog| fog.enabled) else {
        return changed;
    };

    egui::CollapsingHeader::new("Fog settings")
        .id_salt("fog_settings")
        .default_open(true)
        .show(ui, |ui| {
            Grid::new("fog_grid")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Density").on_hover_text(
                        "Light taken away per meter at and below the base height: light \
                         fades to a third over 1 / density meters",
                    );
                    changed |= ui
                        .add(
                            wgpu_cube::ui::FineSlider::new(&mut fog.density, 0.0..=1.0)
                                .step(0.0005)
                                .logarithmic(true),
                        )
                        .changed();
                    ui.end_row();
                    labeled_slider(
                        ui,
                        "Base height (m)",
                        &mut fog.height,
                        -50.0..=100.0,
                        0.05,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "Height falloff (m)",
                        &mut fog.height_falloff,
                        0.0..=200.0,
                        0.1,
                        &mut changed,
                    );

                    ui.label("Color").on_hover_text(
                        "The share of light the fog scatters per channel (the rest it absorbs)",
                    );
                    changed |=
                        egui::widgets::color_picker::color_edit_button_rgb(ui, &mut fog.albedo)
                            .changed();
                    ui.end_row();

                    ui.label("Glow").on_hover_text(
                        "Light the fog gives off itself, the colour it takes on where thick",
                    );
                    changed |=
                        egui::widgets::color_picker::color_edit_button_rgb(ui, &mut fog.emission)
                            .changed();
                    ui.end_row();

                    labeled_slider(
                        ui,
                        "Forward scattering",
                        &mut fog.anisotropy,
                        -Fog::MAX_ANISOTROPY..=Fog::MAX_ANISOTROPY,
                        0.01,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "Ambient light",
                        &mut fog.ambient,
                        0.0..=4.0,
                        0.01,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "Light from lamps and sun",
                        &mut fog.light,
                        0.0..=4.0,
                        0.01,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "Volumetric distance (m)",
                        &mut fog.volumetric_distance,
                        Fog::MIN_VOLUMETRIC_DISTANCE..=Fog::MAX_VOLUMETRIC_DISTANCE,
                        0.5,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "Max distance (m)",
                        &mut fog.max_distance,
                        10.0..=10_000.0,
                        1.0,
                        &mut changed,
                    );
                });
        });
    changed
}
