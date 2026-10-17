mod support;

use glam::{Quat, Vec3};
use log::info;
use std::path::PathBuf;
use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::asset::MaterialAsset;
use wgpu_cube::render_application::{run_application, RenderApplication};
use wgpu_cube::renderer::postprocess::AntiAliasing;
use wgpu_cube::renderer::Material;
use wgpu_cube::scene::components::{
    CanCastShadow, DirectionalLight, Name, PointLight, TransformComponent,
};
use wgpu_cube::scene::{EntityBuilder, Transform};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const CAMERA_RADIUS: f32 = 11.5;
const CAMERA_HEIGHT: f32 = 6.0;

struct BloomExample;

impl RenderApplication for BloomExample {
    fn configure(&self, builder: &mut AppBuilder) {
        builder.disable_default_lighting();
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        setup_bloom_scene(ctx);
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        orbit_camera(ctx, CAMERA_RADIUS, CAMERA_HEIGHT);
    }
}

fn setup_bloom_scene(ctx: &mut StartupContext<'_>) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    info!("Creating bloom showcase scene...");

    support::configure(scene, [0.008, 0.012, 0.025], 0.08, |settings| {
        settings.exposure.compensation = 0.0;
        settings.anti_aliasing = AntiAliasing::Taa;
        settings.bloom.enabled = true;
        settings.bloom.intensity = 0.3;
        settings.bloom.threshold = 1.2;
        settings.bloom.scatter = 0.65;
        settings.ssao.radius = 0.5;
        settings.ssr.enabled = true;
        settings.ssr.intensity = 0.55;
        settings.ssr.blur = 0.35;
        settings.sharpen.enabled = true;
        settings.sharpen.strength = 0.2;
        settings.vignette.enabled = true;
        settings.vignette.intensity = 0.18;
        settings.vignette.smoothness = 0.4;
    });

    let (sphere_verts, sphere_idx) = wgpu_cube::renderer::sphere_mesh(48, 24);
    let sphere_mesh = renderer.create_mesh(&sphere_verts, &sphere_idx);
    let sphere_handle = scene.assets.meshes.insert(sphere_mesh);

    let (cube_verts, cube_idx) = wgpu_cube::renderer::cube_mesh();
    let cube_mesh = renderer.create_mesh(&cube_verts, &cube_idx);
    let cube_handle = scene.assets.meshes.insert(cube_mesh);

    let floor_material = scene.assets.materials.insert(MaterialAsset::from_material(
        Material::new([49, 59, 75, 255])
            .with_metallic(0.3)
            .with_roughness(0.28),
        PathBuf::from("examples/bloom/floor"),
    ));

    EntityBuilder::new(scene)
        .with_name("Floor")
        .with_transform(Transform::from_trs(
            Vec3::new(0.0, -1.25, 0.0),
            Quat::IDENTITY,
            Vec3::new(14.0, 0.5, 14.0),
        ))
        .with_mesh(cube_handle)
        .with_material(floor_material)
        .visible(true)
        .spawn();

    let column_material = scene.assets.materials.insert(MaterialAsset::from_material(
        Material::new([137, 151, 172, 255])
            .with_metallic(0.5)
            .with_roughness(0.24),
        PathBuf::from("examples/bloom/column"),
    ));

    let column_positions = [
        Vec3::new(-3.5, 0.0, -2.2),
        Vec3::new(-3.5, 0.0, 2.2),
        Vec3::new(3.5, 0.0, -2.2),
        Vec3::new(3.5, 0.0, 2.2),
    ];

    for (idx, pos) in column_positions.iter().enumerate() {
        EntityBuilder::new(scene)
            .with_name(format!("Reflector {}", idx))
            .with_transform(Transform::from_trs(
                *pos + Vec3::Y * 0.3,
                Quat::IDENTITY,
                Vec3::new(0.9, 2.6, 0.9),
            ))
            .with_mesh(cube_handle)
            .with_material(column_material)
            .visible(true)
            .spawn();
    }

    let pedestal_material = scene.assets.materials.insert(MaterialAsset::from_material(
        Material::new([200, 205, 215, 255])
            .with_metallic(0.0)
            .with_roughness(0.35),
        PathBuf::from("examples/bloom/pedestal"),
    ));

    let bloom_emitters = [
        (
            Vec3::new(0.0, 1.4, 0.0),
            [255, 123, 34, 255],
            Vec3::new(1.0, 0.65, 0.25),
        ),
        (
            Vec3::new(-2.6, 1.2, -1.8),
            [22, 143, 255, 255],
            Vec3::new(0.15, 0.55, 1.0),
        ),
        (
            Vec3::new(2.6, 1.0, -1.8),
            [206, 41, 255, 255],
            Vec3::new(0.75, 0.15, 1.0),
        ),
        (
            Vec3::new(2.2, 1.1, 2.4),
            [255, 76, 25, 255],
            Vec3::new(1.0, 0.35, 0.1),
        ),
        (
            Vec3::new(-2.2, 1.0, 2.6),
            [53, 255, 120, 255],
            Vec3::new(0.2, 1.0, 0.4),
        ),
    ];

    for (idx, (position, color, light_color)) in bloom_emitters.iter().enumerate() {
        EntityBuilder::new(scene)
            .with_name(format!("Pedestal {}", idx))
            .with_transform(Transform::from_trs(
                Vec3::new(position.x, -0.6, position.z),
                Quat::IDENTITY,
                Vec3::new(1.2, 0.8, 1.2),
            ))
            .with_mesh(cube_handle)
            .with_material(pedestal_material)
            .visible(true)
            .spawn();

        // HDR emission drives the halo; point lights only provide spill on nearby surfaces.
        let orb_material = Material::new(*color)
            .with_roughness(0.25)
            .with_emissive(4.0);
        let orb_material_handle = scene.assets.materials.insert(MaterialAsset::from_material(
            orb_material,
            PathBuf::from(format!("examples/bloom/orb_{idx}")),
        ));

        EntityBuilder::new(scene)
            .with_name(format!("Emitter {}", idx))
            .with_transform(Transform::from_trs(
                *position,
                Quat::IDENTITY,
                Vec3::splat(0.65),
            ))
            .with_mesh(sphere_handle)
            .with_material(orb_material_handle)
            .visible(true)
            .spawn();

        // Four satellite lights fit the renderer's four-point-light budget.
        if idx > 0 {
            scene.world_mut().spawn((
                Name::new(format!("Bloom Light {}", idx)),
                TransformComponent(Transform::from_trs(*position, Quat::IDENTITY, Vec3::ONE)),
                PointLight {
                    color: *light_color,
                    intensity: 24.0,
                    range: 9.0,
                    ..PointLight::default()
                },
                CanCastShadow(false),
            ));
        }
    }

    let key_direction = Vec3::new(-0.45, -1.1, -0.35).normalize();
    let key_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, key_direction);
    scene.world_mut().spawn((
        Name::new("Fill Light"),
        TransformComponent(Transform::from_trs(Vec3::ZERO, key_rotation, Vec3::ONE)),
        DirectionalLight::new(Vec3::splat(0.75), 1.8),
        CanCastShadow(true),
    ));

    let camera = scene.camera_mut();
    camera.eye = Vec3::new(7.5, CAMERA_HEIGHT, 7.5);
    camera.target = Vec3::new(0.0, 0.2, 0.0);
    camera.up = Vec3::Y;
}

fn orbit_camera(ctx: &mut UpdateContext<'_>, radius: f32, height: f32) {
    let t = ctx.scene.time() as f32 * 0.12 + std::f32::consts::FRAC_PI_4;
    let camera = ctx.scene.camera_mut();
    camera.eye = Vec3::new(
        t.cos() * radius,
        height + (t * 0.6).sin() * 0.6,
        t.sin() * radius,
    );
    camera.target = Vec3::new(0.0, 0.2, 0.0);
    camera.up = Vec3::Y;
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(BloomExample).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    run_application(BloomExample).unwrap();
}
