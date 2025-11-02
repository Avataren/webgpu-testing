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
mod environment_window;

#[cfg(feature = "egui")]
mod style; // Add this

#[cfg(feature = "egui")]
mod scene_hierarchy_window;

#[cfg(feature = "egui")]
mod scene_tabs;

#[cfg(feature = "egui")]
pub use stats_window::{FrameSample, FrameStatsHandle, FrameStatsHistory, StatsWindow};

#[cfg(feature = "egui")]
pub use log_viewer::{init_log_recorder, LogBufferHandle, LogEntry, LogWindow};

#[cfg(feature = "egui")]
pub use postprocess_window::{PostProcessEffectsHandle, PostProcessWindow};

#[cfg(feature = "egui")]
pub use environment_window::{
    EnvironmentSettingsControls, EnvironmentSettingsHandle, EnvironmentWindow,
};

#[cfg(feature = "egui")]
pub use style::UiStyle;

#[cfg(feature = "egui")]
pub use scene_hierarchy_window::{
    InspectorMaterial, SceneCreationAction, SceneEntityComponentsSummary, SceneEntityInspectorData,
    SceneHierarchyHandle, SceneHierarchyRegistryHandle, SceneHierarchyState, SceneHierarchyWindow,
    ScenePrimitivePreset,
};

#[cfg(feature = "egui")]
pub use scene_tabs::{SceneTabDescriptor, SceneTabsHandle, SceneTabsState};
