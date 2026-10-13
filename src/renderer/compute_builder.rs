// src/renderer/compute_builder.rs

/// Builder for creating compute pipelines with sensible defaults
///
/// Reduces boilerplate when creating compute pipelines by providing a fluent API
pub struct ComputePipelineBuilder<'a> {
    label: Option<&'a str>,
    shader: Option<&'a wgpu::ShaderModule>,
    entry_point: &'a str,
    bind_group_layouts: Vec<Option<&'a wgpu::BindGroupLayout>>,
    immediate_size: u32,
}

impl<'a> ComputePipelineBuilder<'a> {
    /// Create a new compute pipeline builder
    pub fn new() -> Self {
        Self {
            label: None,
            shader: None,
            entry_point: "main",
            bind_group_layouts: Vec::new(),
            immediate_size: 0,
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
        self.bind_group_layouts.push(Some(layout));
        self
    }

    /// Add multiple bind group layouts
    pub fn with_bind_group_layouts(mut self, layouts: &[&'a wgpu::BindGroupLayout]) -> Self {
        self.bind_group_layouts
            .extend(layouts.iter().copied().map(Some));
        self
    }

    /// Reserve `size` bytes of immediate data (formerly push constants).
    /// Requires [`wgpu::Features::IMMEDIATES`].
    pub fn with_immediate_size(mut self, size: u32) -> Self {
        self.immediate_size = size;
        self
    }

    /// Build the compute pipeline
    pub fn build(self, device: &wgpu::Device) -> wgpu::ComputePipeline {
        let shader = self
            .shader
            .expect("Shader must be provided via with_shader()");
        let label = self.label;
        let layout_label = label.map(|l| format!("{l} Layout"));

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: layout_label.as_deref(),
            bind_group_layouts: &self.bind_group_layouts,
            immediate_size: self.immediate_size,
        });

        device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label,
            layout: Some(&pipeline_layout),
            module: shader,
            entry_point: Some(self.entry_point),
            compilation_options: Default::default(),
            cache: None,
        })
    }
}

impl<'a> Default for ComputePipelineBuilder<'a> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_uses_default_entry_point() {
        // Just ensure the builder compiles and has correct defaults
        let builder = ComputePipelineBuilder::new();
        assert_eq!(builder.entry_point, "main");
    }
}
