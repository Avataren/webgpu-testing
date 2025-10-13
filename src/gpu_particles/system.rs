use bytemuck::{Pod, Zeroable};
use glam::Mat4;
use wgpu::util::DeviceExt;

use crate::asset::Mesh;
use crate::renderer::{
    CustomRenderContext, CustomRenderStage, Material, MaterialData, PipelineBuilder, Renderer,
    SamplerFilterMode, ShaderBuilder, ShadowPassStage, Vertex,
};

use super::behavior::ParticleBehavior;
use super::emitter::ParticleEmitter;
use super::particle::Particle;

const WORKGROUP_SIZE: u32 = 256;

pub struct GpuParticleSystem {
    particles_buffer: wgpu::Buffer,
    params_buffer: wgpu::Buffer,

    compute_pipeline: wgpu::ComputePipeline,
    compute_bind_group: wgpu::BindGroup,

    render_pipeline: wgpu::RenderPipeline,
    render_bind_group: wgpu::BindGroup,
    particle_bind_group_layout: wgpu::BindGroupLayout,
    _material_buffer: wgpu::Buffer,

    shadow_resources: Option<ParticleShadowResources>,

    max_particles: u32,
    active_particles: u32,
    workgroup_count: u32,

    emitters: Vec<ParticleEmitter>,

    render_format: wgpu::TextureFormat,
    render_sample_count: u32,
    sampler_filtering: SamplerFilterMode,
    casts_shadows: bool,
}

struct ParticleShadowResources {
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
    fn new(device: &wgpu::Device, particle_layout: &wgpu::BindGroupLayout) -> Self {
        let shader_source =
            ShaderBuilder::new().build(include_str!("../shader/particle_shadow.wgsl"));
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

    fn update_view_proj(&self, queue: &wgpu::Queue, matrix: Mat4) {
        let uniform = ParticleShadowUniform {
            view_proj: matrix.to_cols_array_2d(),
        };
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }
}

impl GpuParticleSystem {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &Renderer,
        max_particles: u32,
        material: Material,
        behavior: &dyn ParticleBehavior,
    ) -> Self {
        let particles_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ParticlesBuffer"),
            size: (max_particles as usize * std::mem::size_of::<Particle>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let params_buffer = behavior.create_params_buffer(device, queue);

        let sampler_filtering = material.sampler_filtering();
        let material_data = MaterialData::from_material(&material);
        let material_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ParticleMaterial"),
            contents: bytemuck::bytes_of(&material_data),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

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

        let initial_stage = CustomRenderStage::BeforePostprocess;
        let render_format = renderer.color_format_for_stage(initial_stage);
        let render_sample_count = renderer.sample_count_for_stage(initial_stage);
        let uses_bindless = renderer.supports_bindless_textures();

        let particle_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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

        let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ParticleRenderBindGroup"),
            layout: &particle_bind_group_layout,
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

        let render_pipeline = Self::create_render_pipeline(
            device,
            renderer,
            &particle_bind_group_layout,
            render_format,
            render_sample_count,
            uses_bindless,
            sampler_filtering,
        );

        let workgroup_count = max_particles.div_ceil(WORKGROUP_SIZE);

        Self {
            particles_buffer,
            params_buffer,
            compute_pipeline,
            compute_bind_group,
            render_pipeline,
            render_bind_group,
            particle_bind_group_layout,
            _material_buffer: material_buffer,
            shadow_resources: None,
            max_particles,
            active_particles: 0,
            workgroup_count,
            emitters: Vec::new(),
            render_format,
            render_sample_count,
            sampler_filtering,
            casts_shadows: false,
        }
    }

    fn create_render_pipeline(
        device: &wgpu::Device,
        renderer: &Renderer,
        particle_layout: &wgpu::BindGroupLayout,
        color_format: wgpu::TextureFormat,
        sample_count: u32,
        uses_bindless: bool,
        filtering: SamplerFilterMode,
    ) -> wgpu::RenderPipeline {
        let shader_source = ShaderBuilder::particles_filtered(uses_bindless, filtering)
            .build(include_str!("../shader/particle_render.wgsl"));

        let render_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ParticleRenderShader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let camera_layout = renderer.camera_bind_layout();
        let lights_layout = renderer.lights_bind_layout();
        let textures_layout = renderer.textures_bind_layout();

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ParticleRenderPipelineLayout"),
            bind_group_layouts: &[
                camera_layout,
                particle_layout,
                lights_layout,
                textures_layout,
            ],
            push_constant_ranges: &[],
        });

        PipelineBuilder::new(device, &pipeline_layout, &render_shader)
            .with_label("ParticleRenderPipeline")
            .with_vertex_buffer(Vertex::layout())
            .with_color_target(color_format, Some(wgpu::BlendState::REPLACE))
            .with_depth_stencil(
                wgpu::TextureFormat::Depth32Float,
                true,
                wgpu::CompareFunction::LessEqual,
            )
            .with_multisample(sample_count)
            .build()
    }

    pub fn add_emitter(&mut self, emitter: ParticleEmitter) {
        self.emitters.push(emitter);
    }

    pub fn particles_buffer(&self) -> &wgpu::Buffer {
        &self.particles_buffer
    }

    pub fn update(
        &mut self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        behavior: &dyn ParticleBehavior,
        dt: f32,
    ) {
        let mut new_particles = Vec::new();
        for emitter in &mut self.emitters {
            new_particles.extend(emitter.update(dt));
        }

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

        behavior.update_params(queue, &self.params_buffer, dt);

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
        renderer: &Renderer,
        stage: CustomRenderStage,
    ) {
        if matches!(stage, CustomRenderStage::Shadow(_)) {
            return;
        }

        let target_format = renderer.color_format_for_stage(stage);
        let target_sample_count = renderer.sample_count_for_stage(stage);

        if self.render_format == target_format && self.render_sample_count == target_sample_count {
            return;
        }

        let uses_bindless = renderer.supports_bindless_textures();

        let pipeline = Self::create_render_pipeline(
            device,
            renderer,
            &self.particle_bind_group_layout,
            target_format,
            target_sample_count,
            uses_bindless,
            self.sampler_filtering,
        );

        self.render_pipeline = pipeline;
        self.render_format = target_format;
        self.render_sample_count = target_sample_count;
    }

    pub fn render(&mut self, ctx: &mut CustomRenderContext<'_>, mesh: &Mesh) {
        if self.active_particles == 0 {
            return;
        }

        match ctx.stage {
            CustomRenderStage::BeforePostprocess | CustomRenderStage::AfterPostprocess => {
                self.ensure_render_pipeline(ctx.renderer.get_device(), ctx.renderer, ctx.stage);

                let mut pass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ParticleRenderPass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: ctx.color_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: ctx.depth_view,
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
                pass.set_bind_group(0, ctx.renderer.camera_bind_group(), &[]);
                pass.set_bind_group(1, &self.render_bind_group, &[]);
                pass.set_bind_group(2, ctx.renderer.lights_bind_group(), &[]);
                pass.set_bind_group(3, ctx.renderer.textures_bind_group(), &[]);

                pass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
                pass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());

                pass.draw_indexed(0..mesh.index_count(), 0, 0..self.active_particles);
            }
            CustomRenderStage::Shadow(stage_info) => {
                if !self.casts_shadows {
                    return;
                }

                if let Some(matrix) = ctx.shadow_view_proj() {
                    self.render_shadow(ctx, mesh, stage_info, matrix);
                } else {
                    log::warn!("Shadow render requested without view-projection matrix");
                }
            }
        }
    }

    fn render_shadow(
        &mut self,
        ctx: &mut CustomRenderContext<'_>,
        mesh: &Mesh,
        _stage: ShadowPassStage,
        view_proj: Mat4,
    ) {
        if self.shadow_resources.is_none() {
            let resources = ParticleShadowResources::new(
                ctx.renderer.get_device(),
                &self.particle_bind_group_layout,
            );
            self.shadow_resources = Some(resources);
        }

        let resources = self.shadow_resources.as_mut().expect("shadow resources");
        resources.update_view_proj(ctx.renderer.get_queue(), view_proj);

        let mut pass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ParticleShadowPass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: ctx.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_pipeline(&resources.pipeline);
        pass.set_bind_group(0, &resources.uniform_bind_group, &[]);
        pass.set_bind_group(1, &self.render_bind_group, &[]);
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

    pub fn set_casts_shadows(&mut self, casts_shadows: bool) {
        self.casts_shadows = casts_shadows;
    }

    pub fn casts_shadows(&self) -> bool {
        self.casts_shadows
    }
}
