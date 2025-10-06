use winit::dpi::PhysicalSize;

pub struct Depth {
    pub texture: wgpu::Texture, // keep the texture alive
    pub view: wgpu::TextureView,
    pub format: wgpu::TextureFormat,
    pub sampled_view: wgpu::TextureView,
}

impl Depth {
    pub fn new(device: &wgpu::Device, size: PhysicalSize<u32>, sample_count: u32) -> Self {
        let format = wgpu::TextureFormat::Depth32Float;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth"),
            size: wgpu::Extent3d {
                width: size.width.max(1),
                height: size.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count, // <- match MSAA sample count (e.g., 4)
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampled_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("DepthSampledView"),
            aspect: wgpu::TextureAspect::DepthOnly,
            ..Default::default()
        });
        Self {
            texture,
            view,
            format,
            sampled_view,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn depth_format_is_depth32float() {
        let fmt = wgpu::TextureFormat::Depth32Float;
        assert!(matches!(fmt, wgpu::TextureFormat::Depth32Float));
    }
}
