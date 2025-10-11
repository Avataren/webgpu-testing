use glam::{Quat, Vec3};
use rand::{rngs::SmallRng, Rng, SeedableRng};

use wgpu_cube::gpu_particles::{GpuParticleSystem, ParticleFieldSettings, ParticleInit};
use wgpu_cube::renderer::Material;
use wgpu_cube::scene::components::{CanCastShadow, DirectionalLight};
use wgpu_cube::{
    render_application::RenderApplication, run_application, AppBuilder, GpuUpdateContext,
    StartupContext, UpdateContext,
};

// Match the CPU starfield parameters exactly
const STAR_COUNT: usize = 1000_000;
const FIELD_HALF_SIZE: f32 = 60.0;
const NEAR_PLANE: f32 = 0.01;
const FAR_PLANE: f32 = 150.0;
const FAR_RESET_BAND: f32 = 25.0;
const STAR_SPEED_RANGE: std::ops::Range<f32> = 5.0..15.0;
const SPIN_SPEED_RANGE: std::ops::Range<f32> = 0.1..3.5;
const STAR_SCALE_RANGE: std::ops::Range<f32> = 0.1..0.25;
const MIN_SIZE_FROM_CENTER: f32 = 0.15;

struct StarfieldGpuApp {
    particle_system: Option<GpuParticleSystem>,
    mesh_handle: Option<wgpu_cube::asset::Handle<wgpu_cube::asset::Mesh>>,
}

impl StarfieldGpuApp {
    fn new() -> Self {
        Self {
            particle_system: None,
            mesh_handle: None,
        }
    }
}

impl RenderApplication for StarfieldGpuApp {
    fn name(&self) -> &str {
        "GPU Starfield (GPU-Driven)"
    }

    fn configure(&self, builder: &mut AppBuilder) {
        builder.disable_default_lighting();
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        // Use cube mesh to match CPU starfield
        let (vertices, indices) = wgpu_cube::renderer::cube_mesh();
        let mesh = ctx.renderer.create_mesh(&vertices, &indices);
        let mesh_handle = ctx.scene.assets.meshes.insert(mesh);
        self.mesh_handle = Some(mesh_handle);

        // Match the CPU starfield material (checker with roughness 64)
        let mut material = Material::checker();
        material.roughness_factor = 64;

        // Match the CPU starfield environment
        ctx.scene.environment_mut().set_clear_color(wgpu::Color {
            r: 0.001,
            g: 0.005,
            b: 0.01,
            a: 1.0,
        });
        ctx.scene.environment_mut().disable_hdr_background();

        // Match the CPU starfield camera
        ctx.scene.set_camera(wgpu_cube::scene::Camera {
            eye: Vec3::ZERO,
            target: Vec3::new(0.0, 0.0, -1.0),
            up: Vec3::Y,
            near: NEAR_PLANE,
            far: FAR_PLANE,
            ..Default::default()
        });

        // Match the CPU starfield lighting
        let sun1_direction = Vec3::new(0.3, -1.0, -1.1).normalize();
        let sun1_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, sun1_direction);

        ctx.scene.world.spawn((
            wgpu_cube::scene::Name::new("Default Sky Light"),
            wgpu_cube::scene::TransformComponent(wgpu_cube::scene::Transform::from_trs(
                Vec3::ZERO,
                sun1_rotation,
                Vec3::ONE,
            )),
            DirectionalLight::new(Vec3::new(0.49, 0.95, 0.85), 2.5),
            CanCastShadow(false),
        ));

        // Match the CPU starfield particle settings
        let settings = ParticleFieldSettings {
            near_plane: NEAR_PLANE,
            far_plane: FAR_PLANE,
            far_reset_band: FAR_RESET_BAND,
            field_half_size: FIELD_HALF_SIZE,
            min_radius: MIN_SIZE_FROM_CENTER,
            speed_range: STAR_SPEED_RANGE,
            spin_range: SPIN_SPEED_RANGE,
            scale_range: STAR_SCALE_RANGE,
        };

        // Generate initial particles with same distribution as CPU version
        let particle_count = STAR_COUNT as u32;
        let particles: Vec<ParticleInit> = (0..particle_count)
            .map(|i| {
                let mut rng = SmallRng::seed_from_u64(i as u64);

                let position = random_initial_position(&mut rng);
                let rotation = random_rotation(&mut rng);
                let scale = random_scale(&mut rng);
                let speed = rng.gen_range(STAR_SPEED_RANGE);
                let angular_speed = rng.gen_range(SPIN_SPEED_RANGE);
                let angular_axis = random_unit_vector(&mut rng);

                ParticleInit {
                    position,
                    speed,
                    rotation,
                    angular_axis,
                    angular_speed,
                    scale,
                    seed: i,
                }
            })
            .collect();

        let particle_system = GpuParticleSystem::new(
            ctx.renderer,
            particle_count,
            material,
            settings,
            &particles,
        );

        self.particle_system = Some(particle_system);

        log::info!(
            "GPU Starfield setup complete with {} particles",
            particle_count
        );
    }

    fn gpu_update(&mut self, ctx: &mut GpuUpdateContext) {
        if let Some(particle_system) = &mut self.particle_system {
            particle_system.update(ctx.renderer, ctx.dt as f32);
        }
    }

    fn custom_render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        renderer: &wgpu_cube::renderer::Renderer,
        scene: &wgpu_cube::scene::Scene,
        view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
    ) {
        // Render the GPU particles
        if let (Some(particle_system), Some(mesh_handle)) =
            (&self.particle_system, self.mesh_handle)
        {
            if let Some(mesh) = scene.assets.meshes.get(mesh_handle) {
                particle_system.render(encoder, renderer, mesh, view, depth_view);
            }
        }
    }
}

// Helper functions matching the CPU version
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
    rng.gen_range(STAR_SCALE_RANGE)
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(StarfieldGpuApp::new()).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn start_app() {
    run_application(StarfieldGpuApp::new()).unwrap();
}