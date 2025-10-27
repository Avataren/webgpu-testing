//! Rendering bridge for the scene graph.
//!
//! The bridge gathers renderable objects, lights, and gizmos from the scene graph
//! and feeds them into the renderer. Extracting this logic keeps `Scene` focused
//! on data management while leaving presentation concerns to a dedicated helper.

use super::scene_core::Scene;
use crate::renderer::{CustomRenderRequest, LightsData, RenderBatcher, RenderFrame, Renderer};
use crate::scene::components::{SelectedInEditor, WorldTransform};
use crate::scene::internal::{gizmos, lights, rendering, transform_gizmos};
use crate::scene::transform::Transform;

pub struct RenderBridge;

impl RenderBridge {
    #[allow(clippy::too_many_arguments)]
    pub fn render<'a>(
        scene: &'a mut Scene,
        renderer: &mut Renderer,
        batcher: &mut RenderBatcher,
        custom_render: Option<&'a mut CustomRenderRequest<'a>>,
        gizmos_enabled: bool,
    ) -> Result<RenderFrame, wgpu::SurfaceError> {
        scene.refresh_environment_state();
        batcher.clear();
        let camera_vectors = rendering::CameraVectors::from_renderer(renderer);
        let gizmo_resources = if gizmos_enabled {
            Some(scene.ensure_gizmo_resources(renderer))
        } else {
            None
        };
        let mut selected_transform: Option<Transform> = None;

        for node in scene.nodes_iter() {
            let world = node.instance().world();
            let world_transform = *node.world_transform();

            for mut object in rendering::build_render_objects(world, camera_vectors).into_iter() {
                object.transform = world_transform.mul_transform(&object.transform);
                batcher.add(object);
            }

            if let Some(resources) = gizmo_resources {
                for gizmo in
                    gizmos::build_light_gizmos(world, camera_vectors, world_transform, resources)
                {
                    batcher.add(gizmo);
                }
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

        if gizmos_enabled {
            if let Some(selection_transform) = selected_transform {
                let transform_resources = scene.ensure_transform_gizmo_resources(renderer);
                for gizmo in transform_gizmos::build_transform_gizmos(
                    camera_vectors,
                    selection_transform,
                    scene.transform_gizmo_mode(),
                    scene.transform_gizmo_space(),
                    transform_resources,
                    scene.transform_gizmo_hover(),
                ) {
                    batcher.add(gizmo);
                }
            }
        }

        let mut lights_data = LightsData::default();

        for node in scene.nodes_iter() {
            let node_lights = lights::collect_lights(
                node.instance().world(),
                camera_vectors,
                *node.world_transform(),
            );
            lights_data.extend_from(&node_lights);
        }

        renderer.render(
            scene,
            &scene.assets,
            batcher,
            lights_data,
            scene.environment(),
            custom_render,
        )
    }
}
