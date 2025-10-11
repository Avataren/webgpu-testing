use std::mem;
use std::num::NonZeroU64;

use bytemuck::{Pod, Zeroable};
use glam::Mat4;

use crate::asset::Assets;
use crate::renderer::internal::{DynamicObjectsBuffer, OrderedBatch, RenderContext};
use crate::renderer::lights::{
    LightsData, MAX_DIRECTIONAL_LIGHTS, MAX_POINT_LIGHTS, MAX_SPOT_LIGHTS,
};
use crate::renderer::material::Material;
use crate::renderer::RenderPass;
use crate::renderer::Vertex;

const POINT_SHADOW_FACE_COUNT: usize = 6;
const POINT_SHADOW_LAYERS: u32 = (MAX_POINT_LIGHTS * POINT_SHADOW_FACE_COUNT) as u32;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShadowViewUniform {
    view_proj: [[f32; 4]; 4],
}

struct ShadowArray {
    _texture: wgpu::Texture,
    array_view: wgpu::TextureView,
    layer_views: Vec<wgpu::TextureView>,
}

impl ShadowArray {
    fn new(device: &wgpu::Device, label: &str, layers: u32, size: u32) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size,
                height: size,
                depth_or_array_layers: layers.max(1),
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        let array_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(&format!("{label}ArrayView")),
            format: Some(wgpu::TextureFormat::Depth32Float),
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            aspect: wgpu::TextureAspect::All,
            base_mip_level: 0,
            mip_level_count: None,
            base_array_layer: 0,
            array_layer_count: Some(layers.max(1)),
            ..Default::default()
        });

        let mut layer_views = Vec::with_capacity(layers.max(1) as usize);
        for layer in 0..layers.max(1) {
            layer_views.push(texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some(&format!("{label}Layer{layer}")),
                format: Some(wgpu::TextureFormat::Depth32Float),
                dimension: Some(wgpu::TextureViewDimension::D2),
                aspect: wgpu::TextureAspect::All,
                base_mip_level: 0,
                mip_level_count: None,
                base_array_layer: layer,
                array_layer_count: Some(1),
                ..Default::default()
            }));
        }

        Self {
            _texture: texture,
            array_view,
            layer_views,
        }
    }

    fn layer_view(&self, index: usize) -> &wgpu::TextureView {
        let clamped = index.min(self.layer_views.len().saturating_sub(1));
        if clamped != index {
            log::warn!(
                "Shadow layer index {} clamped to {} (max: {})",
                index,
                clamped,
                self.layer_views.len() - 1
            );
        }
        &self.layer_views[clamped]
    }

    fn array_view(&self) -> &wgpu::TextureView {
        &self.array_view
    }
}

pub(crate) struct ShadowResources {
    directional: ShadowArray,
    spot: ShadowArray,
    point: ShadowArray,
    sampler: wgpu::Sampler,
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
    _uniform_layout: wgpu::BindGroupLayout,
    pipeline: wgpu::RenderPipeline,
    staging_buffer: wgpu::Buffer,
}

impl ShadowResources {
    pub(crate) fn new(
        device: &wgpu::Device,
        objects: &DynamicObjectsBuffer,
        shadow_map_size: u32,
    ) -> Self {
        let directional = ShadowArray::new(
            device,
            "DirectionalShadowMap",
            MAX_DIRECTIONAL_LIGHTS as u32,
            shadow_map_size,
        );
        let spot = ShadowArray::new(
            device,
            "SpotShadowMap",
            MAX_SPOT_LIGHTS as u32,
            shadow_map_size,
        );
        let point = ShadowArray::new(
            device,
            "PointShadowMap",
            POINT_SHADOW_LAYERS,
            shadow_map_size,
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("ShadowSampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            compare: Some(wgpu::CompareFunction::LessEqual),
            lod_min_clamp: 0.0,
            lod_max_clamp: 1.0,
            ..Default::default()
        });

        let uniform_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ShadowUniformLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: Some(
                        NonZeroU64::new(mem::size_of::<ShadowViewUniform>() as u64).unwrap(),
                    ),
                },
                count: None,
            }],
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ShadowUniformBuffer"),
            size: mem::size_of::<ShadowViewUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let max_shadows = (MAX_DIRECTIONAL_LIGHTS
            + MAX_SPOT_LIGHTS
            + MAX_POINT_LIGHTS * POINT_SHADOW_FACE_COUNT) as u64;
        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("ShadowStagingBuffer"),
            size: mem::size_of::<ShadowViewUniform>() as u64 * max_shadows,
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        log::info!("Created staging buffer:");
        log::info!("  Max shadows: {}", max_shadows);
        log::info!(
            "  Uniform size: {} bytes",
            mem::size_of::<ShadowViewUniform>()
        );
        log::info!(
            "  Total buffer size: {} bytes",
            mem::size_of::<ShadowViewUniform>() as u64 * max_shadows
        );
        log::info!(
            "  Breakdown: {} dir + {} spot + {} point faces",
            MAX_DIRECTIONAL_LIGHTS,
            MAX_SPOT_LIGHTS,
            MAX_POINT_LIGHTS * POINT_SHADOW_FACE_COUNT
        );

        let uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("ShadowUniformBindGroup"),
            layout: &uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ShadowShader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../../shader/shadow.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ShadowPipelineLayout"),
            bind_group_layouts: &[&uniform_layout, &objects.bind_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ShadowPipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[Vertex::layout()],
                compilation_options: Default::default(),
            },
            fragment: None,
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                cull_mode: Some(wgpu::Face::Back),
                front_face: wgpu::FrontFace::Ccw,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState {
                    constant: 2,
                    slope_scale: 2.0,
                    clamp: 0.0,
                },
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        Self {
            directional,
            spot,
            point,
            sampler,
            uniform_buffer,
            uniform_bind_group,
            _uniform_layout: uniform_layout,
            pipeline,
            staging_buffer,
        }
    }

    pub(crate) fn directional_array_view(&self) -> &wgpu::TextureView {
        self.directional.array_view()
    }

    pub(crate) fn spot_array_view(&self) -> &wgpu::TextureView {
        self.spot.array_view()
    }

    pub(crate) fn point_array_view(&self) -> &wgpu::TextureView {
        self.point.array_view()
    }

    pub(crate) fn sampler(&self) -> &wgpu::Sampler {
        &self.sampler
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render(
        &mut self,
        context: &RenderContext,
        encoder: &mut wgpu::CommandEncoder,
        assets: &Assets,
        batches: &[OrderedBatch],
        lights: &LightsData,
        objects: &DynamicObjectsBuffer,
        materials: &[Material],
    ) {
        if batches.is_empty() {
            return;
        }

        let queue = &context.queue;
        let uniform_size = mem::size_of::<ShadowViewUniform>() as u64;
        let mut staging_offset = 0u64;

        for shadow in lights
            .directional_shadows()
            .iter()
            .take(MAX_DIRECTIONAL_LIGHTS)
        {
            if shadow.params[0] == 0.0 {
                continue;
            }
            let matrix = Mat4::from_cols_array_2d(&shadow.view_proj);
            let uniform = ShadowViewUniform {
                view_proj: matrix.to_cols_array_2d(),
            };
            queue.write_buffer(
                &self.staging_buffer,
                staging_offset,
                bytemuck::bytes_of(&uniform),
            );
            staging_offset += uniform_size;
        }

        let spot_start_offset = staging_offset;
        for shadow in lights.spot_shadows().iter().take(MAX_SPOT_LIGHTS) {
            if shadow.params[0] == 0.0 {
                continue;
            }
            let matrix = Mat4::from_cols_array_2d(&shadow.view_proj);
            let uniform = ShadowViewUniform {
                view_proj: matrix.to_cols_array_2d(),
            };
            queue.write_buffer(
                &self.staging_buffer,
                staging_offset,
                bytemuck::bytes_of(&uniform),
            );
            staging_offset += uniform_size;
        }

        let point_start_offset = staging_offset;
        for shadow in lights.point_shadows().iter().take(MAX_POINT_LIGHTS) {
            if shadow.params[0] == 0.0 {
                continue;
            }

            for face in 0..POINT_SHADOW_FACE_COUNT {
                let matrix = Mat4::from_cols_array_2d(&shadow.view_proj[face]);
                let uniform = ShadowViewUniform {
                    view_proj: matrix.to_cols_array_2d(),
                };
                queue.write_buffer(
                    &self.staging_buffer,
                    staging_offset,
                    bytemuck::bytes_of(&uniform),
                );
                staging_offset += uniform_size;
            }
        }

        staging_offset = 0;
        for (index, shadow) in lights
            .directional_shadows()
            .iter()
            .enumerate()
            .take(MAX_DIRECTIONAL_LIGHTS)
        {
            if shadow.params[0] == 0.0 {
                continue;
            }

            encoder.copy_buffer_to_buffer(
                &self.staging_buffer,
                staging_offset,
                &self.uniform_buffer,
                0,
                uniform_size,
            );

            self.render_pass(
                encoder,
                self.directional.layer_view(index),
                assets,
                batches,
                objects,
                materials,
            );

            staging_offset += uniform_size;
        }

        let mut spot_staging_offset = spot_start_offset;
        for (index, shadow) in lights
            .spot_shadows()
            .iter()
            .enumerate()
            .take(MAX_SPOT_LIGHTS)
        {
            if shadow.params[0] == 0.0 {
                continue;
            }

            encoder.copy_buffer_to_buffer(
                &self.staging_buffer,
                spot_staging_offset,
                &self.uniform_buffer,
                0,
                uniform_size,
            );

            self.render_pass(
                encoder,
                self.spot.layer_view(index),
                assets,
                batches,
                objects,
                materials,
            );

            spot_staging_offset += uniform_size;
        }

        let mut point_staging_offset = point_start_offset;

        for (index, shadow) in lights
            .point_shadows()
            .iter()
            .enumerate()
            .take(MAX_POINT_LIGHTS)
        {
            if shadow.params[0] == 0.0 {
                continue;
            }

            for face in 0..POINT_SHADOW_FACE_COUNT {
                let layer_index = index * POINT_SHADOW_FACE_COUNT + face;

                encoder.copy_buffer_to_buffer(
                    &self.staging_buffer,
                    point_staging_offset,
                    &self.uniform_buffer,
                    0,
                    uniform_size,
                );

                self.render_pass(
                    encoder,
                    self.point.layer_view(layer_index),
                    assets,
                    batches,
                    objects,
                    materials,
                );

                point_staging_offset += uniform_size;
            }
        }
    }

    fn render_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        assets: &Assets,
        batches: &[OrderedBatch],
        objects: &DynamicObjectsBuffer,
        materials: &[Material],
    ) {
        if batches.is_empty() {
            return;
        }
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("ShadowPass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.uniform_bind_group, &[]);
        pass.set_bind_group(1, &objects.bind_group, &[]);

        for batch in batches {
            if matches!(batch.pass, RenderPass::Transparent | RenderPass::Overlay) {
                continue;
            }
            let Some(mesh) = assets.meshes.get(batch.mesh) else {
                continue;
            };

            let instance_count = batch.instances.len() as u32;
            pass.set_vertex_buffer(0, mesh.vertex_buffer().slice(..));
            pass.set_index_buffer(mesh.index_buffer().slice(..), mesh.index_format());
            let mut current_range_start: Option<u32> = None;

            for (local_index, instance) in batch.instances.iter().enumerate() {
                let global_index = batch.first_instance + local_index as u32;
                let material_index = instance.material_index as usize;
                let Some(material) = materials.get(material_index) else {
                    log::warn!(
                        "Material index {} out of bounds during shadow rendering ({} materials)",
                        material_index,
                        materials.len()
                    );
                    if let Some(start) = current_range_start.take() {
                        pass.draw_indexed(0..mesh.index_count(), 0, start..global_index);
                    }
                    continue;
                };
                if material.is_unlit() {
                    if let Some(start) = current_range_start.take() {
                        pass.draw_indexed(0..mesh.index_count(), 0, start..global_index);
                    }
                } else if current_range_start.is_none() {
                    current_range_start = Some(global_index);
                }
            }

            if let Some(start) = current_range_start.take() {
                pass.draw_indexed(
                    0..mesh.index_count(),
                    0,
                    start..(batch.first_instance + instance_count),
                );
            }
        }
    }
}
