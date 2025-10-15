use glam::{Quat, Vec3};
use rand::{rngs::SmallRng, Rng, SeedableRng};

use wgpu_cube::gpu_particles::behaviors::StarfieldBehavior;
use wgpu_cube::gpu_particles::{GpuParticleSystem, Particle};
use wgpu_cube::renderer::{CustomRenderContext, Material};
use wgpu_cube::scene::components::{CanCastShadow, DirectionalLight};
use wgpu_cube::{
    render_application::RenderApplication, run_application, AppBuilder, GpuUpdateContext,
    StartupContext,
};

const STAR_COUNT: usize = 1_000;
const FIELD_HALF_SIZE: f32 = 60.0;
const NEAR_PLANE: f32 = 0.01;
const FAR_PLANE: f32 = 200.0;
const FAR_RESET_BAND: f32 = 5.0; // Smaller band keeps particles more spread out
const STAR_SPEED_RANGE: std::ops::Range<f32> = 5.0..15.0;
const SPIN_SPEED_RANGE: std::ops::Range<f32> = 1.0..5.0;
const STAR_SCALE_RANGE: std::ops::Range<f32> = 4.15..5.25;
const MIN_SIZE_FROM_CENTER: f32 = 0.25;

struct StarfieldGpuApp {
    particle_system: Option<GpuParticleSystem>,
    mesh_handle: Option<wgpu_cube::asset::Handle<wgpu_cube::asset::Mesh>>,
    behavior: StarfieldBehavior,
}

impl StarfieldGpuApp {
    fn new() -> Self {
        Self {
            particle_system: None,
            mesh_handle: None,
            behavior: StarfieldBehavior {
                near_plane: NEAR_PLANE,
                far_plane: FAR_PLANE,
                far_reset_band: FAR_RESET_BAND,
                field_half_size: FIELD_HALF_SIZE,
                min_radius: MIN_SIZE_FROM_CENTER,
            },
        }
    }
}

impl RenderApplication for StarfieldGpuApp {
    fn name(&self) -> &str {
        "GPU Starfield (GPU-Driven)"
    }

    fn configure(&self, _builder: &mut AppBuilder) {
        // Keep default lighting enabled for PBR
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        let (vertices, indices) = wgpu_cube::renderer::cube_mesh();
        let mesh = ctx.renderer.create_mesh(&vertices, &indices);
        let mesh_handle = ctx.scene.assets.meshes.insert(mesh);
        self.mesh_handle = Some(mesh_handle);

        // Create material with checker texture and PBR properties
        let material = Material::checker().with_metallic(0.1).with_roughness(0.7);

        ctx.scene.environment_mut().set_clear_color(wgpu::Color {
            r: 0.001,
            g: 0.005,
            b: 0.01,
            a: 1.0,
        });
        ctx.scene.environment_mut().disable_hdr_background();

        ctx.scene.set_camera(wgpu_cube::scene::Camera {
            eye: Vec3::ZERO,
            target: Vec3::new(0.0, 0.0, -1.0),
            up: Vec3::Y,
            near: NEAR_PLANE,
            far: FAR_PLANE,
            ..Default::default()
        });

        // Add directional light for better visibility
        let sun_direction = Vec3::new(0.3, -1.0, -0.5).normalize();
        let sun_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, sun_direction);

        ctx.scene.world_mut().spawn((
            wgpu_cube::scene::Name::new("Main Light"),
            wgpu_cube::scene::TransformComponent(wgpu_cube::scene::Transform::from_trs(
                Vec3::ZERO,
                sun_rotation,
                Vec3::ONE,
            )),
            DirectionalLight::new(Vec3::new(1.0, 0.95, 0.9), 3.0),
            CanCastShadow(false),
        ));

        // Generate initial particles
        let particle_count = STAR_COUNT as u32;
        let initial_particles: Vec<Particle> = (0..particle_count)
            .map(|i| {
                let mut rng = SmallRng::seed_from_u64(i as u64);

                let position = random_initial_position(&mut rng);
                let rotation_axis = random_unit_vector(&mut rng);
                let rotation_angle = rng.gen_range(0.0..std::f32::consts::TAU);
                let scale = random_scale(&mut rng);
                let speed = rng.gen_range(STAR_SPEED_RANGE);
                let angular_speed = rng.gen_range(SPIN_SPEED_RANGE);

                let color_rgba = [1.0, 1.0, 1.0, 1.0];
                let mut color_key_times = [1.0; Particle::MAX_COLOR_KEYS];
                color_key_times[0] = 0.0;

                Particle {
                    position: position.into(),
                    lifetime: rng.gen_range(0.0..1000.0), // Random start time for variety
                    velocity: [0.0, 0.0, speed],          // Moving in +Z toward camera at origin
                    max_lifetime: f32::INFINITY,
                    rotation: [
                        rotation_axis.x,
                        rotation_axis.y,
                        rotation_axis.z,
                        rotation_angle,
                    ], // axis-angle format
                    scale: [scale, scale, scale],
                    angular_velocity: angular_speed,
                    color: color_rgba,
                    color_keys: [color_rgba; Particle::MAX_COLOR_KEYS],
                    color_key_times,
                    user_data: [0.0, 0.0, 1.0, 0.0],
                }
            })
            .collect();

        // Create particle system
        let mut particle_system = GpuParticleSystem::new(
            ctx.renderer.get_device(),
            ctx.renderer.get_queue(),
            ctx.renderer,
            particle_count,
            material,
            &self.behavior,
        );
        particle_system.set_depth_write_enabled(true);
        particle_system.set_casts_shadows(false);

        // Initialize particles
        particle_system.initialize_particles(ctx.renderer.get_queue(), &initial_particles);

        log::info!(
            "GPU Starfield setup complete with {} particles",
            particle_count
        );
        log::info!(
            "First particle: pos=({:.2}, {:.2}, {:.2}), vel=({:.2}, {:.2}, {:.2})",
            initial_particles[0].position[0],
            initial_particles[0].position[1],
            initial_particles[0].position[2],
            initial_particles[0].velocity[0],
            initial_particles[0].velocity[1],
            initial_particles[0].velocity[2]
        );

        self.particle_system = Some(particle_system);
    }

    fn gpu_update(&mut self, ctx: &mut GpuUpdateContext) {
        if let Some(particle_system) = &mut self.particle_system {
            let mut encoder =
                ctx.renderer
                    .get_device()
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("ParticleUpdateEncoder"),
                    });

            particle_system.update(
                ctx.renderer.get_device(),
                ctx.renderer.get_queue(),
                &mut encoder,
                &self.behavior,
                ctx.dt as f32,
            );

            ctx.renderer.get_queue().submit(Some(encoder.finish()));
        }
    }

    fn custom_render(&mut self, ctx: &mut CustomRenderContext) {
        // Render the GPU particles
        if let (Some(particle_system), Some(mesh_handle)) =
            (self.particle_system.as_mut(), self.mesh_handle)
        {
            if let Some(mesh) = ctx.scene.assets.meshes.get(mesh_handle) {
                particle_system.render(ctx, mesh);
            }
        }
    }

    fn custom_render_includes_shadows(&self) -> bool {
        self.particle_system
            .as_ref()
            .is_some_and(|system| system.casts_shadows())
    }
}

// Helper functions
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
