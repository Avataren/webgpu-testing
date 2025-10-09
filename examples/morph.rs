use glam::Vec3;
use log::info;
use wgpu_cube::app::{AppBuilder, StartupContext};
use wgpu_cube::scene::{Camera, SceneLoader};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const GLTF_PATH: &str = "web/assets/morph/SuzanneMorphSparse.gltf";
const SCENE_SCALE: f32 = 0.25;

fn build_app() -> AppBuilder {
    let mut builder = AppBuilder::new();
    builder.disable_default_textures();
    builder.disable_default_lighting();
    builder.add_startup_system(load_scene);
    builder.skip_initial_frames(5);
    builder
}

fn load_scene(ctx: &mut StartupContext<'_>) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    info!("Loading glTF: {} (scale: {})", GLTF_PATH, SCENE_SCALE);

    match SceneLoader::load_gltf(GLTF_PATH, scene, renderer, SCENE_SCALE) {
        Ok(_) => {
            scene.add_default_lighting();
            scene.set_camera(Camera {
                eye: Vec3::new(0.5, 0.25, 1.0),
                target: Vec3::ZERO,
                up: Vec3::Y,
                ..Camera::default()
            });
            info!("glTF loaded: {} entities", scene.world.len());
        }
        Err(err) => {
            log::error!("Failed to load glTF: {}", err);
        }
    }
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
