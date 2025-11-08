use super::core::SceneAsset;
use super::entity::SceneAssetEntity;
use super::prefabs::ScenePrefabRef;
use super::serialization::SerializedTransform;
use crate::asset::Assets;
use crate::scene::transform::Transform;
use hecs::{Entity, World};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneTreeAsset {
    pub name: String,
    pub root: SceneTreeAssetNode,
}

impl SceneTreeAsset {
    pub fn new(name: impl Into<String>, root: SceneTreeAssetNode) -> Self {
        Self {
            name: name.into(),
            root,
        }
    }

    pub fn to_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }

    pub fn from_json(json: &str) -> serde_json::Result<Self> {
        serde_json::from_str(json)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneTreeAssetNode {
    pub name: String,
    pub transform: SerializedTransform,
    pub asset: Option<SceneAsset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_ref: Option<ScenePrefabRef>,
    #[serde(default)]
    pub children: Vec<SceneTreeAssetNode>,
}

impl SceneTreeAssetNode {
    pub fn builder(name: impl Into<String>) -> SceneTreeAssetNodeBuilder {
        SceneTreeAssetNodeBuilder::new(name)
    }
}

pub struct SceneTreeAssetNodeBuilder {
    name: String,
    transform: SerializedTransform,
    asset: Option<SceneAsset>,
    scene_ref: Option<ScenePrefabRef>,
    children: Vec<SceneTreeAssetNode>,
}

impl SceneTreeAssetNodeBuilder {
    fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            transform: SerializedTransform::identity(),
            asset: None,
            scene_ref: None,
            children: Vec::new(),
        }
    }

    pub fn with_transform(mut self, transform: SerializedTransform) -> Self {
        self.transform = transform;
        self
    }

    pub fn with_asset(mut self, asset: SceneAsset) -> Self {
        self.asset = Some(asset);
        self
    }

    pub fn with_scene_ref(mut self, scene_ref: ScenePrefabRef) -> Self {
        self.scene_ref = Some(scene_ref);
        self
    }

    pub fn with_children<I>(mut self, children: I) -> Self
    where
        I: IntoIterator<Item = SceneTreeAssetNode>,
    {
        self.children = children.into_iter().collect();
        self
    }

    pub fn push_child(&mut self, child: SceneTreeAssetNode) {
        self.children.push(child);
    }

    pub fn add_child(mut self, child: SceneTreeAssetNode) -> Self {
        self.children.push(child);
        self
    }

    pub fn build(self) -> SceneTreeAssetNode {
        SceneTreeAssetNode {
            name: self.name,
            transform: self.transform,
            asset: self.asset,
            scene_ref: self.scene_ref,
            children: self.children,
        }
    }
}

pub(crate) fn serialize_world(
    world: &World,
    assets: &Assets,
) -> (Vec<SceneAssetEntity>, HashMap<Entity, usize>) {
    let mut entities: Vec<Entity> = Vec::new();
    for (entity, _) in world.query::<()>().iter() {
        // Skip editor plugin entities - they should not be serialized with the scene
        if world
            .get::<&crate::scene::components::EditorPlugin>(entity)
            .is_ok()
        {
            continue;
        }
        entities.push(entity);
    }

    let index_map: HashMap<Entity, usize> = entities
        .iter()
        .enumerate()
        .map(|(idx, entity)| (*entity, idx))
        .collect();

    let serialized = entities
        .iter()
        .map(|entity| SceneAssetEntity::from_world_entity(*entity, world, assets, &index_map))
        .collect();

    (serialized, index_map)
}

pub(crate) fn build_tree_asset_node(
    node_name: &str,
    local_transform: Transform,
    asset: Option<SceneAsset>,
    children: Vec<SceneTreeAssetNode>,
) -> SceneTreeAssetNode {
    SceneTreeAssetNode {
        name: node_name.to_string(),
        transform: SerializedTransform::from(local_transform),
        asset,
        scene_ref: None,
        children,
    }
}
