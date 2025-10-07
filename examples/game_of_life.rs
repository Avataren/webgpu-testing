use std::cell::RefCell;
use std::rc::Rc;

use glam::{Quat, Vec3};
use wgpu_cube::app::{AppBuilder, GpuUpdateContext, StartupContext, UpdateContext};
use wgpu_cube::asset::Handle;
use wgpu_cube::renderer::{Material, Texture};
use wgpu_cube::scene::components::{Billboard, BillboardOrientation, BillboardSpace};
use wgpu_cube::scene::{EntityBuilder, Transform};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const GRID_WIDTH: u32 = 256;
const GRID_HEIGHT: u32 = 256;
const STEP_INTERVAL: f64 = 0.05;
const WORKGROUP_SIZE: u32 = 8;

fn build_app() -> AppBuilder {
    let mut builder = AppBuilder::new();
    let state: Rc<RefCell<Option<GameOfLifeState>>> = Rc::new(RefCell::new(None));

    {
        let state_ref = state.clone();
        builder.add_startup_system(move |ctx| {
            let gol_state = GameOfLifeState::new(ctx, GRID_WIDTH, GRID_HEIGHT, STEP_INTERVAL);
            spawn_billboard(
                ctx,
                gol_state.display_texture_handle(),
                GRID_WIDTH,
                GRID_HEIGHT,
            );
            configure_camera(ctx);
            *state_ref.borrow_mut() = Some(gol_state);
        });
    }

    {
        let state_ref = state.clone();
        builder.add_gpu_system(move |ctx| {
            if let Some(state) = state_ref.borrow_mut().as_mut() {
                state.update(ctx);
            }
        });
    }

    builder.add_system(orbit_camera(6.5, 2.5, 0.2));
    builder
}

fn configure_camera(ctx: &mut StartupContext<'_>) {
    let camera = ctx.scene.camera_mut();
    camera.eye = Vec3::new(0.0, 0.0, 7.0);
    camera.target = Vec3::ZERO;
    camera.up = Vec3::Y;
}

fn orbit_camera(
    radius: f32,
    height: f32,
    speed: f32,
) -> Box<dyn for<'a> FnMut(&mut UpdateContext<'a>) + 'static> {
    Box::new(move |ctx: &mut UpdateContext<'_>| {
        let t = ctx.scene.time() as f32 * speed;
        let camera = ctx.scene.camera_mut();
        camera.eye = Vec3::new(t.cos() * radius, height, t.sin() * radius);
        camera.target = Vec3::ZERO;
        camera.up = Vec3::Y;
    })
}

fn spawn_billboard(
    ctx: &mut StartupContext<'_>,
    texture_handle: Handle<Texture>,
    width: u32,
    height: u32,
) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    let (vertices, indices) = wgpu_cube::renderer::quad_mesh();
    let mesh = renderer.create_mesh(&vertices, &indices);
    let mesh_handle = scene.assets.meshes.insert(mesh);

    let scale_x = (width as f32) / 64.0;
    let scale_y = (height as f32) / 64.0;

    let entity = EntityBuilder::new(&mut scene.world)
        .with_name("Game of Life Board")
        .with_transform(Transform::from_trs(
            Vec3::ZERO,
            Quat::IDENTITY,
            Vec3::new(scale_x, scale_y, 1.0),
        ))
        .with_mesh(mesh_handle)
        .with_material(
            Material::pbr()
                .with_unlit()
                .with_base_color_texture(texture_handle.index() as u32),
        )
        .visible(true)
        .spawn();

    scene
        .world
        .insert(
            entity,
            (Billboard::new(BillboardOrientation::FaceCamera).with_space(BillboardSpace::World),),
        )
        .expect("failed to add billboard component");
}

struct GameOfLifeState {
    bind_group: wgpu::BindGroup,
    pipeline: wgpu::ComputePipeline,
    scratch_texture: Texture,
    display_handle: Handle<Texture>,
    dispatch_x: u32,
    dispatch_y: u32,
    extent: wgpu::Extent3d,
    accumulator: f64,
    step_interval: f64,
}

impl GameOfLifeState {
    fn new(ctx: &mut StartupContext<'_>, width: u32, height: u32, step_interval: f64) -> Self {
        let mut initial_data = vec![0u8; (width * height * 4) as usize];
        generate_initial_pattern(&mut initial_data, width, height);

        let (display_texture, scratch_texture, bind_group, pipeline, dispatch_x, dispatch_y) = {
            let device = ctx.renderer.get_device();
            let queue = ctx.renderer.get_queue();

            let display_texture =
                Texture::storage_rgba8(device, width, height, Some("Game of Life"));
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &display_texture.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &initial_data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
                wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
            );

            let scratch_texture =
                Texture::storage_rgba8(device, width, height, Some("Game of Life Scratch"));

            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("Game of Life Compute Shader"),
                source: wgpu::ShaderSource::Wgsl(include_str!("shaders/game_of_life.wgsl").into()),
            });

            let bind_group_layout =
                device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Game of Life Bind Group Layout"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::StorageTexture {
                                access: wgpu::StorageTextureAccess::ReadOnly,
                                format: wgpu::TextureFormat::Rgba8Unorm,
                                view_dimension: wgpu::TextureViewDimension::D2,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::COMPUTE,
                            ty: wgpu::BindingType::StorageTexture {
                                access: wgpu::StorageTextureAccess::WriteOnly,
                                format: wgpu::TextureFormat::Rgba8Unorm,
                                view_dimension: wgpu::TextureViewDimension::D2,
                            },
                            count: None,
                        },
                    ],
                });

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Game of Life Bind Group"),
                layout: &bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&display_texture.view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&scratch_texture.view),
                    },
                ],
            });

            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Game of Life Pipeline Layout"),
                bind_group_layouts: &[&bind_group_layout],
                push_constant_ranges: &[],
            });

            let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some("Game of Life Pipeline"),
                layout: Some(&pipeline_layout),
                module: &shader,
                entry_point: Some("main"),
                compilation_options: Default::default(),
                cache: None,
            });

            let dispatch_x = (width + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;
            let dispatch_y = (height + WORKGROUP_SIZE - 1) / WORKGROUP_SIZE;

            (
                display_texture,
                scratch_texture,
                bind_group,
                pipeline,
                dispatch_x,
                dispatch_y,
            )
        };

        let display_handle = ctx.scene.assets.textures.insert(display_texture);
        ctx.renderer.update_texture_bind_group(&ctx.scene.assets);

        Self {
            bind_group,
            pipeline,
            scratch_texture,
            display_handle,
            dispatch_x,
            dispatch_y,
            extent: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            accumulator: 0.0,
            step_interval,
        }
    }
    fn display_texture_handle(&self) -> Handle<Texture> {
        self.display_handle
    }

    fn update(&mut self, ctx: &mut GpuUpdateContext<'_>) {
        self.accumulator += ctx.dt;
        while self.accumulator >= self.step_interval {
            self.accumulator -= self.step_interval;
            self.run_step(ctx);
        }
    }

    fn run_step(&mut self, ctx: &mut GpuUpdateContext<'_>) {
        let Some(display_texture) = ctx.scene.assets.textures.get(self.display_handle) else {
            return;
        };

        let device = ctx.renderer.get_device();
        let queue = ctx.renderer.get_queue();

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Game of Life Encoder"),
        });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Game of Life Compute"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.dispatch_workgroups(self.dispatch_x, self.dispatch_y, 1);
        }

        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.scratch_texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &display_texture.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            self.extent,
        );

        queue.submit(Some(encoder.finish()));
    }
}

fn generate_initial_pattern(buffer: &mut [u8], width: u32, height: u32) {
    for y in 0..height {
        for x in 0..width {
            let idx = ((y * width + x) * 4) as usize;
            let alive = ((x + y) % 11 == 0) || ((x * y) % 17 == 0);
            let value = if alive { 255 } else { 0 };
            buffer[idx] = value;
            buffer[idx + 1] = value;
            buffer[idx + 2] = value;
            buffer[idx + 3] = 255;
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    env_logger::init();
    if let Err(err) = wgpu_cube::run(build_app()) {
        eprintln!("Application error: {err}");
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    match wgpu_cube::run(build_app()) {
        Ok(_) => {}
        Err(e) => {
            web_sys::console::error_1(&format!("[Rust] Error: {:?}", e).into());
        }
    }
}
