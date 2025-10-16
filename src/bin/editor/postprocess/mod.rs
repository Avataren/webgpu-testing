use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec3};
use wgpu::util::DeviceExt;
use wgpu_cube::renderer::CustomRenderContext;

const SHADER_SOURCE: &str = include_str!("grid_overlay.wgsl");

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GridUniform {
    view_proj: [[f32; 4]; 4],
    view_proj_inv: [[f32; 4]; 4],
    camera_position: [f32; 4],
    resolution: [f32; 2],
    viewport_offset: [f32; 2],
    viewport_scale: [f32; 2],
    _padding: [f32; 2],
}

impl GridUniform {
    fn new(
        view_proj: Mat4,
        view_proj_inv: Mat4,
        camera_position: Vec3,
        resolution: Vec2,
        viewport_offset: Vec2,
        viewport_scale: Vec2,
    ) -> Self {
        Self {
            view_proj: view_proj.to_cols_array_2d(),
            view_proj_inv: view_proj_inv.to_cols_array_2d(),
            camera_position: [camera_position.x, camera_position.y, camera_position.z, 1.0],
            resolution: [resolution.x, resolution.y],
            viewport_offset: [viewport_offset.x, viewport_offset.y],
            viewport_scale: [viewport_scale.x, viewport_scale.y],
            _padding: [0.0, 0.0],
        }
    }
}

impl Default for GridUniform {
    fn default() -> Self {
        Self::zeroed()
    }
}

pub struct ViewportGrid {
    shader: wgpu::ShaderModule,
    bind_group_layout: wgpu::BindGroupLayout,
    uniform_buffer: wgpu::Buffer,
    pipeline: Option<wgpu::RenderPipeline>,
    target_format: Option<wgpu::TextureFormat>,
    sample_count: Option<u32>,
}

impl ViewportGrid {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("EditorGridShader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("EditorGridBindGroupLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Depth,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("EditorGridUniform"),
            contents: bytemuck::bytes_of(&GridUniform::default()),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        Self {
            shader,
            bind_group_layout,
            uniform_buffer,
            pipeline: None,
            target_format: None,
            sample_count: None,
        }
    }

    pub fn render(&mut self, ctx: &mut CustomRenderContext<'_>) {
        let renderer = ctx.renderer;
        let device = renderer.get_device();
        let queue = renderer.get_queue();
        let color_format = ctx.color_format();
        let sample_count = ctx.sample_count();

        self.ensure_pipeline(device, color_format, sample_count);

        let uniform = self.build_uniform(ctx);
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(&uniform));

        let depth_view = renderer.depth_sample_view();
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("EditorGridBindGroup"),
            layout: &self.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(depth_view),
                },
            ],
        });

        if let Some(pipeline) = &self.pipeline {
            let mut pass = ctx.begin_render_pass("EditorGridPostprocess");
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    fn ensure_pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        sample_count: u32,
    ) {
        if self.target_format == Some(format) && self.sample_count == Some(sample_count) {
            return;
        }

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("EditorGridPipelineLayout"),
            bind_group_layouts: &[&self.bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("EditorGridPipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &self.shader,
                entry_point: "vs_fullscreen",
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &self.shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: sample_count,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
        });

        self.pipeline = Some(pipeline);
        self.target_format = Some(format);
        self.sample_count = Some(sample_count);
    }

    fn build_uniform(&self, ctx: &CustomRenderContext<'_>) -> GridUniform {
        let renderer = ctx.renderer;
        let scene = ctx.scene;
        let camera = scene.camera();

        let surface_size = renderer.surface_size();
        let full_width = surface_size.width.max(1) as f32;
        let full_height = surface_size.height.max(1) as f32;

        let (resolution, offset, scale) = if let Some(region) = ctx.render_region() {
            let width = region.width().max(1) as f32;
            let height = region.height().max(1) as f32;
            let offset = Vec2::new(
                region.x() as f32 / full_width,
                region.y() as f32 / full_height,
            );
            let scale = Vec2::new(width / full_width, height / full_height);
            (Vec2::new(width, height), offset, scale)
        } else {
            (Vec2::new(full_width, full_height), Vec2::ZERO, Vec2::ONE)
        };

        let safe_resolution = Vec2::new(resolution.x.max(1.0), resolution.y.max(1.0));
        let aspect = (safe_resolution.x / safe_resolution.y).max(1e-6);

        let view = camera.view();
        let proj = camera.proj(aspect);
        let view_proj = proj * view;
        let view_proj_inv = view_proj.inverse();
        let camera_position = camera.position();

        GridUniform::new(
            view_proj,
            view_proj_inv,
            camera_position,
            safe_resolution,
            offset,
            scale,
        )
    }
}
