use super::assets::{SceneAsset, ScenePrefabOverrides, ScenePrefabRef, SerializedTransform};
use super::components::Visible;
use super::graph::{PrefabOriginMetadata, SceneNodeId};
use super::library::SceneLibrary;
use super::loader::SceneImportDevice;
use super::scene::Scene;
use super::transform::Transform;
use log::warn;
use std::collections::HashMap;

/// Descriptor for a prefab instance that tracks its origin document and node path
#[derive(Clone)]
pub(crate) struct PrefabInstanceDescriptor {
    document_id: String,
    node_path: Vec<String>,
}

impl PrefabInstanceDescriptor {
    pub fn new(document_id: String, node_path: Vec<String>) -> Self {
        Self {
            document_id,
            node_path,
        }
    }

    pub fn document_id(&self) -> &str {
        &self.document_id
    }

    pub fn node_path(&self) -> &[String] {
        &self.node_path
    }
}

/// Helper struct that handles prefab instantiation logic for Scene
pub(crate) struct PrefabInstantiator<'a> {
    scene: &'a mut Scene,
    library: &'a mut SceneLibrary,
}

impl<'a> PrefabInstantiator<'a> {
    pub fn new(scene: &'a mut Scene, library: &'a mut SceneLibrary) -> Self {
        Self { scene, library }
    }

    /// Instantiates an asset with full prefab support (internal implementation)
    #[allow(clippy::too_many_arguments)]
    pub fn instantiate_asset_internal(
        &mut self,
        asset: &SceneAsset,
        name: String,
        parent: Option<SceneNodeId>,
        renderer: &mut Option<&mut dyn SceneImportDevice>,
        document_id: Option<&str>,
        prefab_stack: &mut Vec<String>,
        prefab_origin: Option<PrefabInstanceDescriptor>,
    ) -> SceneNodeId {
        let parent_id = parent.unwrap_or(self.scene.root_id());
        assert!(
            self.scene.is_valid_node(parent_id),
            "Invalid parent node"
        );

        let (instance, renderer_restored) = match renderer.take() {
            Some(device) => {
                let instance = asset.instantiate(Some(device), &mut self.scene.assets);
                (instance, Some(device))
            }
            None => {
                let instance = asset.instantiate(None, &mut self.scene.assets);
                (instance, None)
            }
        };
        *renderer = renderer_restored;

        let should_apply_camera = parent.is_none() && self.scene.main_scene() == self.scene.root_id();
        let active_camera_entity = if should_apply_camera {
            instance.active_camera()
        } else {
            None
        };

        let id = self.scene.allocate_node(name, instance);
        let prefab_metadata = prefab_origin.map(|descriptor| {
            let paths = prefab_entity_paths_for_asset(asset, descriptor.node_path());
            PrefabOriginMetadata::new(descriptor.document_id().to_string(), paths)
        });

        {
            let node = self.scene.node_mut(id);
            node.set_local_transform(Transform::from(asset.root_transform.clone()));
            node.set_prefab_origin(prefab_metadata);
        }

        self.scene.attach_node(id, parent_id);
        self.scene.update_world_transforms();
        self.instantiate_prefab_children(id, asset, renderer, document_id, prefab_stack);

        if should_apply_camera {
            self.scene.set_active_camera_entity(active_camera_entity);
        }
        self.scene.refresh_environment_state();

        id
    }

    /// Instantiates prefab references found within an asset
    fn instantiate_prefab_children(
        &mut self,
        parent_node: SceneNodeId,
        asset: &SceneAsset,
        renderer: &mut Option<&mut dyn SceneImportDevice>,
        document_id: Option<&str>,
        prefab_stack: &mut Vec<String>,
    ) {
        let prefabs: Vec<(usize, ScenePrefabRef, Option<String>)> = asset
            .entities
            .iter()
            .enumerate()
            .filter_map(|(index, entity)| {
                entity
                    .scene_ref
                    .clone()
                    .map(|reference| (index, reference, entity.name.clone()))
            })
            .collect();

        for (index, prefab, name_override) in prefabs {
            let target_document = prefab.document.clone();

            if let Some(source) = document_id {
                if prefab_stack.iter().any(|entry| entry == &target_document)
                    || self
                        .library
                        .prefab_dependency_would_cycle(source, &target_document)
                {
                    warn!(
                        "Skipping prefab instantiation from {source} -> {target_document} due to detected cycle"
                    );
                    continue;
                }
            }

            let resolved = match self
                .library
                .resolve_prefab_node(&target_document, &prefab.node_path)
            {
                Some(resolved) => resolved,
                None => {
                    warn!(
                        "Failed to resolve prefab node {:?} in document {}",
                        prefab.node_path, target_document
                    );
                    continue;
                }
            };

            let child_asset = clone_prefab_asset_subtree(resolved.asset, resolved.entity_index);
            let base_transform = asset_entity_accumulated_transform(asset, index);
            prefab_stack.push(target_document.clone());

            let child_name = name_override
                .or_else(|| resolved.entity.name.clone())
                .unwrap_or_else(|| resolved.asset.name.clone());

            let descriptor =
                PrefabInstanceDescriptor::new(target_document.clone(), prefab.node_path.clone());

            let child_id = self.instantiate_asset_internal(
                &child_asset,
                child_name,
                Some(parent_node),
                renderer,
                Some(&target_document),
                prefab_stack,
                Some(descriptor),
            );

            prefab_stack.pop();

            self.apply_prefab_overrides(child_id, base_transform, &prefab.overrides, false);

            if let Some(source) = document_id {
                self.library
                    .track_prefab_dependency(source.to_string(), target_document);
            }

            self.remove_placeholder_entity(parent_node, index);
        }
    }

    /// Instantiates a standalone prefab reference (used in tree assets)
    #[allow(clippy::too_many_arguments)]
    pub fn instantiate_prefab_reference(
        &mut self,
        prefab: &ScenePrefabRef,
        name: String,
        parent: SceneNodeId,
        renderer: &mut Option<&mut dyn SceneImportDevice>,
        document_id: Option<&str>,
        prefab_stack: &mut Vec<String>,
    ) -> SceneNodeId {
        let target_document = prefab.document.clone();

        if let Some(source) = document_id {
            if prefab_stack.iter().any(|entry| entry == &target_document)
                || self
                    .library
                    .prefab_dependency_would_cycle(source, &target_document)
            {
                warn!(
                    "Skipping prefab instantiation from {source} -> {target_document} due to detected cycle"
                );
                return self.scene.create_node(name, Some(parent));
            }
        }

        let resolved = match self
            .library
            .resolve_prefab_node(&target_document, &prefab.node_path)
        {
            Some(resolved) => resolved,
            None => {
                warn!(
                    "Failed to resolve prefab node {:?} in document {}",
                    prefab.node_path, target_document
                );
                return self.scene.create_node(name, Some(parent));
            }
        };

        let child_asset = clone_prefab_asset_subtree(resolved.asset, resolved.entity_index);
        let base_transform = Transform::IDENTITY;
        prefab_stack.push(target_document.clone());

        let descriptor =
            PrefabInstanceDescriptor::new(target_document.clone(), prefab.node_path.clone());

        let node_id = self.instantiate_asset_internal(
            &child_asset,
            name,
            Some(parent),
            renderer,
            Some(&target_document),
            prefab_stack,
            Some(descriptor),
        );

        prefab_stack.pop();

        self.apply_prefab_overrides(node_id, base_transform, &prefab.overrides, true);

        if let Some(source) = document_id {
            self.library
                .track_prefab_dependency(source.to_string(), target_document);
        }

        node_id
    }

    /// Applies prefab overrides to an instantiated node
    fn apply_prefab_overrides(
        &mut self,
        node_id: SceneNodeId,
        base_transform: Transform,
        overrides: &ScenePrefabOverrides,
        skip_transform_update: bool,
    ) {
        if !skip_transform_update {
            self.scene
                .node_mut(node_id)
                .set_local_transform(base_transform);
        }

        if let Some(transform) = &overrides.transform {
            let mut local = self.scene.node_local_transform_mut(node_id);
            *local = Transform::from(transform.clone());
        }

        if let Some(visible) = overrides.visible {
            if let Some(entity) = self.scene.node(node_id).instance().asset_entity(0) {
                let _ = self
                    .scene
                    .set_entity_component_override(node_id, entity, Visible(visible));
            }
        }
    }

    /// Removes a placeholder entity that was replaced by a prefab
    fn remove_placeholder_entity(&mut self, parent: SceneNodeId, asset_index: usize) {
        let placeholder = {
            let node_ref = self.scene.node(parent);
            node_ref.instance().asset_entity(asset_index)
        };

        if let Some(entity) = placeholder {
            let instance = self.scene.node_mut(parent).instance_mut();
            let _ = instance.world_mut().despawn(entity);
            instance.clear_asset_entity(asset_index);
        }
    }
}

/// Clones a subtree from a scene asset starting at the given root index
pub(crate) fn clone_prefab_asset_subtree(asset: &SceneAsset, root_index: usize) -> SceneAsset {
    let mut indices = Vec::new();
    collect_prefab_indices(asset, root_index, &mut indices);

    let mut index_map = HashMap::new();
    for (new_index, old_index) in indices.iter().enumerate() {
        index_map.insert(*old_index, new_index);
    }

    let mut entities = Vec::with_capacity(indices.len());
    for old_index in &indices {
        let mut entity = asset.entities[*old_index].clone();
        entity.parent = entity
            .parent
            .and_then(|parent| index_map.get(&parent).copied());
        entity.children = entity
            .children
            .iter()
            .filter_map(|child| index_map.get(child).copied())
            .collect();
        entities.push(entity);
    }

    if let Some(root) = entities.get_mut(0) {
        root.parent = None;
    }

    let active_camera = asset
        .active_camera
        .and_then(|camera| index_map.get(&camera).copied());

    SceneAsset {
        name: format!("{}::prefab", asset.name),
        root_transform: SerializedTransform::identity(),
        entities,
        animations: Vec::new(),
        animation_states: Vec::new(),
        mesh_data: asset.mesh_data.clone(),
        active_camera,
    }
}

/// Recursively collects all entity indices in a subtree
fn collect_prefab_indices(asset: &SceneAsset, index: usize, output: &mut Vec<usize>) {
    output.push(index);

    for child in &asset.entities[index].children {
        collect_prefab_indices(asset, *child, output);
    }
}

/// Calculates the accumulated transform for an entity in an asset's hierarchy
pub(crate) fn asset_entity_accumulated_transform(
    asset: &SceneAsset,
    entity_index: usize,
) -> Transform {
    let mut chain = Vec::new();
    let mut current = Some(entity_index);

    while let Some(index) = current {
        chain.push(index);
        current = asset.entities[index].parent;
    }

    let mut transform = Transform::IDENTITY;
    for index in chain.iter().rev() {
        let local = Transform::from(asset.entities[*index].transform.clone());
        transform = transform.mul_transform(&local);
    }

    transform
}

/// Builds entity paths for all entities in a prefab asset
pub(crate) fn prefab_entity_paths_for_asset(
    asset: &SceneAsset,
    root_path: &[String],
) -> Vec<Option<Vec<String>>> {
    let mut paths = vec![None; asset.entities.len()];
    if asset.entities.is_empty() {
        return paths;
    }

    let root_index = asset
        .entities
        .iter()
        .enumerate()
        .find_map(|(index, entity)| entity.parent.is_none().then_some(index))
        .unwrap_or(0);

    assign_prefab_paths(asset, root_index, root_path.to_vec(), &mut paths);
    paths
}

/// Recursively assigns paths to entities in a prefab hierarchy
fn assign_prefab_paths(
    asset: &SceneAsset,
    index: usize,
    current_path: Vec<String>,
    output: &mut [Option<Vec<String>>],
) {
    output[index] = Some(current_path.clone());

    for &child in &asset.entities[index].children {
        let mut child_path = current_path.clone();
        let name = asset.entities[child]
            .name
            .clone()
            .unwrap_or_else(|| format!("Entity{child}"));
        child_path.push(name);
        assign_prefab_paths(asset, child, child_path, output);
    }
}
