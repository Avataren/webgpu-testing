#[cfg(feature = "egui")]
mod egui_integration;

#[cfg(feature = "egui")]
pub use egui_integration::EguiContext;

#[cfg(feature = "egui")]
pub use egui;
