use glam::{Quat, Vec3};
use rand::{rngs::SmallRng, Rng, SeedableRng};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::gpu_particles::{GpuParticleSystem, ParticleFieldSettings, ParticleInit};
use wgpu_cube::render_application::{run_application, RenderApplication};
use wgpu_cube::renderer::Material;
use wgpu_cube::scene::components::{
    CanCastShadow, DirectionalLight, GpuParticleInstance, MaterialComponent, MeshComponent, Name,
    TransformComponent, Visible,
};
use wgpu_cube::scene::{Camera, Transform};

const STAR_COUNT: usize = 150_000;
const FIELD_HALF_SIZE: f32 = 60.0;
const NEAR_PLANE: f32 = 0.01;
const FAR_PLANE: f32 = 150.0;
const FAR_RESET_BAND: f32 = 25.0;
const STAR_SPEED_RANGE: std::ops::Range<f32> = 5.0..15.0;
const SPIN_SPEED_RANGE: std::ops::Range<f32> = 0.1..3.5;
const STAR_SCALE_RANGE: std::ops::Range<f32> = 0.25..0.5;
const MIN_SIZE_FROM_CENTER: f32 = 0.5;

struct StarfieldGpuApp {
    rng: SmallRng,
    particles: Option<GpuParticleSystem>,
}

impl Default for StarfieldGpuApp {
    fn default() -> Self {
        Self {
            rng: SmallRng::seed_from_u64(0x5EED_CAFE),
            particles: None,
        }
    }
}

impl RenderApplication for StarfieldGpuApp {
    fn name(&self) -> &str {
        "3D Starfield (GPU)"
    }

    fn configure(&self, builder: &mut AppBuilder) {
        builder.disable_default_lighting();
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        let (verts, idx) = wgpu_cube::renderer::cube_mesh();
        let mesh = ctx.renderer.create_mesh(&verts, &idx);
        let mesh_handle = ctx.scene.assets.meshes.insert(mesh);

        let mut material = Material::checker();
        material.roughness_factor = 64;

        ctx.scene.environment_mut().set_clear_color(wgpu::Color {
            r: 0.001,
            g: 0.005,
            b: 0.01,
            a: 1.0,
        });
        ctx.scene.environment_mut().disable_hdr_background();
        ctx.scene.set_camera(Camera {
            eye: Vec3::ZERO,
            target: Vec3::new(0.0, 0.0, -1.0),
            up: Vec3::Y,
            near: NEAR_PLANE,
            far: FAR_PLANE,
            ..Camera::default()
        });

        let sun_direction = Vec3::new(0.3, -1.0, -1.1).normalize();
        let sun_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, sun_direction);

        ctx.scene.world.spawn((
            Name::new("Default Sky Light"),
            TransformComponent(Transform::from_trs(Vec3::ZERO, sun_rotation, Vec3::ONE)),
            DirectionalLight::new(Vec3::new(0.49, 0.95, 0.85), 2.5),
            CanCastShadow(false),
        ));

        let mut initial_particles = Vec::with_capacity(STAR_COUNT);

        for index in 0..STAR_COUNT {
            let position = random_initial_position(&mut self.rng);
            let rotation = random_rotation(&mut self.rng);
            let scale = random_scale(&mut self.rng);
            let speed = self.rng.gen_range(STAR_SPEED_RANGE.clone());
            let angular_speed = self.rng.gen_range(SPIN_SPEED_RANGE.clone());
            let angular_axis = random_unit_vector(&mut self.rng);
            let seed = self.rng.gen();

            let transform = Transform::from_trs(position, rotation, Vec3::splat(scale));
            ctx.scene.world.spawn((
                TransformComponent(transform),
                MeshComponent(mesh_handle),
                MaterialComponent(material),
                Visible(true),
                GpuParticleInstance {
                    index: index as u32,
                },
            ));

            initial_particles.push(ParticleInit {
                position,
                speed,
                rotation,
                angular_axis,
                angular_speed,
                scale,
                seed,
            });
        }

        let settings = ParticleFieldSettings {
            near_plane: NEAR_PLANE,
            far_plane: FAR_PLANE,
            far_reset_band: FAR_RESET_BAND,
            field_half_size: FIELD_HALF_SIZE,
            min_radius: MIN_SIZE_FROM_CENTER,
            speed_range: STAR_SPEED_RANGE.clone(),
            spin_range: SPIN_SPEED_RANGE.clone(),
            scale_range: STAR_SCALE_RANGE.clone(),
        };

        let system = GpuParticleSystem::new(
            ctx.renderer,
            STAR_COUNT as u32,
            0,
            0,
            settings,
            &initial_particles,
        );

        self.particles = Some(system);
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        let _ = ctx;
    }

    fn gpu_update(&mut self, ctx: &mut wgpu_cube::app::GpuUpdateContext) {
        if let Some(system) = self.particles.as_mut() {
            system.update(ctx.renderer, ctx.dt as f32);
        }
    }
}

fn random_initial_position(rng: &mut SmallRng) -> Vec3 {
    let mut x: f32 = 0.0;
    let mut y: f32 = 0.0;
    while (x * x + y * y).sqrt() < MIN_SIZE_FROM_CENTER {
        x = rng.gen_range(-FIELD_HALF_SIZE..FIELD_HALF_SIZE);
        y = rng.gen_range(-FIELD_HALF_SIZE..FIELD_HALF_SIZE);
    }

    let z = -rng.gen_range(NEAR_PLANE..FAR_PLANE);
    Vec3::new(x, y, z)
}

fn random_rotation(rng: &mut SmallRng) -> Quat {
    let axis = random_unit_vector(rng);
    let angle = rng.gen_range(0.0..std::f32::consts::TAU);
    Quat::from_axis_angle(axis, angle)
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
    rng.gen_range(STAR_SCALE_RANGE.clone())
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(StarfieldGpuApp::default()).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn start_app() {
    run_application(StarfieldGpuApp::default()).unwrap();
}
