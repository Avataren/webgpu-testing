use glam::{Quat, Vec3};
use log::info;
use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::renderer::{Material, Texture};
use wgpu_cube::scene::{
    MaterialComponent, MeshComponent, Name, Transform, TransformComponent, Visible,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

fn build_app() -> AppBuilder {
    let mut builder = AppBuilder::new();
    builder.add_startup_system(setup_pbr_scene);
    builder.add_system(orbit_camera(8.0, 2.0));
    builder
}

fn setup_pbr_scene(ctx: &mut StartupContext<'_>) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    info!("Creating PBR test scene...");

    let (verts, idx) = wgpu_cube::renderer::sphere_mesh(64, 32);
    let sphere_mesh = renderer.create_mesh(&verts, &idx);
    let sphere_handle = scene.assets.meshes.insert(sphere_mesh);

    let unit_mr = Texture::from_color_linear(
        renderer.get_device(),
        renderer.get_queue(),
        [255, 255, 255, 255],
        Some("UnitMetallicRoughness"),
    );
    let unit_mr_handle = scene.assets.textures.insert(unit_mr);
    renderer.update_texture_bind_group(&scene.assets);

    let grid_size = 5;
    let spacing = 2.5;
    let start_offset = -((grid_size - 1) as f32 * spacing) / 2.0;

    for row in 0..grid_size {
        for col in 0..grid_size {
            let x = start_offset + col as f32 * spacing;
            let z = start_offset + row as f32 * spacing;

            let metallic = col as f32 / (grid_size - 1) as f32;
            let roughness = row as f32 / (grid_size - 1) as f32;

            let color = if col == 0 && row == 0 {
                [200, 200, 200, 255]
            } else if col == grid_size - 1 && row == 0 {
                [200, 150, 100, 255]
            } else if col == 0 && row == grid_size - 1 {
                [180, 180, 200, 255]
            } else {
                [220, 220, 220, 255]
            };

            let material = Material::new(color)
                .with_metallic(metallic)
                .with_roughness(roughness)
                .with_base_color_texture(0)
                .with_metallic_roughness_texture(unit_mr_handle.index() as u32);

            scene.world.spawn((
                Name::new(format!("Sphere_M{:.2}_R{:.2}", metallic, roughness)),
                TransformComponent(Transform::from_trs(
                    Vec3::new(x, 0.0, z),
                    Quat::IDENTITY,
                    Vec3::splat(0.8),
                )),
                MeshComponent(sphere_handle),
                MaterialComponent(material),
                Visible(true),
            ));
        }
    }

    info!("PBR test scene: {} entities", scene.world.len());
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
