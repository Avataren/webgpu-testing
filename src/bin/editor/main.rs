#![cfg(feature = "egui")]

mod camera;
mod inspector;
mod layout;
mod postprocess;
mod windows;

use std::fs;
use std::path::{Path, PathBuf};

use egui_tiles::Tree;
use glam::Vec3;
use log::error;
use wgpu_cube::app::{AppBuilder, GpuUpdateContext, StartupContext, UpdateContext};
use wgpu_cube::renderer::{
    cube_mesh, CustomRenderContext, CustomRenderStage, Material, RenderRegion,
};
use wgpu_cube::scene::{EntityBuilder, MaterialComponent, MeshComponent, Name, Transform, Visible};
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
    viewport: ViewportState,
    camera_controller: EditorCameraController,
    grid_postprocess: Option<ViewportGrid>,
    pending_imports: Vec<PathBuf>,
    windows: WindowToggles,
}

impl EditorApplication {
    fn new() -> Self {
        Self {
            dock_tree: create_editor_layout(),
            viewport: ViewportState::default(),
            camera_controller: EditorCameraController::default(),
            grid_postprocess: None,
            pending_imports: Vec::new(),
            windows: WindowToggles::new(),
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

                ui.menu_button("Window", |ui| {
                    self.windows.window_menu(ui);
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

    fn ui(&mut self, ctx: &egui::Context, default_ui: &mut DefaultUI) {
        self.viewport.clear();
        self.show_menu_bar(ctx);

        self.windows.show(ctx, default_ui);

        let dock_tree = &mut self.dock_tree;
        let viewport = &mut self.viewport;
        let scene_hierarchy_window = default_ui.scene_hierarchy_window_mut();
        let transparent_frame =
            egui::Frame::central_panel(&ctx.style()).fill(egui::Color32::TRANSPARENT);
        egui::CentralPanel::default()
            .frame(transparent_frame)
            .show(ctx, |ui| {
                let mut behavior = EditorBehavior {
                    viewport,
                    scene_hierarchy: scene_hierarchy_window,
                };
                dock_tree.ui(&mut behavior, ui);
            });

        self.camera_controller
            .set_viewport_rect(self.viewport.rect());
        self.camera_controller.capture_input(ctx);
    }

    fn show_default_ui(&self) -> bool {
        false
    }

    fn render_region(&self) -> Option<RenderRegion> {
        self.viewport.region()
    }
}
