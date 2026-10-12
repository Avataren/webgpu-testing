use hecs::Entity;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use wgpu_cube::scene::InputState;
use wgpu_cube::scripting::lua::api::ui::{UiCommand, UiResponse};
use wgpu_cube::scripting::{InputStateGuard, LuaScriptSource, ScriptingState, UiViewportInfo};

/// Type of plugin - distinguishes between editor-provided and user-created plugins
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum PluginType {
    /// Built-in editor plugin
    Editor,
    /// User-created plugin
    #[default]
    User,
}

/// Plugin metadata from ui_plugins.toml
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginMetadata {
    pub name: String,
    pub script: String,
    pub description: String,
    pub category: String,
    #[serde(default)]
    pub plugin_type: PluginType,
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
    /// Separate world for plugin entities (not part of the scene)
    world: hecs::World,
    /// Lua runtime for plugin scripts. Plugins live independently of scenes, so they
    /// keep their own VM and state across scene switches and play/stop.
    scripting: ScriptingState,
}

impl UiPluginManager {
    /// Create a new plugin manager
    pub fn new(base_path: PathBuf) -> Self {
        Self {
            plugins: Vec::new(),
            entity_to_plugin: HashMap::new(),
            base_path,
            world: hecs::World::new(),
            scripting: ScriptingState::default(),
        }
    }

    /// Run plugin script lifecycle hooks (`on_created`, `update`, coroutines, events).
    pub fn run_scripts(&mut self, dt: f64, editor_mode: bool, input: &InputState) {
        let _input = InputStateGuard::enter(input);
        if let Err(err) = self
            .scripting
            .process_scripts(&mut self.world, dt, editor_mode)
        {
            log::error!("Plugin scripting error: {err}");
        }
    }

    /// Collect UI commands from plugin `on_ui` callbacks.
    pub fn process_ui(
        &mut self,
        editor_mode: bool,
        viewport: Option<UiViewportInfo>,
        per_entity_viewports: &HashMap<Entity, UiViewportInfo>,
        input: &InputState,
    ) -> HashMap<Entity, Vec<UiCommand>> {
        let _input = InputStateGuard::enter(input);
        self.scripting.process_ui(
            &self.world,
            editor_mode,
            viewport,
            Some(per_entity_viewports),
        )
    }

    /// Feed widget responses from the last UI pass back to plugin scripts.
    pub fn set_ui_responses(&mut self, responses: HashMap<Entity, HashMap<String, UiResponse>>) {
        self.scripting.set_ui_responses(responses);
    }

    /// Most recent script error for a plugin, if any.
    pub fn last_error(&self, entity: Entity) -> Option<&str> {
        self.scripting.last_error(entity)
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

    /// Get mutable plugin by entity
    pub fn get_plugin_mut(&mut self, entity: Entity) -> Option<&mut LoadedPlugin> {
        let index = *self.entity_to_plugin.get(&entity)?;
        self.plugins.get_mut(index)
    }

    /// Get mutable access to the plugin world
    pub fn world_mut(&mut self) -> &mut hecs::World {
        &mut self.world
    }

    /// Resolve script path relative to base path
    pub fn resolve_script_path(&self, script_path: &str) -> PathBuf {
        self.base_path.join(script_path)
    }

    /// Find entity by absolute script file path
    /// Returns the entity and plugin metadata if found
    pub fn find_entity_by_path(&self, path: &Path) -> Option<(Entity, &PluginMetadata)> {
        // Normalize the input path
        let path = path.canonicalize().ok()?;

        for plugin in &self.plugins {
            let plugin_path = self.resolve_script_path(&plugin.metadata.script);
            if let Ok(canonical_plugin_path) = plugin_path.canonicalize() {
                if canonical_plugin_path == path {
                    return Some((plugin.entity, &plugin.metadata));
                }
            }
        }

        None
    }

    /// Hot-reload a plugin's script from disk, preserving its script state.
    ///
    /// Returns the plugin name on success.
    pub fn reload_plugin(&mut self, entity: Entity, path: &Path) -> Result<String, String> {
        let plugin_name = self
            .get_plugin(entity)
            .ok_or_else(|| format!("Plugin not found for entity {:?}", entity))?
            .metadata
            .name
            .clone();

        if !path.is_file() {
            return Err(format!("Script {} does not exist", path.display()));
        }

        if self.scripting.reload_path(path) == 0 {
            // The script was never instantiated (e.g. the plugin was added while its
            // file was missing); attach a fresh component so it gets compiled.
            self.world
                .insert_one(
                    entity,
                    wgpu_cube::scripting::LuaScriptComponent::new(LuaScriptSource::File {
                        path: path.to_path_buf(),
                    }),
                )
                .map_err(|e| format!("Failed to update component: {}", e))?;
        }

        log::info!("Reloaded Lua plugin '{}' from {:?}", plugin_name, path);
        Ok(plugin_name)
    }

    /// Unload all plugins by despawning their entities
    pub fn unload_all(&mut self) {
        log::info!("Unloading {} plugins", self.plugins.len());
        for plugin in &self.plugins {
            if let Err(e) = self.world.despawn(plugin.entity) {
                log::warn!("Failed to despawn plugin entity {:?}: {}", plugin.entity, e);
            } else {
                log::debug!(
                    "Despawned plugin '{}' entity {:?}",
                    plugin.metadata.name,
                    plugin.entity
                );
            }
        }
        self.plugins.clear();
        self.entity_to_plugin.clear();
    }
}

/// Helper to create LuaScriptSource from plugin metadata
pub fn create_plugin_script_source(
    manager: &UiPluginManager,
    metadata: &PluginMetadata,
) -> Result<LuaScriptSource, String> {
    let script_path = manager.resolve_script_path(&metadata.script);

    // Verify the file exists
    if !script_path.exists() {
        return Err(format!("Script file not found: {}", script_path.display()));
    }

    // Create source - use File source so it can be edited by script editor
    Ok(LuaScriptSource::File { path: script_path })
}
