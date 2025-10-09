use glam::Vec3;
use wgpu_cube::render_application::{RenderApplication, run_application};
use wgpu_cube::app::{StartupContext, UpdateContext};
use wgpu_cube::renderer::Material;
use wgpu_cube::scene::EntityBuilder;

struct TestApp;

impl RenderApplication for TestApp {
    fn setup(&mut self, ctx: &mut StartupContext) {
        log::info!("Setting up test scene");
        
        let (verts, idx) = wgpu_cube::renderer::cube_mesh();
        let mesh = ctx.renderer.create_mesh(&verts, &idx);
        let mesh_handle = ctx.scene.assets.meshes.insert(mesh);
        
        EntityBuilder::new(&mut ctx.scene.world)
            .with_name("Test Cube")
            .with_mesh(mesh_handle)
            .with_material(Material::red())
            .visible(true)
            .spawn();
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        let t = ctx.scene.time() as f32 * 0.25;
        let camera = ctx.scene.camera_mut();
        camera.eye = Vec3::new(t.cos() * 5.0, 3.0, t.sin() * 5.0);
        camera.target = Vec3::ZERO;
        camera.up = Vec3::Y;
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(TestApp).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn start_app() {
    run_application(TestApp).unwrap();
}