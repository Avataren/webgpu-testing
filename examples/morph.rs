mod support;

use glam::{Quat, Vec3};
use log::info;
use wgpu_cube::app::{AppBuilder, StartupContext};
use wgpu_cube::render_application::{run_application, RenderApplication};
use wgpu_cube::scene::components::{CanCastShadow, DirectionalLight, MaterialComponent};
use wgpu_cube::scene::{Camera, Name, SceneLoader, Transform, TransformComponent};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

const GLTF_PATH: &str = "web/assets/morph/SuzanneMorphSparse.gltf";
const SCENE_SCALE: f32 = 0.25;

struct ExampleApp;

impl RenderApplication for ExampleApp {
    fn configure(&self, builder: &mut AppBuilder) {
        builder.disable_default_textures();
        builder.disable_default_lighting();
        builder.skip_initial_frames(5);
    }

    fn setup(&mut self, ctx: &mut StartupContext) {
        load_scene(ctx);
    }
}

fn load_scene(ctx: &mut StartupContext<'_>) {
    let renderer = &mut *ctx.renderer;
    let scene = &mut *ctx.scene;

    info!("Loading glTF: {} (scale: {})", GLTF_PATH, SCENE_SCALE);

    match SceneLoader::load_gltf(GLTF_PATH, scene, renderer, SCENE_SCALE, None) {
        Ok(_) => {
            support::configure(scene, [0.018, 0.026, 0.045], 0.13, |settings| {
                // The model is only half a metre tall; a large AO radius muddies its face.
                settings.ssao.radius = 0.07;
                settings.ssao.bias = 0.01;
                settings.ssao.intensity = 0.5;
            });
            let materials: Vec<_> = scene
                .world()
                .query::<&MaterialComponent>()
                .iter()
                .map(|(_, material)| material.0)
                .collect();
            for handle in materials {
                if let Some(asset) = scene.assets.materials.get_mut(handle) {
                    *asset.material_mut() =
                        asset.material().with_roughness(0.36).with_metallic(0.2);
                    asset.material_mut().base_color = [88, 175, 184, 255];
                }
            }
            for (name, direction, color, intensity) in [
                (
                    "Morph Key Light",
                    Vec3::new(0.35, -1.0, -0.85),
                    Vec3::new(1.0, 0.95, 0.87),
                    3.0,
                ),
                (
                    "Morph Rim Light",
                    Vec3::new(-0.55, -0.35, 0.8),
                    Vec3::new(0.51, 0.75, 1.0),
                    1.8,
                ),
            ] {
                scene.world_mut().spawn((
                    Name::new(name),
                    TransformComponent(Transform::from_trs(
                        Vec3::ZERO,
                        Quat::from_rotation_arc(Vec3::NEG_Z, direction.normalize()),
                        Vec3::ONE,
                    )),
                    DirectionalLight::new(color, intensity),
                    CanCastShadow(false),
                ));
            }
            let mut camera = Camera::default();
            camera.eye = Vec3::new(0.45, 0.22, 0.9);
            camera.target = Vec3::ZERO;
            camera.up = Vec3::Y;
            scene.set_camera(camera);
            info!("glTF loaded: {} entities", scene.world().len());
        }
        Err(err) => {
            log::error!("Failed to load glTF: {}", err);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    run_application(ExampleApp).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
pub fn start_app() {
    web_sys::console::log_1(&"[Rust] start_app() called".into());

    match run_application(ExampleApp) {
        Ok(_) => {
            web_sys::console::log_1(&"[Rust] Application started successfully".into());
        }
        Err(e) => {
            web_sys::console::error_1(&format!("[Rust] Error: {:?}", e).into());
        }
    }
}
