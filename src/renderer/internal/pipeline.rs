use std::collections::HashMap;
use std::num::NonZeroU32;

use crate::asset::Assets;
use crate::renderer::internal::{CameraBuffer, DynamicObjectsBuffer, LightsBuffer, RenderContext};
use crate::renderer::material::MaterialFlags;
use crate::renderer::{Material, PipelineBuilder, Vertex};

const MAX_TEXTURES: usize = 256;

pub(crate) struct RenderPipeline {
    pipelines: HashMap<PipelineKey, wgpu::RenderPipeline>,
    depth_prepass: wgpu::RenderPipeline,
    background: wgpu::RenderPipeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PipelineKey {
    depth_test: bool,
    depth_write: bool,
    alpha_blend: bool,
    sample_count: u32,
}

impl PipelineKey {
    pub(crate) fn new(
        depth_test: bool,
        depth_write: bool,
        alpha_blend: bool,
        sample_count: u32,
    ) -> Self {
        Self {
            depth_test,
            depth_write,
            alpha_blend,
            sample_count,
        }
    }
}

pub(crate) enum TextureBindingModel {
    Bindless(BindlessTextureBinder),
    Classic(TraditionalTextureBinder),
}

impl RenderPipeline {
    pub(crate) fn new(
        context: &RenderContext,
        camera: &CameraBuffer,
        objects: &DynamicObjectsBuffer,
        lights: &LightsBuffer,
        sample_count: u32,
    ) -> (Self, TextureBindingModel) {
        let (texture_bind_layout, texture_binder, shader_source) = if context
            .supports_bindless_textures
        {
            let layout =
                context
                    .device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("TextureArrayBindGroupLayout"),
                        entries: &[
                            wgpu::BindGroupLayoutEntry {
                                binding: 0,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                    multisampled: false,
                                },
                                count: NonZeroU32::new(MAX_TEXTURES as u32),
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 1,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 2,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Sampler(
                                    wgpu::SamplerBindingType::NonFiltering,
                                ),
                                count: None,
                            },
                        ],
                    });

            let binder =
                TextureBindingModel::Bindless(BindlessTextureBinder::new(&context.device, &layout));
            (layout, binder, Self::shader_source(true))
        } else {
            let layout =
                context
                    .device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("TextureBindGroupLayout"),
                        entries: &[
                            wgpu::BindGroupLayoutEntry {
                                binding: 0,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                    multisampled: false,
                                },
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 1,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 2,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Sampler(
                                    wgpu::SamplerBindingType::NonFiltering,
                                ),
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 3,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                    multisampled: false,
                                },
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 4,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                    multisampled: false,
                                },
                                count: None,
                            },
                        ],
                    });

            let binder = TextureBindingModel::Classic(TraditionalTextureBinder::new(
                &context.device,
                &layout,
            ));
            (layout, binder, Self::shader_source(false))
        };

        let shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("RendererShader"),
                source: wgpu::ShaderSource::Wgsl(shader_source.into()),
            });

        let pipeline_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("PipelineLayout"),
                    bind_group_layouts: &[
                        &camera.bind_layout,
                        &objects.bind_layout,
                        &lights.bind_layout,
                        &texture_bind_layout,
                    ],
                    push_constant_ranges: &[],
                });

        let depth_pipeline_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("DepthPipelineLayout"),
                    bind_group_layouts: &[&camera.bind_layout, &objects.bind_layout],
                    push_constant_ranges: &[],
                });

        let depth_shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("DepthShader"),
                source: wgpu::ShaderSource::Wgsl(
                    include_str!("../../shader/depth_prepass.wgsl").into(),
                ),
            });

        let background_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("EnvironmentBackgroundPipelineLayout"),
                    bind_group_layouts: &[&camera.bind_layout, &lights.bind_layout],
                    push_constant_ranges: &[],
                });

        let shader_source = format!(
            "{}\n{}",
            include_str!("../../shader/constants.wgsl"),
            include_str!("../../shader/environment_background.wgsl")
        );

        let background_shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("EnvironmentBackgroundShader"),
                source: wgpu::ShaderSource::Wgsl(shader_source.into()),
            });

        let background_pipeline =
            PipelineBuilder::new(&context.device, &background_layout, &background_shader)
                .with_label("EnvironmentBackgroundPipeline")
                .with_color_target(context.config.format, Some(wgpu::BlendState::REPLACE))
                .with_depth_stencil(
                    context.depth.format,
                    false, // depth_write
                    wgpu::CompareFunction::Always,
                )
                .with_no_culling()
                .with_multisample(sample_count)
                .build();

        let mut pipelines = HashMap::new();
        for &depth_test in &[false, true] {
            for &depth_write in &[false, true] {
                for &alpha_blend in &[false, true] {
                    let key = PipelineKey {
                        depth_test,
                        depth_write,
                        alpha_blend,
                        sample_count,
                    };
                    let pipeline = Self::create_pipeline(
                        context,
                        &pipeline_layout,
                        &shader,
                        depth_test,
                        depth_write,
                        alpha_blend,
                        sample_count,
                    );
                    pipelines.insert(key, pipeline);
                }
            }
        }

        let depth_prepass = Self::create_depth_prepass_pipeline(
            context,
            &depth_pipeline_layout,
            &depth_shader,
            sample_count,
        );

        (
            Self {
                pipelines,
                depth_prepass,
                background: background_pipeline,
            },
            texture_binder,
        )
    }

    fn shader_source(bindless: bool) -> String {
        let constants = include_str!("../../shader/constants.wgsl");
        let bindings = if bindless {
            include_str!("../../shader/bindings_bindless.wgsl")
        } else {
            include_str!("../../shader/bindings_traditional.wgsl")
        };

        // Include shared PBR lighting module before common.wgsl
        format!(
            "{}\n{}\n{}\n{}",
            constants,
            bindings,
            include_str!("../../shader/pbr_lighting.wgsl"),
            include_str!("../../shader/common.wgsl")
        )
    }

    fn create_pipeline(
        context: &RenderContext,
        pipeline_layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        depth_test: bool,
        depth_write: bool,
        alpha_blend: bool,
        sample_count: u32,
    ) -> wgpu::RenderPipeline {
        let depth_compare = if depth_test {
            wgpu::CompareFunction::LessEqual
        } else {
            wgpu::CompareFunction::Always
        };

        let blend_state = if alpha_blend {
            Some(wgpu::BlendState::ALPHA_BLENDING)
        } else {
            Some(wgpu::BlendState::REPLACE)
        };

        let mut builder = PipelineBuilder::new(&context.device, pipeline_layout, shader)
            .with_label("MainRenderPipeline")
            .with_vertex_buffer(Vertex::layout())
            .with_color_target(context.config.format, blend_state)
            .with_multisample(sample_count);

        if depth_test || depth_write {
            builder = builder.with_depth_stencil(context.depth.format, depth_write, depth_compare);
        }

        builder.build()
    }

    pub(crate) fn pipeline(&self, key: PipelineKey) -> &wgpu::RenderPipeline {
        self.pipelines.get(&key).expect("missing pipeline variant")
    }

    fn create_depth_prepass_pipeline(
        context: &RenderContext,
        pipeline_layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        sample_count: u32,
    ) -> wgpu::RenderPipeline {
        PipelineBuilder::new(&context.device, pipeline_layout, shader)
            .with_label("DepthPrepassPipeline")
            .depth_only()
            .with_vertex_buffer(Vertex::layout())
            .with_depth_stencil(context.depth.format, true, wgpu::CompareFunction::LessEqual)
            .with_multisample(sample_count)
            .build()
    }

    pub(crate) fn depth_prepass(&self) -> &wgpu::RenderPipeline {
        &self.depth_prepass
    }

    pub(crate) fn background(&self) -> &wgpu::RenderPipeline {
        &self.background
    }
}

pub(crate) struct BindlessTextureBinder {
    pub(crate) layout: wgpu::BindGroupLayout,
    linear_sampler: wgpu::Sampler,
    nearest_sampler: wgpu::Sampler,
    _fallback_texture: wgpu::Texture,
    fallback_view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

impl BindlessTextureBinder {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("BindlessSamplerLinear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("BindlessSamplerNearest"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let fallback_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("BindlessFallbackTexture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let fallback_view = fallback_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group = Self::create_bind_group_with_views(
            device,
            layout,
            &linear_sampler,
            &nearest_sampler,
            vec![&fallback_view; MAX_TEXTURES],
        );

        Self {
            layout: layout.clone(),
            linear_sampler,
            nearest_sampler,
            _fallback_texture: fallback_texture,
            fallback_view,
            bind_group,
        }
    }

    fn create_bind_group_with_views(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        linear_sampler: &wgpu::Sampler,
        nearest_sampler: &wgpu::Sampler,
        views: Vec<&wgpu::TextureView>,
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("BindlessTextureBindGroup"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureViewArray(&views),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(linear_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(nearest_sampler),
                },
            ],
        })
    }

    fn update(&mut self, device: &wgpu::Device, assets: &Assets) {
        let fallback = &self.fallback_view;
        let views: Vec<&wgpu::TextureView> = (0..MAX_TEXTURES)
            .map(|i| {
                assets
                    .textures
                    .get(crate::asset::Handle::new(i))
                    .map(|t| &t.view)
                    .unwrap_or(fallback)
            })
            .collect();

        self.bind_group = Self::create_bind_group_with_views(
            device,
            &self.layout,
            &self.linear_sampler,
            &self.nearest_sampler,
            views,
        );

        log::debug!(
            "Updated bindless texture array with {} textures",
            assets.textures.len()
        );
    }

    fn global_bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }
}

pub(crate) struct TraditionalTextureBinder {
    pub(crate) layout: wgpu::BindGroupLayout,
    linear_sampler: wgpu::Sampler,
    nearest_sampler: wgpu::Sampler,
    _fallback_texture: wgpu::Texture,
    fallback_view: wgpu::TextureView,
    material_bind_groups: HashMap<Material, wgpu::BindGroup>,
}

impl TraditionalTextureBinder {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("TraditionalSamplerLinear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let nearest_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("TraditionalSamplerNearest"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let fallback_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("TraditionalFallbackTexture"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let fallback_view = fallback_texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            layout: layout.clone(),
            linear_sampler,
            nearest_sampler,
            _fallback_texture: fallback_texture,
            fallback_view,
            material_bind_groups: HashMap::new(),
        }
    }

    fn create_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        linear_sampler: &wgpu::Sampler,
        nearest_sampler: &wgpu::Sampler,
        views: [&wgpu::TextureView; 5],
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("MaterialTextureBindGroup"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(views[3]),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(views[4]),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(linear_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(nearest_sampler),
                },
            ],
        })
    }

    fn view_or_fallback<'a>(
        assets: &'a Assets,
        fallback: &'a wgpu::TextureView,
        index: u32,
    ) -> &'a wgpu::TextureView {
        assets
            .textures
            .get(crate::asset::Handle::new(index as usize))
            .map(|t| &t.view)
            .unwrap_or(fallback)
    }

    fn update(&mut self, _device: &wgpu::Device, _assets: &Assets) {
        self.material_bind_groups.clear();
    }

    fn bind_group_for_material(
        &mut self,
        device: &wgpu::Device,
        assets: &Assets,
        material: Material,
    ) -> &wgpu::BindGroup {
        let layout = self.layout.clone();
        let linear_sampler = self.linear_sampler.clone();
        let nearest_sampler = self.nearest_sampler.clone();
        let fallback_view = self.fallback_view.clone();

        self.material_bind_groups
            .entry(material)
            .or_insert_with(|| {
                let fallback_view_ref = &fallback_view;
                let base_color_view = if material
                    .flags
                    .contains(MaterialFlags::USE_BASE_COLOR_TEXTURE)
                {
                    Self::view_or_fallback(assets, fallback_view_ref, material.base_color_texture)
                } else {
                    fallback_view_ref
                };
                let metallic_roughness_view = if material
                    .flags
                    .contains(MaterialFlags::USE_METALLIC_ROUGHNESS_TEXTURE)
                {
                    Self::view_or_fallback(
                        assets,
                        fallback_view_ref,
                        material.metallic_roughness_texture,
                    )
                } else {
                    fallback_view_ref
                };
                let normal_view = if material.flags.contains(MaterialFlags::USE_NORMAL_TEXTURE) {
                    Self::view_or_fallback(assets, fallback_view_ref, material.normal_texture)
                } else {
                    fallback_view_ref
                };
                let emissive_view = if material.flags.contains(MaterialFlags::USE_EMISSIVE_TEXTURE)
                {
                    Self::view_or_fallback(assets, fallback_view_ref, material.emissive_texture)
                } else {
                    fallback_view_ref
                };
                let occlusion_view = if material
                    .flags
                    .contains(MaterialFlags::USE_OCCLUSION_TEXTURE)
                {
                    Self::view_or_fallback(assets, fallback_view_ref, material.occlusion_texture)
                } else {
                    fallback_view_ref
                };

                Self::create_bind_group(
                    device,
                    &layout,
                    &linear_sampler,
                    &nearest_sampler,
                    [
                        base_color_view,
                        metallic_roughness_view,
                        normal_view,
                        emissive_view,
                        occlusion_view,
                    ],
                )
            })
    }
}

impl TextureBindingModel {
    pub fn update(&mut self, device: &wgpu::Device, assets: &Assets) {
        match self {
            TextureBindingModel::Bindless(binder) => binder.update(device, assets),
            TextureBindingModel::Classic(binder) => binder.update(device, assets),
        }
    }

    pub fn global_bind_group(&self) -> Option<&wgpu::BindGroup> {
        if let TextureBindingModel::Bindless(bindless) = self {
            Some(bindless.global_bind_group())
        } else {
            None
        }
    }

    pub fn bind_layout(&self) -> &wgpu::BindGroupLayout {
        match self {
            TextureBindingModel::Bindless(bindless) => &bindless.layout,
            TextureBindingModel::Classic(classic) => &classic.layout,
        }
    }

    pub fn bind_group_for_material(
        &mut self,
        device: &wgpu::Device,
        assets: &Assets,
        material: Material,
    ) -> Option<&wgpu::BindGroup> {
        match self {
            TextureBindingModel::Bindless(_) => None,
            TextureBindingModel::Classic(classic) => {
                Some(classic.bind_group_for_material(device, assets, material))
            }
        }
    }
}
