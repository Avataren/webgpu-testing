use std::borrow::Cow;

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Quat, Vec3};
use wgpu::util::DeviceExt;

use crate::renderer::{ObjectData, Renderer};

const WORKGROUP_SIZE: u32 = 128;

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParticleState {
    position_speed: [f32; 4],
    rotation: [f32; 4],
    angular_axis_speed: [f32; 4],
    scale_seed: [u32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct GpuParticleParams {
    dt: f32,
    near_plane: f32,
    far_plane: f32,
    far_reset_band: f32,
    field_half_size: f32,
    min_radius: f32,
    speed_min: f32,
    speed_max: f32,
    spin_min: f32,
    spin_max: f32,
    scale_min: f32,
    scale_max: f32,
    base_instance: u32,
    particle_count: u32,
    _padding: [u32; 2],
}

/// Immutable configuration parameters for the particle simulation.
#[derive(Clone)]
pub struct ParticleFieldSettings {
    pub near_plane: f32,
    pub far_plane: f32,
    pub far_reset_band: f32,
    pub field_half_size: f32,
    pub min_radius: f32,
    pub speed_range: std::ops::Range<f32>,
    pub spin_range: std::ops::Range<f32>,
    pub scale_range: std::ops::Range<f32>,
}

impl ParticleFieldSettings {
    pub fn speed_min(&self) -> f32 {
        self.speed_range.start
    }

    pub fn speed_max(&self) -> f32 {
        self.speed_range.end
    }

    pub fn spin_min(&self) -> f32 {
        self.spin_range.start
    }

    pub fn spin_max(&self) -> f32 {
        self.spin_range.end
    }

    pub fn scale_min(&self) -> f32 {
        self.scale_range.start
    }

    pub fn scale_max(&self) -> f32 {
        self.scale_range.end
    }
}

/// CPU-side description of a particle's initial state.
#[derive(Clone, Copy)]
pub struct ParticleInit {
    pub position: Vec3,
    pub speed: f32,
    pub rotation: Quat,
    pub angular_axis: Vec3,
    pub angular_speed: f32,
    pub scale: f32,
    pub seed: u32,
}

impl ParticleInit {
    fn as_state(&self) -> ParticleState {
        ParticleState {
            position_speed: [
                self.position.x,
                self.position.y,
                self.position.z,
                self.speed,
            ],
            rotation: [
                self.rotation.x,
                self.rotation.y,
                self.rotation.z,
                self.rotation.w,
            ],
            angular_axis_speed: [
                self.angular_axis.x,
                self.angular_axis.y,
                self.angular_axis.z,
                self.angular_speed,
            ],
            scale_seed: [self.scale.to_bits(), self.seed, 0, 0],
        }
    }

    fn as_object_data(&self, material_index: u32) -> ObjectData {
        let model = Mat4::from_scale_rotation_translation(
            Vec3::splat(self.scale),
            self.rotation,
            self.position,
        );
        ObjectData::new(model, material_index)
    }
}

pub struct GpuParticleSystem {
    pipeline: wgpu::ComputePipeline,
    bind_group: wgpu::BindGroup,
    params: GpuParticleParams,
    params_buffer: wgpu::Buffer,
    _state_buffer: wgpu::Buffer,
    workgroup_count: u32,
}

impl GpuParticleSystem {
    pub fn new(
        renderer: &mut Renderer,
        particle_count: u32,
        base_instance: u32,
        material_index: u32,
        settings: ParticleFieldSettings,
        initial_particles: &[ParticleInit],
    ) -> Self {
        assert_eq!(particle_count as usize, initial_particles.len());

        renderer.reserve_object_capacity(base_instance + particle_count);

        let device = renderer.get_device();
        let queue = renderer.get_queue();
        let state_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ParticleStateBuffer"),
            size: (particle_count as usize * std::mem::size_of::<ParticleState>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let params = GpuParticleParams {
            dt: 0.0,
            near_plane: settings.near_plane,
            far_plane: settings.far_plane,
            far_reset_band: settings.far_reset_band,
            field_half_size: settings.field_half_size,
            min_radius: settings.min_radius,
            speed_min: settings.speed_min(),
            speed_max: settings.speed_max(),
            spin_min: settings.spin_min(),
            spin_max: settings.spin_max(),
            scale_min: settings.scale_min(),
            scale_max: settings.scale_max(),
            base_instance,
            particle_count,
            _padding: [0; 2],
        };

        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ParticleParams"),
            contents: bytemuck::bytes_of(&params),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let initial_states: Vec<ParticleState> = initial_particles
            .iter()
            .map(ParticleInit::as_state)
            .collect();
        queue.write_buffer(&state_buffer, 0, bytemuck::cast_slice(&initial_states));

        let object_data: Vec<ObjectData> = initial_particles
            .iter()
            .map(|particle| particle.as_object_data(material_index))
            .collect();

        let object_stride = std::mem::size_of::<ObjectData>() as u64;
        queue.write_buffer(
            renderer.objects_buffer(),
            base_instance as u64 * object_stride,
            bytemuck::cast_slice(&object_data),
        );

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("GpuParticleUpdate"),
            source: wgpu::ShaderSource::Wgsl(Cow::Borrowed(include_str!(
                "shader/gpu_particles.wgsl"
            ))),
        });

        let bind_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("GpuParticleBindLayout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: false },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: Some(
                            std::num::NonZeroU64::new(
                                std::mem::size_of::<GpuParticleParams>() as u64
                            )
                            .unwrap(),
                        ),
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("GpuParticlePipelineLayout"),
            bind_group_layouts: &[&bind_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("GpuParticlePipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("update_particles"),
            compilation_options: Default::default(),
            cache: None,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("GpuParticleBindGroup"),
            layout: &bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: state_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: renderer.objects_buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: params_buffer.as_entire_binding(),
                },
            ],
        });

        let workgroup_count = particle_count.div_ceil(WORKGROUP_SIZE);

        Self {
            pipeline,
            bind_group,
            params,
            params_buffer,
            _state_buffer: state_buffer,
            workgroup_count,
        }
    }

    pub fn update(&mut self, renderer: &mut Renderer, dt: f32) {
        if dt <= f32::EPSILON {
            return;
        }

        self.params.dt = dt;
        renderer
            .get_queue()
            .write_buffer(&self.params_buffer, 0, bytemuck::bytes_of(&self.params));

        let mut encoder =
            renderer
                .get_device()
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("GpuParticleEncoder"),
                });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("GpuParticlePass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.dispatch_workgroups(self.workgroup_count, 1, 1);
        }

        renderer.get_queue().submit(Some(encoder.finish()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shader_like_model_matrix(position: Vec3, rotation: Quat, scale: f32) -> Mat4 {
        let x2 = rotation.x + rotation.x;
        let y2 = rotation.y + rotation.y;
        let z2 = rotation.z + rotation.z;

        let xx = rotation.x * x2;
        let xy = rotation.x * y2;
        let xz = rotation.x * z2;
        let yy = rotation.y * y2;
        let yz = rotation.y * z2;
        let zz = rotation.z * z2;
        let wx = rotation.w * x2;
        let wy = rotation.w * y2;
        let wz = rotation.w * z2;

        let col0 = Vec3::new(1.0 - (yy + zz), xy + wz, xz - wy) * scale;
        let col1 = Vec3::new(xy - wz, 1.0 - (xx + zz), yz + wx) * scale;
        let col2 = Vec3::new(xz + wy, yz - wx, 1.0 - (xx + yy)) * scale;

        Mat4::from_cols(
            col0.extend(0.0),
            col1.extend(0.0),
            col2.extend(0.0),
            position.extend(1.0),
        )
    }

    #[test]
    fn shader_model_matches_cpu() {
        let position = Vec3::new(1.0, -2.0, 3.5);
        let rotation = Quat::from_xyzw(0.3, -0.4, 0.1, 0.85).normalize();
        let scale = 0.75;

        let expected =
            Mat4::from_scale_rotation_translation(Vec3::splat(scale), rotation, position);
        let shader_version = shader_like_model_matrix(position, rotation, scale);

        let diff = expected - shader_version;
        for element in diff.to_cols_array() {
            assert!(element.abs() < 1e-5);
        }
    }
}
