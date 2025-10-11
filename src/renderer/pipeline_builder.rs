// src/renderer/pipeline_builder.rs

/// Builder for creating render pipelines with sensible defaults
/// 
/// Reduces boilerplate when creating pipelines by providing a fluent API
/// and common presets for depth/stencil, blending, etc.
pub struct PipelineBuilder<'a> {
    device: &'a wgpu::Device,
    label: Option<&'a str>,
    layout: &'a wgpu::PipelineLayout,
    shader: &'a wgpu::ShaderModule,
    vertex_entry: &'a str,
    fragment_entry: Option<&'a str>,
    vertex_buffers: Vec<wgpu::VertexBufferLayout<'a>>,
    color_targets: Vec<Option<wgpu::ColorTargetState>>,
    depth_stencil: Option<wgpu::DepthStencilState>,
    primitive: wgpu::PrimitiveState,
    multisample: wgpu::MultisampleState,
    custom_vertex_state: Option<wgpu::VertexState<'a>>,
}

impl<'a> PipelineBuilder<'a> {
    /// Create a new pipeline builder with required parameters
    pub fn new(
        device: &'a wgpu::Device,
        layout: &'a wgpu::PipelineLayout,
        shader: &'a wgpu::ShaderModule,
    ) -> Self {
        Self {
            device,
            label: None,
            layout,
            shader,
            vertex_entry: "vs_main",
            fragment_entry: Some("fs_main"),
            vertex_buffers: Vec::new(),
            color_targets: Vec::new(),
            depth_stencil: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                front_face: wgpu::FrontFace::Ccw,
                polygon_mode: wgpu::PolygonMode::Fill,
                ..Default::default()
            },
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            custom_vertex_state: None
        }
    }

    /// Set the pipeline label for debugging
    pub fn with_label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Set a custom vertex state, overriding the default construction
    /// Use this when you need full control over the vertex state
    pub fn with_vertex_state(mut self, vertex_state: wgpu::VertexState<'a>) -> Self {
        self.custom_vertex_state = Some(vertex_state);
        self
    }    

    /// Set the vertex shader entry point (default: "vs_main")
    pub fn with_vertex_entry(mut self, entry: &'a str) -> Self {
        self.vertex_entry = entry;
        self
    }

    /// Set the fragment shader entry point (default: "fs_main")
    pub fn with_fragment_entry(mut self, entry: &'a str) -> Self {
        self.fragment_entry = Some(entry);
        self
    }

    /// Create a depth-only pipeline (no fragment shader)
    pub fn depth_only(mut self) -> Self {
        self.fragment_entry = None;
        self
    }

    /// Add a vertex buffer layout
    pub fn with_vertex_buffer(mut self, layout: wgpu::VertexBufferLayout<'a>) -> Self {
        self.vertex_buffers.push(layout);
        self
    }

    /// Add a color target
    pub fn with_color_target(mut self, format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>) -> Self {
        self.color_targets.push(Some(wgpu::ColorTargetState {
            format,
            blend,
            write_mask: wgpu::ColorWrites::ALL,
        }));
        self
    }

    /// Configure depth/stencil state
    pub fn with_depth_stencil(
        mut self,
        format: wgpu::TextureFormat,
        depth_write: bool,
        depth_compare: wgpu::CompareFunction,
    ) -> Self {
        self.depth_stencil = Some(wgpu::DepthStencilState {
            format,
            depth_write_enabled: depth_write,
            depth_compare,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });
        self
    }

    /// Configure depth/stencil with custom bias (for shadow maps)
    pub fn with_depth_stencil_biased(
        mut self,
        format: wgpu::TextureFormat,
        depth_write: bool,
        depth_compare: wgpu::CompareFunction,
        constant_bias: i32,
        slope_bias: f32,
    ) -> Self {
        self.depth_stencil = Some(wgpu::DepthStencilState {
            format,
            depth_write_enabled: depth_write,
            depth_compare,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState {
                constant: constant_bias,
                slope_scale: slope_bias,
                clamp: 0.0,
            },
        });
        self
    }

    /// Set MSAA sample count
    pub fn with_multisample(mut self, sample_count: u32) -> Self {
        self.multisample.count = sample_count;
        self
    }

    /// Disable backface culling
    pub fn with_no_culling(mut self) -> Self {
        self.primitive.cull_mode = None;
        self
    }

    /// Set primitive topology
    pub fn with_topology(mut self, topology: wgpu::PrimitiveTopology) -> Self {
        self.primitive.topology = topology;
        self
    }

    /// Build the render pipeline
    pub fn build(self) -> wgpu::RenderPipeline {
        self.device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: self.label,
            layout: Some(self.layout),
            vertex: wgpu::VertexState {
                module: self.shader,
                entry_point: Some(self.vertex_entry),
                buffers: &self.vertex_buffers,
                compilation_options: Default::default(),
            },
            fragment: self.fragment_entry.map(|entry| wgpu::FragmentState {
                module: self.shader,
                entry_point: Some(entry),
                targets: &self.color_targets,
                compilation_options: Default::default(),
            }),
            primitive: self.primitive,
            depth_stencil: self.depth_stencil,
            multisample: self.multisample,
            multiview: None,
            cache: None,
        })
    }
}