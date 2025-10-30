use std::collections::HashMap;
use std::num::NonZeroU32;
use std::ptr;

use crate::asset::{material::ShaderMaterialMetadata, Assets, Handle, MaterialAsset, MaterialKind};
use crate::renderer::batch::CullMode;
use crate::renderer::internal::{CameraBuffer, DynamicObjectsBuffer, LightsBuffer, RenderContext};
use crate::renderer::material::MaterialFlags;
use crate::renderer::{
    postprocess::{GBUFFER_NORMAL_FORMAT, GBUFFER_PICK_FORMAT, GBUFFER_POSITION_FORMAT},
    Material, PipelineBuilder, SamplerFilterMode, ShaderBuilder, Vertex, MAX_TEXTURES,
};

pub(crate) struct RenderPipeline {
    pipelines: HashMap<(MaterialPipelineKey, PipelineKey), wgpu::RenderPipeline>,
    shader_modules: HashMap<(MaterialPipelineKey, SamplerFilterMode), wgpu::ShaderModule>,
    pipeline_layout: wgpu::PipelineLayout,
    uses_bindless: bool,
    depth_format: wgpu::TextureFormat,
    depth_prepass: wgpu::RenderPipeline,
    background: wgpu::RenderPipeline,
    background_pick: wgpu::RenderPipeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MaterialPipelineKey {
    Pbr,
    Shader(Handle<MaterialAsset>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PipelineKey {
    depth_test: bool,
    depth_write: bool,
    alpha_blend: bool,
    color_format: wgpu::TextureFormat,
    sample_count: u32,
    sampler_filtering: SamplerFilterMode,
    cull_mode: CullMode,
    gbuffer: bool,
    write_pick: bool,
    wireframe: bool,
}

impl PipelineKey {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        depth_test: bool,
        depth_write: bool,
        alpha_blend: bool,
        color_format: wgpu::TextureFormat,
        sample_count: u32,
        sampler_filtering: SamplerFilterMode,
        cull_mode: CullMode,
        gbuffer: bool,
        write_pick: bool,
        wireframe: bool,
    ) -> Self {
        Self {
            depth_test,
            depth_write,
            alpha_blend,
            color_format,
            sample_count,
            sampler_filtering,
            cull_mode,
            gbuffer,
            write_pick,
            wireframe,
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
        let uses_bindless = context.supports_bindless_textures;
        let (texture_bind_layout, texture_binder) = if uses_bindless {
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
            (layout, binder)
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
                                binding: 2,
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
                            wgpu::BindGroupLayoutEntry {
                                binding: 5,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 6,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                                count: None,
                            },
                        ],
                    });

            let binder = TextureBindingModel::Classic(TraditionalTextureBinder::new(
                &context.device,
                &layout,
            ));
            (layout, binder)
        };

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

        let depth_shader_source = ShaderBuilder::new()
            .with_material_system()
            .build(include_str!("../../shader/depth_prepass.wgsl"));

        let depth_shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("DepthShader"),
                source: wgpu::ShaderSource::Wgsl(depth_shader_source.into()),
            });

        let background_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("EnvironmentBackgroundPipelineLayout"),
                    bind_group_layouts: &[&camera.bind_layout, &lights.bind_layout],
                    push_constant_ranges: &[],
                });

        let background_shader_source = ShaderBuilder::background()
            .build(include_str!("../../shader/environment_background.wgsl"));

        let background_shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("EnvironmentBackgroundShader"),
                source: wgpu::ShaderSource::Wgsl(background_shader_source.into()),
            });

        let scene_format = context.scene_texture_format();

        let background_pipeline =
            PipelineBuilder::new(&context.device, &background_layout, &background_shader)
                .with_label("EnvironmentBackgroundPipeline")
                .with_vertex_entry("vs_main")
                .with_fragment_entry("fs_main_gbuffer")
                .with_color_target(scene_format, Some(wgpu::BlendState::REPLACE))
                .with_color_target(GBUFFER_NORMAL_FORMAT, Some(wgpu::BlendState::REPLACE))
                .with_color_target(GBUFFER_POSITION_FORMAT, Some(wgpu::BlendState::REPLACE))
                .with_depth_stencil(
                    context.depth.format,
                    false, // depth_write
                    wgpu::CompareFunction::Always,
                )
                .with_no_culling()
                .with_multisample(sample_count)
                .build();

        let background_pick_pipeline =
            PipelineBuilder::new(&context.device, &background_layout, &background_shader)
                .with_label("EnvironmentBackgroundPipelinePick")
                .with_vertex_entry("vs_main")
                .with_fragment_entry("fs_main_gbuffer_pick")
                .with_color_target(scene_format, Some(wgpu::BlendState::REPLACE))
                .with_color_target(GBUFFER_NORMAL_FORMAT, Some(wgpu::BlendState::REPLACE))
                .with_color_target(GBUFFER_POSITION_FORMAT, Some(wgpu::BlendState::REPLACE))
                .with_color_target(GBUFFER_PICK_FORMAT, None)
                .with_depth_stencil(
                    context.depth.format,
                    false, // depth_write
                    wgpu::CompareFunction::Always,
                )
                .with_no_culling()
                .with_multisample(sample_count)
                .build();

        let mut shader_modules = HashMap::new();
        for filtering in [SamplerFilterMode::Linear, SamplerFilterMode::Nearest] {
            let label = match filtering {
                SamplerFilterMode::Linear => "RendererShaderLinear",
                SamplerFilterMode::Nearest => "RendererShaderNearest",
            };
            let source = Self::shader_source(uses_bindless, filtering);
            let module = context
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(label),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                });
            shader_modules.insert((MaterialPipelineKey::Pbr, filtering), module);
        }

        let pipelines = HashMap::new();

        let depth_prepass = Self::create_depth_prepass_pipeline(
            context,
            &depth_pipeline_layout,
            &depth_shader,
            sample_count,
        );

        (
            Self {
                pipelines,
                shader_modules,
                pipeline_layout,
                uses_bindless,
                depth_format: context.depth.format,
                depth_prepass,
                background: background_pipeline,
                background_pick: background_pick_pipeline,
            },
            texture_binder,
        )
    }

    fn shader_source(bindless: bool, filtering: SamplerFilterMode) -> String {
        // Use the preset configuration for full PBR with all features
        ShaderBuilder::full_pbr_filtered(bindless, filtering)
            .build(include_str!("../../shader/common.wgsl"))
    }

    fn build_shader_material_source(
        bindless: bool,
        filtering: SamplerFilterMode,
        metadata: &ShaderMaterialMetadata,
    ) -> String {
        let mut builder = ShaderBuilder::new()
            .with_material_system()
            .with_constants()
            .with_bindings_for_filter(bindless, filtering);

        if metadata.needs_lighting_include() {
            builder = builder
                .with_lighting()
                .with_shadows()
                .with_environment()
                .with_lighting_and_shadows();
        }

        let processed = Self::preprocess_shader_source(metadata.wgsl_source());
        let mut source = builder.build_modules_only();
        source.push_str(&processed);
        source
    }

    fn preprocess_shader_source(source: &str) -> String {
        if source.ends_with('\n') {
            source.to_string()
        } else {
            let mut owned = String::with_capacity(source.len() + 1);
            owned.push_str(source);
            owned.push('\n');
            owned
        }
    }

    fn create_pipeline(
        device: &wgpu::Device,
        pipeline_layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        key: PipelineKey,
        depth_format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        let depth_compare = if key.depth_test {
            wgpu::CompareFunction::LessEqual
        } else {
            wgpu::CompareFunction::Always
        };

        let blend_state = if key.alpha_blend {
            Some(wgpu::BlendState::ALPHA_BLENDING)
        } else {
            Some(wgpu::BlendState::REPLACE)
        };

        let mut builder = PipelineBuilder::new(device, pipeline_layout, shader)
            .with_label("MainRenderPipeline")
            .with_vertex_buffer(Vertex::layout())
            .with_multisample(key.sample_count);

        if key.gbuffer {
            let entry = if key.write_pick {
                "fs_main_gbuffer_pick"
            } else {
                "fs_main_gbuffer"
            };
            builder = builder.with_fragment_entry(entry);
        } else if key.write_pick {
            builder = builder.with_fragment_entry("fs_main_pick");
        }

        builder = builder.with_color_target(key.color_format, blend_state);

        if key.gbuffer {
            builder = builder
                .with_color_target(GBUFFER_NORMAL_FORMAT, Some(wgpu::BlendState::REPLACE))
                .with_color_target(GBUFFER_POSITION_FORMAT, Some(wgpu::BlendState::REPLACE));
        }

        if key.write_pick {
            builder = builder.with_color_target(GBUFFER_PICK_FORMAT, None);
        }

        builder = match key.cull_mode {
            CullMode::Back => builder.with_cull_mode(Some(wgpu::Face::Back)),
            CullMode::Front => builder.with_cull_mode(Some(wgpu::Face::Front)),
            CullMode::None => builder.with_cull_mode(None),
        };

        if key.wireframe {
            builder = builder.with_polygon_mode(wgpu::PolygonMode::Line);
        }

        if key.depth_test || key.depth_write {
            builder = builder.with_depth_stencil(depth_format, key.depth_write, depth_compare);
        }

        builder.build()
    }

    pub(crate) fn pipeline_for_material(
        &mut self,
        device: &wgpu::Device,
        assets: &Assets,
        key: PipelineKey,
        material_key: MaterialPipelineKey,
    ) -> &wgpu::RenderPipeline {
        let effective_key = if matches!(material_key, MaterialPipelineKey::Shader(_))
            && (key.gbuffer || key.write_pick)
        {
            MaterialPipelineKey::Pbr
        } else {
            material_key
        };

        let entry_key = (effective_key, key);
        if !self.pipelines.contains_key(&entry_key) {
            let shader_handle = self.shader_module_for_material(
                device,
                assets,
                effective_key,
                key.sampler_filtering,
            );
            let pipeline = Self::create_pipeline(
                device,
                &self.pipeline_layout,
                &shader_handle,
                key,
                self.depth_format,
            );
            self.pipelines.insert(entry_key, pipeline);
        }

        self.pipelines
            .get(&entry_key)
            .expect("pipeline should exist for requested key")
    }

    fn shader_module_for_material(
        &mut self,
        device: &wgpu::Device,
        assets: &Assets,
        material_key: MaterialPipelineKey,
        filtering: SamplerFilterMode,
    ) -> wgpu::ShaderModule {
        if let Some(module) = self.shader_modules.get(&(material_key, filtering)) {
            return module.clone();
        }

        match material_key {
            MaterialPipelineKey::Pbr => self
                .shader_modules
                .get(&(MaterialPipelineKey::Pbr, filtering))
                .expect("missing default PBR shader module")
                .clone(),
            MaterialPipelineKey::Shader(handle) => {
                let fallback = self
                    .shader_modules
                    .get(&(MaterialPipelineKey::Pbr, filtering))
                    .expect("missing default PBR shader module")
                    .clone();

                let Some(asset) = assets.material(handle) else {
                    log::warn!(
                        "Shader material {:?} missing; falling back to default PBR shader",
                        handle
                    );
                    return fallback;
                };

                let MaterialKind::Shader(metadata) = asset.kind() else {
                    log::warn!(
                        "Material {:?} expected to be shader but is {:?}; using default shader",
                        handle,
                        asset.kind()
                    );
                    return fallback;
                };

                let source =
                    Self::build_shader_material_source(self.uses_bindless, filtering, metadata);
                let label = format!("RendererMaterialShader{}", handle.index());
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(&label),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                });
                self.shader_modules
                    .insert((material_key, filtering), module);

                let shader_ref = self
                    .shader_modules
                    .get(&(material_key, filtering))
                    .expect("shader module just inserted");

                if let Some(pbr_module) = self
                    .shader_modules
                    .get(&(MaterialPipelineKey::Pbr, filtering))
                {
                    debug_assert!(
                        !ptr::eq(shader_ref, pbr_module),
                        "Shader material should compile to a distinct shader module"
                    );
                }

                shader_ref.clone()
            }
        }
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

    pub(crate) fn background(&self, write_pick: bool) -> &wgpu::RenderPipeline {
        if write_pick {
            &self.background_pick
        } else {
            &self.background
        }
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
