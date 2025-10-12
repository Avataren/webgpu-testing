pub trait ParticleBehavior: Send + Sync {
    fn shader_source(&self) -> &str;

    fn entry_point(&self) -> &str {
        "main"
    }

    fn create_params_buffer(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Buffer;

    fn update_params(&self, queue: &wgpu::Queue, buffer: &wgpu::Buffer, dt: f32);

    fn additional_bindings(&self, _device: &wgpu::Device) -> Vec<wgpu::BindGroupEntry<'_>> {
        Vec::new()
    }

    fn additional_layout_entries(&self) -> Vec<wgpu::BindGroupLayoutEntry> {
        Vec::new()
    }
}
