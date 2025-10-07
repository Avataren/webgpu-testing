use std::path::Path;

use glam::{Quat, Vec3};
use log::info;
use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::renderer::{Material, Texture};
use wgpu_cube::scene::components::{Billboard, BillboardOrientation, BillboardSpace, DepthState};
use wgpu_cube::scene::{
    MaterialComponent, MeshComponent, Name, Transform, TransformComponent, Visible,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

fn build_app() -> AppBuilder {
    let mut builder = AppBuilder::new();
    builder.add_startup_system(setup_shadow_scene);
    builder.add_system(orbit_camera(12.0, 6.0));
    builder
}

fn setup_shadow_scene(ctx: &mut StartupContext<'_>) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    info!("Creating shadow map test scene...");

    let (verts, idx) = wgpu_cube::renderer::cube_mesh();
    let cube_mesh = renderer.create_mesh(&verts, &idx);
    let cube_handle = scene.assets.meshes.insert(cube_mesh);

    let (quad_vertices, quad_indices) = wgpu_cube::renderer::quad_mesh();
    let quad_mesh = renderer.create_mesh(&quad_vertices, &quad_indices);
    let quad_handle = scene.assets.meshes.insert(quad_mesh);

    let checker_texture = Texture::checkerboard(
        renderer.get_device(),
        renderer.get_queue(),
        512,
        32,
        [200, 200, 200, 255],
        [40, 40, 40, 255],
        Some("Shadow Test Floor"),
    );
    let checker_handle = scene.assets.textures.insert(checker_texture);

    let floor_material = Material::pbr()
        .with_base_color_texture(checker_handle.index() as u32)
        .with_roughness(1.0);

    scene.world.spawn((
        Name::new("Shadow Test Floor"),
        TransformComponent(Transform::from_trs(
            Vec3::new(0.0, -0.05, 0.0),
            Quat::IDENTITY,
            Vec3::new(25.0, 0.1, 25.0),
        )),
        MeshComponent(cube_handle),
        MaterialComponent(floor_material),
        Visible(true),
    ));

    let cube_material = Material::new([220, 220, 230, 255])
        .with_metallic(0.0)
        .with_roughness(0.3);

    scene.world.spawn((
        Name::new("Shadow Test Cube"),
        TransformComponent(Transform::from_trs(
            Vec3::new(0.0, 1.0, 0.0),
            Quat::IDENTITY,
            Vec3::splat(1.5),
        )),
        MeshComponent(cube_handle),
        MaterialComponent(cube_material),
        Visible(true),
    ));

    let webgpu_texture = Texture::from_path(
        renderer.get_device(),
        renderer.get_queue(),
        Path::new("web/assets/textures/webgpu.png"),
        true,
    )
    .expect("Failed to load webgpu billboard texture");
    let webgpu_handle = scene.assets.textures.insert(webgpu_texture);

    let sprite_material = Material::new([255, 255, 255, 255])
        .with_base_color_texture(webgpu_handle.index() as u32)
        .with_alpha();

    let sprite_offset = Vec3::new(3.0, 2.2, 8.0);
    let sprite_transform = Transform::from_trs(sprite_offset, Quat::IDENTITY, Vec3::splat(2.5));

    let billboard =
        Billboard::new(BillboardOrientation::FaceCamera).with_space(BillboardSpace::View {
            offset: sprite_offset,
        });

    scene.world.spawn((
        Name::new("Shadow Test Sprite"),
        TransformComponent(sprite_transform),
        MeshComponent(quad_handle),
        MaterialComponent(sprite_material),
        billboard,
        DepthState::new(false, false),
        Visible(true),
    ));

    renderer.update_texture_bind_group(&scene.assets);

    info!("Shadow test scene created: {} entities", scene.world.len());
}

fn orbit_camera(
    radius: f32,
    height: f32,
) -> Box<dyn for<'a> FnMut(&mut UpdateContext<'a>) + 'static> {
    Box::new(move |ctx: &mut UpdateContext<'_>| {
        let t = ctx.scene.time() as f32 * 0.25;
        let camera = ctx.scene.camera_mut();
        camera.eye = Vec3::new(t.cos() * radius, height, t.sin() * radius);
        camera.target = Vec3::ZERO;
        camera.up = Vec3::Y;
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
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
