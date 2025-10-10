#[cfg(feature = "egui")]
mod egui_integration;

#[cfg(feature = "egui")]
pub use egui_integration::{EguiContext, EguiRenderTarget, EguiUiCallback};

#[cfg(feature = "egui")]
pub use egui;

#[cfg(feature = "egui")]
mod stats_window;

#[cfg(feature = "egui")]
mod log_viewer;

#[cfg(feature = "egui")]
mod postprocess_window;

#[cfg(feature = "egui")]
pub use stats_window::{FrameSample, FrameStatsHandle, FrameStatsHistory, StatsWindow};

#[cfg(feature = "egui")]
pub use log_viewer::{init_log_recorder, LogBufferHandle, LogEntry, LogWindow};

#[cfg(feature = "egui")]
pub use postprocess_window::{PostProcessEffectsHandle, PostProcessWindow};
