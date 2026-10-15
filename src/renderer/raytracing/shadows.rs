//! Ray-traced shadows: a screen-space mask traced between the depth prepass and the main
//! pass (`shader/rt_shadows.wgsl`), one bit per shadowed light, which the main pass reads
//! instead of the shadow maps for those lights.

use crate::renderer::shader_source::wgsl;
use bytemuck::{Pod, Zeroable};

use crate::renderer::lights::LightsData;
use crate::renderer::{PipelineBuilder, RenderRegion, ShaderBuilder};

/// Lights of each kind the mask has bits for: bits 0-3 directional, 4-7 point, 8-11 spot.
pub(crate) const RT_SHADOW_LIGHTS: usize = 4;
const POINT_BIT: usize = 4;
const SPOT_BIT: usize = 8;
/// One bit per light: a ray traced this frame either reaches the light or not.
pub(crate) const MASK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;

/// Mirrors `RtShadowParams` in rt_shadows.wgsl.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
struct RtShadowUniform {
    directionals: [[f32; 4]; RT_SHADOW_LIGHTS],
    points: [[f32; 4]; RT_SHADOW_LIGHTS],
    spots: [[f32; 4]; RT_SHADOW_LIGHTS],
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
    /// Shadow rays alpha-test cut-out surfaces with the bindless textures (group 3).
    textures: bool,
}

impl RtShadowStage {
    /// Build the mask pipeline; fails (instead of raising an uncaptured device error) when
    /// the driver or wgpu rejects it, so the caller keeps shadow maps.
    pub(crate) fn new(
        device: &wgpu::Device,
        camera_layout: &wgpu::BindGroupLayout,
        scene_layout: &wgpu::BindGroupLayout,
        textures_layout: Option<&wgpu::BindGroupLayout>,
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
            source: wgpu::ShaderSource::Wgsl(
                rt_shadows_shader_source(textures_layout.is_some()).into(),
            ),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("RtShadowPipelineLayout"),
            bind_group_layouts: &[
                Some(camera_layout),
                Some(&layout),
                Some(scene_layout),
                textures_layout,
            ],
            immediate_size: 0,
        });
        let pipeline = PipelineBuilder::new(device, &pipeline_layout, &shader)
            .with_label("RtShadowPipeline")
            .with_vertex_entry("vs_fullscreen")
            .with_fragment_entry("fs_rt_shadows")
            .with_color_target(MASK_FORMAT, None)
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
            textures: textures_layout.is_some(),
        })
    }

    /// The mask texture, for reading back.
    pub(crate) fn mask_texture(&self) -> Option<&wgpu::Texture> {
        self.mask.as_ref().map(|(texture, _)| texture)
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
                    | wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_SRC,
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

    /// The lights [`Self::trace`] would trace for `lights`, as its bit set.
    pub(crate) fn traced_bits(&self, lights: &LightsData) -> u32 {
        traced_lights(lights).1
    }

    /// Trace the mask for `lights`' shadow-casting lights. Returns the lights traced as a
    /// bit set (see [`RT_SHADOW_LIGHTS`]), or 0 when nothing ran.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn trace(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        camera: &wgpu::BindGroup,
        scene: &wgpu::BindGroup,
        textures: Option<&wgpu::BindGroup>,
        lights: &LightsData,
        softness_degrees: f32,
        light_radius: f32,
        frame: u32,
        region: Option<RenderRegion>,
    ) -> u32 {
        let (Some(bind_group), Some((texture, view))) = (self.bind_group.as_ref(), &self.mask)
        else {
            return 0;
        };
        let (mut uniform, bits) = traced_lights(lights);
        let textures = match (self.textures, textures) {
            (true, None) => return 0,
            (true, textures) => textures,
            (false, _) => None,
        };
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
        uniform.params = [
            (0.5 * softness_degrees).to_radians().tan(),
            (frame % 1024) as f32,
            light_radius,
            0.0,
        ];
        uniform.region = region_rect;
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
                    // Every light reaches pixels outside the traced region.
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: u32::MAX as f64,
                        g: 0.0,
                        b: 0.0,
                        a: 0.0,
                    }),
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
        if let Some(textures) = textures {
            pass.set_bind_group(3, textures, &[]);
        }
        pass.draw(0..3, 0..1);
        bits
    }
}

/// The shadow-casting lights the mask covers (w = 1 when traced: toward-light directions
/// for directional lights, positions for point and spot lights) and their bits.
fn traced_lights(lights: &LightsData) -> (RtShadowUniform, u32) {
    let mut uniform = RtShadowUniform::default();
    let mut bits = 0u32;
    let directional_shadows = lights.directional_shadows();
    for (index, light) in lights
        .directional_lights()
        .iter()
        .enumerate()
        .take(RT_SHADOW_LIGHTS)
    {
        let casts = directional_shadows
            .get(index)
            .is_some_and(|shadow| shadow.params[0] > 0.5);
        let [x, y, z, _] = light.direction;
        let length = (x * x + y * y + z * z).sqrt();
        if casts && length > 1e-6 {
            uniform.directionals[index] = [-x / length, -y / length, -z / length, 1.0];
            bits |= 1 << index;
        }
    }
    let point_shadows = lights.point_shadows();
    for (index, light) in lights
        .point_lights()
        .iter()
        .enumerate()
        .take(RT_SHADOW_LIGHTS)
    {
        if point_shadows
            .get(index)
            .is_some_and(|shadow| shadow.params[0] > 0.5)
        {
            let [x, y, z, _] = light.position_range;
            uniform.points[index] = [x, y, z, 1.0];
            bits |= 1 << (POINT_BIT + index);
        }
    }
    let spot_shadows = lights.spot_shadows();
    for (index, light) in lights
        .spot_lights()
        .iter()
        .enumerate()
        .take(RT_SHADOW_LIGHTS)
    {
        if spot_shadows
            .get(index)
            .is_some_and(|shadow| shadow.params[0] > 0.5)
        {
            let [x, y, z, _] = light.position_range;
            uniform.spots[index] = [x, y, z, 1.0];
            bits |= 1 << (SPOT_BIT + index);
        }
    }
    (uniform, bits)
}

/// The mask shader: the scene as group 2 and, with `textures`, the bindless material
/// textures as group 3 (for cut-out surfaces).
pub(crate) fn rt_shadows_shader_source(textures: bool) -> String {
    let main = format!(
        "{}\n\n{}",
        super::scene_shader_modules(2, textures.then_some(3)),
        wgsl!("rt_shadows.wgsl")
    );
    let source = ShaderBuilder::new()
        .with_camera()
        .with_material_system()
        .build(&main);
    format!("enable wgpu_ray_query;\n\n{source}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_matches_the_shader_layout() {
        assert_eq!(std::mem::size_of::<RtShadowUniform>(), 224);
    }

    #[test]
    fn shader_validates() {
        for textures in [false, true] {
            let source = rt_shadows_shader_source(textures);
            let module = naga::front::wgsl::parse_str(&source)
                .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
            naga::valid::Validator::new(
                naga::valid::ValidationFlags::all(),
                naga::valid::Capabilities::all(),
            )
            .validate(&module)
            .unwrap_or_else(|err| panic!("{}", err.emit_to_string(&source)));
        }
    }
}
