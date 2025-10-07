use glam::{Quat, Vec3};
use log::info;
use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::renderer::{Material, Texture};
use wgpu_cube::scene::components::{
    Billboard, BillboardOrientation, Children, Parent, RotateAnimation,
};
use wgpu_cube::scene::{
    MaterialComponent, MeshComponent, Name, Transform, TransformComponent, Visible,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

fn build_app() -> AppBuilder {
    let mut builder = AppBuilder::new();
    builder.add_startup_system(setup_hierarchy_scene);
    builder.add_system(orbit_camera(15.0, 8.0));
    builder
}

fn setup_hierarchy_scene(ctx: &mut StartupContext<'_>) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    info!("Creating hierarchy test scene...");

    let (verts, idx) = wgpu_cube::renderer::cube_mesh();
    let cube_mesh = renderer.create_mesh(&verts, &idx);
    let cube_handle = scene.assets.meshes.insert(cube_mesh);

    let colors = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [255, 255, 0, 255],
        [255, 0, 255, 255],
    ];

    for color in colors {
        let texture = Texture::from_color(renderer.get_device(), renderer.get_queue(), color, None);
        scene.assets.textures.insert(texture);
    }

    let parent1 = scene.world.spawn((
        Name::new("Parent1 (Red)"),
        TransformComponent(Transform::from_trs(
            Vec3::new(-6.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::ONE,
        )),
        MeshComponent(cube_handle),
        MaterialComponent(Material::white().with_texture(0)),
        Visible(true),
    ));

    let child1 = scene.world.spawn((
        Name::new("Child1 (Green)"),
        TransformComponent(Transform::from_trs(
            Vec3::new(2.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::splat(0.5),
        )),
        MeshComponent(cube_handle),
        MaterialComponent(Material::white().with_texture(1)),
        Visible(true),
        Parent(parent1),
    ));

    scene.world.insert_one(parent1, Children(vec![child1])).ok();

    let grandparent = scene.world.spawn((
        Name::new("Grandparent (Blue)"),
        TransformComponent(Transform::from_trs(Vec3::ZERO, Quat::IDENTITY, Vec3::ONE)),
        MeshComponent(cube_handle),
        MaterialComponent(Material::white().with_texture(2)),
        Visible(true),
    ));

    let parent2 = scene.world.spawn((
        Name::new("Parent2 (Yellow)"),
        TransformComponent(Transform::from_trs(
            Vec3::new(0.0, 2.0, 0.0),
            Quat::IDENTITY,
            Vec3::splat(0.8),
        )),
        MeshComponent(cube_handle),
        MaterialComponent(Material::white().with_texture(3)),
        Visible(true),
        Parent(grandparent),
    ));

    let child2 = scene.world.spawn((
        Name::new("Child2 (Magenta)"),
        TransformComponent(Transform::from_trs(
            Vec3::new(0.0, 1.5, 0.0),
            Quat::IDENTITY,
            Vec3::splat(0.6),
        )),
        MeshComponent(cube_handle),
        MaterialComponent(Material::white().with_texture(4)),
        Visible(true),
        Parent(parent2),
    ));

    scene
        .world
        .insert_one(grandparent, Children(vec![parent2]))
        .ok();
    scene.world.insert_one(parent2, Children(vec![child2])).ok();

    let rotating_parent = scene.world.spawn((
        Name::new("Rotating Parent (Red)"),
        TransformComponent(Transform::from_trs(
            Vec3::new(6.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::ONE,
        )),
        MeshComponent(cube_handle),
        MaterialComponent(Material::white().with_texture(0)),
        Visible(true),
        RotateAnimation {
            axis: Vec3::Y,
            speed: 1.0,
        },
    ));

    let rotating_child = scene.world.spawn((
        Name::new("Rotating Child (Green)"),
        TransformComponent(Transform::from_trs(
            Vec3::new(3.0, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::splat(0.5),
        )),
        MeshComponent(cube_handle),
        MaterialComponent(Material::white().with_texture(1)),
        Visible(true),
        Parent(rotating_parent),
    ));

    scene
        .world
        .insert_one(rotating_parent, Children(vec![rotating_child]))
        .ok();

    let scaled_parent = scene.world.spawn((
        Name::new("Scaled Parent (Blue)"),
        TransformComponent(Transform::from_trs(
            Vec3::new(0.0, -3.0, 0.0),
            Quat::IDENTITY,
            Vec3::splat(2.0),
        )),
        MeshComponent(cube_handle),
        MaterialComponent(Material::white().with_texture(2)),
        Visible(true),
    ));

    let scaled_child = scene.world.spawn((
        Name::new("Scaled Child (Yellow)"),
        TransformComponent(Transform::from_trs(
            Vec3::new(1.5, 0.0, 0.0),
            Quat::IDENTITY,
            Vec3::splat(0.5),
        )),
        MeshComponent(cube_handle),
        MaterialComponent(Material::white().with_texture(3)),
        Visible(true),
        Parent(scaled_parent),
    ));

    scene
        .world
        .insert_one(scaled_parent, Children(vec![scaled_child]))
        .ok();

    let billboard_parent = scene.world.spawn((
        Name::new("Billboard Parent"),
        TransformComponent(Transform::from_trs(
            Vec3::new(0.0, 0.0, -6.0),
            Quat::IDENTITY,
            Vec3::splat(1.0),
        )),
        MeshComponent(cube_handle),
        MaterialComponent(Material::white().with_texture(0)),
        Visible(true),
    ));

    let billboard_child = scene.world.spawn((
        Name::new("Billboard Child"),
        TransformComponent(Transform::from_trs(
            Vec3::new(0.0, 2.0, 0.0),
            Quat::IDENTITY,
            Vec3::splat(0.6),
        )),
        MeshComponent(cube_handle),
        MaterialComponent(Material::white().with_texture(1)),
        Visible(true),
        Parent(billboard_parent),
        Billboard::new(BillboardOrientation::FaceCameraYAxis),
    ));

    scene
        .world
        .insert_one(billboard_parent, Children(vec![billboard_child]))
        .ok();

    info!(
        "Hierarchy test scene created: {} entities",
        scene.world.len()
    );
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
