use glam::{Quat, Vec3};
use wgpu_cube::app::{AppBuilder, StartupContext};
use wgpu_cube::render_application::{run_application, RenderApplication};
use wgpu_cube::renderer::{Material, Texture};
use wgpu_cube::scene::components::{CanCastShadow, DirectionalLight};
use wgpu_cube::scene::{
    MaterialComponent, MeshComponent, Name, Transform, TransformComponent, Visible,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

struct RoughnessRampApp;

impl RenderApplication for RoughnessRampApp {
    fn configure(&self, builder: &mut AppBuilder) {
        builder.disable_default_lighting();
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        setup_scene(ctx);
    }
}

fn setup_scene(ctx: &mut StartupContext<'_>) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    let (verts, idx) = wgpu_cube::renderer::sphere_mesh(64, 32);
    let sphere_mesh = renderer.create_mesh(&verts, &idx);
    let sphere_handle = scene.assets.meshes.insert(sphere_mesh);

    let unit_mr = Texture::from_color_linear(
        renderer.get_device(),
        renderer.get_queue(),
        [255, 255, 255, 255],
        Some("RoughnessRamp_MR"),
    );
    let mr_handle = scene.assets.textures.insert(unit_mr);
    renderer.update_texture_bind_group(&scene.assets);

    let count = 10;
    let spacing = 2.0;
    let start_x = -((count - 1) as f32 * spacing) * 0.5;

    for i in 0..count {
        let roughness = i as f32 / (count - 1) as f32;
        let material = Material::new([230, 230, 230, 255])
            .with_metallic(1.0)
            .with_roughness(roughness)
            .with_metallic_roughness_texture(mr_handle.index() as u32);

        scene.world.spawn((
            Name::new(format!("Sphere_R{roughness:.2}")),
            TransformComponent(Transform::from_trs(
                Vec3::new(start_x + i as f32 * spacing, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::splat(0.9),
            )),
            MeshComponent(sphere_handle),
            MaterialComponent(material),
            Visible(true),
        ));
    }

    scene.world.spawn((
        Name::new("Key Light"),
        TransformComponent(Transform::from_trs(
            Vec3::ZERO,
            Quat::from_rotation_arc(Vec3::NEG_Z, Vec3::new(-0.5, -1.0, -0.3).normalize()),
            Vec3::ONE,
        )),
        DirectionalLight::new(Vec3::new(1.0, 0.98, 0.92), 2.0),
        CanCastShadow(true),
    ));

    // Slightly elevate camera to get a better overview.
    let camera = scene.camera_mut();
    camera.eye = Vec3::new(0.0, 2.5, 8.0);
    camera.target = Vec3::new(0.0, 0.5, 0.0);
    camera.up = Vec3::Y;
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(RoughnessRampApp).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    run_application(RoughnessRampApp).unwrap();
}
