use glam::{Quat, Vec3};
use log::info;
use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::renderer::{Material, Texture};
use wgpu_cube::scene::{
    MaterialComponent, MeshComponent, Name, Transform, TransformComponent, Visible,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const GRID_SIZE: i32 = 10;

fn build_app() -> AppBuilder {
    let mut builder = AppBuilder::new();
    builder.add_startup_system(setup_grid_scene);
    builder.add_system(orbit_camera(15.0, 8.0));
    builder
}

fn setup_grid_scene(ctx: &mut StartupContext<'_>) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    info!("Creating grid scene...");

    let (verts, idx) = wgpu_cube::renderer::cube_mesh();
    let cube_mesh = renderer.create_mesh(&verts, &idx);
    let cube_handle = scene.assets.meshes.insert(cube_mesh);

    let colors = [
        [255, 100, 100, 255],
        [100, 255, 100, 255],
        [100, 100, 255, 255],
        [255, 255, 100, 255],
        [255, 100, 255, 255],
    ];

    for color in colors {
        let texture = Texture::from_color(renderer.get_device(), renderer.get_queue(), color, None);
        scene.assets.textures.insert(texture);
    }

    let spacing = 2.0;
    for x in -GRID_SIZE..=GRID_SIZE {
        for z in -GRID_SIZE..=GRID_SIZE {
            let pos = Vec3::new(x as f32 * spacing, 0.0, z as f32 * spacing);
            let texture_idx = ((x.abs() + z.abs()) % 5) as u32;

            scene.world.spawn((
                Name::new(format!("Cube_{}_{}", x, z)),
                TransformComponent(Transform::from_trs(pos, Quat::IDENTITY, Vec3::splat(0.4))),
                MeshComponent(cube_handle),
                MaterialComponent(Material::white().with_texture(texture_idx)),
                Visible(true),
            ));
        }
    }

    info!("Grid scene: {} entities", scene.world.len());
}

fn orbit_camera(
    radius: f32,
    height: f32,
) -> Box<dyn for<'a> FnMut(&mut UpdateContext<'a>) + 'static> {
    Box::new(move |ctx: &mut UpdateContext<'_>| {
        let t = ctx.scene.time() as f32 * 0.25;
        ctx.camera.eye = Vec3::new(t.cos() * radius, height, t.sin() * radius);
        ctx.camera.target = Vec3::ZERO;
        ctx.camera.up = Vec3::Y;
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    env_logger::init();
    if let Err(err) = wgpu_cube::run(build_app()) {
        eprintln!("Application error: {err}");
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    web_sys::console::log_1(&"[Rust] start_app() called".into());

    match wgpu_cube::run(build_app()) {
        Ok(_) => {
            web_sys::console::log_1(&"[Rust] Application started successfully".into());
        }
        Err(e) => {
            web_sys::console::error_1(&format!("[Rust] Error: {:?}", e).into());
        }
    }
}
