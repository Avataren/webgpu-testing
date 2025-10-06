mod demo_scenes;

use demo_scenes::DemoScene;
use wgpu_cube::AppBuilder;

const ACTIVE_SCENE: DemoScene = DemoScene::Gltf {
    path: "web/assets/chessboard/ABeautifulGame.gltf",
    scale: 15.0,
};

fn main() {
    let mut builder = AppBuilder::new();
    builder.add_plugin(ACTIVE_SCENE.plugin());

    if let Err(err) = wgpu_cube::run(builder) {
        eprintln!("Application error: {err}");
    }
}
