use glam::{Quat, Vec3};
use log::info;
use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::renderer::{Material, Texture};
use wgpu_cube::scene::components::{CanCastShadow, DirectionalLight, PointLight};
use wgpu_cube::scene::{
    MaterialComponent, MeshComponent, Name, Scene, Transform, TransformComponent, Visible,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

fn build_app() -> AppBuilder {
    let mut builder = AppBuilder::new();
    builder.disable_default_lighting();
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

    spawn_pbr_lighting(scene);

    info!("PBR test scene: {} entities", scene.world.len());
}

fn spawn_pbr_lighting(scene: &mut Scene) {
    let key_direction = Vec3::new(-0.4, -1.0, 0.25).normalize();
    let key_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, key_direction);

    scene.world.spawn((
        Name::new("PBR Key Light"),
        TransformComponent(Transform::from_trs(Vec3::ZERO, key_rotation, Vec3::ONE)),
        DirectionalLight {
            color: Vec3::new(1.0, 0.97, 0.9),
            intensity: 2.2,
        },
        CanCastShadow(true),
    ));

    let fill_position = Vec3::new(0.0, 2.5, 5.5);
    scene.world.spawn((
        Name::new("PBR Fill Light"),
        TransformComponent(Transform::from_trs(
            fill_position,
            Quat::IDENTITY,
            Vec3::ONE,
        )),
        PointLight {
            color: Vec3::new(0.9, 0.95, 1.0),
            intensity: 220.0,
            range: 22.0,
        },
        CanCastShadow(false),
    ));
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
