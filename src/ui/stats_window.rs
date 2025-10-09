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
}

#[cfg(feature = "egui")]
pub type FrameStatsHandle = Arc<Mutex<FrameStatsHistory>>;

#[cfg(feature = "egui")]
pub struct StatsWindow {
    stats: FrameStatsHandle,
    title: String,
}

#[cfg(feature = "egui")]
impl StatsWindow {
    pub fn new(stats: FrameStatsHandle) -> Self {
        Self {
            stats,
            title: "Stats".to_string(),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        let snapshot = {
            let stats = self.stats.lock().ok();
            stats
                .as_ref()
                .map(|history| history.snapshot())
                .unwrap_or_else(FrameStatsSnapshot::empty)
        };

        egui::Window::new(&self.title)
            .default_width(320.0)
            .show(ctx, |ui| {
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
                    self.draw_plot(ui, &snapshot);

                    ui.separator();
                    self.draw_renderer_stats(ui, latest.renderer);
                } else {
                    ui.label("Waiting for frames...");
                }
            });
    }

    fn draw_plot(&self, ui: &mut egui::Ui, snapshot: &FrameStatsSnapshot) {
        let samples = snapshot.samples();
        if samples.len() < 2 {
            ui.label("Collecting frame history...");
            return;
        }

        let width = ui.available_width();
        let height = 160.0;
        let (response, painter) = ui.allocate_painter(vec2(width, height), egui::Sense::hover());
        let rect = response.rect;

        painter.rect_filled(rect, CornerRadius::same(4), Color32::from_gray(30));
        painter.rect_stroke(
            rect,
            CornerRadius::same(4),
            Stroke::new(1.0, Color32::from_gray(70)),
            StrokeKind::Middle,
        );

        let first_time = samples.first().map(|s| s.timestamp).unwrap_or(0.0);
        let last_time = samples.last().map(|s| s.timestamp).unwrap_or(first_time);
        let span = (last_time - first_time).max(1e-6);

        let max_fps = samples
            .iter()
            .map(|sample| sample.fps)
            .fold(0.0_f32, f32::max)
            .max(1.0);
        let max_ms = samples
            .iter()
            .map(|sample| sample.frame_time * 1000.0)
            .fold(0.0_f32, f32::max)
            .max(0.001);

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
                Stroke::new(2.0, Color32::LIGHT_GREEN),
            ));
        }

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
                Stroke::new(2.0, Color32::LIGHT_BLUE),
            ));
        }

        painter.text(
            rect.left_top() + vec2(8.0, 8.0),
            Align2::LEFT_TOP,
            format!("Max FPS: {:.0}", max_fps),
            FontId::proportional(12.0),
            Color32::LIGHT_GREEN,
        );
        painter.text(
            rect.left_top() + vec2(8.0, 26.0),
            Align2::LEFT_TOP,
            format!("Max frame time: {:.1} ms", max_ms),
            FontId::proportional(12.0),
            Color32::LIGHT_BLUE,
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

#[cfg(feature = "egui")]
impl FrameStatsSnapshot {
    fn empty() -> Self {
        Self {
            samples: Vec::new(),
            average_fps: 0.0,
            max_history: DEFAULT_HISTORY_SECONDS,
        }
    }
}
