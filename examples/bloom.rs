use glam::{Quat, Vec3};
use log::info;
use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::render_application::{run_application, RenderApplication};
use wgpu_cube::renderer::Material;
use wgpu_cube::scene::components::{
    CanCastShadow, DirectionalLight, Name, PointLight, TransformComponent,
};
use wgpu_cube::scene::{EntityBuilder, Transform};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const CAMERA_RADIUS: f32 = 8.5;
const CAMERA_HEIGHT: f32 = 4.5;

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

    let (sphere_verts, sphere_idx) = wgpu_cube::renderer::sphere_mesh(48, 24);
    let sphere_mesh = renderer.create_mesh(&sphere_verts, &sphere_idx);
    let sphere_handle = scene.assets.meshes.insert(sphere_mesh);

    let (cube_verts, cube_idx) = wgpu_cube::renderer::cube_mesh();
    let cube_mesh = renderer.create_mesh(&cube_verts, &cube_idx);
    let cube_handle = scene.assets.meshes.insert(cube_mesh);

    EntityBuilder::new(&mut scene.world)
        .with_name("Floor")
        .with_transform(Transform::from_trs(
            Vec3::new(0.0, -1.25, 0.0),
            Quat::IDENTITY,
            Vec3::new(14.0, 0.5, 14.0),
        ))
        .with_mesh(cube_handle)
        .with_material(Material::new([64, 70, 76, 255]).with_roughness(0.85))
        .visible(true)
        .spawn();

    let column_material = Material::new([230, 232, 240, 255])
        .with_metallic(0.05)
        .with_roughness(0.18);

    let column_positions = [
        Vec3::new(-3.5, 0.0, -2.2),
        Vec3::new(-3.5, 0.0, 2.2),
        Vec3::new(3.5, 0.0, -2.2),
        Vec3::new(3.5, 0.0, 2.2),
    ];

    for (idx, pos) in column_positions.iter().enumerate() {
        EntityBuilder::new(&mut scene.world)
            .with_name(format!("Reflector {}", idx))
            .with_transform(Transform::from_trs(
                *pos,
                Quat::IDENTITY,
                Vec3::new(0.9, 2.6, 0.9),
            ))
            .with_mesh(cube_handle)
            .with_material(column_material)
            .visible(true)
            .spawn();
    }

    let pedestal_material = Material::new([200, 205, 215, 255])
        .with_metallic(0.0)
        .with_roughness(0.35);

    let bloom_emitters = [
        (Vec3::new(0.0, 1.4, 0.0), Vec3::new(1.0, 0.85, 0.6), 440.0),
        (Vec3::new(-2.6, 1.2, -1.8), Vec3::new(0.6, 0.85, 1.0), 360.0),
        (Vec3::new(2.6, 1.0, -1.8), Vec3::new(0.95, 0.5, 1.0), 340.0),
        (Vec3::new(2.2, 1.1, 2.4), Vec3::new(1.0, 0.65, 0.4), 380.0),
        (Vec3::new(-2.2, 1.0, 2.6), Vec3::new(0.7, 1.0, 0.6), 320.0),
    ];

    for (idx, (position, color, intensity)) in bloom_emitters.iter().enumerate() {
        EntityBuilder::new(&mut scene.world)
            .with_name(format!("Pedestal {}", idx))
            .with_transform(Transform::from_trs(
                Vec3::new(position.x, -0.4, position.z),
                Quat::IDENTITY,
                Vec3::new(1.2, 0.8, 1.2),
            ))
            .with_mesh(cube_handle)
            .with_material(pedestal_material)
            .visible(true)
            .spawn();

        let mut orb_material = Material::pbr().with_roughness(0.05);
        orb_material.base_color = to_srgb(*color);

        EntityBuilder::new(&mut scene.world)
            .with_name(format!("Emitter {}", idx))
            .with_transform(Transform::from_trs(
                *position,
                Quat::IDENTITY,
                Vec3::splat(0.65),
            ))
            .with_mesh(sphere_handle)
            .with_material(orb_material)
            .visible(true)
            .spawn();

        scene.world.spawn((
            Name::new(format!("Bloom Light {}", idx)),
            TransformComponent(Transform::from_trs(*position, Quat::IDENTITY, Vec3::ONE)),
            PointLight {
                color: *color,
                intensity: *intensity,
                range: 11.0,
            },
            CanCastShadow(false),
        ));
    }

    let key_direction = Vec3::new(-0.45, -1.1, -0.35).normalize();
    let key_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, key_direction);
    scene.world.spawn((
        Name::new("Fill Light"),
        TransformComponent(Transform::from_trs(Vec3::ZERO, key_rotation, Vec3::ONE)),
        DirectionalLight::new(Vec3::splat(0.75), 1.4),
        CanCastShadow(true),
    ));

    let camera = scene.camera_mut();
    camera.eye = Vec3::new(7.5, CAMERA_HEIGHT, 7.5);
    camera.target = Vec3::new(0.0, 1.0, 0.0);
    camera.up = Vec3::Y;
}

fn orbit_camera(ctx: &mut UpdateContext<'_>, radius: f32, height: f32) {
    let t = ctx.scene.time() as f32 * 0.25;
    let camera = ctx.scene.camera_mut();
    camera.eye = Vec3::new(
        t.cos() * radius,
        height + (t * 0.6).sin() * 0.6,
        t.sin() * radius,
    );
    camera.target = Vec3::new(0.0, 1.0, 0.0);
    camera.up = Vec3::Y;
}

fn to_srgb(color: Vec3) -> [u8; 4] {
    [
        (color.x.clamp(0.0, 1.0) * 255.0) as u8,
        (color.y.clamp(0.0, 1.0) * 255.0) as u8,
        (color.z.clamp(0.0, 1.0) * 255.0) as u8,
        255,
    ]
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
