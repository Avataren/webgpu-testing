#![cfg(feature = "egui")]

mod application;
mod camera;
mod history;
mod inspector;
mod layout;
mod postprocess;
mod project;
mod script_editor;
mod windows;

use application::EditorApplication;
use wgpu_cube::run_application;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    run_application(EditorApplication::new())?;
    Ok(())
}
