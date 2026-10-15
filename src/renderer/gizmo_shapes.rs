//! Screen-space editor shapes: anti-aliased lines, discs, rings, squares, arrow
//! heads and quads anchored at world positions but sized in pixels.
//!
//! Gizmos built from them keep crisp edges and a constant on-screen weight at any
//! distance, unlike meshes drawn on the single-sample output. Shapes are drawn in
//! the order given, on top of the finished image and without depth, through the
//! unjittered camera.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;

const SHADER_SOURCE: &str = concat!(
    include_str!("../shader/camera.wgsl"),
    include_str!("gizmo_shapes.wgsl"),
);

const KIND_LINE: f32 = 0.0;
const KIND_DISC: f32 = 1.0;
const KIND_SQUARE: f32 = 2.0;
const KIND_ARROW: f32 = 3.0;
const KIND_QUAD: f32 = 4.0;

/// One shape, as the shader reads it: four world points whose `w` carry the kind
/// and size parameters, a fill color, an outline color (linear, straight alpha),
/// how far to grow the shape (`extra.x`, pixels) and whether a line has square ends
/// (`extra.y`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GizmoShape {
    p0: [f32; 4],
    p1: [f32; 4],
    p2: [f32; 4],
    p3: [f32; 4],
    color: [f32; 4],
    outline: [f32; 4],
    extra: [f32; 4],
}

/// An sRGB color with straight alpha, in 0..1.
pub type GizmoColor = [f32; 4];

fn linear(color: GizmoColor) -> [f32; 4] {
    let channel = |c: f32| {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    [
        channel(color[0]),
        channel(color[1]),
        channel(color[2]),
        color[3].clamp(0.0, 1.0),
    ]
}

fn point(p: Vec3, w: f32) -> [f32; 4] {
    [p.x, p.y, p.z, w]
}

impl GizmoShape {
    fn new(points: [Vec3; 4], params: [f32; 4], color: GizmoColor) -> Self {
        Self {
            p0: point(points[0], params[0]),
            p1: point(points[1], params[1]),
            p2: point(points[2], params[2]),
            p3: point(points[3], params[3]),
            color: linear(color),
            outline: [0.0; 4],
            extra: [0.0; 4],
        }
    }

    /// A line `width` pixels wide with round caps.
    pub fn line(a: Vec3, b: Vec3, width: f32, color: GizmoColor) -> Self {
        Self::new([a, b, Vec3::ZERO, Vec3::ZERO], [KIND_LINE, width, 0.0, 0.0], color)
    }

    /// A filled disc of `radius` pixels.
    pub fn disc(center: Vec3, radius: f32, color: GizmoColor) -> Self {
        Self::new(
            [center, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO],
            [KIND_DISC, radius, 0.0, 0.0],
            color,
        )
    }

    /// A circle of `radius` pixels drawn `thickness` pixels wide.
    pub fn ring(center: Vec3, radius: f32, thickness: f32, color: GizmoColor) -> Self {
        Self::new(
            [center, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO],
            [KIND_DISC, radius, thickness.max(1e-3), 0.0],
            color,
        )
    }

    /// A screen-aligned square `half_size` pixels from center to edge, with corners
    /// rounded by `corner_radius` pixels.
    pub fn square(center: Vec3, half_size: f32, corner_radius: f32, color: GizmoColor) -> Self {
        Self::new(
            [center, Vec3::ZERO, Vec3::ZERO, Vec3::ZERO],
            [KIND_SQUARE, half_size, corner_radius, 0.0],
            color,
        )
    }

    /// A filled triangle pointing from `from` towards `tip`, its point on `tip`:
    /// `length` pixels long and `half_width` pixels to each side.
    pub fn arrow_head(from: Vec3, tip: Vec3, length: f32, half_width: f32, color: GizmoColor) -> Self {
        Self::new(
            [from, tip, Vec3::ZERO, Vec3::ZERO],
            [KIND_ARROW, length, half_width, 0.0],
            color,
        )
    }

    /// A filled quad between four world corners, in order around its edge.
    pub fn quad(corners: [Vec3; 4], color: GizmoColor) -> Self {
        Self::new(corners, [KIND_QUAD, 0.0, 0.0, 0.0], color)
    }

    /// End a line flush at its end points instead of with round caps, so the
    /// segments of a polyline meet without overlaps or seams (which show as beads in
    /// translucent lines). The ends are not anti-aliased; growing the line widens
    /// it but keeps its length.
    pub fn butt_caps(mut self) -> Self {
        self.extra[1] = 1.0;
        self
    }

    /// The same shape grown by `pixels` all round (rounding its corners) in another
    /// color: a halo when drawn underneath it.
    pub fn grown(mut self, pixels: f32, color: GizmoColor) -> Self {
        self.extra[0] += pixels;
        self.color = linear(color);
        self.p3[3] = 0.0;
        self.outline = [0.0; 4];
        self
    }

    /// Draw a band `width` pixels wide just inside the edge in `color`. Not for rings.
    pub fn with_outline(mut self, width: f32, color: GizmoColor) -> Self {
        self.p3[3] = width;
        self.outline = linear(color);
        self
    }
}

struct PipelineState {
    pipeline: wgpu::RenderPipeline,
    format: wgpu::TextureFormat,
}

pub(crate) struct GizmoShapeRenderer {
    shapes: Vec<GizmoShape>,
    shader: Option<wgpu::ShaderModule>,
    bind_layout: Option<wgpu::BindGroupLayout>,
    uniform_buffer: Option<wgpu::Buffer>,
    bind_group: Option<wgpu::BindGroup>,
    instance_buffer: Option<wgpu::Buffer>,
    pipeline: Option<PipelineState>,
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShapeUniform {
    viewport_size: [f32; 2],
    _padding: [f32; 2],
}

impl GizmoShapeRenderer {
    pub(crate) fn new() -> Self {
        Self {
            shapes: Vec::new(),
            shader: None,
            bind_layout: None,
            uniform_buffer: None,
            bind_group: None,
            instance_buffer: None,
            pipeline: None,
        }
    }

    /// Replace the shapes drawn with the next frame.
    pub(crate) fn set_shapes(&mut self, shapes: Vec<GizmoShape>) {
        self.shapes = shapes;
    }

    /// Draw (and forget) the queued shapes onto `view`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
        camera_group: &wgpu::BindGroup,
        viewport: (f32, f32, f32, f32),
    ) -> u32 {
        let shapes = std::mem::take(&mut self.shapes);
        if shapes.is_empty() {
            return 0;
        }
        self.ensure_pipeline(device, format, camera_layout);
        let (Some(state), Some(uniform_buffer), Some(bind_group)) =
            (&self.pipeline, &self.uniform_buffer, &self.bind_group)
        else {
            return 0;
        };

        let (x, y, width, height) = viewport;
        let uniform = ShapeUniform {
            viewport_size: [width.max(1.0), height.max(1.0)],
            _padding: [0.0; 2],
        };
        queue.write_buffer(uniform_buffer, 0, bytemuck::bytes_of(&uniform));

        let bytes: &[u8] = bytemuck::cast_slice(&shapes);
        let needs_buffer = self
            .instance_buffer
            .as_ref()
            .is_none_or(|buffer| buffer.size() < bytes.len() as u64);
        if needs_buffer {
            let capacity = (bytes.len() as u64).next_power_of_two().max(4096);
            self.instance_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("GizmoShapeInstances"),
                size: capacity,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }));
        }
        let Some(instance_buffer) = &self.instance_buffer else {
            return 0;
        };
        queue.write_buffer(instance_buffer, 0, bytes);

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("GizmoShapePass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(x, y, width.max(1.0), height.max(1.0), 0.0, 1.0);
        pass.set_pipeline(&state.pipeline);
        pass.set_bind_group(0, camera_group, &[]);
        pass.set_bind_group(1, bind_group, &[]);
        pass.set_vertex_buffer(0, instance_buffer.slice(..bytes.len() as u64));
        pass.draw(0..6, 0..shapes.len() as u32);
        1
    }

    fn ensure_pipeline(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
    ) {
        if self
            .pipeline
            .as_ref()
            .is_some_and(|state| state.format == format)
        {
            return;
        }

        let shader = self.shader.get_or_insert_with(|| {
            device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("GizmoShapeShader"),
                source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
            })
        });
        let bind_layout = self.bind_layout.get_or_insert_with(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("GizmoShapeBindLayout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            })
        });
        let uniform_buffer = self.uniform_buffer.get_or_insert_with(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("GizmoShapeUniform"),
                contents: bytemuck::bytes_of(&ShapeUniform {
                    viewport_size: [1.0, 1.0],
                    _padding: [0.0; 2],
                }),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            })
        });
        if self.bind_group.is_none() {
            self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("GizmoShapeBindGroup"),
                layout: bind_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                }],
            }));
        }

        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("GizmoShapePipelineLayout"),
            bind_group_layouts: &[Some(camera_layout), Some(bind_layout)],
            immediate_size: 0,
        });
        let attributes = wgpu::vertex_attr_array![
            0 => Float32x4,
            1 => Float32x4,
            2 => Float32x4,
            3 => Float32x4,
            4 => Float32x4,
            5 => Float32x4,
            6 => Float32x4,
        ];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("GizmoShapePipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_main"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<GizmoShape>() as u64,
                    step_mode: wgpu::VertexStepMode::Instance,
                    attributes: &attributes,
                })],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        self.pipeline = Some(PipelineState { pipeline, format });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shader_validates() {
        let source = super::SHADER_SOURCE;
        let module = naga::front::wgsl::parse_str(source)
            .unwrap_or_else(|err| panic!("{}", err.emit_to_string(source)));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::empty(),
        )
        .validate(&module)
        .unwrap_or_else(|err| panic!("{}", err.emit_to_string(source)));
    }

    #[test]
    fn shape_layout_matches_shader() {
        // Seven vec4 attributes per instance.
        assert_eq!(std::mem::size_of::<super::GizmoShape>(), 7 * 16);
    }
}
