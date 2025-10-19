// egui_integration.rs — fixed for egui release-0.33.0 + wgpu 0.27 'static pass

use egui_wgpu::ScreenDescriptor;
use winit::event::{DeviceEvent, WindowEvent};
use winit::window::{CursorGrabMode, Window};

pub use egui;

pub type EguiUiCallback = Box<dyn FnMut(&egui::Context) + 'static>;

pub struct EguiContext {
    ctx: egui::Context,
    state: egui_winit::State,
    pub renderer: egui_wgpu::Renderer,
    ui_callback: Option<EguiUiCallback>,
    pointer_locked: bool,
    pending_mouse_motion: egui::Vec2,
    should_close: bool,
}

pub struct EguiRenderTarget<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub encoder: &'a mut wgpu::CommandEncoder,
    pub window: &'a Window,
    pub view: &'a wgpu::TextureView,
    pub surface_size: [u32; 2],
}

impl EguiContext {
    pub fn new(
        device: &wgpu::Device,
        output_format: wgpu::TextureFormat,
        _sample_count: u32,
        window: &Window,
    ) -> Self {
        let ctx = egui::Context::default();
        let viewport_id = ctx.viewport_id();

        // egui-winit 0.33
        let state = egui_winit::State::new(
            ctx.clone(),
            viewport_id,
            window,
            Some(window.scale_factor() as f32),
            None,       // theme
            Some(2048), // max_texture_side
        );

        // egui-wgpu 0.33
        let renderer = egui_wgpu::Renderer::new(
            device,
            output_format,
            egui_wgpu::RendererOptions {
                depth_stencil_format: None,
                // egui overlays resolve directly into the surface, which is always single-sampled.
                // Using the scene's MSAA sample count here would make the pipeline incompatible
                // with the surface view when MSAA > 1, triggering validation errors.
                msaa_samples: 1,
                dithering: true,
                predictable_texture_filtering: false,
            },
        );

        Self {
            ctx,
            state,
            renderer,
            ui_callback: None,
            pointer_locked: false,
            pending_mouse_motion: egui::Vec2::ZERO,
            should_close: false,
        }
    }

    pub fn set_ui<F>(&mut self, callback: F)
    where
        F: FnMut(&egui::Context) + 'static,
    {
        self.ui_callback = Some(Box::new(callback));
    }

    pub fn set_ui_box(&mut self, callback: EguiUiCallback) {
        self.ui_callback = Some(callback);
    }

    pub fn handle_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        let response = self.state.on_window_event(window, event);
        response.consumed
    }

    pub fn begin_frame(&mut self, window: &Window) {
        let mut raw_input = self.state.take_egui_input(window);
        if self.pointer_locked {
            // When the cursor is locked we only want to feed relative motion into egui.
            // `egui_winit` still receives `WindowEvent::CursorMoved` events which carry
            // the last absolute cursor position before the grab. If we forward those
            // alongside the raw `MouseMoved` deltas we inject below, egui ends up
            // producing inconsistent pointer deltas which show up as jerky camera
            // motion.  Drop the absolute pointer updates while locked so we rely solely
            // on the accumulated device motion.
            raw_input
                .events
                .retain(|event| !matches!(event, egui::Event::PointerMoved(_)));

            if self.pending_mouse_motion != egui::Vec2::ZERO {
                raw_input
                    .events
                    .push(egui::Event::MouseMoved(self.pending_mouse_motion));
            }
        }
        self.pending_mouse_motion = egui::Vec2::ZERO;
        self.ctx.begin_pass(raw_input);
    }

    pub fn end_frame(&mut self, window: &Window) -> egui::FullOutput {
        let output = self.ctx.end_pass();
        self.state
            .handle_platform_output(window, output.platform_output.clone());
        output
    }

    pub fn render(&mut self, target: &mut EguiRenderTarget<'_>, output: egui::FullOutput) {
        self.handle_viewport_commands(target.window, &output.viewport_output);

        if target.surface_size[0] == 0 || target.surface_size[1] == 0 {
            return;
        }

        let screen_descriptor = ScreenDescriptor {
            size_in_pixels: target.surface_size,
            pixels_per_point: target.window.scale_factor() as f32,
        };

        // Upload textures
        for (id, delta) in &output.textures_delta.set {
            self.renderer
                .update_texture(target.device, target.queue, *id, delta);
        }

        // Tessellate UI shapes
        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);

        // Update GPU buffers
        self.renderer.update_buffers(
            target.device,
            target.queue,
            target.encoder,
            &primitives,
            &screen_descriptor,
        );

        // Begin render pass that LOADs the swapchain view
        let pass = target
            .encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui_render_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target.view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });

        // Convert to 'static (consumes `pass`)
        let mut pass_static = pass.forget_lifetime();

        // Render egui with the 'static pass
        self.renderer
            .render(&mut pass_static, &primitives, &screen_descriptor);

        // No `drop(pass)` here — it was moved by `forget_lifetime()` and will end when `pass_static` drops.

        // Free any textures egui wants to drop
        for id in &output.textures_delta.free {
            self.renderer.free_texture(id);
        }
    }

    pub fn run_ui(&mut self) {
        if let Some(callback) = &mut self.ui_callback {
            callback(&self.ctx);
        }
    }

    pub fn context(&self) -> &egui::Context {
        &self.ctx
    }

    fn handle_viewport_commands(
        &mut self,
        window: &Window,
        viewports: &egui::OrderedViewportIdMap<egui::ViewportOutput>,
    ) {
        if let Some(output) = viewports.get(&self.ctx.viewport_id()) {
            // egui emits `CursorVisible(false)` immediately before requesting a pointer grab.
            // If the grab fails (e.g. unsupported platform, unfocused window), make sure we
            // re-enable the cursor so users are not left without a pointer.
            let mut cursor_hidden_for_grab = false;

            for command in &output.commands {
                match command {
                    egui::ViewportCommand::Close => {
                        self.should_close = true;
                    }
                    egui::ViewportCommand::CancelClose => {
                        self.should_close = false;
                    }
                    egui::ViewportCommand::CursorVisible(visible) => {
                        window.set_cursor_visible(*visible);
                        cursor_hidden_for_grab = !*visible;
                    }
                    egui::ViewportCommand::CursorGrab(grab) => {
                        let mode = match grab {
                            egui::viewport::CursorGrab::None => CursorGrabMode::None,
                            egui::viewport::CursorGrab::Confined => CursorGrabMode::Confined,
                            egui::viewport::CursorGrab::Locked => CursorGrabMode::Locked,
                        };

                        match window.set_cursor_grab(mode) {
                            Ok(()) => {
                                self.pointer_locked =
                                    matches!(grab, egui::viewport::CursorGrab::Locked);
                            }
                            Err(err) => {
                                log::warn!(
                                    "Failed to apply cursor grab command {:?}: {}",
                                    grab,
                                    err
                                );

                                self.pointer_locked = false;

                                if cursor_hidden_for_grab {
                                    window.set_cursor_visible(true);
                                    cursor_hidden_for_grab = false;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        } else {
            self.pointer_locked = false;
        }
    }

    pub fn take_should_close(&mut self) -> bool {
        let should_close = self.should_close;
        self.should_close = false;
        should_close
    }

    pub fn handle_device_event(&mut self, event: &DeviceEvent) {
        if let DeviceEvent::MouseMotion { delta } = event {
            if self.pointer_locked {
                self.pending_mouse_motion += egui::vec2(delta.0 as f32, delta.1 as f32);
            }
        }
    }
}
