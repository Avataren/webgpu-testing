use glam::Vec3;
use log::info;
use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::render_application::{run_application, RenderApplication};
use wgpu_cube::scene::SceneLoader;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const GLTF_PATH: &str = "web/assets/chessboard/ABeautifulGame.gltf";
const CHESS_SCALE: f32 = 15.0;

struct ExampleApp;

impl RenderApplication for ExampleApp {
    fn configure(&self, builder: &mut AppBuilder) {
        builder.disable_default_textures();
        builder.disable_default_lighting();
        builder.skip_initial_frames(5);
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        load_chess_scene(ctx);
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        let factor = CHESS_SCALE.log10().max(0.5);
        orbit_camera(ctx, 5.0 * factor, 2.0 * factor);
    }
}

fn load_chess_scene(ctx: &mut StartupContext<'_>) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    info!("Loading glTF: {} (scale: {})", GLTF_PATH, CHESS_SCALE);

    match SceneLoader::load_gltf(GLTF_PATH, scene, renderer, CHESS_SCALE) {
        Ok(_) => {
            scene.add_default_lighting();
            info!("glTF loaded: {} entities", scene.world.len());
        }
        Err(err) => {
            log::error!("Failed to load glTF: {}", err);
        }
    }
}

fn orbit_camera(ctx: &mut UpdateContext<'_>, radius: f32, height: f32) {
    let t = ctx.scene.time() as f32 * 0.25;
    let camera = ctx.scene.camera_mut();
    camera.eye = Vec3::new(t.cos() * radius, height, t.sin() * radius);
    camera.target = Vec3::ZERO;
    camera.up = Vec3::Y;
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(ExampleApp).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    web_sys::console::log_1(&"[Rust] start_app() called".into());

    match run_application(ExampleApp) {
        Ok(_) => {
            web_sys::console::log_1(&"[Rust] Application started successfully".into());
        }
        Err(e) => {
            web_sys::console::error_1(&format!("[Rust] Error: {:?}", e).into());
        }
    }
}
