#[cfg(feature = "egui")]
use crate::renderer::RendererStats;
#[cfg(feature = "egui")]
use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Shape, Stroke, StrokeKind};
#[cfg(feature = "egui")]
use std::collections::VecDeque;
#[cfg(feature = "egui")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "egui")]
const DEFAULT_HISTORY_SECONDS: f32 = 5.0;

#[cfg(feature = "egui")]
#[derive(Clone, Copy, Debug, Default)]
pub struct FrameSample {
    pub timestamp: f32,
    pub frame_time: f32,
    pub fps: f32,
    pub renderer: RendererStats,
}

#[cfg(feature = "egui")]
#[derive(Clone)]
pub struct FrameStatsHistory {
    samples: VecDeque<FrameSample>,
    total_elapsed: f32,
    max_history: f32,
}

#[cfg(feature = "egui")]
impl FrameStatsHistory {
    pub fn new() -> Self {
        Self {
            samples: VecDeque::new(),
            total_elapsed: 0.0,
            max_history: DEFAULT_HISTORY_SECONDS,
        }
    }

    pub fn record(&mut self, dt_seconds: f32, renderer: RendererStats) {
        self.total_elapsed += dt_seconds.max(0.0);
        let fps = if dt_seconds > 0.0 {
            1.0 / dt_seconds
        } else {
            0.0
        };
        let sample = FrameSample {
            timestamp: self.total_elapsed,
            frame_time: dt_seconds,
            fps,
            renderer,
        };
        self.samples.push_back(sample);

        let min_time = self.total_elapsed - self.max_history;
        while let Some(front) = self.samples.front() {
            if front.timestamp < min_time {
                self.samples.pop_front();
            } else {
                break;
            }
        }
    }

    pub fn snapshot(&self) -> FrameStatsSnapshot {
        FrameStatsSnapshot {
            samples: self.samples.iter().copied().collect(),
            average_fps: self.average_fps(),
            max_history: self.max_history,
        }
    }

    fn average_fps(&self) -> f32 {
        let total_dt: f32 = self.samples.iter().map(|s| s.frame_time).sum();
        if total_dt > 0.0 {
            self.samples.len() as f32 / total_dt
        } else {
            0.0
        }
    }

    pub fn handle() -> FrameStatsHandle {
        Arc::new(Mutex::new(Self::new()))
    }
}

#[cfg(feature = "egui")]
impl Default for FrameStatsHistory {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(feature = "egui")]
#[derive(Clone)]
pub struct FrameStatsSnapshot {
    samples: Vec<FrameSample>,
    average_fps: f32,
    max_history: f32,
}

#[cfg(feature = "egui")]
impl FrameStatsSnapshot {
    pub fn latest(&self) -> Option<FrameSample> {
        self.samples.last().copied()
    }

    pub fn samples(&self) -> &[FrameSample] {
        &self.samples
    }

    pub fn average_fps(&self) -> f32 {
        self.average_fps
    }

    pub fn span_seconds(&self) -> f32 {
        match (self.samples.first(), self.samples.last()) {
            (Some(first), Some(last)) => (last.timestamp - first.timestamp).max(0.0),
            _ => 0.0,
        }
    }

    pub fn max_history(&self) -> f32 {
        self.max_history
    }

    fn empty() -> Self {
        Self {
            samples: Vec::new(),
            average_fps: 0.0,
            max_history: DEFAULT_HISTORY_SECONDS,
        }
    }
}

#[cfg(feature = "egui")]
pub type FrameStatsHandle = Arc<Mutex<FrameStatsHistory>>;

#[cfg(feature = "egui")]
pub struct StatsWindow {
    stats: FrameStatsHandle,
    title: String,
    // Smoothed scale bounds to prevent jumping
    smoothed_max_fps: f32,
    smoothed_max_ms: f32,
}

#[cfg(feature = "egui")]
impl StatsWindow {
    pub fn new(stats: FrameStatsHandle) -> Self {
        Self {
            stats,
            title: "Stats".to_string(),
            smoothed_max_fps: 60.0,
            smoothed_max_ms: 16.67,
        }
    }

    /// Display the stats window using the provided egui context.
    ///
    /// Supplying [`Some`] for `open` adds a close button that toggles the provided
    /// handle when the user dismisses the window.
    pub fn show(&mut self, ctx: &egui::Context, open: Option<&mut bool>) {
        let snapshot = {
            let stats = self.stats.lock().ok();
            stats
                .as_ref()
                .map(|history| history.snapshot())
                .unwrap_or_else(FrameStatsSnapshot::empty)
        };

        let mut window = egui::Window::new(&self.title).default_width(320.0);
        if let Some(open) = open {
            window = window.open(open);
        }

        window.show(ctx, |ui| {
            if let Some(latest) = snapshot.latest() {
                ui.heading("Frame timings");
                ui.label(format!("FPS: {:.1}", latest.fps));
                ui.label(format!("Frame time: {:.2} ms", latest.frame_time * 1000.0));
                let span = snapshot.span_seconds().max(1e-6);
                ui.label(format!(
                    "Average FPS (last {:.1}s): {:.1}",
                    span.min(snapshot.max_history()),
                    snapshot.average_fps()
                ));

                ui.separator();
                self.draw_fps_plot(ui, &snapshot);

                ui.add_space(8.0);
                self.draw_frametime_plot(ui, &snapshot);

                ui.separator();
                self.draw_renderer_stats(ui, latest.renderer);
            } else {
                ui.label("Waiting for frames...");
            }
        });
    }

    fn draw_fps_plot(&mut self, ui: &mut egui::Ui, snapshot: &FrameStatsSnapshot) {
        let samples = snapshot.samples();
        if samples.len() < 2 {
            ui.label("Collecting frame history...");
            return;
        }

        let width = ui.available_width();
        let height = 120.0;
        let (response, painter) = ui.allocate_painter(vec2(width, height), egui::Sense::hover());
        let rect = response.rect;

        // Background
        painter.rect_filled(rect, CornerRadius::same(4), Color32::from_gray(25));
        painter.rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.0, Color32::from_gray(60)),
            StrokeKind::Middle,
        );

        let first_time = samples.first().map(|s| s.timestamp).unwrap_or(0.0);
        let last_time = samples.last().map(|s| s.timestamp).unwrap_or(first_time);
        let span = (last_time - first_time).max(1e-6);

        // Calculate current max and smooth it to prevent jumping
        let current_max_fps = samples
            .iter()
            .map(|s| s.fps)
            .fold(0.0_f32, f32::max)
            .max(1.0);

        // Smooth the max value: quickly increase, slowly decrease
        let smoothing_factor = if current_max_fps > self.smoothed_max_fps {
            0.5 // Fast increase
        } else {
            0.02 // Slow decrease
        };
        self.smoothed_max_fps =
            self.smoothed_max_fps * (1.0 - smoothing_factor) + current_max_fps * smoothing_factor;

        // Round up to nice numbers for better readability
        let max_fps = nice_upper_bound(self.smoothed_max_fps);

        // Draw reference lines for common framerates
        let reference_fps = [30.0, 60.0, 120.0, 144.0];
        for &target_fps in &reference_fps {
            if target_fps <= max_fps {
                let y = rect.bottom() - (target_fps / max_fps) * rect.height();
                painter.line_segment(
                    [pos2(rect.left(), y), pos2(rect.right(), y)],
                    Stroke::new(1.0, Color32::from_white_alpha(30)),
                );
                painter.text(
                    pos2(rect.right() - 4.0, y),
                    Align2::RIGHT_CENTER,
                    format!("{:.0}", target_fps),
                    FontId::proportional(10.0),
                    Color32::from_white_alpha(100),
                );
            }
        }

        // Draw FPS line
        let fps_points: Vec<_> = samples
            .iter()
            .map(|sample| {
                let t = (sample.timestamp - first_time) / span;
                let x = rect.left() + t * rect.width();
                let value = (sample.fps / max_fps).clamp(0.0, 1.0);
                let y = rect.bottom() - value * rect.height();
                pos2(x, y)
            })
            .collect();

        if fps_points.len() >= 2 {
            painter.add(Shape::line(
                fps_points,
                Stroke::new(2.0, Color32::from_rgb(100, 220, 100)),
            ));
        }

        // Title and scale
        painter.text(
            rect.left_top() + vec2(6.0, 6.0),
            Align2::LEFT_TOP,
            "FPS",
            FontId::proportional(13.0),
            Color32::WHITE,
        );
        painter.text(
            rect.left_top() + vec2(6.0, 22.0),
            Align2::LEFT_TOP,
            format!("0 - {:.0}", max_fps),
            FontId::proportional(11.0),
            Color32::from_white_alpha(180),
        );
    }

    fn draw_frametime_plot(&mut self, ui: &mut egui::Ui, snapshot: &FrameStatsSnapshot) {
        let samples = snapshot.samples();
        if samples.len() < 2 {
            return;
        }

        let width = ui.available_width();
        let height = 120.0;
        let (response, painter) = ui.allocate_painter(vec2(width, height), egui::Sense::hover());
        let rect = response.rect;

        // Background
        painter.rect_filled(rect, CornerRadius::same(4), Color32::from_gray(25));
        painter.rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.0, Color32::from_gray(60)),
            StrokeKind::Middle,
        );

        let first_time = samples.first().map(|s| s.timestamp).unwrap_or(0.0);
        let last_time = samples.last().map(|s| s.timestamp).unwrap_or(first_time);
        let span = (last_time - first_time).max(1e-6);

        // Calculate current max and smooth it
        let current_max_ms = samples
            .iter()
            .map(|s| s.frame_time * 1000.0)
            .fold(0.0_f32, f32::max)
            .max(0.001);

        let smoothing_factor = if current_max_ms > self.smoothed_max_ms {
            0.5 // Fast increase
        } else {
            0.02 // Slow decrease
        };
        self.smoothed_max_ms =
            self.smoothed_max_ms * (1.0 - smoothing_factor) + current_max_ms * smoothing_factor;

        let max_ms = nice_upper_bound(self.smoothed_max_ms);

        // Draw reference lines for common frame budgets
        let reference_ms = [16.67, 33.33, 8.33]; // 60fps, 30fps, 120fps
        for &target_ms in &reference_ms {
            if target_ms <= max_ms {
                let y = rect.bottom() - (target_ms / max_ms) * rect.height();
                painter.line_segment(
                    [pos2(rect.left(), y), pos2(rect.right(), y)],
                    Stroke::new(1.0, Color32::from_white_alpha(30)),
                );
                painter.text(
                    pos2(rect.right() - 4.0, y),
                    Align2::RIGHT_CENTER,
                    format!("{:.1}ms", target_ms),
                    FontId::proportional(10.0),
                    Color32::from_white_alpha(100),
                );
            }
        }

        // Draw frame time line
        let frame_points: Vec<_> = samples
            .iter()
            .map(|sample| {
                let t = (sample.timestamp - first_time) / span;
                let x = rect.left() + t * rect.width();
                let value = ((sample.frame_time * 1000.0) / max_ms).clamp(0.0, 1.0);
                let y = rect.bottom() - value * rect.height();
                pos2(x, y)
            })
            .collect();

        if frame_points.len() >= 2 {
            painter.add(Shape::line(
                frame_points,
                Stroke::new(2.0, Color32::from_rgb(100, 180, 255)),
            ));
        }

        // Title and scale
        painter.text(
            rect.left_top() + vec2(6.0, 6.0),
            Align2::LEFT_TOP,
            "Frame Time (ms)",
            FontId::proportional(13.0),
            Color32::WHITE,
        );
        painter.text(
            rect.left_top() + vec2(6.0, 22.0),
            Align2::LEFT_TOP,
            format!("0 - {:.1}ms", max_ms),
            FontId::proportional(11.0),
            Color32::from_white_alpha(180),
        );
    }

    fn draw_renderer_stats(&self, ui: &mut egui::Ui, stats: RendererStats) {
        ui.heading("Renderer");
        ui.label(format!("Draw calls: {}", stats.total_draw_calls()));
        ui.indent("draw_breakdown", |ui| {
            ui.label(format!("Depth prepass: {}", stats.depth_prepass_draw_calls));
            ui.label(format!("Opaque: {}", stats.opaque_draw_calls));
            ui.label(format!("Transparent: {}", stats.transparent_draw_calls));
            ui.label(format!("Overlay: {}", stats.overlay_draw_calls));
            ui.label(format!("Shadows: {}", stats.shadow_draw_calls));
        });
        ui.label(format!("Batches: {}", stats.batch_count));
        ui.label(format!("Instances: {}", stats.instance_count));
    }
}

// Helper function to round up to nice round numbers
#[cfg(feature = "egui")]
fn nice_upper_bound(value: f32) -> f32 {
    if value <= 0.0 {
        return 10.0;
    }

    // Find the magnitude
    let magnitude = 10_f32.powf(value.log10().floor());
    let normalized = value / magnitude;

    // Round up to 1, 2, 5, or 10
    let nice = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };

    nice * magnitude
}
