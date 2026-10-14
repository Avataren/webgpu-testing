use winit::dpi::PhysicalSize;
use winit::window::Window;

use std::ops::Deref;
use std::sync::Arc;

use winit::raw_window_handle::{
    DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle,
    WindowHandle as WinitWindowHandle,
};

use crate::renderer::Depth;
use crate::settings::RenderSettings;

pub(crate) struct RenderContext {
    // CRITICAL: Fields drop in declaration order (top to bottom).
    // GPU resources must drop before Device/Queue, which must drop before Surface, which must drop before Instance.

    // Configuration and state (no Drop impl)
    pub(crate) size: PhysicalSize<u32>,
    pub(crate) config: wgpu::SurfaceConfiguration,
    pub(crate) scene_texture_format: wgpu::TextureFormat,
    pub(crate) supports_bindless_textures: bool,
    pub(crate) supports_wireframe: bool,
    pub(crate) sample_count: u32,
    /// How frames reach the display (SDR, or one of the HDR modes).
    pub(crate) hdr_display: crate::renderer::hdr_output::HdrDisplay,

    // GPU resources (must drop first)
    pub(crate) depth: Depth,

    // Queue and Device (must drop before Surface)
    pub(crate) queue: wgpu::Queue,
    pub(crate) device: wgpu::Device,

    // Surface (must drop before Instance)
    pub(crate) surface: wgpu::Surface<'static>,

    // Instance (must drop last - it created the Surface)
    pub(crate) _instance: wgpu::Instance,
}

#[derive(Clone)]
struct OwnedWindowHandle {
    window: Arc<Window>,
}

impl OwnedWindowHandle {
    fn new(window: Arc<Window>) -> Self {
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
    pub(crate) async fn new(
        window: Arc<Window>,
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

        let mut descriptor = wgpu::InstanceDescriptor::new_without_display_handle();
        descriptor.backends = backends;
        // The GL backend needs the display connection (e.g. on Wayland).
        #[cfg(not(target_arch = "wasm32"))]
        let descriptor = descriptor.with_display_handle(Box::new(window_handle.window.clone()));
        let instance = wgpu::Instance::new(descriptor);
        let surface = instance
            .create_surface(window_handle)
            .expect("Failed to create surface");

        log::info!("Surface created successfully!");

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                ..Default::default()
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

        let supports_wireframe = if adapter_features.contains(wgpu::Features::POLYGON_MODE_LINE) {
            required_features |= wgpu::Features::POLYGON_MODE_LINE;
            log::info!("Wireframe rendering enabled");
            true
        } else {
            log::warn!("Wireframe rendering not supported on this adapter");
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

        let sdr_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);
        let (format, color_space, hdr_display) =
            crate::renderer::hdr_output::choose_surface(settings, &surface_caps, sdr_format);

        let scene_texture_format = Self::scene_texture_format_for(format);

        // MSAA renders the HDR scene, so its sample count must suit the scene format.
        let format_features = adapter.get_texture_format_features(scene_texture_format);
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
                    scene_texture_format,
                    sample_count
                );
            }
        }

        if sample_count > 1 && !format_features.flags.sample_count_supported(sample_count) {
            log::warn!(
                "Scene format {:?} reports unsupported sample count {}. Falling back to 1 sample.",
                scene_texture_format,
                sample_count
            );
            sample_count = 1;
        }

        let present_mode = settings.present_mode(&surface_caps.present_modes);

        // COPY_SRC lets remote control read frames back for screenshots.
        let usage = if surface_caps.usages.contains(wgpu::TextureUsages::COPY_SRC) {
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC
        } else {
            wgpu::TextureUsages::RENDER_ATTACHMENT
        };

        let config = wgpu::SurfaceConfiguration {
            usage,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode,
            alpha_mode: surface_caps.alpha_modes[0],
            color_space,
            // Allow a linear view of the sRGB surface for UI: egui does its own gamma
            // handling and looks washed out when rendered through an sRGB view.
            view_formats: if format.is_srgb() {
                vec![format.remove_srgb_suffix()]
            } else {
                vec![]
            },
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
            scene_texture_format,
            size,
            depth,
            supports_bindless_textures,
            supports_wireframe,
            sample_count,
            hdr_display,
        }
    }

    pub(crate) fn scene_texture_format(&self) -> wgpu::TextureFormat {
        self.scene_texture_format
    }

    /// The lit scene renders in HDR whatever the surface format; post-processing
    /// tonemaps it for the display.
    fn scene_texture_format_for(_surface_format: wgpu::TextureFormat) -> wgpu::TextureFormat {
        crate::renderer::postprocess::HDR_FORMAT
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
