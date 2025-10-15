use bytemuck::{bytes_of, Pod, Zeroable};
use glam::Mat4;

use crate::gpu_particles::shader_modules::GPU_PARTICLE_COMMON;
use crate::renderer::{PipelineBuilder, ShaderBuilder, Vertex};

pub(crate) struct ParticleShadowResources {
    pipeline: wgpu::RenderPipeline,
    uniform_buffer: wgpu::Buffer,
    _uniform_bind_group_layout: wgpu::BindGroupLayout,
    uniform_bind_group: wgpu::BindGroup,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParticleShadowUniform {
    view_proj: [[f32; 4]; 4],
}

impl ParticleShadowResources {
    pub(crate) fn new(device: &wgpu::Device, particle_layout: &wgpu::BindGroupLayout) -> Self {
        let shader_source = ShaderBuilder::new()
            .with_module(GPU_PARTICLE_COMMON)
            .build(include_str!("../../shader/particle_shadow.wgsl"));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ParticleShadowShader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let uniform_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ParticleShadowUniformLayout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ParticleShadowUniformBuffer"),
            size: std::mem::size_of::<ParticleShadowUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ParticleShadowUniformBindGroup"),
            layout: &uniform_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ParticleShadowPipelineLayout"),
            bind_group_layouts: &[&uniform_bind_group_layout, particle_layout],
            push_constant_ranges: &[],
        });

        let pipeline = PipelineBuilder::new(device, &pipeline_layout, &shader)
            .with_label("ParticleShadowPipeline")
            .with_vertex_entry("vs_main")
            .depth_only()
            .with_vertex_buffer(Vertex::layout())
            .with_depth_stencil_biased(
                wgpu::TextureFormat::Depth32Float,
                true,
                wgpu::CompareFunction::LessEqual,
                2,
                2.0,
            )
            .build();

        Self {
            pipeline,
            uniform_buffer,
            _uniform_bind_group_layout: uniform_bind_group_layout,
            uniform_bind_group,
        }
    }

    pub(crate) fn update_view_proj(&self, queue: &wgpu::Queue, matrix: Mat4) {
        let uniform = ParticleShadowUniform {
            view_proj: matrix.to_cols_array_2d(),
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytes_of(&uniform));
    }

    pub(crate) fn pipeline(&self) -> &wgpu::RenderPipeline {
        &self.pipeline
    }

    pub(crate) fn uniform_bind_group(&self) -> &wgpu::BindGroup {
        &self.uniform_bind_group
    }
}
