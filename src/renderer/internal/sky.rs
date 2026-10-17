//! The procedural sky's GPU side (see `crate::sky`): its transmittance table and the
//! environment map it renders the sky into, which then lights and reflects the scene like
//! an HDR image would (with mips for rough reflections and the diffuse average). The
//! background draws the rest of the sky per pixel (environment_background.wgsl).

use crate::renderer::shader_source::wgsl;
use crate::renderer::uniforms::SkyUniform;
use crate::renderer::ShaderBuilder;
use crate::sky::SkyFrame;

/// The environment map's size: about a third of a degree per texel, fine enough for the
/// horizon's glow.
const MAP_WIDTH: u32 = 1024;
const MAP_HEIGHT: u32 = 512;
const LUT_WIDTH: u32 = 256;
const LUT_HEIGHT: u32 = 64;
const MULTIPLE_SCATTERING_SIZE: u32 = 32;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// How bright the moon's disc is drawn, as a share of what its light would make it: the
/// moonlight that lights the scene (itself far brighter than the real thing) would make
/// the disc thousands of times brighter than the moonlit walls, a white blot. At this
/// share a full moon glows against a dark night with its maria showing.
const MOON_DISC: f32 = 1e-3;
/// By day the disc is a pale one, about as bright as the blue sky round it.
const DAY_MOON: f32 = 0.35;

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The sky's uniform block for `frame`; `seconds` runs the stars' twinkling.
pub(crate) fn sky_uniform(frame: &SkyFrame, seconds: f32) -> SkyUniform {
    let settings = &frame.settings;
    // Night's light is far brighter than the real thing, so that night can be seen; by
    // day the sky outshines the stars, and the moon is a pale disc.
    let sun_altitude = frame.positions.sun.altitude as f32;
    let starlight = smoothstep(0.0, -12.0, sun_altitude);
    let moon_shine =
        MOON_DISC * (DAY_MOON + (1.0 - DAY_MOON) * smoothstep(4.0, -8.0, sun_altitude));
    let v4 = |v: glam::Vec3, w: f32| [v.x, v.y, v.z, w];
    let sun_light = glam::Vec3::splat(settings.sun_intensity.max(0.0));
    let tint = glam::Vec3::from_array(settings.night_tint).max(glam::Vec3::ZERO);
    let full_moon = tint * settings.moon_intensity.max(0.0);
    let celestial_north = frame.world_from_equatorial * glam::Vec3::Z;
    let moon_north = (celestial_north
        - frame.moon_direction * celestial_north.dot(frame.moon_direction))
    .try_normalize()
    .unwrap_or(glam::Vec3::Y);
    let zenith = crate::sky::transmittance(settings, 90.0).max(glam::Vec3::splat(1e-6));
    let columns = frame.world_from_equatorial;
    SkyUniform {
        sun: v4(frame.sun_direction, 1.0),
        sun_light: v4(sun_light, (settings.sun_size.max(0.01) * 0.5).to_radians()),
        sun_seen: v4(sun_light * frame.sun_transmittance, 0.0),
        moon: v4(
            frame.moon_direction,
            (settings.moon_size.max(0.01) * 0.5).to_radians(),
        ),
        moon_light: v4(
            frame.moon_irradiance,
            frame.positions.moon_illumination as f32,
        ),
        moon_seen: v4(full_moon * frame.moon_transmittance * moon_shine, 0.0),
        moon_north: v4(moon_north, 0.0),
        scattering: [
            settings.rayleigh.max(0.0),
            settings.haze.max(0.0),
            settings.ozone.max(0.0),
            settings.mie_anisotropy.clamp(0.0, 0.99),
        ],
        look: [
            settings.sky_intensity.max(0.0),
            settings.stars.max(0.0) * starlight,
            settings.milky_way.max(0.0) * starlight,
            settings.night_glow.max(0.0),
        ],
        ground: {
            let [r, g, b] = settings.ground_albedo;
            [r.max(0.0), g.max(0.0), b.max(0.0), seconds]
        },
        night_tint: v4(tint, 0.0),
        zenith_depth: v4(
            -glam::Vec3::new(zenith.x.ln(), zenith.y.ln(), zenith.z.ln()),
            0.0,
        ),
        celestial_x: v4(columns.x_axis, 0.0),
        celestial_y: v4(columns.y_axis, 0.0),
        celestial_z: v4(columns.z_axis, 0.0),
    }
}

/// The parts of the uniform the environment map depends on (not the twinkling clock).
fn map_inputs(sky: &SkyUniform) -> SkyUniform {
    SkyUniform {
        ground: [sky.ground[0], sky.ground[1], sky.ground[2], 0.0],
        ..*sky
    }
}

pub(crate) struct SkyResources {
    lut_view: wgpu::TextureView,
    lut: wgpu::Texture,
    multiple_view: wgpu::TextureView,
    multiple: wgpu::Texture,
    map: wgpu::Texture,
    map_view: wgpu::TextureView,
    /// One view per mip level, to render into and to read the level above from.
    mip_views: Vec<wgpu::TextureView>,
    mip_levels: u32,
    settings_group: wgpu::BindGroup,
    lut_group: wgpu::BindGroup,
    multiple_group: wgpu::BindGroup,
    transmittance: wgpu::RenderPipeline,
    multiple_scattering: wgpu::RenderPipeline,
    environment: wgpu::RenderPipeline,
    downsample: wgpu::RenderPipeline,
    downsample_groups: Vec<wgpu::BindGroup>,
    /// What the tables and the map were last rendered from (the air, and the ground's
    /// albedo for the multiple scattering).
    lut_inputs: Option<[f32; 7]>,
    map_inputs: Option<SkyUniform>,
}

impl SkyResources {
    /// The sky's textures and passes; `uniform` is the environment uniform buffer.
    pub(crate) fn new(device: &wgpu::Device, uniform: &wgpu::Buffer) -> Self {
        let texture = |label: &str, width: u32, height: u32, mips: u32| {
            device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: mips,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: FORMAT,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let mip_levels = MAP_WIDTH.max(MAP_HEIGHT).ilog2() + 1;
        let lut = texture("SkyTransmittance", LUT_WIDTH, LUT_HEIGHT, 1);
        let lut_view = lut.create_view(&Default::default());
        let multiple = texture(
            "SkyMultipleScattering",
            MULTIPLE_SCATTERING_SIZE,
            MULTIPLE_SCATTERING_SIZE,
            1,
        );
        let multiple_view = multiple.create_view(&Default::default());
        let map = texture("SkyEnvironment", MAP_WIDTH, MAP_HEIGHT, mip_levels);
        let map_view = map.create_view(&Default::default());
        let mip_views: Vec<_> = (0..mip_levels)
            .map(|level| {
                map.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("SkyEnvironmentMip"),
                    base_mip_level: level,
                    mip_level_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("SkySampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let wrap_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("SkyDownsampleSampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let sampler_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        };
        let settings_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SkySettingsLayout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SkyTextureLayout"),
            entries: &[texture_entry(0), sampler_entry(1)],
        });
        let multiple_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("SkyMultipleScatteringLayout"),
            entries: &[texture_entry(0)],
        });
        let multiple_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SkyMultipleScatteringGroup"),
            layout: &multiple_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&multiple_view),
            }],
        });
        let settings_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("SkySettings"),
            layout: &settings_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let texture_group = |label, view: &wgpu::TextureView, sampler: &wgpu::Sampler| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &texture_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                ],
            })
        };
        let lut_group = texture_group("SkyTransmittanceGroup", &lut_view, &sampler);
        let downsample_groups = mip_views[..mip_views.len() - 1]
            .iter()
            .map(|view| texture_group("SkyDownsampleGroup", view, &wrap_sampler))
            .collect();

        let source = ShaderBuilder::new()
            .with_module(wgsl!("sky.wgsl"))
            .build(wgsl!("sky_generate.wgsl"));
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("SkyGenerate"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let blit = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("SkyDownsample"),
            source: wgpu::ShaderSource::Wgsl(
                ShaderBuilder::new()
                    .build(include_str!("../blit.wgsl"))
                    .into(),
            ),
        });
        let pipeline = |label, layouts: &[Option<&wgpu::BindGroupLayout>], module, fragment| {
            let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: layouts,
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module,
                    entry_point: Some(fragment),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: FORMAT,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let transmittance = pipeline(
            "SkyTransmittancePipeline",
            &[Some(&settings_layout)],
            &module,
            "fs_transmittance",
        );
        let multiple_scattering = pipeline(
            "SkyMultipleScatteringPipeline",
            &[Some(&settings_layout), Some(&texture_layout)],
            &module,
            "fs_multiple_scattering",
        );
        let environment = pipeline(
            "SkyEnvironmentPipeline",
            &[
                Some(&settings_layout),
                Some(&texture_layout),
                Some(&multiple_layout),
            ],
            &module,
            "fs_environment",
        );
        let downsample = pipeline(
            "SkyDownsamplePipeline",
            &[Some(&texture_layout)],
            &blit,
            "fs_main",
        );

        Self {
            lut_view,
            lut,
            multiple_view,
            multiple,
            map,
            map_view,
            mip_views,
            mip_levels,
            settings_group,
            lut_group,
            multiple_group,
            transmittance,
            multiple_scattering,
            environment,
            downsample,
            downsample_groups,
            lut_inputs: None,
            map_inputs: None,
        }
    }

    pub(crate) fn fetchable_textures<'a>(
        &'a self,
        out: &mut Vec<crate::renderer::readback::FetchableTexture<'a>>,
    ) {
        use crate::renderer::readback::FetchableTexture;
        out.push(FetchableTexture::new(
            "sky.environment",
            "The procedural sky rendered into the environment map (equirectangular: u round \
             from +x, v from the zenith down; mip 0): the light the air and the ground send \
             from each direction (rgb, linear), without the sun's and moon's discs and the \
             stars",
            &self.map,
        ));
        out.push(FetchableTexture::new(
            "sky.transmittance",
            "The procedural sky's transmittance table: what the air lets through from a \
             height (v, ground to the top of the atmosphere) toward a direction (u, Bruneton's \
             layout), rgb",
            &self.lut,
        ));
        out.push(FetchableTexture::new(
            "sky.multiple_scattering",
            "The procedural sky's multiple scattering table: light scattered twice and more \
             (isotropic, for sunlight of 1) at a height (v, ground to top) for the sun's \
             zenith cosine (u, -1 to 1), rgb",
            &self.multiple,
        ));
    }

    pub(crate) fn map_view(&self) -> &wgpu::TextureView {
        &self.map_view
    }

    pub(crate) fn mip_levels(&self) -> u32 {
        self.mip_levels
    }

    /// Render the table and the map again where `sky` (as the uniform buffer now holds
    /// it) changed what they show. Returns whether the map was rendered.
    pub(crate) fn encode(&mut self, encoder: &mut wgpu::CommandEncoder, sky: &SkyUniform) -> bool {
        let [a, b, c, d] = sky.scattering;
        let lut_inputs = [a, b, c, d, sky.ground[0], sky.ground[1], sky.ground[2]];
        if self.lut_inputs != Some(lut_inputs) {
            self.lut_inputs = Some(lut_inputs);
            self.map_inputs = None;
            {
                let mut pass = begin(encoder, "SkyTransmittancePass", &self.lut_view);
                pass.set_pipeline(&self.transmittance);
                pass.set_bind_group(0, &self.settings_group, &[]);
                pass.draw(0..3, 0..1);
            }
            let mut pass = begin(encoder, "SkyMultipleScatteringPass", &self.multiple_view);
            pass.set_pipeline(&self.multiple_scattering);
            pass.set_bind_group(0, &self.settings_group, &[]);
            pass.set_bind_group(1, &self.lut_group, &[]);
            pass.draw(0..3, 0..1);
        }
        let inputs = map_inputs(sky);
        if self.map_inputs == Some(inputs) {
            return false;
        }
        self.map_inputs = Some(inputs);
        {
            let mut pass = begin(encoder, "SkyEnvironmentPass", &self.mip_views[0]);
            pass.set_pipeline(&self.environment);
            pass.set_bind_group(0, &self.settings_group, &[]);
            pass.set_bind_group(1, &self.lut_group, &[]);
            pass.set_bind_group(2, &self.multiple_group, &[]);
            pass.draw(0..3, 0..1);
        }
        for (level, group) in self.downsample_groups.iter().enumerate() {
            let mut pass = begin(encoder, "SkyEnvironmentMipPass", &self.mip_views[level + 1]);
            pass.set_pipeline(&self.downsample);
            pass.set_bind_group(0, group, &[]);
            pass.draw(0..3, 0..1);
        }
        true
    }
}

fn begin<'a>(
    encoder: &'a mut wgpu::CommandEncoder,
    label: &'a str,
    view: &'a wgpu::TextureView,
) -> wgpu::RenderPass<'a> {
    encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            depth_slice: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    })
}
