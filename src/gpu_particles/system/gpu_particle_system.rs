// src/gpu_particles/system/gpu_particle_system.rs - Non-blocking particle sorting
// Fixed for wgpu 0.27.0+ API

use super::super::pools::{acquire_particle_vec, acquire_spawn_request_vec};
use crate::asset::Mesh;
use crate::renderer::{
    CustomRenderContext, CustomRenderStage, Material, MaterialData, Renderer, SamplerFilterMode,
    ShadowPassStage,
};
use bytemuck::{bytes_of, cast_slice};
use glam::Mat4;
use wgpu::util::DeviceExt;

use super::super::{behavior::ParticleBehavior, emitter::ParticleEmitter, particle::Particle};
use super::{
    pipeline::create_render_pipeline, shadow::ParticleShadowResources,
    slot_allocator_v2::SlotAllocator, sorting::ParticleSorting,
};

const WORKGROUP_SIZE: u32 = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParticleRenderMode {
    Opaque,
    AlphaBlend,
    Additive,
}

impl ParticleRenderMode {
    fn default_depth_write(self) -> bool {
        matches!(self, Self::Opaque)
    }

    fn needs_sorting(self) -> bool {
        matches!(self, Self::AlphaBlend)
    }

    fn blend_state(self) -> Option<wgpu::BlendState> {
        match self {
            Self::Opaque => None,
            Self::AlphaBlend => Some(wgpu::BlendState::ALPHA_BLENDING),
            Self::Additive => Some(wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
            }),
        }
    }
}

impl Default for ParticleRenderMode {
    fn default() -> Self {
        Self::Opaque
    }
}

pub struct GpuParticleSystem {
    particles_buffer: wgpu::Buffer,
    params_buffer: wgpu::Buffer,

    compute_pipeline: wgpu::ComputePipeline,
    compute_bind_group: wgpu::BindGroup,
    dead_list_buffer: wgpu::Buffer,
    dead_list_size: u64,
    dead_list_readback: wgpu::Buffer,
    pending_readback: bool,

    render_pipeline: wgpu::RenderPipeline,
    render_bind_group: wgpu::BindGroup,
    particle_bind_group_layout: wgpu::BindGroupLayout,
    _material_buffer: wgpu::Buffer,

    shadow_resources: Option<ParticleShadowResources>,

    max_particles: u32,
    active_particles: u32,
    render_high_water: u32,
    workgroup_count: u32,

    emitters: Vec<ParticleEmitter>,
    last_emitter_transform: Mat4,
    slot_allocator: SlotAllocator,
    spawn_scratch: Vec<Particle>,
    dead_list_dirty: bool,

    render_format: wgpu::TextureFormat,
    render_sample_count: u32,
    sampler_filtering: SamplerFilterMode,
    blend_state: Option<wgpu::BlendState>,
    render_mode: ParticleRenderMode,
    casts_shadows: bool,

    sorting: ParticleSorting,
    depth_write_enabled: bool,
    pipeline_depth_write_state: bool,
}

impl GpuParticleSystem {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &Renderer,
        max_particles: u32,
        material: Material,
        behavior: &dyn ParticleBehavior,
    ) -> Self {
        let default_mode = if material.requires_separate_pass() {
            ParticleRenderMode::AlphaBlend
        } else {
            ParticleRenderMode::Opaque
        };

        Self::new_with_mode(
            device,
            queue,
            renderer,
            max_particles,
            material,
            behavior,
            default_mode,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_mode(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        renderer: &Renderer,
        max_particles: u32,
        material: Material,
        behavior: &dyn ParticleBehavior,
        render_mode: ParticleRenderMode,
    ) -> Self {
        let buffer_size = (max_particles as usize * std::mem::size_of::<Particle>()) as u64;
        let particles_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ParticlesBuffer"),
            size: buffer_size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });

        let params_buffer = behavior.create_params_buffer(device, queue);

        let dead_list_entries = max_particles as usize + 1;
        let dead_list_size = (dead_list_entries * std::mem::size_of::<u32>()) as u64;
        let dead_list_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ParticleDeadList"),
            size: dead_list_size,
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::COPY_SRC
                | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&dead_list_buffer, 0, bytes_of(&0u32));

        let dead_list_readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ParticleDeadListReadback"),
            size: dead_list_size,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let sampler_filtering = material.sampler_filtering();
        let blend_state = render_mode.blend_state();

        let depth_write_enabled = render_mode.default_depth_write();
        let needs_alpha_sorting = render_mode.needs_sorting();

        let material_data = MaterialData::from_material(&material);
        let material_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("ParticleMaterial"),
            contents: bytes_of(&material_data),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let compute_shader_source = behavior.build_shader();
        let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ParticleComputeShader"),
            source: wgpu::ShaderSource::Wgsl(compute_shader_source.into()),
        });

        let mut layout_entries = vec![
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
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ];

        layout_entries.push(wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: false },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        });

        let next_binding = 3;
        layout_entries.extend(behavior.additional_layout_entries(next_binding));

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ParticleComputeBindLayout"),
            entries: &layout_entries,
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ParticleComputePipelineLayout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let compute_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("ParticleComputePipeline"),
            layout: Some(&pipeline_layout),
            module: &compute_shader,
            entry_point: Some(behavior.entry_point()),
            compilation_options: Default::default(),
            cache: None,
        });

        let mut bind_entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: particles_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: params_buffer.as_entire_binding(),
            },
        ];

        bind_entries.push(wgpu::BindGroupEntry {
            binding: 2,
            resource: dead_list_buffer.as_entire_binding(),
        });

        bind_entries.extend(behavior.additional_bindings(device, next_binding));

        let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ParticleComputeBindGroup"),
            layout: &bind_group_layout,
            entries: &bind_entries,
        });

        let initial_stage = CustomRenderStage::BeforePostprocess;
        let render_format = renderer.color_format_for_stage(initial_stage);
        let render_sample_count = renderer.sample_count_for_stage(initial_stage);
        let uses_bindless = renderer.supports_bindless_textures();

        let particle_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ParticleRenderBindLayout"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

        let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ParticleRenderBindGroup"),
            layout: &particle_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: particles_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: material_buffer.as_entire_binding(),
                },
            ],
        });

        let render_pipeline = create_render_pipeline(
            device,
            renderer,
            &particle_bind_group_layout,
            render_format,
            render_sample_count,
            uses_bindless,
            sampler_filtering,
            blend_state,
            depth_write_enabled,
        );

        let workgroup_count = max_particles.div_ceil(WORKGROUP_SIZE);
        let sorting = ParticleSorting::new(device, max_particles, needs_alpha_sorting);

        Self {
            particles_buffer,
            params_buffer,
            compute_pipeline,
            compute_bind_group,
            dead_list_buffer,
            dead_list_size,
            dead_list_readback,
            pending_readback: false,
            render_pipeline,
            render_bind_group,
            particle_bind_group_layout,
            _material_buffer: material_buffer,
            shadow_resources: None,
            max_particles,
            active_particles: 0,
            render_high_water: 0,
            workgroup_count,
            emitters: Vec::new(),
            slot_allocator: SlotAllocator::new(max_particles),
            spawn_scratch: Vec::new(),
            dead_list_dirty: false,
            render_format,
            render_sample_count,
            sampler_filtering,
            blend_state,
            render_mode,
            casts_shadows: false,
            sorting,
            depth_write_enabled,
            pipeline_depth_write_state: depth_write_enabled,
            last_emitter_transform: Mat4::IDENTITY,
        }
    }

    pub fn set_depth_write_enabled(&mut self, enabled: bool) {
        if self.depth_write_enabled != enabled {
            self.depth_write_enabled = enabled;
        }
    }

    pub fn render_mode(&self) -> ParticleRenderMode {
        self.render_mode
    }

    pub fn add_emitter(&mut self, emitter: ParticleEmitter) {
        if self.emitters.is_empty() {
            self.last_emitter_transform = Mat4::from(emitter.world_transform());
        }

        self.emitters.push(emitter);
    }

    pub fn particles_buffer(&self) -> &wgpu::Buffer {
        &self.particles_buffer
    }

    // In src/gpu_particles/system/gpu_particle_system.rs
    // Replace the entire update method:

    pub fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        behavior: &dyn ParticleBehavior,
        dt: f32,
    ) {
        self.sorting
            .try_complete_readback(device, self.render_high_water);

        self.harvest_dead_particles(device);
        self.reset_dead_list(queue);

        self.spawn_scratch.clear();
        for emitter in &mut self.emitters {
            emitter.emit_into(dt, &mut self.spawn_scratch);
        }

        if !self.spawn_scratch.is_empty() {
            self.upload_new_particles(queue);
        }

        self.emitters.retain(|emitter| !emitter.is_complete());

        if let Some(emitter) = self.emitters.first() {
            self.last_emitter_transform = Mat4::from(emitter.world_transform());
        }

        let emitter_transform = self.last_emitter_transform;

        behavior.update_params(
            queue,
            &self.params_buffer,
            dt,
            self.active_particles,
            emitter_transform,
        );

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("ParticleComputePass"),
                timestamp_writes: None,
            });

            pass.set_pipeline(&self.compute_pipeline);
            pass.set_bind_group(0, &self.compute_bind_group, &[]);
            pass.dispatch_workgroups(self.workgroup_count, 1, 1);
        }

        encoder.copy_buffer_to_buffer(
            &self.dead_list_buffer,
            0,
            &self.dead_list_readback,
            0,
            self.dead_list_size,
        );

        self.pending_readback = true;
        self.dead_list_dirty = true;

        self.sorting
            .schedule_readback(encoder, &self.particles_buffer, self.render_high_water);
    }

    fn ensure_render_pipeline(
        &mut self,
        device: &wgpu::Device,
        renderer: &Renderer,
        stage: CustomRenderStage,
    ) {
        if matches!(stage, CustomRenderStage::Shadow(_)) {
            return;
        }

        let target_format = renderer.color_format_for_stage(stage);
        let target_sample_count = renderer.sample_count_for_stage(stage);

        if self.render_format == target_format
            && self.render_sample_count == target_sample_count
            && self.pipeline_depth_write_state == self.depth_write_enabled
        {
            return;
        }

        let uses_bindless = renderer.supports_bindless_textures();

        let pipeline = create_render_pipeline(
            device,
            renderer,
            &self.particle_bind_group_layout,
            target_format,
            target_sample_count,
            uses_bindless,
            self.sampler_filtering,
            self.blend_state,
            self.depth_write_enabled,
        );

        self.render_pipeline = pipeline;
        self.render_format = target_format;
        self.render_sample_count = target_sample_count;
        self.pipeline_depth_write_state = self.depth_write_enabled;
    }

    pub fn render(&mut self, ctx: &mut CustomRenderContext<'_>, mesh: &Mesh) {
        if self.render_high_water == 0 {
            return;
        }

        self.sorting
            .record_camera_position(ctx.renderer.camera_position());

        match ctx.stage {
            CustomRenderStage::BeforePostprocess | CustomRenderStage::AfterPostprocess => {
                self.ensure_render_pipeline(ctx.renderer.get_device(), ctx.renderer, ctx.stage);

                let mut pass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ParticleRenderPass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: ctx.color_view,
                        resolve_target: None,
                        depth_slice: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: ctx.depth_view,
                        depth_ops: Some(wgpu::Operations {
                            load: wgpu::LoadOp::Load,
                            store: wgpu::StoreOp::Store,
                        }),
                        stencil_ops: None,
                    }),
                    timestamp_writes: None,
                    occlusion_query_set: None,
                });

                pass.set_pipeline(&self.render_pipeline);
                pass.set_bind_group(0, ctx.renderer.camera_bind_group(), &[]);
                pass.set_bind_group(1, &self.render_bind_group, &[]);
                pass.set_bind_group(2, ctx.renderer.lights_bind_group(), &[]);
                pass.set_bind_group(3, ctx.renderer.textures_bind_group(), &[]);

                pass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
                pass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());

                if self.sorting.needs_alpha_sorting() && !self.sorting.sorted_indices().is_empty() {
                    for &particle_idx in self.sorting.sorted_indices() {
                        pass.draw_indexed(
                            0..mesh.index_count(),
                            0,
                            particle_idx..(particle_idx + 1),
                        );
                    }
                } else {
                    pass.draw_indexed(0..mesh.index_count(), 0, 0..self.render_high_water);
                }
            }
            CustomRenderStage::Shadow(stage_info) => {
                if !self.casts_shadows {
                    return;
                }

                if let Some(matrix) = ctx.shadow_view_proj() {
                    self.render_shadow(ctx, mesh, stage_info, matrix);
                }
            }
        }
    }

    fn render_shadow(
        &mut self,
        ctx: &mut CustomRenderContext<'_>,
        mesh: &Mesh,
        _stage: ShadowPassStage,
        view_proj: glam::Mat4,
    ) {
        if self.shadow_resources.is_none() {
            let resources = ParticleShadowResources::new(
                ctx.renderer.get_device(),
                &self.particle_bind_group_layout,
            );
            self.shadow_resources = Some(resources);
        }

        let resources = self.shadow_resources.as_mut().unwrap();
        resources.update_view_proj(ctx.renderer.get_queue(), view_proj);

        let mut pass = ctx.encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ParticleShadowPass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: ctx.depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_pipeline(resources.pipeline());
        pass.set_bind_group(0, resources.uniform_bind_group(), &[]);
        pass.set_bind_group(1, &self.render_bind_group, &[]);
        pass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
        pass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());
        pass.draw_indexed(0..mesh.index_count(), 0, 0..self.render_high_water);
    }

    pub fn active_particle_count(&self) -> u32 {
        self.active_particles
    }

    pub fn initialize_particles(&mut self, queue: &wgpu::Queue, particles: &[Particle]) {
        let count = particles.len().min(self.max_particles as usize);
        queue.write_buffer(&self.particles_buffer, 0, cast_slice(&particles[..count]));
        self.active_particles = count as u32;
        self.render_high_water = count as u32;
        self.slot_allocator
            .initialize_with_count(self.render_high_water);
        self.sorting.reset();
    }

    pub fn set_casts_shadows(&mut self, casts_shadows: bool) {
        self.casts_shadows = casts_shadows;
    }

    pub fn casts_shadows(&self) -> bool {
        self.casts_shadows
    }

    pub fn emitters_mut(&mut self) -> &mut Vec<ParticleEmitter> {
        &mut self.emitters
    }

    fn harvest_dead_particles(&mut self, device: &wgpu::Device) {
        if !self.dead_list_dirty || !self.pending_readback {
            self.dead_list_dirty = false;
            return;
        }

        let slice = self.dead_list_readback.slice(..self.dead_list_size);
        let status = std::sync::Arc::new(std::sync::Mutex::new(None));
        let status_clone = std::sync::Arc::clone(&status);

        slice.map_async(wgpu::MapMode::Read, move |result| {
            *status_clone.lock().unwrap() = Some(result);
        });

        if let Err(error) = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        }) {
            log::error!("Failed to poll device for particle readback: {error}");
            self.pending_readback = false;
            self.dead_list_dirty = false;
            return;
        }

        let result = status.lock().unwrap().take();
        match result {
            Some(Ok(())) => {
                let data = slice.get_mapped_range();
                self.process_dead_list_bytes(&data);
                drop(data);
                self.dead_list_readback.unmap();
            }
            Some(Err(error)) => {
                log::error!("Failed to map particle dead list: {error}");
            }
            None => {
                log::error!("Particle dead list mapping did not complete");
            }
        }

        self.pending_readback = false;
        self.dead_list_dirty = false;
    }

    fn reset_dead_list(&self, queue: &wgpu::Queue) {
        queue.write_buffer(&self.dead_list_buffer, 0, bytes_of(&0u32));
    }

    fn upload_new_particles(&mut self, queue: &wgpu::Queue) {
        if self.spawn_scratch.is_empty() {
            return;
        }

        let mut spawned = 0u32;
        let mut dropped = 0u32;

        let mut spawn_requests = acquire_spawn_request_vec();
        spawn_requests.clear();

        for &particle in &self.spawn_scratch {
            let Some(slot) = self.slot_allocator.allocate() else {
                dropped += 1;
                continue;
            };

            if slot >= self.max_particles {
                log::warn!("Attempted to spawn particle into out-of-range slot {slot}");
                dropped += 1;
                continue;
            }

            spawn_requests.push((slot, particle));
            self.active_particles += 1;
            spawned += 1;
        }

        if spawned == 0 {
            if dropped > 0 {
                log::debug!("Dropped {dropped} particles due to capacity limits");
            }
            self.spawn_scratch.clear();
            return;
        }

        if dropped > 0 {
            log::debug!("Dropped {dropped} particles due to capacity limits");
        }

        spawn_requests.sort_unstable_by_key(|(slot, _)| *slot);

        let particle_size = std::mem::size_of::<Particle>() as u64;
        let mut index = 0;

        while index < spawn_requests.len() {
            let start_slot = spawn_requests[index].0;

            let mut upload_batch = acquire_particle_vec();
            upload_batch.clear();
            upload_batch.push(spawn_requests[index].1);

            let mut end_slot = start_slot;
            index += 1;

            while index < spawn_requests.len() && spawn_requests[index].0 == end_slot + 1 {
                upload_batch.push(spawn_requests[index].1);
                end_slot = spawn_requests[index].0;
                index += 1;
            }

            let offset = start_slot as u64 * particle_size;
            queue.write_buffer(
                &self.particles_buffer,
                offset,
                bytemuck::cast_slice(&upload_batch),
            );
        }

        self.spawn_scratch.clear();

        // ✅ CRITICAL: Update render high water mark after spawning
        self.render_high_water = self.slot_allocator.high_water();
    }

    fn process_dead_list_bytes(&mut self, bytes: &[u8]) {
        if bytes.len() < std::mem::size_of::<u32>() {
            return;
        }

        let mut count_bytes = [0u8; 4];
        count_bytes.copy_from_slice(&bytes[..4]);
        let recorded = u32::from_ne_bytes(count_bytes);

        if recorded == 0 {
            return;
        }

        let available_indices = ((bytes.len() - 4) / 4) as u32;
        let to_process = recorded.min(available_indices);
        if to_process == 0 {
            return;
        }

        let mut reclaimed = 0u32;
        for chunk in bytes[4..]
            .chunks_exact(std::mem::size_of::<u32>())
            .take(to_process as usize)
        {
            let mut index_bytes = [0u8; 4];
            index_bytes.copy_from_slice(chunk);
            let index = u32::from_ne_bytes(index_bytes);

            if index >= self.max_particles {
                log::warn!(
                    "Discarding reclaimed particle index {} (max {})",
                    index,
                    self.max_particles
                );
                continue;
            }

            if self.slot_allocator.reclaim(index) {
                reclaimed += 1;
            } else {
                log::debug!("Ignoring duplicate reclaimed particle index {index}");
            }
        }

        if reclaimed > 0 {
            self.active_particles = self.active_particles.saturating_sub(reclaimed);
            self.slot_allocator.compact_trailing_free_slots();
            self.render_high_water = self.slot_allocator.high_water();
        }

        if recorded > to_process {
            log::warn!(
                "Dead list recorded {} indices but only processed {}",
                recorded,
                to_process
            );
        }
    }
}
