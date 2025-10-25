pub mod core;
pub mod platform_winit;

#[cfg(feature = "egui")]
pub mod editor;

pub use core::{
    AppBuilder, AppCore, FrameStep, GpuUpdateContext, GpuUpdateSystem, Plugin, RenderParams,
    RenderResult, RuntimeMode, RuntimeStateHandle, RuntimeTransition, StartupContext,
    StartupSystem, UpdateContext, UpdateSystem,
};

pub use platform_winit::WinitApp;

/// Backwards-compatible alias for the default winit-based application shell.
pub type App = platform_winit::WinitApp;
