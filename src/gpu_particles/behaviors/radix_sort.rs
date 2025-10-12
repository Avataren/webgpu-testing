use bytemuck::{Pod, Zeroable};

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

pub(super) struct RadixSort {
    count_pipeline: wgpu::ComputePipeline,
    prefix_sum_pipeline: wgpu::ComputePipeline,
    scatter_pipeline: wgpu::ComputePipeline,

    count_layout: wgpu::BindGroupLayout,
    scatter_layout: wgpu::BindGroupLayout,

    histogram_buffer: StorageBuffer,
    params_buffer: UniformBuffer,
    temp_buffer: StorageBuffer,

    count_bind_group_a: Option<wgpu::BindGroup>,
    count_bind_group_b: Option<wgpu::BindGroup>,
    prefix_sum_bind_group: wgpu::BindGroup,
    scatter_bind_group_a: Option<wgpu::BindGroup>,
    scatter_bind_group_b: Option<wgpu::BindGroup>,
}

impl RadixSort {
    pub fn new(device: &wgpu::Device, max_elements: u32) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("RadixSortShader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/radix_sort.wgsl").into()),
        });

        let histogram_buffer =
            StorageBuffer::with_capacity::<u32>(device, "RadixSortHistogram", 16);

        let temp_buffer = StorageBuffer::with_capacity::<[u32; 2]>(
            device,
            "RadixSortTemp",
            max_elements as usize,
        );

        let params = RadixSortParams {
            count: 0,
            bit_offset: 0,
            _padding: [0, 0],
        };
        let params_buffer = UniformBuffer::new(device, "RadixSortParams", &params);

        let count_layout = BindGroupLayoutBuilder::new(device)
            .with_label("RadixSortCountLayout")
            .add_storage_buffer(0, true)
            .add_storage_buffer(1, false)
            .add_uniform_buffer(2)
            .build();

        let prefix_sum_layout = BindGroupLayoutBuilder::new(device)
            .with_label("RadixSortPrefixSumLayout")
            .add_storage_buffer(0, false)
            .build();

        let scatter_layout = BindGroupLayoutBuilder::new(device)
            .with_label("RadixSortScatterLayout")
            .add_storage_buffer(0, true)
            .add_storage_buffer(1, false)
            .add_storage_buffer(2, false)
            .add_uniform_buffer(3)
            .build();

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

        let prefix_sum_bind_group = BindGroupBuilder::new(device, &prefix_sum_layout)
            .with_label("RadixSortPrefixSumBindGroup")
            .add_buffer(0, histogram_buffer.buffer())
            .build();

        Self {
            count_pipeline,
            prefix_sum_pipeline,
            scatter_pipeline,
            count_layout,
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

    pub fn sort(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        input_buffer: &wgpu::Buffer,
        output_buffer: &wgpu::Buffer,
        count: u32,
    ) {
        if count == 0 {
            return;
        }

        let max_cell_index = count;
        let max_bits = if max_cell_index > 0 {
            32 - max_cell_index.leading_zeros()
        } else {
            1
        };
        let num_passes = ((max_bits + 3) / 4).max(1);

        if self.count_bind_group_a.is_none() {
            self.count_bind_group_a = Some(
                BindGroupBuilder::new(device, &self.count_layout)
                    .with_label("RadixSortCountBindGroupA")
                    .add_buffer(0, input_buffer)
                    .add_buffer(1, self.histogram_buffer.buffer())
                    .add_buffer(2, self.params_buffer.buffer())
                    .build(),
            );

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

            let params = RadixSortParams {
                count,
                bit_offset,
                _padding: [0, 0],
            };
            self.params_buffer.write(queue, &params);

            encoder.clear_buffer(self.histogram_buffer.buffer(), 0, None);

            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("RadixSortCount"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.count_pipeline);
                pass.set_bind_group(0, count_bg, &[]);
                pass.dispatch_workgroups(count.div_ceil(256), 1, 1);
            }

            {
                let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("RadixSortPrefixSum"),
                    timestamp_writes: None,
                });
                pass.set_pipeline(&self.prefix_sum_pipeline);
                pass.set_bind_group(0, &self.prefix_sum_bind_group, &[]);
                pass.dispatch_workgroups(1, 1, 1);
            }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radix_sort_params_alignment() {
        assert_eq!(std::mem::size_of::<RadixSortParams>(), 16);
    }
}
