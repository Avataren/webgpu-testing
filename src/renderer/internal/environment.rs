use std::path::{Path, PathBuf};

use bytemuck::bytes_of;
use half::f16;

use crate::environment::Environment;
use crate::renderer::uniforms::EnvironmentUniform;

pub(crate) struct EnvironmentResources {
    uniform: EnvironmentUniform,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    fallback_texture: TextureResource,
    hdr_texture: Option<TextureResource>,
    current_path: Option<PathBuf>,
    current_view_is_hdr: bool,
    current_max_lod: f32,
}

struct TextureResource {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    levels: u32,
}

impl EnvironmentResources {
    pub(crate) fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let uniform = EnvironmentUniform::default();
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("EnvironmentUniformBuffer"),
            size: std::mem::size_of::<EnvironmentUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&uniform_buffer, 0, bytes_of(&uniform));

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("EnvironmentSampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            lod_min_clamp: 0.0,
            lod_max_clamp: 16.0,
            ..Default::default()
        });

        let fallback_texture = create_single_pixel_texture(device, queue, [0.0, 0.0, 0.0, 1.0]);
        let fallback_max_lod = fallback_texture.levels.saturating_sub(1) as f32;

        Self {
            uniform,
            uniform_buffer,
            sampler,
            fallback_texture,
            hdr_texture: None,
            current_path: None,
            current_view_is_hdr: false,
            current_max_lod: fallback_max_lod,
        }
    }

    pub(crate) fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        environment: &Environment,
    ) -> bool {
        let active_hdr = environment.active_hdr_background();
        let desired_path = active_hdr.map(|hdr| hdr.path().to_path_buf());
        let needs_reload = match (&desired_path, &self.current_path, &self.hdr_texture) {
            (Some(new_path), Some(current_path), Some(_)) if new_path == current_path => false,
            (Some(_), _, _) => true,
            (None, _, _) => false,
        };

        let mut texture_reloaded = false;

        if needs_reload {
            if let Some(path) = desired_path.as_ref() {
                match load_hdr_texture(device, queue, path) {
                    Ok(texture) => {
                        self.hdr_texture = Some(texture);
                        self.current_path = Some(path.clone());
                        texture_reloaded = true;
                    }
                    Err(err) => {
                        log::error!("Failed to load HDR environment {:?}: {}", path, err);
                        self.hdr_texture = None;
                        self.current_path = None;
                    }
                }
            } else {
                self.hdr_texture = None;
                self.current_path = None;
            }
        }

        let has_hdr_texture = self.hdr_texture.is_some();
        let use_hdr = active_hdr.is_some() && has_hdr_texture;

        let active_levels = if use_hdr {
            self.hdr_texture
                .as_ref()
                .map(|tex| tex.levels)
                .unwrap_or(self.fallback_texture.levels)
        } else {
            self.fallback_texture.levels
        };
        self.current_max_lod = active_levels.saturating_sub(1) as f32;

        let hdr_intensity = active_hdr.map(|hdr| hdr.intensity()).unwrap_or(1.0);
        let new_uniform =
            build_uniform(environment, use_hdr, hdr_intensity, self.current_max_lod);
        if new_uniform != self.uniform {
            self.uniform = new_uniform;
            queue.write_buffer(&self.uniform_buffer, 0, bytes_of(&self.uniform));
        }

        let texture_changed =
            self.current_view_is_hdr != use_hdr || (use_hdr && texture_reloaded);
        self.current_view_is_hdr = use_hdr;

        texture_changed
    }

    pub(crate) fn uniform_buffer(&self) -> &wgpu::Buffer {
        &self.uniform_buffer
    }

    pub(crate) fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    pub(crate) fn texture_view(&self) -> &wgpu::TextureView {
        if self.current_view_is_hdr {
            self.hdr_texture
                .as_ref()
                .map(|tex| &tex.view)
                .unwrap_or(&self.fallback_texture.view)
        } else {
            &self.fallback_texture.view
        }
    }
}

fn build_uniform(
    environment: &Environment,
    use_hdr: bool,
    hdr_intensity: f32,
    max_lod: f32,
) -> EnvironmentUniform {
    let color = environment.clear_color();
    EnvironmentUniform {
        flags_intensity: [
            if use_hdr { 1.0 } else { 0.0 },
            hdr_intensity.max(0.0),
            environment.ambient_intensity().max(0.0),
            max_lod.max(0.0),
        ],
        ambient_color: [
            color.r as f32,
            color.g as f32,
            color.b as f32,
            1.0,
        ],
    }
}

fn load_hdr_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    path: &Path,
) -> Result<TextureResource, String> {
    let image = image::open(path)
        .map_err(|err| format!("failed to open HDR image {:?}: {}", path, err))?
        .to_rgba32f();

    let (width, height) = image.dimensions();
    let raw = image.into_raw();
    let mut converted = Vec::with_capacity(raw.len());
    for value in raw {
        converted.push(f16::from_f32(value).to_bits());
    }

    let mip_level_count = calculate_mip_levels(width, height);

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("EnvironmentHDRTexture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });

    let bytes_per_row = width
        .checked_mul(8)
        .ok_or_else(|| "HDR texture width overflow".to_string())?;

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        bytemuck::cast_slice(&converted),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(bytes_per_row),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    generate_mipmaps(
        device,
        queue,
        &texture,
        mip_level_count,
        wgpu::TextureFormat::Rgba16Float,
    );

    Ok(TextureResource {
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        _texture: texture,
        levels: mip_level_count,
    })
}

fn calculate_mip_levels(width: u32, height: u32) -> u32 {
    let mut levels = 1u32;
    let mut size = width.max(height).max(1);
    while size > 1 {
        size = (size + 1) / 2;
        levels += 1;
    }
    levels
}

fn generate_mipmaps(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    texture: &wgpu::Texture,
    mip_level_count: u32,
    format: wgpu::TextureFormat,
) {
    if mip_level_count <= 1 {
        return;
    }

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Environment Mipmap Shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("../blit.wgsl").into()),
    });

    let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Environment Mipmap BindGroupLayout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
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
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Environment Mipmap PipelineLayout"),
        bind_group_layouts: &[&bind_group_layout],
        push_constant_ranges: &[],
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Environment Mipmap Pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            ..Default::default()
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview: None,
        cache: None,
    });

    let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Environment Mipmap Sampler"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        address_mode_w: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::FilterMode::Nearest,
        ..Default::default()
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Environment Mipmap Encoder"),
    });

    for target_mip in 1..mip_level_count {
        let src_mip = target_mip - 1;

        let src_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("Environment Mip Src"),
            format: Some(format),
            dimension: Some(wgpu::TextureViewDimension::D2),
            aspect: wgpu::TextureAspect::All,
            base_mip_level: src_mip,
            mip_level_count: Some(1),
            base_array_layer: 0,
            array_layer_count: Some(1),
            usage: Some(wgpu::TextureUsages::TEXTURE_BINDING),
        });

        let dst_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("Environment Mip Dst"),
            format: Some(format),
            dimension: Some(wgpu::TextureViewDimension::D2),
            aspect: wgpu::TextureAspect::All,
            base_mip_level: target_mip,
            mip_level_count: Some(1),
            base_array_layer: 0,
            array_layer_count: Some(1),
            usage: Some(wgpu::TextureUsages::RENDER_ATTACHMENT),
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Environment Mip BindGroup"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&src_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Environment Mipmap Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &dst_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        rpass.set_pipeline(&pipeline);
        rpass.set_bind_group(0, &bind_group, &[]);
        rpass.draw(0..3, 0..1);
    }

    queue.submit(Some(encoder.finish()));
}

fn create_single_pixel_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    color: [f32; 4],
) -> TextureResource {
    let values: [u16; 4] = {
        [
            f16::from_f32(color[0]).to_bits(),
            f16::from_f32(color[1]).to_bits(),
            f16::from_f32(color[2]).to_bits(),
            f16::from_f32(color[3]).to_bits(),
        ]
    };

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("EnvironmentFallbackTexture"),
        size: wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba16Float,
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
        bytemuck::cast_slice(&values),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(8),
            rows_per_image: Some(1),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );

    TextureResource {
        view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
        _texture: texture,
        levels: 1,
    }
}
