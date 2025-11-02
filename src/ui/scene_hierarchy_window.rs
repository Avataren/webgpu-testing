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
    MaterialComponent, MeshComponent, Name, Parent, ParticleBehaviorPreset,
    ParticleEmitterComponent, ParticleSystemComponent, PointLight, Scene, SceneHandle,
    ScenePrefabEntityMetadata, SceneWorkspace, SpotLight, Transform, TransformComponent,
};
#[cfg(feature = "egui")]
use crate::scripting::RuneScriptComponent;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SceneCreationAction {
    Primitive(ScenePrimitivePreset),
    ParticleSystem(ParticleBehaviorPreset),
    PointLight,
    DirectionalLight,
    SpotLight,
    Camera,
    Environment,
}

#[cfg(feature = "egui")]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SceneHierarchyEvent {
    Create(SceneCreationAction),
    OpenScene(SceneDocumentId),
}

#[cfg(feature = "egui")]
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SceneHierarchyWorkspaceInfo {
    active_document: Option<SceneDocumentId>,
    open_documents: BTreeSet<SceneDocumentId>,
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
    pub script: Option<RuneScriptComponent>,
    pub point_light: Option<PointLight>,
    pub directional_light: Option<DirectionalLight>,
    pub spot_light: Option<SpotLight>,
    pub can_cast_shadow: Option<CanCastShadow>,
    pub particle_system: Option<ParticleSystemComponent>,
    pub particle_emitter: Option<ParticleEmitterComponent>,
    pub environment: Option<EnvironmentComponent>,
    pub billboard: Option<Billboard>,
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
                .get::<&RuneScriptComponent>()
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
            let billboard = entity_ref.get::<&Billboard>().map(|component| *component);

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
                    billboard,
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
    snapshot: SceneHierarchySnapshot,
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
        self.snapshot = SceneHierarchySnapshot::from_scene(scene);
        self.workspace = workspace;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn snapshot_with_revision(&self) -> (u64, SceneHierarchySnapshot) {
        (self.revision, self.snapshot.clone())
    }

    pub fn workspace_info(&self) -> SceneHierarchyWorkspaceInfo {
        self.workspace.clone()
    }
}

#[cfg(feature = "egui")]
pub type SceneHierarchyHandle = Arc<Mutex<SceneHierarchyState>>;

#[cfg(feature = "egui")]
pub type SceneHierarchyRegistry = HashMap<SceneHandle, SceneHierarchyHandle>;

#[cfg(feature = "egui")]
pub type SceneHierarchyRegistryHandle = Arc<Mutex<SceneHierarchyRegistry>>;

#[cfg(feature = "egui")]
pub struct SceneHierarchyWindow {
    handle: SceneHierarchyHandle,
    title: String,
    selected: Option<Entity>,
    last_revision: Option<u64>,
}

#[cfg(feature = "egui")]
impl SceneHierarchyWindow {
    pub fn new(handle: SceneHierarchyHandle) -> Self {
        Self {
            handle,
            title: "Scene Hierarchy".to_string(),
            selected: None,
            last_revision: None,
        }
    }

    pub fn show(
        &mut self,
        ctx: &egui::Context,
        workspace: &SceneHierarchyWorkspaceInfo,
        open: Option<&mut bool>,
    ) -> Vec<SceneHierarchyEvent> {
        if let Some(open_flag) = open {
            if !*open_flag {
                return Vec::new();
            }
        }

        let mut events = Vec::new();
        egui::SidePanel::left("scene_hierarchy_panel")
            .resizable(true)
            .frame(egui::Frame::side_top_panel(&ctx.style()))
            .show(ctx, |ui| {
                events = self.panel_contents(ui, workspace);
            });

        events
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        workspace: &SceneHierarchyWorkspaceInfo,
    ) -> Vec<SceneHierarchyEvent> {
        self.panel_contents(ui, workspace)
    }

    pub fn selected_entity(&self) -> Option<Entity> {
        self.selected
    }

    pub fn selected_entity_data(&self) -> Option<SceneEntityInspectorData> {
        let entity = self.selected?;
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
        self.selected = entity;
    }

    pub fn handle(&self) -> SceneHierarchyHandle {
        self.handle.clone()
    }

    pub fn workspace_info(&self) -> SceneHierarchyWorkspaceInfo {
        match self.handle.lock() {
            Ok(state) => state.workspace_info(),
            Err(_) => SceneHierarchyWorkspaceInfo::default(),
        }
    }

    pub fn set_handle(&mut self, handle: SceneHierarchyHandle) {
        if Arc::ptr_eq(&self.handle, &handle) {
            return;
        }
        self.handle = handle;
        self.last_revision = None;
        if self.snapshot().is_none() {
            self.selected = None;
        }
    }

    fn panel_contents(
        &mut self,
        ui: &mut egui::Ui,
        workspace: &SceneHierarchyWorkspaceInfo,
    ) -> Vec<SceneHierarchyEvent> {
        let Some((revision, snapshot)) = self.snapshot() else {
            ui.label("Scene hierarchy unavailable.");
            return Vec::new();
        };

        if self.last_revision != Some(revision) {
            self.last_revision = Some(revision);
            if self
                .selected
                .is_some_and(|entity| snapshot.node(entity).is_none())
            {
                self.selected = None;
            }
        }

        let mut events = Vec::new();

        ui.horizontal(|ui| {
            ui.heading(&self.title);
            ui.add_space(8.0);
            let mut close_top_menu = false;
            ui.menu_button("Add", |ui| {
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
            });

            if close_top_menu {
                ui.close();
            }
        });
        ui.separator();

        if snapshot.is_empty() {
            ui.label("Scene is empty.");
            return events;
        }

        let mut visited = BTreeSet::new();
        for &root in snapshot.roots() {
            self.draw_entity(ui, root, &snapshot, &mut visited, workspace, &mut events);
        }

        for entity in snapshot.iter_nodes() {
            if visited.contains(&entity) {
                continue;
            }

            let Some(node) = snapshot.node(entity) else {
                continue;
            };

            if self.is_hidden_by_collapsed_parent(node, &snapshot, ui) {
                continue;
            }

            self.draw_entity(ui, entity, &snapshot, &mut visited, workspace, &mut events);
        }
        events
    }

    fn snapshot(&self) -> Option<(u64, SceneHierarchySnapshot)> {
        let Ok(state) = self.handle.lock() else {
            return None;
        };
        Some(state.snapshot_with_revision())
    }

    fn draw_entity(
        &mut self,
        ui: &mut egui::Ui,
        entity: Entity,
        snapshot: &SceneHierarchySnapshot,
        visited: &mut BTreeSet<Entity>,
        workspace: &SceneHierarchyWorkspaceInfo,
        events: &mut Vec<SceneHierarchyEvent>,
    ) {
        if !visited.insert(entity) {
            return;
        }

        let Some(node) = snapshot.node(entity) else {
            return;
        };

        let label = format!("{} ({:?})", node.name, node.entity);
        let prefab_info = snapshot.entity_prefab_metadata(entity).cloned();

        if node.children.is_empty() {
            let selected = self.selected == Some(entity);
            let mut clicked = false;
            ui.horizontal(|ui| {
                let response = ui.selectable_label(selected, &label);
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
                self.selected = Some(entity);
            }
            return;
        }

        let id = egui::Id::new(("scene_hierarchy", entity));
        let state = CollapsingState::load_with_default_open(ui.ctx(), id, true);
        let selected = self.selected == Some(entity);

        let mut clicked = false;
        let header_response = state.show_header(ui, |ui| {
            ui.horizontal(|ui| {
                let response = ui.selectable_label(selected, &label);
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
                response
            })
            .inner
        });

        if clicked {
            self.selected = Some(entity);
        }

        let _ = header_response.body(|ui| {
            for &child in &node.children {
                self.draw_entity(ui, child, snapshot, visited, workspace, events);
            }
        });
    }

    fn is_hidden_by_collapsed_parent(
        &self,
        node: &SceneHierarchyNode,
        snapshot: &SceneHierarchySnapshot,
        ui: &egui::Ui,
    ) -> bool {
        let mut current = node.parent;

        while let Some(parent_entity) = current {
            let id = egui::Id::new(("scene_hierarchy", parent_entity));
            let state = CollapsingState::load_with_default_open(ui.ctx(), id, true);
            if !state.is_open() {
                return true;
            }

            current = snapshot
                .node(parent_entity)
                .and_then(|parent_node| parent_node.parent);
        }

        false
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
