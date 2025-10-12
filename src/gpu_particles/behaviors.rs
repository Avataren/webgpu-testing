// src/gpu_particles/behaviors.rs

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use super::ParticleBehavior;

// ============================================================================
// Starfield Behavior
// ============================================================================

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct StarfieldParams {
    delta_time: f32,
    near_plane: f32,
    far_plane: f32,
    far_reset_band: f32,
    field_half_size: f32,
    min_radius: f32,
    particle_count: u32,
    _padding: u32,
}

pub struct StarfieldBehavior {
    pub near_plane: f32,
    pub far_plane: f32,
    pub far_reset_band: f32,
    pub field_half_size: f32,
    pub min_radius: f32,
}

impl ParticleBehavior for StarfieldBehavior {
    fn shader_source(&self) -> &str {
        include_str!("shaders/starfield.wgsl")
    }

    fn entry_point(&self) -> &str {
        "main"
    }

    fn create_params_buffer(&self, device: &wgpu::Device, _queue: &wgpu::Queue) -> wgpu::Buffer {
        let params = StarfieldParams {
            delta_time: 0.0,
            near_plane: self.near_plane,
            far_plane: self.far_plane,
            far_reset_band: self.far_reset_band,
            field_half_size: self.field_half_size,
            min_radius: self.min_radius,
            particle_count: 0,
            _padding: 0,
        };

        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("StarfieldParams"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn update_params(&self, queue: &wgpu::Queue, buffer: &wgpu::Buffer, dt: f32) {
        let params = StarfieldParams {
            delta_time: dt,
            near_plane: self.near_plane,
            far_plane: self.far_plane,
            far_reset_band: self.far_reset_band,
            field_half_size: self.field_half_size,
            min_radius: self.min_radius,
            particle_count: 0,
            _padding: 0,
        };

        queue.write_buffer(buffer, 0, bytemuck::bytes_of(&params));
    }
}

// ============================================================================
// Physics Behavior
// ============================================================================

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PhysicsParams {
    delta_time: f32,
    drag: f32,
    turbulence_strength: f32,
    turbulence_frequency: f32,
    gravity: [f32; 3],
    particle_count: u32,
}

pub struct PhysicsBehavior {
    pub drag: f32,
    pub turbulence_strength: f32,
    pub turbulence_frequency: f32,
    pub gravity: glam::Vec3,
}

impl Default for PhysicsBehavior {
    fn default() -> Self {
        Self {
            drag: 0.1,
            turbulence_strength: 0.0,
            turbulence_frequency: 1.0,
            gravity: glam::Vec3::new(0.0, -9.81, 0.0),
        }
    }
}

impl ParticleBehavior for PhysicsBehavior {
    fn shader_source(&self) -> &str {
        include_str!("shaders/physics.wgsl")
    }

    fn entry_point(&self) -> &str {
        "main"
    }

    fn create_params_buffer(&self, device: &wgpu::Device, _queue: &wgpu::Queue) -> wgpu::Buffer {
        let params = PhysicsParams {
            delta_time: 0.0,
            drag: self.drag,
            turbulence_strength: self.turbulence_strength,
            turbulence_frequency: self.turbulence_frequency,
            gravity: self.gravity.into(),
            particle_count: 0,
        };

        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("PhysicsParams"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn update_params(&self, queue: &wgpu::Queue, buffer: &wgpu::Buffer, dt: f32) {
        let params = PhysicsParams {
            delta_time: dt,
            drag: self.drag,
            turbulence_strength: self.turbulence_strength,
            turbulence_frequency: self.turbulence_frequency,
            gravity: self.gravity.into(),
            particle_count: 0,
        };

        queue.write_buffer(buffer, 0, bytemuck::bytes_of(&params));
    }
}

// ============================================================================
// Boids Behavior (Original O(n²) version)
// ============================================================================

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BoidsParams {
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
    _padding: u32,
}

pub struct BoidsBehavior {
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
}

impl Default for BoidsBehavior {
    fn default() -> Self {
        Self {
            separation_radius: 2.0,
            alignment_radius: 4.0,
            cohesion_radius: 4.0,
            separation_weight: 1.5,
            alignment_weight: 1.0,
            cohesion_weight: 1.0,
            max_speed: 5.0,
            max_force: 0.5,
            bounds: 20.0,
            particle_count: 0,
        }
    }
}

impl BoidsBehavior {
    pub fn set_particle_count(&mut self, count: u32) {
        self.particle_count = count;
    }
}

impl ParticleBehavior for BoidsBehavior {
    fn shader_source(&self) -> &str {
        include_str!("shaders/boids.wgsl")
    }

    fn entry_point(&self) -> &str {
        "main"
    }

    fn create_params_buffer(&self, device: &wgpu::Device, _queue: &wgpu::Queue) -> wgpu::Buffer {
        let params = BoidsParams {
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
            _padding: 0,
        };

        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("BoidsParams"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn update_params(&self, queue: &wgpu::Queue, buffer: &wgpu::Buffer, dt: f32) {
        let params = BoidsParams {
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
            particle_count: self.particle_count,
            _padding: 0,
        };

        queue.write_buffer(buffer, 0, bytemuck::bytes_of(&params));
    }
}

// ============================================================================
// Radix Sort Implementation
// ============================================================================

use crate::renderer::compute_resources::{
    BindGroupBuilder, BindGroupLayoutBuilder, StorageBuffer, UniformBuffer,
};
use crate::renderer::ComputePipelineBuilder;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct RadixSortParams {
    count: u32,
    bit_offset: u32,
    _padding: [u32; 2],
}

struct RadixSort {
    // Pipelines
    count_pipeline: wgpu::ComputePipeline,
    prefix_sum_pipeline: wgpu::ComputePipeline,
    scatter_pipeline: wgpu::ComputePipeline,
    
    // Layouts
    count_layout: wgpu::BindGroupLayout,
    prefix_sum_layout: wgpu::BindGroupLayout,
    scatter_layout: wgpu::BindGroupLayout,
    
    // Buffers
    histogram_buffer: StorageBuffer,
    params_buffer: UniformBuffer,
    temp_buffer: StorageBuffer,  // For ping-pong sorting
    
    // Bind groups (recreated when buffers change)
    count_bind_group_a: Option<wgpu::BindGroup>,
    count_bind_group_b: Option<wgpu::BindGroup>,
    prefix_sum_bind_group: wgpu::BindGroup,
    scatter_bind_group_a: Option<wgpu::BindGroup>,
    scatter_bind_group_b: Option<wgpu::BindGroup>,
}

impl RadixSort {
    fn new(device: &wgpu::Device, max_elements: u32) -> Self {
        // Create shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RadixSortShader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/radix_sort.wgsl").into(),
            ),
        });
        
        // Create histogram buffer (16 bins for 4-bit radix)
        let histogram_buffer = StorageBuffer::with_capacity::<u32>(
            device,
            "RadixSortHistogram",
            16,
        );
        
        // Create temp buffer for ping-pong
        let temp_buffer = StorageBuffer::with_capacity::<[u32; 2]>(
            device,
            "RadixSortTemp",
            max_elements as usize,
        );
        
        // Create params buffer
        let params = RadixSortParams {
            count: 0,
            bit_offset: 0,
            _padding: [0, 0],
        };
        let params_buffer = UniformBuffer::new(device, "RadixSortParams", &params);
        
        // Create bind group layouts
        let count_layout = BindGroupLayoutBuilder::new(device)
            .with_label("RadixSortCountLayout")
            .add_storage_buffer(0, true)   // input_data (read)
            .add_storage_buffer(1, false)  // histogram (read_write, atomic)
            .add_uniform_buffer(2)         // params
            .build();
        
        let prefix_sum_layout = BindGroupLayoutBuilder::new(device)
            .with_label("RadixSortPrefixSumLayout")
            .add_storage_buffer(0, false)  // histogram (read_write, atomic)
            .build();
        
        let scatter_layout = BindGroupLayoutBuilder::new(device)
            .with_label("RadixSortScatterLayout")
            .add_storage_buffer(0, true)   // input_data (read)
            .add_storage_buffer(1, false)  // output_data (write)
            .add_storage_buffer(2, false)  // histogram (read_write, atomic)
            .add_uniform_buffer(3)         // params
            .build();
        
        // Create pipelines
        let count_pipeline = ComputePipelineBuilder::new()
            .with_label("RadixSortCount")
            .with_shader(&shader)
            .with_entry_point("count_phase")
            .with_bind_group_layout(&count_layout)
            .build(device);
        
        let prefix_sum_pipeline = ComputePipelineBuilder::new()
            .with_label("RadixSortPrefixSum")
            .with_shader(&shader)
            .with_entry_point("prefix_sum_phase")
            .with_bind_group_layout(&prefix_sum_layout)
            .build(device);
        
        let scatter_pipeline = ComputePipelineBuilder::new()
            .with_label("RadixSortScatter")
            .with_shader(&shader)
            .with_entry_point("scatter_phase")
            .with_bind_group_layout(&scatter_layout)
            .build(device);
        
        // Create prefix sum bind group (doesn't depend on data buffers)
        let prefix_sum_bind_group = BindGroupBuilder::new(device, &prefix_sum_layout)
            .with_label("RadixSortPrefixSumBindGroup")
            .add_buffer(0, histogram_buffer.buffer())
            .build();
        
        Self {
            count_pipeline,
            prefix_sum_pipeline,
            scatter_pipeline,
            count_layout,
            prefix_sum_layout,
            scatter_layout,
            histogram_buffer,
            params_buffer,
            temp_buffer,
            count_bind_group_a: None,
            count_bind_group_b: None,
            prefix_sum_bind_group,
            scatter_bind_group_a: None,
            scatter_bind_group_b: None,
        }
    }
    
    /// Sort ParticleGridData array by cell_index using radix sort
    fn sort(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input_buffer: &wgpu::Buffer,
        output_buffer: &wgpu::Buffer,
        count: u32,
    ) {
        if count == 0 { return; }
        
        // Calculate number of passes needed (4 bits per pass)
        // For most cell indices, we need 2-3 passes max
        let max_cell_index = count;  // Worst case: as many cells as particles
        let max_bits = if max_cell_index > 0 {
            32 - max_cell_index.leading_zeros()
        } else {
            1
        };
        let num_passes = ((max_bits + 3) / 4).max(1);  // At least 1 pass
        
        // Create bind groups if needed
        if self.count_bind_group_a.is_none() {
            // A: input -> temp
            self.count_bind_group_a = Some(
                BindGroupBuilder::new(device, &self.count_layout)
                    .with_label("RadixSortCountBindGroupA")
                    .add_buffer(0, input_buffer)
                    .add_buffer(1, self.histogram_buffer.buffer())
                    .add_buffer(2, self.params_buffer.buffer())
                    .build(),
            );
            
            // B: temp -> output (for ping-pong)
            self.count_bind_group_b = Some(
                BindGroupBuilder::new(device, &self.count_layout)
                    .with_label("RadixSortCountBindGroupB")
                    .add_buffer(0, self.temp_buffer.buffer())
                    .add_buffer(1, self.histogram_buffer.buffer())
                    .add_buffer(2, self.params_buffer.buffer())
                    .build(),
            );
            
            self.scatter_bind_group_a = Some(
                BindGroupBuilder::new(device, &self.scatter_layout)
                    .with_label("RadixSortScatterBindGroupA")
                    .add_buffer(0, input_buffer)
                    .add_buffer(1, self.temp_buffer.buffer())
                    .add_buffer(2, self.histogram_buffer.buffer())
                    .add_buffer(3, self.params_buffer.buffer())
                    .build(),
            );
            
            self.scatter_bind_group_b = Some(
                BindGroupBuilder::new(device, &self.scatter_layout)
                    .with_label("RadixSortScatterBindGroupB")
                    .add_buffer(0, self.temp_buffer.buffer())
                    .add_buffer(1, output_buffer)
                    .add_buffer(2, self.histogram_buffer.buffer())
                    .add_buffer(3, self.params_buffer.buffer())
                    .build(),
            );
        }
        
        for pass in 0..num_passes {
            let bit_offset = pass * 4;
            
            // Determine which bind groups to use (ping-pong between buffers)
            let (count_bg, scatter_bg) = if pass % 2 == 0 {
                (
                    self.count_bind_group_a.as_ref().unwrap(),
                    self.scatter_bind_group_a.as_ref().unwrap(),
                )
            } else {
                (
                    self.count_bind_group_b.as_ref().unwrap(),
                    self.scatter_bind_group_b.as_ref().unwrap(),
                )
            };
            
            // Update params
            let params = RadixSortParams {
                count,
                bit_offset,
                _padding: [0, 0],
            };
            self.params_buffer.write(queue, &params);
            
            // Clear histogram
            encoder.clear_buffer(self.histogram_buffer.buffer(), 0, None);
            
            // Sub-pass 1: Count phase
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("RadixSortCount"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.count_pipeline);
                pass.set_bind_group(0, count_bg, &[]);
                pass.dispatch_workgroups(count.div_ceil(256), 1, 1);
            }
            
            // Sub-pass 2: Prefix sum phase
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("RadixSortPrefixSum"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.prefix_sum_pipeline);
                pass.set_bind_group(0, &self.prefix_sum_bind_group, &[]);
                pass.dispatch_workgroups(1, 1, 1);  // Only 16 elements
            }
            
            // Sub-pass 3: Scatter phase
            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("RadixSortScatter"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.scatter_pipeline);
                pass.set_bind_group(0, scatter_bg, &[]);
                pass.dispatch_workgroups(count.div_ceil(256), 1, 1);
            }
        }
        
        // If odd number of passes, data ends up in temp_buffer, need final copy
        if num_passes % 2 == 1 {
            encoder.copy_buffer_to_buffer(
                self.temp_buffer.buffer(),
                0,
                output_buffer,
                0,
                (count as usize * std::mem::size_of::<[u32; 2]>()) as u64,
            );
        }
    }
}

// ============================================================================
// Optimized Boids Behavior with Spatial Hash Grid (O(n) version)
// ============================================================================

#[repr(C)]
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

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GridBuildParams {
    bounds: f32,               // offset 0
    cell_size: f32,            // offset 4
    _padding1: [u32; 2],       // offset 8 (pad to 16)
    grid_dimensions: [u32; 3], // offset 16 (vec3 needs 16-byte alignment!)
    _padding_vec3: u32,        // offset 28 (pad vec3 to 16 bytes)
    particle_count: u32,       // offset 32
    total_cells: u32,          // offset 36
    _padding2: [u32; 2],       // offset 40 (pad to 48 bytes total)
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

    // Grid configuration
    cell_size: f32,
    grid_dimensions: [u32; 3],
    total_cells: u32,

    // GPU resources for spatial grid
    spatial_grid_buffer: StorageBuffer,
    sorted_particle_data_buffer: StorageBuffer,
    particle_grid_data_buffer: StorageBuffer,
    cell_counts_buffer: StorageBuffer,
    grid_params_buffer: UniformBuffer,

    // Radix sorter
    radix_sorter: RadixSort,

    // Compute pipelines for grid building
    compute_cell_indices_pipeline: wgpu::ComputePipeline,
    prefix_sum_pipeline: wgpu::ComputePipeline,

    // Bind group layouts
    cell_indices_layout: wgpu::BindGroupLayout,

    // Bind groups (recreated when particle buffer changes)
    cell_indices_bind_group: Option<wgpu::BindGroup>,
    prefix_sum_bind_group: wgpu::BindGroup,
}

impl OptimizedBoidsBehavior {
    /// Create a new optimized boids behavior with spatial hash grid acceleration.
    ///
    /// # Arguments
    /// * `device` - WGPU device for creating GPU resources
    /// * `max_particles` - Maximum number of particles the system will handle
    /// * `bounds` - Half-extent of the bounding box (e.g., 75.0 means -75 to +75)
    /// * `max_interaction_radius` - Largest interaction radius (cohesion, alignment, or separation)
    ///
    /// # Notes
    /// The cell size is automatically calculated as 1.5× the max interaction radius.
    /// This ensures all potential neighbors are captured while minimizing grid overhead.
    pub fn new(
        device: &wgpu::Device,
        max_particles: u32,
        bounds: f32,
        max_interaction_radius: f32,
    ) -> Self {
        // Cell size = 1.5x largest radius ensures all neighbors are captured
        let cell_size = max_interaction_radius * 1.5;

        // Calculate grid dimensions
        let grid_extent = bounds * 2.0;
        let grid_dim = (grid_extent / cell_size).ceil() as u32;
        let grid_dimensions = [grid_dim, grid_dim, grid_dim];
        let total_cells = grid_dim * grid_dim * grid_dim;

        log::info!(
            "Optimized Boids: Grid {}x{}x{} = {} cells, cell_size = {:.2}",
            grid_dim,
            grid_dim,
            grid_dim,
            total_cells,
            cell_size
        );

        // Create buffers
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

        // Create shaders
        let cell_indices_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("CellIndicesShader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/spatial_grid_build.wgsl").into(),
            ),
        });

        let prefix_sum_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("PrefixSumShader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("shaders/parallel_prefix_sum.wgsl").into(),
            ),
        });

        // Create bind group layouts
        let cell_indices_layout = BindGroupLayoutBuilder::new(device)
            .with_label("CellIndicesLayout")
            .add_storage_buffer(0, true) // particles (read-only)
            .add_uniform_buffer(1) // params
            .add_storage_buffer(2, false) // particle_grid_data (write)
            .add_storage_buffer(3, false) // cell_counts (write, atomic)
            .build();

        let prefix_sum_layout = BindGroupLayoutBuilder::new(device)
            .with_label("PrefixSumLayout")
            .add_storage_buffer(0, true) // cell_counts (read)
            .add_storage_buffer(1, false) // spatial_grid (write)
            .add_uniform_buffer(2) // params
            .build();

        // Create compute pipelines
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

        // Create prefix sum bind group (doesn't depend on particles_buffer)
        let prefix_sum_bind_group = BindGroupBuilder::new(device, &prefix_sum_layout)
            .with_label("PrefixSumBindGroup")
            .add_buffer(0, cell_counts_buffer.buffer())
            .add_buffer(1, spatial_grid_buffer.buffer())
            .add_buffer(2, grid_params_buffer.buffer())
            .build();

        // Create radix sorter
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

    /// Build spatial grid from particle positions.
    ///
    /// This must be called before the main boids update pass. It performs three stages:
    /// 1. Compute cell indices - Assigns each particle to a grid cell and counts particles per cell
    /// 2. Prefix sum - Calculates start indices for each cell in the sorted array
    /// 3. Radix sort - Sorts particles by cell for cache-coherent access (replaces atomic reordering)
    ///
    /// # Arguments
    /// * `device` - WGPU device
    /// * `queue` - WGPU queue for buffer updates
    /// * `encoder` - Command encoder to record compute passes
    /// * `particles_buffer` - The buffer containing particle data
    pub fn build_spatial_grid(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        particles_buffer: &wgpu::Buffer,
    ) {
        if self.particle_count == 0 {
            return;
        }

        // Update grid params if particle count changed
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

        // Recreate bind groups if needed (first time or particle buffer changed)
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

        // Clear cell counts
        encoder.clear_buffer(self.cell_counts_buffer.buffer(), 0, None);

        // Pass 1: Compute cell indices and count particles per cell
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ComputeCellIndices"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.compute_cell_indices_pipeline);
            pass.set_bind_group(0, self.cell_indices_bind_group.as_ref().unwrap(), &[]);
            pass.dispatch_workgroups(self.particle_count.div_ceil(256), 1, 1);
        }

        // Pass 2: Compute prefix sum for start indices
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ComputePrefixSum"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.prefix_sum_pipeline);
            pass.set_bind_group(0, &self.prefix_sum_bind_group, &[]);
            pass.dispatch_workgroups(1, 1, 1);
        }

        // Pass 3: Radix sort particles by cell index (replaces atomic reordering)
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
        include_str!("shaders/boids_optimized.wgsl")
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

    fn update_params(&self, queue: &wgpu::Queue, buffer: &wgpu::Buffer, dt: f32) {
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
            particle_count: self.particle_count,
            cell_size: self.cell_size,
            grid_dimensions: self.grid_dimensions,
            _padding: 0,
        };

        queue.write_buffer(buffer, 0, bytemuck::bytes_of(&params));
    }

    fn additional_bindings(&self, _device: &wgpu::Device) -> Vec<wgpu::BindGroupEntry<'_>> {
        vec![
            wgpu::BindGroupEntry {
                binding: 2,
                resource: self.spatial_grid_buffer.buffer().as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: self.sorted_particle_data_buffer.buffer().as_entire_binding(),
            },
        ]
    }

    fn additional_layout_entries(&self) -> Vec<wgpu::BindGroupLayoutEntry> {
        vec![
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::COMPUTE,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 3,
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_boids_default_values() {
        let boids = BoidsBehavior::default();
        assert_eq!(boids.separation_radius, 2.0);
        assert_eq!(boids.alignment_radius, 4.0);
        assert_eq!(boids.cohesion_radius, 4.0);
        assert_eq!(boids.separation_weight, 1.5);
        assert_eq!(boids.alignment_weight, 1.0);
        assert_eq!(boids.cohesion_weight, 1.0);
        assert_eq!(boids.max_speed, 5.0);
        assert_eq!(boids.max_force, 0.5);
        assert_eq!(boids.bounds, 20.0);
        assert_eq!(boids.particle_count, 0);
    }

    #[test]
    fn test_boids_set_particle_count() {
        let mut boids = BoidsBehavior::default();
        boids.set_particle_count(1000);
        assert_eq!(boids.particle_count, 1000);
    }

    #[test]
    fn test_physics_default_values() {
        let physics = PhysicsBehavior::default();
        assert_eq!(physics.drag, 0.1);
        assert_eq!(physics.turbulence_strength, 0.0);
        assert_eq!(physics.turbulence_frequency, 1.0);
        assert_eq!(physics.gravity, glam::Vec3::new(0.0, -9.81, 0.0));
    }

    #[test]
    fn test_boids_params_size() {
        // Ensure params struct has correct size for GPU alignment
        assert_eq!(std::mem::size_of::<BoidsParams>(), 48);
    }

    #[test]
    fn test_optimized_boids_params_size() {
        // Ensure optimized params struct has correct size for GPU alignment
        assert_eq!(std::mem::size_of::<OptimizedBoidsParams>(), 64);
    }

    #[test]
    fn test_grid_build_params_size() {
        // Ensure grid params struct has correct size for GPU alignment
        assert_eq!(std::mem::size_of::<GridBuildParams>(), 48);
    }

    #[test]
    fn test_starfield_params_size() {
        assert_eq!(std::mem::size_of::<StarfieldParams>(), 32);
    }

    #[test]
    fn test_physics_params_size() {
        assert_eq!(std::mem::size_of::<PhysicsParams>(), 32);
    }

    #[test]
    fn test_radix_sort_params_size() {
        assert_eq!(std::mem::size_of::<RadixSortParams>(), 16);
    }

    #[test]
    fn test_optimized_boids_grid_calculations() {
        // Test various grid configurations
        let test_cases = vec![
            (50.0, 10.0, 7),  // bounds=50, radius=10 -> 7x7x7 grid
            (100.0, 20.0, 7), // bounds=100, radius=20 -> 7x7x7 grid
            (75.0, 20.0, 5),  // bounds=75, radius=20 -> 5x5x5 grid
        ];

        for (bounds, radius, expected_dim) in test_cases {
            let cell_size = radius * 1.5;
            let grid_extent = bounds * 2.0;
            let grid_dim = ((grid_extent / cell_size) as f32).ceil() as u32;
            assert_eq!(
                grid_dim, expected_dim,
                "Failed for bounds={}, radius={}",
                bounds, radius
            );
        }
    }
}