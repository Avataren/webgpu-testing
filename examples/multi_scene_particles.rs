use glam::{Quat, Vec3};
use rand::{rngs::SmallRng, Rng, SeedableRng};
use std::f32::consts::TAU;
use std::path::PathBuf;

use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::asset::{Handle, MaterialAsset, Mesh};
use wgpu_cube::gpu_particles::behaviors::StarfieldBehavior;
use wgpu_cube::gpu_particles::{GpuParticleSystem, Particle};
use wgpu_cube::render_application::{run_application, RenderApplication};
use wgpu_cube::renderer::Material;
use wgpu_cube::scene::components::{CanCastShadow, DirectionalLight};
use wgpu_cube::scene::{EntityBuilder, SceneNodeId, Transform};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const STAR_COUNT: usize = 2000;
const FIELD_HALF_SIZE: f32 = 48.0;
const NEAR_PLANE: f32 = 0.05;
const FAR_PLANE: f32 = 200.0;
const FAR_RESET_BAND: f32 = 6.0;
const STAR_SPEED_RANGE: std::ops::Range<f32> = 6.0..16.0;
const SPIN_SPEED_RANGE: std::ops::Range<f32> = 0.5..3.5;
const STAR_SCALE_RANGE: std::ops::Range<f32> = 1.25..1.5;
const MIN_RADIUS_FROM_CENTER: f32 = 1.25;
const STAR_BASE_HEIGHT: f32 = 3.0;
const STAR_HEIGHT_RANGE: f32 = 7.0;

const FLOOR_DIMENSION: i32 = 10; // 10x10 grid => 100 cubes
const TILE_SPACING: f32 = 3.0;
const TILE_SCALE: Vec3 = Vec3::new(2.8, 0.25, 2.8);

const CAMERA_RADIUS: f32 = 26.0;
const CAMERA_HEIGHT: f32 = 11.0;

struct MultiSceneParticlesExample {
    star_system: Option<GpuParticleSystem>,
    cube_mesh: Option<Handle<Mesh>>,
    star_behavior: StarfieldBehavior,
}

impl MultiSceneParticlesExample {
    fn new() -> Self {
        Self {
            star_system: None,
            cube_mesh: None,
            star_behavior: StarfieldBehavior {
                near_plane: NEAR_PLANE,
                far_plane: FAR_PLANE,
                far_reset_band: FAR_RESET_BAND,
                field_half_size: FIELD_HALF_SIZE,
                min_radius: MIN_RADIUS_FROM_CENTER,
            },
        }
    }

    fn build_particles(&self) -> Vec<Particle> {
        (0..STAR_COUNT as u32)
            .map(|i| {
                let mut rng = SmallRng::seed_from_u64(i as u64);

                let position = random_initial_position(&mut rng);
                let rotation_axis = random_unit_vector(&mut rng);
                let rotation_angle = rng.gen_range(0.0..TAU);
                let scale = random_scale(&mut rng);
                let speed = rng.gen_range(STAR_SPEED_RANGE);
                let angular_speed = rng.gen_range(SPIN_SPEED_RANGE);

                let color_rgba = [1.0, 1.0, 1.0, 1.0];
                let mut color_key_times = [1.0; Particle::MAX_COLOR_KEYS];
                color_key_times[0] = 0.0;

                Particle {
                    position: position.into(),
                    lifetime: rng.gen_range(0.0..500.0),
                    velocity: [0.0, 0.0, speed],
                    max_lifetime: f32::INFINITY,
                    rotation: [
                        rotation_axis.x,
                        rotation_axis.y,
                        rotation_axis.z,
                        rotation_angle,
                    ],
                    scale: [scale, scale, scale],
                    angular_velocity: angular_speed,
                    color: color_rgba,
                    spawn_color: color_rgba,
                    color_keys: [color_rgba; Particle::MAX_COLOR_KEYS],
                    color_key_times,
                    user_data: [0.0, 0.0, 1.0, 0.0],
                }
            })
            .collect()
    }

    fn setup_star_scene(&mut self, ctx: &mut StartupContext<'_>, node: SceneNodeId) {
        let scene = &mut *ctx.scene;
        let renderer = &mut *ctx.renderer;
        let original_main = scene.main_scene();
        scene.set_main_scene(node);

        {
            let world = scene.world_mut();
            let light_direction = Vec3::new(-0.35, -1.0, -0.2).normalize();
            let light_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, light_direction);

            world.spawn((
                wgpu_cube::scene::Name::new("Starfield Sun"),
                wgpu_cube::scene::TransformComponent(Transform::from_trs(
                    Vec3::new(0.0, 18.0, 14.0),
                    light_rotation,
                    Vec3::ONE,
                )),
                DirectionalLight::new(Vec3::new(1.0, 0.97, 0.92), 6.5).with_shadow_size(64.0),
                CanCastShadow(true),
            ));
        }

        let (vertices, indices) = wgpu_cube::renderer::cube_mesh();
        let mesh = renderer.create_mesh(&vertices, &indices);
        let mesh_handle = scene.assets.meshes.insert(mesh);
        self.cube_mesh = Some(mesh_handle);

        let material = Material::checker().with_metallic(0.08).with_roughness(0.65);
        let mut system = GpuParticleSystem::new(
            renderer.get_device(),
            renderer.get_queue(),
            renderer,
            STAR_COUNT as u32,
            material,
            &self.star_behavior,
        );
        system.set_casts_shadows(true);

        let particles = self.build_particles();
        system.initialize_particles(renderer.get_queue(), &particles);

        self.star_system = Some(system);
        scene.set_main_scene(original_main);
    }

    fn setup_floor_scene(
        &mut self,
        ctx: &mut StartupContext<'_>,
        node: SceneNodeId,
        mesh_handle: Handle<Mesh>,
    ) {
        let scene = &mut *ctx.scene;
        let original_main = scene.main_scene();
        scene.set_main_scene(node);

        let dark_handle = scene.assets.materials.insert(MaterialAsset::from_material(
            Material::rgb(40, 48, 62).with_roughness(0.85),
            PathBuf::from("examples/multi_scene_particles/floor_dark"),
        ));
        let light_handle = scene.assets.materials.insert(MaterialAsset::from_material(
            Material::rgb(215, 220, 235).with_roughness(0.55),
            PathBuf::from("examples/multi_scene_particles/floor_light"),
        ));

        {
            let half_extent = (FLOOR_DIMENSION as f32 - 1.0) * 0.5 * TILE_SPACING;

            for z in 0..FLOOR_DIMENSION {
                for x in 0..FLOOR_DIMENSION {
                    let offset = Vec3::new(
                        (x as f32 * TILE_SPACING) - half_extent,
                        -0.5,
                        (z as f32 * TILE_SPACING) - half_extent,
                    );
                    let transform = Transform::from_trs(offset, Quat::IDENTITY, TILE_SCALE);
                    let material_handle = if (x + z) % 2 == 0 {
                        dark_handle
                    } else {
                        light_handle
                    };

                    EntityBuilder::new(scene)
                        .with_name(format!("FloorTile_{}_{}", x, z))
                        .with_transform(transform)
                        .with_mesh(mesh_handle)
                        .with_material(material_handle)
                        .visible(true)
                        .spawn();
                }
            }

            let light_direction = Vec3::new(0.45, -1.0, 0.35).normalize();
            let light_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, light_direction);

            scene.world_mut().spawn((
                wgpu_cube::scene::Name::new("Floor Sun"),
                wgpu_cube::scene::TransformComponent(Transform::from_trs(
                    Vec3::new(0.0, 22.0, -18.0),
                    light_rotation,
                    Vec3::ONE,
                )),
                DirectionalLight::new(Vec3::splat(1.0), 7.5).with_shadow_size(70.0),
                CanCastShadow(true),
            ));
        }
        scene.set_main_scene(original_main);
    }
}

impl RenderApplication for MultiSceneParticlesExample {
    fn name(&self) -> &str {
        "Multi-scene starfield over floor"
    }

    fn configure(&self, builder: &mut AppBuilder) {
        builder.disable_default_lighting();
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        let (star_node, floor_node) = {
            let scene = &mut *ctx.scene;
            scene.environment_mut().set_clear_color(wgpu::Color {
                r: 0.01,
                g: 0.015,
                b: 0.03,
                a: 1.0,
            });

            let root = scene.root_id();
            let star_node = scene.create_node("StarfieldScene", Some(root));
            let floor_node = scene.create_node("CheckeredFloorScene", Some(root));

            scene.node_local_transform_mut(star_node).translation = Vec3::ZERO;
            scene.node_local_transform_mut(floor_node).translation = Vec3::ZERO;

            (star_node, floor_node)
        };

        self.setup_star_scene(ctx, star_node);
        let mesh_handle = self
            .cube_mesh
            .expect("star scene should set cube mesh before floor scene");
        self.setup_floor_scene(ctx, floor_node, mesh_handle);

        {
            let scene = &mut *ctx.scene;
            scene.update(0.0);
            let mut camera = wgpu_cube::scene::Camera::default();
            camera.eye = Vec3::new(0.0, CAMERA_HEIGHT, CAMERA_RADIUS);
            camera.target = Vec3::new(0.0, STAR_BASE_HEIGHT + STAR_HEIGHT_RANGE * 0.5, 0.0);
            camera.up = Vec3::Y;
            camera.set_projection(wgpu_cube::scene::CameraProjection::perspective(
                60f32.to_radians(),
                NEAR_PLANE,
                FAR_PLANE,
            ));
            scene.set_camera(camera);
        }
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        let time = ctx.scene.time() as f32;
        let orbit = time * 0.15;

        let camera = ctx.scene.camera_mut();
        camera.eye = Vec3::new(
            orbit.cos() * CAMERA_RADIUS,
            CAMERA_HEIGHT,
            orbit.sin() * CAMERA_RADIUS,
        );
        camera.target = Vec3::new(0.0, STAR_BASE_HEIGHT + STAR_HEIGHT_RANGE * 0.4, 0.0);
        camera.up = Vec3::Y;
    }

    fn gpu_update(&mut self, ctx: &mut wgpu_cube::GpuUpdateContext) {
        if let Some(system) = &mut self.star_system {
            let mut encoder =
                ctx.renderer
                    .get_device()
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("MultiSceneStarfieldUpdate"),
                    });

            system.update(
                ctx.renderer.get_device(),
                ctx.renderer.get_queue(),
                &mut encoder,
                &self.star_behavior,
                ctx.dt as f32,
            );

            ctx.renderer.get_queue().submit(Some(encoder.finish()));
        }
    }

    fn custom_render(&mut self, ctx: &mut wgpu_cube::renderer::CustomRenderContext) {
        if let (Some(system), Some(mesh_handle)) = (&mut self.star_system, self.cube_mesh) {
            if let Some(mesh) = ctx.scene.assets.meshes.get(mesh_handle) {
                system.render(ctx, mesh);
            }
        }
    }

    fn custom_render_includes_shadows(&self) -> bool {
        self.star_system
            .as_ref()
            .is_some_and(|system| system.casts_shadows())
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .init();
    run_application(MultiSceneParticlesExample::new()).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Info).expect("failed to init logger");
    run_application(MultiSceneParticlesExample::new()).unwrap();
}

fn random_initial_position(rng: &mut SmallRng) -> Vec3 {
    loop {
        let x = rng.gen_range(-FIELD_HALF_SIZE..FIELD_HALF_SIZE);
        let y = rng.gen_range(STAR_BASE_HEIGHT..STAR_BASE_HEIGHT + STAR_HEIGHT_RANGE);

        let radial = Vec3::new(x, y - (STAR_BASE_HEIGHT + STAR_HEIGHT_RANGE * 0.5), 0.0);
        if radial.length() >= MIN_RADIUS_FROM_CENTER {
            let z = -rng.gen_range(NEAR_PLANE..FAR_PLANE);
            return Vec3::new(x, y, z);
        }
    }
}

fn random_unit_vector(rng: &mut SmallRng) -> Vec3 {
    let mut v = Vec3::new(
        rng.gen_range(-1.0..=1.0),
        rng.gen_range(-1.0..=1.0),
        rng.gen_range(-1.0..=1.0),
    );
    if v.length_squared() < 1e-6 {
        v = Vec3::Y;
    }
    v.normalize()
}

fn random_scale(rng: &mut SmallRng) -> f32 {
    rng.gen_range(STAR_SCALE_RANGE)
}
