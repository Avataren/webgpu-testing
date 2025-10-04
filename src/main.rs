// main.rs
// Entry point for the pure hecs ECS renderer

use winit::event_loop::EventLoop;
use crate::app::{App, SceneType};

mod app;
mod asset;
mod renderer;
mod scene;

fn main() {
    // Initialize logging
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    log::info!("Starting wgpu hecs renderer");

    // Create event loop
    let event_loop = EventLoop::new().expect("Failed to create event loop");

    // Create app - choose your scene type:
    
    let mut app = App::new(SceneType::Animated);
    
    // Other options:
    // let mut app = App::new(SceneType::Simple);
    // let mut app = App::new(SceneType::Grid);
    // let mut app = App::new(SceneType::MaterialShowcase);
    
    // Load from glTF:
    // let mut app = App::with_gltf("assets/scene.gltf");

    // Run the application
    if let Err(e) = event_loop.run_app(&mut app) {
        log::error!("Application error: {}", e);
    }

    log::info!("Application shutdown complete");
}