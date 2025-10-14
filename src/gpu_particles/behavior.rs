pub trait ParticleBehavior: PlatformBehaviorBounds {
    fn shader_source(&self) -> &str;

    fn entry_point(&self) -> &str {
        "main"
    }

    fn create_params_buffer(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Buffer;

    fn update_params(&self, queue: &wgpu::Queue, buffer: &wgpu::Buffer, dt: f32, active_count: u32);

    fn additional_bindings(
        &self,
        _device: &wgpu::Device,
        _start_binding: u32,
    ) -> Vec<wgpu::BindGroupEntry<'_>> {
        Vec::new()
    }

    fn additional_layout_entries(&self, _start_binding: u32) -> Vec<wgpu::BindGroupLayoutEntry> {
        Vec::new()
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub trait PlatformBehaviorBounds: Send + Sync {}

#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + Sync> PlatformBehaviorBounds for T {}

#[cfg(target_arch = "wasm32")]
pub trait PlatformBehaviorBounds {}

#[cfg(target_arch = "wasm32")]
impl<T> PlatformBehaviorBounds for T {}
