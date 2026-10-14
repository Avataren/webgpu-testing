mod support;

use glam::{Quat, Vec3};
use log::info;
use std::path::PathBuf;
use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::asset::MaterialAsset;
use wgpu_cube::render_application::{run_application, RenderApplication};
use wgpu_cube::renderer::postprocess::AntiAliasing;
use wgpu_cube::renderer::texture::DEFAULT_WHITE_TEXTURE_INDEX;
use wgpu_cube::renderer::{Material, Texture};
use wgpu_cube::scene::components::{CanCastShadow, DirectionalLight, PointLight};
use wgpu_cube::scene::{
    MaterialComponent, MeshComponent, Name, Scene, Transform, TransformComponent, Visible,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const CAMERA_RADIUS: f32 = 16.0;
const CAMERA_HEIGHT: f32 = 11.0;

struct ExampleApp;

impl RenderApplication for ExampleApp {
    fn configure(&self, builder: &mut AppBuilder) {
        builder.disable_default_lighting();
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        setup_pbr_scene(ctx);
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        orbit_camera(ctx, CAMERA_RADIUS, CAMERA_HEIGHT);
    }
}

fn setup_pbr_scene(ctx: &mut StartupContext<'_>) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    info!("Creating PBR test scene...");

    support::configure(scene, [0.018, 0.026, 0.045], 0.11, |settings| {
        settings.anti_aliasing = AntiAliasing::Taa;
        settings.ssao.radius = 0.45;
        settings.sharpen.enabled = true;
        settings.sharpen.strength = 0.2;
    });

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

            // Keep albedo constant so only metallic and roughness change across the grid.
            let material = Material::new([215, 140, 68, 255])
                .with_metallic(metallic)
                .with_roughness(roughness)
                .with_base_color_texture(DEFAULT_WHITE_TEXTURE_INDEX)
                .with_metallic_roughness_texture(unit_mr_handle.index() as u32);

            let material_handle = scene.assets.materials.insert(MaterialAsset::from_material(
                material,
                PathBuf::from(format!("examples/pbr/material_{}_{}", row, col)),
            ));

            scene.world_mut().spawn((
                Name::new(format!("Sphere_M{:.2}_R{:.2}", metallic, roughness)),
                TransformComponent(Transform::from_trs(
                    Vec3::new(x, 0.0, z),
                    Quat::IDENTITY,
                    Vec3::splat(0.8),
                )),
                MeshComponent(sphere_handle),
                MaterialComponent(material_handle),
                Visible(true),
                CanCastShadow(true),
            ));
        }
    }

    let (vertices, indices) = wgpu_cube::renderer::cube_mesh();
    let plinth = renderer.create_mesh(&vertices, &indices);
    let plinth_mesh = scene.assets.meshes.insert(plinth);
    let plinth_material = scene.assets.materials.insert(MaterialAsset::from_material(
        Material::new([54, 70, 90, 255])
            .with_metallic(0.15)
            .with_roughness(0.42),
        PathBuf::from("examples/pbr/plinth"),
    ));
    scene.world_mut().spawn((
        Name::new("PBR Display Plinth"),
        TransformComponent(Transform::from_trs(
            Vec3::new(0.0, -1.0, 0.0),
            Quat::IDENTITY,
            Vec3::new(14.0, 0.4, 14.0),
        )),
        MeshComponent(plinth_mesh),
        MaterialComponent(plinth_material),
        Visible(true),
    ));

    spawn_pbr_lighting(scene);

    info!("PBR test scene: {} entities", scene.world().len());
}

fn spawn_pbr_lighting(scene: &mut Scene) {
    let key_direction = Vec3::new(-0.4, -1.0, 0.25).normalize();
    let key_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, key_direction);

    scene.world_mut().spawn((
        Name::new("PBR Key Light"),
        TransformComponent(Transform::from_trs(Vec3::ZERO, key_rotation, Vec3::ONE)),
        DirectionalLight::new(Vec3::new(1.0, 0.97, 0.9), 3.0),
        CanCastShadow(true),
    ));

    let fill_position = Vec3::new(-6.0, 5.0, 4.0);
    scene.world_mut().spawn((
        Name::new("PBR Fill Light"),
        TransformComponent(Transform::from_trs(
            fill_position,
            Quat::IDENTITY,
            Vec3::ONE,
        )),
        PointLight {
            color: Vec3::new(0.9, 0.95, 1.0),
            intensity: 75.0,
            range: 22.0,
        },
        CanCastShadow(false),
    ));

    scene.world_mut().spawn((
        Name::new("PBR Rim Light"),
        TransformComponent(Transform::from_trs(
            Vec3::ZERO,
            Quat::from_euler(
                glam::EulerRot::YXZ,
                145f32.to_radians(),
                -25f32.to_radians(),
                0.0,
            ),
            Vec3::ONE,
        )),
        DirectionalLight::new(Vec3::new(0.51, 0.74, 1.0), 1.5),
        CanCastShadow(false),
    ));
}

fn orbit_camera(ctx: &mut UpdateContext<'_>, radius: f32, height: f32) {
    let t = ctx.scene.time() as f32 * 0.12 + std::f32::consts::FRAC_PI_4;
    let camera = ctx.scene.camera_mut();
    camera.eye = Vec3::new(t.cos() * radius, height, t.sin() * radius);
    camera.target = Vec3::new(0.0, -0.2, 0.0);
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
