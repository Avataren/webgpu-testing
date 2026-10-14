mod support;

use std::path::{Path, PathBuf};

use glam::{Quat, Vec3};
use log::info;
use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::asset::MaterialAsset;
use wgpu_cube::render_application::{run_application, RenderApplication};
use wgpu_cube::renderer::postprocess::AntiAliasing;
use wgpu_cube::renderer::{Material, Texture};
use wgpu_cube::scene::components::{
    Billboard, BillboardOrientation, BillboardSpace, CanCastShadow, DepthState, DirectionalLight,
};
use wgpu_cube::scene::{
    MaterialComponent, MeshComponent, Name, Transform, TransformComponent, Visible,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const CAMERA_RADIUS: f32 = 12.0;
const CAMERA_HEIGHT: f32 = 6.0;

struct ExampleApp;

impl RenderApplication for ExampleApp {
    fn configure(&self, builder: &mut AppBuilder) {
        builder.disable_default_lighting();
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        setup_shadow_scene(ctx);
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        orbit_camera(ctx, CAMERA_RADIUS, CAMERA_HEIGHT);
    }
}

fn setup_shadow_scene(ctx: &mut StartupContext<'_>) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    info!("Creating shadow map test scene...");

    support::configure(scene, [0.018, 0.026, 0.045], 0.1, |settings| {
        settings.anti_aliasing = AntiAliasing::Taa;
        settings.sharpen.enabled = true;
        settings.sharpen.strength = 0.2;
    });
    scene.world_mut().spawn((
        Name::new("Shadow Key Light"),
        TransformComponent(Transform::from_trs(
            Vec3::ZERO,
            Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::new(0.35, -1.0, -0.85).normalize()),
            Vec3::ONE,
        )),
        DirectionalLight::new(Vec3::new(1.0, 0.94, 0.85), 3.0),
        CanCastShadow(true),
    ));

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
        64,
        [130, 147, 166, 255],
        [65, 80, 101, 255],
        Some("Shadow Test Floor"),
    );
    let checker_handle = scene.assets.textures.insert(checker_texture);

    let floor_material = scene.assets.materials.insert(MaterialAsset::from_material(
        Material::pbr()
            .with_base_color_texture(checker_handle.index() as u32)
            .with_roughness(1.0),
        PathBuf::from("examples/shadow/floor"),
    ));

    scene.world_mut().spawn((
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

    let cube_material = scene.assets.materials.insert(MaterialAsset::from_material(
        Material::new([231, 119, 78, 255])
            .with_metallic(0.0)
            .with_roughness(0.3),
        PathBuf::from("examples/shadow/cube"),
    ));

    scene.world_mut().spawn((
        Name::new("Shadow Test Cube"),
        TransformComponent(Transform::from_trs(
            Vec3::new(0.0, 0.75, 0.0),
            Quat::IDENTITY,
            Vec3::splat(1.5),
        )),
        MeshComponent(cube_handle),
        MaterialComponent(cube_material),
        Visible(true),
    ));

    let (vertices, indices) = wgpu_cube::renderer::sphere_mesh(48, 24);
    let sphere = renderer.create_mesh(&vertices, &indices);
    let sphere_handle = scene.assets.meshes.insert(sphere);
    for (name, mesh, position, scale, color) in [
        (
            "Shadow Test Sphere",
            sphere_handle,
            Vec3::new(-3.0, 1.0, -1.5),
            Vec3::ONE,
            [72, 166, 173, 255],
        ),
        (
            "Shadow Test Column",
            cube_handle,
            Vec3::new(2.0, 1.75, -2.0),
            Vec3::new(1.2, 3.5, 1.2),
            [217, 173, 99, 255],
        ),
    ] {
        let material = scene.assets.materials.insert(MaterialAsset::from_material(
            Material::new(color)
                .with_metallic(0.15)
                .with_roughness(0.35),
            PathBuf::from(format!("examples/shadow/{name}")),
        ));
        scene.world_mut().spawn((
            Name::new(name),
            TransformComponent(Transform::from_trs(
                position,
                Quat::from_rotation_y(20f32.to_radians()),
                scale,
            )),
            MeshComponent(mesh),
            MaterialComponent(material),
            CanCastShadow(true),
            Visible(true),
        ));
    }

    let webgpu_texture = Texture::from_path(
        renderer.get_device(),
        renderer.get_queue(),
        Path::new("web/assets/textures/webgpu.png"),
        true,
    )
    .expect("Failed to load webgpu billboard texture");
    let webgpu_handle = scene.assets.textures.insert(webgpu_texture);

    let sprite_material = scene.assets.materials.insert(MaterialAsset::from_material(
        Material::new([255, 255, 255, 255])
            .with_base_color_texture(webgpu_handle.index() as u32)
            .with_alpha(),
        PathBuf::from("examples/shadow/sprite"),
    ));

    let sprite_offset = Vec3::new(3.0, 2.2, 8.0);
    let sprite_transform = Transform::from_trs(sprite_offset, Quat::IDENTITY, Vec3::splat(1.6));

    let billboard =
        Billboard::new(BillboardOrientation::FaceCamera).with_space(BillboardSpace::View {
            offset: sprite_offset,
        });

    scene.world_mut().spawn((
        Name::new("Shadow Test Sprite"),
        TransformComponent(sprite_transform),
        MeshComponent(quad_handle),
        MaterialComponent(sprite_material),
        billboard,
        DepthState::new(false, false),
        Visible(true),
    ));

    renderer.update_texture_bind_group(&scene.assets);

    info!(
        "Shadow test scene created: {} entities",
        scene.world().len()
    );
}

fn orbit_camera(ctx: &mut UpdateContext<'_>, radius: f32, height: f32) {
    let t = ctx.scene.time() as f32 * 0.15 + std::f32::consts::FRAC_PI_4;
    let camera = ctx.scene.camera_mut();
    camera.eye = Vec3::new(t.cos() * radius, height, t.sin() * radius);
    camera.target = Vec3::new(0.0, 0.6, 0.0);
    camera.up = Vec3::Y;
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(ExampleApp).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    web_sys::console::log_1(&"[Rust] start_app() called".into());

    match run_application(ExampleApp) {
        Ok(_) => {
            web_sys::console::log_1(&"[Rust] Application started successfully".into());
        }
        Err(e) => {
            web_sys::console::error_1(&format!("[Rust] Error: {:?}", e).into());
        }
    }
}
