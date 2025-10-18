#![cfg(feature = "egui")]

mod camera;
mod inspector;
mod layout;
mod postprocess;
mod windows;

use std::fs;
use std::path::{Path, PathBuf};

use egui_tiles::{Tile, TileId, Tree};
use glam::{Vec2, Vec3, Vec4};
use hecs::Entity;
use log::{error, warn};
use wgpu_cube::app::{
    AppBuilder, GpuUpdateContext, RuntimeMode, RuntimeStateHandle, StartupContext, UpdateContext,
};
use wgpu_cube::renderer::{
    cube_mesh, CustomRenderContext, CustomRenderStage, Material, RenderRegion,
};
use wgpu_cube::scene::{
    EntityBuilder, MaterialComponent, MeshBounds, MeshComponent, Name, SelectedInEditor, Transform,
    TransformComponent, Visible, WorldTransform,
};
use wgpu_cube::scripting::{RuneScriptSource, RuneScriptingPlugin};
use wgpu_cube::{run_application, DefaultUI, RenderApplication};

use camera::EditorCameraController;
use layout::{create_editor_layout, EditorBehavior, EditorPane, ViewportState};
use postprocess::ViewportGrid;
use windows::WindowToggles;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_application(EditorApplication::new())?;
    Ok(())
}

struct EditorApplication {
    dock_tree: Tree<EditorPane>,
    scene_viewport: ViewportState,
    game_viewport: ViewportState,
    camera_controller: EditorCameraController,
    grid_postprocess: Option<ViewportGrid>,
    pending_imports: Vec<PathBuf>,
    windows: WindowToggles,
    selected_entity: Option<Entity>,
    highlighted_entity: Option<Entity>,
    pending_pick: Option<ViewportPick>,
    selection_override: Option<Option<Entity>>,
    runtime_state: RuntimeStateHandle,
    last_runtime_mode: RuntimeMode,
}

struct ViewportPick {
    uv: Vec2,
}

impl EditorApplication {
    fn new() -> Self {
        Self {
            dock_tree: create_editor_layout(),
            scene_viewport: ViewportState::default(),
            game_viewport: ViewportState::default(),
            camera_controller: EditorCameraController::default(),
            grid_postprocess: None,
            pending_imports: Vec::new(),
            windows: WindowToggles::new(),
            selected_entity: None,
            highlighted_entity: None,
            pending_pick: None,
            selection_override: None,
            runtime_state: RuntimeStateHandle::new(),
            last_runtime_mode: RuntimeMode::Editor,
        }
    }

    fn set_runtime_state_handle(&mut self, handle: RuntimeStateHandle) {
        self.runtime_state = handle;
    }

    fn find_pane_tile(&self, pane: EditorPane) -> Option<TileId> {
        self.dock_tree
            .tiles
            .iter()
            .find_map(|(id, tile)| match tile {
                Tile::Pane(current) if *current == pane => Some(*id),
                _ => None,
            })
    }

    fn ensure_viewport_tab_for_mode(&mut self, mode: RuntimeMode) {
        let target = match mode {
            RuntimeMode::Editor => EditorPane::SceneViewport,
            RuntimeMode::Playing => EditorPane::GameViewport,
        };

        if let Some(tile_id) = self.find_pane_tile(target) {
            let _ = self.dock_tree.make_active(|id, _| id == tile_id);
        }
    }

    fn load_script_text(path: &str) -> Option<String> {
        let full_path = PathBuf::from("scripts").join(path);
        match fs::read_to_string(&full_path) {
            Ok(src) => Some(src),
            Err(err) => {
                error!("Failed to read script {:?}: {err}", full_path);
                None
            }
        }
    }

    fn load_script(path: &str) -> Option<RuneScriptSource> {
        Self::load_script_text(path).map(|src| RuneScriptSource::inline(path, src))
    }

    fn create_import_script(path: &Path) -> Option<RuneScriptSource> {
        let template = Self::load_script_text("editor_import_gltf.rn")?;
        let path_string = path.to_string_lossy();
        let encoded_path = match serde_json::to_string(path_string.as_ref()) {
            Ok(value) => value,
            Err(err) => {
                error!("Failed to encode glTF path {path:?}: {err}");
                return None;
            }
        };

        let script_source = template.replace("__GLTF_PATH__", &encoded_path);

        let script_name = path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "imported_gltf".to_string());

        Some(RuneScriptSource::inline(
            format!("editor_import_gltf::{script_name}"),
            script_source,
        ))
    }

    fn process_pending_imports(&mut self, ctx: &mut UpdateContext) {
        if self.pending_imports.is_empty() {
            return;
        }

        let imports = std::mem::take(&mut self.pending_imports);
        for path in imports {
            let Some(script_source) = Self::create_import_script(&path) else {
                continue;
            };

            let entity_name = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "Imported glTF".to_string());

            let mut builder = EntityBuilder::new(ctx.scene.main_world_mut())
                .with_name(format!("{entity_name} (glTF)"))
                .with_transform(Transform::default())
                .with_script(script_source);
            builder.spawn();
        }

        if matches!(ctx.runtime, RuntimeMode::Editor) {
            ctx.scene.set_animation_playback(false);
            ctx.scene.update(0.0);
        }
    }

    fn sync_selection_component(&mut self, ctx: &mut UpdateContext) {
        if self.selected_entity == self.highlighted_entity {
            if let Some(entity) = self.selected_entity {
                let missing_marker = ctx
                    .scene
                    .main_world()
                    .get::<&SelectedInEditor>(entity)
                    .is_err();

                if missing_marker {
                    if let Err(err) = ctx
                        .scene
                        .main_world_mut()
                        .insert_one(entity, SelectedInEditor)
                    {
                        warn!(
                            "failed to reapply editor selection marker to {:?}: {err}",
                            entity
                        );
                        self.selected_entity = None;
                        self.highlighted_entity = None;
                    }
                }
            }
            return;
        }

        let mut new_highlight = None;
        {
            let world = ctx.scene.main_world_mut();

            if let Some(previous) = self.highlighted_entity.take() {
                let _ = world.remove_one::<SelectedInEditor>(previous);
            }

            if let Some(entity) = self.selected_entity {
                match world.insert_one(entity, SelectedInEditor) {
                    Ok(()) => new_highlight = Some(entity),
                    Err(err) => {
                        warn!("failed to mark entity {:?} as selected: {err}", entity);
                        self.selected_entity = None;
                    }
                }
            }
        }

        self.highlighted_entity = new_highlight;
    }

    fn capture_viewport_pick_input(&mut self, ctx: &egui::Context) {
        if matches!(self.runtime_state.active_mode(), RuntimeMode::Playing) {
            self.pending_pick = None;
            return;
        }

        if self.camera_controller.is_looking() {
            return;
        }

        let Some(rect) = self.scene_viewport.rect() else {
            return;
        };
        if rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }

        ctx.input(|input| {
            if !input.pointer.button_clicked(egui::PointerButton::Primary) {
                return;
            }

            let Some(pos) = input.pointer.latest_pos() else {
                return;
            };

            if !rect.contains(pos) {
                return;
            }

            let local_x = (pos.x - rect.min.x) / rect.width();
            let local_y = (pos.y - rect.min.y) / rect.height();
            if !local_x.is_finite() || !local_y.is_finite() {
                return;
            }

            let uv = Vec2::new(local_x.clamp(0.0, 1.0), local_y.clamp(0.0, 1.0));
            self.pending_pick = Some(ViewportPick { uv });
        });
    }

    fn process_viewport_pick(&mut self, ctx: &mut UpdateContext) {
        if !matches!(ctx.runtime, RuntimeMode::Editor) {
            self.pending_pick = None;
            return;
        };

        let Some(request) = self.pending_pick.take() else {
            return;
        };

        let Some(region) = self.scene_viewport.region() else {
            self.selected_entity = None;
            self.selection_override = Some(None);
            return;
        };

        let picked = self.pick_entity(ctx, request.uv, region);
        self.selected_entity = picked;
        self.selection_override = Some(picked);
    }

    fn pick_entity(&self, ctx: &UpdateContext, uv: Vec2, region: RenderRegion) -> Option<Entity> {
        let width = region.width().max(1) as f32;
        let height = region.height().max(1) as f32;
        let aspect = width / height;
        let camera = ctx.scene.camera();
        let (origin, direction) = Self::ray_from_uv(camera, uv, aspect);

        let world = ctx.scene.main_world();
        let mut best: Option<(Entity, f32)> = None;

        for (entity, (bounds, world_transform, local_transform, visible)) in world
            .query::<(
                &MeshBounds,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
                Option<&Visible>,
            )>()
            .iter()
        {
            if visible.is_some_and(|v| !v.0) {
                continue;
            }

            let transform = world_transform
                .map(|wt| wt.0)
                .or_else(|| local_transform.map(|lt| lt.0))
                .unwrap_or(Transform::IDENTITY);

            let Some(distance) = Self::entity_hit_distance(transform, *bounds, origin, direction)
            else {
                continue;
            };

            match best {
                Some((_, best_distance)) if distance >= best_distance => {}
                _ => best = Some((entity, distance)),
            }
        }

        best.map(|(entity, _)| entity)
    }

    fn ray_from_uv(camera: &wgpu_cube::scene::Camera, uv: Vec2, aspect: f32) -> (Vec3, Vec3) {
        let ndc_x = uv.x * 2.0 - 1.0;
        let ndc_y = 1.0 - uv.y * 2.0;

        let view = camera.view();
        let proj = camera.proj(aspect);
        let inv = (proj * view).inverse();

        let near = inv * Vec4::new(ndc_x, ndc_y, -1.0, 1.0);
        let far = inv * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);

        let near_point = if near.w.abs() > f32::EPSILON {
            near.truncate() / near.w
        } else {
            near.truncate()
        };
        let far_point = if far.w.abs() > f32::EPSILON {
            far.truncate() / far.w
        } else {
            far.truncate()
        };

        let origin = camera.eye;
        let mut direction = far_point - origin;
        if direction.length_squared() < 1e-12 {
            direction = (far_point - near_point).normalize_or_zero();
        } else {
            direction = direction.normalize();
        }

        (origin, direction)
    }

    fn entity_hit_distance(
        transform: Transform,
        bounds: MeshBounds,
        origin: Vec3,
        direction: Vec3,
    ) -> Option<f32> {
        let world_matrix = transform.matrix();
        let inverse = world_matrix.inverse();

        let origin_local = (inverse * origin.extend(1.0)).truncate();
        let direction_local = (inverse * direction.extend(0.0)).truncate();

        if !direction_local.is_finite() || direction_local.length_squared() < 1e-12 {
            return None;
        }

        let local_t =
            Self::ray_aabb_intersection(origin_local, direction_local, bounds.min, bounds.max)?;

        let hit_local = origin_local + direction_local * local_t;
        let hit_world = (world_matrix * hit_local.extend(1.0)).truncate();
        let distance = (hit_world - origin).length();

        distance.is_finite().then_some(distance)
    }

    fn ray_aabb_intersection(origin: Vec3, direction: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
        let mut t_min = f32::NEG_INFINITY;
        let mut t_max = f32::INFINITY;

        for axis in 0..3 {
            let o = origin[axis];
            let d = direction[axis];
            let min_bound = min[axis];
            let max_bound = max[axis];

            if d.abs() < 1e-6 {
                if o < min_bound || o > max_bound {
                    return None;
                }
                continue;
            }

            let inv = 1.0 / d;
            let mut t1 = (min_bound - o) * inv;
            let mut t2 = (max_bound - o) * inv;

            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }

            t_min = t_min.max(t1);
            t_max = t_max.min(t2);

            if t_min > t_max {
                return None;
            }
        }

        if t_max < 0.0 {
            return None;
        }

        let hit = if t_min >= 0.0 { t_min } else { t_max };
        (hit.is_finite() && hit >= 0.0).then_some(hit)
    }

    fn ensure_default_scene(&mut self, ctx: &mut StartupContext) {
        let has_editor_cube = {
            ctx.scene
                .main_world()
                .query::<&Name>()
                .iter()
                .any(|(_, name)| name.0 == "Editor Cube")
        };

        if !has_editor_cube {
            let startup_script = Self::load_script("editor_startup.rn");

            if let Some(script) = startup_script {
                let world = ctx.scene.main_world_mut();
                EntityBuilder::new(world)
                    .with_name("Editor Startup Script")
                    .with_script(script)
                    .spawn();

                ctx.scene.update(0.0);
            } else {
                error!("Failed to load editor startup script");
                return;
            }
        }

        let cube_entity = {
            let world = ctx.scene.main_world();
            world
                .query::<&Name>()
                .iter()
                .find(|(_, name)| name.0 == "Editor Cube")
                .map(|(entity, _)| entity)
        };

        if let Some(entity) = cube_entity {
            let missing_mesh = {
                let world = ctx.scene.main_world();
                world.get::<&MeshComponent>(entity).is_err()
            };
            let missing_bounds = {
                let world = ctx.scene.main_world();
                world.get::<&MeshBounds>(entity).is_err()
            };
            let mut cached_bounds = None;
            if missing_mesh {
                let (vertices, indices) = cube_mesh();
                cached_bounds = MeshBounds::from_vertices(&vertices);
                let mesh = ctx.renderer.create_mesh(&vertices, &indices);
                let mesh_handle = ctx.scene.assets.meshes.insert(mesh);
                if let Err(err) = ctx
                    .scene
                    .main_world_mut()
                    .insert_one(entity, MeshComponent(mesh_handle))
                {
                    error!("failed to attach mesh to Editor Cube: {err}");
                }
            }
            if missing_bounds {
                let bounds = cached_bounds
                    .unwrap_or_else(|| MeshBounds::new(Vec3::splat(-0.5), Vec3::splat(0.5)));
                if let Err(err) = ctx.scene.main_world_mut().insert_one(entity, bounds) {
                    error!("failed to attach bounds to Editor Cube: {err}");
                }
            }

            let missing_material = {
                let world = ctx.scene.main_world();
                world.get::<&MaterialComponent>(entity).is_err()
            };
            if missing_material {
                if let Err(err) = ctx
                    .scene
                    .main_world_mut()
                    .insert_one(entity, MaterialComponent(Material::pbr()))
                {
                    error!("failed to attach material to Editor Cube: {err}");
                }
            }

            let missing_visibility = {
                let world = ctx.scene.main_world();
                world.get::<&Visible>(entity).is_err()
            };
            if missing_visibility {
                if let Err(err) = ctx.scene.main_world_mut().insert_one(entity, Visible(true)) {
                    error!("failed to mark Editor Cube visible: {err}");
                }
            }
        }

        if !ctx.scene.has_any_lights() {
            ctx.scene.add_default_lighting();
        }

        let camera = ctx.scene.camera_mut();
        camera.eye = Vec3::new(6.0, 4.0, 6.0);
        camera.target = Vec3::new(0.0, 0.5, 0.0);
        camera.up = Vec3::Y;
    }

    fn show_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("editor_top_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        if ui.button("Import glTF...").clicked() {
                            ui.close();
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("glTF", &["gltf", "glb"])
                                .pick_file()
                            {
                                self.pending_imports.push(path);
                            }
                        }
                    }

                    #[cfg(target_arch = "wasm32")]
                    {
                        ui.add_enabled(false, egui::Button::new("Import glTF..."));
                    }

                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        ui.close();
                    }
                });

                ui.menu_button("Window", |ui| {
                    self.windows.window_menu(ui);
                });
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Toolbar").strong());
                ui.separator();
                let desired = self.runtime_state.desired_mode();
                let active = self.runtime_state.active_mode();
                let requesting_play = matches!(desired, RuntimeMode::Playing);
                let active_play = matches!(active, RuntimeMode::Playing);

                if ui
                    .add_enabled(!requesting_play, egui::Button::new("▶ Play"))
                    .clicked()
                {
                    self.runtime_state.request_mode(RuntimeMode::Playing);
                }

                ui.add_enabled(false, egui::Button::new("Pause"));

                if ui
                    .add_enabled(requesting_play || active_play, egui::Button::new("⏹ Stop"))
                    .clicked()
                {
                    self.runtime_state.request_mode(RuntimeMode::Editor);
                }

                ui.separator();

                let (status_text, status_color) = match (active_play, requesting_play) {
                    (true, true) => ("Play Mode", egui::Color32::from_rgb(120, 200, 120)),
                    (true, false) => ("Stopping...", egui::Color32::from_rgb(220, 190, 0)),
                    (false, true) => ("Starting...", egui::Color32::from_rgb(220, 190, 0)),
                    (false, false) => ("Editor Mode", egui::Color32::from_gray(180)),
                };

                ui.label(egui::RichText::new(status_text).color(status_color));
            });
        });
    }
}

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
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        self.ensure_default_scene(ctx);
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        if matches!(ctx.runtime, RuntimeMode::Editor) {
            self.camera_controller.update_camera(ctx);
        }
        self.process_pending_imports(ctx);
        self.process_viewport_pick(ctx);
        self.sync_selection_component(ctx);
    }

    fn gpu_update(&mut self, ctx: &mut GpuUpdateContext) {
        ctx.scene.process_pending_gltf_imports(ctx.renderer);
    }

    fn custom_render(&mut self, ctx: &mut CustomRenderContext) {
        if matches!(self.runtime_state.active_mode(), RuntimeMode::Editor) {
            let grid = self
                .grid_postprocess
                .get_or_insert_with(|| ViewportGrid::new(ctx.renderer.get_device()));
            grid.render(ctx);
        }
    }

    fn custom_render_stage(&self) -> CustomRenderStage {
        CustomRenderStage::AfterPostprocess
    }

    fn ui(&mut self, ctx: &egui::Context, default_ui: &mut DefaultUI) {
        self.scene_viewport.clear();
        self.game_viewport.clear();
        self.show_menu_bar(ctx);

        self.windows.show(ctx, default_ui);

        let runtime_mode = self.runtime_state.active_mode();
        if runtime_mode != self.last_runtime_mode {
            self.last_runtime_mode = runtime_mode;
            self.ensure_viewport_tab_for_mode(runtime_mode);
        }

        let dock_tree = &mut self.dock_tree;
        let scene_viewport = &mut self.scene_viewport;
        let game_viewport = &mut self.game_viewport;
        let scene_hierarchy_window = default_ui.scene_hierarchy_window_mut();
        if let Some(selection) = self.selection_override.take() {
            scene_hierarchy_window.set_selected_entity(selection);
        }
        let is_playing = matches!(runtime_mode, RuntimeMode::Playing);
        let transparent_frame =
            egui::Frame::central_panel(&ctx.style()).fill(egui::Color32::TRANSPARENT);
        egui::CentralPanel::default()
            .frame(transparent_frame)
            .show(ctx, |ui| {
                let mut behavior = EditorBehavior {
                    scene_viewport,
                    game_viewport,
                    scene_hierarchy: scene_hierarchy_window,
                    is_playing,
                };
                dock_tree.ui(&mut behavior, ui);
            });

        self.selected_entity = scene_hierarchy_window.selected_entity();

        if is_playing {
            self.camera_controller.set_viewport_rect(None);
        } else {
            self.camera_controller
                .set_viewport_rect(self.scene_viewport.rect());
        }

        self.camera_controller.capture_input(ctx);
        if !is_playing {
            self.capture_viewport_pick_input(ctx);
        } else {
            self.pending_pick = None;
        }
    }

    fn show_default_ui(&self) -> bool {
        false
    }

    fn render_region(&self) -> Option<RenderRegion> {
        if matches!(self.runtime_state.active_mode(), RuntimeMode::Playing) {
            self.game_viewport.region()
        } else {
            self.scene_viewport.region()
        }
    }
}
