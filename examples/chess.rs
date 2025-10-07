use glam::Vec3;
use log::info;
use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::scene::SceneLoader;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const CHESS_GLTF_PATH: &str = "web/assets/chessboard/ABeautifulGame.gltf";
const CHESS_SCALE: f32 = 15.0;

fn build_app() -> AppBuilder {
    let mut builder = AppBuilder::new();
    builder.disable_default_textures();
    builder.disable_default_lighting();
    builder.add_startup_system(load_chess_scene);
    let factor = CHESS_SCALE.log10().max(0.5);
    builder.add_system(orbit_camera(5.0 * factor, 2.0 * factor));
    builder.skip_initial_frames(5);
    builder
}

fn load_chess_scene(ctx: &mut StartupContext<'_>) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    info!("Loading glTF: {} (scale: {})", CHESS_GLTF_PATH, CHESS_SCALE);

    match SceneLoader::load_gltf(CHESS_GLTF_PATH, scene, renderer, CHESS_SCALE) {
        Ok(_) => {
            scene.add_default_lighting();
            info!("glTF loaded: {} entities", scene.world.len());
        }
        Err(err) => {
            log::error!("Failed to load glTF: {}", err);
        }
    }
}

fn orbit_camera(
    radius: f32,
    height: f32,
) -> Box<dyn for<'a> FnMut(&mut UpdateContext<'a>) + 'static> {
    Box::new(move |ctx: &mut UpdateContext<'_>| {
        let t = ctx.scene.time() as f32 * 0.25;
        let camera = ctx.scene.camera_mut();
        camera.eye = Vec3::new(t.cos() * radius, height, t.sin() * radius);
        camera.target = Vec3::ZERO;
        camera.up = Vec3::Y;
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(err) = wgpu_cube::run(build_app()) {
        eprintln!("Application error: {err}");
    }
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    web_sys::console::log_1(&"[Rust] start_app() called".into());

    match wgpu_cube::run(build_app()) {
        Ok(_) => {
            web_sys::console::log_1(&"[Rust] Application started successfully".into());
        }
        Err(e) => {
            web_sys::console::error_1(&format!("[Rust] Error: {:?}", e).into());
        }
    }
}
