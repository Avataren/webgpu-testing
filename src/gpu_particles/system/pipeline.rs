use crate::renderer::shader_source::wgsl;
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
    fog_adds_light: bool,
) -> wgpu::RenderPipeline {
    let shader_source = render_shader_source(uses_bindless, filtering, fog_adds_light);

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
            Some(camera_layout),
            Some(particle_layout),
            Some(lights_layout),
            Some(textures_layout),
        ],
        immediate_size: 0,
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
        // Particles are seen from every side (a flat quad that is not billboarded would
        // vanish from behind), as ray tracing sees them.
        .with_no_culling()
        .build()
}

/// The particle shader; `fog_adds_light` false for additive particles, whose light the fog
/// only dims.
fn render_shader_source(
    uses_bindless: bool,
    filtering: SamplerFilterMode,
    fog_adds_light: bool,
) -> String {
    let source = ShaderBuilder::particles_filtered(uses_bindless, filtering)
        .build(wgsl!("particle_render.wgsl"));
    if fog_adds_light {
        source
    } else {
        source.replace(
            "const PARTICLE_FOG_ADDS_LIGHT: bool = true;",
            "const PARTICLE_FOG_ADDS_LIGHT: bool = false;",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_particle_shader_validates() {
        for bindless in [false, true] {
            for fog_adds_light in [false, true] {
                let source =
                    render_shader_source(bindless, SamplerFilterMode::Linear, fog_adds_light);
                assert_eq!(
                    source.contains("const PARTICLE_FOG_ADDS_LIGHT: bool = false;"),
                    !fog_adds_light
                );
                crate::renderer::shader_source::validate(&source).unwrap_or_else(|error| {
                    panic!("bindless {bindless}, fog light {fog_adds_light}: {error}")
                });
            }
        }
    }
}
