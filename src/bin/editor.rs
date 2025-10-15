#![cfg(feature = "egui")]

use egui::{Color32, Stroke, StrokeKind};
use egui_tiles::{Behavior, TileId, Tree, UiResponse};
use glam::{Quat, Vec3};
use wgpu_cube::{run_application, DefaultUI, RenderApplication};

use wgpu_cube::app::{GpuUpdateContext, StartupContext, UpdateContext};
use wgpu_cube::renderer::{cube_mesh, CustomRenderContext, Material, RenderRegion};
use wgpu_cube::scene::components::{CanCastShadow, DirectionalLight};
use wgpu_cube::scene::{
    MaterialComponent, MeshComponent, Name, Transform, TransformComponent, Visible,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_application(EditorApplication::new())?;
    Ok(())
}

struct EditorApplication {
    dock_tree: Tree<EditorPane>,
    viewport_region: Option<RenderRegion>,
}

impl EditorApplication {
    fn new() -> Self {
        Self {
            dock_tree: create_editor_layout(),
            viewport_region: None,
        }
    }

    fn ensure_default_scene(&mut self, ctx: &mut StartupContext) {
        let has_mesh = ctx
            .scene
            .world()
            .query::<&MeshComponent>()
            .iter()
            .next()
            .is_some();
        if has_mesh {
            return;
        }

        let (vertices, indices) = cube_mesh();
        let mesh = ctx.renderer.create_mesh(&vertices, &indices);
        let mesh_handle = ctx.scene.assets.meshes.insert(mesh);

        {
            let world = ctx.scene.main_world_mut();
            world.spawn((
                Name::new("Default Cube"),
                TransformComponent(Transform::from_translation(Vec3::new(0.0, 0.5, 0.0))),
                MeshComponent(mesh_handle),
                MaterialComponent(Material::pbr()),
                Visible(true),
            ));

            let light_direction = Vec3::new(-0.6, -1.0, -0.4).normalize();
            let light_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, light_direction);
            world.spawn((
                Name::new("Directional Light"),
                TransformComponent(Transform::from_trs(Vec3::ZERO, light_rotation, Vec3::ONE)),
                DirectionalLight::new(Vec3::new(1.0, 0.98, 0.92), 3.0),
                CanCastShadow(true),
            ));
        }

        {
            let camera = ctx.scene.camera_mut();
            camera.eye = Vec3::new(6.0, 4.0, 6.0);
            camera.target = Vec3::new(0.0, 0.5, 0.0);
            camera.up = Vec3::Y;
        }
    }

    fn show_menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("editor_menu_bar").show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Exit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        ui.close();
                    }
                });
            });
        });
    }
}

impl RenderApplication for EditorApplication {
    fn name(&self) -> &str {
        "Engine Editor"
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        self.ensure_default_scene(ctx);
    }

    fn update(&mut self, _ctx: &mut UpdateContext) {}

    fn gpu_update(&mut self, _ctx: &mut GpuUpdateContext) {}

    fn custom_render(&mut self, _ctx: &mut CustomRenderContext) {}

    fn ui(&mut self, ctx: &egui::Context, _default_ui: &mut DefaultUI) {
        self.viewport_region = None;
        self.show_menu_bar(ctx);

        let dock_tree = &mut self.dock_tree;
        let viewport_region = &mut self.viewport_region;
        egui::CentralPanel::default().show(ctx, |ui| {
            let mut behavior = EditorBehavior { viewport_region };
            dock_tree.ui(&mut behavior, ui);
        });
    }

    fn show_default_ui(&self) -> bool {
        false
    }

    fn render_region(&self) -> Option<RenderRegion> {
        self.viewport_region
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
}

impl Behavior<EditorPane> for EditorBehavior<'_> {
    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: TileId,
        pane: &mut EditorPane,
    ) -> UiResponse {
        match pane {
            EditorPane::Viewport => show_viewport(ui, self.viewport_region),
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
    let root = tiles.insert_vertical_tile(vec![horizontal, console_tab]);

    Tree::new("editor_dock", root, tiles)
}

fn show_viewport(ui: &mut egui::Ui, region: &mut Option<RenderRegion>) {
    let desired = ui.available_size();
    let desired = egui::vec2(desired.x.max(1.0), desired.y.max(1.0));
    let (rect, _response) = ui.allocate_exact_size(desired, egui::Sense::click_and_drag());

    if rect.width() <= 1.0 || rect.height() <= 1.0 {
        *region = None;
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
