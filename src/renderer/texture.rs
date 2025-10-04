// renderer/texture.rs

#[derive(Debug)]
pub struct Texture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
}

impl Texture {
    /// Create a solid color 1x1 texture
    pub fn from_color(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        color: [u8; 4],
        label: Option<&str>,
    ) -> Self {
        let size = wgpu::Extent3d {
            width: 1,
            height: 1,
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
            &color,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self {
            texture,
            view,
            sampler,
        }
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
        
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        Self {
            texture,
            view,
            sampler,
        }
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
                let is_color1 = (checker_x + checker_y) % 2 == 0;
                
                let color = if is_color1 { color1 } else { color2 };
                let idx = ((y * size + x) * 4) as usize;
                pixels[idx..idx + 4].copy_from_slice(&color);
            }
        }
        
        Self::from_bytes(device, queue, &pixels, size, size, label)
    }

    /// Create a gradient texture
    pub fn gradient(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: u32,
        color1: [u8; 4],
        color2: [u8; 4],
        label: Option<&str>,
    ) -> Self {
        let mut pixels = vec![0u8; (size * size * 4) as usize];
        
        for y in 0..size {
            for x in 0..size {
                let t = x as f32 / size as f32;
                let r = (color1[0] as f32 * (1.0 - t) + color2[0] as f32 * t) as u8;
                let g = (color1[1] as f32 * (1.0 - t) + color2[1] as f32 * t) as u8;
                let b = (color1[2] as f32 * (1.0 - t) + color2[2] as f32 * t) as u8;
                let a = (color1[3] as f32 * (1.0 - t) + color2[3] as f32 * t) as u8;
                
                let idx = ((y * size + x) * 4) as usize;
                pixels[idx..idx + 4].copy_from_slice(&[r, g, b, a]);
            }
        }
        
        Self::from_bytes(device, queue, &pixels, size, size, label)
    }

    /// Create a radial gradient texture
    pub fn radial(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: u32,
        center_color: [u8; 4],
        edge_color: [u8; 4],
        label: Option<&str>,
    ) -> Self {
        let mut pixels = vec![0u8; (size * size * 4) as usize];
        let center = size as f32 / 2.0;
        let max_dist = center * 1.414; // diagonal distance
        
        for y in 0..size {
            for x in 0..size {
                let dx = x as f32 - center;
                let dy = y as f32 - center;
                let dist = (dx * dx + dy * dy).sqrt();
                let t = (dist / max_dist).min(1.0);
                
                let r = (center_color[0] as f32 * (1.0 - t) + edge_color[0] as f32 * t) as u8;
                let g = (center_color[1] as f32 * (1.0 - t) + edge_color[1] as f32 * t) as u8;
                let b = (center_color[2] as f32 * (1.0 - t) + edge_color[2] as f32 * t) as u8;
                let a = (center_color[3] as f32 * (1.0 - t) + edge_color[3] as f32 * t) as u8;
                
                let idx = ((y * size + x) * 4) as usize;
                pixels[idx..idx + 4].copy_from_slice(&[r, g, b, a]);
            }
        }
        
        Self::from_bytes(device, queue, &pixels, size, size, label)
    }

    /// Create a noise texture (simple value noise)
    pub fn noise(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        size: u32,
        seed: u32,
        label: Option<&str>,
    ) -> Self {
        let mut pixels = vec![0u8; (size * size * 4) as usize];
        
        // Simple hash-based noise
        let hash = |x: u32, y: u32| -> u8 {
            let mut h = seed;
            h = h.wrapping_mul(374761393).wrapping_add(x);
            h = h.wrapping_mul(668265263).wrapping_add(y);
            h ^= h >> 13;
            h = h.wrapping_mul(1274126177);
            h ^= h >> 16;
            (h & 0xFF) as u8
        };
        
        for y in 0..size {
            for x in 0..size {
                let value = hash(x, y);
                let idx = ((y * size + x) * 4) as usize;
                pixels[idx..idx + 4].copy_from_slice(&[value, value, value, 255]);
            }
        }
        
        Self::from_bytes(device, queue, &pixels, size, size, label)
    }
}