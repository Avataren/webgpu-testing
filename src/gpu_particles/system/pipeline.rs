use crate::renderer::{PipelineBuilder, Renderer, SamplerFilterMode, ShaderBuilder, Vertex};

#[allow(clippy::too_many_arguments)]
pub(crate) fn create_render_pipeline(
    device: &wgpu::Device,
    renderer: &Renderer,
    particle_layout: &wgpu::BindGroupLayout,
    color_format: wgpu::TextureFormat,
    sample_count: u32,
    uses_bindless: bool,
    filtering: SamplerFilterMode,
    blend_state: Option<wgpu::BlendState>,
    depth_write_enabled: bool,
) -> wgpu::RenderPipeline {
    let shader_source = ShaderBuilder::particles_filtered(uses_bindless, filtering)
        .build(include_str!("../../shader/particle_render.wgsl"));

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
        .with_color_target(color_format, blend_state)
        .with_depth_stencil(
            wgpu::TextureFormat::Depth32Float,
            depth_write_enabled,
            wgpu::CompareFunction::LessEqual,
        )
        .with_multisample(sample_count)
        .build()
}
