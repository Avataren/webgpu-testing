mod bloom;
mod effects;
mod pipelines;
mod resources;
mod ssao;

pub use effects::{BloomSettings, PostProcessEffects, SsaoSettings};

use bloom::{BloomBindGroupInputs, BloomStage, BloomTargets};
use pipelines::{
    build_color_grading, build_composite, build_depth_resolve, ColorGradingPipeline,
    CompositePipeline, DepthResolvePipeline,
};
use resources::{BloomMip, LazyPickTarget, MsaaTarget, TextureBundle};
use ssao::{SsaoBindGroupInputs, SsaoStage, SsaoTargets};

use crate::environment::ColorGrading;
use crate::renderer::{RenderRegion, ShaderBuilder};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3};

const NOISE_TEXTURE_SIZE: u32 = 4;
pub(crate) const BLOOM_MIP_COUNT: usize = 5;
pub(crate) const BLOOM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const GBUFFER_NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const GBUFFER_POSITION_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
pub const GBUFFER_PICK_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Uint;
const SSAO_NOISE_DATA: [f32; (NOISE_TEXTURE_SIZE * NOISE_TEXTURE_SIZE * 4) as usize] = [
    -0.6401949,
    -0.76821256,
    0.0,
    0.0,
    0.98767775,
    0.1565012,
    0.0,
    0.0,
    -0.1566164,
    0.9876595,
    0.0,
    0.0,
    0.1675282,
    0.98586726,
    0.0,
    0.0,
    -0.08490153,
    -0.9963893,
    0.0,
    0.0,
    -0.44445047,
    -0.89580345,
    0.0,
    0.0,
    0.77917,
    -0.62681264,
    0.0,
    0.0,
    0.85447717,
    0.519489,
    0.0,
    0.0,
    -0.88205993,
    0.471_137_3,
    0.0,
    0.0,
    0.98252517,
    0.18612963,
    0.0,
    0.0,
    0.19578062,
    0.98064774,
    0.0,
    0.0,
    -0.99943393,
    -0.03364192,
    0.0,
    0.0,
    0.9861326,
    0.165959,
    0.0,
    0.0,
    0.3159545,
    0.94877434,
    0.0,
    0.0,
    -0.5883725,
    -0.80859,
    0.0,
    0.0,
    -0.96039623,
    -0.278638,
    0.0,
    0.0,
];

pub struct PostProcess {
    scene_source: TextureBundle,
    scene: TextureBundle,
    scene_msaa: Option<MsaaTarget>,
    normal_source: TextureBundle,
    normal_msaa: Option<MsaaTarget>,
    position_source: TextureBundle,
    position_msaa: Option<MsaaTarget>,
    pick_target: LazyPickTarget,
    ssao: TextureBundle,
    ssao_ping: TextureBundle,
    bloom_down_chain: Vec<BloomMip>,
    bloom_up_chain: Vec<BloomMip>,
    sampler_linear: wgpu::Sampler,
    sampler_noise: wgpu::Sampler,
    _noise_texture: wgpu::Texture,
    noise_view: wgpu::TextureView,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    color_grading_pipeline: ColorGradingPipeline,
    depth_resolve_pipeline: Option<DepthResolvePipeline>,
    composite_pipeline: CompositePipeline,
    ssao_stage: SsaoStage,
    bloom_stage: BloomStage,
    size: wgpu::Extent3d,
    scene_format: wgpu::TextureFormat,
    effects: PostProcessEffects,
    depth_resolve_bind_group: Option<wgpu::BindGroup>,
    color_grading_bind_group: Option<wgpu::BindGroup>,
    composite_bind_group: Option<wgpu::BindGroup>,
    resolved_depth: Option<TextureBundle>,
    cached_depth_view: Option<wgpu::TextureView>,
    bind_groups_dirty: bool,
    last_proj: Mat4,
    last_view: Mat4,
    last_view_inv: Mat4,
    last_view_proj: Mat4,
    last_view_proj_inv: Mat4,
    last_camera_position: Vec3,
    last_near: f32,
    last_far: f32,
    viewport_resolution: Vec2,
    viewport_offset: Vec2,
    viewport_scale: Vec2,
    sample_count: u32,
    color_grading: ColorGrading,
}

pub struct PostProcessCamera {
    pub proj: Mat4,
    pub view_proj: Mat4,
    pub view_proj_inv: Mat4,
    pub view: Mat4,
    pub view_inv: Mat4,
    pub position: Vec3,
    pub near: f32,
    pub far: f32,
}

pub struct GBufferViews<'a> {
    pub normal: (&'a wgpu::TextureView, Option<&'a wgpu::TextureView>),
    pub position: (&'a wgpu::TextureView, Option<&'a wgpu::TextureView>),
    pub id: Option<PickAttachmentViews<'a>>,
}

#[derive(Clone, Copy)]
pub struct PickAttachmentViews<'a> {
    pub multisample: &'a wgpu::TextureView,
    pub resolve: Option<&'a wgpu::TextureView>,
}

impl<'a> PickAttachmentViews<'a> {
    pub fn single_sample(self) -> &'a wgpu::TextureView {
        self.resolve.unwrap_or(self.multisample)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct ColorAttachment<'a> {
    view: &'a wgpu::TextureView,
    resolve: Option<&'a wgpu::TextureView>,
}

impl<'a> ColorAttachment<'a> {
    fn single(view: &'a wgpu::TextureView) -> Self {
        Self {
            view,
            resolve: None,
        }
    }
}

pub(crate) fn begin_color_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &'static str,
    attachment: ColorAttachment<'a>,
    load: wgpu::LoadOp<wgpu::Color>,
    region: Option<RenderRegion>,
) -> wgpu::RenderPass<'a> {
    let color_attachment = wgpu::RenderPassColorAttachment {
        view: attachment.view,
        resolve_target: attachment.resolve,
        depth_slice: None,
        ops: wgpu::Operations {
            load,
            store: wgpu::StoreOp::Store,
        },
    };

    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(color_attachment)],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });

    if let Some(region) = region {
        region.apply_to_pass(&mut pass);
    }

    pass
}

fn begin_depth_pass<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &'static str,
    view: &'a wgpu::TextureView,
    load: wgpu::LoadOp<f32>,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[],
        depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
            view,
            depth_ops: Some(wgpu::Operations {
                load,
                store: wgpu::StoreOp::Store,
            }),
            stencil_ops: None,
        }),
        timestamp_writes: None,
        occlusion_query_set: None,
    })
}

pub(crate) fn fullscreen_vertex(shader: &wgpu::ShaderModule) -> wgpu::VertexState<'_> {
    wgpu::VertexState {
        module: shader,
        entry_point: Some("vs_fullscreen"),
        buffers: &[],
        compilation_options: Default::default(),
    }
}

impl PostProcess {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: &wgpu::SurfaceConfiguration,
        sample_count: u32,
    ) -> Self {
        let size = wgpu::Extent3d {
            width: config.width.max(1),
            height: config.height.max(1),
            depth_or_array_layers: 1,
        };
        let scene_format = config.format.remove_srgb_suffix();

        let sampler_linear = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("PostProcessLinearSampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let sampler_noise = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("PostProcessNoiseSampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let (scene_source, scene, scene_msaa) =
            create_scene_targets(device, &size, scene_format, sample_count);
        let (normal_source, normal_msaa) = create_gbuffer_target(
            device,
            &size,
            GBUFFER_NORMAL_FORMAT,
            sample_count,
            "SceneNormal",
        );
        let (position_source, position_msaa) = create_gbuffer_target(
            device,
            &size,
            GBUFFER_POSITION_FORMAT,
            sample_count,
            "ScenePosition",
        );
        let ssao = TextureBundle::ssao(device, &size);
        let ssao_ping = TextureBundle::ssao(device, &size);
        let (bloom_down_chain, bloom_up_chain) = create_bloom_chain(device, &size);

        let resolved_depth = if sample_count > 1 {
            Some(TextureBundle::depth(device, &size, "ResolvedDepth"))
        } else {
            None
        };

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("PostProcessUniformLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(
                        wgpu::BufferSize::new(std::mem::size_of::<PostProcessUniform>() as u64)
                            .expect("post process uniform must have non-zero size"),
                    ),
                },
                count: None,
            }],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("PostProcessUniformBuffer"),
            size: std::mem::size_of::<PostProcessUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("PostProcessUniformBindGroup"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let noise_texture = create_noise_texture(device, queue);
        let noise_view = noise_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let postprocess_source = ShaderBuilder::new()
            .with_module(include_str!("../../shader/postprocess/fullscreen.wgsl"))
            .with_module(include_str!("../../shader/postprocess/common.wgsl"))
            .with_module(include_str!("../../shader/postprocess/color_grading.wgsl"))
            .with_module(include_str!("../../shader/postprocess/ssao.wgsl"))
            .with_module(include_str!("../../shader/postprocess/ssao_blur.wgsl"))
            .with_module(include_str!("../../shader/postprocess/bloom_common.wgsl"))
            .with_module(include_str!(
                "../../shader/postprocess/bloom_prefilter.wgsl"
            ))
            .with_module(include_str!(
                "../../shader/postprocess/bloom_downsample.wgsl"
            ))
            .with_module(include_str!("../../shader/postprocess/bloom_upsample.wgsl"))
            .with_module(include_str!("../../shader/postprocess/composite.wgsl"))
            .build_modules_only();

        let postprocess_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("PostProcessShader"),
            source: wgpu::ShaderSource::Wgsl(postprocess_source.into()),
        });

        let color_grading_pipeline =
            build_color_grading(device, &uniform_layout, &postprocess_shader, scene_format);
        let depth_resolve_pipeline = build_depth_resolve(device, &uniform_layout, sample_count);
        let ssao_stage = SsaoStage::new(device, &uniform_layout, &postprocess_shader);
        let bloom_stage = BloomStage::new(device, &uniform_layout, &postprocess_shader);
        let composite_pipeline =
            build_composite(device, &uniform_layout, &postprocess_shader, config.format);

        let post = Self {
            scene_source,
            scene,
            scene_msaa,
            normal_source,
            normal_msaa,
            position_source,
            position_msaa,
            pick_target: LazyPickTarget::new(),
            ssao,
            ssao_ping,
            bloom_down_chain,
            bloom_up_chain,
            sampler_linear,
            sampler_noise,
            _noise_texture: noise_texture,
            noise_view,
            uniform_buffer,
            uniform_bind_group,
            color_grading_pipeline,
            depth_resolve_pipeline,
            composite_pipeline,
            ssao_stage,
            bloom_stage,
            size,
            scene_format,
            effects: PostProcessEffects::default(),
            depth_resolve_bind_group: None,
            color_grading_bind_group: None,
            composite_bind_group: None,
            resolved_depth,
            cached_depth_view: None,
            bind_groups_dirty: true,
            last_proj: Mat4::IDENTITY,
            last_view: Mat4::IDENTITY,
            last_view_inv: Mat4::IDENTITY,
            last_view_proj: Mat4::IDENTITY,
            last_view_proj_inv: Mat4::IDENTITY,
            last_camera_position: Vec3::ZERO,
            last_near: 0.01,
            last_far: 100.0,
            viewport_resolution: Vec2::new(size.width as f32, size.height as f32),
            viewport_offset: Vec2::ZERO,
            viewport_scale: Vec2::ONE,
            sample_count,
            color_grading: ColorGrading::default(),
        };

        let initial_uniform = PostProcessUniform::new(
            post.last_proj,
            post.last_proj.inverse(),
            post.last_view,
            post.last_view_inv,
            post.last_view_proj,
            post.last_view_proj_inv,
            post.last_camera_position,
            post.viewport_resolution,
            post.viewport_offset,
            post.viewport_scale,
            post.last_near,
            post.last_far,
            post.effects,
            post.color_grading,
        );
        queue.write_buffer(
            &post.uniform_buffer,
            0,
            bytemuck::bytes_of(&initial_uniform),
        );

        post
    }

    pub fn resize(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let (scene_source, scene, scene_msaa) =
            create_scene_targets(device, &self.size, self.scene_format, self.sample_count);
        self.scene_source = scene_source;
        self.scene = scene;
        self.scene_msaa = scene_msaa;
        let (normal_source, normal_msaa) = create_gbuffer_target(
            device,
            &self.size,
            GBUFFER_NORMAL_FORMAT,
            self.sample_count,
            "SceneNormal",
        );
        self.normal_source = normal_source;
        self.normal_msaa = normal_msaa;
        let (position_source, position_msaa) = create_gbuffer_target(
            device,
            &self.size,
            GBUFFER_POSITION_FORMAT,
            self.sample_count,
            "ScenePosition",
        );
        self.position_source = position_source;
        self.position_msaa = position_msaa;
        self.pick_target.invalidate();
        self.ssao = TextureBundle::ssao(device, &self.size);
        self.ssao_ping = TextureBundle::ssao(device, &self.size);
        self.resolved_depth = if self.sample_count > 1 {
            Some(TextureBundle::depth(device, &self.size, "ResolvedDepth"))
        } else {
            None
        };
        let (down_chain, up_chain) = create_bloom_chain(device, &self.size);
        self.bloom_down_chain = down_chain;
        self.bloom_up_chain = up_chain;
        self.viewport_resolution = Vec2::new(width as f32, height as f32);
        self.viewport_offset = Vec2::ZERO;
        self.viewport_scale = Vec2::ONE;
        self.mark_bind_groups_dirty();
        self.upload_uniform(queue);
    }

    pub fn update_viewport(&mut self, queue: &wgpu::Queue, region: Option<RenderRegion>) {
        let full_width = self.size.width.max(1) as f32;
        let full_height = self.size.height.max(1) as f32;

        let (resolution, offset, scale) = if let Some(region) = region {
            let region_width = region.width().max(1) as f32;
            let region_height = region.height().max(1) as f32;
            let offset = Vec2::new(
                region.x() as f32 / full_width,
                region.y() as f32 / full_height,
            );
            let scale = Vec2::new(region_width / full_width, region_height / full_height);
            (Vec2::new(region_width, region_height), offset, scale)
        } else {
            (Vec2::new(full_width, full_height), Vec2::ZERO, Vec2::ONE)
        };

        if self.viewport_resolution != resolution
            || self.viewport_offset != offset
            || self.viewport_scale != scale
        {
            self.viewport_resolution = resolution;
            self.viewport_offset = offset;
            self.viewport_scale = scale;
            self.upload_uniform(queue);
        }
    }

    pub fn update_camera(&mut self, queue: &wgpu::Queue, camera: PostProcessCamera) {
        self.last_proj = camera.proj;
        self.last_view = camera.view;
        self.last_view_inv = camera.view_inv;
        self.last_view_proj = camera.view_proj;
        self.last_view_proj_inv = camera.view_proj_inv;
        self.last_camera_position = camera.position;
        self.last_near = camera.near;
        self.last_far = camera.far;
        self.upload_uniform(queue);
    }

    pub fn scene_color_views(&self) -> (&wgpu::TextureView, Option<&wgpu::TextureView>) {
        match self.scene_msaa.as_ref() {
            Some(msaa) => (&msaa.view, Some(&self.scene_source.view)),
            None => (&self.scene_source.view, None),
        }
    }

    pub fn gbuffer_views(&self) -> GBufferViews<'_> {
        let normal = match self.normal_msaa.as_ref() {
            Some(msaa) => (&msaa.view, Some(&self.normal_source.view)),
            None => (&self.normal_source.view, None),
        };
        let position = match self.position_msaa.as_ref() {
            Some(msaa) => (&msaa.view, Some(&self.position_source.view)),
            None => (&self.position_source.view, None),
        };
        let id = self.pick_target.views();
        GBufferViews {
            normal,
            position,
            id,
        }
    }

    pub fn scene_view(&self) -> &wgpu::TextureView {
        &self.scene.view
    }

    pub fn ssao_texture(&self) -> &wgpu::TextureView {
        &self.ssao.view
    }

    pub fn bloom_texture(&self) -> &wgpu::TextureView {
        &self.bloom_up_chain[0].view
    }

    pub fn set_depth_view(&mut self, depth_view: &wgpu::TextureView) {
        self.cached_depth_view = Some(depth_view.clone());
        self.mark_bind_groups_dirty();
    }

    pub fn after_postprocess_depth_view(&self) -> Option<&wgpu::TextureView> {
        if let Some(resolved) = self.resolved_depth.as_ref() {
            Some(&resolved.view)
        } else {
            self.cached_depth_view.as_ref()
        }
    }

    pub fn set_effects(&mut self, queue: &wgpu::Queue, effects: PostProcessEffects) {
        if self.effects != effects {
            self.effects = effects;
            self.upload_uniform(queue);
        }
    }

    pub fn effects(&self) -> PostProcessEffects {
        self.effects
    }

    pub fn ensure_pick_attachment(&mut self, device: &wgpu::Device) {
        self.pick_target
            .ensure(device, &self.size, self.sample_count);
    }

    pub fn pick_attachment_views(&self) -> Option<PickAttachmentViews<'_>> {
        self.pick_target.views()
    }

    pub fn pick_texture(&self) -> Option<&wgpu::Texture> {
        self.pick_target.texture()
    }

    pub fn pick_texture_extent(&self) -> Option<wgpu::Extent3d> {
        self.pick_target.extent()
    }

    pub fn set_color_grading(&mut self, queue: &wgpu::Queue, grading: ColorGrading) {
        if self.color_grading != grading {
            self.color_grading = grading;
            self.upload_uniform(queue);
        }
    }

    pub fn execute(
        &mut self,
        encoder: &mut wgpu::CommandEncoder,
        device: &wgpu::Device,
        target: &wgpu::TextureView,
        region: Option<RenderRegion>,
    ) {
        self.ensure_cached_bind_groups(device);

        if let Some(color_group) = self.color_grading_bind_group.as_ref() {
            let mut pass = begin_color_pass(
                encoder,
                "ColorGradingPass",
                ColorAttachment::single(&self.scene.view),
                wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                None,
            );
            pass.set_pipeline(&self.color_grading_pipeline.pipeline);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            pass.set_bind_group(1, color_group, &[]);
            pass.draw(0..3, 0..1);
        }

        if let (Some(depth_pipeline), Some(bind_group), Some(resolved)) = (
            self.depth_resolve_pipeline.as_ref(),
            self.depth_resolve_bind_group.as_ref(),
            self.resolved_depth.as_ref(),
        ) {
            let mut pass = begin_depth_pass(
                encoder,
                "DepthResolvePass",
                &resolved.view,
                wgpu::LoadOp::Clear(1.0),
            );
            pass.set_pipeline(&depth_pipeline.pipeline);
            pass.set_bind_group(0, &self.uniform_bind_group, &[]);
            pass.set_bind_group(1, bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        let ssao_targets = SsaoTargets {
            output: &self.ssao.view,
            ping: &self.ssao_ping.view,
        };
        if self.effects.ssao {
            self.ssao_stage
                .render(encoder, &self.uniform_bind_group, ssao_targets);
        } else {
            self.ssao_stage.clear(encoder, ssao_targets);
        }

        let bloom_targets = BloomTargets {
            down_chain: &self.bloom_down_chain,
            up_chain: &self.bloom_up_chain,
        };
        if self.effects.bloom {
            self.bloom_stage
                .render(encoder, &self.uniform_bind_group, bloom_targets);
        } else {
            self.bloom_stage.clear(encoder, bloom_targets);
        }

        let composite_bind_group = self
            .composite_bind_group
            .as_ref()
            .expect("Composite bind group not initialized");

        let mut pass = begin_color_pass(
            encoder,
            "CompositePass",
            ColorAttachment::single(target),
            wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            region,
        );
        pass.set_pipeline(&self.composite_pipeline.pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_bind_group(1, composite_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    fn mark_bind_groups_dirty(&mut self) {
        self.depth_resolve_bind_group = None;
        self.color_grading_bind_group = None;
        self.composite_bind_group = None;
        self.ssao_stage.invalidate();
        self.bloom_stage.invalidate();
        self.bind_groups_dirty = true;
    }

    fn ensure_cached_bind_groups(&mut self, device: &wgpu::Device) {
        if !self.bind_groups_dirty {
            return;
        }

        let depth_view = self
            .cached_depth_view
            .as_ref()
            .expect("Depth view must be set before executing post process");

        if let (Some(depth_pipeline), Some(resolved)) = (
            self.depth_resolve_pipeline.as_ref(),
            self.resolved_depth.as_ref(),
        ) {
            self.depth_resolve_bind_group =
                Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("DepthResolveBindGroup"),
                    layout: &depth_pipeline.layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(depth_view),
                    }],
                }));
            let ssao_inputs = SsaoBindGroupInputs {
                depth_view: &resolved.view,
                noise_view: &self.noise_view,
                sampler_noise: &self.sampler_noise,
                sampler_linear: &self.sampler_linear,
                normal_view: &self.normal_source.view,
                position_view: &self.position_source.view,
                targets: SsaoTargets {
                    output: &self.ssao.view,
                    ping: &self.ssao_ping.view,
                },
            };
            self.ssao_stage.ensure_bind_groups(device, ssao_inputs);
        } else {
            self.depth_resolve_bind_group = None;
            let ssao_inputs = SsaoBindGroupInputs {
                depth_view,
                noise_view: &self.noise_view,
                sampler_noise: &self.sampler_noise,
                sampler_linear: &self.sampler_linear,
                normal_view: &self.normal_source.view,
                position_view: &self.position_source.view,
                targets: SsaoTargets {
                    output: &self.ssao.view,
                    ping: &self.ssao_ping.view,
                },
            };
            self.ssao_stage.ensure_bind_groups(device, ssao_inputs);
        }

        let bloom_inputs = BloomBindGroupInputs {
            scene_view: &self.scene.view,
            sampler_linear: &self.sampler_linear,
            targets: BloomTargets {
                down_chain: &self.bloom_down_chain,
                up_chain: &self.bloom_up_chain,
            },
        };
        self.bloom_stage.ensure_bind_groups(device, bloom_inputs);

        self.color_grading_bind_group =
            Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ColorGradingBindGroup"),
                layout: &self.color_grading_pipeline.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 10,
                        resource: wgpu::BindingResource::TextureView(&self.scene_source.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 11,
                        resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                    },
                ],
            }));

        let bloom_targets = BloomTargets {
            down_chain: &self.bloom_down_chain,
            up_chain: &self.bloom_up_chain,
        };
        let composite_depth_view: &wgpu::TextureView =
            if let Some(resolved) = self.resolved_depth.as_ref() {
                &resolved.view
            } else {
                depth_view
            };

        self.composite_bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("CompositeBindGroup"),
            layout: &self.composite_pipeline.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(composite_depth_view),
                },
                wgpu::BindGroupEntry {
                    binding: 50,
                    resource: wgpu::BindingResource::TextureView(&self.scene.view),
                },
                wgpu::BindGroupEntry {
                    binding: 51,
                    resource: wgpu::BindingResource::TextureView(&self.ssao.view),
                },
                wgpu::BindGroupEntry {
                    binding: 52,
                    resource: wgpu::BindingResource::TextureView(&bloom_targets.up_chain[0].view),
                },
                wgpu::BindGroupEntry {
                    binding: 53,
                    resource: wgpu::BindingResource::Sampler(&self.sampler_linear),
                },
            ],
        }));

        self.bind_groups_dirty = false;
    }

    fn upload_uniform(&self, queue: &wgpu::Queue) {
        let proj_inv = self.last_proj.inverse();
        let uniform = PostProcessUniform::new(
            self.last_proj,
            proj_inv,
            self.last_view,
            self.last_view_inv,
            self.last_view_proj,
            self.last_view_proj_inv,
            self.last_camera_position,
            self.viewport_resolution,
            self.viewport_offset,
            self.viewport_scale,
            self.last_near,
            self.last_far,
            self.effects,
            self.color_grading,
        );
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));
    }
}

fn create_scene_targets(
    device: &wgpu::Device,
    size: &wgpu::Extent3d,
    format: wgpu::TextureFormat,
    sample_count: u32,
) -> (TextureBundle, TextureBundle, Option<MsaaTarget>) {
    let source = TextureBundle::color(device, size, format, "SceneColorSource");
    let target = TextureBundle::color(device, size, format, "SceneColor");
    let msaa = if sample_count > 1 {
        Some(MsaaTarget::new(
            device,
            size,
            format,
            sample_count,
            "SceneColorSourceMsaa",
        ))
    } else {
        None
    };

    (source, target, msaa)
}

fn create_gbuffer_target(
    device: &wgpu::Device,
    size: &wgpu::Extent3d,
    format: wgpu::TextureFormat,
    sample_count: u32,
    label: &str,
) -> (TextureBundle, Option<MsaaTarget>) {
    let source = TextureBundle::color(device, size, format, &format!("{label}Source"));
    let msaa = if sample_count > 1 {
        Some(MsaaTarget::new(
            device,
            size,
            format,
            sample_count,
            &format!("{label}Msaa"),
        ))
    } else {
        None
    };
    (source, msaa)
}

fn create_bloom_chain(
    device: &wgpu::Device,
    size: &wgpu::Extent3d,
) -> (Vec<BloomMip>, Vec<BloomMip>) {
    let mut down_chain = Vec::with_capacity(BLOOM_MIP_COUNT);
    let mut up_chain = Vec::with_capacity(BLOOM_MIP_COUNT);
    let mut width = (size.width.max(2) / 2).max(1);
    let mut height = (size.height.max(2) / 2).max(1);

    for level in 0..BLOOM_MIP_COUNT {
        let mip_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        down_chain.push(BloomMip::new(
            device,
            mip_size,
            &format!("BloomDown{level}"),
        ));
        up_chain.push(BloomMip::new(device, mip_size, &format!("BloomUp{level}")));
        width = (width / 2).max(1);
        height = (height / 2).max(1);
    }

    (down_chain, up_chain)
}

fn create_noise_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
    let data_bytes = bytemuck::cast_slice(&SSAO_NOISE_DATA);

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("SsaoNoiseTexture"),
        size: wgpu::Extent3d {
            width: NOISE_TEXTURE_SIZE,
            height: NOISE_TEXTURE_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data_bytes,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some((4 * std::mem::size_of::<f32>()) as u32 * NOISE_TEXTURE_SIZE),
            rows_per_image: Some(NOISE_TEXTURE_SIZE),
        },
        wgpu::Extent3d {
            width: NOISE_TEXTURE_SIZE,
            height: NOISE_TEXTURE_SIZE,
            depth_or_array_layers: 1,
        },
    );

    texture
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostProcessUniform {
    view_proj: [[f32; 4]; 4],
    view_proj_inv: [[f32; 4]; 4],
    proj: [[f32; 4]; 4],
    proj_inv: [[f32; 4]; 4],
    view: [[f32; 4]; 4],
    view_inv: [[f32; 4]; 4],
    camera_position: [f32; 4],
    viewport_resolution: [f32; 2],
    viewport_offset: [f32; 2],
    viewport_scale: [f32; 2],
    radius_bias: [f32; 2],
    intensity_power: [f32; 2],
    noise_scale: [f32; 2],
    near_far: [f32; 2],
    _padding0: [f32; 2],
    color_adjust: [f32; 4],
    bloom_params: [f32; 4],
    effects: [f32; 4],
}

impl PostProcessUniform {
    #[allow(clippy::too_many_arguments)]
    fn new(
        proj: Mat4,
        proj_inv: Mat4,
        view: Mat4,
        view_inv: Mat4,
        view_proj: Mat4,
        view_proj_inv: Mat4,
        camera_position: Vec3,
        viewport_resolution: Vec2,
        viewport_offset: Vec2,
        viewport_scale: Vec2,
        near: f32,
        far: f32,
        effects: PostProcessEffects,
        grading: ColorGrading,
    ) -> Self {
        let noise_scale = Vec2::new(
            viewport_resolution.x / NOISE_TEXTURE_SIZE as f32,
            viewport_resolution.y / NOISE_TEXTURE_SIZE as f32,
        );
        let ssao = effects.ssao_settings;
        let bloom = effects.bloom_settings;
        let effects_arr = effects.uniform_components();
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            view_proj_inv: view_proj_inv.to_cols_array_2d(),
            proj: proj.to_cols_array_2d(),
            proj_inv: proj_inv.to_cols_array_2d(),
            view: view.to_cols_array_2d(),
            view_inv: view_inv.to_cols_array_2d(),
            camera_position: [camera_position.x, camera_position.y, camera_position.z, 1.0],
            viewport_resolution: [viewport_resolution.x, viewport_resolution.y],
            viewport_offset: [viewport_offset.x, viewport_offset.y],
            viewport_scale: [viewport_scale.x, viewport_scale.y],
            radius_bias: [ssao.radius, ssao.bias],
            intensity_power: [ssao.intensity, ssao.power],
            noise_scale: [noise_scale.x, noise_scale.y],
            near_far: [near, far],
            _padding0: [0.0, 0.0],
            color_adjust: [
                grading.exposure(),
                grading.saturation(),
                grading.contrast(),
                0.0,
            ],
            bloom_params: [bloom.threshold, bloom.knee, bloom.scatter, 0.0],
            effects: effects_arr,
        }
    }
}
