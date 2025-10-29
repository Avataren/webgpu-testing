use super::importer::ImportQueue;
use super::loader::SceneImportDevice;
use super::scene::Scene;
use crate::renderer::Renderer;
use hecs::Entity;

pub(crate) struct SceneImports {
    queue: ImportQueue,
}

impl SceneImports {
    pub(crate) fn new() -> Self {
        Self {
            queue: ImportQueue::new(),
        }
    }

    pub(crate) fn merge_as_child(
        &mut self,
        scene: &mut Scene,
        parent_entity: Entity,
        other: Scene,
        renderer: &mut dyn SceneImportDevice,
    ) {
        if let Some(asset) = other.export_main_asset("MergedScene") {
            let mut instance = asset.instantiate(Some(renderer), &mut scene.assets);
            let (animations, animation_states) = instance.take_animation_data();
            let entity_map = scene.merge_world_as_child(parent_entity, instance.into_world());
            scene.attach_imported_animations(animations, animation_states, &entity_map);
            scene.refresh_environment_state();
        }
    }

    pub(crate) fn process_pending_gltf_imports(
        &mut self,
        scene: &mut Scene,
        renderer: &mut Renderer,
    ) {
        self.queue.process_pending_gltf_imports(scene, renderer);
    }
}

impl Default for SceneImports {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::SceneImports;
    use crate::scene::animation::{
        AnimationChannel, AnimationClip, AnimationInterpolation, AnimationOutput, AnimationSampler,
        AnimationTarget, TransformProperty,
    };
    use crate::scene::components::{Name, TransformComponent, Visible};
    use crate::scene::loader::SceneImportDevice;
    use crate::scene::{Scene, Transform};
    use glam::Vec3;

    struct PanicDevice;

    impl SceneImportDevice for PanicDevice {
        fn device(&self) -> &wgpu::Device {
            panic!("renderer device should not be used in tests")
        }

        fn queue(&self) -> &wgpu::Queue {
            panic!("renderer queue should not be used in tests")
        }

        fn create_mesh(
            &mut self,
            _vertices: &[crate::renderer::Vertex],
            _indices: &[u32],
        ) -> crate::asset::Mesh {
            panic!("mesh creation should not be used in tests")
        }
    }

    #[test]
    fn merge_as_child_attaches_animations() {
        let mut scene = Scene::new();
        let parent = scene.main_world_mut().spawn((
            Name::new("Parent"),
            TransformComponent(Transform::default()),
            Visible(true),
        ));

        let mut other = Scene::new();
        let child = other.main_world_mut().spawn((
            Name::new("Child"),
            TransformComponent(Transform::default()),
            Visible(true),
        ));

        let sampler = AnimationSampler {
            times: vec![0.0, 1.0],
            output: AnimationOutput::Vec3(vec![Vec3::ZERO, Vec3::splat(1.0)]),
            interpolation: AnimationInterpolation::Linear,
        };

        let mut clip = AnimationClip::new("MoveChild");
        clip.add_channel(AnimationChannel {
            sampler,
            target: AnimationTarget::Transform {
                entity: child,
                property: TransformProperty::Translation,
            },
        });

        let clip_index = other.add_animation_clip(clip);
        other.play_animation(clip_index, true);

        let mut imports = SceneImports::new();
        let mut renderer = PanicDevice;

        imports.merge_as_child(&mut scene, parent, other, &mut renderer);

        assert_eq!(scene.animations().len(), 1);
        assert_eq!(scene.animation_states().len(), 1);
    }
}
