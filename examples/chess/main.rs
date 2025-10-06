mod demo_scenes;

use demo_scenes::DemoScene;
use wgpu_cube::AppBuilder;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const ACTIVE_SCENE: DemoScene = DemoScene::Gltf {
    path: "web/assets/chessboard/ABeautifulGame.gltf",
    scale: 15.0,
};

fn build_app() -> AppBuilder {
    let mut builder = AppBuilder::new();
    builder.add_plugin(ACTIVE_SCENE.plugin());
    builder
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