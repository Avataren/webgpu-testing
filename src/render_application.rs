// src/render_application.rs
// Complete fix to make custom_render work

#[cfg(feature = "egui")]
use crate::app::RuntimeStateHandle;
use crate::app::{AppBuilder, GpuUpdateContext, StartupContext, UpdateContext, WinitApp};
#[cfg(feature = "egui")]
use crate::scene::SceneHandle;
use crate::scene::SceneWorkspace;

use crate::renderer::{CustomRenderContext, CustomRenderStage, RenderRegion};
#[cfg(feature = "egui")]
use crate::ui::{
    init_log_recorder, EnvironmentSettingsHandle, EnvironmentWindow, FrameStatsHandle,
    LogBufferHandle, LogWindow, PostProcessEffectsHandle, PostProcessWindow, SceneHierarchyHandle,
    SceneHierarchyRegistryHandle, SceneHierarchyWindow, SceneTabsHandle, StatsWindow, UiStyle,
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

    #[cfg(feature = "egui")]
    fn install_runtime_state_handle(&mut self, handle: RuntimeStateHandle) {
        let _ = handle;
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
    scene_hierarchy_registry: SceneHierarchyRegistryHandle,
    scene_tabs: SceneTabsHandle,
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
        hierarchy_registry: SceneHierarchyRegistryHandle,
        hierarchy_handle: Option<(SceneHandle, SceneHierarchyHandle)>,
        scene_tabs: SceneTabsHandle,
    ) -> Self {
        Self {
            stats_window: StatsWindow::new(stats_handle),
            log_window: LogWindow::new(log_handle),
            postprocess_window: PostProcessWindow::new(post_handle),
            environment_window: EnvironmentWindow::new(env_handle),
            scene_hierarchy_window: SceneHierarchyWindow::new(hierarchy_handle),
            scene_hierarchy_registry: hierarchy_registry,
            scene_tabs,
            stats_open: false,
            log_open: false,
            postprocess_open: false,
            environment_open: false,
            scene_hierarchy_open: true,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        self.show_menu_bar(ctx);

        let workspace_info = self.scene_hierarchy_window.workspace_info();
        let _ = self.scene_hierarchy_window.show(
            ctx,
            &workspace_info,
            Some(&mut self.scene_hierarchy_open),
        );
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

    pub fn scene_hierarchy_and_log_windows_mut(
        &mut self,
    ) -> (&mut SceneHierarchyWindow, &mut LogWindow) {
        (&mut self.scene_hierarchy_window, &mut self.log_window)
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

    pub fn scene_hierarchy_registry(&self) -> SceneHierarchyRegistryHandle {
        self.scene_hierarchy_registry.clone()
    }

    pub fn scene_tabs_handle(&self) -> SceneTabsHandle {
        self.scene_tabs.clone()
    }
}

fn build_winit_app_internal<T>(app_rc: Rc<RefCell<T>>, mut builder: AppBuilder) -> WinitApp
where
    T: RenderApplication,
{
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

    let core = builder.build();
    let mut app = WinitApp::from_core(core);

    #[cfg(feature = "egui")]
    {
        let runtime_handle = app.runtime_state_handle();
        app_rc
            .borrow_mut()
            .install_runtime_state_handle(runtime_handle.clone());
    }

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
        let ui_style = app_rc.borrow().ui_style();
        let stats_handle = app.frame_stats_handle();
        let log_handle = init_log_recorder();
        let post_handle = app.postprocess_effects_handle();
        let env_handle = app.environment_settings_handle();
        let hierarchy_entry = app.active_scene_hierarchy();
        let hierarchy_registry = app.scene_hierarchy_registry();
        let scene_tabs = app.scene_tabs_handle();

        if show_default {
            let mut default_ui = DefaultUI::new(
                stats_handle,
                log_handle,
                post_handle,
                env_handle.clone(),
                hierarchy_registry.clone(),
                hierarchy_entry.clone(),
                scene_tabs.clone(),
            );
            let app_ref = app_rc.clone();

            app.set_egui_ui(move |ctx| {
                ui_style.apply(ctx);
                default_ui.show(ctx);
                app_ref.borrow_mut().ui(ctx, &mut default_ui);
            });
        } else {
            let mut default_ui = DefaultUI::new(
                stats_handle,
                log_handle,
                post_handle,
                env_handle.clone(),
                hierarchy_registry,
                hierarchy_entry,
                scene_tabs,
            );
            let app_ref = app_rc.clone();

            app.set_egui_ui(move |ctx| {
                ui_style.apply(ctx);
                app_ref.borrow_mut().ui(ctx, &mut default_ui);
            });
        }
    }

    app
}

fn build_winit_app<T>(app_rc: Rc<RefCell<T>>) -> WinitApp
where
    T: RenderApplication,
{
    build_winit_app_internal(app_rc, AppBuilder::new())
}

fn build_winit_app_with_workspace<T>(app_rc: Rc<RefCell<T>>, workspace: SceneWorkspace) -> WinitApp
where
    T: RenderApplication,
{
    build_winit_app_internal(app_rc, AppBuilder::new().with_workspace(workspace))
}

/// Run an application that implements RenderApplication
#[cfg(not(target_arch = "wasm32"))]
pub fn run_application<T>(application: T) -> Result<(), winit::error::EventLoopError>
where
    T: RenderApplication,
{
    let app_rc = Rc::new(RefCell::new(application));
    let app = build_winit_app(app_rc);
    crate::run_with_app(app)
}

/// Run an application with a pre-populated scene workspace
#[cfg(not(target_arch = "wasm32"))]
pub fn run_application_with_workspace<T>(
    application: T,
    workspace: SceneWorkspace,
) -> Result<(), winit::error::EventLoopError>
where
    T: RenderApplication,
{
    let app_rc = Rc::new(RefCell::new(application));
    let app = build_winit_app_with_workspace(app_rc, workspace);
    crate::run_with_app(app)
}

/// Run an application (WebAssembly version)
#[cfg(target_arch = "wasm32")]
pub fn run_application<T>(application: T) -> Result<(), wasm_bindgen::JsValue>
where
    T: RenderApplication,
{
    let app_rc = Rc::new(RefCell::new(application));
    let app = build_winit_app(app_rc);
    crate::run_with_app(app)
}

/// Run an application with a pre-populated scene workspace (WebAssembly version)
#[cfg(target_arch = "wasm32")]
pub fn run_application_with_workspace<T>(
    application: T,
    workspace: SceneWorkspace,
) -> Result<(), wasm_bindgen::JsValue>
where
    T: RenderApplication,
{
    let app_rc = Rc::new(RefCell::new(application));
    let app = build_winit_app_with_workspace(app_rc, workspace);
    crate::run_with_app(app)
}
