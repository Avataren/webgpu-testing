//! Local exposure on the GPU: a bilateral grid of the image's brightness, built and blurred
//! in compute passes after auto exposure, which the composite pass samples to pull each
//! region's brightness toward middle grey (see shader/postprocess/local_exposure.wgsl).

use glam::Vec2;

/// Brightness bins per grid cell (`LOCAL_EXPOSURE_BINS` in the shader).
const BINS: u32 = 32;
/// Cells along the view's longer side.
const CELLS_ALONG_LONG_SIDE: f32 = 48.0;
/// Smallest cell, in pixels: tiny views get fewer cells instead of smaller ones.
const MIN_CELL_SIZE: f32 = 4.0;
/// The most cells the grid can hold along x or y (`LOCAL_EXPOSURE_MAX_CELLS` in the
/// shader). Rounding the cell size gives at most 48 * 192 / 168 = 54.9 along a side.
pub(super) const MAX_CELLS: u32 = 56;
/// A grid cell in the buffers: sum of log brightness and pixel count.
const CELL_BYTES: u64 = 8;

/// The grid for a view of `resolution` pixels: the cell size in pixels and the cells in
/// use along x and y.
pub(super) fn grid_layout(resolution: Vec2) -> (f32, u32, u32) {
    let long_side = resolution.x.max(resolution.y).max(1.0);
    let cell = (long_side / CELLS_ALONG_LONG_SIDE)
        .round()
        .max(MIN_CELL_SIZE);
    let cells = |pixels: f32| ((pixels.max(1.0) / cell).ceil() as u32).min(MAX_CELLS);
    (cell, cells(resolution.x), cells(resolution.y))
}

pub struct LocalExposureStage {
    layout: wgpu::BindGroupLayout,
    build_pipeline: wgpu::ComputePipeline,
    blur_x_pipeline: wgpu::ComputePipeline,
    blur_y_pipeline: wgpu::ComputePipeline,
    blur_bins_pipeline: wgpu::ComputePipeline,
    /// Grid buffers A and B, each the largest grid.
    grids: [wgpu::Buffer; 2],
    /// The blurred grid the composite pass samples: sum and count in r and g.
    view: wgpu::TextureView,
    /// [writes A (reads B), writes B (reads A)].
    bind_groups: Option<[wgpu::BindGroup; 2]>,
}

impl LocalExposureStage {
    pub fn new(
        device: &wgpu::Device,
        uniform_layout: &wgpu::BindGroupLayout,
        shader: &wgpu::ShaderModule,
    ) -> Self {
        let storage = |binding, read_only| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("LocalExposureLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 113,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                storage(114, true),
                storage(115, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 116,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::StorageTexture {
                        access: wgpu::StorageTextureAccess::WriteOnly,
                        format: wgpu::TextureFormat::Rgba16Float,
                        view_dimension: wgpu::TextureViewDimension::D3,
                    },
                    count: None,
                },
                storage(117, true),
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("LocalExposurePipelineLayout"),
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

        let grid = |label| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: (MAX_CELLS * MAX_CELLS * BINS) as u64 * CELL_BYTES,
                usage: wgpu::BufferUsages::STORAGE,
                mapped_at_creation: false,
            })
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("LocalExposureGrid"),
            size: wgpu::Extent3d {
                width: MAX_CELLS,
                height: MAX_CELLS,
                depth_or_array_layers: BINS,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        Self {
            layout,
            build_pipeline: compute("LocalExposureBuildPipeline", "cs_local_exposure_build"),
            blur_x_pipeline: compute("LocalExposureBlurXPipeline", "cs_local_exposure_blur_x"),
            blur_y_pipeline: compute("LocalExposureBlurYPipeline", "cs_local_exposure_blur_y"),
            blur_bins_pipeline: compute(
                "LocalExposureBlurBinsPipeline",
                "cs_local_exposure_blur_bins",
            ),
            grids: [grid("LocalExposureGridA"), grid("LocalExposureGridB")],
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            bind_groups: None,
        }
    }

    /// The blurred grid, bound by the composite pass.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    pub fn invalidate(&mut self) {
        self.bind_groups = None;
    }

    pub fn ensure_bind_groups(
        &mut self,
        device: &wgpu::Device,
        scene_view: &wgpu::TextureView,
        exposure_state: &wgpu::Buffer,
    ) {
        if self.bind_groups.is_some() {
            return;
        }
        let [a, b] = &self.grids;
        let group = |label, input: &wgpu::Buffer, output: &wgpu::Buffer| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 113,
                        resource: wgpu::BindingResource::TextureView(scene_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 114,
                        resource: input.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 115,
                        resource: output.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 116,
                        resource: wgpu::BindingResource::TextureView(&self.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 117,
                        resource: exposure_state.as_entire_binding(),
                    },
                ],
            })
        };
        self.bind_groups = Some([
            group("LocalExposureWriteA", b, a),
            group("LocalExposureWriteB", a, b),
        ]);
    }

    /// Build and blur the grid for the visible `viewport` (in pixels), after auto exposure
    /// has adapted for this frame.
    pub fn render(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        uniform_bind_group: &wgpu::BindGroup,
        viewport: Vec2,
    ) {
        let Some([write_a, write_b]) = self.bind_groups.as_ref() else {
            return;
        };
        let (_, cells_x, cells_y) = grid_layout(viewport);
        let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("LocalExposure"),
            timestamp_writes: None,
        });
        pass.set_bind_group(0, uniform_bind_group, &[]);
        // One workgroup per cell.
        pass.set_bind_group(1, write_a, &[]);
        pass.set_pipeline(&self.build_pipeline);
        pass.dispatch_workgroups(cells_x, cells_y, 1);
        // One thread per cell and bin.
        let (groups_x, groups_y) = (cells_x.div_ceil(8), cells_y.div_ceil(8));
        pass.set_bind_group(1, write_b, &[]);
        pass.set_pipeline(&self.blur_x_pipeline);
        pass.dispatch_workgroups(groups_x, groups_y, BINS);
        pass.set_bind_group(1, write_a, &[]);
        pass.set_pipeline(&self.blur_y_pipeline);
        pass.dispatch_workgroups(groups_x, groups_y, BINS);
        pass.set_bind_group(1, write_b, &[]);
        pass.set_pipeline(&self.blur_bins_pipeline);
        pass.dispatch_workgroups(groups_x, groups_y, BINS);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_fits_any_view() {
        for (width, height) in [
            (1.0, 1.0),
            (64.0, 4096.0),
            (191.0, 100.0),
            (215.0, 40.0),
            (1045.0, 687.0),
            (1824.0, 1080.0),
            (3840.0, 2160.0),
            (7680.0, 400.0),
        ] {
            let (cell, x, y) = grid_layout(Vec2::new(width, height));
            assert!(cell >= MIN_CELL_SIZE);
            assert!((1..=MAX_CELLS).contains(&x) && (1..=MAX_CELLS).contains(&y));
            // Every pixel lands in a cell in use.
            assert!(x as f32 * cell >= width && y as f32 * cell >= height);
        }
        // About 48 cells along the longer side whatever the resolution.
        assert_eq!(grid_layout(Vec2::new(1824.0, 1080.0)), (38.0, 48, 29));
    }
}
