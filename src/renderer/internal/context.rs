use std::mem;

use winit::dpi::PhysicalSize;
use winit::window::Window;

use crate::renderer::Depth;
use crate::settings::RenderSettings;

pub(crate) struct RenderContext {
    pub(crate) surface: wgpu::Surface<'static>,
    pub(crate) device: wgpu::Device,
    pub(crate) queue: wgpu::Queue,
    pub(crate) config: wgpu::SurfaceConfiguration,
    pub(crate) size: PhysicalSize<u32>,
    pub(crate) depth: Depth,
    pub(crate) supports_bindless_textures: bool,
    pub(crate) sample_count: u32,
}

impl RenderContext {
    pub(crate) async fn new(
        window: &Window,
        size: PhysicalSize<u32>,
        settings: &RenderSettings,
    ) -> Self {
        let backends = if cfg!(target_arch = "wasm32") {
            wgpu::Backends::BROWSER_WEBGPU | wgpu::Backends::GL
        } else {
            wgpu::Backends::all()
        };

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends,
            ..Default::default()
        });
        let surface = instance
            .create_surface(window)
            .expect("Failed to create surface");

        let surface: wgpu::Surface<'static> = unsafe { mem::transmute(surface) };

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

        if adapter_features.contains(wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES) {
            required_features |= wgpu::Features::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES;
        }

        if adapter_features.contains(wgpu::Features::FLOAT32_FILTERABLE) {
            required_features |= wgpu::Features::FLOAT32_FILTERABLE;
        }

        let mut limits = if supports_bindless_textures {
            wgpu::Limits {
                max_binding_array_elements_per_shader_stage: 256,
                ..wgpu::Limits::default()
            }
        } else {
            wgpu::Limits::default()
        };

        limits.max_bind_groups = limits.max_bind_groups.max(4);

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
            .find(|f| !f.is_srgb())
            .unwrap_or_else(|| {
                surface_caps
                    .formats
                    .iter()
                    .copied()
                    .find(|f| f.is_srgb())
                    .unwrap_or(surface_caps.formats[0])
            });

        let present_mode = settings.present_mode(&surface_caps.present_modes);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let depth = Depth::new(&device, size, settings.sample_count);

        Self {
            surface,
            device,
            queue,
            config,
            size,
            depth,
            supports_bindless_textures,
            sample_count: settings.sample_count,
        }
    }

    pub(crate) fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth = Depth::new(&self.device, new_size, self.sample_count);
    }
}
