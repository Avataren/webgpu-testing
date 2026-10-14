/// Auto exposure on the GPU: a luminance histogram of the visible scene and a single
/// workgroup that turns it into an adapted average kept in a small storage buffer. The
/// composite pass reads that buffer directly, so nothing is read back to the CPU.
pub struct ExposureStage {
    layout: wgpu::BindGroupLayout,
    histogram_pipeline: wgpu::ComputePipeline,
    average_pipeline: wgpu::ComputePipeline,
    histogram_buffer: wgpu::Buffer,
    state_buffer: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
}

/// Bytes in the shader's `ExposureState`: average EV, initialized flag, padding.
const STATE_SIZE: u64 = 16;
const HISTOGRAM_BINS: u64 = 64;
/// Each histogram thread samples one pixel of a 2x2 block; workgroups are 16x16 threads.
const PIXELS_PER_WORKGROUP: u32 = 32;

impl ExposureStage {
    pub fn new(
        device: &wgpu::Device,
        uniform_layout: &wgpu::BindGroupLayout,
        shader: &wgpu::ShaderModule,
    ) -> Self {
        let storage = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ExposureLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 110,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                storage(111),
                storage(112),
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ExposurePipelineLayout"),
            bind_group_layouts: &[Some(uniform_layout), Some(&layout)],
            immediate_size: 0,
        });
        let compute = |label: &str, entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                module: shader,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let histogram_pipeline = compute("ExposureHistogramPipeline", "cs_exposure_histogram");
        let average_pipeline = compute("ExposureAveragePipeline", "cs_exposure_average");

        // Buffers start zeroed: an empty histogram and an uninitialized state.
        let histogram_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ExposureHistogram"),
            size: HISTOGRAM_BINS * 4,
            usage: wgpu::BufferUsages::STORAGE,
            mapped_at_creation: false,
        });
        let state_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ExposureState"),
            size: STATE_SIZE,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            layout,
            histogram_pipeline,
            average_pipeline,
            histogram_buffer,
            state_buffer,
            bind_group: None,
        }
    }

    /// The adapted exposure state, bound read-only by the composite pass.
    pub fn state_buffer(&self) -> &wgpu::Buffer {
        &self.state_buffer
    }

    pub fn invalidate(&mut self) {
        self.bind_group = None;
    }

    /// Forget the adapted value so the next measurement applies immediately instead of
    /// easing in (after a scene change, say).
    pub fn reset(&self, queue: &wgpu::Queue) {
        queue.write_buffer(&self.state_buffer, 0, &[0u8; STATE_SIZE as usize]);
    }

    pub fn ensure_bind_group(&mut self, device: &wgpu::Device, scene_view: &wgpu::TextureView) {
        if self.bind_group.is_some() {
            return;
        }
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ExposureBindGroup"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 110,
                    resource: wgpu::BindingResource::TextureView(scene_view),
                },
                wgpu::BindGroupEntry {
                    binding: 111,
                    resource: self.histogram_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 112,
                    resource: self.state_buffer.as_entire_binding(),
                },
            ],
        }));
    }

    /// Measure the visible `viewport` (in pixels) of the scene and adapt.
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        viewport: (u32, u32),
    ) {
        let Some(bind_group) = self.bind_group.as_ref() else {
            return;
        };
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("AutoExposure"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, uniform_bind_group, &[]);
        pass.set_bind_group(1, bind_group, &[]);
        pass.set_pipeline(&self.histogram_pipeline);
        pass.dispatch_workgroups(
            viewport.0.max(1).div_ceil(PIXELS_PER_WORKGROUP),
            viewport.1.max(1).div_ceil(PIXELS_PER_WORKGROUP),
            1,
        );
        pass.set_pipeline(&self.average_pipeline);
        pass.dispatch_workgroups(1, 1, 1);
    }
}
