// src/render_application.rs
// Minimal trait system that integrates with existing UI module

use crate::app::{AppBuilder, GpuUpdateContext, StartupContext, UpdateContext};

#[cfg(feature = "egui")]
use crate::ui::{
    init_log_recorder, FrameStatsHandle, LogBufferHandle, LogWindow, PostProcessEffectsHandle,
    PostProcessWindow, StatsWindow,
};

use std::cell::RefCell;
use std::rc::Rc;

/// Core trait for render applications. Implement this to define your application's behavior.
pub trait RenderApplication: Sized + 'static {
    /// Name of your application
    fn name(&self) -> &str {
        "Render Application"
    }

    /// Called once during startup to initialize the scene
    fn setup(&mut self, ctx: &mut StartupContext);

    /// Called every frame to update application logic
    fn update(&mut self, ctx: &mut UpdateContext) {
        let _ = ctx;
    }

    /// Called every frame for GPU-related updates
    fn gpu_update(&mut self, ctx: &mut GpuUpdateContext) {
        let _ = ctx;
    }

    /// Configure the AppBuilder before systems are added
    fn configure(&self, builder: &mut AppBuilder) {
        let _ = builder;
    }

    /// Custom egui UI (called after default UI is rendered)
    #[cfg(feature = "egui")]
    fn ui(&mut self, ctx: &egui::Context, default_ui: &mut DefaultUI) {
        let _ = ctx;
        let _ = default_ui;
    }

    /// Whether to show the default UI windows
    #[cfg(feature = "egui")]
    fn show_default_ui(&self) -> bool {
        true
    }
}

/// Helper that manages the default UI windows (stats + logs)
#[cfg(feature = "egui")]
pub struct DefaultUI {
    stats_window: StatsWindow,
    log_window: LogWindow,
    postprocess_window: PostProcessWindow,
    stats_open: bool,
    log_open: bool,
    postprocess_open: bool,
}

#[cfg(feature = "egui")]
impl DefaultUI {
    pub fn new(
        stats_handle: FrameStatsHandle,
        log_handle: LogBufferHandle,
        post_handle: PostProcessEffectsHandle,
    ) -> Self {
        Self {
            stats_window: StatsWindow::new(stats_handle),
            log_window: LogWindow::new(log_handle),
            postprocess_window: PostProcessWindow::new(post_handle),
            stats_open: true,
            log_open: false,
            postprocess_open: true,
        }
    }

    /// Show both stats and log windows
    pub fn show(&mut self, ctx: &egui::Context) {
        self.stats_window.show(ctx, Some(&mut self.stats_open));
        self.postprocess_window
            .show(ctx, Some(&mut self.postprocess_open));
        self.log_window.show(ctx, Some(&mut self.log_open));
    }

    /// Show only stats window
    pub fn show_stats(&mut self, ctx: &egui::Context) {
        self.stats_window.show(ctx, Some(&mut self.stats_open));
        self.postprocess_window
            .show(ctx, Some(&mut self.postprocess_open));
    }

    /// Show only log window
    pub fn show_logs(&mut self, ctx: &egui::Context) {
        self.log_window.show(ctx, Some(&mut self.log_open));
    }

    /// Get mutable access to stats window
    pub fn stats_window_mut(&mut self) -> &mut StatsWindow {
        &mut self.stats_window
    }

    /// Get mutable access to log window
    pub fn log_window_mut(&mut self) -> &mut LogWindow {
        &mut self.log_window
    }

    /// Get mutable access to the post-processing window
    pub fn postprocess_window_mut(&mut self) -> &mut PostProcessWindow {
        &mut self.postprocess_window
    }
}

/// Run an application that implements RenderApplication
#[cfg(not(target_arch = "wasm32"))]
pub fn run_application<T>(application: T) -> Result<(), winit::error::EventLoopError>
where
    T: RenderApplication,
{
    let app_rc = Rc::new(RefCell::new(application));
    let mut builder = AppBuilder::new();

    app_rc.borrow().configure(&mut builder);

    {
        let app = app_rc.clone();
        builder.add_startup_system(move |ctx| {
            app.borrow_mut().setup(ctx);
        });
    }

    {
        let app = app_rc.clone();
        builder.add_system(move |ctx| {
            app.borrow_mut().update(ctx);
        });
    }

    {
        let app = app_rc.clone();
        builder.add_gpu_system(move |ctx| {
            app.borrow_mut().gpu_update(ctx);
        });
    }

    #[cfg_attr(not(feature = "egui"), allow(unused_mut))]
    let mut app = builder.build();

    #[cfg(feature = "egui")]
    {
        let show_default = app_rc.borrow().show_default_ui();
        let stats_handle = app.frame_stats_handle();
        let log_handle = init_log_recorder();
        let post_handle = app.postprocess_effects_handle();

        if show_default {
            let mut default_ui = DefaultUI::new(stats_handle, log_handle, post_handle);
            let app_ref = app_rc.clone();

            app.set_egui_ui(move |ctx| {
                default_ui.show(ctx);
                app_ref.borrow_mut().ui(ctx, &mut default_ui);
            });
        } else {
            let mut default_ui = DefaultUI::new(stats_handle, log_handle, post_handle);
            let app_ref = app_rc.clone();

            app.set_egui_ui(move |ctx| {
                app_ref.borrow_mut().ui(ctx, &mut default_ui);
            });
        }
    }

    crate::run_with_app(app)
}

/// Run an application (WebAssembly version)
#[cfg(target_arch = "wasm32")]
pub fn run_application<T>(application: T) -> Result<(), wasm_bindgen::JsValue>
where
    T: RenderApplication,
{
    let app_rc = Rc::new(RefCell::new(application));
    let mut builder = AppBuilder::new();

    app_rc.borrow().configure(&mut builder);

    {
        let app = app_rc.clone();
        builder.add_startup_system(move |ctx| {
            app.borrow_mut().setup(ctx);
        });
    }

    {
        let app = app_rc.clone();
        builder.add_system(move |ctx| {
            app.borrow_mut().update(ctx);
        });
    }

    {
        let app = app_rc.clone();
        builder.add_gpu_system(move |ctx| {
            app.borrow_mut().gpu_update(ctx);
        });
    }

    #[cfg_attr(not(feature = "egui"), allow(unused_mut))]
    let mut app = builder.build();

    #[cfg(feature = "egui")]
    {
        let show_default = app_rc.borrow().show_default_ui();
        let stats_handle = app.frame_stats_handle();
        let log_handle = init_log_recorder();
        let post_handle = app.postprocess_effects_handle();

        if show_default {
            let mut default_ui = DefaultUI::new(stats_handle, log_handle, post_handle);
            let app_ref = app_rc.clone();

            app.set_egui_ui(move |ctx| {
                default_ui.show(ctx);
                app_ref.borrow_mut().ui(ctx, &mut default_ui);
            });
        } else {
            let mut default_ui = DefaultUI::new(stats_handle, log_handle, post_handle);
            let app_ref = app_rc.clone();

            app.set_egui_ui(move |ctx| {
                app_ref.borrow_mut().ui(ctx, &mut default_ui);
            });
        }
    }

    crate::run_with_app(app)
}
