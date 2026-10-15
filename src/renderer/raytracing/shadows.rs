//! Ray-traced shadows of directional lights: a screen-space mask traced between the
//! depth prepass and the main pass (`shader/rt_shadows.wgsl`), which the main pass reads
//! instead of the shadow maps for those lights.

use bytemuck::{Pod, Zeroable};

use crate::renderer::lights::LightsData;
use crate::renderer::{PipelineBuilder, RenderRegion};

/// Directional lights the mask has a channel for.
pub(crate) const RT_SHADOW_LIGHTS: usize = 4;
const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Mirrors `RtShadowParams` in rt_shadows.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
struct RtShadowUniform {
    lights: [[f32; 4]; RT_SHADOW_LIGHTS],
    params: [f32; 4],
    region: [f32; 4],
}

pub(crate) struct RtShadowStage {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniform: wgpu::Buffer,
    last_uniform: Option<RtShadowUniform>,
    bind_group: Option<wgpu::BindGroup>,
    /// The depth view `bind_group` reads.
    bound_depth: Option<wgpu::TextureView>,
    mask: Option<(wgpu::Texture, wgpu::TextureView)>,
}

impl RtShadowStage {
    /// Build the mask pipeline; fails (instead of raising an uncaptured device error) when
    /// the driver or wgpu rejects it, so the caller keeps shadow maps.
    pub(crate) fn new(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        scene_layout: &wgpu::BindGroupLayout,
    ) -> Result<Self, String> {
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let internal = device.push_error_scope(wgpu::ErrorFilter::Internal);

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("RtShadowLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
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
            label: Some("RtShadowShader"),
            source: wgpu::ShaderSource::Wgsl(rt_shadows_shader_source().into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RtShadowPipelineLayout"),
            bind_group_layouts: &[Some(camera_layout), Some(&layout), Some(scene_layout)],
            immediate_size: 0,
        });
        let pipeline = PipelineBuilder::new(device, &pipeline_layout, &shader)
            .with_label("RtShadowPipeline")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_rt_shadows")
            .with_color_target(MASK_FORMAT, Some(wgpu::BlendState::REPLACE))
            .with_no_culling()
            .build();
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("RtShadowUniform"),
            size: std::mem::size_of::<RtShadowUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let internal = pollster::block_on(internal.pop());
        let validation = pollster::block_on(validation.pop());
        if let Some(error) = validation.or(internal) {
            return Err(error.to_string());
        }
        Ok(Self {
            pipeline,
            layout,
            uniform,
            last_uniform: None,
            bind_group: None,
            bound_depth: None,
            mask: None,
        })
    }

    /// The mask the main pass reads, once [`Self::ensure_targets`] made one.
    pub(crate) fn mask_view(&self) -> Option<&wgpu::TextureView> {
        self.mask.as_ref().map(|(_, view)| view)
    }

    /// Make the mask match `size` and the bind group read `depth` (the depth prepass's
    /// single-sample depth). Returns whether the mask was (re)created, so whoever binds it
    /// can rebind.
    pub(crate) fn ensure_targets(
        &mut self,
        device: &wgpu::Device,
        size: wgpu::Extent3d,
        depth: &wgpu::TextureView,
    ) -> bool {
        let mut mask_changed = false;
        if self
            .mask
            .as_ref()
            .is_none_or(|(texture, _)| texture.size() != size)
        {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("RtShadowMask"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: MASK_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            });
            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            self.mask = Some((texture, view));
            mask_changed = true;
        }
        if self.bound_depth.as_ref() != Some(depth) {
            self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("RtShadowBindGroup"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: self.uniform.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(depth),
                    },
                ],
            }));
            self.bound_depth = Some(depth.clone());
        }
        mask_changed
    }

    /// Trace the mask for `lights`' shadow-casting directional lights. Returns the lights
    /// traced as a bit set (bit i = directional light i), or 0 when nothing ran.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn trace(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        camera: &wgpu::BindGroup,
        scene: &wgpu::BindGroup,
        lights: &LightsData,
        softness_degrees: f32,
        frame: u32,
        region: Option<RenderRegion>,
    ) -> u32 {
        let (Some(bind_group), Some((texture, view))) = (self.bind_group.as_ref(), &self.mask)
        else {
            return 0;
        };
        let (directions, bits) = traced_lights(lights);
        if bits == 0 {
            return 0;
        }
        let size = texture.size();
        let region_rect = region.map_or(
            [0.0, 0.0, size.width as f32, size.height as f32],
            |region| {
                [
                    region.x() as f32,
                    region.y() as f32,
                    region.width().max(1) as f32,
                    region.height().max(1) as f32,
                ]
            },
        );
        let uniform = RtShadowUniform {
            lights: directions,
            params: [
                (0.5 * softness_degrees).to_radians().tan(),
                (frame % 1024) as f32,
                0.0,
                0.0,
            ],
            region: region_rect,
        };
        if self.last_uniform != Some(uniform) {
            queue.write_buffer(&self.uniform, 0, bytemuck::bytes_of(&uniform));
            self.last_uniform = Some(uniform);
        }

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("RtShadows"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(region) = region {
            pass.set_scissor_rect(region.x(), region.y(), region.width(), region.height());
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, camera, &[]);
        pass.set_bind_group(1, bind_group, &[]);
        pass.set_bind_group(2, scene, &[]);
        pass.draw(0..3, 0..1);
        bits
    }
}

/// Toward-light directions of the shadow-casting directional lights the mask covers
/// (w = 1 when traced), and their bits.
fn traced_lights(lights: &LightsData) -> ([[f32; 4]; RT_SHADOW_LIGHTS], u32) {
    let mut directions = [[0.0; 4]; RT_SHADOW_LIGHTS];
    let mut bits = 0u32;
    let shadows = lights.directional_shadows();
    for (index, light) in lights
        .directional_lights()
        .iter()
        .enumerate()
        .take(RT_SHADOW_LIGHTS)
    {
        let casts = shadows
            .get(index)
            .is_some_and(|shadow| shadow.params[0] > 0.5);
        let [x, y, z, _] = light.direction;
        let length = (x * x + y * y + z * z).sqrt();
        if !casts || length < 1e-6 {
            continue;
        }
        directions[index] = [-x / length, -y / length, -z / length, 1.0];
        bits |= 1 << index;
    }
    (directions, bits)
}

pub(crate) fn rt_shadows_shader_source() -> String {
    format!(
        "enable wgpu_ray_query;\n\n{}",
        include_str!("../../shader/rt_shadows.wgsl")
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_matches_the_shader_layout() {
        assert_eq!(std::mem::size_of::<RtShadowUniform>(), 96);
    }

    #[test]
    fn shader_validates() {
        let source = rt_shadows_shader_source();
        let module = naga::front::wgsl::parse_str(&source)
            .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::RAY_QUERY,
        )
        .validate(&module)
        .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
    }
}
