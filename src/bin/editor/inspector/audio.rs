//! Inspector sections for sound: an entity's `AudioSource`, `AudioListener`
//! and `AudioVolume`.

use egui::{ComboBox, DragValue, Grid};
use hecs::Entity;
use wgpu_cube::audio::{AudioListener, AudioSource, AudioVolume, RolloffSetting, SpatialSetting};
use wgpu_cube::scene::VolumeShape;
use wgpu_cube::ui::audio_changes as changes;
use wgpu_cube::SceneEntityComponentsSummary;

use super::{float_drag_value, InspectorAction};
use crate::theme::{self, icons};

fn set(
    actions: &mut Vec<InspectorAction>,
    entity: Entity,
    component: &'static str,
    value: impl serde::Serialize,
) {
    match serde_json::to_value(value) {
        // The generic component edit (with undo) also routes audio components.
        Ok(value) => actions.push(InspectorAction::SetPhysics {
            entity,
            component,
            value,
        }),
        Err(err) => log::warn!("Cannot edit {component}: {err}"),
    }
}

fn removable(
    header: &egui::Response,
    entity: Entity,
    component: &'static str,
    label: &str,
    actions: &mut Vec<InspectorAction>,
) {
    header.context_menu(|ui| {
        let text = egui::RichText::new(format!("{}  Remove {label}", icons::DELETE))
            .color(theme::colors::ERROR);
        if ui.button(text).clicked() {
            actions.push(InspectorAction::SetPhysics {
                entity,
                component,
                value: serde_json::Value::Null,
            });
            ui.close();
        }
    });
}

const SPATIAL_CHOICES: [(SpatialSetting, &str, &str); 4] = [
    (
        SpatialSetting::Auto,
        "Auto",
        "2D under an orthographic camera, else 3D",
    ),
    (
        SpatialSetting::Planar,
        "2D",
        "Depth along the view does not count; panned across the screen",
    ),
    (
        SpatialSetting::Full,
        "3D",
        "Distance in space, panned by direction",
    ),
    (
        SpatialSetting::Off,
        "Off",
        "Heard as it is, wherever the entity is",
    ),
];

fn spatial_combo(
    ui: &mut egui::Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut SpatialSetting,
    choices: &[(SpatialSetting, &str, &str)],
) -> bool {
    let mut changed = false;
    let current = choices
        .iter()
        .find(|(s, ..)| s == value)
        .map_or("Auto", |(_, name, _)| name);
    ComboBox::from_id_salt(id)
        .selected_text(current)
        .show_ui(ui, |ui| {
            for (setting, name, hint) in choices {
                if ui
                    .selectable_label(value == setting, *name)
                    .on_hover_text(*hint)
                    .clicked()
                    && value != setting
                {
                    *value = *setting;
                    changed = true;
                }
            }
        });
    changed
}

fn source_section(
    ui: &mut egui::Ui,
    entity: Entity,
    source: &AudioSource,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::AUDIO_SOURCE, "Audio Source", |ui| {
        let mut edited = source.clone();
        let mut changed = false;
        Grid::new(("audio_source_grid", entity))
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Clip");
                changed |= ui
                    .text_edit_singleline(&mut edited.clip)
                    .on_hover_text(
                        "A wav, ogg, mp3 or flac file in the project, e.g. content/audio/hum.wav, or a chip-music song (.ahx, .hvl, .a2m, .asid)",
                    )
                    .lost_focus();
                ui.end_row();
                ui.label("Bus");
                let buses: Vec<String> = wgpu_cube::audio::buses()
                    .into_iter()
                    .map(|b| b.layout.name)
                    .collect();
                ComboBox::from_id_salt(("audio_source_bus", entity))
                    .selected_text(edited.bus.clone())
                    .show_ui(ui, |ui| {
                        for bus in buses {
                            if ui.selectable_label(edited.bus == bus, &bus).clicked()
                                && edited.bus != bus
                            {
                                edited.bus = bus;
                                changed = true;
                            }
                        }
                    });
                ui.end_row();
                changed |=
                    float_drag_value(ui, "Volume", &mut edited.volume, 0.01, Some(0.0..=4.0));
                changed |=
                    float_drag_value(ui, "Pitch", &mut edited.pitch, 0.01, Some(0.01..=16.0));
                ui.label("Loop");
                changed |= ui.checkbox(&mut edited.looped, "").changed();
                ui.end_row();
                ui.label("Play on start");
                changed |= ui
                    .checkbox(&mut edited.play_on_start, "")
                    .on_hover_text("Start when play starts (off: a script calls play_audio_source)")
                    .changed();
                ui.end_row();
                ui.label("Spatial");
                changed |= spatial_combo(
                    ui,
                    ("audio_source_spatial", entity),
                    &mut edited.spatial,
                    &SPATIAL_CHOICES,
                );
                ui.end_row();
                if edited.spatial != SpatialSetting::Off {
                    changed |= float_drag_value(
                        ui,
                        "Min distance",
                        &mut edited.min_distance,
                        0.05,
                        Some(0.01..=1000.0),
                    );
                    changed |= float_drag_value(
                        ui,
                        "Max distance",
                        &mut edited.max_distance,
                        0.1,
                        Some(0.01..=10000.0),
                    );
                    ui.label("Rolloff");
                    let inverse = edited.rolloff == RolloffSetting::Inverse;
                    ComboBox::from_id_salt(("audio_source_rolloff", entity))
                        .selected_text(if inverse { "Inverse" } else { "Linear" })
                        .show_ui(ui, |ui| {
                            if ui
                                .selectable_label(inverse, "Inverse")
                                .on_hover_text("min / distance, as in air")
                                .clicked()
                                && !inverse
                            {
                                edited.rolloff = RolloffSetting::Inverse;
                                changed = true;
                            }
                            if ui
                                .selectable_label(!inverse, "Linear")
                                .on_hover_text("Silent at max distance")
                                .clicked()
                                && inverse
                            {
                                edited.rolloff = RolloffSetting::Linear;
                                changed = true;
                            }
                        });
                    ui.end_row();
                    changed |=
                        float_drag_value(ui, "Doppler", &mut edited.doppler, 0.01, Some(0.0..=4.0));
                }
                ui.label("Priority");
                changed |= ui
                    .add(egui::DragValue::new(&mut edited.priority).range(0..=255))
                    .on_hover_text(
                        "Lower priorities are stolen first when a bus runs out of voices",
                    )
                    .changed();
                ui.end_row();
            });
        if edited.max_distance < edited.min_distance {
            edited.max_distance = edited.min_distance;
        }
        if changed {
            set(actions, entity, "audio_source", &edited);
        }
    })
}

fn listener_section(
    ui: &mut egui::Ui,
    entity: Entity,
    listener: &AudioListener,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::AUDIO_LISTENER, "Audio Listener", |ui| {
        let mut edited = listener.clone();
        let mut changed = false;
        Grid::new(("audio_listener_grid", entity))
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Mode");
                changed |= spatial_combo(
                    ui,
                    ("audio_listener_mode", entity),
                    &mut edited.mode,
                    &SPATIAL_CHOICES[..3],
                );
                ui.end_row();
                changed |= float_drag_value(
                    ui,
                    "2D pan width",
                    &mut edited.pan_width,
                    0.1,
                    Some(0.0..=10000.0),
                );
            });
        ui.label(
            egui::RichText::new("The mix is heard from here instead of the active camera. Pan width 0: half an orthographic camera's view.")
                .small()
                .weak(),
        );
        if changed {
            set(actions, entity, "audio_listener", &edited);
        }
    })
}

/// The project's bus names, for pickers.
fn bus_names() -> Vec<String> {
    wgpu_cube::audio::buses()
        .into_iter()
        .map(|b| b.layout.name)
        .collect()
}

fn distance(value: &mut f32) -> DragValue<'_> {
    DragValue::new(value)
        .speed(0.05)
        .range(0.0..=100_000.0)
        .max_decimals(2)
}

fn volume_section(
    ui: &mut egui::Ui,
    entity: Entity,
    volume: &AudioVolume,
    actions: &mut Vec<InspectorAction>,
) -> egui::Response {
    theme::section(ui, icons::AUDIO_VOLUME, "Audio Volume", |ui| {
        let mut edited = volume.clone();
        let mut changed = false;
        let buses = bus_names();
        let snapshots: Vec<String> = wgpu_cube::audio::update_snapshots(|list| {
            list.iter().map(|s| s.name.clone()).collect()
        });
        Grid::new(("audio_volume_grid", entity))
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Enabled");
                changed |= ui.checkbox(&mut edited.enabled, "").changed();
                ui.end_row();
                ui.label("Extent");
                let current = edited.shape;
                ComboBox::from_id_salt(("audio_volume_shape", entity))
                    .selected_text(current.label())
                    .show_ui(ui, |ui| {
                        for option in [
                            VolumeShape::Infinite,
                            VolumeShape::Box {
                                size: [10.0, 4.0, 10.0],
                            },
                            VolumeShape::Sphere { radius: 5.0 },
                        ] {
                            let selected =
                                std::mem::discriminant(&current) == std::mem::discriminant(&option);
                            if ui.selectable_label(selected, option.label()).clicked() && !selected
                            {
                                edited.shape = option;
                                changed = true;
                            }
                        }
                    });
                ui.end_row();
                match &mut edited.shape {
                    VolumeShape::Infinite => {}
                    VolumeShape::Box { size } => {
                        ui.label("Size");
                        ui.horizontal(|ui| {
                            for value in size.iter_mut() {
                                changed |= ui.add(distance(value)).changed();
                            }
                        });
                        ui.end_row();
                    }
                    VolumeShape::Sphere { radius } => {
                        ui.label("Radius");
                        changed |= ui.add(distance(radius)).changed();
                        ui.end_row();
                    }
                }
                if !edited.shape.is_infinite() {
                    ui.label("Blend distance");
                    changed |= ui
                        .add(distance(&mut edited.blend_distance).suffix(" m"))
                        .on_hover_text("How far outside the shape it fades out")
                        .changed();
                    ui.end_row();
                }
                ui.label("Priority");
                changed |= ui
                    .add(DragValue::new(&mut edited.priority).speed(0.1))
                    .on_hover_text("Higher-priority volumes blend over lower ones")
                    .changed();
                ui.end_row();
                ui.label("Reverb bus");
                changed |= changes::bus_combo(
                    ui,
                    ("audio_volume_reverb_bus", entity),
                    &mut edited.reverb_bus,
                    &buses,
                );
                ui.end_row();
                ui.label("Snapshot");
                let none = "None (the buses below)";
                ComboBox::from_id_salt(("audio_volume_snapshot", entity))
                    .selected_text(edited.snapshot.clone().unwrap_or_else(|| none.into()))
                    .show_ui(ui, |ui| {
                        if ui.selectable_label(edited.snapshot.is_none(), none).clicked()
                            && edited.snapshot.is_some()
                        {
                            edited.snapshot = None;
                            changed = true;
                        }
                        for name in &snapshots {
                            let on = edited.snapshot.as_ref() == Some(name);
                            if ui.selectable_label(on, name).clicked() && !on {
                                edited.snapshot = Some(name.clone());
                                changed = true;
                            }
                        }
                    })
                    .response
                    .on_hover_text("Sound like one of the mixer's snapshots instead of listing buses here (edit them in Window > Mixer)");
                ui.end_row();
                if edited.snapshot.is_none() {
                    changed |= changes::reverb_rows(
                        ui,
                        &mut edited.reverb_room_size,
                        &mut edited.reverb_damping,
                    )
                    .changed;
                }
            });

        if edited.snapshot.is_none() {
            ui.label(egui::RichText::new("Buses").small().strong());
            let reverb = edited.reverb_bus.clone();
            changed |= changes::bus_changes(
                ui,
                ("audio_volume", entity),
                &mut edited.buses,
                &buses,
                &reverb,
            )
            .changed;
        }
        ui.label(
            egui::RichText::new("While the listener is inside (in play mode): buses send to the reverb, are muffled or turned down. Buses not listed are left alone.")
                .small()
                .weak(),
        );
        if changed {
            set(actions, entity, "audio_volume", &edited);
        }
    })
}

pub(super) fn show_audio_sections(
    ui: &mut egui::Ui,
    entity: Entity,
    components: &SceneEntityComponentsSummary,
    actions: &mut Vec<InspectorAction>,
) {
    if let Some(source) = &components.audio_source {
        let header = source_section(ui, entity, source, actions);
        removable(&header, entity, "audio_source", "Audio Source", actions);
    }
    if let Some(listener) = &components.audio_listener {
        let header = listener_section(ui, entity, listener, actions);
        removable(&header, entity, "audio_listener", "Audio Listener", actions);
    }
    if let Some(volume) = &components.audio_volume {
        let header = volume_section(ui, entity, volume, actions);
        removable(&header, entity, "audio_volume", "Audio Volume", actions);
    }
}

pub(super) fn add_audio_menu(
    ui: &mut egui::Ui,
    entity: Entity,
    components: &SceneEntityComponentsSummary,
    actions: &mut Vec<InspectorAction>,
) {
    ui.menu_button(format!("{}  Audio", icons::AUDIO_SOURCE), |ui| {
        let mut add =
            |ui: &mut egui::Ui, enabled: bool, label: &str, component, value: serde_json::Value| {
                if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                    actions.push(InspectorAction::SetPhysics {
                        entity,
                        component,
                        value,
                    });
                    ui.close();
                }
            };
        add(
            ui,
            components.audio_source.is_none(),
            "Audio Source",
            "audio_source",
            serde_json::to_value(AudioSource::default()).unwrap_or_default(),
        );
        add(
            ui,
            components.audio_listener.is_none(),
            "Audio Listener",
            "audio_listener",
            serde_json::to_value(AudioListener::default()).unwrap_or_default(),
        );
        add(
            ui,
            components.audio_volume.is_none(),
            "Audio Volume",
            "audio_volume",
            serde_json::to_value(AudioVolume::default()).unwrap_or_default(),
        );
    });
}
