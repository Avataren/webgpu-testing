use crate::renderer::{PipelineBuilder, ShaderBuilder};

use super::fullscreen_vertex;

pub struct ColorGradingPipeline {
    pub layout: wgpu::BindGroupLayout,
    pub pipeline: wgpu::RenderPipeline,
}

pub struct DepthResolvePipeline {
    pub layout: wgpu::BindGroupLayout,
    pub pipeline: wgpu::RenderPipeline,
}

pub struct CompositePipeline {
    pub layout: wgpu::BindGroupLayout,
    pub pipeline: wgpu::RenderPipeline,
}

pub fn build_color_grading(
    device: &wgpu::Device,
    uniform_layout: &wgpu::BindGroupLayout,
    shader: &wgpu::ShaderModule,
    scene_format: wgpu::TextureFormat,
) -> ColorGradingPipeline {
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("ColorGradingLayout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 10,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 11,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("ColorGradingPipelineLayout"),
        bind_group_layouts: &[uniform_layout, &layout],
        push_constant_ranges: &[],
    });

    let pipeline = PipelineBuilder::new(device, &pipeline_layout, shader)
        .with_label("ColorGradingPipeline")
        .with_vertex_entry("vs_fullscreen")
        .with_fragment_entry("fs_color_adjust")
        .with_color_target(scene_format, Some(wgpu::BlendState::REPLACE))
        .with_vertex_state(fullscreen_vertex(shader))
        .with_no_culling()
        .build();

    ColorGradingPipeline { layout, pipeline }
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

    let depth_resolve_source =
        ShaderBuilder::new().build(include_str!("../../shader/depth_resolve.wgsl"));

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("DepthResolveShader"),
        source: wgpu::ShaderSource::Wgsl(depth_resolve_source.into()),
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("DepthResolvePipelineLayout"),
        bind_group_layouts: &[uniform_layout, &layout],
        push_constant_ranges: &[],
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
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Depth,
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 50,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 51,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 52,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 53,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });

    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("CompositePipelineLayout"),
        bind_group_layouts: &[uniform_layout, &layout],
        push_constant_ranges: &[],
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
