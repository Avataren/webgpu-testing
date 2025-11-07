use glam::Vec3;
use log::{error, info};
use std::path::PathBuf;
use wgpu_cube::asset::MaterialAsset;
use wgpu_cube::renderer::{cube_mesh, Material, Renderer};
use wgpu_cube::scene::components::{MaterialComponent, MeshBounds, MeshComponent, Name, Visible};
use wgpu_cube::scene::{Camera, EntityBuilder, Scene, Transform};

use super::core::{EditorApplication, EditorSharedState};
use super::ui_plugin_manager::{create_plugin_script_source, UiPluginManager};

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
            let startup_script = Self::load_script("editor_startup.lua");

            if let Some(script) = startup_script {
                EntityBuilder::new(scene)
                    .with_name("Editor Startup Script")
                    .with_script(script)
                    .spawn();

                scene.update(0.0);
            } else {
                error!("Failed to load editor startup script - creating Editor Cube directly");
                // Create Editor Cube directly if script fails
                EntityBuilder::new(scene)
                    .with_name("Editor Cube")
                    .with_transform(Transform::from_translation(Vec3::new(0.0, 0.5, 0.0)))
                    .spawn();
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

    /// Load UI plugins from manifest and create entities for them
    pub(super) fn load_ui_plugins(shared: &mut EditorSharedState, scene: &mut Scene) {
        info!("Loading UI plugins...");

        // Unload existing plugins first to avoid duplicates
        if let Some(existing_manager) = shared.ui_plugin_manager.as_mut() {
            info!("Unloading existing UI plugins before reload");
            existing_manager.unload_all(scene.main_world_mut());
        }

        // Try to find ui_plugins.toml in examples/scripts
        let manifest_path = PathBuf::from("examples/scripts/ui_plugins.toml");

        if !manifest_path.exists() {
            info!("No UI plugin manifest found at {}", manifest_path.display());
            return;
        }

        // Create plugin manager with base path for scripts
        let mut manager = UiPluginManager::new(PathBuf::from("examples/scripts"));

        // Load manifest
        let manifest = match manager.load_manifest(&manifest_path) {
            Ok(manifest) => manifest,
            Err(err) => {
                error!("Failed to load plugin manifest: {}", err);
                return;
            }
        };

        info!("Found {} plugins in manifest", manifest.plugins.len());

        // Create entities for enabled plugins
        let mut loaded_count = 0;
        for plugin_meta in manifest.plugins {
            if !plugin_meta.enabled {
                info!("Skipping disabled plugin: {}", plugin_meta.name);
                continue;
            }

            info!("Loading plugin: {} ({})", plugin_meta.name, plugin_meta.script);

            // Create script source
            let script_source = match create_plugin_script_source(&manager, &plugin_meta) {
                Ok(source) => source,
                Err(err) => {
                    error!("Failed to create script source for {}: {}", plugin_meta.name, err);
                    continue;
                }
            };

            // Create entity with script
            let entity = EntityBuilder::new(scene)
                .with_name(format!("Plugin: {}", plugin_meta.name))
                .with_script(script_source)
                .spawn();

            // Register with manager
            manager.register_plugin(plugin_meta, entity);
            loaded_count += 1;
        }

        info!("Loaded {} UI plugins", loaded_count);

        // Store manager in shared state
        shared.ui_plugin_manager = Some(manager);
    }
}
