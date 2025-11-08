/// UI module for exposing egui to Rune scripts.
///
/// This module provides a safe interface for Rune scripts to create
/// egui-based user interfaces that can run in both editor and play modes.
mod commands;
mod context;

pub use commands::{UiCommand, UiResponse};
pub use context::UiContext;
