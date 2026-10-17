//! Debug geometry in world space: translucent triangles and anti-aliased lines a few
//! pixels wide, drawn over the finished image and depth tested against the scene
//! (navmeshes, paths, agents). With `x_ray` set, the parts hidden behind scenery are
//! drawn too, faintly, so floors under overhangs stay readable.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;

const SHADER_SOURCE: &str = concat!(
    include_str!("../shader/camera.wgsl"),
    include_str!("../shader/debug_draw.wgsl"),
);

/// An sRGB colour with straight alpha, in 0..1.
pub type DebugColor = [f32; 4];

fn linear(color: DebugColor) -> [f32; 4] {
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

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct DebugVertex {
    position: [f32; 4],
    color: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct DebugLine {
    /// xyz and the width in pixels.
    a: [f32; 4],
    b: [f32; 4],
    color: [f32; 4],
}

/// What to draw this frame.
#[derive(Clone, Debug, Default)]
pub struct DebugGeometry {
    pub triangles: Vec<DebugVertex>,
    pub lines: Vec<DebugLine>,
    /// How strongly hidden parts show (0: not at all).
    pub x_ray: f32,
}

impl DebugGeometry {
    pub fn is_empty(&self) -> bool {
        self.triangles.is_empty() && self.lines.is_empty()
    }

    pub fn triangle(&mut self, a: Vec3, b: Vec3, c: Vec3, color: DebugColor) {
        let color = linear(color);
        for p in [a, b, c] {
            self.triangles.push(DebugVertex {
                position: [p.x, p.y, p.z, 1.0],
                color,
            });
        }
    }

    pub fn line(&mut self, a: Vec3, b: Vec3, width: f32, color: DebugColor) {
        self.lines.push(DebugLine {
            a: [a.x, a.y, a.z, width],
            b: [b.x, b.y, b.z, 0.0],
            color: linear(color),
        });
    }

    /// A horizontal circle.
    pub fn circle(&mut self, center: Vec3, radius: f32, width: f32, color: DebugColor) {
        const SEGMENTS: usize = 20;
        let point = |i: usize| {
            let angle = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            center + Vec3::new(angle.cos() * radius, 0.0, angle.sin() * radius)
        };
        for i in 0..SEGMENTS {
            self.line(point(i), point(i + 1), width, color);
        }
    }

    /// A line with a small head at `b`.
    pub fn arrow(&mut self, a: Vec3, b: Vec3, width: f32, color: DebugColor) {
        self.line(a, b, width, color);
        let dir = b - a;
        let length = dir.length();
        if length < 1e-3 {
            return;
        }
        let dir = dir / length;
        let side = dir.cross(Vec3::Y).normalize_or_zero() * (length * 0.15).min(0.25);
        let back = b - dir * (length * 0.25).min(0.4);
        self.line(b, back + side, width, color);
        self.line(b, back - side, width, color);
    }

    pub fn append(&mut self, other: DebugGeometry) {
        self.triangles.extend(other.triangles);
        self.lines.extend(other.lines);
        self.x_ray = self.x_ray.max(other.x_ray);
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct DebugUniform {
    viewport_size: [f32; 2],
    alpha_scale: f32,
    _padding: f32,
}

struct Pipelines {
    format: wgpu::TextureFormat,
    depth_format: wgpu::TextureFormat,
    /// Triangles and lines, depth tested as usual and inverted (x-ray).
    triangles: [wgpu::RenderPipeline; 2],
    lines: [wgpu::RenderPipeline; 2],
}

#[derive(Default)]
pub(crate) struct DebugDrawStage {
    geometry: DebugGeometry,
    layout: Option<wgpu::BindGroupLayout>,
    /// Uniforms of the visible and the x-ray pass.
    uniforms: Option<[wgpu::Buffer; 2]>,
    bind_groups: Option<[wgpu::BindGroup; 2]>,
    triangle_buffer: Option<wgpu::Buffer>,
    line_buffer: Option<wgpu::Buffer>,
    pipelines: Option<Pipelines>,
}

fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    slot: &mut Option<wgpu::Buffer>,
    bytes: &[u8],
    label: &str,
) {
    let needs = slot.as_ref().is_none_or(|b| b.size() < bytes.len() as u64);
    if needs {
        let size = (bytes.len() as u64).next_power_of_two().max(4096);
        *slot = Some(device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        }));
    }
    if let Some(buffer) = slot {
        queue.write_buffer(buffer, 0, bytes);
    }
}

impl DebugDrawStage {
    pub(crate) fn set_geometry(&mut self, geometry: DebugGeometry) {
        self.geometry = geometry;
    }

    /// Draw (and forget) the queued geometry onto `target`. Returns the draw calls.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        depth: &wgpu::TextureView,
        depth_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
        camera_group: &wgpu::BindGroup,
        viewport: (f32, f32, f32, f32),
    ) -> u32 {
        let geometry = std::mem::take(&mut self.geometry);
        if geometry.is_empty() {
            return 0;
        }
        self.ensure_pipelines(device, format, depth_format, camera_layout);
        let (x, y, width, height) = viewport;
        if let Some(uniforms) = &self.uniforms {
            for (buffer, alpha) in uniforms.iter().zip([1.0, geometry.x_ray]) {
                queue.write_buffer(
                    buffer,
                    0,
                    bytemuck::bytes_of(&DebugUniform {
                        viewport_size: [width.max(1.0), height.max(1.0)],
                        alpha_scale: alpha,
                        _padding: 0.0,
                    }),
                );
            }
        }
        if !geometry.triangles.is_empty() {
            upload(
                device,
                queue,
                &mut self.triangle_buffer,
                bytemuck::cast_slice(&geometry.triangles),
                "DebugTriangles",
            );
        }
        if !geometry.lines.is_empty() {
            upload(
                device,
                queue,
                &mut self.line_buffer,
                bytemuck::cast_slice(&geometry.lines),
                "DebugLines",
            );
        }
        let (Some(pipelines), Some(bind_groups)) = (&self.pipelines, &self.bind_groups) else {
            return 0;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("DebugDraw"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_viewport(x, y, width.max(1.0), height.max(1.0), 0.0, 1.0);
        pass.set_bind_group(0, camera_group, &[]);
        let mut draws = 0;
        let passes: &[usize] = if geometry.x_ray > 0.0 { &[1, 0] } else { &[0] };
        for &which in passes {
            pass.set_bind_group(1, &bind_groups[which], &[]);
            if let (false, Some(buffer)) = (geometry.triangles.is_empty(), &self.triangle_buffer) {
                let bytes = std::mem::size_of_val(geometry.triangles.as_slice()) as u64;
                pass.set_pipeline(&pipelines.triangles[which]);
                pass.set_vertex_buffer(0, buffer.slice(..bytes));
                pass.draw(0..geometry.triangles.len() as u32, 0..1);
                draws += 1;
            }
            if let (false, Some(buffer)) = (geometry.lines.is_empty(), &self.line_buffer) {
                let bytes = std::mem::size_of_val(geometry.lines.as_slice()) as u64;
                pass.set_pipeline(&pipelines.lines[which]);
                pass.set_vertex_buffer(0, buffer.slice(..bytes));
                pass.draw(0..6, 0..geometry.lines.len() as u32);
                draws += 1;
            }
        }
        draws
    }

    fn ensure_pipelines(
        &mut self,
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
        camera_layout: &wgpu::BindGroupLayout,
    ) {
        if self
            .pipelines
            .as_ref()
            .is_some_and(|p| p.format == format && p.depth_format == depth_format)
        {
            return;
        }
        let layout = self.layout.get_or_insert_with(|| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("DebugDraw"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            })
        });
        let uniforms = self.uniforms.get_or_insert_with(|| {
            let make = || {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("DebugDrawUniform"),
                    contents: bytemuck::bytes_of(&DebugUniform {
                        viewport_size: [1.0, 1.0],
                        alpha_scale: 1.0,
                        _padding: 0.0,
                    }),
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                })
            };
            [make(), make()]
        });
        if self.bind_groups.is_none() {
            let group = |buffer: &wgpu::Buffer| {
                device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("DebugDraw"),
                    layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                })
            };
            self.bind_groups = Some([group(&uniforms[0]), group(&uniforms[1])]);
        }
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("DebugDraw"),
            source: wgpu::ShaderSource::Wgsl(SHADER_SOURCE.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("DebugDraw"),
            bind_group_layouts: &[Some(camera_layout), Some(layout)],
            immediate_size: 0,
        });
        let triangle_attributes = wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4];
        let line_attributes =
            wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];
        let make = |entry: (&str, &str),
                    stride: u64,
                    step: wgpu::VertexStepMode,
                    attributes: &[wgpu::VertexAttribute],
                    hidden: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("DebugDraw"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(entry.0),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: stride,
                        step_mode: step,
                        attributes,
                    })],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry.1),
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
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: depth_format,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(if hidden {
                        wgpu::CompareFunction::Greater
                    } else {
                        wgpu::CompareFunction::LessEqual
                    }),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let tri_stride = std::mem::size_of::<DebugVertex>() as u64;
        let line_stride = std::mem::size_of::<DebugLine>() as u64;
        let triangles = [
            make(
                ("vs_triangle", "fs_triangle"),
                tri_stride,
                wgpu::VertexStepMode::Vertex,
                &triangle_attributes,
                false,
            ),
            make(
                ("vs_triangle", "fs_triangle"),
                tri_stride,
                wgpu::VertexStepMode::Vertex,
                &triangle_attributes,
                true,
            ),
        ];
        let lines = [
            make(
                ("vs_line", "fs_line"),
                line_stride,
                wgpu::VertexStepMode::Instance,
                &line_attributes,
                false,
            ),
            make(
                ("vs_line", "fs_line"),
                line_stride,
                wgpu::VertexStepMode::Instance,
                &line_attributes,
                true,
            ),
        ];
        self.pipelines = Some(Pipelines {
            format,
            depth_format,
            triangles,
            lines,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shader_validates() {
        let source = SHADER_SOURCE;
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
    fn layouts_are_aligned() {
        assert_eq!(std::mem::size_of::<DebugVertex>(), 32);
        assert_eq!(std::mem::size_of::<DebugLine>(), 48);
        assert_eq!(std::mem::size_of::<DebugUniform>() % 16, 0);
    }
}
