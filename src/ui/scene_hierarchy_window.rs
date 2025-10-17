#[cfg(feature = "egui")]
use crate::scene::{Children, Name, Parent, Scene};
#[cfg(feature = "egui")]
use crate::ui::egui;
#[cfg(feature = "egui")]
use egui::collapsing_header::CollapsingState;
#[cfg(feature = "egui")]
use hecs::Entity;
#[cfg(feature = "egui")]
use std::collections::{BTreeMap, BTreeSet};
#[cfg(feature = "egui")]
use std::sync::{Arc, Mutex};

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
}

#[cfg(feature = "egui")]
impl SceneHierarchySnapshot {
    pub fn from_scene(scene: &Scene) -> Self {
        let world = scene.world();
        let mut nodes = BTreeMap::new();

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

            nodes.insert(
                entity,
                SceneHierarchyNode {
                    entity,
                    name,
                    parent,
                    children,
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

        Self { roots, nodes }
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
}

#[cfg(feature = "egui")]
#[derive(Default)]
pub struct SceneHierarchyState {
    snapshot: SceneHierarchySnapshot,
    revision: u64,
}

#[cfg(feature = "egui")]
impl SceneHierarchyState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn handle() -> SceneHierarchyHandle {
        Arc::new(Mutex::new(Self::default()))
    }

    pub fn refresh_from_scene(&mut self, scene: &Scene) {
        self.snapshot = SceneHierarchySnapshot::from_scene(scene);
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn snapshot_with_revision(&self) -> (u64, SceneHierarchySnapshot) {
        (self.revision, self.snapshot.clone())
    }
}

#[cfg(feature = "egui")]
pub type SceneHierarchyHandle = Arc<Mutex<SceneHierarchyState>>;

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

    pub fn show(&mut self, ctx: &egui::Context, open: Option<&mut bool>) {
        if let Some(open_flag) = open {
            if !*open_flag {
                return;
            }
        }

        egui::SidePanel::left("scene_hierarchy_panel")
            .resizable(true)
            .frame(egui::Frame::side_top_panel(&ctx.style()))
            .show(ctx, |ui| {
                self.panel_contents(ui);
            });
    }

    pub fn ui(&mut self, ui: &mut egui::Ui) {
        self.panel_contents(ui);
    }

    fn panel_contents(&mut self, ui: &mut egui::Ui) {
        let Some((revision, snapshot)) = self.snapshot() else {
            ui.label("Scene hierarchy unavailable.");
            return;
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

        if snapshot.is_empty() {
            ui.label("Scene is empty.");
            return;
        }

        ui.heading(&self.title);
        ui.separator();

        let mut visited = BTreeSet::new();
        for &root in snapshot.roots() {
            self.draw_entity(ui, root, &snapshot, &mut visited);
        }

        for entity in snapshot.iter_nodes() {
            if !visited.contains(&entity) {
                self.draw_entity(ui, entity, &snapshot, &mut visited);
            }
        }
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
    ) {
        if !visited.insert(entity) {
            return;
        }

        let Some(node) = snapshot.node(entity) else {
            return;
        };

        let label = format!("{} ({:?})", node.name, node.entity);

        if node.children.is_empty() {
            let selected = self.selected == Some(entity);
            if ui.selectable_label(selected, label).clicked() {
                self.selected = Some(entity);
            }
            return;
        }

        let id = ui.make_persistent_id(("scene_hierarchy", entity));
        let state = CollapsingState::load_with_default_open(ui.ctx(), id, true);
        let selected = self.selected == Some(entity);

        let header_response = state.show_header(ui, |ui| {
            let response = ui.selectable_label(selected, &label);
            if response.clicked() {
                self.selected = Some(entity);
            }
            response
        });

        let _ = header_response.body(|ui| {
            for &child in &node.children {
                self.draw_entity(ui, child, snapshot, visited);
            }
        });
    }
}
