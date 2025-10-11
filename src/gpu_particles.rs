use std::borrow::Cow;

use bytemuck::{Pod, Zeroable};
use glam::{Quat, Vec3};
use wgpu::util::DeviceExt;

use crate::renderer::{Material, Renderer, Vertex};

const WORKGROUP_SIZE: u32 = 256;

// Optimized particle state
#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParticleState {
    position: [f32; 3],
    speed: f32,
    rotation: [f32; 4],
    angular_axis: [f32; 3],
    angular_speed: f32,
    scale: f32,
    seed: u32,
    _padding: [u32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuParticleParams {
    dt: f32,
    near_plane: f32,
    far_plane: f32,
    far_reset_band: f32,
    field_half_size: f32,
    min_radius: f32,
    speed_min: f32,
    speed_max: f32,
    spin_min: f32,
    spin_max: f32,
    scale_min: f32,
    scale_max: f32,
    base_instance: u32,
    particle_count: u32,
    _padding: [u32; 2],
}

/// Immutable configuration parameters for the particle simulation.
#[derive(Clone)]
pub struct ParticleFieldSettings {
    pub near_plane: f32,
    pub far_plane: f32,
    pub far_reset_band: f32,
    pub field_half_size: f32,
    pub min_radius: f32,
    pub speed_range: std::ops::Range<f32>,
    pub spin_range: std::ops::Range<f32>,
    pub scale_range: std::ops::Range<f32>,
}

impl ParticleFieldSettings {
    pub fn speed_min(&self) -> f32 {
        self.speed_range.start
    }

    pub fn speed_max(&self) -> f32 {
        self.speed_range.end
    }

    pub fn spin_min(&self) -> f32 {
        self.spin_range.start
    }

    pub fn spin_max(&self) -> f32 {
        self.spin_range.end
    }

    pub fn scale_min(&self) -> f32 {
        self.scale_range.start
    }

    pub fn scale_max(&self) -> f32 {
        self.scale_range.end
    }
}

/// CPU-side description of a particle's initial state.
#[derive(Clone, Copy)]
pub struct ParticleInit {
    pub position: Vec3,
    pub speed: f32,
    pub rotation: Quat,
    pub angular_axis: Vec3,
    pub angular_speed: f32,
    pub scale: f32,
    pub seed: u32,
}

impl ParticleInit {
    fn as_state(&self) -> ParticleState {
        ParticleState {
            position: [self.position.x, self.position.y, self.position.z],
            speed: self.speed,
            rotation: [
                self.rotation.x,
                self.rotation.y,
                self.rotation.z,
                self.rotation.w,
            ],
            angular_axis: [
                self.angular_axis.x,
                self.angular_axis.y,
                self.angular_axis.z,
            ],
            angular_speed: self.angular_speed,
            scale: self.scale,
            seed: self.seed,
            _padding: [0, 0],
        }
    }
}

pub struct GpuParticleSystem {
    // Compute pipeline
    compute_pipeline: wgpu::ComputePipeline,
    compute_bind_group: wgpu::BindGroup,
    
    // Render pipeline for GPU-driven rendering
    render_pipeline: wgpu::RenderPipeline,
    render_bind_group: wgpu::BindGroup,
    
    params: GpuParticleParams,
    params_buffer: wgpu::Buffer,
    _state_buffer: wgpu::Buffer,
    _material_buffer: wgpu::Buffer,
    workgroup_count: u32,
    particle_count: u32,
    frame_count: u32,
}

impl GpuParticleSystem {
    pub fn new(
        renderer: &mut Renderer,
        particle_count: u32,
        material: Material,
        settings: ParticleFieldSettings,
        initial_particles: &[ParticleInit],
    ) -> Self {
        assert_eq!(particle_count as usize, initial_particles.len());

        let device = renderer.get_device();
        let queue = renderer.get_queue();
        
        // Create particle state buffer
        let state_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ParticleStateBuffer"),
            size: (particle_count as usize * std::mem::size_of::<ParticleState>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let params = GpuParticleParams {
            dt: 0.0,
            near_plane: settings.near_plane,
            far_plane: settings.far_plane,
            far_reset_band: settings.far_reset_band,
            field_half_size: settings.field_half_size,
            min_radius: settings.min_radius,
            speed_min: settings.speed_min(),
            speed_max: settings.speed_max(),
            spin_min: settings.spin_min(),
            spin_max: settings.spin_max(),
            scale_min: settings.scale_min(),
            scale_max: settings.scale_max(),
            base_instance: 0, // Not used anymore
            particle_count,
            _padding: [0; 2],
        };

        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ParticleParams"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Initialize particle states
        let initial_states: Vec<ParticleState> = initial_particles
            .iter()
            .map(ParticleInit::as_state)
            .collect();
        queue.write_buffer(&state_buffer, 0, bytemuck::cast_slice(&initial_states));

        // Create material buffer
        let material_data = crate::renderer::MaterialData::from_material(&material);
        let material_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("GpuParticleMaterial"),
            contents: bytemuck::bytes_of(&material_data),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create compute pipeline
        let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("GpuParticleCompute"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "shader/gpu_particles.wgsl"
            ))),
        });

        let compute_bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("GpuParticleComputeBindLayout"),
            entries: &[
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
                        min_binding_size: Some(
                            std::num::NonZeroU64::new(
                                std::mem::size_of::<GpuParticleParams>() as u64
                            )
                            .unwrap(),
                        ),
                    },
                    count: None,
                },
            ],
        });

        let compute_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("GpuParticleComputePipelineLayout"),
            bind_group_layouts: &[&compute_bind_layout],
            push_constant_ranges: &[],
        });

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("GpuParticleComputePipeline"),
            layout: Some(&compute_pipeline_layout),
            module: &compute_shader,
            entry_point: Some("update_particles"),
            compilation_options: Default::default(),
            cache: None,
        });

        let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GpuParticleComputeBindGroup"),
            layout: &compute_bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: state_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });

        // Create render pipeline
        let (render_pipeline, render_bind_group) = Self::create_render_pipeline(
            device,
            renderer,
            &state_buffer,
            &material_buffer,
        );

        let workgroup_count = particle_count.div_ceil(WORKGROUP_SIZE);

        log::info!(
            "GPU Particle System (GPU-driven) initialized: {} particles, {} workgroups",
            particle_count,
            workgroup_count
        );

        Self {
            compute_pipeline,
            compute_bind_group,
            render_pipeline,
            render_bind_group,
            params,
            params_buffer,
            _state_buffer: state_buffer,
            _material_buffer: material_buffer,
            workgroup_count,
            particle_count,
            frame_count: 0,
        }
    }

    fn create_render_pipeline(
        device: &wgpu::Device,
        renderer: &Renderer,
        state_buffer: &wgpu::Buffer,
        material_buffer: &wgpu::Buffer,
    ) -> (wgpu::RenderPipeline, wgpu::BindGroup) {
        let render_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("GpuParticleRender"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "shader/gpu_particle_render.wgsl"
            ))),
        });

        // Get existing bind group layouts from renderer
        let camera_layout = renderer.camera_bind_layout();
        let lights_layout = renderer.lights_bind_layout();
        let textures_layout = renderer.textures_bind_layout();

        // Create particle-specific bind group layout
        let particle_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("GpuParticleRenderBindLayout"),
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
            label: Some("GpuParticleRenderBindGroup"),
            layout: &particle_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: state_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: material_buffer.as_entire_binding(),
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("GpuParticleRenderPipelineLayout"),
            bind_group_layouts: &[
                camera_layout,
                &particle_layout,
                lights_layout,
                textures_layout,
            ],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("GpuParticleRenderPipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &render_shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::layout()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &render_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: renderer.surface_format(),
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                front_face: wgpu::FrontFace::Ccw,
                polygon_mode: wgpu::PolygonMode::Fill,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: renderer.sample_count(),
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        (pipeline, particle_bind_group)
    }

    pub fn update(&mut self, renderer: &mut Renderer, dt: f32) {
        if dt <= f32::EPSILON {
            return;
        }

        self.frame_count += 1;
        self.params.dt = dt;
        
        renderer
            .get_queue()
            .write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&self.params));

        let mut encoder =
            renderer
                .get_device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("GpuParticleEncoder"),
                });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("GpuParticleComputePass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.compute_pipeline);
            pass.set_bind_group(0, &self.compute_bind_group, &[]);
            pass.dispatch_workgroups(self.workgroup_count, 1, 1);
        }

        renderer.get_queue().submit(Some(encoder.finish()));

        if self.frame_count % 300 == 0 {
            let fps = if dt > 0.0 { 1.0 / dt } else { 0.0 };
            log::debug!(
                "GPU Particles (GPU-driven) FPS: {:.1}, dt: {:.3}ms",
                fps,
                dt * 1000.0
            );
        }
    }

    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        renderer: &Renderer,
        mesh: &crate::asset::Mesh,
        view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
    ) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("GpuParticleRenderPass"),
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
        pass.set_bind_group(2, renderer.lights_bind_group(), &[]);
        pass.set_bind_group(3, renderer.textures_bind_group(), &[]);
        
        pass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
        pass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());
        
        // Draw all particles in one call!
        pass.draw_indexed(0..mesh.index_count(), 0, 0..self.particle_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn particle_state_size_is_reasonable() {
        let size = std::mem::size_of::<ParticleState>();
        assert!(
            size <= 64,
            "ParticleState is {} bytes, should be <= 64",
            size
        );
    }
}