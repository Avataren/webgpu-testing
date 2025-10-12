// src/gpu_particles/mod.rs - Complete implementation

use bytemuck::{Pod, Zeroable};
use glam::{Quat, Vec3};
use wgpu::util::DeviceExt;

use crate::renderer::ShaderBuilder;

pub mod behaviors;

const WORKGROUP_SIZE: u32 = 256;

// ============================================================================
// Core Particle Data Structure
// ============================================================================

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct Particle {
    pub position: [f32; 3],
    pub lifetime: f32,
    pub velocity: [f32; 3],
    pub max_lifetime: f32,
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub angular_velocity: f32,
    pub color: [f32; 4],
    pub user_data: [f32; 4],
}

impl Default for Particle {
    fn default() -> Self {
        Self {
            position: [0.0; 3],
            lifetime: 0.0,
            velocity: [0.0; 3],
            max_lifetime: 5.0,
            rotation: [0.0, 0.0, 0.0, 1.0],
            scale: [1.0; 3],
            angular_velocity: 0.0,
            color: [1.0; 4],
            user_data: [0.0; 4],
        }
    }
}

// ============================================================================
// Particle Behavior Trait
// ============================================================================

pub trait ParticleBehavior: Send + Sync {
    fn shader_source(&self) -> &str;
    fn entry_point(&self) -> &str {
        "main"
    }
    fn create_params_buffer(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Buffer;
    fn update_params(&self, queue: &wgpu::Queue, buffer: &wgpu::Buffer, dt: f32);
    fn additional_bindings(&self, _device: &wgpu::Device) -> Vec<wgpu::BindGroupEntry<'_>> {
        Vec::new()
    }
    fn additional_layout_entries(&self) -> Vec<wgpu::BindGroupLayoutEntry> {
        Vec::new()
    }
}

// ============================================================================
// Emission Shapes
// ============================================================================

#[derive(Clone, Copy, Debug)]
pub enum EmissionShape {
    Point,
    Sphere { radius: f32 },
    Box { half_extents: Vec3 },
    Cone { angle: f32, radius: f32 },
}

// ============================================================================
// Particle Emitter
// ============================================================================

pub struct ParticleEmitter {
    pub spawn_rate: f32,
    pub burst_count: Option<u32>,
    pub position: Vec3,
    pub emission_shape: EmissionShape,
    pub initial_velocity_range: (Vec3, Vec3),
    pub initial_scale_range: (Vec3, Vec3),
    pub lifetime_range: (f32, f32),
    pub initial_color: [f32; 4],
    spawn_accumulator: f32,
    total_spawned: u32,
}

impl ParticleEmitter {
    pub fn new(position: Vec3, spawn_rate: f32) -> Self {
        Self {
            spawn_rate,
            burst_count: None,
            position,
            emission_shape: EmissionShape::Point,
            initial_velocity_range: (Vec3::ZERO, Vec3::ZERO),
            initial_scale_range: (Vec3::ONE, Vec3::ONE),
            lifetime_range: (5.0, 5.0),
            initial_color: [1.0; 4],
            spawn_accumulator: 0.0,
            total_spawned: 0,
        }
    }

    pub fn with_burst(mut self, count: u32) -> Self {
        self.burst_count = Some(count);
        self
    }

    pub fn with_emission_shape(mut self, shape: EmissionShape) -> Self {
        self.emission_shape = shape;
        self
    }

    pub fn with_velocity(mut self, min: Vec3, max: Vec3) -> Self {
        self.initial_velocity_range = (min, max);
        self
    }

    pub fn with_scale(mut self, min: Vec3, max: Vec3) -> Self {
        self.initial_scale_range = (min, max);
        self
    }

    pub fn with_lifetime(mut self, min: f32, max: f32) -> Self {
        self.lifetime_range = (min, max);
        self
    }

    pub fn with_color(mut self, color: [f32; 4]) -> Self {
        self.initial_color = color;
        self
    }

    pub fn update(&mut self, dt: f32) -> Vec<Particle> {
        if let Some(burst_count) = self.burst_count {
            if self.total_spawned >= burst_count {
                return Vec::new();
            }
        }

        self.spawn_accumulator += dt * self.spawn_rate;
        let to_spawn = self.spawn_accumulator.floor() as u32;
        self.spawn_accumulator -= to_spawn as f32;

        if let Some(burst_count) = self.burst_count {
            let remaining = burst_count.saturating_sub(self.total_spawned);
            let actual_spawn = to_spawn.min(remaining);
            self.total_spawned += actual_spawn;
            (0..actual_spawn).map(|_| self.spawn_particle()).collect()
        } else {
            (0..to_spawn).map(|_| self.spawn_particle()).collect()
        }
    }

    fn spawn_particle(&self) -> Particle {
        use rand::Rng;
        let mut rng = rand::thread_rng();

        let offset = match self.emission_shape {
            EmissionShape::Point => Vec3::ZERO,
            EmissionShape::Sphere { radius } => {
                let theta = rng.gen_range(0.0..std::f32::consts::TAU);
                let phi = rng.gen_range(0.0..std::f32::consts::PI);
                let r = rng.gen_range(0.0..radius);
                Vec3::new(
                    r * phi.sin() * theta.cos(),
                    r * phi.sin() * theta.sin(),
                    r * phi.cos(),
                )
            }
            EmissionShape::Box { half_extents } => Vec3::new(
                rng.gen_range(-half_extents.x..half_extents.x),
                rng.gen_range(-half_extents.y..half_extents.y),
                rng.gen_range(-half_extents.z..half_extents.z),
            ),
            EmissionShape::Cone { angle, radius } => {
                let theta = rng.gen_range(0.0..std::f32::consts::TAU);
                let phi = rng.gen_range(0.0..angle);
                let r = rng.gen_range(0.0..radius);
                Vec3::new(
                    r * phi.sin() * theta.cos(),
                    r * phi.cos(),
                    r * phi.sin() * theta.sin(),
                )
            }
        };

        let position = self.position + offset;
        let velocity = Vec3::new(
            rng.gen_range(self.initial_velocity_range.0.x..self.initial_velocity_range.1.x),
            rng.gen_range(self.initial_velocity_range.0.y..self.initial_velocity_range.1.y),
            rng.gen_range(self.initial_velocity_range.0.z..self.initial_velocity_range.1.z),
        );

        let scale = Vec3::new(
            rng.gen_range(self.initial_scale_range.0.x..self.initial_scale_range.1.x),
            rng.gen_range(self.initial_scale_range.0.y..self.initial_scale_range.1.y),
            rng.gen_range(self.initial_scale_range.0.z..self.initial_scale_range.1.z),
        );

        let lifetime = rng.gen_range(self.lifetime_range.0..self.lifetime_range.1);

        Particle {
            position: position.into(),
            lifetime: 0.0,
            velocity: velocity.into(),
            max_lifetime: lifetime,
            rotation: Quat::IDENTITY.into(),
            scale: scale.into(),
            angular_velocity: rng.gen_range(-1.0..1.0),
            color: self.initial_color,
            user_data: [0.0; 4],
        }
    }
}

// ============================================================================
// Main GPU Particle System
// ============================================================================

pub struct GpuParticleSystem {
    particles_buffer: wgpu::Buffer,
    params_buffer: wgpu::Buffer,

    compute_pipeline: wgpu::ComputePipeline,
    compute_bind_group: wgpu::BindGroup,

    render_pipeline: wgpu::RenderPipeline,
    render_bind_group: wgpu::BindGroup,
    material_buffer: wgpu::Buffer,

    max_particles: u32,
    active_particles: u32,
    workgroup_count: u32,

    emitters: Vec<ParticleEmitter>,

    render_format: wgpu::TextureFormat,
    render_sample_count: u32,
}

impl GpuParticleSystem {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &crate::renderer::Renderer,
        max_particles: u32,
        material: crate::renderer::Material,
        behavior: &dyn ParticleBehavior,
    ) -> Self {
        let particles_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ParticlesBuffer"),
            size: (max_particles as usize * std::mem::size_of::<Particle>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let params_buffer = behavior.create_params_buffer(device, queue);

        let material_data = crate::renderer::MaterialData::from_material(&material);
        let material_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ParticleMaterial"),
            contents: bytemuck::bytes_of(&material_data),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create compute pipeline (unchanged)
        let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ParticleComputeShader"),
            source: wgpu::ShaderSource::Wgsl(behavior.shader_source().into()),
        });

        let mut layout_entries = vec![
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: false },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];

        layout_entries.extend(behavior.additional_layout_entries());

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ParticleComputeBindLayout"),
            entries: &layout_entries,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ParticleComputePipelineLayout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ParticleComputePipeline"),
            layout: Some(&pipeline_layout),
            module: &compute_shader,
            entry_point: Some(behavior.entry_point()),
            compilation_options: Default::default(),
            cache: None,
        });

        let mut bind_entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: particles_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: params_buffer.as_entire_binding(),
            },
        ];

        bind_entries.extend(behavior.additional_bindings(device));

        let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ParticleComputeBindGroup"),
            layout: &bind_group_layout,
            entries: &bind_entries,
        });

        // ====================================================================
        // NEW: Create render pipeline using ShaderBuilder
        // ====================================================================
        let initial_stage = crate::renderer::CustomRenderStage::BeforePostprocess;
        let render_format = renderer.color_format_for_stage(initial_stage);
        let render_sample_count = renderer.sample_count_for_stage(initial_stage);

        // Determine if renderer uses bindless textures
        let uses_bindless = renderer.supports_bindless_textures();

        let (render_pipeline, render_bind_group) = Self::create_render_pipeline(
            device,
            renderer,
            &particles_buffer,
            &material_buffer,
            render_format,
            render_sample_count,
            uses_bindless,
        );

        let workgroup_count = max_particles.div_ceil(WORKGROUP_SIZE);

        Self {
            particles_buffer,
            params_buffer,
            compute_pipeline,
            compute_bind_group,
            render_pipeline,
            render_bind_group,
            material_buffer,
            max_particles,
            active_particles: 0,
            workgroup_count,
            emitters: Vec::new(),
            render_format,
            render_sample_count,
        }
    }

    fn create_render_pipeline(
        device: &wgpu::Device,
        renderer: &crate::renderer::Renderer,
        particles_buffer: &wgpu::Buffer,
        material_buffer: &wgpu::Buffer,
        color_format: wgpu::TextureFormat,
        sample_count: u32,
        uses_bindless: bool,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroup) {
        // NEW: Use ShaderBuilder to compose the particle shader
        // Particles use lighting and environment but NOT shadows
        let shader_source = ShaderBuilder::particles(uses_bindless)
            .build(include_str!("../shader/particle_render.wgsl"));

        let render_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ParticleRenderShader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let camera_layout = renderer.camera_bind_layout();
        let lights_layout = renderer.lights_bind_layout();
        let textures_layout = renderer.textures_bind_layout();

        let particle_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ParticleRenderBindLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let particle_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ParticleRenderBindGroup"),
            layout: &particle_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: particles_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: material_buffer.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ParticleRenderPipelineLayout"),
            bind_group_layouts: &[
                camera_layout,
                &particle_layout,
                lights_layout, // NEW: Now includes environment!
                textures_layout,
            ],
            push_constant_ranges: &[],
        });

        let pipeline =
            crate::renderer::PipelineBuilder::new(device, &pipeline_layout, &render_shader)
                .with_label("ParticleRenderPipeline")
                .with_vertex_buffer(crate::renderer::Vertex::layout())
                .with_color_target(color_format, Some(wgpu::BlendState::REPLACE))
                .with_depth_stencil(
                    wgpu::TextureFormat::Depth32Float,
                    true,
                    wgpu::CompareFunction::LessEqual,
                )
                .with_multisample(sample_count)
                .build();

        (pipeline, particle_bind_group)
    }

    pub fn add_emitter(&mut self, emitter: ParticleEmitter) {
        self.emitters.push(emitter);
    }

    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        behavior: &dyn ParticleBehavior,
        dt: f32,
    ) {
        // Update emitters and spawn new particles
        let mut new_particles = Vec::new();
        for emitter in &mut self.emitters {
            new_particles.extend(emitter.update(dt));
        }

        // Upload new particles
        if !new_particles.is_empty() && self.active_particles < self.max_particles {
            let space_available = (self.max_particles - self.active_particles) as usize;
            let to_spawn = new_particles.len().min(space_available);

            if to_spawn > 0 {
                let offset =
                    (self.active_particles as usize * std::mem::size_of::<Particle>()) as u64;
                queue.write_buffer(
                    &self.particles_buffer,
                    offset,
                    bytemuck::cast_slice(&new_particles[..to_spawn]),
                );
                self.active_particles += to_spawn as u32;
            }
        }

        // Update behavior parameters
        behavior.update_params(queue, &self.params_buffer, dt);

        // Run compute shader
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("ParticleComputePass"),
            timestamp_writes: None,
        });

        pass.set_pipeline(&self.compute_pipeline);
        pass.set_bind_group(0, &self.compute_bind_group, &[]);
        pass.dispatch_workgroups(self.workgroup_count, 1, 1);
    }

    fn ensure_render_pipeline(
        &mut self,
        device: &wgpu::Device,
        renderer: &crate::renderer::Renderer,
        stage: crate::renderer::CustomRenderStage,
    ) {
        let target_format = renderer.color_format_for_stage(stage);
        let target_sample_count = renderer.sample_count_for_stage(stage);

        if self.render_format == target_format && self.render_sample_count == target_sample_count {
            return;
        }

        let uses_bindless = renderer.supports_bindless_textures();

        let (pipeline, bind_group) = Self::create_render_pipeline(
            device,
            renderer,
            &self.particles_buffer,
            &self.material_buffer,
            target_format,
            target_sample_count,
            uses_bindless,
        );

        self.render_pipeline = pipeline;
        self.render_bind_group = bind_group;
        self.render_format = target_format;
        self.render_sample_count = target_sample_count;
    }

    pub fn render(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        renderer: &crate::renderer::Renderer,
        mesh: &crate::asset::Mesh,
        view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        stage: crate::renderer::CustomRenderStage,
    ) {
        if self.active_particles == 0 {
            return;
        }

        self.ensure_render_pipeline(renderer.get_device(), renderer, stage);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ParticleRenderPass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_pipeline(&self.render_pipeline);
        pass.set_bind_group(0, renderer.camera_bind_group(), &[]);
        pass.set_bind_group(1, &self.render_bind_group, &[]);
        pass.set_bind_group(2, renderer.lights_bind_group(), &[]); // Now includes environment!
        pass.set_bind_group(3, renderer.textures_bind_group(), &[]);

        pass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
        pass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());

        pass.draw_indexed(0..mesh.index_count(), 0, 0..self.active_particles);
    }

    pub fn active_particle_count(&self) -> u32 {
        self.active_particles
    }

    pub fn initialize_particles(&mut self, queue: &wgpu::Queue, particles: &[Particle]) {
        let count = particles.len().min(self.max_particles as usize);
        queue.write_buffer(
            &self.particles_buffer,
            0,
            bytemuck::cast_slice(&particles[..count]),
        );
        self.active_particles = count as u32;
    }
}
