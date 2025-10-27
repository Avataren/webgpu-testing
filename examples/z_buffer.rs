use glam::{Quat, Vec3};
use std::path::PathBuf;
use wgpu_cube::app::{StartupContext, UpdateContext};
use wgpu_cube::asset::MaterialAsset;
use wgpu_cube::render_application::{run_application, RenderApplication};
use wgpu_cube::renderer::Material;
use wgpu_cube::scene::components::{
    CanCastShadow, DepthState, MaterialComponent, MeshComponent, Name, PointLight,
    TransformComponent, Visible,
};
use wgpu_cube::scene::{EntityBuilder, Transform};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const CAMERA_RADIUS: f32 = 7.0;
const CAMERA_HEIGHT: f32 = 3.0;

struct ZBufferExample;

impl RenderApplication for ZBufferExample {
    fn setup(&mut self, ctx: &mut StartupContext) {
        setup_scene(ctx);
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        orbit_camera(ctx, CAMERA_RADIUS, CAMERA_HEIGHT);
    }
}

fn setup_scene(ctx: &mut StartupContext<'_>) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    let (cube_vertices, cube_indices) = wgpu_cube::renderer::cube_mesh();
    let cube_mesh = renderer.create_mesh(&cube_vertices, &cube_indices);
    let cube_handle = scene.assets.meshes.insert(cube_mesh);

    let (quad_vertices, quad_indices) = wgpu_cube::renderer::quad_mesh();
    let quad_mesh = renderer.create_mesh(&quad_vertices, &quad_indices);
    let quad_handle = scene.assets.meshes.insert(quad_mesh);

    // Ground plane to provide context.
    let floor_material = scene.assets.materials.insert(MaterialAsset::from_material(
        Material::new([70, 80, 90, 255]).with_roughness(1.0),
        PathBuf::from("examples/z_buffer/floor"),
    ));
    EntityBuilder::new(scene)
        .with_name("Test Floor")
        .with_transform(Transform::from_trs(
            Vec3::new(0.0, -0.05, 0.0),
            Quat::IDENTITY,
            Vec3::new(12.0, 0.1, 12.0),
        ))
        .with_mesh(cube_handle)
        .with_material(floor_material)
        .visible(true)
        .spawn();

    // Intersecting cubes show which surfaces win the depth test.
    let red_material = scene.assets.materials.insert(MaterialAsset::from_material(
        Material::new([205, 75, 65, 255]).with_roughness(0.4),
        PathBuf::from("examples/z_buffer/front_cube"),
    ));
    EntityBuilder::new(scene)
        .with_name("Front Cube")
        .with_transform(Transform::from_trs(
            Vec3::new(-0.35, 0.6, -0.2),
            Quat::IDENTITY,
            Vec3::splat(1.2),
        ))
        .with_mesh(cube_handle)
        .with_material(red_material)
        .visible(true)
        .spawn();

    let blue_material = scene.assets.materials.insert(MaterialAsset::from_material(
        Material::new([65, 115, 205, 255]).with_roughness(0.35),
        PathBuf::from("examples/z_buffer/back_cube"),
    ));
    EntityBuilder::new(scene)
        .with_name("Back Cube")
        .with_transform(Transform::from_trs(
            Vec3::new(0.35, 0.6, -0.05),
            Quat::IDENTITY,
            Vec3::splat(1.2),
        ))
        .with_mesh(cube_handle)
        .with_material(blue_material)
        .visible(true)
        .spawn();

    // Cube with depth writes disabled still tests depth but lets previously drawn pixels win.
    let no_depth_write = scene.assets.materials.insert(MaterialAsset::from_material(
        Material::new([60, 200, 120, 200])
            .with_roughness(0.2)
            .with_alpha(),
        PathBuf::from("examples/z_buffer/no_depth_write"),
    ));

    scene.world_mut().spawn((
        Name::new("No Depth Write Cube"),
        TransformComponent(Transform::from_trs(
            Vec3::new(0.0, 0.6, 1.1),
            Quat::IDENTITY,
            Vec3::splat(1.0),
        )),
        MeshComponent(cube_handle),
        MaterialComponent(no_depth_write),
        //DepthState::new(true, false),
        Visible(true),
    ));

    // Cube with depth testing disabled ignores the Z-buffer and always draws last.
    let depth_disabled = scene.assets.materials.insert(MaterialAsset::from_material(
        Material::new([245, 210, 90, 255]).with_roughness(0.15),
        PathBuf::from("examples/z_buffer/depth_disabled"),
    ));

    scene.world_mut().spawn((
        Name::new("Depth Test Disabled Cube"),
        TransformComponent(Transform::from_trs(
            Vec3::new(0.0, 0.6, -1.4),
            Quat::IDENTITY,
            Vec3::splat(1.0),
        )),
        MeshComponent(cube_handle),
        MaterialComponent(depth_disabled),
        //DepthState::new(false, true),
        Visible(true),
    ));

    // Pair of quads almost coplanar to highlight z-fighting.
    let base_plate = Transform::from_trs(
        Vec3::new(-2.5, 0.02, 0.0),
        Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
        Vec3::splat(2.5),
    );
    let reference_quad_material = scene.assets.materials.insert(MaterialAsset::from_material(
        Material::new([220, 220, 220, 255]).with_roughness(0.9),
        PathBuf::from("examples/z_buffer/reference_quad"),
    ));

    scene.world_mut().spawn((
        Name::new("Reference Quad"),
        TransformComponent(base_plate),
        MeshComponent(quad_handle),
        MaterialComponent(reference_quad_material),
        Visible(true),
    ));
    let offset_quad_material = scene.assets.materials.insert(MaterialAsset::from_material(
        Material::new([200, 60, 60, 180])
            .with_roughness(0.3)
            .with_alpha(),
        PathBuf::from("examples/z_buffer/offset_quad"),
    ));

    scene.world_mut().spawn((
        Name::new("Offset Quad"),
        TransformComponent(Transform::from_trs(
            Vec3::new(-2.5, 0.022, 0.0),
            base_plate.rotation,
            base_plate.scale,
        )),
        MeshComponent(quad_handle),
        MaterialComponent(offset_quad_material),
        DepthState::new(true, true),
        Visible(true),
    ));

    // Simple point light to keep the scene readable.
    scene.world_mut().spawn((
        Name::new("Z Buffer Light"),
        TransformComponent(Transform::from_trs(
            Vec3::new(4.0, 5.0, 4.0),
            Quat::IDENTITY,
            Vec3::ONE,
        )),
        PointLight {
            color: Vec3::splat(1.0),
            intensity: 420.0,
            range: 14.0,
        },
        CanCastShadow(false),
    ));
}

fn orbit_camera(ctx: &mut UpdateContext<'_>, radius: f32, height: f32) {
    let t = ctx.scene.time() as f32 * 0.5;
    let camera = ctx.scene.camera_mut();
    camera.eye = Vec3::new(t.cos() * radius, height, t.sin() * radius);
    camera.target = Vec3::new(0.0, 0.5, 0.0);
    camera.up = Vec3::Y;
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(ZBufferExample).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    run_application(ZBufferExample).unwrap();
}
