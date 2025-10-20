use glam::Vec3;
use log::error;
use wgpu_cube::app::StartupContext;
use wgpu_cube::renderer::{cube_mesh, Material};
use wgpu_cube::scene::components::{MaterialComponent, MeshBounds, MeshComponent, Name, Visible};
use wgpu_cube::scene::EntityBuilder;

use super::core::EditorApplication;

impl EditorApplication {
    pub(super) fn ensure_default_scene(&mut self, ctx: &mut StartupContext) {
        let has_editor_cube = {
            ctx.scene
                .main_world()
                .query::<&Name>()
                .iter()
                .any(|(_, name)| name.0 == "Editor Cube")
        };

        if !has_editor_cube {
            let startup_script = Self::load_script("editor_startup.rn");

            if let Some(script) = startup_script {
                let world = ctx.scene.main_world_mut();
                EntityBuilder::new(world)
                    .with_name("Editor Startup Script")
                    .with_script(script)
                    .spawn();

                ctx.scene.update(0.0);
            } else {
                error!("Failed to load editor startup script");
                return;
            }
        }

        let cube_entity = {
            let world = ctx.scene.main_world();
            world
                .query::<&Name>()
                .iter()
                .find(|(_, name)| name.0 == "Editor Cube")
                .map(|(entity, _)| entity)
        };

        if let Some(entity) = cube_entity {
            let missing_mesh = {
                let world = ctx.scene.main_world();
                world.get::<&MeshComponent>(entity).is_err()
            };
            let missing_bounds = {
                let world = ctx.scene.main_world();
                world.get::<&MeshBounds>(entity).is_err()
            };
            let mut cached_bounds = None;
            if missing_mesh {
                let (vertices, indices) = cube_mesh();
                cached_bounds = MeshBounds::from_vertices(&vertices);
                let mesh = ctx.renderer.create_mesh(&vertices, &indices);
                let mesh_handle = ctx.scene.assets.meshes.insert(mesh);
                if let Err(err) = ctx
                    .scene
                    .main_world_mut()
                    .insert_one(entity, MeshComponent(mesh_handle))
                {
                    error!("failed to attach mesh to Editor Cube: {err}");
                }
            }
            if missing_bounds {
                let bounds = cached_bounds
                    .unwrap_or_else(|| MeshBounds::new(Vec3::splat(-0.5), Vec3::splat(0.5)));
                if let Err(err) = ctx.scene.main_world_mut().insert_one(entity, bounds) {
                    error!("failed to attach bounds to Editor Cube: {err}");
                }
            }

            let missing_material = {
                let world = ctx.scene.main_world();
                world.get::<&MaterialComponent>(entity).is_err()
            };
            if missing_material {
                if let Err(err) = ctx
                    .scene
                    .main_world_mut()
                    .insert_one(entity, MaterialComponent(Material::pbr()))
                {
                    error!("failed to attach material to Editor Cube: {err}");
                }
            }

            let missing_visibility = {
                let world = ctx.scene.main_world();
                world.get::<&Visible>(entity).is_err()
            };
            if missing_visibility {
                if let Err(err) = ctx.scene.main_world_mut().insert_one(entity, Visible(true)) {
                    error!("failed to mark Editor Cube visible: {err}");
                }
            }
        }

        if !ctx.scene.has_any_lights() {
            ctx.scene.add_default_lighting();
        }

        let camera = ctx.scene.camera_mut();
        camera.eye = Vec3::new(6.0, 4.0, 6.0);
        camera.target = Vec3::new(0.0, 0.5, 0.0);
        camera.up = Vec3::Y;
    }
}
