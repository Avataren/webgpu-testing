mod asset_browser_system;
mod system;
pub(crate) use system::*;
mod camera_system;
mod core;
mod history_system;
mod input;
mod particle_system;
mod picking;
mod project_system;
mod runtime_mode;
mod scene_creation_system;
mod script_editor_system;
mod scripts;
mod selection_system;
mod setup;
mod ui;

use self::asset_browser_system::AssetBrowserSystem;
pub use self::core::EditorApplication;
#[allow(unused_imports)]
pub use self::core::EditorApplicationBuilder;
use self::core::GameViewDisplayMode;

use glam::{Quat, Vec3};
use hecs::Entity;
use wgpu_cube::app::{
    AppBuilder, GpuUpdateContext, RuntimeMode, RuntimeStateHandle, StartupContext, UpdateContext,
};
use wgpu_cube::asset::{Handle, Mesh};
use wgpu_cube::gpu_particles::ParticleEmitter;
use wgpu_cube::renderer::primitives::{
    cone_mesh, cube_mesh, cylinder_mesh, quad_mesh, sphere_mesh, torus_mesh,
};
use wgpu_cube::renderer::{CustomRenderContext, CustomRenderStage, Material, RenderRegion};
use wgpu_cube::scene::components::{Billboard, BillboardOrientation, DepthState};
use wgpu_cube::scene::{
    CameraComponent, CanCastShadow, DirectionalLight, EntityBuilder, EnvironmentComponent,
    MaterialComponent, MeshBounds, ParticleBehaviorPreset, ParticleEmitterComponent,
    ParticleSystemComponent, PointLight, Scene, SpotLight, Transform, TransformComponent,
};
use wgpu_cube::scripting::RuneScriptingPlugin;
use wgpu_cube::{DefaultUI, RenderApplication, ScenePrimitivePreset};

use crate::inspector::InspectorAction;
use crate::layout::{EditorBehavior, EditorPane};
use crate::postprocess::ViewportGrid;

impl RenderApplication for EditorApplication {
    fn name(&self) -> &str {
        "Engine Editor"
    }

    fn install_runtime_state_handle(&mut self, handle: RuntimeStateHandle) {
        self.set_runtime_state_handle(handle);
    }

    fn configure(&self, builder: &mut AppBuilder) {
        builder.add_plugin(RuneScriptingPlugin::new());
        builder.disable_default_textures();
        builder.disable_default_lighting();
        builder.disable_escape_exit();
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        self.initialize_history_state(ctx.scene);
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        let mut ctx = self.make_update_context(ctx);
        ctx.with_update(|app, update_ctx| app.run_update_impl(update_ctx))
            .expect("update context is available");
    }

    fn gpu_update(&mut self, ctx: &mut GpuUpdateContext) {
        let mut ctx = self.make_gpu_update_context(ctx);
        ctx.with_gpu(|app, gpu_ctx| app.run_gpu_update_impl(gpu_ctx))
            .expect("gpu update context is available");
    }

    fn custom_render(&mut self, ctx: &mut CustomRenderContext) {
        if !matches!(ctx.stage, CustomRenderStage::Shadow(_))
            && matches!(self.runtime_state.active_mode(), RuntimeMode::Editor)
        {
            let grid = self
                .viewports
                .grid_postprocess
                .get_or_insert_with(|| ViewportGrid::new(ctx.renderer.get_device()));
            grid.render(ctx);
        }

        self.particle_system_mut().render(ctx);
    }

    fn custom_render_stage(&self) -> CustomRenderStage {
        CustomRenderStage::AfterPostprocess
    }

    fn custom_render_includes_shadows(&self) -> bool {
        self.particle_system().has_shadow_casters()
    }

    fn ui(&mut self, ctx: &egui::Context, default_ui: &mut DefaultUI) {
        let mut ctx = self.make_ui_context(ctx, default_ui);
        ctx.with_ui(|app, mut ui_ctx| app.run_ui_impl(ui_ctx.egui(), ui_ctx.default_ui()))
            .expect("ui context is available");
    }

    fn show_default_ui(&self) -> bool {
        false
    }

    fn render_region(&self) -> Option<RenderRegion> {
        self.render_region_for_mode(self.runtime_state.active_mode())
    }
}

impl EditorApplication {
    fn run_update_impl(&mut self, ctx: &mut UpdateContext) {
        // DETECT MODE CHANGES FIRST - saves state BEFORE any animation changes
        let current_mode = ctx.runtime;
        if current_mode != self.last_runtime_mode {
            self.detect_mode_transition(ctx, current_mode);
        }

        // Regular editor updates
        self.drain_update_commands(ctx);
        self.run_system_updates(ctx);
    }

    fn run_gpu_update_impl(&mut self, ctx: &mut GpuUpdateContext) {
        // PROCESS MODE TRANSITIONS FIRST
        self.process_pending_mode_transition(ctx);

        self.run_system_gpu_updates(ctx);
    }

    fn run_ui_impl(&mut self, ctx: &egui::Context, default_ui: &mut DefaultUI) {
        self.viewports.scene_viewport.clear();
        self.viewports.game_viewport.clear();

        self.project_system_mut().show_startup_dialog(ctx);

        if self.project_system().is_startup_dialog_visible() {
            return;
        }

        self.show_menu_bar(ctx);

        let active_mode = self.runtime_state.active_mode();
        let is_playing = matches!(active_mode, RuntimeMode::Playing);
        let show_fullscreen_game = is_playing
            && matches!(
                self.viewports.game_view_display,
                GameViewDisplayMode::Fullscreen
            );

        self.project_system_mut()
            .set_auxiliary_windows_enabled(!show_fullscreen_game);

        if !show_fullscreen_game {
            self.windows.show(ctx, default_ui);
        }

        let content_root = self.project_system().content_root();
        let override_selection = { self.selection_system_mut().take_override() };
        let dock_tree = &mut self.dock_tree;
        let scene_viewport = &mut self.viewports.scene_viewport;
        let game_viewport = &mut self.viewports.game_viewport;
        let (scene_hierarchy_window, log_window) = default_ui.scene_hierarchy_and_log_windows_mut();
        if let Some(selection) = override_selection {
            scene_hierarchy_window.set_selected_entity(selection);
        }
        let mut inspector_actions = Vec::new();
        let mut creation_actions = Vec::new();
        let systems_ptr = self.systems.as_mut_ptr();
        let asset_browser_index = self.asset_browser_system_index;
        let transparent_frame =
            egui::Frame::central_panel(&ctx.style()).fill(egui::Color32::TRANSPARENT);
        egui::CentralPanel::default()
            .frame(transparent_frame)
            .show(ctx, |ui| {
                if show_fullscreen_game {
                    crate::layout::show_fullscreen_viewport(ui, game_viewport);
                } else {
                    let asset_browser_state = unsafe {
                        (&mut *systems_ptr.add(asset_browser_index))
                            .as_any_mut()
                            .downcast_mut::<AssetBrowserSystem>()
                            .expect("asset browser system registered")
                            .state_mut()
                    };
                    let mut behavior = EditorBehavior {
                        scene_viewport,
                        game_viewport,
                        scene_hierarchy: scene_hierarchy_window,
                        log_window,
                        is_playing,
                        inspector_actions: &mut inspector_actions,
                        scene_creation_actions: &mut creation_actions,
                        asset_browser: asset_browser_state,
                        content_root,
                    };
                    dock_tree.ui(&mut behavior, ui);
                }
            });

        if self.scene_hierarchy_handle.is_none() {
            self.scene_hierarchy_handle = Some(scene_hierarchy_window.handle());
        }

        let game_tile_active = self
            .find_pane_tile(EditorPane::GameViewport)
            .map(|id| self.dock_tree.active_tiles().contains(&id))
            .unwrap_or(false);

        let scene_tile_active = self
            .find_pane_tile(EditorPane::SceneViewport)
            .map(|id| self.dock_tree.active_tiles().contains(&id))
            .unwrap_or(false);

        let has_pending_transition = self.pending_mode_transition.is_some();

        if !has_pending_transition
            && is_playing
            && !show_fullscreen_game
            && !game_tile_active
            && scene_tile_active
        {
            self.runtime_state.request_mode(RuntimeMode::Editor);
        }

        if !has_pending_transition
            && !is_playing
            && !matches!(self.runtime_state.desired_mode(), RuntimeMode::Playing)
            && game_tile_active
        {
            self.runtime_state.request_mode(RuntimeMode::Playing);
        }

        self.selection_system_mut()
            .set_selected(scene_hierarchy_window.selected_entity());

        for action in inspector_actions {
            match action {
                InspectorAction::EditScript { entity, component } => {
                    self.script_editor_system_mut()
                        .open_script_editor(entity, component);
                }
                other => {
                    self.enqueue_command(EditorCommand::Inspector(other));
                }
            }
        }

        for action in creation_actions {
            self.enqueue_command(EditorCommand::CreateScene(action));
        }

        if !is_playing {
            self.capture_viewport_pick_input(ctx);
            self.handle_history_shortcuts(ctx);
            self.handle_gizmo_shortcuts(ctx);
            self.handle_general_shortcuts(ctx);
        } else {
            self.selection_system_mut().clear_pending_pick();
        }

        self.script_editor_system_mut()
            .set_window_enabled(!show_fullscreen_game);
        self.run_system_ui(ctx, default_ui);
    }
}

impl EditorApplication {
    fn apply_pending_inspector_actions(
        &mut self,
        ctx: &mut UpdateContext,
        actions: Vec<InspectorAction>,
    ) {
        if actions.is_empty() {
            return;
        }

        self.resolve_active_camera_entity(ctx.scene);
        let mut transforms_changed = false;

        for action in actions {
            match action {
                InspectorAction::UpdateTransform { entity, transform } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut TransformComponent>(entity) {
                            Ok(mut component) => {
                                component.0 = transform;
                                updated = true;
                            }
                            Err(err) => {
                                log::warn!("Failed to update transform for {:?}: {}", entity, err);
                            }
                        }
                    }

                    if updated {
                        transforms_changed = true;
                        self.record_scene_change(ctx.scene);
                    }
                }
                InspectorAction::UpdateCamera { entity, component } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut CameraComponent>(entity) {
                            Ok(mut existing) => {
                                if *existing != component {
                                    *existing = component;
                                    updated = true;
                                }
                            }
                            Err(err) => {
                                log::warn!("Failed to update camera for {:?}: {}", entity, err);
                            }
                        }
                    }

                    if updated {
                        if self.active_camera_entity.is_none()
                            || self.active_camera_entity == Some(entity)
                        {
                            ctx.scene.set_active_camera_entity(Some(entity));
                            self.active_camera_entity = ctx.scene.active_camera_entity();
                        }

                        self.record_scene_change(ctx.scene);
                    }
                }
                InspectorAction::UpdateMaterial { entity, material } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut MaterialComponent>(entity) {
                            Ok(mut component) => {
                                component.0 = material;
                                updated = true;
                            }
                            Err(err) => {
                                log::warn!("Failed to update material for {:?}: {}", entity, err);
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(ctx.scene);
                    }
                }
                InspectorAction::UpdatePointLight { entity, light } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut PointLight>(entity) {
                            Ok(mut component) => {
                                *component = light;
                                updated = true;
                            }
                            Err(err) => {
                                log::warn!(
                                    "Failed to update point light for {:?}: {}",
                                    entity,
                                    err
                                );
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(ctx.scene);
                    }
                }
                InspectorAction::UpdateDirectionalLight { entity, light } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut DirectionalLight>(entity) {
                            Ok(mut component) => {
                                *component = light;
                                updated = true;
                            }
                            Err(err) => {
                                log::warn!(
                                    "Failed to update directional light for {:?}: {}",
                                    entity,
                                    err
                                );
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(ctx.scene);
                    }
                }
                InspectorAction::UpdateSpotLight { entity, light } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut SpotLight>(entity) {
                            Ok(mut component) => {
                                *component = light;
                                updated = true;
                            }
                            Err(err) => {
                                log::warn!("Failed to update spot light for {:?}: {}", entity, err);
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(ctx.scene);
                    }
                }
                InspectorAction::UpdateEnvironment { entity, component } => {
                    let previous = ctx
                        .scene
                        .main_world()
                        .get::<&EnvironmentComponent>(entity)
                        .ok()
                        .map(|existing| EnvironmentComponent::clone(&*existing));

                    let mut component = component;
                    let should_enable_hdr = {
                        let new_path = component
                            .hdr
                            .as_ref()
                            .and_then(|hdr| hdr.path.as_ref())
                            .map(|path| path.as_path());
                        let previous_path = previous
                            .as_ref()
                            .and_then(|prev| prev.hdr.as_ref())
                            .and_then(|hdr| hdr.path.as_ref())
                            .map(|path| path.as_path());

                        match (previous_path, new_path) {
                            (Some(prev), Some(new)) => prev != new,
                            (None, Some(_)) => true,
                            _ => false,
                        }
                    };

                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if let Err(err) =
                            self.copy_environment_asset_if_needed(&mut component, previous.as_ref())
                        {
                            log::warn!("{err}");
                            self.asset_browser_state_mut().report_error(err);
                        }
                    }

                    if let Some(hdr) = component.hdr.as_mut() {
                        if should_enable_hdr && hdr.path.is_some() {
                            hdr.enabled = true;
                        }
                    }

                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut EnvironmentComponent>(entity) {
                            Ok(mut existing) => {
                                if *existing != component {
                                    *existing = component.clone();
                                    updated = true;
                                }
                            }
                            Err(err) => {
                                log::warn!(
                                    "Failed to update environment for {:?}: {}",
                                    entity,
                                    err
                                );
                            }
                        }
                    }

                    if updated {
                        ctx.scene.set_environment(component.to_environment());
                        self.record_scene_change(ctx.scene);
                    }
                }
                InspectorAction::UpdateParticleSystem { entity, component } => {
                    let mut updated = false;
                    let new_spawn_rate = component.spawn_rate;
                    let mut spawn_rate_changed = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut ParticleSystemComponent>(entity) {
                            Ok(mut existing) => {
                                spawn_rate_changed =
                                    (existing.spawn_rate - new_spawn_rate).abs() > f32::EPSILON;
                                if *existing != component {
                                    *existing = component;
                                    updated = true;
                                }
                            }
                            Err(err) => {
                                log::warn!(
                                    "Failed to update particle system for {:?}: {}",
                                    entity,
                                    err
                                );
                            }
                        }

                        if spawn_rate_changed {
                            match world.get::<&mut ParticleEmitterComponent>(entity) {
                                Ok(mut emitter) => {
                                    if (emitter.spawn_rate - new_spawn_rate).abs() > f32::EPSILON {
                                        emitter.spawn_rate = new_spawn_rate;
                                        updated = true;
                                    }
                                }
                                Err(err) => {
                                    log::warn!(
                                        "Failed to update particle emitter spawn rate for {:?}: {}",
                                        entity,
                                        err
                                    );
                                }
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(ctx.scene);
                    }
                }
                InspectorAction::UpdateParticleEmitter { entity, component } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut ParticleEmitterComponent>(entity) {
                            Ok(mut existing) => {
                                if *existing != component {
                                    *existing = component;
                                    updated = true;
                                }
                            }
                            Err(err) => {
                                log::warn!(
                                    "Failed to update particle emitter for {:?}: {}",
                                    entity,
                                    err
                                );
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(ctx.scene);
                    }
                }
                InspectorAction::UpdateParticleBehavior {
                    entity,
                    behavior,
                    config,
                } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        match world.get::<&mut ParticleSystemComponent>(entity) {
                            Ok(mut existing) => {
                                let config = config.ensure_variant(behavior);
                                if existing.behavior != behavior
                                    || existing.behavior_config != config
                                {
                                    existing.behavior = behavior;
                                    existing.behavior_config = config;
                                    updated = true;
                                }
                            }
                            Err(err) => {
                                log::warn!(
                                    "Failed to update particle behavior for {:?}: {}",
                                    entity,
                                    err
                                );
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(ctx.scene);
                    }
                }
                InspectorAction::SetCanCastShadow {
                    entity,
                    casts_shadow,
                } => {
                    let mut updated = false;
                    {
                        let world = ctx.scene.main_world_mut();
                        let mut needs_insert = false;
                        match world.get::<&mut CanCastShadow>(entity) {
                            Ok(mut component) => {
                                if component.0 != casts_shadow {
                                    component.0 = casts_shadow;
                                    updated = true;
                                }
                            }
                            Err(err) => {
                                if casts_shadow {
                                    needs_insert = true;
                                } else {
                                    log::debug!(
                                        "CanCastShadow missing for {:?} while disabling shadows: {}",
                                        entity,
                                        err
                                    );
                                }
                            }
                        }

                        if needs_insert {
                            match world.insert(entity, (CanCastShadow(true),)) {
                                Ok(_) => {
                                    updated = true;
                                }
                                Err(insert_err) => {
                                    log::warn!(
                                        "Failed to add CanCastShadow to {:?}: {}",
                                        entity,
                                        insert_err
                                    );
                                }
                            }
                        }
                    }

                    if updated {
                        self.record_scene_change(ctx.scene);
                    }
                }
                InspectorAction::EditScript { .. } => {
                    // Script edits are handled immediately in the UI stage.
                }
            }
        }

        if transforms_changed {
            ctx.scene.propagate_transforms();
        }
    }

    fn resolve_active_camera_entity(&mut self, scene: &mut Scene) {
        if let Some(entity) = self.active_camera_entity {
            if scene.main_world().contains(entity) {
                return;
            }
            scene.set_active_camera_entity(None);
            self.active_camera_entity = scene.active_camera_entity();
        }

        let target_projection = scene.camera().projection();
        let candidate = {
            let world = scene.main_world();
            world
                .query::<&CameraComponent>()
                .iter()
                .find(|(_, component)| component.projection == target_projection)
                .map(|(entity, _)| entity)
        };

        if let Some(entity) = candidate {
            scene.set_active_camera_entity(Some(entity));
            self.active_camera_entity = scene.active_camera_entity();
        }
    }

    fn create_primitive(
        &mut self,
        ctx: &mut GpuUpdateContext,
        preset: ScenePrimitivePreset,
    ) -> Option<Entity> {
        let (vertices, indices) = match preset {
            ScenePrimitivePreset::Cube => cube_mesh(),
            ScenePrimitivePreset::Sphere => sphere_mesh(32, 16),
            ScenePrimitivePreset::Plane => quad_mesh(),
            ScenePrimitivePreset::Cylinder => cylinder_mesh(32),
            ScenePrimitivePreset::Cone => cone_mesh(32),
            ScenePrimitivePreset::Torus => torus_mesh(32, 16, 1.0, 0.35),
        };

        let mesh = ctx.renderer.create_mesh(&vertices, &indices);
        let mesh_handle = ctx.scene.assets.meshes.insert(mesh);
        let bounds = MeshBounds::from_vertices(&vertices);

        let mut builder = EntityBuilder::new(ctx.scene.main_world_mut());
        builder = builder
            .with_name(preset.display_name())
            .with_transform(Transform::default())
            .with_mesh(mesh_handle)
            .with_material(Material::pbr())
            .visible(true);

        if let Some(bounds) = bounds {
            builder = builder.with_component(bounds);
        }

        Some(builder.spawn())
    }

    fn create_particle_system(
        &mut self,
        ctx: &mut GpuUpdateContext,
        preset: ParticleBehaviorPreset,
    ) -> Option<Entity> {
        let name = format!("{} Particle System", preset.display_name());
        let spawn_rate = Self::default_particle_spawn_rate(preset);
        let (mesh_handle, bounds) = self.ensure_particle_mesh(ctx);
        let material = Self::default_particle_material(preset);
        let system_component = ParticleSystemComponent::new(spawn_rate, preset);
        let emitter_component = Self::default_particle_emitter(spawn_rate);

        let mut builder = EntityBuilder::new(ctx.scene.main_world_mut())
            .with_name(name)
            .with_transform(Transform::default())
            .with_mesh(mesh_handle)
            .with_material(material)
            .with_particle_system(system_component)
            .with_particle_emitter(emitter_component)
            .visible(true);

        if let Some(bounds) = bounds {
            builder = builder.with_component(bounds);
        }

        if let Some(billboard) = Self::default_particle_billboard(preset) {
            builder = builder.with_component(billboard);
        }

        if let Some(depth_state) = Self::default_particle_depth_state(preset) {
            builder = builder.with_component(depth_state);
        }

        Some(builder.spawn())
    }

    fn ensure_particle_mesh(
        &mut self,
        ctx: &mut GpuUpdateContext,
    ) -> (Handle<Mesh>, Option<MeshBounds>) {
        if let Some(handle) = self.particle_mesh {
            if ctx.scene.assets.meshes.get(handle).is_some() {
                return (handle, self.particle_mesh_bounds);
            }

            self.particle_mesh = None;
            self.particle_mesh_bounds = None;
        }

        let (vertices, indices) = quad_mesh();
        let mesh = ctx.renderer.create_mesh(&vertices, &indices);
        let handle = ctx.scene.assets.meshes.insert(mesh);
        let bounds = MeshBounds::from_vertices(&vertices);

        self.particle_mesh = Some(handle);
        self.particle_mesh_bounds = bounds;

        (handle, bounds)
    }

    fn default_particle_material(preset: ParticleBehaviorPreset) -> Material {
        match preset {
            ParticleBehaviorPreset::Starfield => Material::new([255, 255, 255, 255])
                .with_unlit()
                .with_billboarding(),
            _ => Material::pbr(),
        }
    }

    fn default_particle_emitter(spawn_rate: f32) -> ParticleEmitterComponent {
        let emitter = ParticleEmitter::new(Vec3::ZERO, spawn_rate);
        ParticleEmitterComponent::from(&emitter)
    }

    fn default_particle_billboard(preset: ParticleBehaviorPreset) -> Option<Billboard> {
        match preset {
            ParticleBehaviorPreset::Starfield => {
                Some(Billboard::new(BillboardOrientation::FaceCamera))
            }
            _ => None,
        }
    }

    fn default_particle_depth_state(preset: ParticleBehaviorPreset) -> Option<DepthState> {
        match preset {
            ParticleBehaviorPreset::Starfield => Some(DepthState::new(true, false)),
            _ => None,
        }
    }

    const fn default_particle_spawn_rate(_preset: ParticleBehaviorPreset) -> f32 {
        120.0
    }

    fn create_point_light(&mut self, ctx: &mut GpuUpdateContext) -> Option<Entity> {
        let transform = Transform::from_trs(Vec3::new(3.0, 3.0, 2.0), Quat::IDENTITY, Vec3::ONE);
        let light = PointLight {
            color: Vec3::splat(1.0),
            intensity: 120.0,
            range: 12.0,
        };

        let entity = EntityBuilder::new(ctx.scene.main_world_mut())
            .with_name("Point Light")
            .with_transform(transform)
            .with_component(light)
            .with_component(CanCastShadow(true))
            .spawn();

        Some(entity)
    }

    fn create_directional_light(&mut self, ctx: &mut GpuUpdateContext) -> Option<Entity> {
        let direction = Vec3::new(0.35, -1.0, -0.85).normalize();
        let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction);
        let transform = Transform::from_trs(Vec3::new(0.0, 6.0, 0.0), rotation, Vec3::ONE);
        let light = DirectionalLight::new(Vec3::new(0.9, 0.95, 1.0), 3.0);

        let entity = EntityBuilder::new(ctx.scene.main_world_mut())
            .with_name("Directional Light")
            .with_transform(transform)
            .with_component(light)
            .with_component(CanCastShadow(true))
            .spawn();

        Some(entity)
    }

    fn create_spot_light(&mut self, ctx: &mut GpuUpdateContext) -> Option<Entity> {
        let position = Vec3::new(-4.0, 5.0, -3.0);
        let target = Vec3::new(0.0, 1.0, 0.0);
        let direction = (target - position).normalize();
        let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction);
        let transform = Transform::from_trs(position, rotation, Vec3::ONE);
        let light = SpotLight {
            color: Vec3::new(1.0, 0.95, 0.9),
            intensity: 15.0,
            inner_angle: 0.35,
            outer_angle: 0.6,
            range: 18.0,
        };

        let entity = EntityBuilder::new(ctx.scene.main_world_mut())
            .with_name("Spot Light")
            .with_transform(transform)
            .with_component(light)
            .with_component(CanCastShadow(true))
            .spawn();

        Some(entity)
    }

    fn create_camera(&mut self, ctx: &mut GpuUpdateContext) -> Option<Entity> {
        let position = Vec3::new(0.0, 3.0, 6.0);
        let target = Vec3::ZERO;
        let direction = {
            let dir = target - position;
            if dir.length_squared() > 1e-6 {
                dir.normalize()
            } else {
                Vec3::NEG_Z
            }
        };
        let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, direction);
        let transform = Transform::from_trs(position, rotation, Vec3::ONE);

        let current_camera = *ctx.scene.camera();
        let camera_component = CameraComponent::from(current_camera);
        let entity = EntityBuilder::new(ctx.scene.main_world_mut())
            .with_name("Camera")
            .with_transform(transform)
            .with_component(camera_component)
            .spawn();

        ctx.scene.set_active_camera_entity(Some(entity));
        self.active_camera_entity = ctx.scene.active_camera_entity();

        Some(entity)
    }

    fn create_environment(&mut self, ctx: &mut GpuUpdateContext) -> Option<Entity> {
        let component = EnvironmentComponent::from_environment(ctx.scene.environment());
        let entity = EntityBuilder::new(ctx.scene.main_world_mut())
            .with_name("Environment")
            .with_component(component.clone())
            .spawn();

        ctx.scene.set_environment(component.to_environment());

        Some(entity)
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn copy_environment_asset_if_needed(
        &mut self,
        component: &mut EnvironmentComponent,
        previous: Option<&EnvironmentComponent>,
    ) -> Result<(), String> {
        let Some(hdr) = component.hdr.as_mut() else {
            return Ok(());
        };
        let Some(requested_path) = hdr.path.clone() else {
            return Ok(());
        };

        let previous_path = previous
            .and_then(|prev| prev.hdr.as_ref())
            .and_then(|hdr| hdr.path.clone());

        if previous_path.as_ref() == Some(&requested_path) {
            return Ok(());
        }

        let Some(content_root) = self.project_system().content_root() else {
            return Err(
                "Open or create a project before assigning environment HDR files.".to_string(),
            );
        };

        let absolute_source = if requested_path.is_absolute() {
            requested_path
        } else {
            std::env::current_dir()
                .map_err(|err| format!("Failed to resolve current directory: {err}"))?
                .join(&requested_path)
        };

        if !absolute_source.exists() {
            return Err(format!(
                "Selected HDR file {:?} does not exist",
                absolute_source
            ));
        }

        let environment_dir = content_root.join("environment");
        std::fs::create_dir_all(&environment_dir).map_err(|err| {
            format!(
                "Failed to create environment asset folder {:?}: {err}",
                environment_dir
            )
        })?;

        let file_name = absolute_source
            .file_name()
            .map(|name| name.to_owned())
            .ok_or_else(|| {
                format!(
                    "HDR file {:?} is missing a valid file name",
                    absolute_source
                )
            })?;

        let destination = environment_dir.join(&file_name);

        if destination != absolute_source {
            std::fs::copy(&absolute_source, &destination).map_err(|err| {
                format!(
                    "Failed to copy HDR file from {:?} to {:?}: {err}",
                    absolute_source, destination
                )
            })?;
            self.asset_browser_state_mut().report_info(format!(
                "Copied environment HDR to {}",
                destination.display()
            ));
        }

        let resolved = destination.canonicalize().unwrap_or(destination);
        hdr.path = Some(resolved);

        Ok(())
    }
}
