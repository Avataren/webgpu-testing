//! The light probe debug view: a sphere at every probe of the grid (where relocation put
//! it), shaded with the light its irradiance tile holds for each direction, over the
//! finished image and depth tested against the scene (`light_probe_debug.wgsl`).

use bytemuck::{Pod, Zeroable};

use crate::renderer::light_probes::LightProbes;

const SHADER_SOURCE: &str = concat!(
    include_str!("../shader/camera.wgsl"),
    include_str!("../shader/light_probe_debug.wgsl"),
);

/// How the debug view draws the probes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightProbeDebugView {
    /// Sphere radius (m).
    pub radius: f32,
    /// A factor on the light shown (before a simple tone curve).
    pub exposure: f32,
}

impl Default for LightProbeDebugView {
    fn default() -> Self {
        Self {
            radius: 0.06,
            exposure: 3.0,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DebugUniform {
    params: [f32; 4],
}

struct PipelineState {
    pipeline: wgpu::RenderPipeline,
    format: wgpu::TextureFormat,
}

pub(crate) struct LightProbeDebugStage {
    view: Option<LightProbeDebugView>,
    layout: Option<wgpu::BindGroupLayout>,
    uniform: Option<wgpu::Buffer>,
    sampler: Option<wgpu::Sampler>,
    pipeline: Option<PipelineState>,
}

impl LightProbeDebugStage {
    pub(crate) fn new() -> Self {
        Self {
            view: None,
            layout: None,
            uniform: None,
            sampler: None,
            pipeline: None,
        }
    }

    pub(crate) fn view(&self) -> Option<LightProbeDebugView> {
        self.view
    }

    pub(crate) fn set_view(&mut self, view: Option<LightProbeDebugView>) {
        self.view = view;
    }

    /// Draw the probes into `target` (depth tested against `depth`, not written).
    /// Returns the draw calls.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        probes: &LightProbes,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        depth: &wgpu::TextureView,
        depth_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
        camera_group: &wgpu::BindGroup,
        viewport: (f32, f32, f32, f32),
    ) -> u32 {
        let Some(view) = self.view else {
            return 0;
        };
        let Some(probe_count) = probes.debug_probe_count() else {
            return 0;
        };
        self.ensure_pipeline(device, format, depth_format, camera_layout);
        let (Some(layout), Some(uniform), Some(sampler), Some(state)) =
            (&self.layout, &self.uniform, &self.sampler, &self.pipeline)
        else {
            return 0;
        };
        queue.write_buffer(
            uniform,
            0,
            bytemuck::bytes_of(&DebugUniform {
                params: [view.radius.max(0.001), view.exposure.max(0.0), 0.0, 0.0],
            }),
        );
        // Made each frame: the atlases are recreated when the grid's layout changes.
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("LightProbeDebug"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: probes.grid_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(probes.irradiance_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(sampler),
                },
            ],
        });

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("LightProbeDebug"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let (x, y, width, height) = viewport;
        pass.set_viewport(x, y, width.max(1.0), height.max(1.0), 0.0, 1.0);
        pass.set_pipeline(&state.pipeline);
        pass.set_bind_group(0, camera_group, &[]);
        pass.set_bind_group(1, &bind_group, &[]);
        pass.draw(0..6, 0..probe_count);
        1
    }

    fn ensure_pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
    ) {
        if self
            .pipeline
            .as_ref()
            .is_some_and(|state| state.format == format)
        {
            return;
        }
        let layout = self.layout.get_or_insert_with(|| {
            let uniform = |binding| wgpu::BindGroupLayoutEntry {
                binding,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            };
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("LightProbeDebug"),
                entries: &[
                    uniform(0),
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    uniform(2),
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            })
        });
        self.uniform.get_or_insert_with(|| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("LightProbeDebug"),
                size: std::mem::size_of::<DebugUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        });
        self.sampler.get_or_insert_with(|| {
            device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("LightProbeDebug"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            })
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("LightProbeDebug"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("LightProbeDebug"),
            bind_group_layouts: &[Some(camera_layout), Some(layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("LightProbeDebug"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_probe"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_probe"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.pipeline = Some(PipelineState { pipeline, format });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shader_validates() {
        let source = super::SHADER_SOURCE;
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|err| panic!("{}", err.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|err| panic!("{}", err.emit_to_string(source)));
    }
}
