// renderer/texture.rs (with mipmaps)

use std::path::Path;

#[cfg(target_arch = "wasm32")]
use crate::io;

struct RgbaTextureSource<'a> {
    data: &'a [u8],
    width: u32,
    height: u32,
    texture_format: wgpu::TextureFormat,
    view_format: Option<wgpu::TextureFormat>,
    label: Option<&'a str>,
}

#[derive(Debug)]
pub struct Texture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
}

impl Texture {
    /// Calculate the number of mip levels for a given texture size
    fn calculate_mip_levels(width: u32, height: u32) -> u32 {
        let max_dimension = width.max(height).max(1);
        u32::BITS - max_dimension.leading_zeros()
    }

    fn rgba_source<'a>(
        data: &'a [u8],
        width: u32,
        height: u32,
        texture_format: wgpu::TextureFormat,
        view_format: Option<wgpu::TextureFormat>,
        label: Option<&'a str>,
    ) -> RgbaTextureSource<'a> {
        RgbaTextureSource {
            data,
            width,
            height,
            texture_format,
            view_format,
            label,
        }
    }

    /// Load texture from file path with mipmaps
    pub fn from_path(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: impl AsRef<Path>,
        is_srgb: bool,
    ) -> Result<Self, String> {
        let path = path.as_ref();
        log::info!("Loading texture: {:?}", path);

        #[cfg(target_arch = "wasm32")]
        let img = {
            let bytes = io::load_binary(path)?;
            image::load_from_memory(&bytes)
                .map_err(|e| format!("Failed to decode image {:?}: {}", path, e))?
        };

        #[cfg(not(target_arch = "wasm32"))]
        let img =
            image::open(path).map_err(|e| format!("Failed to load image {:?}: {}", path, e))?;

        let rgba = img.to_rgba8();
        let (width, height) = rgba.dimensions();

        let (texture_format, view_format) = Self::formats_for_color_space(is_srgb);

        let source = Self::rgba_source(
            &rgba,
            width,
            height,
            texture_format,
            view_format,
            path.to_str(),
        );

        Ok(Self::from_rgba8(device, queue, source))
    }

    /// Create texture from rgba8 data with mipmaps
    fn from_rgba8(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        source: RgbaTextureSource<'_>,
    ) -> Self {
        let mip_level_count = Self::calculate_mip_levels(source.width, source.height);

        let size = wgpu::Extent3d {
            width: source.width,
            height: source.height,
            depth_or_array_layers: 1,
        };

        let mut view_formats = Vec::new();
        if let Some(format) = source.view_format {
            view_formats.push(format);
        }

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: source.label,
            size,
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: source.texture_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::RENDER_ATTACHMENT, // Needed for mipmap generation
            view_formats: &view_formats,
        });

        // Upload base mip level (mip 0)
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            source.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * source.width),
                rows_per_image: Some(source.height),
            },
            size,
        );

        // Generate mipmaps
        Self::generate_mipmaps(
            device,
            queue,
            &texture,
            mip_level_count,
            source.texture_format,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            format: source.view_format.or(Some(source.texture_format)),
            ..Default::default()
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear, // Enable trilinear filtering
            ..Default::default()
        });

        Self {
            texture,
            view,
            sampler,
        }
    }

    pub fn storage_rgba8(
        device: &wgpu::Device,
        width: u32,
        height: u32,
        label: Option<&str>,
    ) -> Self {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label,
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::STORAGE_BINDING,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let sampler_label = label.map(|name| format!("{name} Sampler"));
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: sampler_label.as_deref(),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self {
            texture,
            view,
            sampler,
        }
    }

    /// Generate mipmaps using GPU rendering
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

        // Create a simple shader for downsampling
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Blit Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blit.wgsl").into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Blit Bind Group Layout"),
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
            label: Some("Blit Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Blit Pipeline"),
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
            label: Some("Mip Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Mipmap Generator"),
        });

        for target_mip in 1..mip_level_count {
            let src_mip = target_mip - 1;

            let src_view = texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("Mip Source"),
                format: Some(format),
                dimension: Some(wgpu::TextureViewDimension::D2),
                aspect: wgpu::TextureAspect::All,
                base_mip_level: src_mip,
                mip_level_count: Some(1),
                base_array_layer: 0,
                array_layer_count: Some(1),
                usage: Some(wgpu::TextureUsages::TEXTURE_BINDING), // Add this line
            });

            let dst_view = texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("Mip Destination"),
                format: Some(format),
                dimension: Some(wgpu::TextureViewDimension::D2),
                aspect: wgpu::TextureAspect::All,
                base_mip_level: target_mip,
                mip_level_count: Some(1),
                base_array_layer: 0,
                array_layer_count: Some(1),
                usage: Some(wgpu::TextureUsages::RENDER_ATTACHMENT), // Add this line
            });

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Mip Bind Group"),
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
                label: Some("Mipmap Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &dst_view,
                    resolve_target: None,
                    depth_slice: None, // Add this line
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
            rpass.draw(0..3, 0..1); // Fullscreen triangle
        }

        queue.submit(Some(encoder.finish()));
    }

    /// Create a solid color 1x1 texture (no mipmaps needed)
    pub fn from_color(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color: [u8; 4],
        label: Option<&str>,
    ) -> Self {
        let source = Self::rgba_source(
            &color,
            1,
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            label,
        );

        Self::from_rgba8(device, queue, source)
    }

    /// Create texture from rgba8 image data
    pub fn from_bytes(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bytes: &[u8],
        width: u32,
        height: u32,
        label: Option<&str>,
    ) -> Self {
        let source = Self::rgba_source(
            bytes,
            width,
            height,
            wgpu::TextureFormat::Rgba8Unorm,
            Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            label,
        );

        Self::from_rgba8(device, queue, source)
    }

    /// Create a procedural checkerboard texture
    pub fn checkerboard(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: u32,
        checker_size: u32,
        color1: [u8; 4],
        color2: [u8; 4],
        label: Option<&str>,
    ) -> Self {
        let mut pixels = vec![0u8; (size * size * 4) as usize];

        for y in 0..size {
            for x in 0..size {
                let checker_x = (x / checker_size) % 2;
                let checker_y = (y / checker_size) % 2;
                let is_color1 = (checker_x + checker_y).is_multiple_of(2);

                let color = if is_color1 { color1 } else { color2 };
                let idx = ((y * size + x) * 4) as usize;
                pixels[idx..idx + 4].copy_from_slice(&color);
            }
        }

        Self::from_bytes(device, queue, &pixels, size, size, label)
    }

    /// Create default white texture (1x1)
    pub fn white(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        Self::from_color(device, queue, [255, 255, 255, 255], Some("White"))
    }

    /// Create a solid-color texture stored in a linear color space (1x1)
    pub fn from_color_linear(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color: [u8; 4],
        label: Option<&str>,
    ) -> Self {
        let source = Self::rgba_source(&color, 1, 1, wgpu::TextureFormat::Rgba8Unorm, None, label);

        Self::from_rgba8(device, queue, source)
    }

    /// Create default normal map (1x1, pointing up)
    pub fn default_normal(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        // Normal pointing straight up: (0, 0, 1) -> (128, 128, 255) in texture space
        let source = Self::rgba_source(
            &[128, 128, 255, 255],
            1,
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            Some("DefaultNormal"),
        );

        Self::from_rgba8(device, queue, source)
    }

    /// Create default metallic-roughness (1x1, non-metallic, mid-roughness)
    pub fn default_metallic_roughness(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        // R=occlusion(1.0), G=roughness(0.5), B=metallic(0.0)
        let source = Self::rgba_source(
            &[255, 128, 0, 255],
            1,
            1,
            wgpu::TextureFormat::Rgba8Unorm,
            None,
            Some("DefaultMetallicRoughness"),
        );

        Self::from_rgba8(device, queue, source)
    }

    /// Determine the texture and view formats used for a colour texture.
    fn formats_for_color_space(
        is_srgb: bool,
    ) -> (wgpu::TextureFormat, Option<wgpu::TextureFormat>) {
        if is_srgb {
            (
                wgpu::TextureFormat::Rgba8Unorm,
                Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            )
        } else {
            (wgpu::TextureFormat::Rgba8Unorm, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mip_level_calculation() {
        // 1x1 should have 1 mip level
        assert_eq!(Texture::calculate_mip_levels(1, 1), 1);

        // 2x2 should have 2 mip levels (2x2, 1x1)
        assert_eq!(Texture::calculate_mip_levels(2, 2), 2);

        // 4x4 should have 3 mip levels (4x4, 2x2, 1x1)
        assert_eq!(Texture::calculate_mip_levels(4, 4), 3);

        // 8x8 should have 4 mip levels
        assert_eq!(Texture::calculate_mip_levels(8, 8), 4);

        // 16x16 should have 5 mip levels
        assert_eq!(Texture::calculate_mip_levels(16, 16), 5);

        // 256x256 should have 9 mip levels
        assert_eq!(Texture::calculate_mip_levels(256, 256), 9);

        // 512x512 should have 10 mip levels
        assert_eq!(Texture::calculate_mip_levels(512, 512), 10);

        // 1024x1024 should have 11 mip levels
        assert_eq!(Texture::calculate_mip_levels(1024, 1024), 11);

        // 2048x2048 should have 12 mip levels
        assert_eq!(Texture::calculate_mip_levels(2048, 2048), 12);

        // Non-square textures use the larger dimension
        assert_eq!(Texture::calculate_mip_levels(256, 128), 9);
        assert_eq!(Texture::calculate_mip_levels(128, 256), 9);
        assert_eq!(Texture::calculate_mip_levels(1024, 512), 11);
    }

    #[test]
    fn test_mip_levels_power_of_two() {
        // Common texture sizes that are powers of 2
        for power in 0..12 {
            let size = 2u32.pow(power);
            let expected_mips = power + 1;
            assert_eq!(
                Texture::calculate_mip_levels(size, size),
                expected_mips,
                "2^{} = {} should have {} mip levels",
                power,
                size,
                expected_mips
            );
        }
    }

    #[test]
    fn test_mip_levels_non_power_of_two() {
        // NPOT textures should still work
        assert_eq!(Texture::calculate_mip_levels(100, 100), 7); // log2(100) ≈ 6.64, floor + 1 = 7
        assert_eq!(Texture::calculate_mip_levels(300, 200), 9); // log2(300) ≈ 8.22, floor + 1 = 9
        assert_eq!(Texture::calculate_mip_levels(1920, 1080), 11); // log2(1920) ≈ 10.90
    }

    #[test]
    fn srgb_textures_use_renderable_storage_format() {
        let (storage, view) = Texture::formats_for_color_space(true);
        assert_eq!(storage, wgpu::TextureFormat::Rgba8Unorm);
        assert_eq!(view, Some(wgpu::TextureFormat::Rgba8UnormSrgb));

        let (storage_linear, view_linear) = Texture::formats_for_color_space(false);
        assert_eq!(storage_linear, wgpu::TextureFormat::Rgba8Unorm);
        assert_eq!(view_linear, None);
    }

    // This test requires a GPU - run with `cargo test --features gpu-tests` or similar
    #[test]
    #[ignore] // Ignore by default since it requires GPU
    fn test_texture_creation_with_mipmaps() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::default(),
                    compatible_surface: None,
                    force_fallback_adapter: false,
                })
                .await
                .expect("Failed to find adapter");

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .expect("Failed to create device");

            // Create a simple 4x4 test texture
            let data = vec![255u8; 4 * 4 * 4]; // 4x4 RGBA
            let source = Texture::rgba_source(
                &data,
                4,
                4,
                wgpu::TextureFormat::Rgba8Unorm,
                None,
                Some("Test Texture"),
            );

            let texture = Texture::from_rgba8(&device, &queue, source);

            // Verify the texture has the expected number of mip levels
            // We can't directly query mip levels, but we can verify it was created
            assert_eq!(texture.texture.size().width, 4);
            assert_eq!(texture.texture.size().height, 4);
            assert_eq!(texture.texture.mip_level_count(), 3); // 4x4, 2x2, 1x1
        });
    }

    #[test]
    #[ignore]
    fn test_default_textures_no_mipmaps() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .expect("Failed to find adapter");

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .expect("Failed to create device");

            // 1x1 textures should only have 1 mip level
            let white = Texture::white(&device, &queue);
            assert_eq!(white.texture.mip_level_count(), 1);

            let normal = Texture::default_normal(&device, &queue);
            assert_eq!(normal.texture.mip_level_count(), 1);

            let mr = Texture::default_metallic_roughness(&device, &queue);
            assert_eq!(mr.texture.mip_level_count(), 1);
        });
    }

    #[test]
    #[ignore]
    fn test_larger_texture_has_more_mips() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                ..Default::default()
            });

            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions::default())
                .await
                .unwrap();

            let (device, queue) = adapter
                .request_device(&wgpu::DeviceDescriptor::default())
                .await
                .unwrap();

            // Create textures of different sizes
            let data_4x4 = vec![255u8; 4 * 4 * 4];
            let tex_4x4 = Texture::from_rgba8(
                &device,
                &queue,
                Texture::rgba_source(
                    &data_4x4,
                    4,
                    4,
                    wgpu::TextureFormat::Rgba8Unorm,
                    None,
                    Some("4x4"),
                ),
            );

            let data_256x256 = vec![255u8; 256 * 256 * 4];
            let tex_256x256 = Texture::from_rgba8(
                &device,
                &queue,
                Texture::rgba_source(
                    &data_256x256,
                    256,
                    256,
                    wgpu::TextureFormat::Rgba8Unorm,
                    None,
                    Some("256x256"),
                ),
            );

            // Verify mip counts
            assert_eq!(tex_4x4.texture.mip_level_count(), 3); // 4, 2, 1
            assert_eq!(tex_256x256.texture.mip_level_count(), 9); // 256, 128, 64, 32, 16, 8, 4, 2, 1
        });
    }
}
