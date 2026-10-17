use crate::renderer::shader_source::wgsl;
use crate::renderer::{PipelineBuilder, ShaderBuilder};

use super::{begin_color_pass, fullscreen_vertex, ColorAttachment};

/// Ray-traced ambient occlusion: traces short hemisphere rays from every G-buffer pixel
/// (placed by the depth buffer, which is more precise than the half-float G-buffer
/// position) into the occlusion texture SSAO uses, so [`super::ssao::SsaoStage::blur`]
/// and the composite finish it the same way.
pub struct RtAoStage {
    inputs_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    inputs_bind_group: Option<wgpu::BindGroup>,
    /// Rays alpha-test cut-out surfaces with the bindless textures (group 3).
    textures: bool,
}

/// The occlusion shader: post-processing's common helpers and the ray-traced pass, with
/// ray queries enabled; the scene is group 2 and, with `textures`, the bindless material
/// textures group 3 (for cut-out surfaces).
pub(crate) fn rt_ao_shader_source(textures: bool) -> String {
    let main = format!(
        "{}\n\n{}",
        crate::renderer::raytracing::scene_shader_modules(2, textures.then_some(3)),
        wgsl!("postprocess/rt_ao.wgsl")
    );
    let source = ShaderBuilder::new()
        .with_module(wgsl!("postprocess/fullscreen.wgsl"))
        .with_module(wgsl!("postprocess/common.wgsl"))
        .with_material_system()
        .build(&main);
    format!("enable wgpu_ray_query;\n\n{source}")
}

impl RtAoStage {
    /// Build the pipeline. Fails (instead of raising an uncaptured device error) when the
    /// driver or wgpu rejects it, so the caller can keep screen-space occlusion.
    pub fn new(
        device: &wgpu::Device,
        uniform_layout: &wgpu::BindGroupLayout,
        scene_layout: &wgpu::BindGroupLayout,
        textures_layout: Option<&wgpu::BindGroupLayout>,
    ) -> Result<Self, String> {
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);

        let unfilterable = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let inputs_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RtAoInputsLayout"),
            entries: &[
                unfilterable(180),
                unfilterable(181),
                wgpu::BindGroupLayoutEntry {
                    binding: 182,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RtAoShader"),
            source: wgpu::ShaderSource::Wgsl(rt_ao_shader_source(textures_layout.is_some()).into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RtAoPipelineLayout"),
            bind_group_layouts: &[
                Some(uniform_layout),
                Some(&inputs_layout),
                Some(scene_layout),
                textures_layout,
            ],
            immediate_size: 0,
        });
        let pipeline = PipelineBuilder::new(device, &pipeline_layout, &shader)
            .with_label("RtAoPipeline")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_rt_ao")
            .with_color_target(
                wgpu::TextureFormat::R8Unorm,
                Some(wgpu::BlendState::REPLACE),
            )
            .with_vertex_state(fullscreen_vertex(&shader))
            .with_no_culling()
            .build();

        let internal = pollster::block_on(internal.pop());
        let validation = pollster::block_on(validation.pop());
        if let Some(error) = validation.or(internal) {
            return Err(error.to_string());
        }
        Ok(Self {
            inputs_layout,
            pipeline,
            inputs_bind_group: None,
            textures: textures_layout.is_some(),
        })
    }

    pub fn invalidate(&mut self) {
        self.inputs_bind_group = None;
    }

    /// `depth` is single-sample (the resolved copy under MSAA).
    pub fn ensure_bind_groups(
        &mut self,
        device: &wgpu::Device,
        normal: &wgpu::TextureView,
        position: &wgpu::TextureView,
        depth: &wgpu::TextureView,
    ) {
        self.inputs_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("RtAoInputsBindGroup"),
            layout: &self.inputs_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 180,
                    resource: wgpu::BindingResource::TextureView(normal),
                },
                wgpu::BindGroupEntry {
                    binding: 181,
                    resource: wgpu::BindingResource::TextureView(position),
                },
                wgpu::BindGroupEntry {
                    binding: 182,
                    resource: wgpu::BindingResource::TextureView(depth),
                },
            ],
        }));
    }

    /// Trace into `occlusion` (the SSAO texture). Returns false, drawing nothing, when the
    /// bind group is not ready.
    pub fn trace(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        scene: &wgpu::BindGroup,
        textures: Option<&wgpu::BindGroup>,
        occlusion: &wgpu::TextureView,
    ) -> bool {
        let Some(inputs) = self.inputs_bind_group.as_ref() else {
            return false;
        };
        let textures = match (self.textures, textures) {
            (true, None) => return false,
            (true, textures) => textures,
            (false, _) => None,
        };
        let mut pass = begin_color_pass(
            encoder,
            "RtAo",
            ColorAttachment::single(occlusion),
            wgpu::LoadOp::Clear(wgpu::Color::WHITE),
            None,
        );
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, uniform_bind_group, &[]);
        pass.set_bind_group(1, inputs, &[]);
        pass.set_bind_group(2, scene, &[]);
        if let Some(textures) = textures {
            pass.set_bind_group(3, textures, &[]);
        }
        pass.draw(0..3, 0..1);
        true
    }
}
