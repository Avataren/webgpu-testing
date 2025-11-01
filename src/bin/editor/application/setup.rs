use glam::Vec3;
use log::error;
use std::path::PathBuf;
use wgpu_cube::asset::MaterialAsset;
use wgpu_cube::renderer::{cube_mesh, Material, Renderer};
use wgpu_cube::scene::components::{MaterialComponent, MeshBounds, MeshComponent, Name, Visible};
use wgpu_cube::scene::{Camera, EntityBuilder, Scene};

use super::core::EditorApplication;

const EDITOR_CUBE_MATERIAL_CANONICAL_PATH: &str = "builtin/editor/editor_cube.material";

impl EditorApplication {
    pub(super) fn ensure_editor_scene_basics(scene: &mut Scene, renderer: &mut Renderer) {
        let has_editor_cube = {
            scene
                .main_world()
                .query::<&Name>()
                .iter()
                .any(|(_, name)| name.0 == "Editor Cube")
        };

        if !has_editor_cube {
            let startup_script = Self::load_script("editor_startup.rn");

            if let Some(script) = startup_script {
                EntityBuilder::new(scene)
                    .with_name("Editor Startup Script")
                    .with_script(script)
                    .spawn();

                scene.update(0.0);
            } else {
                error!("Failed to load editor startup script");
                return;
            }
        }

        let cube_entity = {
            let world = scene.main_world();
            world
                .query::<&Name>()
                .iter()
                .find(|(_, name)| name.0 == "Editor Cube")
                .map(|(entity, _)| entity)
        };

        if let Some(entity) = cube_entity {
            let missing_mesh = {
                let world = scene.main_world();
                world.get::<&MeshComponent>(entity).is_err()
            };
            let missing_bounds = {
                let world = scene.main_world();
                world.get::<&MeshBounds>(entity).is_err()
            };
            let mut cached_bounds = None;
            if missing_mesh {
                let (vertices, indices) = cube_mesh();
                cached_bounds = MeshBounds::from_vertices(&vertices);
                let mesh = renderer.create_mesh(&vertices, &indices);
                let mesh_handle = scene.assets.meshes.insert(mesh);
                if let Err(err) = scene
                    .main_world_mut()
                    .insert_one(entity, MeshComponent(mesh_handle))
                {
                    error!("failed to attach mesh to Editor Cube: {err}");
                }
            }
            if missing_bounds {
                let bounds = cached_bounds
                    .unwrap_or_else(|| MeshBounds::new(Vec3::splat(-0.5), Vec3::splat(0.5)));
                if let Err(err) = scene.main_world_mut().insert_one(entity, bounds) {
                    error!("failed to attach bounds to Editor Cube: {err}");
                }
            }

            let missing_material = {
                let world = scene.main_world();
                world.get::<&MaterialComponent>(entity).is_err()
            };
            if missing_material {
                let canonical_path = PathBuf::from(EDITOR_CUBE_MATERIAL_CANONICAL_PATH);
                let handle = scene
                    .assets
                    .material_handle_for_path(&canonical_path)
                    .unwrap_or_else(|| {
                        scene
                            .assets
                            .insert_material_asset(MaterialAsset::from_material(
                                Material::pbr(),
                                canonical_path.clone(),
                            ))
                    });
                if let Err(err) = scene
                    .main_world_mut()
                    .insert_one(entity, MaterialComponent(handle))
                {
                    error!("failed to attach material to Editor Cube: {err}");
                }
            }

            let missing_visibility = {
                let world = scene.main_world();
                world.get::<&Visible>(entity).is_err()
            };
            if missing_visibility {
                if let Err(err) = scene.main_world_mut().insert_one(entity, Visible(true)) {
                    error!("failed to mark Editor Cube visible: {err}");
                }
            }
        }

        if !scene.has_any_lights() {
            scene.add_default_lighting();
        }

        let default_camera = Camera::default();
        let current_camera = scene.camera();
        let camera_matches_default = current_camera.eye.abs_diff_eq(default_camera.eye, 1e-5)
            && current_camera
                .target
                .abs_diff_eq(default_camera.target, 1e-5)
            && current_camera.up.abs_diff_eq(default_camera.up, 1e-5)
            && current_camera.projection() == default_camera.projection();

        if camera_matches_default {
            let camera = scene.camera_mut();
            camera.eye = Vec3::new(6.0, 4.0, 6.0);
            camera.target = Vec3::new(0.0, 0.5, 0.0);
            camera.up = Vec3::Y;
        }
    }
}
