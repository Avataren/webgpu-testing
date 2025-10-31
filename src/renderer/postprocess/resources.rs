use super::{PickAttachmentViews, BLOOM_FORMAT, GBUFFER_PICK_FORMAT};

pub struct MsaaTarget {
    _texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl MsaaTarget {
    pub fn new(
        device: &wgpu::Device,
        size: &wgpu::Extent3d,
        format: wgpu::TextureFormat,
        sample_count: u32,
        label: &str,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: *size,
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            _texture: texture,
            view,
        }
    }
}

#[derive(Clone)]
pub struct TextureBundle {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
}

impl TextureBundle {
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    pub fn color(
        device: &wgpu::Device,
        size: &wgpu::Extent3d,
        format: wgpu::TextureFormat,
        label: &str,
    ) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: *size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view }
    }

    pub fn depth(device: &wgpu::Device, size: &wgpu::Extent3d, label: &str) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: *size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view }
    }

    pub fn ssao(device: &wgpu::Device, size: &wgpu::Extent3d) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SsaoTexture"),
            size: *size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view }
    }

    pub fn pick(device: &wgpu::Device, size: &wgpu::Extent3d) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("SceneId"),
            size: *size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: GBUFFER_PICK_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture, view }
    }
}

pub struct BloomMip {
    texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    size: wgpu::Extent3d,
}

impl BloomMip {
    pub fn new(device: &wgpu::Device, size: wgpu::Extent3d, label: &str) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: BLOOM_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            texture,
            view,
            size,
        }
    }

    pub fn extent(&self) -> wgpu::Extent3d {
        self.size
    }

    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }
}

#[derive(Default)]
pub struct LazyPickTarget {
    source: Option<TextureBundle>,
    msaa: Option<MsaaTarget>,
    size: Option<wgpu::Extent3d>,
    sample_count: Option<u32>,
}

impl LazyPickTarget {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn invalidate(&mut self) {
        self.source = None;
        self.msaa = None;
        self.size = None;
        self.sample_count = None;
    }

    pub fn ensure(&mut self, device: &wgpu::Device, size: &wgpu::Extent3d, sample_count: u32) {
        let needs_rebuild = self.size.map(|current| current != *size).unwrap_or(true)
            || self.sample_count != Some(sample_count);

        if needs_rebuild {
            self.source = Some(TextureBundle::pick(device, size));
            self.msaa = (sample_count > 1).then(|| {
                MsaaTarget::new(
                    device,
                    size,
                    GBUFFER_PICK_FORMAT,
                    sample_count,
                    "SceneIdMsaa",
                )
            });
            self.size = Some(*size);
            self.sample_count = Some(sample_count);
        }
    }

    pub fn views(&self) -> Option<PickAttachmentViews<'_>> {
        let source = self.source.as_ref()?;
        let views = match self.msaa.as_ref() {
            Some(msaa) => PickAttachmentViews {
                multisample: &msaa.view,
                resolve: Some(&source.view),
            },
            None => PickAttachmentViews {
                multisample: &source.view,
                resolve: None,
            },
        };
        Some(views)
    }

    pub fn texture(&self) -> Option<&wgpu::Texture> {
        self.source.as_ref().map(|bundle| bundle.texture())
    }

    pub fn extent(&self) -> Option<wgpu::Extent3d> {
        self.size
    }
}
