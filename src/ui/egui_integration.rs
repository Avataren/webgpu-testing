// egui_integration.rs — fixed for egui release-0.33.0 + wgpu 0.27 'static pass

use egui_wgpu::ScreenDescriptor;
use winit::event::WindowEvent;
use winit::window::Window;

pub use egui;

pub type EguiUiCallback = Box<dyn FnMut(&egui::Context) + 'static>;

pub struct EguiContext {
    ctx: egui::Context,
    state: egui_winit::State,
    pub renderer: egui_wgpu::Renderer,
    ui_callback: Option<EguiUiCallback>,
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
        let raw_input = self.state.take_egui_input(window);
        self.ctx.begin_pass(raw_input);
    }

    pub fn end_frame(&mut self, window: &Window) -> egui::FullOutput {
        let output = self.ctx.end_pass();
        self.state
            .handle_platform_output(window, output.platform_output.clone());
        output
    }

    pub fn render(&mut self, target: &mut EguiRenderTarget<'_>, output: egui::FullOutput) {
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
}
