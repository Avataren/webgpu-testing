// renderer/texture.rs
use wgpu::util::DeviceExt;

pub struct Texture {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    size: wgpu::Extent3d,
}

impl Texture {
    /// Create a texture from raw RGBA8 data
    pub fn from_bytes(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        bytes: &[u8],
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
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
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
            bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self { texture, view, size }
    }

    /// Create a simple 2x2 checkerboard texture for testing
    pub fn checkerboard(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let white = [255u8, 255, 255, 255];
        let black = [0u8, 0, 0, 255];
        
        let pixels = [
            white, black,
            black, white,
        ];
        
        let bytes: Vec<u8> = pixels.iter().flat_map(|p| p.iter()).copied().collect();
        Self::from_bytes(device, queue, &bytes, 2, 2, Some("Checkerboard"))
    }

    /// Create a solid color texture
    pub fn solid_color(device: &wgpu::Device, queue: &wgpu::Queue, color: [u8; 4]) -> Self {
        let bytes = color.to_vec();
        Self::from_bytes(device, queue, &bytes, 1, 1, Some("SolidColor"))
    }

    /// Get the texture view for binding
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Get the underlying wgpu texture
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// Get texture dimensions
    pub fn size(&self) -> wgpu::Extent3d {
        self.size
    }
}

/// Texture array manager for bindless rendering
pub struct TextureArray {
    textures: Vec<Texture>,
    bind_group: Option<wgpu::BindGroup>,
    bind_group_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl TextureArray {
    pub fn new(device: &wgpu::Device) -> Self {
        // Create a sampler for all textures
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("TextureArraySampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Create bind group layout for texture array
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("TextureArrayBindGroupLayout"),
            entries: &[
                // Texture array
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None, // For true bindless, you'd use Some(NonZeroU32::new(max_textures))
                },
                // Sampler
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        Self {
            textures: Vec::new(),
            bind_group: None,
            bind_group_layout,
            sampler,
        }
    }

    /// Add a texture and return its index (1-based, 0 means no texture)
    pub fn add(&mut self, texture: Texture) -> u32 {
        self.textures.push(texture);
        self.bind_group = None; // Invalidate bind group
        self.textures.len() as u32
    }

    /// Rebuild the bind group (call after adding textures)
    pub fn rebuild_bind_group(&mut self, device: &wgpu::Device) {
        if self.textures.is_empty() {
            return;
        }

        let texture_views: Vec<_> = self.textures.iter().map(|t| t.view()).collect();

        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("TextureArrayBindGroup"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(texture_views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        }));
    }

    pub fn bind_group(&self) -> Option<&wgpu::BindGroup> {
        self.bind_group.as_ref()
    }

    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bind_group_layout
    }

    pub fn len(&self) -> usize {
        self.textures.len()
    }

    pub fn is_empty(&self) -> bool {
        self.textures.is_empty()
    }
}