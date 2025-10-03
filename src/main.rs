use glam::Vec3;
use std::mem;
use std::num::NonZeroU64;
use wgpu::util::DeviceExt;
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::*,
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{Key, NamedKey},
    window::{Window, WindowId},
};

mod renderer;
use renderer::{cube_mesh, CameraUniform, Depth, Vertex};

mod scene;
use scene::Camera;

struct Gpu {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,
    size: PhysicalSize<u32>,
    depth: Depth,

    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,

    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    index_count: u32,

    camera_buf: wgpu::Buffer,
}

impl Gpu {
    async fn new(window: &Window) -> Self {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
        let surface = unsafe {
            instance
                .create_surface_unsafe(
                    wgpu::SurfaceTargetUnsafe::from_window(window).expect("surface target"),
                )
                .expect("surface")
        };
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .expect("adapter");

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            })
            .await
            .expect("device");

        let surface_caps = surface.get_capabilities(&adapter);
        let format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        // Prefer PreMultiplied if available; otherwise fall back to first.
        let alpha_mode = surface_caps
            .alpha_modes
            .iter()
            .copied()
            .find(|m| *m == wgpu::CompositeAlphaMode::PreMultiplied)
            .unwrap_or(surface_caps.alpha_modes[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::Fifo, // safest cross-platform choice
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let depth = Depth::new(&device, size);

        let (verts, idx) = cube_mesh();
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("VertexBuffer"),
            contents: bytemuck::cast_slice(&verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let ibuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("IndexBuffer"),
            contents: bytemuck::cast_slice(&idx),
            usage: wgpu::BufferUsages::INDEX,
        });

        let camera = CameraUniform::new();
        let camera_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("CameraBuffer"),
            contents: bytemuck::bytes_of(&camera),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("BindLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX, // restrict to vertex for tighter validation
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(
                        NonZeroU64::new(mem::size_of::<CameraUniform>() as u64).unwrap(),
                    ),
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("BindGroup"),
            layout: &bind_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_buf.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("PipelineLayout"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::layout()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: config.format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                front_face: wgpu::FrontFace::Ccw,
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
                strip_index_format: None,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth.format,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            surface,
            device,
            queue,
            config,
            size,
            depth,
            pipeline,
            bind_group,
            vbuf,
            ibuf,
            index_count: idx.len() as u32,
            camera_buf,
        }
    }

    fn resize(&mut self, new_size: PhysicalSize<u32>) {
        if new_size.width == 0 || new_size.height == 0 {
            return;
        }
        self.size = new_size;
        self.config.width = new_size.width;
        self.config.height = new_size.height;
        self.surface.configure(&self.device, &self.config);
        self.depth = Depth::new(&self.device, new_size);
    }

    fn set_view_proj(&self, vp: glam::Mat4) {
        let uni = renderer::CameraUniform {
            view_proj: vp.to_cols_array_2d(),
        };
        self.queue
            .write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(&uni));
    }

    // fn update_camera(&self, angle: f32) {
    //     let eye = Vec3::new(angle.cos() * 2.0, 1.2, angle.sin() * 2.0);
    //     let target = Vec3::ZERO;
    //     let up = Vec3::Y;
    //     let view = Mat4::look_at_rh(eye, target, up);

    //     let aspect = self.config.width as f32 / self.config.height.max(1) as f32;
    //     let proj = Mat4::perspective_rh(f32::to_radians(60.0), aspect, 0.01, 100.0);

    //     let vp = proj * view;
    //     let uni = CameraUniform {
    //         view_proj: vp.to_cols_array_2d(),
    //     };
    //     self.queue
    //         .write_buffer(&self.camera_buf, 0, bytemuck::bytes_of(&uni));
    // }

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let frame = self.surface.get_current_texture()?;
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Encoder"),
            });

        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("RenderPass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.07,
                            b: 0.10,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth.view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard, // don't preserve depth between frames
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            rpass.set_pipeline(&self.pipeline);
            rpass.set_bind_group(0, &self.bind_group, &[]);
            rpass.set_vertex_buffer(0, self.vbuf.slice(..));
            rpass.set_index_buffer(self.ibuf.slice(..), wgpu::IndexFormat::Uint16);
            rpass.draw_indexed(0..self.index_count, 0, 0..1);
        }

        self.queue.submit(Some(encoder.finish()));
        frame.present();
        Ok(())
    }
}

// Put `gpu` before `window` so the surface drops before the window (important for some platforms).
struct App {
    gpu: Option<Gpu>,
    window: Option<Window>,
    window_id: Option<WindowId>,
    time: f32,
    last_frame: std::time::Instant,
}

impl App {
    fn new() -> Self {
        Self {
            gpu: None,
            window: None,
            window_id: None,
            time: 0.0,
            last_frame: std::time::Instant::now(),
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window = event_loop
                .create_window(Window::default_attributes().with_title("wgpu cube"))
                .expect("create window");
            let id = window.id();

            let gpu = pollster::block_on(Gpu::new(&window));
            self.window = Some(window);
            self.window_id = Some(id);
            self.gpu = Some(gpu);

            // Kick off the first frame explicitly on platforms that don't auto-redraw
            if let Some(w) = &self.window {
                w.request_redraw();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if Some(id) != self.window_id {
            return;
        }
        let gpu = match self.gpu.as_mut() {
            Some(g) => g,
            None => return,
        };

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }
            WindowEvent::Destroyed => {
                event_loop.exit();
            }
            WindowEvent::Resized(size) => {
                gpu.resize(size);
            }
            WindowEvent::ScaleFactorChanged {
                scale_factor: _, ..
            } => {
                // On DPI change, reconfigure the surface using the window's current inner size.
                if let Some(w) = &self.window {
                    gpu.resize(w.inner_size());
                }
            }
            WindowEvent::RedrawRequested => {
                // Variable timestep based on real time
                let now = std::time::Instant::now();
                let dt = (now - self.last_frame).as_secs_f32();
                self.last_frame = now;

                // Keep time bounded to avoid precision loss
                self.time = (self.time + dt) % std::f32::consts::TAU;

                //gpu.update_camera(self.time);
                let aspect = gpu.config.width as f32 / gpu.config.height.max(1) as f32;

                // keep the same orbiting behavior
                let eye = Vec3::new(self.time.cos() * 2.0, 1.2, self.time.sin() * 2.0);
                let cam = Camera {
                    eye,
                    target: Vec3::ZERO,
                    up: Vec3::Y,
                    fov_y_radians: 60f32.to_radians(),
                    near: 0.01,
                    far: 100.0,
                };

                let vp = cam.view_proj(aspect);
                gpu.set_view_proj(vp);

                match gpu.render() {
                    Ok(()) => {}
                    Err(wgpu::SurfaceError::Lost) => {
                        if let Some(w) = &self.window {
                            gpu.resize(w.inner_size());
                        }
                    }
                    Err(wgpu::SurfaceError::OutOfMemory) => {
                        // Fatal: exit the loop
                        event_loop.exit();
                    }
                    Err(wgpu::SurfaceError::Timeout) => {
                        // Non-fatal: skip this frame
                    }
                    Err(_) => {}
                }
                if let Some(w) = &self.window {
                    w.request_redraw();
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key: Key::Named(NamedKey::Escape),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                // ESC to quit
                event_loop.exit();
            }
            _ => {}
        }
    }
}

fn main() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App::new();
    event_loop.run_app(&mut app).unwrap();
}
