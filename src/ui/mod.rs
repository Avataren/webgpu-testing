#[cfg(feature = "egui")]
mod egui_integration;

#[cfg(feature = "egui")]
pub use egui_integration::EguiContext;

#[cfg(feature = "egui")]
pub use egui;

#[cfg(feature = "egui")]
mod stats_window;

#[cfg(feature = "egui")]
pub use stats_window::{FrameSample, FrameStatsHandle, FrameStatsHistory, StatsWindow};
