use wgpu_cube::demo_scenes;

fn main() {
    let builder = demo_scenes::build_app_for_scene(demo_scenes::ACTIVE_SCENE);

    if let Err(err) = wgpu_cube::run(builder) {
        eprintln!("Application error: {err}");
    }
}
