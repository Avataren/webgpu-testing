#![cfg(not(target_arch = "wasm32"))]

use std::sync::Arc;

use winit::{
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    event_loop::ActiveEventLoop,
    monitor::MonitorHandle,
    window::{Window, WindowAttributes},
};

use crate::renderer::Renderer;

use super::{PlatformDriver, PlatformState};

#[derive(Default)]
pub struct NativeWinitDriver;

impl NativeWinitDriver {
    fn window_attributes(
        settings: &crate::settings::RenderSettings,
        event_loop: &ActiveEventLoop,
    ) -> WindowAttributes {
        let requested = LogicalSize::new(
            f64::from(settings.resolution.width),
            f64::from(settings.resolution.height),
        );
        let attributes = Window::default_attributes()
            .with_title("wgpu hecs Renderer")
            .with_inner_size(requested);

        if settings.monitor.is_none() {
            return attributes;
        }

        let monitors: Vec<MonitorHandle> = event_loop.available_monitors().collect();
        let names: Vec<String> = monitors
            .iter()
            .map(|monitor| monitor.name().unwrap_or_default())
            .collect();
        let primary = event_loop
            .primary_monitor()
            .and_then(|primary| monitors.iter().position(|monitor| *monitor == primary));

        let Some(index) = settings.select_monitor(&names, primary) else {
            let available: Vec<String> = monitors
                .iter()
                .zip(&names)
                .enumerate()
                .map(|(index, (monitor, name))| {
                    let size = monitor.size();
                    format!("{} = {name} ({}x{})", index + 1, size.width, size.height)
                })
                .collect();
            log::warn!(
                "Monitor {:?} not found; available: {}",
                settings.monitor.as_deref().unwrap_or_default(),
                available.join(", ")
            );
            return attributes;
        };

        // Center the window on the monitor, shrinking it to fit when it is too big.
        let monitor = &monitors[index];
        let area = monitor.size();
        let origin = monitor.position();
        let wanted: PhysicalSize<u32> = requested.to_physical(monitor.scale_factor());
        let size = PhysicalSize::new(
            wanted.width.min(area.width * 95 / 100).max(1),
            wanted.height.min(area.height * 90 / 100).max(1),
        );
        let position = PhysicalPosition::new(
            origin.x + ((area.width - size.width) / 2) as i32,
            origin.y + ((area.height - size.height) / 2) as i32,
        );
        log::info!(
            "Opening the window on monitor {} ({}) at {}x{}",
            index + 1,
            names[index],
            size.width,
            size.height
        );
        attributes.with_inner_size(size).with_position(position)
    }
}

impl PlatformDriver for NativeWinitDriver {
    type WindowHandle = Arc<Window>;

    fn initialize(
        &mut self,
        state: &mut PlatformState<Self::WindowHandle>,
        event_loop: &ActiveEventLoop,
        settings: &crate::settings::RenderSettings,
    ) {
        let window = event_loop
            .create_window(Self::window_attributes(settings, event_loop))
            .expect("Failed to create window");

        let handle = Arc::new(window);
        let renderer = pollster::block_on(Renderer::new(handle.clone(), settings.clone()));

        state.set_window(handle);
        state.put_renderer(renderer);
    }

    fn handle_event(
        &mut self,
        _state: &mut PlatformState<Self::WindowHandle>,
        _event_loop: &ActiveEventLoop,
        _event: &winit::event::WindowEvent,
    ) {
    }

    fn render_frame(
        &mut self,
        state: &mut PlatformState<Self::WindowHandle>,
        _event_loop: &ActiveEventLoop,
    ) -> bool {
        state.has_renderer()
    }
}
