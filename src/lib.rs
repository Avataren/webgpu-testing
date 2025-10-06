pub mod app;
pub mod asset;
pub mod io;
pub mod renderer;
pub mod scene;
pub mod time;

pub use app::{
    App, AppBuilder, Plugin, StartupContext, StartupSystem, UpdateContext, UpdateSystem,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;
use winit::event_loop::EventLoop;

#[cfg(not(target_arch = "wasm32"))]
fn init_logging() {
    let _ = env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .try_init();
}

#[cfg(target_arch = "wasm32")]
fn init_logging() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    console_log::init_with_level(log::Level::Info).expect("Failed to initialize logger");
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run(builder: AppBuilder) -> Result<(), winit::error::EventLoopError> {
    run_with_app(builder.build())
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_with_app(mut app: App) -> Result<(), winit::error::EventLoopError> {
    init_logging();

    log::info!("Starting wgpu hecs renderer");

    let event_loop = EventLoop::new()?;
    let result = event_loop.run_app(&mut app);

    if let Err(ref err) = result {
        log::error!("Application error: {}", err);
    }

    log::info!("Application shutdown complete");

    result
}

#[cfg(target_arch = "wasm32")]
pub fn run(builder: AppBuilder) -> Result<(), JsValue> {
    run_with_app(builder.build())
}

#[cfg(target_arch = "wasm32")]
pub fn run_with_app(app: App) -> Result<(), JsValue> {
    use wasm_bindgen::JsValue;
    use winit::platform::web::EventLoopExtWebSys;

    init_logging();
    log::info!("Starting wgpu hecs renderer - WebAssembly");

    let event_loop = EventLoop::new().map_err(|err| JsValue::from_str(&err.to_string()))?;
    event_loop.spawn_app(app);

    Ok(())
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    run(AppBuilder::default())
}
