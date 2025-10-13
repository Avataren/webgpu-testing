use glam::{Quat, Vec3};
use log::info;
use wgpu_cube::app::{AppBuilder, StartupContext, UpdateContext};
use wgpu_cube::render_application::{run_application, RenderApplication};
use wgpu_cube::scene::{Camera, SceneLoader, SceneNodeId};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const ANIMATION_GLTF: &str = "web/assets/blender/physics_boxes2.gltf";
const CHESS_GLTF: &str = "web/assets/chessboard/ABeautifulGame.gltf";
const ANIMATION_SCALE: f32 = 1.0;
const CHESS_SCALE: f32 = 15.0;

#[derive(Default)]
struct ExampleApp {
    root_node: Option<SceneNodeId>,
}

impl RenderApplication for ExampleApp {
    fn configure(&self, builder: &mut AppBuilder) {
        builder.disable_default_textures();
        builder.disable_default_lighting();
        builder.skip_initial_frames(5);
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        let renderer = &mut *ctx.renderer;
        let scene = &mut *ctx.scene;

        let mut animation_asset =
            match SceneLoader::load_gltf_asset(ANIMATION_GLTF, renderer, ANIMATION_SCALE) {
                Ok(asset) => asset,
                Err(err) => {
                    log::error!("Failed to load animation glTF: {err}");
                    return;
                }
            };

        let mut chess_asset = match SceneLoader::load_gltf_asset(CHESS_GLTF, renderer, CHESS_SCALE)
        {
            Ok(asset) => asset,
            Err(err) => {
                log::error!("Failed to load chess glTF: {err}");
                return;
            }
        };

        info!(
            "Loaded animation asset with {} entities",
            animation_asset.asset.entities.len()
        );
        info!(
            "Loaded chess asset with {} entities",
            chess_asset.asset.entities.len()
        );

        scene.add_default_lighting();

        let root = scene.create_node("CombinedRoot", None);

        let mut textures_changed = animation_asset.register_resources(&mut scene.assets);
        let animation_node =
            scene.instantiate_asset_named(&animation_asset.asset, "AnimatedBoxes", Some(root));
        scene.node_local_transform_mut(animation_node).translation = Vec3::new(-8.0, 0.0, 0.0);

        textures_changed |= chess_asset.register_resources(&mut scene.assets);
        let chess_node =
            scene.instantiate_asset_named(&chess_asset.asset, "ChessBoard", Some(root));
        scene.node_local_transform_mut(chess_node).translation = Vec3::new(8.0, 0.0, 0.0);

        if textures_changed {
            renderer.update_texture_bind_group(&scene.assets);
        }

        scene.update(0.0);

        scene.set_camera(Camera {
            eye: Vec3::new(0.0, 6.0, 12.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            ..Camera::default()
        });

        self.root_node = Some(root);
    }

    fn update(&mut self, ctx: &mut UpdateContext) {
        let t = ctx.scene.time() as f32 * 0.25;

        if let Some(root) = self.root_node {
            let rotation = Quat::from_rotation_y(t * 0.5);
            let transform = ctx.scene.node_local_transform_mut(root);
            transform.rotation = rotation;
        }

        let radius = 12.0;
        let height = 6.0;
        let camera = ctx.scene.camera_mut();
        camera.eye = Vec3::new(t.cos() * radius, height, t.sin() * radius);
        camera.target = Vec3::ZERO;
        camera.up = Vec3::Y;
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(ExampleApp::default()).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    web_sys::console::log_1(&"[Rust] start_app() called".into());

    match run_application(ExampleApp::default()) {
        Ok(_) => {
            web_sys::console::log_1(&"[Rust] Application started successfully".into());
        }
        Err(e) => {
            web_sys::console::error_1(&format!("[Rust] Error: {:?}", e).into());
        }
    }
}
