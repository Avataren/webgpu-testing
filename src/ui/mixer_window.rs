//! The mixer window: a strip per bus with its fader, meter, balance, mute and
//! solo. Changes apply to what is playing at once and are saved to the
//! project's `content/audio/mixer.json` when a drag ends or a button is
//! pressed.

use egui::{pos2, vec2, Color32, Rect, Sense, Stroke, StrokeKind, Ui};

use crate::audio::{self, BusState, MASTER};

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
    egui::Window::new("Mixer")
        .open(open)
        .resizable([true, false])
        .default_width((STRIP_WIDTH + 20.0) * buses.len() as f32 + 16.0)
        .show(ctx, |ui| {
            egui::ScrollArea::horizontal().show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for bus in &buses {
                        // A frame, not separators: those stretch to the window's height.
                        egui::Frame::group(ui.style()).show(ui, |ui| {
                            save |= strip(ui, bus);
                        });
                    }
                });
            });
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
fn strip(ui: &mut Ui, bus: &BusState) -> bool {
    let name = bus.layout.name.clone();
    let mut save = false;
    ui.allocate_ui_with_layout(
        vec2(STRIP_WIDTH, FADER_HEIGHT + 90.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.set_width(STRIP_WIDTH);
            let label = egui::RichText::new(&name).strong();
            ui.label(if name == MASTER {
                label.underline()
            } else {
                label
            });

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
        },
    );
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
