//! Utilities for processing editor-driven glTF imports.
//!
//! The importer integrates assets spawned by the scripting system back into the
//! live scene graph. Isolating the logic here keeps `scene_core` focused on graph
//! management and exposes a narrow entry point for `Scene` to trigger imports.

use super::scene_core::Scene;
use crate::renderer::Renderer;
use crate::scene::animation::{AnimationClip, AnimationState, AnimationTarget};
use crate::scene::loader::{SceneImportDevice, SceneLoader};
use crate::scripting::PendingGltfImport;
use log::{error, warn};
use std::collections::HashMap;

#[derive(Default)]
pub struct ImportQueue;

impl ImportQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn process_pending_gltf_imports(&mut self, scene: &mut Scene, renderer: &mut Renderer) {
        let pending = scene.scripting_mut().take_pending_gltf_imports();
        if pending.is_empty() {
            return;
        }

        let mut textures_updated = false;

        for PendingGltfImport {
            parent,
            path,
            scale,
        } in pending
        {
            if scene.main_world().entity(parent).is_err() {
                warn!(
                    "Skipping glTF import for missing target entity {:?}: {:?}",
                    parent, path
                );
                continue;
            }

            match SceneLoader::load_gltf_asset(&path, renderer, scale) {
                Ok(mut bundle) => {
                    let registration = bundle.register_resources(renderer, &mut scene.assets);
                    if registration.textures_changed() {
                        textures_updated = true;
                    }

                    let mut instance = bundle.asset.instantiate(
                        Some(renderer as &mut dyn SceneImportDevice),
                        &mut scene.assets,
                    );
                    let (animations, animation_states) = instance.take_animation_data();
                    let entity_map = super::internal::composition::merge_world_as_child(
                        scene.main_world_mut(),
                        parent,
                        instance.into_world(),
                    );

                    self.attach_imported_animations(
                        scene,
                        animations,
                        animation_states,
                        &entity_map,
                    );
                }
                Err(err) => {
                    error!("Failed to import glTF {:?}: {err}", path);
                }
            }
        }

        if textures_updated {
            renderer.update_texture_bind_group(&scene.assets);
        }

        scene.propagate_transforms();

        for node in scene.nodes_iter_mut() {
            let instance = node.instance_mut();
            instance.begin_playback();
            instance.restore_rest_pose();
        }
    }

    pub(super) fn attach_imported_animations(
        &mut self,
        scene: &mut Scene,
        mut animations: Vec<AnimationClip>,
        animation_states: Vec<AnimationState>,
        entity_map: &HashMap<hecs::Entity, hecs::Entity>,
    ) {
        if animations.is_empty() && animation_states.is_empty() {
            return;
        }

        for clip in &mut animations {
            for channel in &mut clip.channels {
                if let AnimationTarget::Transform { entity, .. } = &mut channel.target {
                    if let Some(&mapped) = entity_map.get(entity) {
                        *entity = mapped;
                    } else {
                        warn!(
                            "Skipping animation channel targeting missing entity {:?}",
                            entity
                        );
                    }
                }
            }
        }

        let main_instance = scene.main_instance_mut();
        let mut clip_indices = Vec::with_capacity(animations.len());

        for clip in animations.into_iter() {
            let index = main_instance.add_animation_clip(clip);
            clip_indices.push(index);
        }

        for mut state in animation_states.into_iter() {
            let Some(&new_index) = clip_indices.get(state.clip_index) else {
                warn!(
                    "Skipping animation state referencing missing clip index {}",
                    state.clip_index
                );
                continue;
            };

            state.clip_index = new_index;
            if main_instance.push_animation_state(state).is_none() {
                warn!("Failed to attach animation state for clip {}", new_index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::animation::{
        AnimationChannel, AnimationInterpolation, AnimationOutput, AnimationSampler,
        TransformProperty,
    };
    use crate::scene::components::{TransformComponent, Visible};
    use crate::scene::transform::Transform;
    use glam::Vec3;
    use hecs::Entity;

    #[test]
    fn imported_animations_remap_entities() {
        let mut scene = Scene::new();
        let new_entity = scene
            .main_world_mut()
            .spawn((TransformComponent(Transform::IDENTITY), Visible(true)));

        let old_entity = Entity::from_bits((1u64 << 32) | 1).expect("entity bits");
        let mut clip = AnimationClip::new("Imported");
        clip.add_channel(AnimationChannel {
            sampler: AnimationSampler {
                times: vec![0.0, 1.0],
                output: AnimationOutput::Vec3(vec![Vec3::ZERO, Vec3::ONE]),
                interpolation: AnimationInterpolation::Linear,
            },
            target: AnimationTarget::Transform {
                entity: old_entity,
                property: TransformProperty::Translation,
            },
        });

        let mut queue = ImportQueue::new();
        let mut map = HashMap::new();
        map.insert(old_entity, new_entity);

        queue.attach_imported_animations(&mut scene, vec![clip], Vec::new(), &map);
        assert_eq!(scene.animation_states().len(), 0);
        assert_eq!(scene.animations().len(), 1);

        let added_clip = &scene.animations()[0];
        match &added_clip.channels[0].target {
            AnimationTarget::Transform { entity, .. } => assert_eq!(*entity, new_entity),
            _ => panic!("expected transform animation"),
        }
    }
}
