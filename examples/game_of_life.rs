// examples/game_of_life_refactored.rs - Refactored version using new compute abstractions

use glam::{Quat, Vec3};
use wgpu_cube::app::{GpuUpdateContext, StartupContext, UpdateContext};
use wgpu_cube::asset::Handle;
use wgpu_cube::render_application::{run_application, RenderApplication};
use wgpu_cube::renderer::{
    BindGroupBuilder, BindGroupLayoutBuilder, ComputePass, ComputePipelineBuilder, Material,
    Texture,
};
use wgpu_cube::scene::components::{Billboard, BillboardOrientation, BillboardSpace};
use wgpu_cube::scene::{EntityBuilder, Transform};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const GRID_WIDTH: u32 = 1920;
const GRID_HEIGHT: u32 = 1080;
const STEP_INTERVAL: f64 = 0.05;
const WORKGROUP_SIZE: u32 = 8;

#[derive(Default)]
struct GameOfLifeApp {
    state: Option<GameOfLifeState>,
}

impl RenderApplication for GameOfLifeApp {
    fn setup(&mut self, ctx: &mut StartupContext) {
        let gol_state = GameOfLifeState::new(ctx, GRID_WIDTH, GRID_HEIGHT);
        spawn_billboard(ctx, gol_state.display_handle, GRID_WIDTH, GRID_HEIGHT);
        configure_camera(ctx);
        self.state = Some(gol_state);
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        orbit_camera(ctx);
    }

    fn gpu_update(&mut self, ctx: &mut GpuUpdateContext) {
        if let Some(state) = &mut self.state {
            state.update(ctx);
        }
    }
}

struct GameOfLifeState {
    // Textures for ping-pong buffering
    texture_0: Texture,
    texture_1: Texture,
    display_handle: Handle<Texture>,

    // Compute pipeline and resources
    compute_pipeline: wgpu::ComputePipeline,
    bind_group_0: wgpu::BindGroup, // Read from 0, write to 1
    bind_group_1: wgpu::BindGroup, // Read from 1, write to 0

    // Simulation state
    width: u32,
    height: u32,
    dispatch_x: u32,
    dispatch_y: u32,
    accumulator: f64,
    current_buffer: bool,
}

impl GameOfLifeState {
    fn new(ctx: &mut StartupContext, width: u32, height: u32) -> Self {
        let device = ctx.renderer.get_device();
        let queue = ctx.renderer.get_queue();

        // Generate initial pattern
        let mut initial_data = vec![0u8; (width * height * 4) as usize];
        generate_initial_pattern(&mut initial_data, width, height);

        // Create storage textures for ping-pong buffering
        let texture_0 = Texture::storage_rgba8(device, width, height, Some("GoL Texture 0"));
        let texture_1 = Texture::storage_rgba8(device, width, height, Some("GoL Texture 1"));
        let display_texture = Texture::storage_rgba8(device, width, height, Some("GoL Display"));

        // Initialize textures with pattern
        for texture in [&texture_0, &display_texture] {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &texture.texture,
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
        }

        // Create compute pipeline using new builder API
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Game of Life Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/game_of_life.wgsl").into()),
        });

        let bind_group_layout = BindGroupLayoutBuilder::new(device)
            .with_label("Game of Life Bind Group Layout")
            .add_storage_texture_read(0, wgpu::TextureFormat::Rgba8Unorm)
            .add_storage_texture_write(1, wgpu::TextureFormat::Rgba8Unorm)
            .build();

        let compute_pipeline = ComputePipelineBuilder::new()
            .with_label("Game of Life Pipeline")
            .with_shader(&shader)
            .with_entry_point("main")
            .with_bind_group_layout(&bind_group_layout)
            .build(device);

        // Create bind groups for ping-pong buffering
        let bind_group_0 = BindGroupBuilder::new(device, &bind_group_layout)
            .with_label("Game of Life Bind Group 0")
            .add_texture_view(0, &texture_0.view)
            .add_texture_view(1, &texture_1.view)
            .build();

        let bind_group_1 = BindGroupBuilder::new(device, &bind_group_layout)
            .with_label("Game of Life Bind Group 1")
            .add_texture_view(0, &texture_1.view)
            .add_texture_view(1, &texture_0.view)
            .build();

        // Insert display texture and get handle
        let display_handle = ctx.scene.assets.textures.insert(display_texture);
        ctx.renderer.update_texture_bind_group(&ctx.scene.assets);

        let dispatch_x = width.div_ceil(WORKGROUP_SIZE);
        let dispatch_y = height.div_ceil(WORKGROUP_SIZE);

        Self {
            texture_0,
            texture_1,
            display_handle,
            compute_pipeline,
            bind_group_0,
            bind_group_1,
            width,
            height,
            dispatch_x,
            dispatch_y,
            accumulator: 0.0,
            current_buffer: false,
        }
    }

    fn update(&mut self, ctx: &mut GpuUpdateContext) {
        self.accumulator += ctx.dt;

        while self.accumulator >= STEP_INTERVAL {
            self.accumulator -= STEP_INTERVAL;
            self.run_step(ctx);
        }
    }

    fn run_step(&mut self, ctx: &mut GpuUpdateContext) {
        let device = ctx.renderer.get_device();
        let queue = ctx.renderer.get_queue();

        // Get the display texture from assets
        let Some(display_texture) = ctx.scene.assets.textures.get(self.display_handle) else {
            log::warn!("Display texture not found in assets");
            return;
        };

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Game of Life Encoder"),
        });

        // Run compute shader with ping-pong buffering
        {
            let mut pass = ComputePass::begin(&mut encoder, "Game of Life Compute");
            pass.set_pipeline(&self.compute_pipeline);

            // Alternate which bind group we use (swaps read/write textures)
            let bind_group = if self.current_buffer {
                &self.bind_group_1
            } else {
                &self.bind_group_0
            };

            pass.set_bind_group(0, bind_group, &[]).dispatch_workgroups(
                self.dispatch_x,
                self.dispatch_y,
                1,
            );
        }

        // Copy result to display texture
        let source_texture = if self.current_buffer {
            &self.texture_0.texture
        } else {
            &self.texture_1.texture
        };

        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: source_texture,
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
            wgpu::Extent3d {
                width: self.width,
                height: self.height,
                depth_or_array_layers: 1,
            },
        );

        queue.submit(Some(encoder.finish()));
        self.current_buffer = !self.current_buffer;
    }
}

fn generate_initial_pattern(buffer: &mut [u8], width: u32, height: u32) {
    use std::collections::hash_map::RandomState;
    use std::hash::BuildHasher;

    let mut set_cell = |x: u32, y: u32, alive: bool| {
        if x < width && y < height {
            let idx = ((y * width + x) * 4) as usize;
            let value = if alive { 255 } else { 0 };
            buffer[idx] = value;
            buffer[idx + 1] = value;
            buffer[idx + 2] = value;
            buffer[idx + 3] = 255;
        }
    };

    // Random noise
    let hasher = RandomState::new();
    for y in 0..height {
        for x in 0..width {
            let hash_val = hasher.hash_one((x, y));
            if hash_val % 100 < 30 {
                set_cell(x, y, true);
            }
        }
    }

    // Add interesting patterns
    let patterns = [
        (vec![(1, 0), (2, 1), (0, 2), (1, 2), (2, 2)], 20, 20), // Glider
        (vec![(0, 0), (1, 0), (2, 0)], 50, 30),                 // Blinker
    ];

    for (pattern, base_x, base_y) in patterns {
        for (dx, dy) in pattern {
            set_cell(base_x + dx, base_y + dy, true);
        }
    }
}

fn configure_camera(ctx: &mut StartupContext) {
    let camera = ctx.scene.camera_mut();
    camera.eye = Vec3::new(0.0, 0.0, 7.0);
    camera.target = Vec3::ZERO;
    camera.up = Vec3::Y;
}

fn orbit_camera(ctx: &mut UpdateContext) {
    let t = ctx.scene.time() as f32 * 0.2;
    let camera = ctx.scene.camera_mut();
    camera.eye = Vec3::new(t.cos() * 6.5, 2.5, t.sin() * 6.5);
    camera.target = Vec3::ZERO;
}

fn spawn_billboard(
    ctx: &mut StartupContext,
    texture_handle: Handle<Texture>,
    width: u32,
    height: u32,
) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    let (vertices, indices) = wgpu_cube::renderer::quad_mesh();
    let mesh = renderer.create_mesh(&vertices, &indices);
    let mesh_handle = scene.assets.meshes.insert(mesh);

    let scale_x = (width as f32) / 128.0;
    let scale_y = (height as f32) / 128.0;

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
                .with_nearest_filtering()
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

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(GameOfLifeApp::default()).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    run_application(GameOfLifeApp::default()).unwrap();
}
