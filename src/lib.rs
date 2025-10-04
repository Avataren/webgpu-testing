pub mod app;
pub mod asset;
pub mod renderer;
pub mod scene;

use app::{App, SceneType};
use winit::event_loop::EventLoop;

fn create_app() -> App {
    // Central place to select which demo scene should run by default
    App::new(SceneType::Simple)
    //App::with_gltf("assets/chessboard/ABeautifulGame.gltf", 10.0)
}

#[cfg(not(target_arch = "wasm32"))]
fn init_logging() {
    let _ = env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .try_init();
}

#[cfg(target_arch = "wasm32")]
fn init_logging() {
    log::set_max_level(log::LevelFilter::Info);
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run() -> Result<(), winit::error::EventLoopError> {
    init_logging();

    log::info!("Starting wgpu hecs renderer - Hierarchy Test Mode");

    let event_loop = EventLoop::new()?;
    let mut app = create_app();

    let result = event_loop.run_app(&mut app);

    if let Err(ref err) = result {
        log::error!("Application error: {}", err);
    }

    log::info!("Application shutdown complete");

    result
}

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn run() -> Result<(), JsValue> {
    use wasm_bindgen::JsValue;
    use winit::platform::web::EventLoopExtWebSys;

    init_logging();
    log::info!("Starting wgpu hecs renderer - WebAssembly");

    let event_loop = EventLoop::new().map_err(|err| JsValue::from_str(&err.to_string()))?;
    let app = create_app();

    event_loop.spawn_app(app);

    Ok(())
}
