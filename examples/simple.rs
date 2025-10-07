use glam::{Quat, Vec3};
use log::info;
use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::renderer::{Material, Texture};
use wgpu_cube::scene::{
    EntityBuilder, MaterialComponent, MeshComponent, Name, Transform, TransformComponent, Visible,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

fn build_app() -> AppBuilder {
    let mut builder = AppBuilder::new();
    builder.add_startup_system(setup_simple_scene);
    builder.add_system(orbit_camera(8.0, 4.0));
    builder
}

fn setup_simple_scene(ctx: &mut StartupContext<'_>) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    info!("Creating simple scene...");

    let (verts, idx) = wgpu_cube::renderer::cube_mesh();
    let cube_mesh = renderer.create_mesh(&verts, &idx);
    let cube_handle = scene.assets.meshes.insert(cube_mesh);

    let texture = Texture::checkerboard(
        renderer.get_device(),
        renderer.get_queue(),
        256,
        32,
        [255, 255, 255, 255],
        [0, 0, 0, 255],
        Some("Checkerboard"),
    );
    scene.assets.textures.insert(texture);
    renderer.update_texture_bind_group(&scene.assets);

    EntityBuilder::new(&mut scene.world)
        .with_name("Red Cube")
        .with_transform(Transform::from_trs(
            Vec3::new(-2.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::ONE,
        ))
        .with_mesh(cube_handle)
        .with_material(Material::red())
        .visible(true)
        .spawn();

    scene.world.spawn((
        Name::new("Green Cube"),
        TransformComponent(Transform::from_trs(
            Vec3::new(0.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::ONE,
        )),
        MeshComponent(cube_handle),
        MaterialComponent(Material::green()),
        Visible(true),
    ));

    EntityBuilder::new(&mut scene.world)
        .with_name("Blue Cube")
        .with_transform(Transform::from_trs(
            Vec3::new(2.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::ONE,
        ))
        .with_mesh(cube_handle)
        .with_material(Material::blue())
        .visible(true)
        .spawn();

    info!("Simple scene: {} entities", scene.world.len());
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
