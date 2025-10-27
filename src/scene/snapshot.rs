use super::internal::{gizmos, transform_gizmos};
use super::state::{TransformGizmoHandle, TransformGizmoMode, TransformGizmoSpace};
use super::{Scene, SceneTreeAsset};
use crate::asset::Assets;
use crate::environment::Environment;
use crate::scene::Camera;

#[derive(Clone)]
pub(crate) struct SceneSnapshot {
    pub(crate) assets: Assets,
    pub(crate) environment: Environment,
    pub(crate) camera: Camera,
    pub(crate) time: f64,
    pub(crate) tree: SceneTreeAsset,
    pub(crate) main_scene_path: Vec<usize>,
    pub(crate) gizmo_resources: Option<gizmos::GizmoResources>,
    pub(crate) transform_gizmo_resources: Option<transform_gizmos::TransformGizmoResources>,
    pub(crate) gizmo_mode: TransformGizmoMode,
    pub(crate) gizmo_space: TransformGizmoSpace,
    pub(crate) gizmo_hover: Option<TransformGizmoHandle>,
}

impl SceneSnapshot {
    pub(crate) fn capture(scene: &Scene) -> Self {
        let assets = scene.assets.clone();
        let environment = scene.environment().clone();
        let camera = *scene.camera();
        let time = scene.time();
        let tree = scene.export_tree_asset("SceneSnapshot");
        let main_scene_path = scene.path_from_root(scene.main_scene());
        let gizmo_resources = scene.gizmo_resources();
        let transform_gizmo_resources = scene.transform_gizmo_resources();
        let gizmo_mode = scene.transform_gizmo_mode();
        let gizmo_space = scene.transform_gizmo_space();
        let gizmo_hover = scene.transform_gizmo_hover();

        Self {
            assets,
            environment,
            camera,
            time,
            tree,
            main_scene_path,
            gizmo_resources,
            transform_gizmo_resources,
            gizmo_mode,
            gizmo_space,
            gizmo_hover,
        }
    }

    pub(crate) fn into_scene(self) -> Scene {
        let mut scene = Scene::new();
        scene.replace_assets(self.assets);
        scene.set_environment(self.environment);
        scene.set_camera(self.camera);
        scene.set_time(self.time);
        let tree_root = scene.instantiate_tree_asset(&self.tree, None);

        let main_scene = if self.main_scene_path.is_empty() {
            Some(tree_root)
        } else {
            let mut current = tree_root;
            let mut found = true;
            for &index in &self.main_scene_path {
                let children = scene.node_children(current);
                match children.get(index).copied() {
                    Some(child) => current = child,
                    None => {
                        found = false;
                        break;
                    }
                }
            }

            if found {
                Some(current)
            } else {
                scene.node_from_path(&self.main_scene_path)
            }
        };

        if let Some(main) = main_scene {
            scene.set_main_scene(main);
        }

        scene.propagate_transforms();

        scene.set_gizmo_resources(self.gizmo_resources);
        scene.set_transform_gizmo_resources(self.transform_gizmo_resources);
        scene.set_transform_gizmo_mode(self.gizmo_mode);
        scene.set_transform_gizmo_space(self.gizmo_space);
        scene.set_transform_gizmo_hover(self.gizmo_hover);

        scene
    }

    pub fn restore_without_assets(&self, scene: &mut Scene) {
        // Clear current entities
        let entities_to_remove: Vec<_> = scene
            .main_world()
            .iter()
            .map(|entity_ref| entity_ref.entity())
            .collect();

        for entity in entities_to_remove {
            let _ = scene.main_world_mut().despawn(entity);
        }

        // Restore the tree (uses current asset storage)
        let tree_root = scene.instantiate_tree_asset(&self.tree, None);

        // Restore main scene
        let main_scene = if self.main_scene_path.is_empty() {
            Some(tree_root)
        } else {
            scene
                .node_from_path(&self.main_scene_path)
                .or(Some(tree_root))
        };

        if let Some(main) = main_scene {
            scene.set_main_scene(main);
        }

        // Restore other state (but NOT assets)
        scene.set_camera(self.camera);
        scene.set_environment(self.environment.clone());
        scene.set_time(self.time);

        // Restore gizmo state
        scene.set_gizmo_resources(self.gizmo_resources);
        scene.set_transform_gizmo_resources(self.transform_gizmo_resources);
        scene.set_transform_gizmo_mode(self.gizmo_mode);
        scene.set_transform_gizmo_space(self.gizmo_space);
        scene.set_transform_gizmo_hover(self.gizmo_hover);

        scene.propagate_transforms();
    }
}

#[derive(Clone)]
pub struct SceneStateSnapshot {
    snapshot: SceneSnapshot,
}

impl SceneStateSnapshot {
    pub fn capture(scene: &Scene) -> Self {
        Self {
            snapshot: SceneSnapshot::capture(scene),
        }
    }

    pub fn into_scene(self) -> Scene {
        self.snapshot.into_scene()
    }

    pub fn restore(&self, scene: &mut Scene) {
        *scene = self.snapshot.clone().into_scene();
    }

    pub fn restore_without_assets(&self, scene: &mut Scene) {
        self.snapshot.restore_without_assets(scene);
    }
}
