// renderer/renderer.rs
use crate::asset::Assets;
use crate::renderer::material::MaterialFlags;
use crate::renderer::{CameraUniform, Depth, Material, RenderBatcher, Vertex};
use crate::scene::Camera;

use std::{collections::HashMap, mem, num::NonZeroU64};
use wgpu::util::DeviceExt;
use winit::{dpi::PhysicalSize, window::Window};

const INITIAL_OBJECTS_CAPACITY: u32 = 1024;
const SAMPLE_COUNT: u32 = 4;

pub struct Renderer {
    context: RenderContext,
    pipeline: RenderPipeline,
    objects_buffer: DynamicObjectsBuffer,
    camera_buffer: CameraBuffer,
}

struct MsaaTexture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl MsaaTexture {
    fn new(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration, sample_count: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("MSAA Texture"),
            size: wgpu::Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format: config.format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view }
    }
}

struct RenderContext {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    depth: Depth,
    msaa_texture: MsaaTexture,
    supports_bindless_textures: bool,
}

struct RenderPipeline {
    pipeline: wgpu::RenderPipeline,
    texture_bind_layout: wgpu::BindGroupLayout,
    texture_binder: TextureBindingModel,
}

struct DynamicObjectsBuffer {
    buffer: wgpu::Buffer,
    capacity: u32,
    bind_group: wgpu::BindGroup,
    bind_layout: wgpu::BindGroupLayout,
    scratch: Vec<crate::renderer::ObjectData>,
}

struct CameraBuffer {
    buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    bind_layout: wgpu::BindGroupLayout,
}

impl Renderer {
    pub async fn new(window: &Window) -> Self {
        let size = window.inner_size();
        let context = RenderContext::new(window, size).await;
        let camera_buffer = CameraBuffer::new(&context.device);
        let objects_buffer = DynamicObjectsBuffer::new(&context.device, INITIAL_OBJECTS_CAPACITY);
        let pipeline = RenderPipeline::new(&context, &camera_buffer, &objects_buffer);

        Self {
            context,
            pipeline,
            objects_buffer,
            camera_buffer,
        }
    }

    pub fn get_device(&self) -> &wgpu::Device {
        &self.context.device
    }

    pub fn get_queue(&self) -> &wgpu::Queue {
        &self.context.queue
    }

    pub fn resize(&mut self, new_size: PhysicalSize<u32>) {
        self.context.resize(new_size);
    }

    pub fn aspect_ratio(&self) -> f32 {
        self.context.config.width as f32 / self.context.config.height.max(1) as f32
    }

    pub fn set_camera(&self, camera: &Camera, aspect: f32) {
        let vp = camera.view_proj(aspect);
        let uni = CameraUniform::from_matrix(vp, camera.position()); // Pass camera position
        self.context
            .queue
            .write_buffer(&self.camera_buffer.buffer, 0, bytemuck::bytes_of(&uni));
    }

    pub fn create_mesh(&self, vertices: &[Vertex], indices: &[u32]) -> crate::asset::Mesh {
        crate::asset::Mesh::from_vertices(&self.context.device, vertices, indices)
    }

    pub fn update_texture_bind_group(&mut self, assets: &Assets) {
        self.pipeline
            .texture_binder
            .update(&self.context.device, assets);
    }

    pub fn render(
        &mut self,
        assets: &Assets,
        batcher: &RenderBatcher,
    ) -> Result<(), wgpu::SurfaceError> {
        let frame = self.context.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder =
            self.context
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("Encoder"),
                });

        self.objects_buffer.update(&self.context, batcher)?;

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("MainPass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.context.msaa_texture.view, // Render to MSAA
                    depth_slice: None,
                    resolve_target: Some(&view), // Resolve to surface
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.07,
                            b: 0.10,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.context.depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            rpass.set_pipeline(&self.pipeline.pipeline);
            rpass.set_bind_group(0, &self.camera_buffer.bind_group, &[]);
            rpass.set_bind_group(1, &self.objects_buffer.bind_group, &[]);

            match &mut self.pipeline.texture_binder {
                TextureBindingModel::Bindless(bindless) => {
                    rpass.set_bind_group(2, bindless.global_bind_group(), &[]);

                    let mut object_offset = 0u32;
                    for (mesh_handle, instances) in batcher.iter() {
                        let Some(mesh) = assets.meshes.get(mesh_handle) else {
                            log::warn!("Skipping batch with invalid mesh handle");
                            object_offset += instances.len() as u32;
                            continue;
                        };

                        let instance_count = instances.len() as u32;
                        rpass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
                        rpass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());
                        rpass.draw_indexed(
                            0..mesh.index_count(),
                            0,
                            object_offset..(object_offset + instance_count),
                        );

                        object_offset += instance_count;
                    }
                }
                TextureBindingModel::Classic(classic) => {
                    rpass.set_bind_group(2, classic.default_bind_group(), &[]);

                    let mut object_offset = 0u32;
                    for (mesh_handle, instances) in batcher.iter() {
                        let Some(mesh) = assets.meshes.get(mesh_handle) else {
                            log::warn!("Skipping batch with invalid mesh handle");
                            object_offset += instances.len() as u32;
                            continue;
                        };

                        let instance_count = instances.len() as u32;
                        rpass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
                        rpass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());

                        let mut local_offset = 0usize;
                        while local_offset < instances.len() {
                            let material = instances[local_offset].material;
                            let bind_group = classic.bind_group_for_material(
                                &self.context.device,
                                assets,
                                material,
                            );
                            rpass.set_bind_group(2, bind_group, &[]);

                            let mut run_length = 1usize;
                            while local_offset + run_length < instances.len()
                                && instances[local_offset + run_length].material == material
                            {
                                run_length += 1;
                            }

                            let start_instance = object_offset + local_offset as u32;
                            let end_instance = start_instance + run_length as u32;
                            rpass.draw_indexed(
                                0..mesh.index_count(),
                                0,
                                start_instance..end_instance,
                            );

                            local_offset += run_length;
                        }

                        object_offset += instance_count;
                    }
                }
            }
        }

        self.context.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

impl RenderContext {
    async fn new(window: &Window, size: PhysicalSize<u32>) -> Self {
        #[cfg(target_arch = "wasm32")]
        {
            log::info!("Checking WebGPU/WebGL availability...");
        }

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            #[cfg(target_arch = "wasm32")]
            backends: wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL,
            #[cfg(not(target_arch = "wasm32"))]
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        log::info!("Instance created, creating surface...");

        // SAFETY: The window is owned by the App struct and lives for the entire
        // duration of the program. The surface is also owned by the App (via Renderer).
        // The window is guaranteed to outlive the surface because the Renderer is
        // dropped before the window when the App is destroyed.
        let surface = unsafe {
            instance
                .create_surface_unsafe(
                    wgpu::SurfaceTargetUnsafe::from_window(window)
                        .expect("Failed to create surface target"),
                )
                .expect("Failed to create surface")
        };

        log::info!("Surface created successfully!");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("Failed to find adapter");

        log::info!("Using adapter: {:?}", adapter.get_info());
        log::info!("Using backend: {:?}", adapter.get_info().backend);
        let adapter_features = adapter.features();
        log::info!("Adapter features: {:?}", adapter_features);
        
        // FORCE TRADITIONAL PATH FOR TESTING
        // Set to true to force traditional, false to allow bindless (when available)
        let force_traditional = false;

        let mut required_features = wgpu::Features::empty();
        let supports_bindless_textures = if force_traditional {
            log::warn!("Bindless textures DISABLED (forced for testing)");
            false
        } else if adapter_features
            .contains(wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING)
        {
            required_features |=
                wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
                    | wgpu::Features::TEXTURE_BINDING_ARRAY;
            log::info!("Bindless textures enabled");
            true
        } else {
            log::warn!("Bindless textures not supported");
            false
        };

        // Only set special limits if using bindless
        let limits = if supports_bindless_textures {
            wgpu::Limits {
                max_binding_array_elements_per_shader_stage: 256,
                ..wgpu::Limits::default()
            }
        } else {
            wgpu::Limits::default()
        };

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Device"),
                required_features,
                required_limits: limits,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("Failed to create device");

        let surface_caps = surface.get_capabilities(&adapter);

        let format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())  // Prefer LINEAR format
            .unwrap_or_else(|| {
                surface_caps.formats.iter()
                    .copied()
                    .find(|f| f.is_srgb())
                    .unwrap_or(surface_caps.formats[0])
            });

        // let format = surface_caps
        //     .formats
        //     .iter()
        //     .copied()
        //     .find(|f| f.is_srgb())
        //     .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let depth = Depth::new(&device, size, SAMPLE_COUNT);
        let msaa_texture = MsaaTexture::new(&device, &config, SAMPLE_COUNT);

        log::info!("MSAA enabled: {}x", SAMPLE_COUNT);

        Self {
            surface,
            device,
            queue,
            config,
            size,
            depth,
            msaa_texture,
            supports_bindless_textures,
        }
    }

    fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth = Depth::new(&self.device, new_size, SAMPLE_COUNT);
        self.msaa_texture = MsaaTexture::new(&self.device, &self.config, SAMPLE_COUNT);
    }
}

impl CameraBuffer {
    fn new(device: &wgpu::Device) -> Self {
        let camera = CameraUniform::new();
        let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("CameraBuffer"),
            contents: bytemuck::bytes_of(&camera),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("CameraBindLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT, // <-- Add FRAGMENT here
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(
                        NonZeroU64::new(mem::size_of::<CameraUniform>() as u64).unwrap(),
                    ),
                },
                count: None,
            }],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("CameraBindGroup"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        Self {
            buffer,
            bind_group,
            bind_layout,
        }
    }
}

impl DynamicObjectsBuffer {
    fn new(device: &wgpu::Device, capacity: u32) -> Self {
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ObjectsBindLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                // CHANGE THIS LINE - add FRAGMENT to visibility
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let buffer_size =
            (capacity as usize * mem::size_of::<crate::renderer::ObjectData>()) as u64;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ObjectsBuffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ObjectsBindGroup"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        Self {
            buffer,
            capacity,
            bind_group,
            bind_layout,
            scratch: Vec::with_capacity(capacity as usize),
        }
    }

    fn update(
        &mut self,
        context: &RenderContext,
        batcher: &RenderBatcher,
    ) -> Result<(), wgpu::SurfaceError> {
        self.scratch.clear();
        for (_, instances) in batcher.iter() {
            self.scratch.extend(instances.iter().map(|inst| {
                crate::renderer::ObjectData::from_material(inst.transform.matrix(), &inst.material)
            }));
        }

        let required = self.scratch.len() as u32;
        if required > self.capacity {
            self.grow(context, required);
        }

        if !self.scratch.is_empty() {
            context
                .queue
                .write_buffer(&self.buffer, 0, bytemuck::cast_slice(&self.scratch));
        }

        Ok(())
    }

    fn grow(&mut self, context: &RenderContext, required: u32) {
        let new_capacity = required.max(self.capacity * 2);
        log::info!(
            "Growing objects buffer: {} -> {}",
            self.capacity,
            new_capacity
        );

        let buffer_size =
            (new_capacity as usize * mem::size_of::<crate::renderer::ObjectData>()) as u64;
        self.buffer = context.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ObjectsBuffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        self.bind_group = context
            .device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ObjectsBindGroup"),
                layout: &self.bind_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.buffer.as_entire_binding(),
                }],
            });

        self.capacity = new_capacity;
    }
}

const MAX_TEXTURES: usize = 256;

enum TextureBindingModel {
    Bindless(BindlessTextureBinder),
    Classic(TraditionalTextureBinder),
}

impl TextureBindingModel {
    fn update(&mut self, device: &wgpu::Device, assets: &Assets) {
        match self {
            TextureBindingModel::Bindless(binder) => binder.update(device, assets),
            TextureBindingModel::Classic(binder) => binder.update(device, assets),
        }
    }
}

struct BindlessTextureBinder {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    fallback_texture: wgpu::Texture,
    fallback_view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
}

impl BindlessTextureBinder {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("BindlessSampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
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
            &sampler,
            vec![&fallback_view; MAX_TEXTURES],
        );

        Self {
            layout: layout.clone(),
            sampler,
            fallback_texture,
            fallback_view,
            bind_group,
        }
    }

    fn create_bind_group_with_views(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
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
                    resource: wgpu::BindingResource::Sampler(sampler),
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

        self.bind_group =
            Self::create_bind_group_with_views(device, &self.layout, &self.sampler, views);

        log::debug!(
            "Updated bindless texture array with {} textures",
            assets.textures.len()
        );
    }

    fn global_bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }
}

struct TraditionalTextureBinder {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    fallback_texture: wgpu::Texture,
    fallback_view: wgpu::TextureView,
    default_bind_group: wgpu::BindGroup,
    material_bind_groups: HashMap<Material, wgpu::BindGroup>,
}

impl TraditionalTextureBinder {
    fn new(device: &wgpu::Device, layout: &wgpu::BindGroupLayout) -> Self {
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("TraditionalSampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
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

        let default_bind_group =
            Self::create_bind_group(device, layout, &sampler, [&fallback_view; 5]);

        Self {
            layout: layout.clone(),
            sampler,
            fallback_texture,
            fallback_view,
            default_bind_group,
            material_bind_groups: HashMap::new(),
        }
    }

    fn create_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        sampler: &wgpu::Sampler,
        views: [&wgpu::TextureView; 5],
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("TraditionalTextureBindGroup"),
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
                    resource: wgpu::BindingResource::Sampler(sampler),
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
        // When assets change we clear cached bind groups to force recreation with new views
        self.material_bind_groups.clear();
    }

    fn default_bind_group(&self) -> &wgpu::BindGroup {
        &self.default_bind_group
    }

    fn bind_group_for_material(
        &mut self,
        device: &wgpu::Device,
        assets: &Assets,
        material: Material,
    ) -> &wgpu::BindGroup {
        let layout = self.layout.clone();
        let sampler = self.sampler.clone();
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
                    &sampler,
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

impl RenderPipeline {
    fn new(context: &RenderContext, camera: &CameraBuffer, objects: &DynamicObjectsBuffer) -> Self {
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
                                count: std::num::NonZero::new(MAX_TEXTURES as u32),
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 1,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                                count: None,
                            },
                        ],
                    });

            let binder =
                TextureBindingModel::Bindless(BindlessTextureBinder::new(&context.device, &layout));
            let shader_source = Self::shader_source(true);
            (layout, binder, shader_source)
        } else {
            let layout =
                context
                    .device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("MaterialTextureBindGroupLayout"),
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
                        ],
                    });

            let binder = TextureBindingModel::Classic(TraditionalTextureBinder::new(
                &context.device,
                &layout,
            ));
            let shader_source = Self::shader_source(false);
            (layout, binder, shader_source)
        };

        let shader = context
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Shader"),
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
                        &texture_bind_layout,
                    ],
                    push_constant_ranges: &[],
                });

        let pipeline = context
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Pipeline"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[Vertex::layout()],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: context.config.format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    cull_mode: Some(wgpu::Face::Back),
                    front_face: wgpu::FrontFace::Ccw,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: context.depth.format,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::LessEqual,
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLE_COUNT,
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },
                multiview: None,
                cache: None,
            });

        Self {
            pipeline,
            texture_bind_layout,
            texture_binder,
        }
    }

    fn shader_source(bindless: bool) -> String {
        let bindings = if bindless {
            include_str!("../shader/bindings_bindless.wgsl")
        } else {
            include_str!("../shader/bindings_traditional.wgsl")
        };
        format!("{bindings}\n{}", include_str!("../shader/common.wgsl"))
    }
}
