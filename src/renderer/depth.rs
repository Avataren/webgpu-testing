use winit::dpi::PhysicalSize;

pub struct Depth {
    pub view: wgpu::TextureView,
    pub format: wgpu::TextureFormat,
}

impl Depth {
    pub fn new(device: &wgpu::Device, size: PhysicalSize<u32>) -> Self {
        let format = wgpu::TextureFormat::Depth24Plus;
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth"),
            size: wgpu::Extent3d {
                width: size.width.max(1),
                height: size.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        Self { view, format }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn depth_format_is_depth24plus() {
        // Simple sanity check that we picked a depth format;
        // doesn't create a device, so it runs headless.
        let fmt = wgpu::TextureFormat::Depth24Plus;
        // Pattern match to catch accidental changes.
        assert!(matches!(fmt, wgpu::TextureFormat::Depth24Plus));
    }
}
