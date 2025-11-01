use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::project::{ProjectError, SceneDocument, SceneDocumentDependencies};

use super::{SceneAsset, SceneAssetEntity};

pub struct ResolvedPrefabNode<'a> {
    pub asset: &'a SceneAsset,
    pub entity_index: usize,
    pub entity: &'a SceneAssetEntity,
}

#[derive(Debug, Default)]
pub struct SceneLibrary {
    assets: HashMap<String, SceneAsset>,
    dependencies: HashMap<String, SceneDocumentDependencies>,
    paths: HashMap<String, PathBuf>,
}

impl SceneLibrary {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.assets.clear();
        self.dependencies.clear();
        self.paths.clear();
    }

    pub fn register_document(&mut self, document: &SceneDocument) {
        self.paths
            .insert(document.id.clone(), document.relative_path.clone());
        self.dependencies
            .insert(document.id.clone(), document.dependencies.clone());
    }

    pub fn load_document(
        &mut self,
        document: &SceneDocument,
        project_root: &Path,
    ) -> Result<&SceneAsset, ProjectError> {
        self.register_document(document);

        if !self.assets.contains_key(&document.id) {
            let path = project_root.join(&document.relative_path);
            let json = fs::read_to_string(&path)?;
            let asset = SceneAsset::from_json(&json).map_err(ProjectError::Serialization)?;
            self.assets.insert(document.id.clone(), asset);
        }

        Ok(self
            .assets
            .get(&document.id)
            .expect("scene asset should exist after loading"))
    }

    pub fn insert(&mut self, document_id: impl Into<String>, asset: SceneAsset) {
        self.assets.insert(document_id.into(), asset);
    }

    pub fn asset(&self, document_id: &str) -> Option<&SceneAsset> {
        self.assets.get(document_id)
    }

    pub fn set_dependencies(
        &mut self,
        document_id: impl Into<String>,
        dependencies: SceneDocumentDependencies,
    ) {
        self.dependencies.insert(document_id.into(), dependencies);
    }

    pub fn dependencies(&self, document_id: &str) -> Option<&SceneDocumentDependencies> {
        self.dependencies.get(document_id)
    }

    pub fn prefab_dependencies(&self, document_id: &str) -> Option<&BTreeSet<String>> {
        self.dependencies(document_id)
            .map(|deps| &deps.prefab_instances)
    }

    pub fn resolve_prefab_node<'a>(
        &'a self,
        document_id: &str,
        node_path: &[String],
    ) -> Option<ResolvedPrefabNode<'a>> {
        let asset = self.assets.get(document_id)?;
        let entity_index = resolve_prefab_entity_path(asset, node_path)?;
        let entity = asset.entities.get(entity_index)?;
        Some(ResolvedPrefabNode {
            asset,
            entity_index,
            entity,
        })
    }

    pub fn prefab_dependency_would_cycle(&self, source: &str, target: &str) -> bool {
        if source == target {
            return true;
        }

        let mut stack = vec![target];
        let mut visited = HashSet::new();

        while let Some(current) = stack.pop() {
            if !visited.insert(current.to_string()) {
                continue;
            }

            if current == source {
                return true;
            }

            if let Some(deps) = self.prefab_dependencies(current) {
                for dependency in deps {
                    stack.push(dependency);
                }
            }
        }

        false
    }

    pub fn track_prefab_dependency(
        &mut self,
        source_document: impl Into<String>,
        target_document: impl Into<String>,
    ) {
        let source_document = source_document.into();
        let target_document = target_document.into();
        self.dependencies
            .entry(source_document)
            .or_default()
            .prefab_instances
            .insert(target_document);
    }

    pub fn path(&self, document_id: &str) -> Option<&PathBuf> {
        self.paths.get(document_id)
    }
}

fn resolve_prefab_entity_path(asset: &SceneAsset, node_path: &[String]) -> Option<usize> {
    if node_path.is_empty() {
        return None;
    }

    let mut candidates: Vec<usize> = asset
        .entities
        .iter()
        .enumerate()
        .filter_map(|(index, entity)| entity.parent.is_none().then_some(index))
        .collect();

    let mut current_index = None;

    for segment in node_path {
        let mut next_index = None;
        for candidate in &candidates {
            let entity = &asset.entities[*candidate];
            if entity.name.as_deref() == Some(segment.as_str()) {
                next_index = Some(*candidate);
                break;
            }
        }

        let index = next_index?;

        current_index = Some(index);
        candidates = asset.entities[index].children.clone();
    }

    current_index
}
