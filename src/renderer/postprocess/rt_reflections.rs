use crate::renderer::{PipelineBuilder, RenderRegion, ShaderBuilder};

use super::pipelines::{sampler_entry, texture_entry};
use super::{begin_color_pass, fullscreen_vertex, ColorAttachment, HDR_FORMAT};
use crate::renderer::raytracing::RayTracingFrame;

/// Ray-traced reflections: traces each smooth pixel's reflection against the scene's
/// acceleration structure into the half-resolution reflection texture SSR uses, so
/// [`super::ssr::SsrStage::resolve`] blurs and applies it the same way.
pub struct RtReflectionStage {
    inputs_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    inputs_bind_group: Option<wgpu::BindGroup>,
    /// Ray-traced refraction through transmissive surfaces, added to the scene.
    refraction_pipeline: wgpu::RenderPipeline,
    /// As `inputs_bind_group`, but reading a copy of the scene colour, which the
    /// refraction pass adds to.
    refraction_bind_group: Option<wgpu::BindGroup>,
    /// Whether hits sample material textures (group 4 is the bindless texture array).
    textures: bool,
}

pub struct RtReflectionInputs<'a> {
    pub scene: &'a wgpu::TextureView,
    /// A copy of `scene` taken before refraction is added to it.
    pub scene_copy: &'a wgpu::TextureView,
    pub normal: &'a wgpu::TextureView,
    pub position: &'a wgpu::TextureView,
    pub sampler: &'a wgpu::Sampler,
}

/// The trace shader: post-processing helpers plus the main pass's material, lighting and
/// environment code, with ray queries enabled. With `textures`, hits sample material
/// textures from the bindless texture array bound as group 4.
pub(crate) fn rt_reflections_shader_source(textures: bool) -> String {
    let textures_module = if textures {
        include_str!("../../shader/postprocess/rt_textures_bindless.wgsl")
    } else {
        include_str!("../../shader/postprocess/rt_textures_none.wgsl")
    };
    let modules = ShaderBuilder::new()
        .with_module(include_str!("../../shader/postprocess/fullscreen.wgsl"))
        .with_module(include_str!("../../shader/postprocess/common.wgsl"))
        .with_module(include_str!(
            "../../shader/postprocess/reflection_common.wgsl"
        ))
        .with_constants()
        .with_material_system()
        .with_lighting()
        .with_environment()
        .with_module(textures_module)
        .with_module(include_str!("../../shader/postprocess/rt_reflections.wgsl"))
        .build_modules_only();
    format!("enable wgpu_ray_query;\n\n{modules}")
}

impl RtReflectionStage {
    /// Build the trace pipeline. Fails (instead of raising an uncaptured device error)
    /// when the driver or wgpu rejects the shader or pipeline, so the caller can keep
    /// screen-space reflections. `textures_layout` is the bindless texture array's
    /// layout, when hits can sample material textures.
    pub fn new(
        device: &wgpu::Device,
        uniform_layout: &wgpu::BindGroupLayout,
        lights_layout: &wgpu::BindGroupLayout,
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
            label: Some("RtReflectionInputsLayout"),
            entries: &[
                texture_entry(170),
                unfilterable(171),
                unfilterable(172),
                sampler_entry(173),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RtReflectionShader"),
            source: wgpu::ShaderSource::Wgsl(
                rt_reflections_shader_source(textures_layout.is_some()).into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RtReflectionPipelineLayout"),
            bind_group_layouts: &[
                Some(uniform_layout),
                Some(&inputs_layout),
                Some(lights_layout),
                Some(scene_layout),
                textures_layout,
            ],
            immediate_size: 0,
        });
        let pipeline = PipelineBuilder::new(device, &pipeline_layout, &shader)
            .with_label("RtReflectionTracePipeline")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_rt_reflection_trace")
            .with_color_target(HDR_FORMAT, Some(wgpu::BlendState::REPLACE))
            .with_vertex_state(fullscreen_vertex(&shader))
            .with_no_culling()
            .build();
        // Adds the refracted light to the scene's colour and keeps its alpha.
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let refraction_pipeline = PipelineBuilder::new(device, &pipeline_layout, &shader)
            .with_label("RtRefractionTracePipeline")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_rt_refraction_trace")
            .with_color_target(HDR_FORMAT, Some(add))
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
            refraction_pipeline,
            refraction_bind_group: None,
            textures: textures_layout.is_some(),
        })
    }

    pub fn invalidate(&mut self) {
        self.inputs_bind_group = None;
        self.refraction_bind_group = None;
    }

    pub fn ensure_bind_groups(&mut self, device: &wgpu::Device, inputs: RtReflectionInputs<'_>) {
        let create = |label, scene| {
            let texture = |binding, view| wgpu::BindGroupEntry {
                binding,
                resource: wgpu::BindingResource::TextureView(view),
            };
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.inputs_layout,
                entries: &[
                    texture(170, scene),
                    texture(171, inputs.normal),
                    texture(172, inputs.position),
                    wgpu::BindGroupEntry {
                        binding: 173,
                        resource: wgpu::BindingResource::Sampler(inputs.sampler),
                    },
                ],
            })
        };
        self.inputs_bind_group = Some(create("RtReflectionInputsBindGroup", inputs.scene));
        self.refraction_bind_group = Some(create("RtRefractionInputsBindGroup", inputs.scene_copy));
    }

    fn textures<'a>(&self, frame: &RayTracingFrame<'a>) -> Result<Option<&'a wgpu::BindGroup>, ()> {
        match (self.textures, frame.textures) {
            (true, Some(textures)) => Ok(Some(textures)),
            (true, None) => Err(()),
            (false, _) => Ok(None),
        }
    }

    /// Add the light refracted through transmissive surfaces to `scene` (full
    /// resolution); `scene_copy` must hold the scene as it is now. Returns false, drawing
    /// nothing, when the bind groups are not ready.
    pub fn refract(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        frame: &RayTracingFrame<'_>,
        scene: &wgpu::TextureView,
        region: Option<RenderRegion>,
    ) -> bool {
        let Some(inputs) = self.refraction_bind_group.as_ref() else {
            return false;
        };
        let Ok(textures) = self.textures(frame) else {
            return false;
        };
        let mut pass = begin_color_pass(
            encoder,
            "RtRefraction",
            ColorAttachment::single(scene),
            wgpu::LoadOp::Load,
            region,
        );
        pass.set_pipeline(&self.refraction_pipeline);
        pass.set_bind_group(0, uniform_bind_group, &[]);
        pass.set_bind_group(1, inputs, &[]);
        pass.set_bind_group(2, frame.lights, &[]);
        pass.set_bind_group(3, frame.scene, &[]);
        if let Some(textures) = textures {
            pass.set_bind_group(4, textures, &[]);
        }
        pass.draw(0..3, 0..1);
        true
    }

    /// Trace into `reflection` (half resolution). Returns false, drawing nothing, when
    /// the bind groups are not ready (including the texture array the pipeline was
    /// built for).
    pub fn trace(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        frame: &RayTracingFrame<'_>,
        reflection: &wgpu::TextureView,
    ) -> bool {
        let Some(inputs) = self.inputs_bind_group.as_ref() else {
            return false;
        };
        let Ok(textures) = self.textures(frame) else {
            return false;
        };
        let mut pass = begin_color_pass(
            encoder,
            "RtReflectionTrace",
            ColorAttachment::single(reflection),
            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
            None::<RenderRegion>,
        );
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, uniform_bind_group, &[]);
        pass.set_bind_group(1, inputs, &[]);
        pass.set_bind_group(2, frame.lights, &[]);
        pass.set_bind_group(3, frame.scene, &[]);
        if let Some(textures) = textures {
            pass.set_bind_group(4, textures, &[]);
        }
        pass.draw(0..3, 0..1);
        true
    }
}
