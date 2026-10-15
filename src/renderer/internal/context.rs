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
    /// Whether the device has hardware ray queries, or why not.
    pub(crate) ray_tracing: crate::renderer::raytracing::RayTracingSupport,
    pub(crate) sample_count: u32,
    /// How frames reach the display (SDR, or one of the HDR modes).
    pub(crate) hdr_display: crate::renderer::hdr_output::HdrDisplay,
    /// The present mode used without vsync (the configured one, unless that waits for
    /// the display): Fifo when the surface offers nothing else.
    pub(crate) unsynced_present_mode: wgpu::PresentMode,

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

        // WGPU_CUBE_FORCE_TRADITIONAL_TEXTURES=1 runs the per-material texture path that
        // adapters without bindless textures (many browsers) use, for testing it natively.
        let force_traditional = std::env::var("WGPU_CUBE_FORCE_TRADITIONAL_TEXTURES")
            .is_ok_and(|value| !matches!(value.as_str(), "" | "0" | "false"));

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

        // Per-stage GPU timings (engine_stats).
        if adapter_features.contains(crate::renderer::gpu_timer::GPU_TIMER_FEATURES) {
            required_features |= crate::renderer::gpu_timer::GPU_TIMER_FEATURES;
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

        let mut ray_tracing = crate::renderer::raytracing::RayTracingSupport::probe(&adapter);
        let (device, queue) = if ray_tracing.is_available() {
            let mut rt_limits = limits
                .clone()
                .using_acceleration_structure_values(adapter.limits());
            // The ray-traced reflection pass binds the texture array as a fifth group.
            rt_limits.max_bind_groups = rt_limits
                .max_bind_groups
                .max(adapter.limits().max_bind_groups.min(5));
            match adapter
                .request_device(&wgpu::DeviceDescriptor {
                    label: Some("Device"),
                    required_features: required_features
                        | crate::renderer::raytracing::RAY_TRACING_FEATURES,
                    required_limits: rt_limits,
                    // SAFETY: ray queries are experimental in wgpu. The engine only uses
                    // them through the validated wgpu API, and falls back to screen-space
                    // reflections when anything about them fails.
                    experimental_features: unsafe { wgpu::ExperimentalFeatures::enabled() },
                    memory_hints: wgpu::MemoryHints::Performance,
                    trace: wgpu::Trace::Off,
                })
                .await
            {
                Ok(pair) => {
                    log::info!("Hardware ray tracing enabled (ray queries)");
                    pair
                }
                Err(error) => {
                    log::warn!(
                        "Device with ray queries failed ({error}); retrying without ray tracing"
                    );
                    ray_tracing = crate::renderer::raytracing::RayTracingSupport::Unavailable(
                        format!("device request failed: {error}"),
                    );
                    Self::request_plain_device(&adapter, required_features, limits).await
                }
            }
        } else {
            log::info!("Hardware ray tracing unavailable: {ray_tracing}");
            Self::request_plain_device(&adapter, required_features, limits).await
        };

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
        let unsynced_present_mode = match present_mode {
            wgpu::PresentMode::Mailbox
            | wgpu::PresentMode::Immediate
            | wgpu::PresentMode::AutoNoVsync => present_mode,
            _ => [wgpu::PresentMode::Mailbox, wgpu::PresentMode::Immediate]
                .into_iter()
                .find(|mode| surface_caps.present_modes.contains(mode))
                .unwrap_or(wgpu::PresentMode::Fifo),
        };

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
            ray_tracing,
            sample_count,
            hdr_display,
            unsynced_present_mode,
        }
    }

    /// Wait for the display's refresh (Fifo), or present as soon as a frame is ready.
    pub(crate) fn set_vsync(&mut self, vsync: bool) {
        let mode = if vsync {
            wgpu::PresentMode::Fifo
        } else {
            self.unsynced_present_mode
        };
        if self.config.present_mode != mode {
            self.config.present_mode = mode;
            self.surface.configure(&self.device, &self.config);
        }
    }

    async fn request_plain_device(
        adapter: &wgpu::Adapter,
        required_features: wgpu::Features,
        required_limits: wgpu::Limits,
    ) -> (wgpu::Device, wgpu::Queue) {
        adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Device"),
                required_features,
                required_limits,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("Failed to create device")
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
