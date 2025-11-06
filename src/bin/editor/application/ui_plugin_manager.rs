use hecs::Entity;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use wgpu_cube::scripting::RuneScriptSource;

/// Plugin metadata from ui_plugins.toml
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginMetadata {
    pub name: String,
    pub script: String,
    pub description: String,
    pub category: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub default_visible: bool,
}

fn default_true() -> bool {
    true
}

/// Plugin manifest file structure
#[derive(Debug, Deserialize, Serialize)]
pub struct PluginManifest {
    #[serde(rename = "plugin")]
    pub plugins: Vec<PluginMetadata>,
}

/// Runtime state for a loaded plugin
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct LoadedPlugin {
    pub metadata: PluginMetadata,
    pub entity: Entity,
    pub visible: bool,
}

/// Manages UI plugins loaded from manifest
pub struct UiPluginManager {
    /// All loaded plugins
    plugins: Vec<LoadedPlugin>,
    /// Map from entity to plugin index
    entity_to_plugin: HashMap<Entity, usize>,
    /// Base path for resolving script paths
    base_path: PathBuf,
}

impl UiPluginManager {
    /// Create a new plugin manager
    pub fn new(base_path: PathBuf) -> Self {
        Self {
            plugins: Vec::new(),
            entity_to_plugin: HashMap::new(),
            base_path,
        }
    }

    /// Load plugins from a manifest file
    pub fn load_manifest(&mut self, manifest_path: &Path) -> Result<PluginManifest, String> {
        let content = std::fs::read_to_string(manifest_path)
            .map_err(|e| format!("Failed to read manifest: {}", e))?;

        let manifest: PluginManifest =
            toml::from_str(&content).map_err(|e| format!("Failed to parse manifest: {}", e))?;

        log::info!("Loaded {} plugins from manifest", manifest.plugins.len());
        for plugin in &manifest.plugins {
            log::info!(
                "  - {} ({}) - enabled: {}, visible: {}",
                plugin.name,
                plugin.script,
                plugin.enabled,
                plugin.default_visible
            );
        }

        Ok(manifest)
    }

    /// Register a loaded plugin
    pub fn register_plugin(&mut self, metadata: PluginMetadata, entity: Entity) {
        let visible = metadata.default_visible;
        let plugin = LoadedPlugin {
            metadata,
            entity,
            visible,
        };

        let index = self.plugins.len();
        self.plugins.push(plugin);
        self.entity_to_plugin.insert(entity, index);

        log::debug!("Registered plugin entity: {:?}", entity);
    }

    /// Get all plugins
    #[allow(dead_code)]
    pub fn plugins(&self) -> &[LoadedPlugin] {
        &self.plugins
    }

    /// Get mutable access to plugins
    pub fn plugins_mut(&mut self) -> &mut [LoadedPlugin] {
        &mut self.plugins
    }

    /// Get visible plugin entities
    #[allow(dead_code)]
    pub fn visible_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.plugins
            .iter()
            .filter(|p| p.visible && p.metadata.enabled)
            .map(|p| p.entity)
    }

    /// Set plugin visibility
    #[allow(dead_code)]
    pub fn set_visibility(&mut self, entity: Entity, visible: bool) {
        if let Some(&index) = self.entity_to_plugin.get(&entity) {
            self.plugins[index].visible = visible;
        }
    }

    /// Toggle plugin visibility
    #[allow(dead_code)]
    pub fn toggle_visibility(&mut self, entity: Entity) -> bool {
        if let Some(&index) = self.entity_to_plugin.get(&entity) {
            self.plugins[index].visible = !self.plugins[index].visible;
            self.plugins[index].visible
        } else {
            false
        }
    }

    /// Get plugin by entity
    pub fn get_plugin(&self, entity: Entity) -> Option<&LoadedPlugin> {
        self.entity_to_plugin
            .get(&entity)
            .and_then(|&index| self.plugins.get(index))
    }

    /// Resolve script path relative to base path
    pub fn resolve_script_path(&self, script_path: &str) -> PathBuf {
        self.base_path.join(script_path)
    }
}

/// Helper to create RuneScriptSource from plugin metadata
pub fn create_plugin_script_source(
    manager: &UiPluginManager,
    metadata: &PluginMetadata,
) -> Result<RuneScriptSource, String> {
    let script_path = manager.resolve_script_path(&metadata.script);

    // Read script source
    let source_code = std::fs::read_to_string(&script_path)
        .map_err(|e| format!("Failed to read script {}: {}", script_path.display(), e))?;

    // Create source - use inline source with the plugin name
    Ok(RuneScriptSource::inline(&metadata.name, source_code))
}
