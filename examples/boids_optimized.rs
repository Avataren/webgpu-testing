// examples/boids_optimized.rs
//
// Optimized 3D Boids Simulation using Spatial Hash Grid

use glam::{Quat, Vec3};
use rand::{rngs::SmallRng, Rng, SeedableRng};

use wgpu_cube::gpu_particles::behaviors::OptimizedBoidsBehavior;
use wgpu_cube::gpu_particles::{GpuParticleSystem, Particle};
use wgpu_cube::renderer::{CustomRenderContext, Material};
use wgpu_cube::scene::components::{CanCastShadow, DirectionalLight};
use wgpu_cube::scene::{Name, Transform, TransformComponent};
use wgpu_cube::{
    render_application::RenderApplication, run_application, AppBuilder, GpuUpdateContext,
    StartupContext, UpdateContext,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

// Tuned parameters chosen to keep the spatial grid within a single prefix-sum workgroup
const BOID_COUNT: usize = 4_096;
const BOUNDS: f32 = 60.0;
const MAX_SPEED: f32 = 36.0;
const MAX_FORCE: f32 = 4.5;

const SEPARATION_RADIUS: f32 = 4.0;
const ALIGNMENT_RADIUS: f32 = 18.0;
const COHESION_RADIUS: f32 = 22.0;
const SEPARATION_WEIGHT: f32 = 1.2;
const ALIGNMENT_WEIGHT: f32 = 1.4;
const COHESION_WEIGHT: f32 = 2.4;

const INITIAL_SPAWN_RADIUS: f32 = 20.0;
const MIN_SPEED: f32 = 6.0;
const SCALE_RANGE: std::ops::Range<f32> = 0.9..1.4;

struct BoidsOptimizedApp {
    particle_system: Option<GpuParticleSystem>,
    mesh_handle: Option<wgpu_cube::asset::Handle<wgpu_cube::asset::Mesh>>,
    behavior: Option<OptimizedBoidsBehavior>,
    frame_count: u32,
}

impl BoidsOptimizedApp {
    fn new() -> Self {
        Self {
            particle_system: None,
            mesh_handle: None,
            behavior: None,
            frame_count: 0,
        }
    }
}

impl RenderApplication for BoidsOptimizedApp {
    fn name(&self) -> &str {
        "3D Boids Simulation (Optimized with Spatial Grid)"
    }

    fn configure(&self, _builder: &mut AppBuilder) {}

    fn setup(&mut self, ctx: &mut StartupContext) {
        // Environment setup
        ctx.scene.environment_mut().set_clear_color(wgpu::Color {
            r: 0.02,
            g: 0.025,
            b: 0.04,
            a: 1.0,
        });
        ctx.scene.environment_mut().disable_hdr_background();

        // Camera setup
        ctx.scene.camera_mut().eye = Vec3::new(0.0, 45.0, 130.0);
        ctx.scene.camera_mut().target = Vec3::ZERO;
        ctx.scene.camera_mut().far = 500.0;

        // Lighting setup
        let sun_direction = Vec3::new(0.4, -1.2, -0.5).normalize();
        let sun_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, sun_direction);
        ctx.scene.world_mut().spawn((
            Name::new("Main Light"),
            TransformComponent(Transform::from_trs(
                Vec3::ZERO,
                sun_rotation,
                Vec3::splat(1.0),
            )),
            DirectionalLight::new(Vec3::new(1.0, 0.95, 0.9), 6.0),
            CanCastShadow(true),
        ));

        // Create mesh for boid rendering
        let (vertices, indices) = wgpu_cube::renderer::cube_mesh();
        let mesh = ctx.renderer.create_mesh(&vertices, &indices);
        let mesh_handle = ctx.scene.assets.meshes.insert(mesh);
        self.mesh_handle = Some(mesh_handle);

        // Create material
        let material = Material::new([180, 210, 255, 255])
            .with_metallic(0.2)
            .with_roughness(0.45);

        // Create optimized boids behavior with spatial grid
        let max_interaction_radius = COHESION_RADIUS.max(ALIGNMENT_RADIUS);
        let mut behavior = OptimizedBoidsBehavior::new(
            ctx.renderer.get_device(),
            BOID_COUNT as u32,
            BOUNDS,
            max_interaction_radius,
        );

        // Set behavior parameters
        behavior.separation_radius = SEPARATION_RADIUS;
        behavior.alignment_radius = ALIGNMENT_RADIUS;
        behavior.cohesion_radius = COHESION_RADIUS;
        behavior.separation_weight = SEPARATION_WEIGHT;
        behavior.alignment_weight = ALIGNMENT_WEIGHT;
        behavior.cohesion_weight = COHESION_WEIGHT;
        behavior.max_speed = MAX_SPEED;
        behavior.max_force = MAX_FORCE;
        behavior.bounds = BOUNDS;

        // Initialize particles
        let initial_particles = initial_boid_particles();
        let particle_count = initial_particles.len() as u32;

        // Create GPU particle system
        let mut particle_system = GpuParticleSystem::new(
            ctx.renderer.get_device(),
            ctx.renderer.get_queue(),
            ctx.renderer,
            particle_count,
            material,
            &behavior,
        );

        particle_system.initialize_particles(ctx.renderer.get_queue(), &initial_particles);

        log::info!(
            "Optimized Boids GPU simulation initialized with {} boids",
            particle_count
        );
        println!(
            "Initial particle 0 position: {:?}",
            initial_particles[0].position
        );
        println!(
            "Initial particle 0 velocity: {:?}",
            initial_particles[0].velocity
        );
        self.behavior = Some(behavior);
        self.particle_system = Some(particle_system);
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        ctx.scene.camera_mut().eye = Vec3::new(0.0, 0.0, 150.0);
        ctx.scene.camera_mut().target = Vec3::ZERO;
    }

    fn gpu_update(&mut self, ctx: &mut GpuUpdateContext) {
        if let (Some(particle_system), Some(behavior)) =
            (&mut self.particle_system, &mut self.behavior)
        {
            if self.frame_count % 60 == 0 {
                println!("dt = {} seconds", ctx.dt);
            }
            let frame_start = std::time::Instant::now();
            let mut encoder =
                ctx.renderer
                    .get_device()
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("BoidsOptimizedGpuUpdate"),
                    });

            // Rebuild every 2 frames
            behavior.build_spatial_grid(
                ctx.renderer.get_device(),
                ctx.renderer.get_queue(),
                &mut encoder,
                particle_system.particles_buffer(),
                particle_system.active_particle_count(),
            );

            // Update particles (using possibly slightly stale grid)
            particle_system.update(
                ctx.renderer.get_device(),
                ctx.renderer.get_queue(),
                &mut encoder,
                behavior,
                ctx.dt as f32,
            );
            let before_submit = std::time::Instant::now();
            ctx.renderer.get_queue().submit(Some(encoder.finish()));
            let submit_time = before_submit.elapsed();

            let total_time = frame_start.elapsed();
            if self.frame_count % 60 == 0 {
                log::info!(
                    "Frame {}: Submit took {:?}, Total {:?}",
                    self.frame_count,
                    submit_time,
                    total_time
                );
            }
            self.frame_count += 1;
        }
    }

    fn custom_render(&mut self, ctx: &mut CustomRenderContext) {
        if let (Some(particle_system), Some(mesh_handle)) =
            (self.particle_system.as_mut(), self.mesh_handle)
        {
            if let Some(mesh) = ctx.scene.assets.meshes.get(mesh_handle) {
                particle_system.render(ctx, mesh);
            }
        }
    }
}

fn initial_boid_particles() -> Vec<Particle> {
    let mut rng = SmallRng::seed_from_u64(42);
    (0..BOID_COUNT)
        .map(|i| {
            let position = Vec3::new(
                rng.gen_range(-INITIAL_SPAWN_RADIUS..INITIAL_SPAWN_RADIUS),
                rng.gen_range(-INITIAL_SPAWN_RADIUS..INITIAL_SPAWN_RADIUS),
                rng.gen_range(-INITIAL_SPAWN_RADIUS..INITIAL_SPAWN_RADIUS),
            );

            let mut velocity = Vec3::new(
                rng.gen_range(-MAX_SPEED..MAX_SPEED) * 0.4,
                rng.gen_range(-MAX_SPEED..MAX_SPEED) * 0.4,
                rng.gen_range(-MAX_SPEED..MAX_SPEED) * 0.4,
            );
            if velocity.length() < MIN_SPEED {
                let direction = Vec3::new(
                    rng.gen_range(-1.0..1.0),
                    rng.gen_range(-1.0..1.0),
                    rng.gen_range(-1.0..1.0),
                )
                .normalize_or_zero();
                velocity = direction * MIN_SPEED;
            }

            let rotation = velocity_to_rotation(velocity);
            let scale = rng.gen_range(SCALE_RANGE.clone());
            let hue = (i as f32 / BOID_COUNT as f32) * 360.0;
            let rgb = hue_to_rgb(hue);

            let color_rgba = [rgb[0], rgb[1], rgb[2], 1.0];
            let mut color_key_times = [1.0; Particle::MAX_COLOR_KEYS];
            color_key_times[0] = 0.0;

            Particle {
                position: position.into(),
                lifetime: 0.0,
                velocity: velocity.into(),
                max_lifetime: f32::INFINITY,
                rotation,
                scale: [scale * 0.3, scale * 0.6, scale],
                angular_velocity: 0.0,
                color: color_rgba,
                color_keys: [color_rgba; Particle::MAX_COLOR_KEYS],
                color_key_times,
                user_data: [1.0, 1.0, 1.0, 0.0],
            }
        })
        .collect()
}

fn velocity_to_rotation(velocity: Vec3) -> [f32; 4] {
    let base_forward = Vec3::Z;
    let speed_sq = velocity.length_squared();
    if speed_sq < 1e-6 {
        return [0.0, 1.0, 0.0, 0.0];
    }

    let forward = velocity.normalize();
    let dot = base_forward.dot(forward).clamp(-1.0, 1.0);
    let axis = base_forward.cross(forward);
    let axis_length = axis.length();

    if axis_length < 1e-5 {
        if dot > 0.0 {
            [0.0, 1.0, 0.0, 0.0]
        } else {
            [0.0, 1.0, 0.0, std::f32::consts::PI]
        }
    } else {
        let normalized_axis = axis / axis_length;
        let angle = dot.acos();
        [
            normalized_axis.x,
            normalized_axis.y,
            normalized_axis.z,
            angle,
        ]
    }
}

fn hue_to_rgb(hue: f32) -> [f32; 3] {
    let h = (hue / 60.0) % 6.0;
    let c = 1.0;
    let x = c * (1.0 - ((h % 2.0) - 1.0).abs());

    let (r, g, b) = if h < 1.0 {
        (c, x, 0.0)
    } else if h < 2.0 {
        (x, c, 0.0)
    } else if h < 3.0 {
        (0.0, c, x)
    } else if h < 4.0 {
        (0.0, x, c)
    } else if h < 5.0 {
        (x, 0.0, c)
    } else {
        (c, 0.0, x)
    };

    [r, g, b]
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(BoidsOptimizedApp::new()).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    run_application(BoidsOptimizedApp::new()).unwrap();
}
