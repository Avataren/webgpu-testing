mod demo_scenes;

use demo_scenes::DemoScene;
use wgpu_cube::AppBuilder;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

//const ACTIVE_SCENE: DemoScene = DemoScene::ShadowTest;

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
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    wgpu_cube::run(build_app())
}
