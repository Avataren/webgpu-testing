#[cfg(feature = "egui")]
use crate::asset::{
    Handle, MaterialAsset, MaterialKind, MaterialTextureReference, MaterialTextureSlot,
};
#[cfg(feature = "egui")]
use crate::renderer::Material;
#[cfg(feature = "egui")]
use crate::scene::components::Billboard;
#[cfg(feature = "egui")]
use crate::scene::workspace::SceneDocumentId;
#[cfg(feature = "egui")]
use crate::scene::{
    CameraComponent, CanCastShadow, Children, DirectionalLight, EnvironmentComponent,
    LightProbeVolume, MaterialComponent, MeshComponent, Name, Parent, ParticleBehaviorPreset,
    ParticleEmitterComponent, ParticleSystemComponent, PointLight, PostProcessVolume, Scene,
    SceneHandle, ScenePrefabEntityMetadata, SceneWorkspace, SpotLight, Transform,
    TransformComponent,
};
#[cfg(feature = "egui")]
use crate::scripting::LuaScriptComponent;
#[cfg(feature = "egui")]
use crate::ui::egui;
#[cfg(feature = "egui")]
use egui::collapsing_header::CollapsingState;
#[cfg(feature = "egui")]
use hecs::Entity;
#[cfg(feature = "egui")]
use std::collections::{BTreeMap, BTreeSet, HashMap};
#[cfg(feature = "egui")]
use std::sync::{Arc, Mutex};

#[cfg(feature = "egui")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScenePrimitivePreset {
    Cube,
    Sphere,
    Plane,
    Cylinder,
    Cone,
    Torus,
}

#[cfg(feature = "egui")]
impl ScenePrimitivePreset {
    pub const fn variants() -> [ScenePrimitivePreset; 6] {
        [
            ScenePrimitivePreset::Cube,
            ScenePrimitivePreset::Sphere,
            ScenePrimitivePreset::Plane,
            ScenePrimitivePreset::Cylinder,
            ScenePrimitivePreset::Cone,
            ScenePrimitivePreset::Torus,
        ]
    }

    pub fn display_name(self) -> &'static str {
        match self {
            ScenePrimitivePreset::Cube => "Cube",
            ScenePrimitivePreset::Sphere => "Sphere",
            ScenePrimitivePreset::Plane => "Plane",
            ScenePrimitivePreset::Cylinder => "Cylinder",
            ScenePrimitivePreset::Cone => "Cone",
            ScenePrimitivePreset::Torus => "Torus",
        }
    }
}

#[cfg(feature = "egui")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneCreationAction {
    Empty,
    Primitive(ScenePrimitivePreset),
    ParticleSystem(ParticleBehaviorPreset),
    PointLight,
    DirectionalLight,
    SpotLight,
    Camera,
    Environment,
    PostProcessVolume,
    LightProbeVolume,
    SceneInstance {
        document_id: SceneDocumentId,
        node_path: Vec<String>,
    },
}

#[cfg(feature = "egui")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneHierarchyEvent {
    Create(SceneCreationAction),
    OpenScene(SceneDocumentId),
    AddScriptToEntity(Entity),
    DeleteEntity(Entity),
}

#[cfg(feature = "egui")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneHierarchySceneDescriptor {
    pub document_id: SceneDocumentId,
    pub display_name: String,
}

#[cfg(feature = "egui")]
impl SceneHierarchySceneDescriptor {
    pub fn new(document_id: SceneDocumentId, display_name: impl Into<String>) -> Self {
        Self {
            document_id,
            display_name: display_name.into(),
        }
    }
}

#[cfg(feature = "egui")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneHierarchyWorkspaceInfo {
    active_document: Option<SceneDocumentId>,
    open_documents: BTreeSet<SceneDocumentId>,
    available_scenes: Vec<SceneHierarchySceneDescriptor>,
}

#[cfg(feature = "egui")]
impl SceneHierarchyWorkspaceInfo {
    pub fn from_workspace(workspace: &SceneWorkspace) -> Self {
        let active_document = workspace.active_document_id().cloned();
        let open_documents = workspace
            .scene_handles()
            .filter_map(|handle| workspace.document_id_for_handle(handle).cloned())
            .collect();

        Self {
            active_document,
            open_documents,
            available_scenes: Vec::new(),
        }
    }

    pub fn active_document_id(&self) -> Option<&SceneDocumentId> {
        self.active_document.as_ref()
    }

    pub fn is_open(&self, document_id: &str) -> bool {
        self.open_documents.contains(document_id)
    }

    pub fn is_foreign_document(&self, document_id: &str) -> bool {
        self.active_document.as_deref() != Some(document_id)
    }

    pub fn available_scenes(&self) -> &[SceneHierarchySceneDescriptor] {
        &self.available_scenes
    }

    pub fn set_available_scenes(&mut self, scenes: Vec<SceneHierarchySceneDescriptor>) {
        self.available_scenes = scenes;
    }
}

#[cfg(feature = "egui")]
#[derive(Clone, Debug)]
struct SceneHierarchyNode {
    entity: Entity,
    name: String,
    parent: Option<Entity>,
    children: Vec<Entity>,
}

#[cfg(feature = "egui")]
#[derive(Clone, Debug, Default)]
pub struct SceneHierarchySnapshot {
    roots: Vec<Entity>,
    nodes: BTreeMap<Entity, SceneHierarchyNode>,
    components: BTreeMap<Entity, SceneEntityComponentsSummary>,
    prefabs: BTreeMap<Entity, ScenePrefabEntityMetadata>,
}

#[cfg(feature = "egui")]
#[derive(Clone, Debug)]
pub struct InspectorMaterial {
    pub handle: Handle<MaterialAsset>,
    pub material: Material,
    pub textures: Vec<(MaterialTextureSlot, MaterialTextureReference)>,
    pub kind: MaterialKind,
    pub canonical_path: Option<std::path::PathBuf>,
}

#[cfg(feature = "egui")]
#[derive(Clone, Debug, Default)]
pub struct SceneEntityComponentsSummary {
    pub camera: Option<CameraComponent>,
    pub transform: Option<Transform>,
    pub mesh: Option<MeshComponent>,
    pub material: Option<InspectorMaterial>,
    pub script: Option<LuaScriptComponent>,
    pub point_light: Option<PointLight>,
    pub directional_light: Option<DirectionalLight>,
    pub spot_light: Option<SpotLight>,
    pub can_cast_shadow: Option<CanCastShadow>,
    pub particle_system: Option<ParticleSystemComponent>,
    pub particle_emitter: Option<ParticleEmitterComponent>,
    pub environment: Option<EnvironmentComponent>,
    pub post_process_volume: Option<PostProcessVolume>,
    pub light_probe_volume: Option<LightProbeVolume>,
    pub billboard: Option<Billboard>,
    pub rigid_body: Option<crate::physics::RigidBody>,
    pub collider: Option<crate::physics::Collider>,
    pub character_controller: Option<crate::physics::CharacterController>,
    pub spring_camera: Option<crate::physics::SpringCamera>,
    pub animator: Option<crate::scene::skeletal::Animator>,
    pub animation_tree: Option<crate::scene::skeletal::AnimationTree>,
    #[cfg(feature = "navigation")]
    pub nav_mesh_surface: Option<crate::navigation::NavMeshSurface>,
    #[cfg(feature = "navigation")]
    pub nav_agent: Option<crate::navigation::NavAgent>,
    #[cfg(feature = "navigation")]
    pub nav_obstacle: Option<crate::navigation::NavObstacle>,
    #[cfg(feature = "navigation")]
    pub nav_modifier: Option<crate::navigation::NavModifier>,
    #[cfg(feature = "navigation")]
    pub nav_link: Option<crate::navigation::NavLink>,
    pub ai_agent: Option<crate::ai::AiAgent>,
}

#[cfg(feature = "egui")]
#[derive(Clone, Debug)]
pub struct SceneEntityInspectorData {
    pub entity: Entity,
    pub name: String,
    pub components: SceneEntityComponentsSummary,
}

#[cfg(feature = "egui")]
impl SceneHierarchySnapshot {
    pub fn from_scene(scene: &Scene) -> Self {
        let world = scene.world();
        let mut nodes = BTreeMap::new();
        let mut components = BTreeMap::new();
        let prefabs = scene.prefab_entity_metadata();

        for entity_ref in world.iter() {
            let entity = entity_ref.entity();

            // Skip editor plugin entities - they should not appear in the scene hierarchy
            if world
                .get::<&crate::scene::components::EditorPlugin>(entity)
                .is_ok()
            {
                continue;
            }

            let name = world
                .get::<&Name>(entity)
                .map(|name| name.0.clone())
                .unwrap_or_else(|_| format!("Entity {:?}", entity));
            let parent = world.get::<&Parent>(entity).ok().map(|p| p.0);
            let children = world
                .get::<&Children>(entity)
                .map(|children| children.0.clone())
                .unwrap_or_default();

            let transform = entity_ref
                .get::<&TransformComponent>()
                .map(|component| component.0);
            let camera = entity_ref
                .get::<&CameraComponent>()
                .map(|component| *component);
            let mesh = entity_ref
                .get::<&MeshComponent>()
                .map(|component| *component);
            let material = entity_ref
                .get::<&MaterialComponent>()
                .and_then(|component| {
                    scene
                        .assets
                        .material(component.0)
                        .map(|asset| InspectorMaterial {
                            handle: component.0,
                            material: *asset.material(),
                            textures: asset
                                .texture_references()
                                .map(|(slot, reference)| (slot, reference.clone()))
                                .collect(),
                            kind: asset.kind().clone(),
                            canonical_path: if asset.canonical_path().as_os_str().is_empty() {
                                None
                            } else {
                                Some(asset.canonical_path().to_path_buf())
                            },
                        })
                });
            let script = entity_ref
                .get::<&LuaScriptComponent>()
                .map(|component| (*component).clone());
            let point_light = entity_ref.get::<&PointLight>().map(|component| *component);
            let directional_light = entity_ref
                .get::<&DirectionalLight>()
                .map(|component| *component);
            let spot_light = entity_ref.get::<&SpotLight>().map(|component| *component);
            let can_cast_shadow = entity_ref
                .get::<&CanCastShadow>()
                .map(|component| *component);
            let particle_system = entity_ref
                .get::<&ParticleSystemComponent>()
                .map(|component| (*component).clone());
            let particle_emitter = entity_ref
                .get::<&ParticleEmitterComponent>()
                .map(|component| (*component).clone());
            let environment = entity_ref
                .get::<&EnvironmentComponent>()
                .map(|component| (*component).clone());
            let post_process_volume = entity_ref
                .get::<&PostProcessVolume>()
                .map(|component| (*component).clone());
            let light_probe_volume = entity_ref
                .get::<&LightProbeVolume>()
                .map(|component| (*component).clone());
            let billboard = entity_ref.get::<&Billboard>().map(|component| *component);
            let rigid_body = entity_ref
                .get::<&crate::physics::RigidBody>()
                .map(|component| (*component).clone());
            let collider = entity_ref
                .get::<&crate::physics::Collider>()
                .map(|component| (*component).clone());
            let character_controller = entity_ref
                .get::<&crate::physics::CharacterController>()
                .map(|component| (*component).clone());
            let spring_camera = entity_ref
                .get::<&crate::physics::SpringCamera>()
                .map(|component| (*component).clone());
            let animator = entity_ref
                .get::<&crate::scene::skeletal::Animator>()
                .map(|component| (*component).clone());
            let animation_tree = entity_ref
                .get::<&crate::scene::skeletal::AnimationTree>()
                .map(|component| (*component).clone());

            nodes.insert(
                entity,
                SceneHierarchyNode {
                    entity,
                    name,
                    parent,
                    children,
                },
            );

            components.insert(
                entity,
                SceneEntityComponentsSummary {
                    camera,
                    transform,
                    mesh,
                    material,
                    script,
                    point_light,
                    directional_light,
                    spot_light,
                    can_cast_shadow,
                    particle_system,
                    particle_emitter,
                    environment,
                    post_process_volume,
                    light_probe_volume,
                    billboard,
                    rigid_body,
                    collider,
                    character_controller,
                    spring_camera,
                    animator,
                    animation_tree,
                    #[cfg(feature = "navigation")]
                    nav_mesh_surface: entity_ref
                        .get::<&crate::navigation::NavMeshSurface>()
                        .map(|component| (*component).clone()),
                    #[cfg(feature = "navigation")]
                    nav_agent: entity_ref
                        .get::<&crate::navigation::NavAgent>()
                        .map(|component| (*component).clone()),
                    #[cfg(feature = "navigation")]
                    nav_obstacle: entity_ref
                        .get::<&crate::navigation::NavObstacle>()
                        .map(|component| (*component).clone()),
                    #[cfg(feature = "navigation")]
                    nav_modifier: entity_ref
                        .get::<&crate::navigation::NavModifier>()
                        .map(|component| (*component).clone()),
                    #[cfg(feature = "navigation")]
                    nav_link: entity_ref
                        .get::<&crate::navigation::NavLink>()
                        .map(|component| (*component).clone()),
                    ai_agent: entity_ref
                        .get::<&crate::ai::AiAgent>()
                        .map(|component| (*component).clone()),
                },
            );
        }

        let known_entities: BTreeSet<_> = nodes.keys().copied().collect();

        for node in nodes.values_mut() {
            node.children.retain(|child| known_entities.contains(child));
        }

        let mut roots: Vec<_> = nodes
            .iter()
            .filter_map(|(&entity, node)| {
                let is_root = node
                    .parent
                    .map(|parent| !known_entities.contains(&parent))
                    .unwrap_or(true);
                is_root.then_some(entity)
            })
            .collect();

        roots.sort_by(|a, b| {
            let node_a = nodes.get(a).map(|node| node.name.to_lowercase());
            let node_b = nodes.get(b).map(|node| node.name.to_lowercase());
            node_a.cmp(&node_b).then_with(|| a.cmp(b))
        });

        Self {
            roots,
            nodes,
            components,
            prefabs,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    fn node(&self, entity: Entity) -> Option<&SceneHierarchyNode> {
        self.nodes.get(&entity)
    }

    fn iter_nodes(&self) -> impl Iterator<Item = Entity> + '_ {
        self.nodes.keys().copied()
    }

    fn roots(&self) -> &[Entity] {
        &self.roots
    }

    fn entity_components(&self, entity: Entity) -> Option<&SceneEntityComponentsSummary> {
        self.components.get(&entity)
    }

    fn entity_prefab_metadata(&self, entity: Entity) -> Option<&ScenePrefabEntityMetadata> {
        self.prefabs.get(&entity)
    }
}

#[cfg(feature = "egui")]
#[derive(Default)]
pub struct SceneHierarchyState {
    /// Shared with the panel, which reads it every frame: a scene of thousands of
    /// entities (a crowd's bones) must not be copied each time.
    snapshot: Arc<SceneHierarchySnapshot>,
    revision: u64,
    workspace: SceneHierarchyWorkspaceInfo,
}

#[cfg(feature = "egui")]
impl SceneHierarchyState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle() -> SceneHierarchyHandle {
        Arc::new(Mutex::new(Self::default()))
    }

    pub fn refresh_from_scene(&mut self, scene: &Scene, workspace: SceneHierarchyWorkspaceInfo) {
        self.snapshot = Arc::new(SceneHierarchySnapshot::from_scene(scene));
        self.workspace = workspace;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn snapshot_with_revision(&self) -> (u64, Arc<SceneHierarchySnapshot>) {
        (self.revision, Arc::clone(&self.snapshot))
    }

    pub fn workspace_info(&self) -> SceneHierarchyWorkspaceInfo {
        self.workspace.clone()
    }

    pub fn set_workspace_info(&mut self, workspace: SceneHierarchyWorkspaceInfo) {
        self.workspace = workspace;
    }
}

#[cfg(feature = "egui")]
pub type SceneHierarchyHandle = Arc<Mutex<SceneHierarchyState>>;

#[cfg(feature = "egui")]
pub type SceneHierarchyRegistryHandle = Arc<Mutex<SceneHierarchyRegistry>>;

#[cfg(feature = "egui")]
#[derive(Default)]
pub struct SceneHierarchyRegistry {
    entries: HashMap<SceneHandle, SceneHierarchyRegistryEntry>,
}

#[cfg(feature = "egui")]
#[derive(Clone)]
struct SceneHierarchyRegistryEntry {
    handle: SceneHierarchyHandle,
    dirty: bool,
}

#[cfg(feature = "egui")]
impl SceneHierarchyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ensure_entry(&mut self, handle: SceneHandle) -> SceneHierarchyHandle {
        let entry = self
            .entries
            .entry(handle)
            .or_insert_with(|| SceneHierarchyRegistryEntry {
                handle: SceneHierarchyState::handle(),
                dirty: true,
            });
        entry.handle.clone()
    }

    pub fn get_handle(&self, handle: SceneHandle) -> Option<SceneHierarchyHandle> {
        self.entries.get(&handle).map(|entry| entry.handle.clone())
    }

    pub fn mark_dirty(&mut self, handle: SceneHandle) {
        if let Some(entry) = self.entries.get_mut(&handle) {
            entry.dirty = true;
        }
    }

    pub fn retain_handles<F>(&mut self, mut keep: F)
    where
        F: FnMut(SceneHandle) -> bool,
    {
        self.entries.retain(|handle, _| keep(*handle));
    }

    pub fn take_dirty_handles(&mut self) -> Vec<(SceneHandle, SceneHierarchyHandle)> {
        let mut dirty = Vec::new();
        for (handle, entry) in self.entries.iter_mut() {
            if entry.dirty {
                entry.dirty = false;
                dirty.push((*handle, entry.handle.clone()));
            }
        }
        dirty
    }
}

#[cfg(feature = "egui")]
#[derive(Clone, Debug, Default)]
struct SceneHierarchyViewState {
    selected: Option<Entity>,
    _expanded: BTreeSet<Entity>,
    last_revision: Option<u64>,
}

#[cfg(feature = "egui")]
/// One line of the hierarchy tree.
struct HierarchyRow {
    entity: Entity,
    depth: usize,
    has_children: bool,
}

pub struct SceneHierarchyWindow {
    handle: Option<SceneHierarchyHandle>,
    title: String,
    active_scene: Option<SceneHandle>,
    view_states: HashMap<SceneHandle, SceneHierarchyViewState>,
    pending_scroll_to_selection: bool,
    workspace_cache: SceneHierarchyWorkspaceInfo,
}

#[cfg(feature = "egui")]
impl SceneHierarchyWindow {
    pub fn new(initial: Option<(SceneHandle, SceneHierarchyHandle)>) -> Self {
        let mut window = Self {
            handle: None,
            title: "Scene Hierarchy".to_string(),
            active_scene: None,
            view_states: HashMap::new(),
            pending_scroll_to_selection: false,
            workspace_cache: SceneHierarchyWorkspaceInfo::default(),
        };

        if let Some((scene_handle, handle)) = initial {
            window.active_scene = Some(scene_handle);
            window.handle = Some(handle);
            window.view_states.entry(scene_handle).or_default();
        }

        window
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        workspace: &SceneHierarchyWorkspaceInfo,
        open: Option<&mut bool>,
    ) -> Vec<SceneHierarchyEvent> {
        if let Some(open_flag) = open {
            if !*open_flag {
                return Vec::new();
            }
        }

        let mut events = Vec::new();
        egui::Panel::left("scene_hierarchy_panel")
            .resizable(true)
            .frame(egui::Frame::side_top_panel(ui.style()))
            .show(ui, |ui| {
                events = self.panel_contents(ui, workspace, true);
            });

        events
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        workspace: &SceneHierarchyWorkspaceInfo,
    ) -> Vec<SceneHierarchyEvent> {
        self.panel_contents(ui, workspace, false)
    }

    pub fn selected_entity(&self) -> Option<Entity> {
        self.active_view_state().and_then(|state| state.selected)
    }

    pub fn selected_entity_data(&self) -> Option<SceneEntityInspectorData> {
        let entity = self.selected_entity()?;
        let (_, snapshot) = self.snapshot()?;
        let node = snapshot.node(entity)?;
        let components = snapshot
            .entity_components(entity)
            .cloned()
            .unwrap_or_default();

        Some(SceneEntityInspectorData {
            entity,
            name: node.name.clone(),
            components,
        })
    }

    pub fn set_selected_entity(&mut self, entity: Option<Entity>) {
        self.update_selected_entity(entity, true);
    }

    pub fn sync_selected_entity(&mut self, entity: Option<Entity>) {
        self.update_selected_entity(entity, false);
    }

    pub fn set_active_scene(&mut self, scene_handle: SceneHandle, handle: SceneHierarchyHandle) {
        let same_scene = self.active_scene == Some(scene_handle);
        let same_handle = self
            .handle
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, &handle));

        if same_scene && same_handle {
            return;
        }

        self.active_scene = Some(scene_handle);
        self.handle = Some(handle);
        let view_state = self.view_states.entry(scene_handle).or_default();
        self.pending_scroll_to_selection = view_state.selected.is_some();
    }

    pub fn clear_active_scene(&mut self) {
        self.active_scene = None;
        self.handle = None;
        self.pending_scroll_to_selection = false;
    }

    pub fn workspace_info(&mut self) -> SceneHierarchyWorkspaceInfo {
        if let Some(handle) = &self.handle {
            if let Ok(state) = handle.lock() {
                let info = state.workspace_info();
                self.workspace_cache = info.clone();
                return info;
            }
        }

        self.workspace_cache.clone()
    }

    pub fn set_available_scenes(&mut self, scenes: Vec<SceneHierarchySceneDescriptor>) {
        self.workspace_cache.set_available_scenes(scenes.clone());

        if let Some(handle) = &self.handle {
            if let Ok(mut state) = handle.lock() {
                let mut info = state.workspace_info();
                if info.available_scenes() != scenes.as_slice() {
                    info.set_available_scenes(scenes);
                    state.set_workspace_info(info);
                }
            }
        }
    }

    fn update_selected_entity(&mut self, entity: Option<Entity>, scroll_to_selection: bool) {
        let Some(view_state) = self.active_view_state_mut() else {
            return;
        };

        if view_state.selected == entity {
            if scroll_to_selection && entity.is_some() {
                self.pending_scroll_to_selection = true;
            } else if entity.is_none() {
                self.pending_scroll_to_selection = false;
            }
            return;
        }

        view_state.selected = entity;
        self.pending_scroll_to_selection = scroll_to_selection && entity.is_some();
    }

    fn active_view_state_mut(&mut self) -> Option<&mut SceneHierarchyViewState> {
        let handle = self.active_scene?;
        Some(self.view_states.entry(handle).or_default())
    }

    fn active_view_state(&self) -> Option<&SceneHierarchyViewState> {
        let handle = self.active_scene?;
        self.view_states.get(&handle)
    }

    /// The Add menu and the entity tree. `show_heading` adds the window title, for
    /// hosts that do not show it in a tab.
    fn panel_contents(
        &mut self,
        ui: &mut egui::Ui,
        workspace: &SceneHierarchyWorkspaceInfo,
        show_heading: bool,
    ) -> Vec<SceneHierarchyEvent> {
        let Some(active_scene) = self.active_scene else {
            ui.label("No active scene selected.");
            return Vec::new();
        };

        let Some((revision, snapshot)) = self.snapshot() else {
            ui.label("Scene hierarchy unavailable.");
            return Vec::new();
        };

        let view_state = self.view_states.entry(active_scene).or_default();

        if view_state.last_revision != Some(revision) {
            view_state.last_revision = Some(revision);
            if view_state
                .selected
                .is_some_and(|entity| snapshot.node(entity).is_none())
            {
                view_state.selected = None;
                self.pending_scroll_to_selection = false;
            }
        }

        let mut events = Vec::new();

        ui.horizontal(|ui| {
            if show_heading {
                ui.heading(&self.title);
                ui.add_space(8.0);
            }
            let mut close_top_menu = false;
            ui.menu_button("✚ Add", |ui| {
                if ui.button("Empty Entity").clicked() {
                    events.push(SceneHierarchyEvent::Create(SceneCreationAction::Empty));
                    close_top_menu = true;
                    ui.close();
                }

                ui.menu_button("Primitive", |ui| {
                    for preset in ScenePrimitivePreset::variants() {
                        if ui.button(preset.display_name()).clicked() {
                            events.push(SceneHierarchyEvent::Create(
                                SceneCreationAction::Primitive(preset),
                            ));
                            close_top_menu = true;
                            ui.close();
                        }
                    }
                });

                if workspace.available_scenes().is_empty() {
                    ui.add_enabled(false, egui::Button::new("Instance Scene"));
                    ui.label("Open a project with scenes to enable instancing.");
                } else {
                    ui.menu_button("Instance Scene", |ui| {
                        for scene in workspace.available_scenes() {
                            let label = scene.display_name.as_str();
                            if ui.button(label).clicked() {
                                events.push(SceneHierarchyEvent::Create(
                                    SceneCreationAction::SceneInstance {
                                        document_id: scene.document_id.clone(),
                                        node_path: Vec::new(),
                                    },
                                ));
                                close_top_menu = true;
                                ui.close();
                            }
                        }
                    });
                }

                ui.menu_button("Light", |ui| {
                    if ui.button("Directional Light").clicked() {
                        events.push(SceneHierarchyEvent::Create(
                            SceneCreationAction::DirectionalLight,
                        ));
                        close_top_menu = true;
                        ui.close();
                    }
                    if ui.button("Point Light").clicked() {
                        events.push(SceneHierarchyEvent::Create(SceneCreationAction::PointLight));
                        close_top_menu = true;
                        ui.close();
                    }
                    if ui.button("Spot Light").clicked() {
                        events.push(SceneHierarchyEvent::Create(SceneCreationAction::SpotLight));
                        close_top_menu = true;
                        ui.close();
                    }
                });

                ui.menu_button("Particle System", |ui| {
                    for preset in ParticleBehaviorPreset::variants() {
                        if ui.button(preset.display_name()).clicked() {
                            events.push(SceneHierarchyEvent::Create(
                                SceneCreationAction::ParticleSystem(preset),
                            ));
                            close_top_menu = true;
                            ui.close();
                        }
                    }
                });

                if ui.button("Camera").clicked() {
                    events.push(SceneHierarchyEvent::Create(SceneCreationAction::Camera));
                    close_top_menu = true;
                    ui.close();
                }
                if ui.button("Environment").clicked() {
                    events.push(SceneHierarchyEvent::Create(
                        SceneCreationAction::Environment,
                    ));
                    close_top_menu = true;
                    ui.close();
                }
                if ui.button("Post Process Volume").clicked() {
                    events.push(SceneHierarchyEvent::Create(
                        SceneCreationAction::PostProcessVolume,
                    ));
                    close_top_menu = true;
                    ui.close();
                }
                if ui.button("Light Probe Volume").clicked() {
                    events.push(SceneHierarchyEvent::Create(
                        SceneCreationAction::LightProbeVolume,
                    ));
                    close_top_menu = true;
                    ui.close();
                }
            });

            if close_top_menu {
                ui.close();
            }
        });
        ui.add_space(2.0);
        ui.separator();

        if snapshot.is_empty() {
            ui.add_space(16.0);
            ui.vertical_centered(|ui| {
                ui.weak("The scene is empty");
                ui.label(
                    egui::RichText::new("Use ✚ Add to create an entity.")
                        .small()
                        .weak(),
                );
            });
            return events;
        }

        // Only the rows in view are laid out, so a game that spawns thousands of entities
        // during play does not slow the editor down.
        let rows = self.visible_rows(ui.ctx(), &snapshot);
        let row_height = ui.spacing().interact_size.y;
        let mut area = egui::ScrollArea::vertical().auto_shrink([false; 2]);
        if self.pending_scroll_to_selection {
            if let Some(index) = rows.iter().position(|row| self.is_selected(row.entity)) {
                let pitch = row_height + ui.spacing().item_spacing.y;
                let offset = index as f32 * pitch - ui.available_height() * 0.5;
                area = area.vertical_scroll_offset(offset.max(0.0));
                self.pending_scroll_to_selection = false;
            }
        }
        area.show_rows(ui, row_height, rows.len(), |ui, range| {
            for row in &rows[range] {
                self.draw_row(ui, row, &snapshot, workspace, &mut events);
            }
        });
        events
    }

    /// The rows the tree shows, in order: every entity whose ancestors are all expanded.
    fn visible_rows(
        &self,
        ctx: &egui::Context,
        snapshot: &SceneHierarchySnapshot,
    ) -> Vec<HierarchyRow> {
        let mut rows = Vec::new();
        let mut visited = BTreeSet::new();
        for &root in snapshot.roots() {
            self.collect_rows(ctx, root, 0, snapshot, &mut visited, &mut rows);
        }

        for entity in snapshot.iter_nodes() {
            if visited.contains(&entity) {
                continue;
            }

            let Some(node) = snapshot.node(entity) else {
                continue;
            };

            if self.is_hidden_by_collapsed_parent(node, snapshot, ctx) {
                continue;
            }

            self.collect_rows(ctx, entity, 0, snapshot, &mut visited, &mut rows);
        }
        rows
    }

    fn collect_rows(
        &self,
        ctx: &egui::Context,
        entity: Entity,
        depth: usize,
        snapshot: &SceneHierarchySnapshot,
        visited: &mut BTreeSet<Entity>,
        rows: &mut Vec<HierarchyRow>,
    ) {
        if !visited.insert(entity) {
            return;
        }
        let Some(node) = snapshot.node(entity) else {
            return;
        };
        let has_children = !node.children.is_empty();
        rows.push(HierarchyRow {
            entity,
            depth,
            has_children,
        });
        if has_children && self.collapse_state(ctx, entity).is_open() {
            for &child in &node.children {
                self.collect_rows(ctx, child, depth + 1, snapshot, visited, rows);
            }
        }
    }

    fn collapse_state(&self, ctx: &egui::Context, entity: Entity) -> CollapsingState {
        let id = egui::Id::new(("scene_hierarchy", self.active_scene, entity));
        CollapsingState::load_with_default_open(ctx, id, true)
    }

    fn snapshot(&self) -> Option<(u64, Arc<SceneHierarchySnapshot>)> {
        let handle = self.handle.as_ref()?;
        let Ok(state) = handle.lock() else {
            return None;
        };
        Some(state.snapshot_with_revision())
    }

    fn draw_row(
        &mut self,
        ui: &mut egui::Ui,
        row: &HierarchyRow,
        snapshot: &SceneHierarchySnapshot,
        workspace: &SceneHierarchyWorkspaceInfo,
        events: &mut Vec<SceneHierarchyEvent>,
    ) {
        let entity = row.entity;
        let Some(node) = snapshot.node(entity) else {
            return;
        };

        let (icon, icon_color) = entity_icon(snapshot.entity_components(entity));
        let mut label = egui::text::LayoutJob::default();
        let font = egui::TextStyle::Body.resolve(ui.style());
        label.append(
            icon,
            0.0,
            egui::TextFormat::simple(font.clone(), icon_color),
        );
        label.append(
            &node.name,
            6.0,
            egui::TextFormat::simple(font, ui.visuals().text_color()),
        );
        let prefab_info = snapshot.entity_prefab_metadata(entity).cloned();
        let selected = self.is_selected(entity);

        let mut clicked = false;
        ui.horizontal(|ui| {
            ui.add_space(row.depth as f32 * ui.spacing().indent);
            if row.has_children {
                let mut state = self.collapse_state(ui.ctx(), entity);
                state.show_toggle_button(ui, egui::collapsing_header::paint_default_icon);
                state.store(ui.ctx());
            } else {
                // Line leaf rows up with the labels of rows that have a collapse arrow.
                ui.add_space(ui.spacing().indent);
            }
            let response = ui
                .selectable_label(selected, label.clone())
                .on_hover_text(format!("Entity {:?}", node.entity));

            response.context_menu(|ui| {
                if ui.button("Add Script").clicked() {
                    events.push(SceneHierarchyEvent::AddScriptToEntity(entity));
                    ui.close();
                }
                ui.separator();
                if ui.button("Delete").clicked() {
                    events.push(SceneHierarchyEvent::DeleteEntity(entity));
                    ui.close();
                }
            });

            if let Some(info) = &prefab_info {
                self.decorate_prefab_response(&response, info, workspace, events);
                if workspace.is_foreign_document(&info.document_id) {
                    ui.add_space(4.0);
                    self.add_prefab_badge(ui, info);
                }
            }
            if response.clicked() {
                clicked = true;
            }
        });

        if clicked {
            self.update_selected_entity(Some(entity), false);
        }
    }

    fn is_hidden_by_collapsed_parent(
        &self,
        node: &SceneHierarchyNode,
        snapshot: &SceneHierarchySnapshot,
        ctx: &egui::Context,
    ) -> bool {
        let mut current = node.parent;

        while let Some(parent_entity) = current {
            if !self.collapse_state(ctx, parent_entity).is_open() {
                return true;
            }

            current = snapshot
                .node(parent_entity)
                .and_then(|parent_node| parent_node.parent);
        }

        false
    }

    fn is_selected(&self, entity: Entity) -> bool {
        self.selected_entity() == Some(entity)
    }

    fn add_prefab_badge(&self, ui: &mut egui::Ui, info: &ScenePrefabEntityMetadata) {
        let badge = egui::Label::new(
            egui::RichText::new("Prefab")
                .small()
                .color(egui::Color32::from_rgb(120, 180, 255)),
        );
        let response = ui.add(badge);
        response.on_hover_text(Self::prefab_tooltip(info));
    }

    fn decorate_prefab_response(
        &self,
        response: &egui::Response,
        info: &ScenePrefabEntityMetadata,
        workspace: &SceneHierarchyWorkspaceInfo,
        events: &mut Vec<SceneHierarchyEvent>,
    ) {
        if !workspace.is_foreign_document(&info.document_id) {
            return;
        }

        response.clone().on_hover_text(Self::prefab_tooltip(info));

        if workspace.is_open(&info.document_id) {
            response.context_menu(|ui| {
                if ui.button("Open source scene").clicked() {
                    events.push(SceneHierarchyEvent::OpenScene(info.document_id.clone()));
                    ui.close();
                }
            });
        }
    }

    fn prefab_tooltip(info: &ScenePrefabEntityMetadata) -> String {
        let path = if info.node_path.is_empty() {
            String::from("/")
        } else {
            info.node_path.join(" / ")
        };

        format!("Prefab from {}\n{}", info.document_id, path)
    }
}

/// Icon and color for an entity in the tree, from its most telling component.
#[cfg(feature = "egui")]
fn entity_icon(components: Option<&SceneEntityComponentsSummary>) -> (&'static str, egui::Color32) {
    use egui::Color32;
    let Some(c) = components else {
        return ("○", Color32::from_gray(140));
    };
    let light = Color32::from_rgb(0xf5, 0xd0, 0x5c);
    if c.camera.is_some() {
        ("🎥", Color32::from_rgb(0x7f, 0xb8, 0xe6))
    } else if c.directional_light.is_some() {
        ("☀", light)
    } else if c.spot_light.is_some() {
        ("🔦", light)
    } else if c.point_light.is_some() {
        ("💡", light)
    } else if c.particle_system.is_some() {
        ("✨", Color32::from_rgb(0xe8, 0x78, 0xc8))
    } else if c.environment.is_some() {
        ("🌐", Color32::from_rgb(0x5c, 0xc9, 0x8a))
    } else if c.post_process_volume.is_some() {
        ("🎞", Color32::from_rgb(0xb4, 0x8c, 0xff))
    } else if c.light_probe_volume.is_some() {
        ("🔆", Color32::from_rgb(0xf0, 0xa8, 0x5a))
    } else if c.mesh.is_some() {
        ("▣", Color32::from_rgb(0x4f, 0xc7, 0xd1))
    } else if c.script.is_some() {
        ("📜", Color32::from_rgb(0x6c, 0xa8, 0xff))
    } else {
        ("○", Color32::from_gray(140))
    }
}
