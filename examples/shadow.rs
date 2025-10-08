use std::path::Path;

use glam::{Quat, Vec2, Vec3};
use log::info;
use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::renderer::{Material, Texture};
use wgpu_cube::scene::components::{
    Billboard, BillboardOrientation, BillboardProjection, BillboardSpace, DepthState,
};
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

    let resolution = renderer.settings().resolution.clone();
    let base_ortho = Vec2::new(1920.0, 1080.0);
    let ortho_size = if resolution.width == 0 || resolution.height == 0 {
        base_ortho
    } else {
        let aspect = resolution.width as f32 / resolution.height as f32;
        let base_aspect = base_ortho.x / base_ortho.y;

        if aspect >= base_aspect {
            Vec2::new(base_ortho.y * aspect, base_ortho.y)
        } else {
            Vec2::new(base_ortho.x, base_ortho.x / aspect)
        }
    };
    renderer.set_billboard_ortho_size(ortho_size.x, ortho_size.y);

    let resolution_vec = Vec2::new(resolution.width as f32, resolution.height as f32);
    let units_per_pixel = if resolution.width == 0 || resolution.height == 0 {
        Vec2::ONE
    } else {
        Vec2::new(
            ortho_size.x / resolution_vec.x,
            ortho_size.y / resolution_vec.y,
        )
    };

    let sprite_pixels = Vec2::splat(256.0);
    let sprite_scale = Vec3::new(
        sprite_pixels.x * units_per_pixel.x,
        sprite_pixels.y * units_per_pixel.y,
        1.0,
    );

    let half = ortho_size * 0.5;
    let sprite_half = Vec2::new(sprite_scale.x * 0.5, sprite_scale.y * 0.5);

    let placements = [
        (
            "Top Left",
            Vec2::new(-half.x + sprite_half.x, half.y - sprite_half.y),
        ),
        (
            "Top Right",
            Vec2::new(half.x - sprite_half.x, half.y - sprite_half.y),
        ),
        (
            "Bottom Left",
            Vec2::new(-half.x + sprite_half.x, -half.y + sprite_half.y),
        ),
        (
            "Bottom Right",
            Vec2::new(half.x - sprite_half.x, -half.y + sprite_half.y),
        ),
    ];

    for (label, pos) in placements {
        let translation = Vec3::new(pos.x, pos.y, 0.0);
        let transform = Transform::from_trs(translation, Quat::IDENTITY, sprite_scale);
        let billboard = Billboard::new(BillboardOrientation::FaceCamera)
            .with_projection(BillboardProjection::Orthographic)
            .with_space(BillboardSpace::World);

        scene.world.spawn((
            Name::new(format!("Shadow Test {} Billboard", label)),
            TransformComponent(transform),
            MeshComponent(quad_handle),
            MaterialComponent(sprite_material),
            billboard,
            DepthState::new(false, false),
            Visible(true),
        ));
    }

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
