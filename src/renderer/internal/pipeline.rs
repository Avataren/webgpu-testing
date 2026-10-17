use crate::renderer::shader_source::wgsl;
use std::collections::{HashMap, HashSet};
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
    /// Shader material modules with a generated `fs_main_gbuffer` (see
    /// [`super::shader_material`]): their own geometry fills the G-buffer.
    gbuffer_shader_modules: HashSet<(Handle<MaterialAsset>, SamplerFilterMode)>,
    pipeline_layout: wgpu::PipelineLayout,
    uses_bindless: bool,
    depth_format: wgpu::TextureFormat,
    /// The depth prepass, culling back faces ([0]) or drawing both sides of
    /// double-sided materials ([1]).
    depth_prepass: [wgpu::RenderPipeline; 2],
    /// The prepass for cut-out surfaces, with the alpha test (main pipeline layout), by
    /// sidedness like `depth_prepass`.
    depth_prepass_masked: [wgpu::RenderPipeline; 2],
    background: wgpu::RenderPipeline,
}

/// What [`RenderPipeline::rebuild_shaders`] builds.
pub(crate) struct RebuiltShaders {
    shader_modules: HashMap<(MaterialPipelineKey, SamplerFilterMode), wgpu::ShaderModule>,
    depth_prepass: [wgpu::RenderPipeline; 2],
    depth_prepass_masked: [wgpu::RenderPipeline; 2],
    background: wgpu::RenderPipeline,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum MaterialPipelineKey {
    Pbr,
    Error,
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
    /// The pick pass: only entity ids, single-sample, with a depth attachment whatever
    /// the batch's depth state.
    pick_only: bool,
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
        pick_only: bool,
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
            pick_only,
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
                        Some(&camera.bind_layout),
                        Some(&objects.bind_layout),
                        Some(&lights.bind_layout),
                        Some(&texture_bind_layout),
                    ],
                    immediate_size: 0,
                });

        let shaders = Self::build_shaders(
            context,
            camera,
            objects,
            lights,
            &pipeline_layout,
            uses_bindless,
            sample_count,
        )
        .expect("built-in renderer shaders should compile through naga");

        (
            Self {
                pipelines: HashMap::new(),
                shader_modules: shaders.shader_modules,
                gbuffer_shader_modules: HashSet::new(),
                pipeline_layout,
                uses_bindless,
                depth_format: context.depth.format,
                depth_prepass: shaders.depth_prepass,
                depth_prepass_masked: shaders.depth_prepass_masked,
                background: shaders.background,
            },
            texture_binder,
        )
    }

    /// Build the shader modules again from the current sources (see
    /// [`shader_source`](crate::renderer::shader_source)), for a shader reload. Pipelines
    /// and shader materials are rebuilt from them as frames ask for them.
    pub(crate) fn rebuild_shaders(
        &self,
        context: &RenderContext,
        camera: &CameraBuffer,
        objects: &DynamicObjectsBuffer,
        lights: &LightsBuffer,
        sample_count: u32,
    ) -> Result<RebuiltShaders, String> {
        Self::build_shaders(
            context,
            camera,
            objects,
            lights,
            &self.pipeline_layout,
            self.uses_bindless,
            sample_count,
        )
    }

    /// Switch to rebuilt shaders, dropping every pipeline built from the old ones.
    pub(crate) fn install_shaders(&mut self, shaders: RebuiltShaders) {
        self.pipelines.clear();
        self.shader_modules = shaders.shader_modules;
        self.gbuffer_shader_modules.clear();
        self.depth_prepass = shaders.depth_prepass;
        self.depth_prepass_masked = shaders.depth_prepass_masked;
        self.background = shaders.background;
    }

    /// The modules and fixed pipelines that depend on shader source. Each source is
    /// checked with naga first, so a broken shader is an error rather than a device error.
    fn build_shaders(
        context: &RenderContext,
        camera: &CameraBuffer,
        objects: &DynamicObjectsBuffer,
        lights: &LightsBuffer,
        main_layout: &wgpu::PipelineLayout,
        uses_bindless: bool,
        sample_count: u32,
    ) -> Result<RebuiltShaders, String> {
        let masked_source = Self::depth_prepass_masked_source(uses_bindless);
        Self::validate_shader(&masked_source)
            .map_err(|error| format!("cut-out depth prepass: {error}"))?;
        let masked_shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("DepthMaskedShader"),
                source: wgpu::ShaderSource::Wgsl(masked_source.into()),
            });
        let depth_prepass_masked = [false, true].map(|double_sided| {
            PipelineBuilder::new(&context.device, main_layout, &masked_shader)
                .with_label(if double_sided {
                    "DepthPrepassMaskedDoubleSidedPipeline"
                } else {
                    "DepthPrepassMaskedPipeline"
                })
                .with_vertex_entry("vs_masked")
                .with_fragment_entry("fs_masked")
                .with_vertex_buffer(Vertex::layout())
                .with_depth_stencil(context.depth.format, true, wgpu::CompareFunction::LessEqual)
                .with_cull_mode((!double_sided).then_some(wgpu::Face::Back))
                .with_multisample(sample_count)
                .build()
        });

        let depth_pipeline_layout =
            context
                .device
                .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                    label: Some("DepthPipelineLayout"),
                    bind_group_layouts: &[Some(&camera.bind_layout), Some(&objects.bind_layout)],
                    immediate_size: 0,
                });

        let depth_shader_source = Self::depth_prepass_source();

        Self::validate_shader(&depth_shader_source)
            .map_err(|error| format!("depth prepass: {error}"))?;
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
                    bind_group_layouts: &[Some(&camera.bind_layout), Some(&lights.bind_layout)],
                    immediate_size: 0,
                });

        let background_shader_source = Self::background_source();

        Self::validate_shader(&background_shader_source)
            .map_err(|error| format!("environment background: {error}"))?;
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

        let mut shader_modules = HashMap::new();
        for filtering in [SamplerFilterMode::Linear, SamplerFilterMode::Nearest] {
            let label = match filtering {
                SamplerFilterMode::Linear => "RendererShaderLinear",
                SamplerFilterMode::Nearest => "RendererShaderNearest",
            };
            let source = Self::shader_source(uses_bindless, filtering);
            Self::validate_shader(&source).map_err(|error| format!("{label}: {error}"))?;
            let module = context
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(label),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                });
            shader_modules.insert((MaterialPipelineKey::Pbr, filtering), module);

            let error_label = match filtering {
                SamplerFilterMode::Linear => "RendererShaderErrorLinear",
                SamplerFilterMode::Nearest => "RendererShaderErrorNearest",
            };
            let error_source = Self::error_shader_source(uses_bindless, filtering);
            Self::validate_shader(&error_source)
                .map_err(|error| format!("{error_label}: {error}"))?;
            let error_module = context
                .device
                .create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(error_label),
                    source: wgpu::ShaderSource::Wgsl(error_source.into()),
                });
            shader_modules.insert((MaterialPipelineKey::Error, filtering), error_module);
        }

        let depth_prepass = [false, true].map(|double_sided| {
            Self::create_depth_prepass_pipeline(
                context,
                &depth_pipeline_layout,
                &depth_shader,
                sample_count,
                double_sided,
            )
        });

        Ok(RebuiltShaders {
            shader_modules,
            depth_prepass,
            depth_prepass_masked,
            background: background_pipeline,
        })
    }

    fn depth_prepass_source() -> String {
        ShaderBuilder::new()
            .with_camera()
            .with_objects()
            .build(wgsl!("depth_prepass.wgsl"))
    }

    fn depth_prepass_masked_source(bindless: bool) -> String {
        ShaderBuilder::new()
            .with_camera()
            .with_objects()
            .with_constants()
            .with_bindings_for_filter(bindless, SamplerFilterMode::Linear)
            .with_module(wgsl!("depth_prepass.wgsl"))
            .build(wgsl!("depth_prepass_masked.wgsl"))
    }

    fn background_source() -> String {
        ShaderBuilder::background().build(wgsl!("environment_background.wgsl"))
    }

    fn shader_source(bindless: bool, filtering: SamplerFilterMode) -> String {
        // Use the preset configuration for full PBR with all features
        ShaderBuilder::full_pbr_filtered(bindless, filtering).build(wgsl!("common.wgsl"))
    }

    fn error_shader_source(bindless: bool, filtering: SamplerFilterMode) -> String {
        ShaderBuilder::new()
            .with_camera()
            .with_objects()
            .with_constants()
            .with_bindings_for_filter(bindless, filtering)
            .build(include_str!("../shader/error_fallback.wgsl"))
    }

    fn validate_shader(source: &str) -> Result<(), String> {
        crate::renderer::shader_source::validate(source)
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
        builder.build(&processed)
    }

    /// Whether shader material `handle` fills the G-buffer from its own shader (else the
    /// PBR shader stands in for it in the main pass, and it draws its colour after).
    pub(crate) fn shader_writes_gbuffer(
        &mut self,
        device: &wgpu::Device,
        assets: &Assets,
        handle: Handle<MaterialAsset>,
        filtering: SamplerFilterMode,
    ) -> bool {
        self.shader_module_for_material(
            device,
            assets,
            MaterialPipelineKey::Shader(handle),
            filtering,
        );
        self.gbuffer_shader_modules.contains(&(handle, filtering))
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
            // Every batch binds last frame's positions as slot 1 (see `BatchRecorder`).
            .with_vertex_buffer(Vertex::previous_position_layout())
            .with_multisample(key.sample_count);

        if key.pick_only {
            builder = builder
                .with_fragment_entry("fs_pick")
                .with_color_target(GBUFFER_PICK_FORMAT, None);
        } else {
            if key.gbuffer {
                builder = builder.with_fragment_entry("fs_main_gbuffer");
            }
            builder = builder.with_color_target(key.color_format, blend_state);
            if key.gbuffer {
                builder = builder
                    .with_color_target(GBUFFER_NORMAL_FORMAT, Some(wgpu::BlendState::REPLACE))
                    .with_color_target(GBUFFER_POSITION_FORMAT, Some(wgpu::BlendState::REPLACE));
            }
        }

        builder = match key.cull_mode {
            CullMode::Back => builder.with_cull_mode(Some(wgpu::Face::Back)),
            CullMode::Front => builder.with_cull_mode(Some(wgpu::Face::Front)),
            CullMode::None => builder.with_cull_mode(None),
        };

        if key.wireframe {
            builder = builder.with_polygon_mode(wgpu::PolygonMode::Line);
        }

        // The pick pass always has a depth attachment, so every pipeline in it declares one.
        if key.depth_test || key.depth_write || key.pick_only {
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
        // Pick ids come from the PBR shader's geometry for every material, and so does a
        // shader material's G-buffer when it has no G-buffer entry of its own.
        let effective_key = match material_key {
            _ if key.pick_only => MaterialPipelineKey::Pbr,
            MaterialPipelineKey::Shader(handle)
                if key.gbuffer
                    && !self.shader_writes_gbuffer(
                        device,
                        assets,
                        handle,
                        key.sampler_filtering,
                    ) =>
            {
                MaterialPipelineKey::Pbr
            }
            _ => material_key,
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
            MaterialPipelineKey::Error => self
                .shader_modules
                .get(&(MaterialPipelineKey::Error, filtering))
                .expect("missing error fallback shader module")
                .clone(),
            MaterialPipelineKey::Shader(handle) => {
                let default_module = self
                    .shader_modules
                    .get(&(MaterialPipelineKey::Pbr, filtering))
                    .expect("missing default PBR shader module")
                    .clone();

                let Some(asset) = assets.material(handle) else {
                    log::warn!(
                        "Shader material {:?} missing; falling back to default PBR shader",
                        handle
                    );
                    return default_module;
                };

                let MaterialKind::Shader(metadata) = asset.kind() else {
                    log::warn!(
                        "Material {:?} expected to be shader but is {:?}; using default shader",
                        handle,
                        asset.kind()
                    );
                    return default_module;
                };

                let source =
                    Self::build_shader_material_source(self.uses_bindless, filtering, metadata);
                if let Err(error) = Self::validate_shader(&source) {
                    let fallback = self
                        .shader_modules
                        .get(&(MaterialPipelineKey::Error, filtering))
                        .expect("missing error fallback shader module")
                        .clone();

                    if let Some(path) = metadata.source_path() {
                        log::error!(
                            "Failed to compile shader material {:?} ({}):\n{}",
                            handle,
                            path.display(),
                            error
                        );
                    } else {
                        log::error!("Failed to compile shader material {:?}: {}", handle, error);
                    }

                    // Until the material changes (`invalidate_material_shader_modules`).
                    self.shader_modules
                        .insert((material_key, filtering), fallback.clone());
                    return fallback;
                }

                // Its G-buffer entry, when the shader's shape allows one.
                let source = match super::shader_material::with_gbuffer_entry(&source) {
                    Some(with_gbuffer) => {
                        self.gbuffer_shader_modules.insert((handle, filtering));
                        with_gbuffer
                    }
                    None => {
                        log::debug!(
                            "Shader material {handle:?} has no G-buffer entry; the PBR shader \
                             stands in for its G-buffer"
                        );
                        source
                    }
                };
                let label = format!("RendererMaterialShader{}", handle.index());
                let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some(&label),
                    source: wgpu::ShaderSource::Wgsl(source.into()),
                });
                self.shader_modules
                    .insert((material_key, filtering), module.clone());

                if let Some(pbr_module) = self
                    .shader_modules
                    .get(&(MaterialPipelineKey::Pbr, filtering))
                {
                    debug_assert!(
                        !ptr::eq(&module, pbr_module),
                        "Shader material should compile to a distinct shader module"
                    );
                }

                module
            }
        }
    }

    pub(crate) fn invalidate_material_shader_modules(
        &mut self,
        handle: Handle<MaterialAsset>,
        filter: Option<SamplerFilterMode>,
    ) {
        match filter {
            Some(mode) => {
                self.shader_modules
                    .remove(&(MaterialPipelineKey::Shader(handle), mode));
                self.gbuffer_shader_modules.remove(&(handle, mode));
                self.pipelines.retain(|(material_key, pipeline_key), _| {
                    if let MaterialPipelineKey::Shader(existing) = material_key {
                        if *existing == handle {
                            return pipeline_key.sampler_filtering != mode;
                        }
                    }
                    true
                });
            }
            None => {
                for mode in [SamplerFilterMode::Linear, SamplerFilterMode::Nearest] {
                    self.shader_modules
                        .remove(&(MaterialPipelineKey::Shader(handle), mode));
                    self.gbuffer_shader_modules.remove(&(handle, mode));
                }
                self.pipelines.retain(|(material_key, _), _| {
                    !matches!(material_key, MaterialPipelineKey::Shader(existing) if *existing == handle)
                });
            }
        }
    }

    fn create_depth_prepass_pipeline(
        context: &RenderContext,
        pipeline_layout: &wgpu::PipelineLayout,
        shader: &wgpu::ShaderModule,
        sample_count: u32,
        double_sided: bool,
    ) -> wgpu::RenderPipeline {
        PipelineBuilder::new(&context.device, pipeline_layout, shader)
            .with_label(if double_sided {
                "DepthPrepassDoubleSidedPipeline"
            } else {
                "DepthPrepassPipeline"
            })
            .depth_only()
            .with_vertex_buffer(Vertex::layout())
            .with_depth_stencil(context.depth.format, true, wgpu::CompareFunction::LessEqual)
            .with_cull_mode((!double_sided).then_some(wgpu::Face::Back))
            .with_multisample(sample_count)
            .build()
    }

    /// The depth prepass; `double_sided` draws both sides (no culling).
    pub(crate) fn depth_prepass(&self, double_sided: bool) -> &wgpu::RenderPipeline {
        &self.depth_prepass[usize::from(double_sided)]
    }

    /// The prepass for cut-out surfaces: the main pipeline layout (lights and textures
    /// as groups 2 and 3), with the alpha test.
    pub(crate) fn depth_prepass_masked(&self, double_sided: bool) -> &wgpu::RenderPipeline {
        &self.depth_prepass_masked[usize::from(double_sided)]
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
    /// Textures past `MAX_TEXTURES` when last warned about (0: none).
    reported_overflow: usize,
}

impl BindlessTextureBinder {
    fn create_sampler(device: &wgpu::Device, linear: bool) -> wgpu::Sampler {
        let (filter, mipmap_filter) = if linear {
            (wgpu::FilterMode::Linear, wgpu::MipmapFilterMode::Linear)
        } else {
            (wgpu::FilterMode::Nearest, wgpu::MipmapFilterMode::Nearest)
        };
        let label = if linear {
            "BindlessSamplerLinear"
        } else {
            "BindlessSamplerNearest"
        };
        device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(label),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: filter,
            min_filter: filter,
            mipmap_filter,
            ..Default::default()
        })
    }

    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let linear_sampler = Self::create_sampler(device, true);
        let nearest_sampler = Self::create_sampler(device, false);

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
            reported_overflow: 0,
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

        let overflow = assets.textures.len().saturating_sub(MAX_TEXTURES);
        if overflow > 0 && overflow != self.reported_overflow {
            log::warn!(
                "{} textures are loaded but the bindless texture array holds {MAX_TEXTURES}; \
                 materials using the {overflow} past it draw without them",
                assets.textures.len()
            );
        }
        self.reported_overflow = overflow;

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
    fn create_sampler(device: &wgpu::Device, linear: bool) -> wgpu::Sampler {
        let (filter, mipmap_filter) = if linear {
            (wgpu::FilterMode::Linear, wgpu::MipmapFilterMode::Linear)
        } else {
            (wgpu::FilterMode::Nearest, wgpu::MipmapFilterMode::Nearest)
        };
        let label = if linear {
            "TraditionalSamplerLinear"
        } else {
            "TraditionalSamplerNearest"
        };
        device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(label),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: filter,
            min_filter: filter,
            mipmap_filter,
            ..Default::default()
        })
    }

    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let linear_sampler = Self::create_sampler(device, true);
        let nearest_sampler = Self::create_sampler(device, false);

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

    /// Builds the texture bind group for `material`: the views of the textures it uses, and
    /// the fallback texture for the others.
    fn create_material_bind_group(
        &self,
        device: &wgpu::Device,
        assets: &Assets,
        material: Material,
    ) -> wgpu::BindGroup {
        let fallback = &self.fallback_view;
        let view_for = |flag: MaterialFlags, index: u32| {
            if material.flags.contains(flag) {
                Self::view_or_fallback(assets, fallback, index)
            } else {
                fallback
            }
        };
        let base_color_view = view_for(
            MaterialFlags::USE_BASE_COLOR_TEXTURE,
            material.base_color_texture,
        );
        let metallic_roughness_view = view_for(
            MaterialFlags::USE_METALLIC_ROUGHNESS_TEXTURE,
            material.metallic_roughness_texture,
        );
        let normal_view = view_for(MaterialFlags::USE_NORMAL_TEXTURE, material.normal_texture);
        let emissive_view = view_for(
            MaterialFlags::USE_EMISSIVE_TEXTURE,
            material.emissive_texture,
        );
        let occlusion_view = view_for(
            MaterialFlags::USE_OCCLUSION_TEXTURE,
            material.occlusion_texture,
        );

        Self::create_bind_group(
            device,
            &self.layout,
            &self.linear_sampler,
            &self.nearest_sampler,
            [
                base_color_view,
                metallic_roughness_view,
                normal_view,
                emissive_view,
                occlusion_view,
            ],
        )
    }

    fn bind_group_for_material(
        &mut self,
        device: &wgpu::Device,
        assets: &Assets,
        material: Material,
    ) -> &wgpu::BindGroup {
        if !self.material_bind_groups.contains_key(&material) {
            let bind_group = self.create_material_bind_group(device, assets, material);
            self.material_bind_groups.insert(material, bind_group);
        }
        &self.material_bind_groups[&material]
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

    /// A new bind group with `material`'s textures, for draws outside the batched passes
    /// (GPU particles). `None` with bindless textures, where the one global group serves
    /// every material.
    pub fn create_material_bind_group(
        &self,
        device: &wgpu::Device,
        assets: &Assets,
        material: Material,
    ) -> Option<wgpu::BindGroup> {
        match self {
            TextureBindingModel::Bindless(_) => None,
            TextureBindingModel::Classic(classic) => {
                Some(classic.create_material_bind_group(device, assets, material))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every source the main passes build at startup (the device only sees them then).
    #[test]
    fn main_pass_shaders_validate() {
        let mut sources = vec![
            ("depth prepass", RenderPipeline::depth_prepass_source()),
            (
                "cut-out depth prepass",
                RenderPipeline::depth_prepass_masked_source(true),
            ),
            (
                "cut-out depth prepass (classic)",
                RenderPipeline::depth_prepass_masked_source(false),
            ),
            ("background", RenderPipeline::background_source()),
        ];
        for bindless in [true, false] {
            for filtering in [SamplerFilterMode::Linear, SamplerFilterMode::Nearest] {
                sources.push(("pbr", RenderPipeline::shader_source(bindless, filtering)));
                sources.push((
                    "error",
                    RenderPipeline::error_shader_source(bindless, filtering),
                ));
            }
        }
        for (name, source) in sources {
            if let Err(error) = RenderPipeline::validate_shader(&source) {
                panic!("{name}: {error}");
            }
        }
    }
}
