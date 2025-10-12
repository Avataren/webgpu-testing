// examples/boids_3d.rs - 3D Boids Simulation using new compute abstractions

use bytemuck::{Pod, Zeroable};
use glam::{Quat, Vec3};
use rand::{rngs::SmallRng, Rng, SeedableRng};
use wgpu_cube::app::{GpuUpdateContext, StartupContext, UpdateContext};
use wgpu_cube::render_application::{run_application, RenderApplication};
use wgpu_cube::renderer::{
    BindGroupBuilder, BindGroupLayoutBuilder, ComputePass, ComputePipelineBuilder, Material,
    StorageBuffer, UniformBuffer,
};
use wgpu_cube::scene::{EntityBuilder, Transform};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

// Simulation parameters
const BOID_COUNT: u32 = 2000;
const WORKGROUP_SIZE: u32 = 64;
const BOUNDS: f32 = 50.0;
const MAX_SPEED: f32 = 15.0;
const MAX_FORCE: f32 = 0.3;

// Boid behavior parameters
const SEPARATION_RADIUS: f32 = 3.0;
const ALIGNMENT_RADIUS: f32 = 8.0;
const COHESION_RADIUS: f32 = 8.0;
const SEPARATION_WEIGHT: f32 = 1.5;
const ALIGNMENT_WEIGHT: f32 = 1.0;
const COHESION_WEIGHT: f32 = 1.0;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Boid {
    position: [f32; 3],
    _padding1: f32,
    velocity: [f32; 3],
    _padding2: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BoidParams {
    delta_time: f32,
    separation_radius: f32,
    alignment_radius: f32,
    cohesion_radius: f32,
    separation_weight: f32,
    alignment_weight: f32,
    cohesion_weight: f32,
    max_speed: f32,
    max_force: f32,
    bounds: f32,
    boid_count: u32,
    _padding: f32,
}

struct BoidsApp {
    boid_state: Option<BoidSimulation>,
}

impl BoidsApp {
    fn new() -> Self {
        Self { boid_state: None }
    }
}

impl RenderApplication for BoidsApp {
    fn name(&self) -> &str {
        "3D Boids Simulation (Compute)"
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        // Setup camera
        ctx.scene.camera_mut().eye = Vec3::new(0.0, 40.0, 80.0);
        ctx.scene.camera_mut().target = Vec3::ZERO;
        ctx.scene.camera_mut().far = 500.0;

        // Create boid simulation
        self.boid_state = Some(BoidSimulation::new(ctx));

        log::info!("3D Boids simulation initialized with {} boids", BOID_COUNT);
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        // Orbit camera
        let t = ctx.scene.time() as f32 * 0.1;
        let radius = 100.0;
        let height = 60.0;
        ctx.scene.camera_mut().eye = Vec3::new(t.cos() * radius, height, t.sin() * radius);
        ctx.scene.camera_mut().target = Vec3::ZERO;
    }

    fn gpu_update(&mut self, ctx: &mut GpuUpdateContext) {
        if let Some(state) = &mut self.boid_state {
            state.update(ctx);
        }
    }
}

struct BoidSimulation {
    boid_buffer: StorageBuffer,
    params_buffer: UniformBuffer,
    compute_pipeline: wgpu::ComputePipeline,
    compute_bind_group: wgpu::BindGroup,
    mesh_handle: wgpu_cube::asset::Handle<wgpu_cube::asset::Mesh>,
    material: Material,
    workgroup_count: u32,
}

impl BoidSimulation {
    fn new(ctx: &mut StartupContext) -> Self {
        let device = ctx.renderer.get_device();
        let queue = ctx.renderer.get_queue();

        // Initialize boids with random positions and velocities
        let boids = Self::initialize_boids();
        let boid_buffer = StorageBuffer::new(device, "Boids", &boids);

        // Create params buffer
        let params = BoidParams {
            delta_time: 0.0,
            separation_radius: SEPARATION_RADIUS,
            alignment_radius: ALIGNMENT_RADIUS,
            cohesion_radius: COHESION_RADIUS,
            separation_weight: SEPARATION_WEIGHT,
            alignment_weight: ALIGNMENT_WEIGHT,
            cohesion_weight: COHESION_WEIGHT,
            max_speed: MAX_SPEED,
            max_force: MAX_FORCE,
            bounds: BOUNDS,
            boid_count: BOID_COUNT,
            _padding: 0.0,
        };
        let params_buffer = UniformBuffer::new(device, "BoidParams", &params);

        // Create compute pipeline using the new builder
        let shader_source = include_str!("shaders/boids.wgsl");
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("BoidsComputeShader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let bind_group_layout = BindGroupLayoutBuilder::new(device)
            .with_label("BoidsBindGroupLayout")
            .add_storage_buffer(0, false) // Boids buffer (read/write)
            .add_uniform_buffer(1) // Params buffer
            .build();

        let compute_pipeline = ComputePipelineBuilder::new(device)
            .with_label("BoidsComputePipeline")
            .with_shader(&shader)
            .with_entry_point("main")
            .with_bind_group_layout(&bind_group_layout)
            .build();

        let compute_bind_group = BindGroupBuilder::new(device, &bind_group_layout)
            .with_label("BoidsBindGroup")
            .add_buffer(0, boid_buffer.buffer())
            .add_buffer(1, params_buffer.buffer())
            .build();

        // Create visual representation (small cubes for boids)
        let (vertices, indices) = wgpu_cube::renderer::cube_mesh();
        let mesh = ctx.renderer.create_mesh(&vertices, &indices);
        let mesh_handle = ctx.scene.assets.meshes.insert(mesh);

        let material = Material::pbr()
            .with_metallic(0.2)
            .with_roughness(0.6);

        // Spawn visual entities for each boid
        for i in 0..BOID_COUNT {
            let color_hue = (i as f32 / BOID_COUNT as f32) * 360.0;
            let color = Self::hue_to_rgb(color_hue);
            
            let boid_material = Material::new([
                (color.0 * 255.0) as u8,
                (color.1 * 255.0) as u8,
                (color.2 * 255.0) as u8,
                255,
            ])
            .with_metallic(0.3)
            .with_roughness(0.5);

            EntityBuilder::new(&mut ctx.scene.world)
                .with_name(format!("Boid{}", i))
                .with_transform(Transform::from_translation(Vec3::ZERO))
                .with_mesh(mesh_handle)
                .with_material(boid_material)
                .visible(true)
                .spawn();
        }

        let workgroup_count = BOID_COUNT.div_ceil(WORKGROUP_SIZE);

        Self {
            boid_buffer,
            params_buffer,
            compute_pipeline,
            compute_bind_group,
            mesh_handle,
            material,
            workgroup_count,
        }
    }

    fn initialize_boids() -> Vec<Boid> {
        let mut rng = SmallRng::from_entropy();
        (0..BOID_COUNT)
            .map(|_| {
                let position = [
                    rng.gen_range(-BOUNDS..BOUNDS),
                    rng.gen_range(-BOUNDS..BOUNDS),
                    rng.gen_range(-BOUNDS..BOUNDS),
                ];
                let velocity = [
                    rng.gen_range(-MAX_SPEED..MAX_SPEED) * 0.1,
                    rng.gen_range(-MAX_SPEED..MAX_SPEED) * 0.1,
                    rng.gen_range(-MAX_SPEED..MAX_SPEED) * 0.1,
                ];
                Boid {
                    position,
                    _padding1: 0.0,
                    velocity,
                    _padding2: 0.0,
                }
            })
            .collect()
    }

    fn update(&mut self, ctx: &mut GpuUpdateContext) {
        let device = ctx.renderer.get_device();
        let queue = ctx.renderer.get_queue();

        // Update params with delta time
        let params = BoidParams {
            delta_time: ctx.dt as f32,
            separation_radius: SEPARATION_RADIUS,
            alignment_radius: ALIGNMENT_RADIUS,
            cohesion_radius: COHESION_RADIUS,
            separation_weight: SEPARATION_WEIGHT,
            alignment_weight: ALIGNMENT_WEIGHT,
            cohesion_weight: COHESION_WEIGHT,
            max_speed: MAX_SPEED,
            max_force: MAX_FORCE,
            bounds: BOUNDS,
            boid_count: BOID_COUNT,
            _padding: 0.0,
        };
        self.params_buffer.write(queue, &params);

        // Run compute shader
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("BoidsComputeEncoder"),
        });

        {
            let mut pass = ComputePass::begin(&mut encoder, "BoidsComputePass");
            pass.set_pipeline(&self.compute_pipeline)
                .set_bind_group(0, &self.compute_bind_group, &[])
                .dispatch_workgroups(self.workgroup_count, 1, 1);
        }

        queue.submit(Some(encoder.finish()));

        // Update visual positions (simplified - in production you'd read back from GPU)
        // For now we'll just let the compute shader update and trust it's working
    }

    fn hue_to_rgb(hue: f32) -> (f32, f32, f32) {
        let h = hue / 60.0;
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
        
        (r, g, b)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(BoidsApp::new()).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    run_application(BoidsApp::new()).unwrap();
}