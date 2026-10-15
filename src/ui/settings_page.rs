//! The engine's graphics settings page and FPS readout, drawn over every app's UI (see
//! [`crate::graphics_settings`]).

use crate::graphics_settings::{
    graphics_settings, DisplayMode, EffectQuality, GlobalIllumination, GraphicsCapabilities,
    GraphicsSettings,
};
use crate::renderer::postprocess::{AntiAliasing, Upscaler};

/// The FPS readout's colour.
const FPS_COLOR: egui::Color32 = egui::Color32::from_rgb(255, 221, 0);
/// How often the readout updates.
const FPS_INTERVAL_SECONDS: f64 = 0.5;

/// Frames per second, averaged over half a second so the number is readable.
pub struct FpsCounter {
    start: Option<instant::Instant>,
    frames: u32,
    fps: Option<f64>,
}

impl Default for FpsCounter {
    fn default() -> Self {
        Self::new()
    }
}

impl FpsCounter {
    pub fn new() -> Self {
        Self {
            start: None,
            frames: 0,
            fps: None,
        }
    }

    /// Count a frame; returns the latest average.
    pub fn tick(&mut self) -> Option<f64> {
        let now = instant::Instant::now();
        let start = *self.start.get_or_insert(now);
        self.frames += 1;
        let elapsed = now.duration_since(start).as_secs_f64();
        if elapsed >= FPS_INTERVAL_SECONDS {
            self.fps = Some(f64::from(self.frames) / elapsed);
            self.frames = 0;
            self.start = Some(now);
        }
        self.fps
    }
}

/// What the engine draws over an app's UI: the FPS readout in the corner of `view` (the
/// rendered view, in points) and the settings page when it is open, which Escape toggles
/// while the hotkey is enabled (it can also be opened from code or a menu).
pub fn show_engine_overlays(ctx: &egui::Context, view: egui::Rect, fps: &mut FpsCounter) {
    let fps = fps.tick();
    let (settings, capabilities, mut open, hotkey, layout) = {
        let graphics = graphics_settings();
        (
            graphics.settings.clone(),
            graphics.capabilities.clone(),
            graphics.open,
            graphics.hotkey_enabled,
            PageLayout {
                manage_window: graphics.manage_window,
                exit_button: graphics.exit_button && !cfg!(target_arch = "wasm32"),
            },
        )
    };

    if settings.show_fps {
        if let Some(fps) = fps {
            draw_fps(ctx, view, fps);
        }
    }

    if hotkey && ctx.input(|input| input.key_pressed(egui::Key::Escape)) {
        open = !open;
    }
    let mut edited = settings.clone();
    if open {
        show_page(ctx, &mut edited, &capabilities, layout, &mut open);
    }

    let mut graphics = graphics_settings();
    graphics.open = open;
    if edited != settings {
        graphics.set(edited);
    }
}

fn draw_fps(ctx: &egui::Context, view: egui::Rect, fps: f64) {
    let painter = ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("engine_fps_readout"),
    ));
    let text = format!("{fps:.0} FPS");
    let font = egui::FontId::monospace(16.0);
    let position = view.min + egui::vec2(10.0, 8.0);
    // A dark shadow keeps the text readable over bright scenes.
    painter.text(
        position + egui::vec2(1.0, 1.0),
        egui::Align2::LEFT_TOP,
        &text,
        font.clone(),
        egui::Color32::from_black_alpha(200),
    );
    painter.text(position, egui::Align2::LEFT_TOP, text, font, FPS_COLOR);
}

/// Which of the app-dependent parts the page shows.
#[derive(Clone, Copy)]
struct PageLayout {
    /// Display mode and window size.
    manage_window: bool,
    exit_button: bool,
}

fn show_page(
    ctx: &egui::Context,
    settings: &mut GraphicsSettings,
    capabilities: &GraphicsCapabilities,
    layout: PageLayout,
    open: &mut bool,
) {
    let mut keep_open = true;
    egui::Window::new("Settings")
        .id(egui::Id::new("engine_settings_page"))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .default_width(420.0)
        // Opaque: the page often sits over a busy, bright scene.
        .frame(egui::Frame::window(&ctx.global_style()).fill(ctx.global_style().visuals.panel_fill))
        .open(&mut keep_open)
        .show(ctx, |ui| {
            if layout.manage_window && !cfg!(target_arch = "wasm32") {
                section(ui, "Display", |ui| {
                    display_rows(ui, ctx, settings, capabilities)
                });
            } else if capabilities.vsync_optional {
                section(ui, "Display", |ui| vsync_row(ui, settings, capabilities));
            }
            section(ui, "Quality", |ui| quality_rows(ui, settings, capabilities));
            section(ui, "Ray tracing", |ui| {
                ray_tracing_rows(ui, settings, capabilities)
            });
            section(ui, "Path tracing", |ui| {
                path_tracing_rows(ui, settings, capabilities)
            });
            if !capabilities.ray_tracing {
                ui.label(
                    egui::RichText::new(
                        "No hardware ray tracing: ray-traced effects use their raster versions.",
                    )
                    .small()
                    .weak(),
                );
            }
            section(ui, "Interface", |ui| {
                ui.label("Show FPS");
                ui.checkbox(&mut settings.show_fps, "");
                ui.end_row();
            });

            ui.separator();
            ui.horizontal(|ui| {
                if ui.button("Reset to defaults").clicked() {
                    *settings = GraphicsSettings::default();
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if layout.exit_button && ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                    if ui.button("Close").clicked() {
                        *open = false;
                    }
                });
            });
        });
    if !keep_open {
        *open = false;
    }
}

fn section(ui: &mut egui::Ui, title: &str, rows: impl FnOnce(&mut egui::Ui)) {
    ui.add_space(4.0);
    ui.label(egui::RichText::new(title).strong());
    egui::Grid::new(("engine_settings", title))
        .num_columns(2)
        .spacing([24.0, 6.0])
        .min_col_width(150.0)
        .show(ui, rows);
}

fn display_rows(
    ui: &mut egui::Ui,
    ctx: &egui::Context,
    settings: &mut GraphicsSettings,
    capabilities: &GraphicsCapabilities,
) {
    ui.label("Display mode");
    egui::ComboBox::from_id_salt("engine_settings_display_mode")
        .selected_text(match settings.display_mode {
            DisplayMode::Windowed => "Windowed",
            DisplayMode::Fullscreen => "Fullscreen",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(
                &mut settings.display_mode,
                DisplayMode::Windowed,
                "Windowed",
            );
            ui.selectable_value(
                &mut settings.display_mode,
                DisplayMode::Fullscreen,
                "Fullscreen",
            );
        });
    ui.end_row();

    ui.label("Window size");
    ui.add_enabled_ui(settings.display_mode == DisplayMode::Windowed, |ui| {
        // Sizes that fit the monitor (all of them when it is unknown).
        let monitor = ctx.input(|input| {
            input
                .viewport()
                .monitor_size
                .map(|size| size * input.pixels_per_point)
        });
        let fits = |size: &[u32; 2]| {
            monitor.is_none_or(|monitor| size[0] as f32 <= monitor.x && size[1] as f32 <= monitor.y)
        };
        let label = |size: Option<[u32; 2]>| match size {
            Some([width, height]) => format!("{width} x {height}"),
            None => "As started".to_owned(),
        };
        egui::ComboBox::from_id_salt("engine_settings_window_size")
            .selected_text(label(settings.window_size))
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut settings.window_size, None, label(None));
                for size in GraphicsSettings::WINDOW_SIZES
                    .iter()
                    .filter(|size| fits(size))
                {
                    ui.selectable_value(&mut settings.window_size, Some(*size), label(Some(*size)));
                }
            });
    });
    ui.end_row();

    vsync_row(ui, settings, capabilities);
}

fn vsync_row(
    ui: &mut egui::Ui,
    settings: &mut GraphicsSettings,
    capabilities: &GraphicsCapabilities,
) {
    ui.label("VSync");
    ui.add_enabled(
        capabilities.vsync_optional,
        egui::Checkbox::without_text(&mut settings.vsync),
    )
    .on_disabled_hover_text("The display only offers vsync");
    ui.end_row();
}

fn quality_rows(
    ui: &mut egui::Ui,
    settings: &mut GraphicsSettings,
    capabilities: &GraphicsCapabilities,
) {
    ui.label("Anti-aliasing");
    egui::ComboBox::from_id_salt("engine_settings_anti_aliasing")
        .selected_text(match settings.anti_aliasing {
            AntiAliasing::None => "Off",
            AntiAliasing::Fxaa => "FXAA",
            AntiAliasing::Taa => "TAA",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut settings.anti_aliasing, AntiAliasing::None, "Off");
            ui.selectable_value(&mut settings.anti_aliasing, AntiAliasing::Fxaa, "FXAA");
            ui.selectable_value(&mut settings.anti_aliasing, AntiAliasing::Taa, "TAA");
        });
    ui.end_row();

    let upscaling = capabilities.upscaling;
    ui.label("Render resolution");
    ui.add_enabled_ui(upscaling, |ui| {
        let preset = GraphicsSettings::RENDER_SCALE_PRESETS
            .iter()
            .find(|(_, scale)| (scale - settings.render_scale).abs() < 0.005);
        let selected = match preset {
            Some((name, scale)) => format!("{name} ({:.0}%)", scale * 100.0),
            None => format!("Custom ({:.0}%)", settings.render_scale * 100.0),
        };
        egui::ComboBox::from_id_salt("engine_settings_render_scale")
            .selected_text(selected)
            .show_ui(ui, |ui| {
                for (name, scale) in GraphicsSettings::RENDER_SCALE_PRESETS {
                    ui.selectable_value(
                        &mut settings.render_scale,
                        scale,
                        format!("{name} ({:.0}%)", scale * 100.0),
                    );
                }
            });
    })
    .response
    .on_disabled_hover_text("Upscaling needs MSAA off");
    ui.end_row();

    let scaled = upscaling && settings.render_scale < 0.999;
    ui.label("Upscaler");
    ui.add_enabled_ui(scaled, |ui| {
        egui::ComboBox::from_id_salt("engine_settings_upscaler")
            .selected_text(match settings.upscaler {
                Upscaler::Fsr => "AMD FSR 3.1",
                Upscaler::Taau => "TAAU",
            })
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut settings.upscaler, Upscaler::Fsr, "AMD FSR 3.1");
                ui.selectable_value(&mut settings.upscaler, Upscaler::Taau, "TAAU");
            });
    });
    ui.end_row();

    ui.label("FSR sharpness");
    ui.add_enabled(
        scaled && settings.upscaler == Upscaler::Fsr,
        egui::Slider::new(&mut settings.sharpness, 0.0..=1.0).fixed_decimals(2),
    );
    ui.end_row();
}

fn ray_tracing_rows(
    ui: &mut egui::Ui,
    settings: &mut GraphicsSettings,
    capabilities: &GraphicsCapabilities,
) {
    let rt = capabilities.ray_tracing;
    ui.label("Ray-traced shadows");
    ui.add_enabled(
        rt,
        egui::Checkbox::without_text(&mut settings.ray_traced_shadows),
    );
    ui.end_row();

    // Path tracing traces its own reflections and occlusion; traced global illumination
    // is occluded already.
    let path_traced = capabilities.path_tracing && settings.path_tracing;
    let gi_traced = capabilities.path_tracing
        && !settings.path_tracing
        && settings.global_illumination == GlobalIllumination::RayTraced;
    effect_row(
        ui,
        "Reflections",
        "engine_settings_reflections",
        &mut settings.reflections,
        rt,
        (!path_traced).then_some(()).ok_or("Part of path tracing"),
    );
    effect_row(
        ui,
        "Ambient occlusion",
        "engine_settings_ambient_occlusion",
        &mut settings.ambient_occlusion,
        rt,
        if path_traced {
            Err("Part of path tracing")
        } else if gi_traced {
            Err("Part of the traced global illumination")
        } else {
            Ok(())
        },
    );

    let name = |gi: GlobalIllumination| match gi {
        GlobalIllumination::Off => "Off",
        GlobalIllumination::RayTraced => "Ray traced",
    };
    ui.label("Global illumination");
    ui.add_enabled_ui(capabilities.path_tracing && !path_traced, |ui| {
        egui::ComboBox::from_id_salt("engine_settings_global_illumination")
            .selected_text(name(settings.global_illumination))
            .show_ui(ui, |ui| {
                for gi in [GlobalIllumination::Off, GlobalIllumination::RayTraced] {
                    ui.selectable_value(&mut settings.global_illumination, gi, name(gi));
                }
            });
    })
    .response
    .on_hover_text(
        "Light bounced off other surfaces and the sky seen through openings, traced and \
         denoised over the raster lighting: closer to path tracing at a fraction of its cost.",
    )
    .on_disabled_hover_text(if path_traced {
        "Part of path tracing"
    } else if capabilities.ray_tracing {
        "Ray-traced global illumination needs MSAA off"
    } else {
        "Ray-traced global illumination needs hardware ray tracing"
    });
    ui.end_row();

    ui.label("Ray-traced refraction");
    ui.add_enabled(
        rt,
        egui::Checkbox::without_text(&mut settings.ray_traced_refraction),
    );
    ui.end_row();
}

fn effect_row(
    ui: &mut egui::Ui,
    label: &str,
    id: &str,
    value: &mut EffectQuality,
    rt: bool,
    // Err: why it is disabled.
    enabled: Result<(), &str>,
) {
    let name = |quality: EffectQuality| match quality {
        EffectQuality::Off => "Off",
        EffectQuality::ScreenSpace => "Screen space",
        EffectQuality::RayTraced => "Ray traced",
    };
    ui.label(label);
    ui.add_enabled_ui(enabled.is_ok(), |ui| {
        egui::ComboBox::from_id_salt(id)
            .selected_text(name(*value))
            .show_ui(ui, |ui| {
                for quality in [
                    EffectQuality::Off,
                    EffectQuality::ScreenSpace,
                    EffectQuality::RayTraced,
                ] {
                    ui.add_enabled_ui(rt || quality != EffectQuality::RayTraced, |ui| {
                        ui.selectable_value(value, quality, name(quality));
                    });
                }
            });
    })
    .response
    .on_disabled_hover_text(enabled.err().unwrap_or_default());
    ui.end_row();
}

fn path_tracing_rows(
    ui: &mut egui::Ui,
    settings: &mut GraphicsSettings,
    capabilities: &GraphicsCapabilities,
) {
    let available = capabilities.path_tracing;
    ui.label("Path tracing");
    ui.add_enabled(
        available,
        egui::Checkbox::without_text(&mut settings.path_tracing),
    )
    .on_hover_text(
        "Light traced per pixel and denoised: soft shadows, bounced light, glowing surfaces \
         lighting their surroundings. Demanding; reflections and ambient occlusion are part of it.",
    )
    .on_disabled_hover_text(if capabilities.ray_tracing {
        "Path tracing needs MSAA off"
    } else {
        "Path tracing needs hardware ray tracing"
    });
    ui.end_row();

    let on = available && settings.path_tracing;
    ui.label("Bounces");
    ui.add_enabled(
        on,
        egui::Slider::new(
            &mut settings.path_tracing_bounces,
            1..=crate::renderer::postprocess::PathTracingSettings::MAX_BOUNCES,
        ),
    )
    .on_hover_text("Surfaces light bounces off: more fills enclosed spaces, at a cost per bounce");
    ui.end_row();

    ui.label("Samples per pixel");
    ui.add_enabled_ui(on, |ui| {
        egui::ComboBox::from_id_salt("engine_settings_path_tracing_samples")
            .selected_text(settings.path_tracing_samples.to_string())
            .show_ui(ui, |ui| {
                for samples in [1, 2, 4, 8] {
                    ui.selectable_value(
                        &mut settings.path_tracing_samples,
                        samples,
                        samples.to_string(),
                    );
                }
            });
    })
    .response
    .on_hover_text("Paths traced per pixel each frame: less noise, proportionally slower");
    ui.end_row();

    ui.label("Denoiser");
    ui.add_enabled(
        on,
        egui::Checkbox::without_text(&mut settings.path_tracing_denoiser),
    )
    .on_hover_text("Spatiotemporal denoiser (ReLAX-style); off shows the raw samples");
    ui.end_row();

    ui.label("Caustics");
    ui.add_enabled(
        on,
        egui::Checkbox::without_text(&mut settings.path_tracing_caustics),
    )
    .on_hover_text("Light focused through glass and off mirrors onto the surfaces around them");
    ui.end_row();
}
