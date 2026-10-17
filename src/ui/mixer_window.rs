//! The mixer window: a strip per bus with its fader, meter, balance, mute and
//! solo, and below them the selected bus's output, effects, sends and
//! ducking. Changes apply to what is playing at once and are saved to the
//! project's `content/audio/mixer.json` when a drag ends or a button is
//! pressed.

use cubed_mixer::EffectKind;
use egui::{pos2, vec2, Color32, Rect, Sense, Stroke, StrokeKind, Ui};

use super::audio_changes::{self as changes, Edit};
use crate::audio::{
    self, BusState, DuckLayout, EffectLayout, SendLayout, SnapshotLayout, MASTER, MAX_EFFECTS,
    MAX_SENDS, REVERB_BUS,
};

/// The fader's travel; its bottom stop is silence.
const FADER_MIN_DB: f32 = -60.0;
const FADER_MAX_DB: f32 = 12.0;
/// The meter's scale, in dBFS.
const METER_MIN_DB: f32 = -60.0;
const METER_MAX_DB: f32 = 6.0;
const STRIP_WIDTH: f32 = 70.0;
const FADER_HEIGHT: f32 = 170.0;

/// Shows the window while `open` is set.
pub fn show_mixer_window(ctx: &egui::Context, open: &mut bool) {
    if !*open {
        return;
    }
    let buses = audio::buses();
    let mut save = false;
    let status_id = egui::Id::new("mixer_window_status");
    let selected_id = egui::Id::new("mixer_window_selected");
    let mut selected: String = ctx
        .data(|d| d.get_temp(selected_id))
        .unwrap_or_else(|| MASTER.to_string());
    if !buses.iter().any(|b| b.layout.name == selected) {
        selected = MASTER.to_string();
    }
    egui::Window::new("Mixer")
        .open(open)
        .resizable([true, false])
        .default_width((STRIP_WIDTH + 20.0) * buses.len() as f32 + 16.0)
        .show(ctx, |ui| {
            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for bus in &buses {
                        // A frame, not separators: those stretch to the window's height.
                        let mut frame = egui::Frame::group(ui.style());
                        if bus.layout.name == selected {
                            frame =
                                frame.stroke(Stroke::new(1.5, ui.visuals().selection.stroke.color));
                        }
                        frame.show(ui, |ui| {
                            save |= strip(ui, bus, &mut selected);
                        });
                    }
                });
            });
            ui.separator();
            if let Some(bus) = buses.iter().find(|b| b.layout.name == selected) {
                save |= details(ui, bus, &buses);
                ui.separator();
            }
            save |= snapshots(ui, &buses);
            ui.separator();
            let status: Option<String> = ui.data(|d| d.get_temp(status_id));
            ui.label(
                egui::RichText::new(status.unwrap_or_else(|| {
                    format!("Saved with the project in {}", audio::LAYOUT_FILE)
                }))
                .small()
                .weak(),
            );
        });
    ctx.data_mut(|d| d.insert_temp(selected_id, selected));
    if save {
        let status = match audio::save_layout() {
            Ok(path) => format!("Saved {}", path.display()),
            Err(err) => format!("Not saved: {err}"),
        };
        ctx.data_mut(|d| d.insert_temp(status_id, status));
    }
    // The meters move while sound plays.
    ctx.request_repaint_after(std::time::Duration::from_millis(33));
}

/// One bus's strip. Returns whether a change should be saved.
fn strip(ui: &mut Ui, bus: &BusState, selected: &mut String) -> bool {
    let name = bus.layout.name.clone();
    let mut save = false;
    ui.allocate_ui_with_layout(
        vec2(STRIP_WIDTH, FADER_HEIGHT + 114.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.set_width(STRIP_WIDTH);
            let label = egui::RichText::new(&name).strong();
            let label = if name == MASTER {
                label.underline()
            } else {
                label
            };
            if ui
                .add(egui::Label::new(label).sense(Sense::click()))
                .on_hover_text("Show the bus's effects, sends and ducking")
                .clicked()
            {
                *selected = name.clone();
            }

            ui.horizontal(|ui| {
                // A vertical slider's length is the spacing's slider width.
                ui.spacing_mut().slider_width = FADER_HEIGHT;
                let mut db = bus.layout.gain_db.max(FADER_MIN_DB);
                let fader = ui
                    .add_sized(
                        vec2(22.0, FADER_HEIGHT),
                        egui::Slider::new(&mut db, FADER_MIN_DB..=FADER_MAX_DB)
                            .vertical()
                            .show_value(false),
                    )
                    .on_hover_text("Fader; double-click for 0 dB");
                let mut changed = fader.changed();
                if fader.double_clicked() {
                    db = 0.0;
                    changed = true;
                }
                if changed {
                    let gain_db = if db <= FADER_MIN_DB {
                        cubed_mixer::MIN_DB
                    } else {
                        db
                    };
                    let _ = audio::update_bus(&name, |b| b.gain_db = gain_db);
                }
                save |= fader.drag_stopped() || (changed && !fader.dragged());
                meter(ui, bus);
            });

            let db = bus.layout.gain_db;
            ui.label(if db <= FADER_MIN_DB {
                "-inf dB".to_string()
            } else {
                format!("{db:+.1} dB")
            });

            ui.spacing_mut().slider_width = STRIP_WIDTH - 8.0;
            let mut pan = bus.layout.pan;
            let balance = ui
                .add_sized(
                    vec2(STRIP_WIDTH - 8.0, 16.0),
                    egui::Slider::new(&mut pan, -1.0..=1.0).show_value(false),
                )
                .on_hover_text(format!("Balance {pan:+.2}; double-click to centre"));
            let mut pan_changed = balance.changed();
            if balance.double_clicked() {
                pan = 0.0;
                pan_changed = true;
            }
            if pan_changed {
                let _ = audio::update_bus(&name, |b| b.pan = pan);
            }
            save |= balance.drag_stopped() || (pan_changed && !balance.dragged());

            ui.horizontal(|ui| {
                let mute = ui
                    .selectable_label(bus.layout.mute, "M")
                    .on_hover_text("Mute");
                if mute.clicked() {
                    let _ = audio::update_bus(&name, |b| b.mute = !b.mute);
                    save = true;
                }
                if name != MASTER {
                    let solo = ui
                        .selectable_label(bus.layout.solo, "S")
                        .on_hover_text("Solo: hear only soloed buses");
                    if solo.clicked() {
                        let _ = audio::update_bus(&name, |b| b.solo = !b.solo);
                        save = true;
                    }
                }
            });
            let effects = bus.layout.effects.len();
            let mut text = if effects == 0 {
                "FX".to_string()
            } else {
                format!("FX {effects}")
            };
            if bus.layout.duck.is_some() {
                text.push_str(" ↓");
            }
            if ui
                .selectable_label(*selected == name, text)
                .on_hover_text(strip_summary(bus))
                .clicked()
            {
                *selected = name.clone();
            }
        },
    );
    save
}

fn strip_summary(bus: &BusState) -> String {
    let mut lines = Vec::new();
    for effect in &bus.layout.effects {
        lines.push(format!(
            "{}{}",
            effect_label(&effect.kind),
            if effect.bypass { " (bypassed)" } else { "" }
        ));
    }
    for send in &bus.layout.sends {
        lines.push(format!("→ {} {:+.1} dB", send.to, send.level_db));
    }
    if let Some(duck) = &bus.layout.duck {
        lines.push(format!("Ducks {:.0} dB under {}", duck.depth_db, duck.key));
    }
    if lines.is_empty() {
        "No effects, sends or ducking: click to add".into()
    } else {
        lines.join("\n")
    }
}

/// "low_pass" as "Low pass".
fn effect_label(kind: &str) -> String {
    let words = kind.replace('_', " ");
    let mut chars = words.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => words,
    }
}

/// The selected bus's output, effects, sends and ducking. Returns whether a
/// change should be saved.
fn details(ui: &mut Ui, bus: &BusState, buses: &[BusState]) -> bool {
    let name = bus.layout.name.clone();
    let mut save = false;
    let others: Vec<String> = buses
        .iter()
        .map(|b| b.layout.name.clone())
        .filter(|n| *n != name)
        .collect();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&name).strong());
        if name != MASTER {
            ui.label("feeds");
            let current = bus.layout.output.clone().unwrap_or_else(|| MASTER.into());
            egui::ComboBox::from_id_salt(("mixer_output", &name))
                .selected_text(&current)
                .show_ui(ui, |ui| {
                    for other in &others {
                        if ui.selectable_label(*other == current, other).clicked()
                            && *other != current
                        {
                            let output = (other != MASTER).then(|| other.clone());
                            if audio::update_bus(&name, |b| b.output = output).is_ok() {
                                save = true;
                            }
                        }
                    }
                });
        }
    });

    ui.label(egui::RichText::new("Effects").small().strong());
    let count = bus.layout.effects.len();
    for (slot, effect) in bus.layout.effects.iter().enumerate() {
        save |= effect_row(ui, &name, slot, count, effect);
    }
    ui.add_enabled_ui(count < MAX_EFFECTS, |ui| {
        ui.menu_button("+ Add effect", |ui| {
            for kind in EffectKind::ALL {
                if ui.button(effect_label(kind.name())).clicked() {
                    let mut effect = EffectLayout::new(kind);
                    if name == REVERB_BUS && kind == EffectKind::Reverb {
                        // A return carries only the reverb.
                        effect.params.insert("wet".into(), 1.0);
                        effect.params.insert("dry".into(), 0.0);
                    }
                    let _ = audio::update_bus(&name, |b| b.effects.push(effect));
                    save = true;
                    ui.close();
                }
            }
        });
    });

    ui.add_space(4.0);
    ui.label(egui::RichText::new("Sends").small().strong());
    for (slot, send) in bus.layout.sends.iter().enumerate() {
        save |= send_row(ui, &name, slot, send, &others);
    }
    let targets: Vec<&String> = others.iter().filter(|o| *o != MASTER).collect();
    ui.add_enabled_ui(
        bus.layout.sends.len() < MAX_SENDS && !targets.is_empty(),
        |ui| {
            if ui.button("+ Add send").clicked() {
                let to = targets
                    .iter()
                    .find(|t| t.as_str() == REVERB_BUS)
                    .or(targets.first())
                    .map(|t| t.to_string())
                    .unwrap_or_default();
                let _ = audio::update_bus(&name, |b| {
                    b.sends.push(SendLayout {
                        to,
                        level_db: -12.0,
                        pre_fader: false,
                    })
                });
                save = true;
            }
        },
    );

    if name != MASTER {
        ui.add_space(4.0);
        save |= duck_row(ui, &name, bus.layout.duck.as_ref(), &others);
    }
    save
}

fn effect_row(ui: &mut Ui, bus: &str, slot: usize, count: usize, effect: &EffectLayout) -> bool {
    let mut save = false;
    let kind = EffectKind::from_name(&effect.kind);
    let id = ui.make_persistent_id(("mixer_effect", bus, slot));
    egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
        .show_header(ui, |ui| {
            let mut on = !effect.bypass;
            if ui
                .checkbox(&mut on, effect_label(&effect.kind))
                .on_hover_text("Off: bypassed")
                .changed()
            {
                let _ = audio::update_bus(bus, |b| b.effects[slot].bypass = !on);
                save = true;
            }
            if slot > 0 && ui.small_button("⏶").on_hover_text("Earlier").clicked() {
                let _ = audio::update_bus(bus, |b| b.effects.swap(slot, slot - 1));
                save = true;
            }
            if slot + 1 < count && ui.small_button("⏷").on_hover_text("Later").clicked() {
                let _ = audio::update_bus(bus, |b| b.effects.swap(slot, slot + 1));
                save = true;
            }
            if ui.small_button("✕").on_hover_text("Remove").clicked() {
                let _ = audio::update_bus(bus, |b| {
                    b.effects.remove(slot);
                });
                save = true;
            }
        })
        .body(|ui| {
            let Some(kind) = kind else {
                ui.label("Unknown effect");
                return;
            };
            egui::Grid::new(("mixer_effect_params", bus, slot))
                .num_columns(2)
                .show(ui, |ui| {
                    for info in kind.params() {
                        let mut value = effect
                            .params
                            .get(info.name)
                            .copied()
                            .unwrap_or(info.default);
                        ui.label(
                            effect_label(info.name)
                                .replace(" hz", "")
                                .replace(" ms", "")
                                .replace(" db", ""),
                        );
                        let slider = egui::Slider::new(&mut value, info.min..=info.max)
                            .logarithmic(matches!(info.unit, "Hz" | "ms") && info.min > 0.0)
                            .suffix(if info.unit.is_empty() {
                                String::new()
                            } else {
                                format!(" {}", info.unit)
                            });
                        let response = ui.add(slider);
                        if response.changed() {
                            let name = info.name.to_string();
                            let _ = audio::update_bus(bus, |b| {
                                b.effects[slot].params.insert(name, value);
                            });
                        }
                        save |=
                            response.drag_stopped() || (response.changed() && !response.dragged());
                        ui.end_row();
                    }
                });
        });
    save
}

fn send_row(ui: &mut Ui, bus: &str, slot: usize, send: &SendLayout, others: &[String]) -> bool {
    let mut save = false;
    ui.horizontal(|ui| {
        ui.label("→");
        egui::ComboBox::from_id_salt(("mixer_send_to", bus, slot))
            .selected_text(&send.to)
            .width(90.0)
            .show_ui(ui, |ui| {
                for other in others.iter().filter(|o| *o != MASTER) {
                    if ui.selectable_label(*other == send.to, other).clicked() && *other != send.to
                    {
                        let to = other.clone();
                        let _ = audio::update_bus(bus, |b| b.sends[slot].to = to);
                        save = true;
                    }
                }
            });
        let mut level = send.level_db;
        let response = ui.add(
            egui::Slider::new(&mut level, -60.0..=6.0)
                .suffix(" dB")
                .fixed_decimals(1),
        );
        if response.changed() {
            let _ = audio::update_bus(bus, |b| b.sends[slot].level_db = level);
        }
        save |= response.drag_stopped() || (response.changed() && !response.dragged());
        let mut pre = send.pre_fader;
        if ui
            .checkbox(&mut pre, "pre")
            .on_hover_text("Taken before the bus's fader")
            .changed()
        {
            let _ = audio::update_bus(bus, |b| b.sends[slot].pre_fader = pre);
            save = true;
        }
        if ui.small_button("✕").on_hover_text("Remove").clicked() {
            let _ = audio::update_bus(bus, |b| {
                b.sends.remove(slot);
            });
            save = true;
        }
    });
    save
}

fn duck_row(ui: &mut Ui, bus: &str, duck: Option<&DuckLayout>, others: &[String]) -> bool {
    let mut save = false;
    let mut on = duck.is_some();
    ui.horizontal(|ui| {
        if ui
            .checkbox(&mut on, "Duck under")
            .on_hover_text(
                "Turn this bus down while another is loud (music under effects or dialogue)",
            )
            .changed()
        {
            let key = others
                .iter()
                .find(|o| o.as_str() == "voice" || o.as_str() == audio::DEFAULT_BUS)
                .or_else(|| others.iter().find(|o| o.as_str() != MASTER))
                .cloned();
            let _ = audio::update_bus(bus, |b| {
                b.duck = match (on, key) {
                    (true, Some(key)) => Some(DuckLayout {
                        key,
                        ..DuckLayout::default()
                    }),
                    _ => None,
                }
            });
            save = true;
        }
        if let Some(duck) = duck {
            egui::ComboBox::from_id_salt(("mixer_duck_key", bus))
                .selected_text(&duck.key)
                .width(90.0)
                .show_ui(ui, |ui| {
                    for other in others.iter().filter(|o| *o != MASTER) {
                        if ui.selectable_label(*other == duck.key, other).clicked()
                            && *other != duck.key
                        {
                            let key = other.clone();
                            let _ = audio::update_bus(bus, |b| {
                                if let Some(d) = &mut b.duck {
                                    d.key = key;
                                }
                            });
                            save = true;
                        }
                    }
                });
        }
    });
    let Some(duck) = duck else {
        return save;
    };
    egui::Grid::new(("mixer_duck", bus))
        .num_columns(2)
        .show(ui, |ui| {
            let mut row = |ui: &mut Ui,
                           label: &str,
                           value: f32,
                           range: std::ops::RangeInclusive<f32>,
                           unit: &str,
                           apply: fn(&mut DuckLayout, f32)| {
                ui.label(label);
                let mut v = value;
                let response = ui.add(
                    egui::Slider::new(&mut v, range)
                        .suffix(format!(" {unit}"))
                        .logarithmic(unit == "ms"),
                );
                if response.changed() {
                    let _ = audio::update_bus(bus, |b| {
                        if let Some(d) = &mut b.duck {
                            apply(d, v);
                        }
                    });
                }
                save |= response.drag_stopped() || (response.changed() && !response.dragged());
                ui.end_row();
            };
            row(ui, "Depth", duck.depth_db, 0.0..=40.0, "dB", |d, v| {
                d.depth_db = v
            });
            row(
                ui,
                "Threshold",
                duck.threshold_db,
                -80.0..=0.0,
                "dB",
                |d, v| d.threshold_db = v,
            );
            row(ui, "Attack", duck.attack_ms, 1.0..=500.0, "ms", |d, v| {
                d.attack_ms = v
            });
            row(
                ui,
                "Release",
                duck.release_ms,
                10.0..=4000.0,
                "ms",
                |d, v| d.release_ms = v,
            );
        });
    save
}

/// Left and right bars: the RMS filled, the peak as a line.
fn meter(ui: &mut Ui, bus: &BusState) {
    let (rect, response) = ui.allocate_exact_size(vec2(16.0, FADER_HEIGHT), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2.0, Color32::from_gray(24));
    let (Some(peak), Some(rms)) = (bus.peak, bus.rms) else {
        response.on_hover_text("No meter until sound plays");
        return;
    };
    let level = |amplitude: f32| {
        let db = if amplitude > 1e-6 {
            20.0 * amplitude.log10()
        } else {
            METER_MIN_DB
        };
        ((db - METER_MIN_DB) / (METER_MAX_DB - METER_MIN_DB)).clamp(0.0, 1.0)
    };
    let zero_db = level(1.0);
    let half = rect.width() / 2.0;
    for channel in 0..2 {
        let x0 = rect.left() + half * channel as f32 + 1.0;
        let column = |t: f32| {
            Rect::from_min_max(
                pos2(x0, rect.bottom() - t * rect.height()),
                pos2(x0 + half - 2.0, rect.bottom()),
            )
        };
        let rms_level = level(rms[channel]);
        let color = if peak[channel] >= 1.0 {
            Color32::from_rgb(230, 70, 60)
        } else if rms_level > level(10f32.powf(-12.0 / 20.0)) {
            Color32::from_rgb(220, 190, 60)
        } else {
            Color32::from_rgb(80, 190, 110)
        };
        painter.rect_filled(column(rms_level), 0.0, color);
        let y = rect.bottom() - level(peak[channel]) * rect.height();
        painter.line_segment(
            [pos2(x0, y), pos2(x0 + half - 2.0, y)],
            Stroke::new(1.5, Color32::from_gray(235)),
        );
    }
    let y0 = rect.bottom() - zero_db * rect.height();
    painter.line_segment(
        [pos2(rect.left(), y0), pos2(rect.right(), y0)],
        Stroke::new(1.0, Color32::from_gray(110)),
    );
    painter.rect_stroke(
        rect,
        2.0,
        Stroke::new(1.0, Color32::from_gray(60)),
        StrokeKind::Inside,
    );
    let db = |a: f32| {
        if a > 1e-6 {
            format!("{:.1}", 20.0 * a.log10())
        } else {
            "-inf".into()
        }
    };
    response.on_hover_text(format!(
        "Peak {} / {} dBFS\nRMS {} / {} dBFS",
        db(peak[0]),
        db(peak[1]),
        db(rms[0]),
        db(rms[1])
    ));
}

/// The project's snapshots: named changes to the mix that scripts fade to
/// (`set_audio_snapshot`) and audio volumes can name. Returns whether a
/// change should be saved.
fn snapshots(ui: &mut Ui, buses: &[BusState]) -> bool {
    let mut list = audio::update_snapshots(|list| list.clone());
    let names: Vec<String> = buses.iter().map(|b| b.layout.name.clone()).collect();
    let active = audio::snapshot();
    let mut edit = Edit::default();
    let mut remove = None;
    egui::CollapsingHeader::new(format!("Snapshots ({})", list.len()))
        .id_salt("mixer_snapshots")
        .show(ui, |ui| {
            for (i, snapshot) in list.iter_mut().enumerate() {
                let id = ui.make_persistent_id(("mixer_snapshot", i));
                egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, false)
                    .show_header(ui, |ui| {
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut snapshot.name).desired_width(110.0),
                        );
                        edit.changed |= response.changed();
                        edit.done |= response.lost_focus();
                        if active.as_deref() == Some(snapshot.name.as_str()) {
                            ui.label(egui::RichText::new("on").small().strong());
                        }
                        if ui.small_button("✕").on_hover_text("Remove").clicked() {
                            remove = Some(i);
                        }
                    })
                    .body(|ui| {
                        egui::Grid::new(("mixer_snapshot_reverb", i))
                            .num_columns(2)
                            .show(ui, |ui| {
                                edit.merge(changes::reverb_rows(
                                    ui,
                                    &mut snapshot.reverb_room_size,
                                    &mut snapshot.reverb_damping,
                                ));
                            });
                        edit.merge(changes::bus_changes(
                            ui,
                            ("mixer_snapshot_buses", i),
                            &mut snapshot.buses,
                            &names,
                            REVERB_BUS,
                        ));
                    });
            }
            if ui.button("+ Add snapshot").clicked() {
                let mut n = list.len() + 1;
                while list.iter().any(|s| s.name == format!("snapshot {n}")) {
                    n += 1;
                }
                list.push(SnapshotLayout {
                    name: format!("snapshot {n}"),
                    ..SnapshotLayout::default()
                });
                edit.changed = true;
                edit.done = true;
            }
            ui.label(
                egui::RichText::new("Scripts fade to one with set_audio_snapshot(name, seconds) in play mode; an Audio Volume can name one.")
                    .small()
                    .weak(),
            );
        });
    if let Some(i) = remove {
        list.remove(i);
        edit.changed = true;
        edit.done = true;
    }
    if edit.changed {
        audio::update_snapshots(|snapshots| *snapshots = list);
    }
    edit.done
}
