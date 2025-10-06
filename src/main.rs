use wgpu_cube::{demo_scenes, AppBuilder};

fn main() {
    let mut builder = AppBuilder::new();
    demo_scenes::add_scene_to_app(&mut builder, demo_scenes::ACTIVE_SCENE);

    if let Err(err) = wgpu_cube::run(builder) {
        eprintln!("Application error: {err}");
    }
}
