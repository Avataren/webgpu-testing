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
use wgpu_cube::scene::{EntityBuilder, Transform, TransformComponent};

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

// GPU representation of a boid
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BoidGpuData {
    position: [f32; 3],
    _padding1: f32,
    velocity: [f32; 3],
    _padding2: f32,
}

// Component marker for boid entities with their index
#[derive(Clone, Copy, Debug)]
struct BoidComponent {
    index: u32,
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
    workgroup_count: u32,
}

impl BoidSimulation {
    fn new(ctx: &mut StartupContext) -> Self {
        let device = ctx.renderer.get_device();

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

            // Spawn entity with BoidComponent marker
            EntityBuilder::new(&mut ctx.scene.world)
                .with_name(format!("Boid{}", i))
                .with_transform(Transform::from_translation(Vec3::ZERO))
                .with_mesh(mesh_handle)
                .with_material(boid_material)
                .with_component(BoidComponent { index: i })
                .visible(true)
                .spawn();
        }

        log::info!("Spawned {} boid entities", BOID_COUNT);

        let workgroup_count = BOID_COUNT.div_ceil(WORKGROUP_SIZE);

        Self {
            boid_buffer,
            params_buffer,
            compute_pipeline,
            compute_bind_group,
            workgroup_count,
        }
    }

    fn update_visual_entities(&self, ctx: &mut GpuUpdateContext) {
        let device = ctx.renderer.get_device();
        let queue = ctx.renderer.get_queue();

        // Create staging buffer for readback
        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Boid Staging Buffer"),
            size: self.boid_buffer.buffer().size(),
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Copy GPU buffer to staging buffer
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Boid Readback Encoder"),
        });
        encoder.copy_buffer_to_buffer(
            self.boid_buffer.buffer(),
            0,
            &staging_buffer,
            0,
            self.boid_buffer.buffer().size(),
        );
        queue.submit(Some(encoder.finish()));

        // Map and read the staging buffer
        let buffer_slice = staging_buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |result| {
            tx.send(result).unwrap();
        });

        device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
        rx.recv().unwrap().unwrap();

        let data = buffer_slice.get_mapped_range();
        let boids: &[BoidGpuData] = bytemuck::cast_slice(&data);

        // Debug: print first few boid positions (only occasionally to reduce spam)
        static mut FRAME_COUNT: u32 = 0;
        unsafe {
            FRAME_COUNT += 1;
            if FRAME_COUNT % 120 == 1 {
                // Log every 120 frames
                if !boids.is_empty() {
                    log::info!(
                        "Boid 0 pos: {:?}, vel: {:?}",
                        boids[0].position,
                        boids[0].velocity
                    );
                    if boids.len() > 1 {
                        log::info!(
                            "Boid 1 pos: {:?}, vel: {:?}",
                            boids[1].position,
                            boids[1].velocity
                        );
                    }
                }
            }
        }

        // Query all entities with both BoidComponent and TransformComponent (not Transform!)
        let mut updated_count = 0;
        for (_entity, (boid_comp, transform_comp)) in ctx
            .scene
            .world
            .query_mut::<(&BoidComponent, &mut TransformComponent)>()
        {
            let index = boid_comp.index as usize;
            if index < boids.len() {
                let boid = &boids[index];
                // Access the inner Transform from TransformComponent
                transform_comp.0.translation = Vec3::from(boid.position);
                transform_comp.0.scale = Vec3::splat(0.5); // Make cubes smaller

                // Orient boid in direction of velocity
                let vel = Vec3::from(boid.velocity);
                if vel.length_squared() > 0.001 {
                    let forward = vel.normalize();
                    let up = Vec3::Y;
                    let right = up.cross(forward).normalize_or_zero();
                    if right.length_squared() > 0.001 {
                        let new_up = forward.cross(right).normalize();
                        transform_comp.0.rotation =
                            Quat::from_mat3(&glam::Mat3::from_cols(right, new_up, forward));
                    }
                }
                updated_count += 1;
            }
        }

        unsafe {
            if FRAME_COUNT % 120 == 1 {
                log::info!("Updated {} boid transforms", updated_count);
            }
        }

        drop(data);
        staging_buffer.unmap();
    }

    fn initialize_boids() -> Vec<BoidGpuData> {
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
                BoidGpuData {
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
        self.update_visual_entities(ctx);
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
