use wgpu_cube::{demo_scenes, AppBuilder};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

fn build_demo_app() -> AppBuilder {
    let mut builder = AppBuilder::new();
    demo_scenes::add_scene_to_app(&mut builder, demo_scenes::ACTIVE_SCENE);
    builder
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    if let Err(err) = wgpu_cube::run(build_demo_app()) {
        eprintln!("Application error: {err}");
    }
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn wasm_start() -> Result<(), wasm_bindgen::JsValue> {
    wgpu_cube::run(build_demo_app())
}
