// src/renderer/compute_builder.rs

/// Builder for creating compute pipelines with sensible defaults
///
/// Reduces boilerplate when creating compute pipelines by providing a fluent API
pub struct ComputePipelineBuilder<'a> {
    device: &'a wgpu::Device,
    label: Option<&'a str>,
    shader: Option<&'a wgpu::ShaderModule>,
    entry_point: &'a str,
    bind_group_layouts: Vec<&'a wgpu::BindGroupLayout>,
    push_constant_ranges: Vec<wgpu::PushConstantRange>,
}

impl<'a> ComputePipelineBuilder<'a> {
    /// Create a new compute pipeline builder
    pub fn new(device: &'a wgpu::Device) -> Self {
        Self {
            device,
            label: None,
            shader: None,
            entry_point: "main",
            bind_group_layouts: Vec::new(),
            push_constant_ranges: Vec::new(),
        }
    }

    /// Set the pipeline label for debugging
    pub fn with_label(mut self, label: &'a str) -> Self {
        self.label = Some(label);
        self
    }

    /// Set the compute shader
    pub fn with_shader(mut self, shader: &'a wgpu::ShaderModule) -> Self {
        self.shader = Some(shader);
        self
    }

    /// Set the entry point (default: "main")
    pub fn with_entry_point(mut self, entry_point: &'a str) -> Self {
        self.entry_point = entry_point;
        self
    }

    /// Add a bind group layout
    pub fn with_bind_group_layout(mut self, layout: &'a wgpu::BindGroupLayout) -> Self {
        self.bind_group_layouts.push(layout);
        self
    }

    /// Add multiple bind group layouts
    pub fn with_bind_group_layouts(mut self, layouts: &[&'a wgpu::BindGroupLayout]) -> Self {
        self.bind_group_layouts.extend_from_slice(layouts);
        self
    }

    /// Add a push constant range
    pub fn with_push_constant_range(mut self, range: wgpu::PushConstantRange) -> Self {
        self.push_constant_ranges.push(range);
        self
    }

    /// Build the compute pipeline
    pub fn build(self) -> wgpu::ComputePipeline {
        let shader = self.shader.expect("Shader must be provided via with_shader()");

        let pipeline_layout = self.device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: self.label.map(|l| format!("{l} Layout")).as_deref(),
            bind_group_layouts: &self.bind_group_layouts,
            push_constant_ranges: &self.push_constant_ranges,
        });

        self.device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: self.label,
            layout: Some(&pipeline_layout),
            module: shader,
            entry_point: Some(self.entry_point),
            compilation_options: Default::default(),
            cache: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_uses_default_entry_point() {
        // Just ensure the builder compiles and has correct defaults
        let builder = ComputePipelineBuilder::new(unsafe { &*(std::ptr::null::<wgpu::Device>()) });
        assert_eq!(builder.entry_point, "main");
    }
}