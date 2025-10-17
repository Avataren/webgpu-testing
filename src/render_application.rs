// src/render_application.rs
// Complete fix to make custom_render work

use crate::app::{AppBuilder, GpuUpdateContext, StartupContext, UpdateContext};

use crate::renderer::{CustomRenderContext, CustomRenderStage, RenderRegion};
#[cfg(feature = "egui")]
use crate::ui::{
    init_log_recorder, EnvironmentSettingsHandle, EnvironmentWindow, FrameStatsHandle,
    LogBufferHandle, LogWindow, PostProcessEffectsHandle, PostProcessWindow, SceneHierarchyHandle,
    SceneHierarchyWindow, StatsWindow, UiStyle,
};

use std::cell::RefCell;
use std::rc::Rc;

/// Core trait for render applications. Implement this to define your application's behavior.
pub trait RenderApplication: Sized + 'static {
    fn name(&self) -> &str {
        "Render Application"
    }

    fn setup(&mut self, ctx: &mut StartupContext);

    fn update(&mut self, ctx: &mut UpdateContext) {
        let _ = ctx;
    }

    fn gpu_update(&mut self, ctx: &mut GpuUpdateContext) {
        let _ = ctx;
    }

    fn configure(&self, builder: &mut AppBuilder) {
        let _ = builder;
    }

    fn custom_render(&mut self, _ctx: &mut CustomRenderContext) {}

    fn custom_render_stage(&self) -> CustomRenderStage {
        CustomRenderStage::BeforePostprocess
    }

    fn custom_render_includes_shadows(&self) -> bool {
        false
    }

    fn render_region(&self) -> Option<RenderRegion> {
        None
    }

    #[cfg(feature = "egui")]
    fn ui(&mut self, ctx: &egui::Context, default_ui: &mut DefaultUI) {
        let _ = ctx;
        let _ = default_ui;
    }

    #[cfg(feature = "egui")]
    fn show_default_ui(&self) -> bool {
        true
    }

    /// Return the UI style to use. Override this to customize appearance.
    #[cfg(feature = "egui")]
    fn ui_style(&self) -> UiStyle {
        UiStyle::default()
    }
}

/// Helper that manages the default UI windows (stats + logs)
#[cfg(feature = "egui")]
pub struct DefaultUI {
    stats_window: StatsWindow,
    log_window: LogWindow,
    postprocess_window: PostProcessWindow,
    environment_window: EnvironmentWindow,
    scene_hierarchy_window: SceneHierarchyWindow,
    stats_open: bool,
    log_open: bool,
    postprocess_open: bool,
    environment_open: bool,
    scene_hierarchy_open: bool,
}

#[cfg(feature = "egui")]
impl DefaultUI {
    pub fn new(
        stats_handle: FrameStatsHandle,
        log_handle: LogBufferHandle,
        post_handle: PostProcessEffectsHandle,
        env_handle: EnvironmentSettingsHandle,
        hierarchy_handle: SceneHierarchyHandle,
    ) -> Self {
        Self {
            stats_window: StatsWindow::new(stats_handle),
            log_window: LogWindow::new(log_handle),
            postprocess_window: PostProcessWindow::new(post_handle),
            environment_window: EnvironmentWindow::new(env_handle),
            scene_hierarchy_window: SceneHierarchyWindow::new(hierarchy_handle),
            stats_open: true,
            log_open: false,
            postprocess_open: false,
            environment_open: false,
            scene_hierarchy_open: true,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        self.show_menu_bar(ctx);

        self.scene_hierarchy_window
            .show(ctx, Some(&mut self.scene_hierarchy_open));
        self.stats_window.show(ctx, Some(&mut self.stats_open));
        self.environment_window
            .show(ctx, Some(&mut self.environment_open));
        self.postprocess_window
            .show(ctx, Some(&mut self.postprocess_open));
        self.log_window.show(ctx, Some(&mut self.log_open));
    }

    fn show_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("default_ui_menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        ui.close();
                    }
                });

                ui.menu_button("Window", |ui| {
                    ui.checkbox(&mut self.stats_open, "Statistics");
                    ui.checkbox(&mut self.environment_open, "Environment");
                    ui.checkbox(&mut self.postprocess_open, "Post-processing");
                    ui.checkbox(&mut self.scene_hierarchy_open, "Scene Hierarchy");
                    ui.checkbox(&mut self.log_open, "Log");
                });
            });
        });
    }

    pub fn show_stats(&mut self, ctx: &egui::Context) {
        self.stats_window.show(ctx, Some(&mut self.stats_open));
        self.environment_window
            .show(ctx, Some(&mut self.environment_open));
        self.postprocess_window
            .show(ctx, Some(&mut self.postprocess_open));
    }

    pub fn show_logs(&mut self, ctx: &egui::Context) {
        self.log_window.show(ctx, Some(&mut self.log_open));
    }

    pub fn stats_window_mut(&mut self) -> &mut StatsWindow {
        &mut self.stats_window
    }

    pub fn log_window_mut(&mut self) -> &mut LogWindow {
        &mut self.log_window
    }

    pub fn postprocess_window_mut(&mut self) -> &mut PostProcessWindow {
        &mut self.postprocess_window
    }

    pub fn environment_window_mut(&mut self) -> &mut EnvironmentWindow {
        &mut self.environment_window
    }

    pub fn scene_hierarchy_window_mut(&mut self) -> &mut SceneHierarchyWindow {
        &mut self.scene_hierarchy_window
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

    let mut app = builder.build();

    // Install custom render callback
    {
        let stage = app_rc.borrow().custom_render_stage();
        app.set_custom_render_stage(stage);

        let initial_shadow = app_rc.borrow().custom_render_includes_shadows();
        app.enable_custom_render_shadows(initial_shadow);

        let shadow_query_app = app_rc.clone();
        app.set_custom_render_shadow_query(Box::new(move || {
            shadow_query_app.borrow().custom_render_includes_shadows()
        }));

        let callback_app = app_rc.clone();
        app.set_custom_render_callback(Box::new(move |ctx| {
            callback_app.borrow_mut().custom_render(ctx);
        }));
    }

    #[cfg(feature = "egui")]
    {
        let region_app = app_rc.clone();
        app.set_render_region_query(move || region_app.borrow().render_region());
    }

    #[cfg(feature = "egui")]
    {
        let show_default = app_rc.borrow().show_default_ui();
        let ui_style = app_rc.borrow().ui_style(); // Get the style
        let stats_handle = app.frame_stats_handle();
        let log_handle = init_log_recorder();
        let post_handle = app.postprocess_effects_handle();
        let env_handle = app.environment_settings_handle();
        let hierarchy_handle = app.scene_hierarchy_handle();

        if show_default {
            let mut default_ui = DefaultUI::new(
                stats_handle,
                log_handle,
                post_handle,
                env_handle.clone(),
                hierarchy_handle.clone(),
            );
            let app_ref = app_rc.clone();

            app.set_egui_ui(move |ctx| {
                ui_style.apply(ctx); // Apply style at the start of each frame
                default_ui.show(ctx);
                app_ref.borrow_mut().ui(ctx, &mut default_ui);
            });
        } else {
            let mut default_ui = DefaultUI::new(
                stats_handle,
                log_handle,
                post_handle,
                env_handle.clone(),
                hierarchy_handle,
            );
            let app_ref = app_rc.clone();

            app.set_egui_ui(move |ctx| {
                ui_style.apply(ctx); // Apply style at the start of each frame
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

    // Install custom render callback
    {
        let stage = app_rc.borrow().custom_render_stage();
        app.set_custom_render_stage(stage);

        let initial_shadow = app_rc.borrow().custom_render_includes_shadows();
        app.enable_custom_render_shadows(initial_shadow);

        let shadow_query_app = app_rc.clone();
        app.set_custom_render_shadow_query(Box::new(move || {
            shadow_query_app.borrow().custom_render_includes_shadows()
        }));

        let app_ref = app_rc.clone();
        app.set_custom_render_callback(Box::new(move |ctx| {
            app_ref.borrow_mut().custom_render(ctx);
        }));
    }

    #[cfg(feature = "egui")]
    {
        let region_app = app_rc.clone();
        app.set_render_region_query(move || region_app.borrow().render_region());
    }

    #[cfg(feature = "egui")]
    {
        let show_default = app_rc.borrow().show_default_ui();
        let stats_handle = app.frame_stats_handle();
        let log_handle = init_log_recorder();
        let post_handle = app.postprocess_effects_handle();
        let env_handle = app.environment_settings_handle();
        let hierarchy_handle = app.scene_hierarchy_handle();

        if show_default {
            let mut default_ui = DefaultUI::new(
                stats_handle,
                log_handle,
                post_handle,
                env_handle.clone(),
                hierarchy_handle.clone(),
            );
            let app_ref = app_rc.clone();

            app.set_egui_ui(move |ctx| {
                default_ui.show(ctx);
                app_ref.borrow_mut().ui(ctx, &mut default_ui);
            });
        } else {
            let mut default_ui = DefaultUI::new(
                stats_handle,
                log_handle,
                post_handle,
                env_handle.clone(),
                hierarchy_handle,
            );
            let app_ref = app_rc.clone();

            app.set_egui_ui(move |ctx| {
                app_ref.borrow_mut().ui(ctx, &mut default_ui);
            });
        }
    }

    crate::run_with_app(app)
}
