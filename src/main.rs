// main.rs
use winit::event_loop::EventLoop;

use crate::app::App;

mod app;
mod renderer;
mod scene;

fn main() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    let event_loop = EventLoop::new().expect("event loop");
    let mut app = App::new();
    
    if let Err(e) = event_loop.run_app(&mut app) {
        log::error!("Application error: {}", e);
    }
}