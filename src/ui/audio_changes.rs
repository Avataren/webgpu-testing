//! Editors for changes to the mix that audio volumes and mixer snapshots
//! share: per-bus fader offsets, reverb sends and low-passes, and the
//! reverb's character.

use egui::{ComboBox, DragValue, Grid, Ui};

use crate::audio::{AudioVolumeBus, DEFAULT_BUS};

/// What an edit did: `changed` values (apply them), `done` once a drag ends
/// or a discrete change is made (save then).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edit {
    pub changed: bool,
    pub done: bool,
}

impl Edit {
    fn response(&mut self, response: &egui::Response) {
        self.changed |= response.changed();
        self.done |= response.drag_stopped() || (response.changed() && !response.dragged());
    }

    fn discrete(&mut self) {
        self.changed = true;
        self.done = true;
    }

    pub fn merge(&mut self, other: Edit) {
        self.changed |= other.changed;
        self.done |= other.done;
    }
}

/// A combo of the project's buses.
pub fn bus_combo(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug,
    value: &mut String,
    buses: &[String],
) -> bool {
    let mut changed = false;
    ComboBox::from_id_salt(id)
        .selected_text(value.clone())
        .show_ui(ui, |ui| {
            for bus in buses {
                if ui.selectable_label(value == bus, bus).clicked() && value != bus {
                    *value = bus.clone();
                    changed = true;
                }
            }
        });
    changed
}

/// An optional value: a checkbox, and the value's slider while it is set.
fn optional(
    ui: &mut Ui,
    value: &mut Option<f32>,
    default: f32,
    hint: &str,
    slider: impl FnOnce(&mut f32) -> egui::Slider<'_>,
) -> Edit {
    let mut edit = Edit::default();
    ui.horizontal(|ui| {
        let mut on = value.is_some();
        if ui.checkbox(&mut on, "").on_hover_text(hint).changed() {
            *value = on.then_some(default);
            edit.discrete();
        }
        if let Some(v) = value {
            let response = ui.add(slider(v));
            edit.response(&response);
        }
    });
    edit
}

/// Rows of a two-column grid: the reverb's room size and damping, each off
/// (the layout's) or set.
pub fn reverb_rows(ui: &mut Ui, room_size: &mut Option<f32>, damping: &mut Option<f32>) -> Edit {
    let mut edit = Edit::default();
    ui.label("Room size");
    edit.merge(optional(ui, room_size, 0.85, "Off: the layout's", |v| {
        egui::Slider::new(v, 0.0..=1.0)
    }));
    ui.end_row();
    ui.label("Damping");
    edit.merge(optional(ui, damping, 0.5, "Off: the layout's", |v| {
        egui::Slider::new(v, 0.0..=1.0)
    }));
    ui.end_row();
    edit
}

/// The buses a volume or snapshot changes, each with its fader offset,
/// reverb send and low-pass, and a button to add one. `avoid` is a bus not
/// to suggest for a new row (the reverb return).
pub fn bus_changes(
    ui: &mut Ui,
    id: impl std::hash::Hash + std::fmt::Debug + Copy,
    buses: &mut Vec<AudioVolumeBus>,
    names: &[String],
    avoid: &str,
) -> Edit {
    let mut edit = Edit::default();
    let mut remove = None;
    for (i, bus) in buses.iter_mut().enumerate() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            Grid::new(("audio_bus_change", id, i))
                .num_columns(2)
                .show(ui, |ui| {
                    ui.label("Bus");
                    ui.horizontal(|ui| {
                        if bus_combo(ui, ("audio_bus_change_name", id, i), &mut bus.bus, names) {
                            edit.discrete();
                        }
                        if ui.small_button("✕").on_hover_text("Remove").clicked() {
                            remove = Some(i);
                        }
                    });
                    ui.end_row();
                    ui.label("Fader");
                    let response = ui
                        .add(
                            DragValue::new(&mut bus.gain_db)
                                .speed(0.1)
                                .range(-80.0..=24.0)
                                .suffix(" dB"),
                        )
                        .on_hover_text("Added to the bus's fader");
                    edit.response(&response);
                    ui.end_row();
                    ui.label("Reverb send");
                    edit.merge(optional(
                        ui,
                        &mut bus.reverb_db,
                        -6.0,
                        "How much of the bus goes to the reverb",
                        |v| egui::Slider::new(v, -60.0..=6.0).suffix(" dB"),
                    ));
                    ui.end_row();
                    ui.label("Low-pass");
                    edit.merge(optional(
                        ui,
                        &mut bus.low_pass_hz,
                        1_200.0,
                        "Muffle the bus (behind a wall, under water)",
                        |v| {
                            egui::Slider::new(v, 20.0..=20_000.0)
                                .logarithmic(true)
                                .suffix(" Hz")
                        },
                    ));
                    ui.end_row();
                });
        });
    }
    if let Some(i) = remove {
        buses.remove(i);
        edit.discrete();
    }
    if ui.button("+ Add bus").clicked() {
        let unused = names
            .iter()
            .find(|n| !buses.iter().any(|b| &b.bus == *n) && n.as_str() != avoid)
            .cloned()
            .unwrap_or_else(|| DEFAULT_BUS.to_string());
        buses.push(AudioVolumeBus {
            bus: unused,
            ..AudioVolumeBus::default()
        });
        edit.discrete();
    }
    edit
}
