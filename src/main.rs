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

    log::info!("Starting wgpu hecs renderer - Hierarchy Test Mode");

    // Create event loop
    let event_loop = EventLoop::new().expect("Failed to create event loop");

    // Create app with hierarchy test scene
    //let mut app = App::new(SceneType::HierarchyTest);
    
    // Other test scenes:
     let mut app = App::new(SceneType::Simple);
    // let mut app = App::new(SceneType::Grid);
    // let mut app = App::new(SceneType::Animated);
    // let mut app = App::new(SceneType::MaterialShowcase);
    
    // Load from glTF:
    // let mut app = App::with_gltf("assets/camera/AntiqueCamera.gltf", 1.0);
    // let mut app = App::with_gltf("assets/damagedhelmet/DamagedHelmet.gltf", 1.0);
    // let mut app = App::with_gltf("assets/avocado/Avocado.gltf", 20.0);
    // let mut app = App::with_gltf("assets/chessboard/ABeautifulGame.gltf", 1.0);

    // Run the application
    if let Err(e) = event_loop.run_app(&mut app) {
        log::error!("Application error: {}", e);
    }

    log::info!("Application shutdown complete");
}