use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::project::{ProjectError, SceneDocument, SceneDocumentDependencies};

use super::SceneAsset;

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
