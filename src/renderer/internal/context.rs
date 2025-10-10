use winit::dpi::PhysicalSize;
use winit::window::Window;

use std::ops::Deref;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Arc;

use winit::raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle,
    WindowHandle as WinitWindowHandle,
};

use crate::renderer::Depth;
use crate::settings::RenderSettings;

pub(crate) struct RenderContext {
    // Drop order: bottom to top (fields declared earlier drop last)
    // Keep instance alive for the lifetime of the surface and drop the surface before the window.
    pub(crate) _instance: wgpu::Instance,
    pub(crate) size: PhysicalSize<u32>,
    pub(crate) config: wgpu::SurfaceConfiguration,
    pub(crate) supports_bindless_textures: bool,
    pub(crate) sample_count: u32,
    // GPU resources (drop before device/queue)
    pub(crate) depth: Depth,
    // Device and queue (drop before surface)
    pub(crate) queue: wgpu::Queue,
    pub(crate) device: wgpu::Device,
    // Surface dropped last
    pub(crate) surface: wgpu::Surface<'static>,
}

#[cfg(not(target_arch = "wasm32"))]
type SharedWindow = Arc<Window>;
#[cfg(target_arch = "wasm32")]
type SharedWindow = Rc<Window>;

#[derive(Clone)]
struct OwnedWindowHandle {
    window: SharedWindow,
}

impl OwnedWindowHandle {
    fn new(window: SharedWindow) -> Self {
        Self { window }
    }
}

impl Deref for OwnedWindowHandle {
    type Target = Window;

    fn deref(&self) -> &Self::Target {
        &self.window
    }
}

impl HasWindowHandle for OwnedWindowHandle {
    fn window_handle(&self) -> Result<WinitWindowHandle<'_>, HandleError> {
        self.window.window_handle()
    }
}

impl HasDisplayHandle for OwnedWindowHandle {
    fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
        self.window.display_handle()
    }
}

impl RenderContext {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) async fn new(
        window: Arc<Window>,
        size: PhysicalSize<u32>,
        settings: &RenderSettings,
    ) -> Self {
        Self::new_internal(OwnedWindowHandle::new(window), size, settings).await
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) async fn new(
        window: Rc<Window>,
        size: PhysicalSize<u32>,
        settings: &RenderSettings,
    ) -> Self {
        Self::new_internal(OwnedWindowHandle::new(window), size, settings).await
    }

    async fn new_internal(
        window_handle: OwnedWindowHandle,
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
            .create_surface(window_handle)
            .expect("Failed to create surface");

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
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let format_features = adapter.get_texture_format_features(format);
        let supported_sample_counts = format_features.flags.supported_sample_counts();
        let requested_samples = settings.sample_count.max(1);
        let mut sample_count =
            Self::choose_supported_sample_count(requested_samples, &supported_sample_counts);
        if sample_count != requested_samples {
            let max_supported = supported_sample_counts.iter().copied().max().unwrap_or(1);
            if requested_samples > max_supported {
                log::warn!(
                    "Requested MSAA sample count {} exceeds format capability (max {}). Using {} instead.",
                    requested_samples,
                    max_supported,
                    sample_count
                );
            } else {
                log::warn!(
                    "MSAA sample count {} is not supported for format {:?}. Using {} instead.",
                    requested_samples,
                    format,
                    sample_count
                );
            }
        }

        if sample_count > 1 && !format_features.flags.sample_count_supported(sample_count) {
            log::warn!(
                "Surface format {:?} reports unsupported sample count {}. Falling back to 1 sample.",
                format,
                sample_count
            );
            sample_count = 1;
        }

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

        let depth = Depth::new(&device, size, sample_count);

        Self {
            _instance: instance,
            surface,
            device,
            queue,
            config,
            size,
            depth,
            supports_bindless_textures,
            sample_count,
        }
    }

    fn choose_supported_sample_count(requested: u32, supported: &[u32]) -> u32 {
        supported
            .iter()
            .copied()
            .filter(|&count| count <= requested)
            .max()
            .or_else(|| supported.iter().copied().min())
            .unwrap_or(1)
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

#[cfg(test)]
mod tests {
    use super::RenderContext;

    #[test]
    fn choose_supported_sample_count_prefers_highest_not_exceeding_request() {
        let supported = [1, 2, 4, 8];
        assert_eq!(
            RenderContext::choose_supported_sample_count(4, &supported),
            4
        );
        assert_eq!(
            RenderContext::choose_supported_sample_count(3, &supported),
            2
        );
    }

    #[test]
    fn choose_supported_sample_count_handles_empty_and_small_requests() {
        assert_eq!(RenderContext::choose_supported_sample_count(1, &[]), 1);
        let supported = [2, 4, 8];
        assert_eq!(
            RenderContext::choose_supported_sample_count(1, &supported),
            2
        );
        assert_eq!(
            RenderContext::choose_supported_sample_count(16, &supported),
            8
        );
    }
}
