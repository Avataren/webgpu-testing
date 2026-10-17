//! Rendering bridge for the scene graph.
//!
//! The bridge gathers renderable objects, lights, and gizmos from the scene graph
//! and feeds them into the renderer. Extracting this logic keeps `Scene` focused
//! on data management while leaving presentation concerns to a dedicated helper.

use super::scene::Scene;
use crate::renderer::{CustomRenderRequest, LightsData, RenderBatcher, RenderFrame, Renderer};
use crate::scene::components::{SelectedInEditor, WorldTransform};
use crate::scene::internal::{gizmos, lights, rendering, transform_gizmos};
use crate::scene::transform::Transform;
use log::error;

pub struct RenderBridge;

impl RenderBridge {
    #[allow(clippy::too_many_arguments)]
    pub fn render<'a>(
        scene: &'a mut Scene,
        renderer: &mut Renderer,
        batcher: &mut RenderBatcher,
        custom_render: Option<&'a mut CustomRenderRequest<'a>>,
        gizmos_enabled: bool,
    ) -> Result<RenderFrame, crate::renderer::SurfaceError> {
        scene.refresh_environment_state();
        batcher.clear();
        batcher.set_ray_traced_transmission(renderer.ray_traced_transmission());
        let camera_vectors = rendering::CameraVectors::from_renderer(renderer);
        let gizmo_resources = if gizmos_enabled {
            Some(scene.ensure_gizmo_resources(renderer))
        } else {
            None
        };
        // Skeletons are a debugging aid: drawn whenever asked, gizmos or not.
        let skeleton_resources = if scene.show_skeletons() {
            Some(gizmo_resources.unwrap_or_else(|| scene.ensure_gizmo_resources(renderer)))
        } else {
            None
        };
        let mut selected_transform: Option<Transform> = None;
        // Outlines of selected light probe volumes, under the transform gizmo.
        let mut volume_shapes = Vec::new();

        for (node_key, node) in scene.nodes_with_keys() {
            let world = node.instance().world();
            let world_transform = *node.world_transform();

            let objects = crate::app::core::frame_timings::time("render_collect", || {
                rendering::build_render_objects(world, &scene.assets, camera_vectors, node_key)
            });
            for mut object in objects {
                object.transform = world_transform.mul_transform(&object.transform);
                if let Err(err) = batcher.add(object, &scene.assets) {
                    error!("failed to enqueue render object: {err}");
                }
            }

            if let Some(resources) = skeleton_resources {
                for bone in gizmos::build_skeleton_gizmos(world, world_transform, resources) {
                    if let Err(err) = batcher.add(bone, &scene.assets) {
                        error!("failed to enqueue skeleton gizmo: {err}");
                    }
                }
            }

            if let Some(resources) = gizmo_resources {
                for gizmo in
                    gizmos::build_light_gizmos(world, camera_vectors, world_transform, resources)
                {
                    if let Err(err) = batcher.add(gizmo, &scene.assets) {
                        error!("failed to enqueue light gizmo: {err}");
                    }
                }
            }

            if gizmos_enabled {
                volume_shapes.extend(crate::scene::light_probes::selected_volume_gizmos(
                    world,
                    world_transform,
                ));
                volume_shapes.extend(crate::physics::selected_collider_gizmos(
                    world,
                    &scene.assets,
                    &world_transform,
                ));
            }

            if gizmos_enabled && selected_transform.is_none() {
                if let Some((_, (entity_world, _))) = world
                    .query::<(&WorldTransform, &SelectedInEditor)>()
                    .iter()
                    .next()
                {
                    let combined = world_transform.mul_transform(&entity_world.0);
                    selected_transform = Some(combined);
                }
            }
        }

        if gizmos_enabled && selected_transform.is_none() {
            selected_transform = scene.selected_gizmo_transform();
        }

        // Every simulated collider, while physics debug drawing is on (play mode too).
        if scene.physics().debug_draw() {
            let node = *scene.node(scene.main_scene()).world_transform();
            volume_shapes.extend(crate::physics::simulation_gizmos(scene.physics(), &node));
        }

        // Navmeshes, agents and paths, and what agents think (and what the editor has
        // selected of them).
        {
            let selected: Vec<hecs::Entity> = if gizmos_enabled {
                scene
                    .main_world()
                    .query::<&SelectedInEditor>()
                    .iter()
                    .map(|(entity, _)| entity)
                    .collect()
            } else {
                Vec::new()
            };
            #[cfg(feature = "navigation")]
            let navigation = scene.navigation_debug_geometry(&selected);
            #[cfg(not(feature = "navigation"))]
            let navigation: Option<crate::renderer::debug_draw::DebugGeometry> = None;
            let geometry = match (navigation, scene.ai_debug_geometry(&selected)) {
                (Some(mut navigation), Some(ai)) => {
                    navigation.append(ai);
                    Some(navigation)
                }
                (navigation, ai) => navigation.or(ai),
            };
            if let Some(geometry) = geometry {
                renderer.set_debug_geometry(geometry);
            }
        }

        // Picking between frames must see the gizmo at the size it was drawn.
        scene.set_gizmo_viewport_height(camera_vectors.viewport_height);
        let mut gizmo_shapes = volume_shapes;
        if let Some(selection_transform) = selected_transform.filter(|_| gizmos_enabled) {
            gizmo_shapes.extend(transform_gizmos::build_transform_gizmos(
                camera_vectors,
                selection_transform,
                scene.transform_gizmo_mode(),
                scene.transform_gizmo_space(),
                scene.transform_gizmo_hover(),
            ));
        }
        renderer.set_gizmo_shapes(gizmo_shapes);

        let mut lights_data = LightsData::default();

        for (node_key, node) in scene.nodes_with_keys() {
            let node_lights = lights::collect_lights(
                node.instance().world(),
                camera_vectors,
                *node.world_transform(),
                node_key,
            );
            lights_data.extend_from(&node_lights);
        }

        crate::app::core::frame_timings::time("render_gpu", || {
            renderer.render(
                scene,
                &scene.assets,
                batcher,
                lights_data,
                scene.environment(),
                custom_render,
            )
        })
    }
}
