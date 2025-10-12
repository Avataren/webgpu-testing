// src/renderer/compute_pass.rs

/// Helper for managing compute pass execution
pub struct ComputePass<'a> {
    pass: wgpu::ComputePass<'a>,
}

impl<'a> ComputePass<'a> {
    /// Begin a new compute pass
    pub fn begin(encoder: &'a mut wgpu::CommandEncoder, label: &str) -> Self {
        let pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some(label),
            timestamp_writes: None,
        });

        Self { pass }
    }

    /// Set the compute pipeline
    pub fn set_pipeline(&mut self, pipeline: &'a wgpu::ComputePipeline) -> &mut Self {
        self.pass.set_pipeline(pipeline);
        self
    }

    /// Set a bind group
    pub fn set_bind_group(
        &mut self,
        index: u32,
        bind_group: &'a wgpu::BindGroup,
        offsets: &[u32],
    ) -> &mut Self {
        self.pass.set_bind_group(index, bind_group, offsets);
        self
    }

    /// Dispatch workgroups
    pub fn dispatch_workgroups(&mut self, x: u32, y: u32, z: u32) -> &mut Self {
        self.pass.dispatch_workgroups(x, y, z);
        self
    }

    /// Dispatch workgroups with automatic calculation
    /// 
    /// Calculates workgroups based on total work size and workgroup size
    pub fn dispatch_auto(
        &mut self,
        total_x: u32,
        total_y: u32,
        total_z: u32,
        workgroup_size_x: u32,
        workgroup_size_y: u32,
        workgroup_size_z: u32,
    ) -> &mut Self {
        let x = total_x.div_ceil(workgroup_size_x);
        let y = total_y.div_ceil(workgroup_size_y);
        let z = total_z.div_ceil(workgroup_size_z);
        self.dispatch_workgroups(x, y, z)
    }

    /// End the compute pass explicitly (happens automatically on drop)
    pub fn end(self) {
        drop(self);
    }
}

/// Convenience function for running a simple compute dispatch
pub fn dispatch_compute(
    encoder: &mut wgpu::CommandEncoder,
    label: &str,
    pipeline: &wgpu::ComputePipeline,
    bind_groups: &[(u32, &wgpu::BindGroup)],
    workgroups: (u32, u32, u32),
) {
    let mut pass = ComputePass::begin(encoder, label);
    pass.set_pipeline(pipeline);
    
    for (index, bind_group) in bind_groups {
        pass.set_bind_group(*index, bind_group, &[]);
    }
    
    pass.dispatch_workgroups(workgroups.0, workgroups.1, workgroups.2);
}