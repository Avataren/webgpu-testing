//! The environment section's procedural sky: its day/night clock, where on Earth it is,
//! and how it looks.

use egui::{DragValue, Grid};
use wgpu_cube::scene::EnvironmentComponent;
use wgpu_cube::sky::{Date, ProceduralSky};

use super::{labeled_slider, InspectorAction};

/// The sky's clock as the scene runs it, for the environment section (refreshed every
/// frame by the light probe system).
#[derive(Clone, Debug, Default)]
pub struct SkyPanelStatus {
    /// The date and local time the sky shows now.
    pub date: String,
    pub clock: String,
    /// Whether the clock runs (play mode, or the editor's preview, with a day length).
    pub running: bool,
    pub preview: bool,
    pub sun_altitude: f64,
    pub moon_illumination: f64,
    /// "sun", "moon" or "none": what the lights following the sky shine with.
    pub light: &'static str,
    /// Whether any directional light follows the sky.
    pub followed: bool,
    /// A bake of the day's hours: hours done, of how many.
    pub day_bake: Option<(usize, usize)>,
}

/// What the sky's controls ask of the scene beyond its settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkyRequest {
    /// Run the clock in the editor too.
    Preview(bool),
    /// Back to the sky's own date and time.
    ResetClock,
    /// Make the scene's first directional light follow the sky (a new Sun without one).
    FollowFirstLight,
}

/// The sky's controls, inside the environment section's grid. Returns whether `edited`
/// changed.
pub(super) fn show_sky_controls(
    ui: &mut egui::Ui,
    edited: &mut EnvironmentComponent,
    status: Option<&SkyPanelStatus>,
    actions: &mut Vec<InspectorAction>,
) -> bool {
    let mut changed = false;
    let mut enabled = edited.sky.as_ref().is_some_and(|sky| sky.enabled);
    let toggled = ui
        .checkbox(&mut enabled, "Procedural sky with day and night")
        .on_hover_text(
            "Scattered sunlight and moonlight, stars and the moon, on their real paths for \
             the date and place below; replaces the HDR sky while on",
        )
        .changed();
    if toggled {
        let sky = edited.sky.get_or_insert_with(ProceduralSky::default);
        sky.enabled = enabled;
        changed = true;
        if enabled && !status.is_some_and(|status| status.followed) {
            actions.push(InspectorAction::Sky(SkyRequest::FollowFirstLight));
        }
    }
    let Some(sky) = edited.sky.as_mut().filter(|sky| sky.enabled) else {
        return changed;
    };

    if let Some(status) = status {
        ui.horizontal_wrapped(|ui| {
            ui.label(format!(
                "{} {} on {} · sun {:+.1}° · moon {:.0}% lit · light: {}",
                if status.running { "Running:" } else { "Now" },
                status.clock,
                status.date,
                status.sun_altitude,
                status.moon_illumination * 100.0,
                status.light
            ));
        });
        ui.horizontal(|ui| {
            let mut preview = status.preview;
            if ui
                .checkbox(&mut preview, "Run in editor")
                .on_hover_text("Run the clock while editing (it always runs in play mode)")
                .changed()
            {
                actions.push(InspectorAction::Sky(SkyRequest::Preview(preview)));
            }
            if ui
                .button("Reset clock")
                .on_hover_text("Back to the date and time set below")
                .clicked()
            {
                actions.push(InspectorAction::Sky(SkyRequest::ResetClock));
            }
            if !status.followed
                && ui
                    .button("Light from sky")
                    .on_hover_text("Make the first directional light the sun and moon")
                    .clicked()
            {
                actions.push(InspectorAction::Sky(SkyRequest::FollowFirstLight));
            }
        });
        if let Some((done, total)) = status.day_bake {
            ui.label(format!(
                "Baking light probes for the day's hours: {done} of {total}"
            ));
        }
    }

    egui::CollapsingHeader::new("Time and place")
        .id_salt("sky_time_place")
        .default_open(true)
        .show(ui, |ui| {
            Grid::new("sky_time_grid")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Date");
                    let mut text = sky.date.to_string();
                    let response =
                        ui.add(egui::TextEdit::singleline(&mut text).desired_width(90.0));
                    if response.changed() {
                        if let Ok(date) = Date::parse(&text) {
                            sky.date = date;
                            changed = true;
                        }
                    }
                    ui.end_row();

                    ui.label("Time (h)");
                    let minutes = (sky.time * 60.0).round() as i32;
                    changed |= ui
                        .add(
                            wgpu_cube::ui::FineSlider::new(&mut sky.time, 0.0..=24.0)
                                .step(1.0 / 60.0),
                        )
                        .on_hover_text(format!(
                            "{:02}:{:02} on the local clock",
                            (minutes / 60) % 24,
                            minutes % 60
                        ))
                        .changed();
                    ui.end_row();

                    ui.label("Day length (s)");
                    changed |= ui
                        .add(
                            DragValue::new(&mut sky.day_length)
                                .speed(1.0)
                                .range(0.0..=86_400.0),
                        )
                        .on_hover_text("Real seconds a day takes while the clock runs; 0 stops it")
                        .changed();
                    ui.end_row();

                    labeled_slider(
                        ui,
                        "Latitude",
                        &mut sky.latitude,
                        -90.0..=90.0,
                        0.1,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "Longitude",
                        &mut sky.longitude,
                        -180.0..=180.0,
                        0.1,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "UTC offset (h)",
                        &mut sky.utc_offset,
                        -12.0..=14.0,
                        0.25,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "North heading",
                        &mut sky.north_heading,
                        0.0..=360.0,
                        1.0,
                        &mut changed,
                    );
                });
        });

    egui::CollapsingHeader::new("Sky look")
        .id_salt("sky_look")
        .default_open(false)
        .show(ui, |ui| {
            Grid::new("sky_look_grid")
                .num_columns(2)
                .striped(true)
                .show(ui, |ui| {
                    labeled_slider(
                        ui,
                        "Sun intensity",
                        &mut sky.sun_intensity,
                        0.0..=50.0,
                        0.1,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "Moon intensity",
                        &mut sky.moon_intensity,
                        0.0..=5.0,
                        0.01,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "Sky intensity",
                        &mut sky.sky_intensity,
                        0.0..=4.0,
                        0.01,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "Air (Rayleigh)",
                        &mut sky.rayleigh,
                        0.0..=4.0,
                        0.01,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "Haze (Mie)",
                        &mut sky.haze,
                        0.0..=20.0,
                        0.05,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "Haze glare",
                        &mut sky.mie_anisotropy,
                        0.0..=0.99,
                        0.01,
                        &mut changed,
                    );
                    labeled_slider(ui, "Ozone", &mut sky.ozone, 0.0..=4.0, 0.01, &mut changed);
                    labeled_slider(
                        ui,
                        "Sun size (°)",
                        &mut sky.sun_size,
                        0.1..=5.0,
                        0.01,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "Moon size (°)",
                        &mut sky.moon_size,
                        0.1..=5.0,
                        0.01,
                        &mut changed,
                    );
                    labeled_slider(ui, "Stars", &mut sky.stars, 0.0..=10.0, 0.05, &mut changed);
                    labeled_slider(
                        ui,
                        "Milky Way",
                        &mut sky.milky_way,
                        0.0..=10.0,
                        0.05,
                        &mut changed,
                    );
                    labeled_slider(
                        ui,
                        "Night glow",
                        &mut sky.night_glow,
                        0.0..=10.0,
                        0.05,
                        &mut changed,
                    );
                    ui.label("Night tint");
                    changed |=
                        egui::widgets::color_picker::color_edit_button_rgb(ui, &mut sky.night_tint)
                            .changed();
                    ui.end_row();
                    ui.label("Ground");
                    changed |= egui::widgets::color_picker::color_edit_button_rgb(
                        ui,
                        &mut sky.ground_albedo,
                    )
                    .changed();
                    ui.end_row();
                });
        });
    changed
}
