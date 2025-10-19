use super::animation::{AnimationClip, AnimationState, AnimationTarget};
use super::assets::{
    build_tree_asset_node, serialize_world, SceneAsset, SceneTreeAsset, SceneTreeAssetNode,
    SerializedAnimationClip, SerializedTransform,
};
use super::graph::{SceneInstance, SceneNode, SceneNodeId};
use super::internal::{gizmos, lights, rendering, transform_gizmos};
use super::loader::SceneLoader;
use crate::asset::Assets;
use crate::environment::Environment;
use crate::renderer::{CustomRenderRequest, RenderBatcher, Renderer};
use crate::scene::components::{SelectedInEditor, TransformComponent, WorldTransform};
use crate::scene::transform::Transform;
use crate::scene::Camera;
use crate::scripting::{PendingGltfImport, RuneScriptComponent, RuneScriptSource, ScriptingState};
use crate::time::Instant;
use glam::Vec3;
use hecs::World;
use log::{error, warn};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformGizmoMode {
    Translate,
    Rotate,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransformGizmoSpace {
    Local,
    World,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransformGizmoAxis {
    X,
    Y,
    Z,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TransformGizmoHandle {
    TranslateAxis(TransformGizmoAxis),
    TranslateCenter,
    TranslatePlane(TransformGizmoAxis, TransformGizmoAxis),
    RotateAxis(TransformGizmoAxis),
    RotateScreen,
    ScaleAxis(TransformGizmoAxis),
    ScaleUniform,
}

#[derive(Clone)]
pub(crate) struct SceneSnapshot {
    assets: Assets,
    environment: Environment,
    camera: Camera,
    time: f64,
    tree: SceneTreeAsset,
    main_scene_path: Vec<usize>,
    gizmo_resources: Option<gizmos::GizmoResources>,
    transform_gizmo_resources: Option<transform_gizmos::TransformGizmoResources>,
    gizmo_mode: TransformGizmoMode,
    gizmo_space: TransformGizmoSpace,
    gizmo_hover: Option<TransformGizmoHandle>,
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
        let gizmo_mode = scene.transform_gizmo_mode;
        let gizmo_space = scene.transform_gizmo_space;
        let gizmo_hover = scene.transform_gizmo_hover;

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
                let children = scene.node(current).children();
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

        scene.gizmo_resources = self.gizmo_resources;
        scene.transform_gizmo_resources = self.transform_gizmo_resources;
        scene.transform_gizmo_mode = self.gizmo_mode;
        scene.transform_gizmo_space = self.gizmo_space;
        scene.transform_gizmo_hover = self.gizmo_hover;

        scene
    }
}
pub struct Scene {
    pub assets: Assets,
    environment: Environment,
    camera: Camera,
    time: f64,
    last_frame: Option<Instant>,
    nodes: Vec<Option<SceneNode>>,
    free_list: Vec<SceneNodeId>,
    root: SceneNodeId,
    main_scene: SceneNodeId,
    scripting: ScriptingState,
    gizmo_resources: Option<gizmos::GizmoResources>,
    transform_gizmo_resources: Option<transform_gizmos::TransformGizmoResources>,
    transform_gizmo_mode: TransformGizmoMode,
    transform_gizmo_space: TransformGizmoSpace,
    transform_gizmo_hover: Option<TransformGizmoHandle>,
}

impl Scene {
    pub fn new() -> Self {
        let mut nodes = Vec::new();
        let root_id = SceneNodeId::new(0);
        let root_node = SceneNode::new(root_id, "Root", SceneInstance::new());
        nodes.push(Some(root_node));

        Self {
            assets: Assets::default(),
            environment: Environment::default(),
            camera: Camera::default(),
            time: 0.0,
            last_frame: None,
            nodes,
            free_list: Vec::new(),
            root: root_id,
            main_scene: root_id,
            scripting: ScriptingState::default(),
            gizmo_resources: None,
            transform_gizmo_resources: None,
            transform_gizmo_mode: TransformGizmoMode::Translate,
            transform_gizmo_space: TransformGizmoSpace::Local,
            transform_gizmo_hover: None,
        }
    }

    fn node(&self, id: SceneNodeId) -> &SceneNode {
        self.nodes[id.index()].as_ref().expect("Invalid scene node")
    }

    fn node_mut(&mut self, id: SceneNodeId) -> &mut SceneNode {
        self.nodes[id.index()].as_mut().expect("Invalid scene node")
    }

    fn is_valid_node(&self, id: SceneNodeId) -> bool {
        self.nodes
            .get(id.index())
            .and_then(|slot| slot.as_ref())
            .is_some()
    }

    fn nodes_iter(&self) -> impl Iterator<Item = &SceneNode> {
        self.nodes.iter().filter_map(|n| n.as_ref())
    }

    fn nodes_iter_mut(&mut self) -> impl Iterator<Item = &mut SceneNode> {
        self.nodes.iter_mut().filter_map(|n| n.as_mut())
    }

    fn attach_node(&mut self, child: SceneNodeId, parent: SceneNodeId) {
        {
            let child_node = self.node_mut(child);
            child_node.set_parent(Some(parent));
        }
        self.node_mut(parent).add_child(child);
    }

    fn detach_node(&mut self, node: SceneNodeId) -> Option<SceneNodeId> {
        let parent = { self.node(node).parent() };
        if let Some(parent_id) = parent {
            {
                let parent_node = self.node_mut(parent_id);
                parent_node.remove_child(node);
            }
            self.node_mut(node).set_parent(None);
        }
        parent
    }

    pub fn root_id(&self) -> SceneNodeId {
        self.root
    }

    pub fn main_scene(&self) -> SceneNodeId {
        self.main_scene
    }

    pub fn set_main_scene(&mut self, node: SceneNodeId) {
        if self.is_valid_node(node) {
            self.main_scene = node;
        }
    }

    pub fn scripting(&self) -> &ScriptingState {
        &self.scripting
    }

    pub fn scripting_mut(&mut self) -> &mut ScriptingState {
        &mut self.scripting
    }

    pub fn transform_gizmo_mode(&self) -> TransformGizmoMode {
        self.transform_gizmo_mode
    }

    pub fn set_transform_gizmo_mode(&mut self, mode: TransformGizmoMode) {
        self.transform_gizmo_mode = mode;
    }

    pub fn transform_gizmo_space(&self) -> TransformGizmoSpace {
        self.transform_gizmo_space
    }

    pub fn set_transform_gizmo_space(&mut self, space: TransformGizmoSpace) {
        self.transform_gizmo_space = space;
    }

    pub fn transform_gizmo_hover(&self) -> Option<TransformGizmoHandle> {
        self.transform_gizmo_hover
    }

    pub fn set_transform_gizmo_hover(&mut self, hover: Option<TransformGizmoHandle>) {
        self.transform_gizmo_hover = hover;
    }

    pub fn selected_gizmo_transform(&self) -> Option<Transform> {
        for node in self.nodes_iter() {
            let node_world = *node.world_transform();
            let world = node.instance().world();

            if let Some((_, (world_transform, _))) = world
                .query::<(&WorldTransform, &SelectedInEditor)>()
                .iter()
                .next()
            {
                return Some(node_world.mul_transform(&world_transform.0));
            }

            if let Some((_, (local_transform, _))) = world
                .query::<(&TransformComponent, &SelectedInEditor)>()
                .iter()
                .next()
            {
                return Some(node_world.mul_transform(&local_transform.0));
            }
        }

        None
    }

    pub fn transform_gizmo_hit(
        &self,
        ray_origin: Vec3,
        ray_dir: Vec3,
    ) -> Option<TransformGizmoHandle> {
        let transform = self.selected_gizmo_transform()?;
        let camera = self.camera();
        let camera_vectors = rendering::CameraVectors {
            position: camera.eye,
            target: camera.target,
            up: camera.up,
            fov_y: camera.fov_y_radians,
        };

        transform_gizmos::hit_test(
            camera_vectors,
            transform,
            self.transform_gizmo_mode,
            self.transform_gizmo_space,
            ray_origin,
            ray_dir,
        )
    }

    pub fn node_name(&self, node: SceneNodeId) -> &str {
        self.node(node).name()
    }

    pub fn set_node_name(&mut self, node: SceneNodeId, name: impl Into<String>) {
        self.node_mut(node).set_name(name);
    }

    pub fn node_parent(&self, node: SceneNodeId) -> Option<SceneNodeId> {
        self.node(node).parent()
    }

    pub fn node_children(&self, node: SceneNodeId) -> &[SceneNodeId] {
        self.node(node).children()
    }

    pub fn iter_children(&self, node: SceneNodeId) -> impl Iterator<Item = SceneNodeId> + '_ {
        self.node(node).children().iter().copied()
    }

    pub fn node_local_transform(&self, node: SceneNodeId) -> &Transform {
        self.node(node).local_transform()
    }

    pub fn node_local_transform_mut(&mut self, node: SceneNodeId) -> &mut Transform {
        self.node_mut(node).local_transform_mut()
    }

    pub fn node_world_transform(&self, node: SceneNodeId) -> &Transform {
        self.node(node).world_transform()
    }

    pub fn init_timer(&mut self) {
        self.last_frame = Some(Instant::now());
    }

    pub fn reset_script_runtime(&mut self) {
        {
            let world = self.main_world_mut();
            let mut query = world.query::<&mut RuneScriptComponent>();
            for (_, component) in query.iter() {
                let source = component.source();
                if matches!(
                    source,
                    RuneScriptSource::Inline { name, .. } if name.as_ref() == "editor_startup.rn"
                ) {
                    continue;
                }
                component.set_created_called(false);
            }
        }

        self.scripting_mut().reset_runtime();
    }

    pub fn set_animation_playback(&mut self, playing: bool) {
        for node in self.nodes_iter_mut() {
            let states = node.instance_mut().animation_states_mut();
            for state in states {
                state.playing = playing;
                if !playing {
                    state.time = 0.0;
                }
            }
        }
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub(crate) fn set_time(&mut self, time: f64) {
        self.time = time;
    }

    pub fn last_frame(&self) -> Instant {
        self.last_frame
            .expect("Scene timer not initialized - call init_timer() first")
    }

    pub fn last_frame_instant(&self) -> Option<Instant> {
        self.last_frame
    }

    pub fn set_last_frame(&mut self, instant: Instant) {
        self.last_frame = Some(instant);
    }

    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    pub fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    pub fn set_camera(&mut self, camera: Camera) {
        self.camera = camera;
    }

    pub fn environment(&self) -> &Environment {
        &self.environment
    }

    pub fn environment_mut(&mut self) -> &mut Environment {
        &mut self.environment
    }

    pub fn set_environment(&mut self, environment: Environment) {
        self.environment = environment;
    }

    pub(crate) fn replace_assets(&mut self, assets: Assets) {
        self.assets = assets;
        self.gizmo_resources = None;
    }

    pub fn node_animations(&self, node: SceneNodeId) -> &[AnimationClip] {
        self.node(node).instance().animations()
    }

    pub fn node_animation_states(&self, node: SceneNodeId) -> &[AnimationState] {
        self.node(node).instance().animation_states()
    }

    pub fn add_node_animation_clip(&mut self, node: SceneNodeId, clip: AnimationClip) -> usize {
        self.node_mut(node).instance_mut().add_animation_clip(clip)
    }

    pub fn play_node_animation(
        &mut self,
        node: SceneNodeId,
        clip_index: usize,
        looping: bool,
    ) -> Option<usize> {
        self.node_mut(node)
            .instance_mut()
            .play_animation(clip_index, looping)
    }

    fn main_instance(&self) -> &SceneInstance {
        self.node(self.main_scene).instance()
    }

    fn main_instance_mut(&mut self) -> &mut SceneInstance {
        self.node_mut(self.main_scene).instance_mut()
    }

    pub fn main_world(&self) -> &World {
        self.main_instance().world()
    }

    pub fn main_world_mut(&mut self) -> &mut World {
        self.main_instance_mut().world_mut()
    }

    pub(crate) fn gizmo_resources(&self) -> Option<gizmos::GizmoResources> {
        self.gizmo_resources
    }

    fn ensure_gizmo_resources(&mut self, renderer: &mut Renderer) -> gizmos::GizmoResources {
        if self.gizmo_resources.is_none() {
            let resources = gizmos::create_resources(renderer, &mut self.assets);
            self.gizmo_resources = Some(resources);
        }
        self.gizmo_resources
            .expect("gizmo resources must be initialized")
    }

    pub(crate) fn transform_gizmo_resources(
        &self,
    ) -> Option<transform_gizmos::TransformGizmoResources> {
        self.transform_gizmo_resources
    }

    fn ensure_transform_gizmo_resources(
        &mut self,
        renderer: &mut Renderer,
    ) -> transform_gizmos::TransformGizmoResources {
        if self.transform_gizmo_resources.is_none() {
            let resources = transform_gizmos::create_resources(renderer, &mut self.assets);
            self.transform_gizmo_resources = Some(resources);
        }
        self.transform_gizmo_resources
            .expect("transform gizmo resources must be initialized")
    }

    pub fn world(&self) -> &World {
        self.main_world()
    }

    pub fn world_mut(&mut self) -> &mut World {
        self.main_world_mut()
    }

    pub fn animations(&self) -> &[AnimationClip] {
        self.node_animations(self.main_scene)
    }

    pub fn animation_states(&self) -> &[AnimationState] {
        self.node_animation_states(self.main_scene)
    }

    pub fn add_animation_clip(&mut self, clip: AnimationClip) -> usize {
        self.add_node_animation_clip(self.main_scene, clip)
    }

    pub fn play_animation(&mut self, clip_index: usize, looping: bool) -> Option<usize> {
        self.play_node_animation(self.main_scene, clip_index, looping)
    }

    pub fn update(&mut self, dt: f64) {
        self.time += dt;

        let absolute_time = self.time;
        for node in self.nodes_iter_mut() {
            node.instance_mut().update(dt, absolute_time);
        }

        self.update_world_transforms();

        let main_scene_index = self.main_scene.index();
        if let Some(Some(main_node)) = self.nodes.get_mut(main_scene_index) {
            let world = main_node.instance_mut().world_mut();
            if let Err(err) = self.scripting.update_scripts(world, dt) {
                error!("Rune scripting error: {err}");
            }
        } else {
            error!("Rune scripting error: main scene node is missing");
        }
    }

    pub(crate) fn path_from_root(&self, mut node: SceneNodeId) -> Vec<usize> {
        let mut path = Vec::new();
        while let Some(parent) = self.node_parent(node) {
            let parent_node = self.node(parent);
            if let Some(index) = parent_node
                .children()
                .iter()
                .position(|&child| child == node)
            {
                path.push(index);
            } else {
                break;
            }

            node = parent;

            if parent == self.root {
                break;
            }
        }

        path.reverse();
        path
    }

    pub(crate) fn node_from_path(&self, path: &[usize]) -> Option<SceneNodeId> {
        let mut current = self.root;
        for &index in path {
            let children = self.node(current).children();
            let next = *children.get(index)?;
            current = next;
        }

        Some(current)
    }

    fn update_world_transforms(&mut self) {
        let root_transform = Transform::IDENTITY;
        self.update_world_transform_recursive(self.root, root_transform);
    }

    fn update_world_transform_recursive(&mut self, node_id: SceneNodeId, parent: Transform) {
        let (world_transform, children) = {
            let node = self.node_mut(node_id);
            let world_transform = parent.mul_transform(node.local_transform());
            node.set_world_transform(world_transform);
            (world_transform, node.children().to_vec())
        };

        for child in children {
            self.update_world_transform_recursive(child, world_transform);
        }
    }

    pub fn propagate_transforms(&mut self) {
        for node in self.nodes_iter_mut() {
            node.instance_mut().propagate_transforms();
        }
    }

    pub fn render(
        &mut self,
        renderer: &mut Renderer,
        batcher: &mut RenderBatcher,
        custom_render: &mut Option<CustomRenderRequest<'_>>,
        gizmos_enabled: bool,
    ) -> Result<crate::renderer::RenderFrame, wgpu::SurfaceError> {
        batcher.clear();
        let camera_vectors = rendering::CameraVectors::from_renderer(renderer);
        let gizmo_resources = if gizmos_enabled {
            Some(self.ensure_gizmo_resources(renderer))
        } else {
            None
        };
        let mut selected_transform: Option<Transform> = None;

        for node in self.nodes_iter() {
            let world = node.instance().world();
            let world_transform = *node.world_transform();

            for mut object in rendering::build_render_objects(world, camera_vectors).into_iter() {
                object.transform = world_transform.mul_transform(&object.transform);
                batcher.add(object);
            }

            if let Some(resources) = gizmo_resources {
                for gizmo in
                    gizmos::build_light_gizmos(world, camera_vectors, world_transform, resources)
                {
                    batcher.add(gizmo);
                }
            }

            if gizmos_enabled && selected_transform.is_none() {
                if let Some((_, (entity_world, _))) = world
                    .query::<(&WorldTransform, &SelectedInEditor)>()
                    .iter()
                    .next()
                {
                    let combined = world_transform.mul_transform(&entity_world.0);
                    selected_transform = Some(combined);
                }
            }
        }

        if gizmos_enabled && selected_transform.is_none() {
            selected_transform = self.selected_gizmo_transform();
        }

        if gizmos_enabled {
            if let Some(selection_transform) = selected_transform {
                let transform_resources = self.ensure_transform_gizmo_resources(renderer);
                for gizmo in transform_gizmos::build_transform_gizmos(
                    camera_vectors,
                    selection_transform,
                    self.transform_gizmo_mode,
                    self.transform_gizmo_space,
                    transform_resources,
                    self.transform_gizmo_hover,
                ) {
                    batcher.add(gizmo);
                }
            }
        }

        let mut lights_data = crate::renderer::LightsData::default();

        for node in self.nodes_iter() {
            let node_lights = lights::collect_lights(
                node.instance().world(),
                camera_vectors,
                *node.world_transform(),
            );
            lights_data.extend_from(&node_lights);
        }

        renderer.render(
            self,
            &self.assets,
            batcher,
            &lights_data,
            &self.environment,
            custom_render,
        )
    }

    pub fn add_default_lighting(&mut self) -> usize {
        lights::add_default_lighting(self.main_world_mut())
    }

    pub fn has_any_lights(&self) -> bool {
        lights::has_any_lights(self.main_world())
    }

    pub fn merge_as_child(&mut self, parent_entity: hecs::Entity, other: Scene) {
        if let Some(asset) = other.export_main_asset("MergedScene") {
            let mut instance = asset.instantiate();
            let (animations, animation_states) = instance.take_animation_data();
            let entity_map = super::internal::composition::merge_world_as_child(
                self.main_world_mut(),
                parent_entity,
                instance.into_world(),
            );
            self.attach_imported_animations(animations, animation_states, &entity_map);
        }
    }

    pub fn process_pending_gltf_imports(&mut self, renderer: &mut Renderer) {
        let pending = self.scripting_mut().take_pending_gltf_imports();
        if pending.is_empty() {
            return;
        }

        let mut textures_updated = false;

        for PendingGltfImport {
            parent,
            path,
            scale,
        } in pending
        {
            if self.main_world().entity(parent).is_err() {
                warn!(
                    "Skipping glTF import for missing target entity {:?}: {:?}",
                    parent, path
                );
                continue;
            }

            match SceneLoader::load_gltf_asset(&path, renderer, scale) {
                Ok(mut bundle) => {
                    if bundle.register_resources(&mut self.assets) {
                        textures_updated = true;
                    }

                    let mut instance = bundle.asset.instantiate();
                    let (animations, animation_states) = instance.take_animation_data();
                    let entity_map = super::internal::composition::merge_world_as_child(
                        self.main_world_mut(),
                        parent,
                        instance.into_world(),
                    );

                    self.attach_imported_animations(animations, animation_states, &entity_map);
                }
                Err(err) => {
                    error!("Failed to import glTF {:?}: {err}", path);
                }
            }
        }

        if textures_updated {
            renderer.update_texture_bind_group(&self.assets);
        }
    }

    fn attach_imported_animations(
        &mut self,
        mut animations: Vec<AnimationClip>,
        animation_states: Vec<AnimationState>,
        entity_map: &HashMap<hecs::Entity, hecs::Entity>,
    ) {
        if animations.is_empty() && animation_states.is_empty() {
            return;
        }

        for clip in &mut animations {
            for channel in &mut clip.channels {
                if let AnimationTarget::Transform { entity, .. } = &mut channel.target {
                    if let Some(&mapped) = entity_map.get(entity) {
                        *entity = mapped;
                    } else {
                        warn!(
                            "Skipping animation channel targeting missing entity {:?}",
                            entity
                        );
                    }
                }
            }
        }

        {
            let main_instance = self.main_instance_mut();
            let mut clip_indices = Vec::with_capacity(animations.len());

            for clip in animations.into_iter() {
                let index = main_instance.add_animation_clip(clip);
                clip_indices.push(index);
            }

            for mut state in animation_states.into_iter() {
                let Some(&new_index) = clip_indices.get(state.clip_index) else {
                    warn!(
                        "Skipping animation state referencing missing clip index {}",
                        state.clip_index
                    );
                    continue;
                };

                state.clip_index = new_index;
                if main_instance.push_animation_state(state).is_none() {
                    warn!("Failed to attach animation state for clip {}", new_index);
                }
            }
        }
    }

    pub fn debug_print_transforms(&self) {
        super::internal::debug::debug_print_transforms(self.main_instance().world());
    }

    pub fn create_node(
        &mut self,
        name: impl Into<String>,
        parent: Option<SceneNodeId>,
    ) -> SceneNodeId {
        let name = name.into();
        let parent_id = parent.unwrap_or(self.root);
        assert!(self.is_valid_node(parent_id), "Invalid parent node");
        let id = self.allocate_node(name, SceneInstance::new());
        self.attach_node(id, parent_id);
        self.update_world_transforms();
        id
    }

    pub fn instantiate_asset_named(
        &mut self,
        asset: &SceneAsset,
        name: impl Into<String>,
        parent: Option<SceneNodeId>,
    ) -> SceneNodeId {
        let name = name.into();
        let parent_id = parent.unwrap_or(self.root);
        assert!(self.is_valid_node(parent_id), "Invalid parent node");
        let id = self.allocate_node(name, asset.instantiate());
        {
            let node = self.node_mut(id);
            node.set_local_transform(Transform::from(asset.root_transform.clone()));
        }
        self.attach_node(id, parent_id);
        self.update_world_transforms();
        id
    }

    pub fn instantiate_asset(
        &mut self,
        asset: &SceneAsset,
        parent: Option<SceneNodeId>,
    ) -> SceneNodeId {
        self.instantiate_asset_named(asset, asset.name.clone(), parent)
    }

    pub fn instantiate_tree_asset(
        &mut self,
        asset: &SceneTreeAsset,
        parent: Option<SceneNodeId>,
    ) -> SceneNodeId {
        let parent_id = parent.unwrap_or(self.root);
        assert!(self.is_valid_node(parent_id), "Invalid parent node");
        let node_id = self.instantiate_tree_node(&asset.root, parent_id);
        self.update_world_transforms();
        node_id
    }

    fn instantiate_tree_node(
        &mut self,
        node_asset: &SceneTreeAssetNode,
        parent: SceneNodeId,
    ) -> SceneNodeId {
        let node_id = if let Some(asset) = &node_asset.asset {
            self.instantiate_asset_named(asset, node_asset.name.clone(), Some(parent))
        } else {
            self.create_node(node_asset.name.clone(), Some(parent))
        };

        *self.node_local_transform_mut(node_id) = Transform::from(node_asset.transform.clone());

        for child in &node_asset.children {
            self.instantiate_tree_node(child, node_id);
        }

        node_id
    }

    pub fn export_main_asset(&self, name: impl Into<String>) -> Option<SceneAsset> {
        self.export_node_asset_internal(self.main_scene, Some(name.into()))
    }

    pub fn export_node_asset(&self, node: SceneNodeId) -> Option<SceneAsset> {
        self.export_node_asset_internal(node, None)
    }

    pub fn export_tree_asset(&self, name: impl Into<String>) -> SceneTreeAsset {
        SceneTreeAsset {
            name: name.into(),
            root: self.build_tree_asset_node(self.root),
        }
    }

    fn build_tree_asset_node(&self, node: SceneNodeId) -> SceneTreeAssetNode {
        let node_ref = self.node(node);
        let children = node_ref
            .children()
            .iter()
            .map(|&child| self.build_tree_asset_node(child))
            .collect();

        build_tree_asset_node(
            node_ref.name(),
            *node_ref.local_transform(),
            self.export_node_asset_internal(node, None),
            children,
        )
    }

    pub fn remove_node(&mut self, node: SceneNodeId) -> Option<SceneTreeAssetNode> {
        if node == self.root || !self.is_valid_node(node) {
            return None;
        }

        if node == self.main_scene {
            self.main_scene = self.root;
        }

        self.detach_node(node);
        let removed = self.remove_node_recursive(node)?;
        self.update_world_transforms();
        Some(removed)
    }

    fn remove_node_recursive(&mut self, node: SceneNodeId) -> Option<SceneTreeAssetNode> {
        if !self.is_valid_node(node) {
            return None;
        }

        let asset = self.export_node_asset_internal(node, None);
        let (name, transform, children_ids) = {
            let node_ref = self.node(node);
            (
                node_ref.name().to_string(),
                SerializedTransform::from(*node_ref.local_transform()),
                node_ref.children().to_vec(),
            )
        };

        let mut children = Vec::with_capacity(children_ids.len());
        for child in children_ids {
            if let Some(child_asset) = self.remove_node_recursive(child) {
                children.push(child_asset);
            }
        }

        self.nodes[node.index()] = None;
        self.free_list.push(node);

        Some(SceneTreeAssetNode {
            name,
            transform,
            asset,
            children,
        })
    }

    fn export_node_asset_internal(
        &self,
        node: SceneNodeId,
        name_override: Option<String>,
    ) -> Option<SceneAsset> {
        if !self.is_valid_node(node) {
            return None;
        }

        let node_ref = self.node(node);
        let (entities, index_map) = serialize_world(node_ref.instance().world());
        let animations: Vec<_> = node_ref
            .instance()
            .animations()
            .iter()
            .filter_map(|clip| SerializedAnimationClip::from_clip(clip, &index_map))
            .collect();
        let animation_states = node_ref.instance().animation_states().to_vec();

        if entities.is_empty() && animations.is_empty() && animation_states.is_empty() {
            return None;
        }

        let name = name_override.unwrap_or_else(|| node_ref.name().to_string());

        Some(SceneAsset {
            name,
            root_transform: SerializedTransform::from(*node_ref.local_transform()),
            entities,
            animations,
            animation_states,
        })
    }

    fn allocate_node(&mut self, name: String, instance: SceneInstance) -> SceneNodeId {
        if let Some(id) = self.free_list.pop() {
            self.nodes[id.index()] = Some(SceneNode::new(id, name, instance));
            id
        } else {
            let id = SceneNodeId::new(self.nodes.len() as u32);
            self.nodes.push(Some(SceneNode::new(id, name, instance)));
            id
        }
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::super::animation::{
        AnimationChannel, AnimationInterpolation, AnimationOutput, AnimationSampler,
        AnimationTarget, MaterialProperty, TransformProperty,
    };
    use super::super::builder::EntityBuilder;
    use super::super::components::{
        Children, DirectionalLight, Name, Parent, TransformComponent, Visible,
    };
    use super::*;
    use crate::scripting::{RuneScriptComponent, RuneScriptSource};
    use glam::{Quat, Vec3};

    #[test]
    fn serialized_transform_roundtrip() {
        let transform = Transform::from_trs(
            Vec3::new(1.0, 2.0, 3.0),
            glam::Quat::from_rotation_y(1.2),
            Vec3::new(0.5, 0.75, 1.25),
        );

        let serialized = SerializedTransform::from(transform);
        let restored: Transform = serialized.into();

        assert!(restored
            .translation
            .abs_diff_eq(transform.translation, 1e-5));
        assert!(restored.rotation.abs_diff_eq(transform.rotation, 1e-5));
        assert!(restored.scale.abs_diff_eq(transform.scale, 1e-5));
    }

    #[test]
    fn asset_json_roundtrip() {
        let mut scene = Scene::new();
        let world = scene.main_world_mut();
        let entity = world.spawn((
            Name::new("TestEntity"),
            TransformComponent(Transform::IDENTITY),
            Visible(true),
        ));
        let mut children = Vec::new();
        for i in 0..2 {
            let child = world.spawn((
                Name::new(format!("Child_{i}")),
                TransformComponent(Transform::from_translation(Vec3::new(i as f32, 0.0, 0.0))),
                Parent(entity),
            ));
            children.push(child);
        }
        let _ = world.insert_one(entity, Children(children));

        let asset = scene.export_main_asset("Test").unwrap();
        let json = asset.to_json().unwrap();
        let restored = SceneAsset::from_json(&json).unwrap();
        assert_eq!(restored.entities.len(), 3);
        assert_eq!(restored.name, "Test");
        assert!(restored.animations.is_empty());
        assert!(restored.animation_states.is_empty());

        let mut other_scene = Scene::new();
        let node = other_scene.instantiate_asset(&restored, None);
        assert_eq!(other_scene.node_children(other_scene.root_id()).len(), 1);
        assert_eq!(other_scene.node_parent(node), Some(other_scene.root_id()));
    }

    #[test]
    fn scene_node_creation_and_removal() {
        let mut base = Scene::new();
        let entity = base.main_world_mut().spawn((
            Name::new("Unit"),
            TransformComponent(Transform::IDENTITY),
            Visible(true),
        ));
        base.main_world_mut()
            .insert_one(entity, Children(Vec::new()))
            .ok();
        let unit_asset = base.export_main_asset("Unit").unwrap();

        let mut scene = Scene::new();
        let parent = scene.create_node("Parent", None);
        let child = scene.instantiate_asset_named(&unit_asset, "Child", Some(parent));

        assert_eq!(scene.node_name(parent), "Parent");
        assert_eq!(scene.node_name(child), "Child");
        assert_eq!(scene.node_parent(child), Some(parent));
        assert_eq!(scene.node_children(parent), &[child]);

        let removed = scene.remove_node(parent).expect("node removed");
        assert_eq!(removed.name, "Parent");
        assert!(removed.asset.is_none());
        assert_eq!(removed.children.len(), 1);
        assert_eq!(removed.children[0].name, "Child");
        assert!(scene.iter_children(scene.root_id()).next().is_none());

        let mut rebuilt = Scene::new();
        let tree = SceneTreeAsset {
            name: "Rebuilt".to_string(),
            root: removed,
        };
        let new_node = rebuilt.instantiate_tree_asset(&tree, None);
        assert_eq!(rebuilt.node_children(rebuilt.root_id()), &[new_node]);
        assert_eq!(rebuilt.node_children(new_node).len(), 1);
    }

    #[test]
    fn animation_serialization_roundtrip() {
        use glam::{Quat, Vec4};

        let mut scene = Scene::new();
        let entity = scene.main_world_mut().spawn((
            Name::new("Animated"),
            TransformComponent(Transform::IDENTITY),
            Visible(true),
        ));

        let sampler = AnimationSampler {
            times: vec![0.0, 1.0],
            output: AnimationOutput::Vec3(vec![Vec3::ZERO, Vec3::splat(1.0)]),
            interpolation: AnimationInterpolation::Linear,
        };

        let rotation_sampler = AnimationSampler {
            times: vec![0.0, 1.0],
            output: AnimationOutput::Quat(vec![Quat::IDENTITY, Quat::from_rotation_y(1.0)]),
            interpolation: AnimationInterpolation::Linear,
        };

        let color_sampler = AnimationSampler {
            times: vec![0.0, 1.0],
            output: AnimationOutput::Vec4(vec![Vec4::ZERO, Vec4::splat(1.0)]),
            interpolation: AnimationInterpolation::Linear,
        };

        let mut clip = AnimationClip::new("Move");
        clip.add_channel(AnimationChannel {
            sampler,
            target: AnimationTarget::Transform {
                entity,
                property: TransformProperty::Translation,
            },
        });
        clip.add_channel(AnimationChannel {
            sampler: rotation_sampler,
            target: AnimationTarget::Transform {
                entity,
                property: TransformProperty::Rotation,
            },
        });
        clip.add_channel(AnimationChannel {
            sampler: color_sampler,
            target: AnimationTarget::Material {
                material_index: 0,
                property: MaterialProperty::BaseColorFactor,
            },
        });

        let clip_index = scene.add_animation_clip(clip);
        assert_eq!(clip_index, 0);
        scene.play_animation(clip_index, true);

        let asset = scene.export_main_asset("Anim").unwrap();
        assert_eq!(asset.animations.len(), 1);
        assert_eq!(asset.animation_states.len(), 1);

        let json = asset.to_json().unwrap();
        let restored = SceneAsset::from_json(&json).unwrap();
        assert_eq!(restored.animations.len(), 1);
        assert_eq!(restored.animation_states.len(), 1);

        let mut other = Scene::new();
        let node = other.instantiate_asset(&restored, None);
        assert_eq!(other.node_animations(node).len(), 1);
        assert_eq!(other.node_animation_states(node).len(), 1);
    }

    #[test]
    fn scene_tree_asset_roundtrip() {
        let mut base_a = Scene::new();
        let _ = base_a.main_world_mut().spawn((
            Name::new("A"),
            TransformComponent(Transform::IDENTITY),
            Visible(true),
        ));
        let asset_a = base_a.export_main_asset("AssetA").unwrap();

        let mut base_b = Scene::new();
        let _ = base_b.main_world_mut().spawn((
            Name::new("B"),
            TransformComponent(Transform::IDENTITY),
            Visible(true),
        ));
        let asset_b = base_b.export_main_asset("AssetB").unwrap();

        let mut scene = Scene::new();
        let node_a = scene.instantiate_asset(&asset_a, None);
        let _node_b = scene.instantiate_asset_named(&asset_b, "NodeB", Some(node_a));

        let tree = scene.export_tree_asset("SceneGraph");
        let json = tree.to_json().unwrap();
        let restored = SceneTreeAsset::from_json(&json).unwrap();
        assert_eq!(restored.name, "SceneGraph");
        assert_eq!(restored.root.children.len(), 1);

        let mut other = Scene::new();
        let instantiated_root = other.instantiate_tree_asset(&restored, None);
        assert_eq!(other.node_children(other.root_id()), &[instantiated_root]);
        assert_eq!(other.node_children(instantiated_root).len(), 1);
    }

    #[test]
    fn reset_script_runtime_allows_rerun() {
        let mut scene = Scene::new();

        let script = RuneScriptSource::inline(
            "scene_core_restart_test",
            r#"
                struct CubeState { angle }

                pub fn on_created(self_entity) {
                    set_state(self_entity, "cube_state", CubeState { angle: 0.0 });
                }

                pub fn update(self_entity, dt) {
                    let state = get_state(self_entity, "cube_state", CubeState { angle: 0.0 });
                    let angle = state.angle + dt * 1.5;
                    set_rotation(self_entity, angle, 0.0, 0.0);
                    set_state(self_entity, "cube_state", CubeState { angle });
                }
            "#,
        );

        {
            let world = scene.main_world_mut();
            EntityBuilder::new(world)
                .with_name("Runtime Cube")
                .with_transform(Transform::default())
                .with_script(script.clone())
                .spawn();
        }

        scene.update(0.016);
        {
            let world = scene.main_world();
            let mut query = world.query::<&TransformComponent>();
            assert!(
                query
                    .iter()
                    .any(|(_, transform)| transform.0.rotation != Quat::IDENTITY),
                "script did not run before reset"
            );
        }

        {
            let world = scene.main_world_mut();
            let mut query = world.query::<&mut TransformComponent>();
            for (_, transform) in query.iter() {
                transform.0.rotation = Quat::IDENTITY;
            }
        }

        scene.reset_script_runtime();
        scene.set_time(0.0);
        scene.update(0.0);
        scene.update(0.016);

        {
            let world = scene.main_world();
            let mut query = world.query::<&RuneScriptComponent>();
            for (_, component) in query.iter() {
                assert!(component.created_called(), "on_created was not re-run");
            }
        }

        let mut rotated_again = false;
        {
            let world = scene.main_world();
            let mut query = world.query::<&TransformComponent>();
            for (_, transform) in query.iter() {
                if transform.0.rotation != Quat::IDENTITY {
                    rotated_again = true;
                    break;
                }
            }
        }

        assert!(rotated_again, "script did not rerun after reset");
    }

    #[test]
    fn set_animation_playback_toggles_states() {
        let mut scene = Scene::new();
        let clip_index = scene.add_animation_clip(AnimationClip::new("TestClip"));
        let state_index = scene.play_animation(clip_index, true).expect("state");
        assert_eq!(state_index, 0);
        assert!(scene.animation_states()[state_index].playing);

        scene.set_animation_playback(false);
        let states = scene.animation_states();
        assert!(!states[state_index].playing);
        assert_eq!(states[state_index].time, 0.0);

        scene.set_animation_playback(true);
        assert!(scene.animation_states()[state_index].playing);
    }

    #[test]
    fn snapshot_restores_default_lighting() {
        let mut scene = Scene::new();
        assert!(!scene.has_any_lights());
        assert_eq!(scene.add_default_lighting(), 3);
        assert!(scene.has_any_lights());

        let tree = scene.export_tree_asset("SnapshotTest");
        let root_asset = tree
            .root
            .asset
            .as_ref()
            .expect("root asset missing in export");
        assert!(
            root_asset
                .entities
                .iter()
                .any(|entity| entity.directional_light.is_some()),
            "exported asset missing directional light"
        );

        let instance = root_asset.instantiate();
        let instanced_directional = instance.world().query::<&DirectionalLight>().iter().count();
        assert!(
            instanced_directional > 0,
            "instantiating asset did not produce directional lights"
        );

        let mut direct_scene = Scene::new();
        let node_id = direct_scene.instantiate_asset(root_asset, None);
        direct_scene.set_main_scene(node_id);
        let direct_count = direct_scene
            .main_world()
            .query::<&DirectionalLight>()
            .iter()
            .count();
        assert!(
            direct_count > 0,
            "instantiating asset into scene lost lights"
        );

        let snapshot = SceneSnapshot::capture(&scene);
        let scene = snapshot.into_scene();
        let restored_main_asset = scene
            .export_main_asset("RestoredMain")
            .expect("restored main asset missing");
        assert!(
            restored_main_asset
                .entities
                .iter()
                .any(|entity| entity.directional_light.is_some()),
            "restored asset missing directional lights"
        );
        let directional_count = scene
            .main_world()
            .query::<&DirectionalLight>()
            .iter()
            .count();
        assert!(directional_count > 0, "no directional lights after restore");
        assert!(scene.has_any_lights());
    }
}
