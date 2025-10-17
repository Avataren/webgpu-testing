#![cfg(feature = "egui")]

mod postprocess;

use std::f32::consts::FRAC_PI_2;
use std::fs;
use std::path::{Path, PathBuf};

use egui::{Color32, Stroke, StrokeKind};
use egui_tiles::{Behavior, Container, Tile, TileId, Tree, UiResponse};
use glam::{Vec2, Vec3};
use log::error;
use wgpu_cube::{run_application, DefaultUI, RenderApplication};

use wgpu_cube::app::{AppBuilder, GpuUpdateContext, StartupContext, UpdateContext};
use wgpu_cube::renderer::{
    cube_mesh, CustomRenderContext, CustomRenderStage, Material, RenderRegion,
};
use wgpu_cube::scene::{EntityBuilder, MaterialComponent, MeshComponent, Name, Transform, Visible};
use wgpu_cube::scripting::{RuneScriptSource, RuneScriptingPlugin};

use postprocess::ViewportGrid;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_application(EditorApplication::new())?;
    Ok(())
}

struct EditorApplication {
    dock_tree: Tree<EditorPane>,
    viewport_region: Option<RenderRegion>,
    viewport_rect: Option<egui::Rect>,
    camera_controller: EditorCameraController,
    grid_postprocess: Option<ViewportGrid>,
    pending_imports: Vec<PathBuf>,
}

impl EditorApplication {
    fn new() -> Self {
        Self {
            dock_tree: create_editor_layout(),
            viewport_region: None,
            viewport_rect: None,
            camera_controller: EditorCameraController::default(),
            grid_postprocess: None,
            pending_imports: Vec::new(),
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
            if missing_mesh {
                let (vertices, indices) = cube_mesh();
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
            });

            ui.separator();
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Toolbar").strong());
                ui.separator();
                ui.add_enabled(false, egui::Button::new("Play"));
                ui.add_enabled(false, egui::Button::new("Pause"));
                ui.add_enabled(false, egui::Button::new("Stop"));
            });
        });
    }
}

impl RenderApplication for EditorApplication {
    fn name(&self) -> &str {
        "Engine Editor"
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
        self.camera_controller.update_camera(ctx);
        self.process_pending_imports(ctx);
    }

    fn gpu_update(&mut self, ctx: &mut GpuUpdateContext) {
        ctx.scene.process_pending_gltf_imports(ctx.renderer);
    }

    fn custom_render(&mut self, ctx: &mut CustomRenderContext) {
        let grid = self
            .grid_postprocess
            .get_or_insert_with(|| ViewportGrid::new(ctx.renderer.get_device()));
        grid.render(ctx);
    }

    fn custom_render_stage(&self) -> CustomRenderStage {
        CustomRenderStage::AfterPostprocess
    }

    fn ui(&mut self, ctx: &egui::Context, _default_ui: &mut DefaultUI) {
        self.viewport_region = None;
        self.viewport_rect = None;
        self.show_menu_bar(ctx);

        let dock_tree = &mut self.dock_tree;
        let viewport_region = &mut self.viewport_region;
        let viewport_rect = &mut self.viewport_rect;
        let transparent_frame = egui::Frame::central_panel(&ctx.style()).fill(Color32::TRANSPARENT);
        egui::CentralPanel::default()
            .frame(transparent_frame)
            .show(ctx, |ui| {
                let mut behavior = EditorBehavior {
                    viewport_region,
                    viewport_rect,
                };
                dock_tree.ui(&mut behavior, ui);
            });

        self.camera_controller.set_viewport_rect(self.viewport_rect);
        self.camera_controller.capture_input(ctx);
    }

    fn show_default_ui(&self) -> bool {
        false
    }

    fn render_region(&self) -> Option<RenderRegion> {
        self.viewport_region
    }
}

struct EditorCameraController {
    move_forward: bool,
    move_back: bool,
    move_left: bool,
    move_right: bool,
    boost: bool,
    looking: bool,
    viewport_rect: Option<egui::Rect>,
    look_delta: Vec2,
    base_speed: f32,
    boost_multiplier: f32,
    look_sensitivity: f32,
}

impl Default for EditorCameraController {
    fn default() -> Self {
        Self {
            move_forward: false,
            move_back: false,
            move_left: false,
            move_right: false,
            boost: false,
            looking: false,
            viewport_rect: None,
            look_delta: Vec2::ZERO,
            base_speed: 5.0,
            boost_multiplier: 3.0,
            look_sensitivity: 0.0025,
        }
    }
}

impl EditorCameraController {
    fn set_viewport_rect(&mut self, rect: Option<egui::Rect>) {
        self.viewport_rect = rect;
        if rect.is_none() {
            self.looking = false;
            self.reset_movement();
            self.look_delta = Vec2::ZERO;
        }
    }

    fn reset_movement(&mut self) {
        self.move_forward = false;
        self.move_back = false;
        self.move_left = false;
        self.move_right = false;
        self.boost = false;
    }

    fn capture_input(&mut self, ctx: &egui::Context) {
        let viewport_rect = self.viewport_rect;
        let wants_keyboard = ctx.wants_keyboard_input();
        self.look_delta = Vec2::ZERO;

        ctx.input(|input| {
            if wants_keyboard || viewport_rect.is_none() {
                self.looking = false;
                self.reset_movement();
                return;
            }

            let rect = viewport_rect.unwrap();

            if input.pointer.secondary_pressed() {
                let press_pos = input
                    .pointer
                    .press_origin()
                    .or_else(|| input.pointer.hover_pos());
                self.looking = press_pos.is_some_and(|pos| rect.contains(pos));
                if !self.looking {
                    self.reset_movement();
                }
            } else if !input.pointer.secondary_down() {
                self.looking = false;
                self.reset_movement();
            }

            if !self.looking {
                return;
            }

            self.move_forward = input.key_down(egui::Key::W);
            self.move_back = input.key_down(egui::Key::S);
            self.move_left = input.key_down(egui::Key::A);
            self.move_right = input.key_down(egui::Key::D);
            self.boost = input.modifiers.shift;

            let motion = input
                .pointer
                .motion()
                .unwrap_or_else(|| input.pointer.delta());
            self.look_delta = Vec2::new(motion.x, motion.y);
        });
    }

    fn update_camera(&mut self, ctx: &mut UpdateContext) {
        let camera = ctx.scene.camera_mut();

        let mut forward = camera.target - camera.eye;
        if forward.length_squared() < f32::EPSILON {
            forward = Vec3::NEG_Z;
        }
        forward = forward.normalize();

        let mut up = camera.up;
        if up.length_squared() < f32::EPSILON {
            up = Vec3::Y;
        }
        up = up.normalize();

        if self.looking && self.look_delta.length_squared() > 0.0 {
            let delta = self.look_delta * self.look_sensitivity;
            self.look_delta = Vec2::ZERO;

            let mut yaw = forward.x.atan2(-forward.z);
            let mut pitch = forward.y.clamp(-1.0, 1.0).asin();
            yaw += delta.x;
            let max_pitch = FRAC_PI_2 - 0.01;
            pitch = (pitch - delta.y).clamp(-max_pitch, max_pitch);

            let cos_pitch = pitch.cos();
            let sin_pitch = pitch.sin();
            let sin_yaw = yaw.sin();
            let cos_yaw = yaw.cos();
            forward = Vec3::new(cos_pitch * sin_yaw, sin_pitch, -cos_pitch * cos_yaw);

            let mut right = forward.cross(Vec3::Y);
            if right.length_squared() < 1e-6 {
                right = Vec3::X;
            }
            right = right.normalize();
            up = right.cross(forward).normalize();

            camera.target = camera.eye + forward;
            camera.up = up;
        }

        let dt = ctx.dt as f32;
        if !self.looking || dt <= 0.0 {
            return;
        }

        let mut right = forward.cross(up);
        if right.length_squared() < 1e-6 {
            right = Vec3::X;
        }
        right = right.normalize();

        let mut movement = Vec3::ZERO;
        if self.move_forward {
            movement += forward;
        }
        if self.move_back {
            movement -= forward;
        }
        if self.move_left {
            movement -= right;
        }
        if self.move_right {
            movement += right;
        }

        if movement.length_squared() < 1e-6 {
            return;
        }

        movement = movement.normalize();
        let mut speed = self.base_speed * dt;
        if self.boost {
            speed *= self.boost_multiplier;
        }
        let delta = movement * speed;
        camera.eye += delta;
        camera.target += delta;
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EditorPane {
    Viewport,
    Inspector,
    Console,
}

struct EditorBehavior<'a> {
    viewport_region: &'a mut Option<RenderRegion>,
    viewport_rect: &'a mut Option<egui::Rect>,
}

impl Behavior<EditorPane> for EditorBehavior<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: TileId,
        pane: &mut EditorPane,
    ) -> UiResponse {
        match pane {
            EditorPane::Viewport => show_viewport(ui, self.viewport_region, self.viewport_rect),
            EditorPane::Inspector => {
                ui.heading("Inspector");
                ui.label("Select an entity to view its components.");
            }
            EditorPane::Console => {
                ui.heading("Console");
                ui.label("Engine logs will appear in this panel.");
            }
        }

        UiResponse::None
    }

    fn tab_title_for_pane(&mut self, pane: &EditorPane) -> egui::WidgetText {
        match pane {
            EditorPane::Viewport => "Viewport".into(),
            EditorPane::Inspector => "Inspector".into(),
            EditorPane::Console => "Console".into(),
        }
    }
}

fn create_editor_layout() -> Tree<EditorPane> {
    let mut tiles = egui_tiles::Tiles::default();

    let viewport = tiles.insert_pane(EditorPane::Viewport);
    let inspector = tiles.insert_pane(EditorPane::Inspector);
    let console = tiles.insert_pane(EditorPane::Console);

    let viewport_tab = tiles.insert_tab_tile(vec![viewport]);
    let inspector_tab = tiles.insert_tab_tile(vec![inspector]);
    let console_tab = tiles.insert_tab_tile(vec![console]);

    let horizontal = tiles.insert_horizontal_tile(vec![viewport_tab, inspector_tab]);
    if let Some(Tile::Container(Container::Linear(linear))) = tiles.get_mut(horizontal) {
        linear.shares.set_share(viewport_tab, 0.9);
        linear.shares.set_share(inspector_tab, 0.1);
    }

    let root = tiles.insert_vertical_tile(vec![horizontal, console_tab]);
    if let Some(Tile::Container(Container::Linear(linear))) = tiles.get_mut(root) {
        linear.shares.set_share(horizontal, 0.8);
        linear.shares.set_share(console_tab, 0.2);
    }

    Tree::new("editor_dock", root, tiles)
}

fn show_viewport(
    ui: &mut egui::Ui,
    region: &mut Option<RenderRegion>,
    rect_out: &mut Option<egui::Rect>,
) {
    let desired = ui.available_size();
    let desired = egui::vec2(desired.x.max(1.0), desired.y.max(1.0));
    let (rect, _response) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());
    *rect_out = Some(rect);

    if rect.width() <= 1.0 || rect.height() <= 1.0 {
        *region = None;
        *rect_out = None;
        return;
    }

    *region = compute_region(ui.ctx(), rect);

    let painter = ui.painter_at(rect);
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(1.0, Color32::from_gray(60)),
        StrokeKind::Outside,
    );
    painter.text(
        rect.center_top() + egui::vec2(0.0, 8.0),
        egui::Align2::CENTER_TOP,
        "Viewport",
        egui::TextStyle::Button.resolve(ui.style()),
        Color32::from_gray(180),
    );
}

fn compute_region(ctx: &egui::Context, rect: egui::Rect) -> Option<RenderRegion> {
    let pixels_per_point = ctx.pixels_per_point();
    let screen = ctx.viewport_rect();
    let max_width = (screen.width() * pixels_per_point).round().max(0.0) as u32;
    let max_height = (screen.height() * pixels_per_point).round().max(0.0) as u32;

    let min_x = (rect.min.x * pixels_per_point).floor().max(0.0);
    let min_y = (rect.min.y * pixels_per_point).floor().max(0.0);
    let width = (rect.width() * pixels_per_point).round().max(0.0);
    let height = (rect.height() * pixels_per_point).round().max(0.0);

    let region = RenderRegion::new(min_x as u32, min_y as u32, width as u32, height as u32)?;
    region.clamp(max_width, max_height)
}
