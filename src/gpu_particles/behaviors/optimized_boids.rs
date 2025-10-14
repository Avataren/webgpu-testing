use bytemuck::{Pod, Zeroable};

use crate::renderer::compute_resources::{
    BindGroupBuilder, BindGroupLayoutBuilder, StorageBuffer, UniformBuffer,
};
use crate::renderer::ComputePipelineBuilder;
use wgpu::util::DeviceExt;

use crate::gpu_particles::ParticleBehavior;

use super::radix_sort::RadixSort;

const MAX_TOTAL_CELLS: u32 = 256;

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct OptimizedBoidsParams {
    delta_time: f32,
    separation_radius: f32,
    alignment_radius: f32,
    cohesion_radius: f32,
    separation_weight: f32,
    alignment_weight: f32,
    cohesion_weight: f32,
    max_speed: f32,
    max_force: f32,
    bounds: f32,
    particle_count: u32,
    cell_size: f32,
    grid_dimensions: [u32; 3],
    _padding: u32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GridBuildParams {
    bounds: f32,
    cell_size: f32,
    _padding1: [u32; 2],
    grid_dimensions: [u32; 3],
    _padding_vec3: u32,
    particle_count: u32,
    total_cells: u32,
    _padding2: [u32; 2],
}

pub struct OptimizedBoidsBehavior {
    pub separation_radius: f32,
    pub alignment_radius: f32,
    pub cohesion_radius: f32,
    pub separation_weight: f32,
    pub alignment_weight: f32,
    pub cohesion_weight: f32,
    pub max_speed: f32,
    pub max_force: f32,
    pub bounds: f32,
    pub particle_count: u32,

    cell_size: f32,
    grid_dimensions: [u32; 3],
    total_cells: u32,

    spatial_grid_buffer: StorageBuffer,
    sorted_particle_data_buffer: StorageBuffer,
    particle_grid_data_buffer: StorageBuffer,
    cell_counts_buffer: StorageBuffer,
    grid_params_buffer: UniformBuffer,

    radix_sorter: RadixSort,

    compute_cell_indices_pipeline: wgpu::ComputePipeline,
    prefix_sum_pipeline: wgpu::ComputePipeline,

    cell_indices_layout: wgpu::BindGroupLayout,

    cell_indices_bind_group: Option<wgpu::BindGroup>,
    prefix_sum_bind_group: wgpu::BindGroup,
}

fn calculate_grid_parameters(bounds: f32, max_interaction_radius: f32) -> (f32, [u32; 3], u32) {
    let grid_extent = bounds * 2.0;
    let preferred_cell_size = max_interaction_radius * 1.5;
    let max_grid_dim = (MAX_TOTAL_CELLS as f32).cbrt().floor() as u32;

    let mut grid_dim = ((grid_extent / preferred_cell_size).ceil() as u32).max(1);
    let mut cell_size = preferred_cell_size;

    if grid_dim > max_grid_dim {
        grid_dim = max_grid_dim.max(1);
        cell_size = grid_extent / grid_dim as f32;
    }

    if cell_size < max_interaction_radius {
        cell_size = max_interaction_radius;
        grid_dim = ((grid_extent / cell_size).ceil() as u32).clamp(1, max_grid_dim.max(1));
        cell_size = grid_extent / grid_dim as f32;
    }

    let grid_dimensions = [grid_dim, grid_dim, grid_dim];
    let total_cells = grid_dim * grid_dim * grid_dim;

    debug_assert!(
        total_cells <= MAX_TOTAL_CELLS,
        "prefix sum shader expects at most {MAX_TOTAL_CELLS} cells but got {total_cells}",
    );

    (cell_size, grid_dimensions, total_cells)
}

impl OptimizedBoidsBehavior {
    pub fn new(
        device: &wgpu::Device,
        max_particles: u32,
        bounds: f32,
        max_interaction_radius: f32,
    ) -> Self {
        let (cell_size, grid_dimensions, total_cells) =
            calculate_grid_parameters(bounds, max_interaction_radius);
        let grid_dim = grid_dimensions[0];

        log::info!(
            "Optimized Boids: Grid {}x{}x{} = {} cells, cell_size = {:.2}",
            grid_dim,
            grid_dim,
            grid_dim,
            total_cells,
            cell_size
        );

        let spatial_grid_buffer = StorageBuffer::with_capacity::<[u32; 2]>(
            device,
            "SpatialGridBuffer",
            total_cells as usize,
        );

        let sorted_particle_data_buffer = StorageBuffer::with_capacity::<[u32; 2]>(
            device,
            "SortedParticleDataBuffer",
            max_particles as usize,
        );

        let particle_grid_data_buffer = StorageBuffer::with_capacity::<[u32; 2]>(
            device,
            "ParticleGridDataBuffer",
            max_particles as usize,
        );

        let cell_counts_buffer =
            StorageBuffer::with_capacity::<u32>(device, "CellCountsBuffer", total_cells as usize);

        let grid_params = GridBuildParams {
            bounds,
            cell_size,
            grid_dimensions,
            particle_count: 0,
            total_cells,
            _padding1: [0, 0],
            _padding_vec3: 0,
            _padding2: [0, 0],
        };

        let grid_params_buffer = UniformBuffer::new(device, "GridParams", &grid_params);

        let cell_indices_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("CellIndicesShader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/spatial_grid_build.wgsl").into(),
            ),
        });

        let prefix_sum_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("PrefixSumShader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/parallel_prefix_sum.wgsl").into(),
            ),
        });

        let cell_indices_layout = BindGroupLayoutBuilder::new(device)
            .with_label("CellIndicesLayout")
            .add_storage_buffer(0, true)
            .add_uniform_buffer(1)
            .add_storage_buffer(2, false)
            .add_storage_buffer(3, false)
            .build();

        let prefix_sum_layout = BindGroupLayoutBuilder::new(device)
            .with_label("PrefixSumLayout")
            .add_storage_buffer(0, true)
            .add_storage_buffer(1, false)
            .add_uniform_buffer(2)
            .build();

        let compute_cell_indices_pipeline = ComputePipelineBuilder::new()
            .with_label("CellIndicesPipeline")
            .with_shader(&cell_indices_shader)
            .with_entry_point("compute_cell_indices")
            .with_bind_group_layout(&cell_indices_layout)
            .build(device);

        let prefix_sum_pipeline = ComputePipelineBuilder::new()
            .with_label("PrefixSumPipeline")
            .with_shader(&prefix_sum_shader)
            .with_entry_point("compute_start_indices")
            .with_bind_group_layout(&prefix_sum_layout)
            .build(device);

        let prefix_sum_bind_group = BindGroupBuilder::new(device, &prefix_sum_layout)
            .with_label("PrefixSumBindGroup")
            .add_buffer(0, cell_counts_buffer.buffer())
            .add_buffer(1, spatial_grid_buffer.buffer())
            .add_buffer(2, grid_params_buffer.buffer())
            .build();

        let radix_sorter = RadixSort::new(device, max_particles);

        Self {
            separation_radius: 3.0,
            alignment_radius: 20.0,
            cohesion_radius: 20.0,
            separation_weight: 1.0,
            alignment_weight: 1.5,
            cohesion_weight: 3.0,
            max_speed: 50.0,
            max_force: 6.0,
            bounds,
            particle_count: 0,
            cell_size,
            grid_dimensions,
            total_cells,
            spatial_grid_buffer,
            sorted_particle_data_buffer,
            particle_grid_data_buffer,
            cell_counts_buffer,
            grid_params_buffer,
            radix_sorter,
            compute_cell_indices_pipeline,
            prefix_sum_pipeline,
            cell_indices_layout,
            cell_indices_bind_group: None,
            prefix_sum_bind_group,
        }
    }

    pub fn set_particle_count(&mut self, count: u32) {
        self.particle_count = count;
    }

    pub fn build_spatial_grid(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        particles_buffer: &wgpu::Buffer,
        particle_count: u32,
    ) {
        self.particle_count = particle_count;

        if self.particle_count == 0 {
            return;
        }

        let grid_params = GridBuildParams {
            bounds: self.bounds,
            cell_size: self.cell_size,
            grid_dimensions: self.grid_dimensions,
            particle_count: self.particle_count,
            total_cells: self.total_cells,
            _padding1: [0, 0],
            _padding_vec3: 0,
            _padding2: [0, 0],
        };
        self.grid_params_buffer.write(queue, &grid_params);

        if self.cell_indices_bind_group.is_none() {
            self.cell_indices_bind_group = Some(
                BindGroupBuilder::new(device, &self.cell_indices_layout)
                    .with_label("CellIndicesBindGroup")
                    .add_buffer(0, particles_buffer)
                    .add_buffer(1, self.grid_params_buffer.buffer())
                    .add_buffer(2, self.particle_grid_data_buffer.buffer())
                    .add_buffer(3, self.cell_counts_buffer.buffer())
                    .build(),
            );
        }

        encoder.clear_buffer(self.cell_counts_buffer.buffer(), 0, None);

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ComputeCellIndices"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.compute_cell_indices_pipeline);
            pass.set_bind_group(0, self.cell_indices_bind_group.as_ref().unwrap(), &[]);
            pass.dispatch_workgroups(self.particle_count.div_ceil(256), 1, 1);
        }

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ComputePrefixSum"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.prefix_sum_pipeline);
            pass.set_bind_group(0, &self.prefix_sum_bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }

        self.radix_sorter.sort(
            device,
            queue,
            encoder,
            self.particle_grid_data_buffer.buffer(),
            self.sorted_particle_data_buffer.buffer(),
            self.particle_count,
        );
    }
}

impl ParticleBehavior for OptimizedBoidsBehavior {
    fn shader_source(&self) -> &str {
        include_str!("../shaders/boids_optimized.wgsl")
    }

    fn entry_point(&self) -> &str {
        "main"
    }

    fn create_params_buffer(&self, device: &wgpu::Device, _queue: &wgpu::Queue) -> wgpu::Buffer {
        let params = OptimizedBoidsParams {
            delta_time: 0.0,
            separation_radius: self.separation_radius,
            alignment_radius: self.alignment_radius,
            cohesion_radius: self.cohesion_radius,
            separation_weight: self.separation_weight,
            alignment_weight: self.alignment_weight,
            cohesion_weight: self.cohesion_weight,
            max_speed: self.max_speed,
            max_force: self.max_force,
            bounds: self.bounds,
            particle_count: self.particle_count,
            cell_size: self.cell_size,
            grid_dimensions: self.grid_dimensions,
            _padding: 0,
        };

        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("OptimizedBoidsParams"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn update_params(
        &self,
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
        dt: f32,
        active_count: u32,
    ) {
        let params = OptimizedBoidsParams {
            delta_time: dt,
            separation_radius: self.separation_radius,
            alignment_radius: self.alignment_radius,
            cohesion_radius: self.cohesion_radius,
            separation_weight: self.separation_weight,
            alignment_weight: self.alignment_weight,
            cohesion_weight: self.cohesion_weight,
            max_speed: self.max_speed,
            max_force: self.max_force,
            bounds: self.bounds,
            particle_count: active_count,
            cell_size: self.cell_size,
            grid_dimensions: self.grid_dimensions,
            _padding: 0,
        };

        queue.write_buffer(buffer, 0, bytemuck::bytes_of(&params));
    }

    fn additional_bindings(
        &self,
        _device: &wgpu::Device,
        start_binding: u32,
    ) -> Vec<wgpu::BindGroupEntry<'_>> {
        vec![
            wgpu::BindGroupEntry {
                binding: start_binding,
                resource: self.spatial_grid_buffer.buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: start_binding + 1,
                resource: self
                    .sorted_particle_data_buffer
                    .buffer()
                    .as_entire_binding(),
            },
        ]
    }

    fn additional_layout_entries(&self, start_binding: u32) -> Vec<wgpu::BindGroupLayoutEntry> {
        vec![
            wgpu::BindGroupLayoutEntry {
                binding: start_binding,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: start_binding + 1,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn optimized_boids_params_alignment() {
        assert_eq!(std::mem::size_of::<OptimizedBoidsParams>(), 64);
        assert_eq!(std::mem::align_of::<OptimizedBoidsParams>(), 16);
    }

    #[test]
    fn grid_build_params_alignment() {
        assert_eq!(std::mem::size_of::<GridBuildParams>(), 48);
        assert_eq!(std::mem::align_of::<GridBuildParams>(), 16);
    }

    #[test]
    fn grid_dimension_calculations() {
        let test_cases = vec![
            (50.0, 10.0, 6, 216),
            (100.0, 20.0, 6, 216),
            (75.0, 20.0, 5, 125),
        ];

        for (bounds, radius, expected_dim, expected_total) in test_cases {
            let (_cell_size, grid_dims, total_cells) = calculate_grid_parameters(bounds, radius);
            assert_eq!(grid_dims[0], expected_dim);
            assert_eq!(total_cells, expected_total);
            assert!(total_cells <= MAX_TOTAL_CELLS);
        }
    }
}
