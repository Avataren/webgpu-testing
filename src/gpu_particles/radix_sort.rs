// // src/gpu_particles/radix_sort.rs

// use bytemuck::{Pod, Zeroable};
// use wgpu::util::DeviceExt;

// use crate::renderer::compute_resources::{
//     BindGroupBuilder, BindGroupLayoutBuilder, StorageBuffer, UniformBuffer,
// };
// use crate::renderer::ComputePipelineBuilder;

// #[repr(C)]
// #[derive(Clone, Copy, Pod, Zeroable)]
// struct SortParams {
//     count: u32,
//     bit_offset: u32,
// }

// pub struct RadixSort {
//     // Pipelines
//     count_pipeline: wgpu::ComputePipeline,
//     prefix_sum_pipeline: wgpu::ComputePipeline,
//     scatter_pipeline: wgpu::ComputePipeline,
    
//     // Layouts
//     count_layout: wgpu::BindGroupLayout,
//     prefix_sum_layout: wgpu::BindGroupLayout,
//     scatter_layout: wgpu::BindGroupLayout,
    
//     // Buffers
//     histogram_buffer: StorageBuffer,
//     params_buffer: UniformBuffer,
    
//     // Bind groups (recreated when buffers change)
//     count_bind_group: Option<wgpu::BindGroup>,
//     prefix_sum_bind_group: wgpu::BindGroup,
//     scatter_bind_group: Option<wgpu::BindGroup>,
// }

// impl RadixSort {
//     pub fn new(device: &wgpu::Device) -> Self {
//         // Create shader
//         let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
//             label: Some("RadixSortShader"),
//             source: wgpu::ShaderSource::Wgsl(
//                 include_str!("shaders/radix_sort.wgsl").into(),
//             ),
//         });
        
//         // Create histogram buffer (16 bins for 4-bit radix)
//         let histogram_buffer = StorageBuffer::with_capacity::<u32>(
//             device,
//             "RadixSortHistogram",
//             16,
//         );
        
//         // Create params buffer
//         let params = SortParams {
//             count: 0,
//             bit_offset: 0,
//         };
//         let params_buffer = UniformBuffer::new(device, "RadixSortParams", &params);
        
//         // Create bind group layouts
//         let count_layout = BindGroupLayoutBuilder::new(device)
//             .with_label("RadixSortCountLayout")
//             .add_storage_buffer(0, true)   // input_data
//             .add_storage_buffer(1, false)  // output_data (unused in count)
//             .add_storage_buffer(2, false)  // histogram (atomic writes)
//             .add_uniform_buffer(3)         // params
//             .build();
        
//         let prefix_sum_layout = BindGroupLayoutBuilder::new(device)
//             .with_label("RadixSortPrefixSumLayout")
//             .add_storage_buffer(0, false)  // histogram (read/write)
//             .build();
        
//         let scatter_layout = BindGroupLayoutBuilder::new(device)
//             .with_label("RadixSortScatterLayout")
//             .add_storage_buffer(0, true)   // input_data
//             .add_storage_buffer(1, false)  // output_data
//             .add_storage_buffer(2, false)  // histogram (atomic reads/writes)
//             .add_uniform_buffer(3)         // params
//             .build();
        
//         // Create pipelines
//         let count_pipeline = ComputePipelineBuilder::new()
//             .with_label("RadixSortCount")
//             .with_shader(&shader)
//             .with_entry_point("count_phase")
//             .with_bind_group_layout(&count_layout)
//             .build(device);
        
//         let prefix_sum_pipeline = ComputePipelineBuilder::new()
//             .with_label("RadixSortPrefixSum")
//             .with_shader(&shader)
//             .with_entry_point("prefix_sum_phase")
//             .with_bind_group_layout(&prefix_sum_layout)
//             .build(device);
        
//         let scatter_pipeline = ComputePipelineBuilder::new()
//             .with_label("RadixSortScatter")
//             .with_shader(&shader)
//             .with_entry_point("scatter_phase")
//             .with_bind_group_layout(&scatter_layout)
//             .build(device);
        
//         // Create prefix sum bind group (doesn't depend on data buffers)
//         let prefix_sum_bind_group = BindGroupBuilder::new(device, &prefix_sum_layout)
//             .with_label("RadixSortPrefixSumBindGroup")
//             .add_buffer(0, histogram_buffer.buffer())
//             .build();
        
//         Self {
//             count_pipeline,
//             prefix_sum_pipeline,
//             scatter_pipeline,
//             count_layout,
//             prefix_sum_layout,
//             scatter_layout,
//             histogram_buffer,
//             params_buffer,
//             count_bind_group: None,
//             prefix_sum_bind_group,
//             scatter_bind_group: None,
//         }
//     }
    
//     /// Sort ParticleGridData array by cell_index using radix sort
//     pub fn sort(
//         &mut self,
//         device: &wgpu::Device,
//         queue: &wgpu::Queue,
//         encoder: &mut wgpu::CommandEncoder,
//         input_buffer: &wgpu::Buffer,
//         output_buffer: &wgpu::Buffer,
//         count: u32,
//     ) {
//         if count == 0 { return; }
        
//         // We need to sort up to 32 bits (cell indices)
//         // With 4-bit radix, we need 8 passes (32 / 4 = 8)
//         // But for 125 cells (7 bits), we only need 2 passes
//         let max_bits = 32u32.saturating_sub(count.leading_zeros());
//         let num_passes = (max_bits + 3) / 4;  // Ceiling division by 4
        
//         for pass in 0..num_passes {
//             let bit_offset = pass * 4;
//             let is_last_pass = pass == num_passes - 1;
            
//             // Determine input/output for this pass (ping-pong between buffers)
//             let (pass_input, pass_output) = if pass % 2 == 0 {
//                 (input_buffer, output_buffer)
//             } else {
//                 (output_buffer, input_buffer)
//             };
            
//             // Update params
//             let params = SortParams { count, bit_offset };
//             self.params_buffer.write(queue, &params);
            
//             // Recreate bind groups if needed (first time or buffer changed)
//             if self.count_bind_group.is_none() {
//                 self.count_bind_group = Some(
//                     BindGroupBuilder::new(device, &self.count_layout)
//                         .with_label("RadixSortCountBindGroup")
//                         .add_buffer(0, pass_input)
//                         .add_buffer(1, pass_output)
//                         .add_buffer(2, self.histogram_buffer.buffer())
//                         .add_buffer(3, self.params_buffer.buffer())
//                         .build(),
//                 );
                
//                 self.scatter_bind_group = Some(
//                     BindGroupBuilder::new(device, &self.scatter_layout)
//                         .with_label("RadixSortScatterBindGroup")
//                         .add_buffer(0, pass_input)
//                         .add_buffer(1, pass_output)
//                         .add_buffer(2, self.histogram_buffer.buffer())
//                         .add_buffer(3, self.params_buffer.buffer())
//                         .build(),
//                 );
//             }
            
//             // Clear histogram
//             encoder.clear_buffer(self.histogram_buffer.buffer(), 0, None);
            
//             // Pass 1: Count phase
//             {
//                 let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
//                     label: Some("RadixSortCount"),
//                     timestamp_writes: None,
//                 });
//                 pass.set_pipeline(&self.count_pipeline);
//                 pass.set_bind_group(0, self.count_bind_group.as_ref().unwrap(), &[]);
//                 pass.dispatch_workgroups(count.div_ceil(256), 1, 1);
//             }
            
//             // Pass 2: Prefix sum phase
//             {
//                 let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
//                     label: Some("RadixSortPrefixSum"),
//                     timestamp_writes: None,
//                 });
//                 pass.set_pipeline(&self.prefix_sum_pipeline);
//                 pass.set_bind_group(0, &self.prefix_sum_bind_group, &[]);
//                 pass.dispatch_workgroups(1, 1, 1);  // Only 16 elements
//             }
            
//             // Pass 3: Scatter phase
//             {
//                 let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
//                     label: Some("RadixSortScatter"),
//                     timestamp_writes: None,
//                 });
//                 pass.set_pipeline(&self.scatter_pipeline);
//                 pass.set_bind_group(0, self.scatter_bind_group.as_ref().unwrap(), &[]);
//                 pass.dispatch_workgroups(count.div_ceil(256), 1, 1);
//             }
//         }
        
//         // If odd number of passes, data ends up in output_buffer, which is correct
//         // If even number of passes, data ends up in input_buffer, need one more copy
//         if num_passes % 2 == 0 {
//             encoder.copy_buffer_to_buffer(
//                 input_buffer,
//                 0,
//                 output_buffer,
//                 0,
//                 (count as usize * std::mem::size_of::<[u32; 2]>()) as u64,
//             );
//         }
//     }
// }