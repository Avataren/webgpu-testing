use std::path::Path;

use glam::{Quat, Vec3};
use log::info;
use wgpu_cube::app::{AppBuilder, Plugin, StartupContext, UpdateContext};
use wgpu_cube::renderer::{Material, Texture};
use wgpu_cube::scene::components::{
    Billboard, BillboardOrientation, BillboardSpace, CanCastShadow, DepthState, DirectionalLight,
    PointLight,
};
use wgpu_cube::scene::{
    Children, EntityBuilder, MaterialComponent, MeshComponent, Name, Parent, RotateAnimation,
    SceneLoader, Transform, TransformComponent, Visible,
};

#[allow(dead_code)]
#[derive(Clone, Copy, Debug)]
pub enum DemoScene {
    Simple,
    Grid { size: i32 },
    HierarchyTest,
    ShadowTest,
    PbrTest,
    Gltf { path: &'static str, scale: f32 },
}

impl DemoScene {
    pub fn plugin(self) -> DemoScenePlugin {
        DemoScenePlugin::new(self)
    }
}

#[allow(dead_code)]
pub fn default_scene() -> DemoScene {
    DemoScene::ShadowTest
}

pub struct DemoScenePlugin {
    scene: DemoScene,
}

impl DemoScenePlugin {
    pub fn new(scene: DemoScene) -> Self {
        Self { scene }
    }
}

impl Plugin for DemoScenePlugin {
    fn build(&self, app: &mut AppBuilder) {
        match self.scene {
            DemoScene::Simple => {
                app.add_startup_system(setup_simple_scene);
                app.add_system(orbit_camera(8.0, 4.0));
            }
            DemoScene::Grid { size } => {
                app.add_startup_system(move |ctx: &mut StartupContext<'_>| {
                    setup_grid_scene(ctx, size)
                });
                app.add_system(orbit_camera(15.0, 8.0));
            }
            DemoScene::HierarchyTest => {
                app.add_startup_system(setup_hierarchy_test_scene);
                app.add_system(orbit_camera(15.0, 8.0));
            }
            DemoScene::ShadowTest => {
                app.disable_default_lighting();
                app.add_startup_system(setup_shadow_test_scene);
                app.add_system(orbit_camera(12.0, 6.0));
            }
            DemoScene::PbrTest => {
                app.add_startup_system(setup_pbr_test_scene);
                app.add_system(orbit_camera(8.0, 2.0));
            }
            DemoScene::Gltf { path, scale } => {
                app.disable_default_textures();
                app.disable_default_lighting();
                app.add_startup_system(move |ctx: &mut StartupContext<'_>| {
                    load_gltf_scene(ctx, path, scale)
                });
                let factor = scale.log10().max(0.5);
                app.add_system(orbit_camera(5.0 * factor, 2.0 * factor));
                app.skip_initial_frames(5);
            }
        }
    }
}

fn orbit_camera(
    radius: f32,
    height: f32,
) -> Box<dyn for<'a> FnMut(&mut UpdateContext<'a>) + 'static> {
    Box::new(move |ctx: &mut UpdateContext<'_>| {
        let t = ctx.scene.time() as f32;
        ctx.camera.eye = Vec3::new(t.cos() * radius, height, t.sin() * radius);
        ctx.camera.target = Vec3::ZERO;
        ctx.camera.up = Vec3::Y;
    })
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

fn setup_grid_scene(ctx: &mut StartupContext<'_>, size: i32) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    info!("Creating grid scene...");

    let (verts, idx) = wgpu_cube::renderer::cube_mesh();
    let cube_mesh = renderer.create_mesh(&verts, &idx);
    let cube_handle = scene.assets.meshes.insert(cube_mesh);

    let colors = [
        [255, 100, 100, 255],
        [100, 255, 100, 255],
        [100, 100, 255, 255],
        [255, 255, 100, 255],
        [255, 100, 255, 255],
    ];

    for color in colors {
        let texture = Texture::from_color(renderer.get_device(), renderer.get_queue(), color, None);
        scene.assets.textures.insert(texture);
    }

    let spacing = 2.0;
    for x in -size..=size {
        for z in -size..=size {
            let pos = Vec3::new(x as f32 * spacing, 0.0, z as f32 * spacing);
            let texture_idx = ((x.abs() + z.abs()) % 5) as u32;

            scene.world.spawn((
                Name::new(format!("Cube_{}_{}", x, z)),
                TransformComponent(Transform::from_trs(pos, Quat::IDENTITY, Vec3::splat(0.4))),
                MeshComponent(cube_handle),
                MaterialComponent(Material::white().with_texture(texture_idx)),
                Visible(true),
            ));
        }
    }

    info!("Grid scene: {} entities", scene.world.len());
}

fn setup_hierarchy_test_scene(ctx: &mut StartupContext<'_>) {
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

fn setup_shadow_test_scene(ctx: &mut StartupContext<'_>) {
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

    let sun_direction = Vec3::new(-0.6, -1.0, -0.4).normalize();
    let sun_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, sun_direction);

    scene.world.spawn((
        Name::new("Shadow Test Sun"),
        TransformComponent(Transform::from_trs(Vec3::ZERO, sun_rotation, Vec3::ONE)),
        DirectionalLight {
            color: Vec3::splat(1.0),
            intensity: 6.0,
        },
        CanCastShadow(true),
    ));

    scene.world.spawn((
        Name::new("Shadow Test Fill"),
        TransformComponent(Transform::from_trs(
            Vec3::new(3.0, 4.0, 2.0),
            Quat::IDENTITY,
            Vec3::ONE,
        )),
        PointLight {
            color: Vec3::new(0.9, 0.95, 1.0),
            intensity: 2.0,
            range: 20.0,
        },
        CanCastShadow(false),
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

    let sprite_offset = Vec3::new(3.0, 2.2, 8.0);
    let sprite_transform = Transform::from_trs(sprite_offset, Quat::IDENTITY, Vec3::splat(2.5));

    let billboard =
        Billboard::new(BillboardOrientation::FaceCamera).with_space(BillboardSpace::View {
            offset: sprite_offset,
        });

    scene.world.spawn((
        Name::new("Shadow Test Sprite"),
        TransformComponent(sprite_transform),
        MeshComponent(quad_handle),
        MaterialComponent(sprite_material),
        billboard,
        DepthState::new(false, false),
        Visible(true),
    ));

    renderer.update_texture_bind_group(&scene.assets);

    info!("Shadow test scene created: {} entities", scene.world.len());
}

fn setup_pbr_test_scene(ctx: &mut StartupContext<'_>) {
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

    info!("PBR test scene: {} entities", scene.world.len());
}

fn load_gltf_scene(ctx: &mut StartupContext<'_>, path: &'static str, scale: f32) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    info!("Loading glTF: {} (scale: {})", path, scale);

    match SceneLoader::load_gltf(path, scene, renderer, scale) {
        Ok(_) => {
            let sun_direction = Vec3::new(-0.6, -1.0, -0.4).normalize();
            let sun2_direction = Vec3::new(0.3, -1.0, -0.7).normalize();
            let sun_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, sun_direction);
            let sun2_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, sun2_direction);

            scene.world.spawn((
                Name::new("Shadow Test Sun"),
                TransformComponent(Transform::from_trs(Vec3::ZERO, sun_rotation, Vec3::ONE)),
                DirectionalLight {
                    color: Vec3::splat(1.0),
                    intensity: 3.0,
                },
                CanCastShadow(true),
            ));

            scene.world.spawn((
                Name::new("Shadow Test Sun 2"),
                TransformComponent(Transform::from_trs(Vec3::ZERO, sun2_rotation, Vec3::ONE)),
                DirectionalLight {
                    color: Vec3::splat(1.0),
                    intensity: 3.0,
                },
                CanCastShadow(true),
            ));

            renderer.update_texture_bind_group(&scene.assets);
            info!("glTF loaded: {} entities", scene.world.len());
        }
        Err(err) => {
            log::error!("Failed to load glTF: {}", err);
        }
    }
}
