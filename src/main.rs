mod demo_scenes;

use demo_scenes::DemoScene;
use wgpu_cube::AppBuilder;

const ACTIVE_SCENE: DemoScene = DemoScene::ShadowTest;

fn main() {
    let mut builder = AppBuilder::new();
    builder.add_plugin(ACTIVE_SCENE.plugin());

    if let Err(err) = wgpu_cube::run(builder) {
        eprintln!("Application error: {err}");
    }
}
