use crate::renderer::shader_source::wgsl;
use crate::renderer::{PipelineBuilder, ShaderBuilder};

use super::fullscreen_vertex;

pub struct DepthResolvePipeline {
    pub layout: wgpu::BindGroupLayout,
    pub pipeline: wgpu::RenderPipeline,
}

/// HDR -> display: AO, bloom, exposure, colour correction and tonemapping.
pub struct CompositePipeline {
    pub layout: wgpu::BindGroupLayout,
    pub pipeline: wgpu::RenderPipeline,
}

/// Display image -> surface: anti-aliasing, sharpening, grain and dither.
pub struct FinalPipeline {
    pub layout: wgpu::BindGroupLayout,
    pub pipeline: wgpu::RenderPipeline,
}

pub(super) fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

pub(super) fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

pub fn build_depth_resolve(
    device: &wgpu::Device,
    uniform_layout: &wgpu::BindGroupLayout,
    sample_count: u32,
) -> Option<DepthResolvePipeline> {
    if sample_count <= 1 {
        return None;
    }

    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("DepthResolveLayout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Depth,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: true,
            },
            count: None,
        }],
    });

    let depth_resolve_source = ShaderBuilder::new()
        .with_module(wgsl!("postprocess/fullscreen.wgsl"))
        .build(wgsl!("depth_resolve.wgsl"));

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("DepthResolveShader"),
        source: wgpu::ShaderSource::Wgsl(depth_resolve_source.into()),
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("DepthResolvePipelineLayout"),
        bind_group_layouts: &[Some(uniform_layout), Some(&layout)],
        immediate_size: 0,
    });

    let pipeline = PipelineBuilder::new(device, &pipeline_layout, &shader)
        .with_label("DepthResolvePipeline")
        .with_vertex_entry("vs_fullscreen")
        .with_fragment_entry("fs_resolve_depth")
        .with_depth_stencil(
            wgpu::TextureFormat::Depth32Float,
            true,
            wgpu::CompareFunction::Always,
        )
        .with_vertex_state(fullscreen_vertex(&shader))
        .with_no_culling()
        .build();

    Some(DepthResolvePipeline { layout, pipeline })
}

pub fn build_composite(
    device: &wgpu::Device,
    uniform_layout: &wgpu::BindGroupLayout,
    shader: &wgpu::ShaderModule,
    output_format: wgpu::TextureFormat,
) -> CompositePipeline {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("CompositeLayout"),
        entries: &[
            texture_entry(90),
            texture_entry(91),
            texture_entry(92),
            sampler_entry(93),
            texture_entry(96),
            wgpu::BindGroupLayoutEntry {
                binding: 95,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D3,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 94,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("CompositePipelineLayout"),
        bind_group_layouts: &[Some(uniform_layout), Some(&layout)],
        immediate_size: 0,
    });

    let pipeline = PipelineBuilder::new(device, &pipeline_layout, shader)
        .with_label("CompositePipeline")
        .with_vertex_entry("vs_fullscreen")
        .with_fragment_entry("fs_composite")
        .with_color_target(output_format, Some(wgpu::BlendState::REPLACE))
        .with_vertex_state(fullscreen_vertex(shader))
        .with_no_culling()
        .build();

    CompositePipeline { layout, pipeline }
}

pub fn build_final(
    device: &wgpu::Device,
    uniform_layout: &wgpu::BindGroupLayout,
    shader: &wgpu::ShaderModule,
    output_format: wgpu::TextureFormat,
) -> FinalPipeline {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("FinalLayout"),
        entries: &[texture_entry(100), sampler_entry(101)],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("FinalPipelineLayout"),
        bind_group_layouts: &[Some(uniform_layout), Some(&layout)],
        immediate_size: 0,
    });

    let pipeline = PipelineBuilder::new(device, &pipeline_layout, shader)
        .with_label("FinalPipeline")
        .with_vertex_entry("vs_fullscreen")
        .with_fragment_entry("fs_final")
        .with_color_target(output_format, Some(wgpu::BlendState::REPLACE))
        .with_vertex_state(fullscreen_vertex(shader))
        .with_no_culling()
        .build();

    FinalPipeline { layout, pipeline }
}
