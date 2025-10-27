use super::animation::{AnimationClip, AnimationState};
use super::assets::{
    build_tree_asset_node, serialize_world, SceneAsset, SceneTreeAsset, SceneTreeAssetNode,
    SerializedAnimationClip, SerializedTransform,
};
use super::graph::{SceneInstance, SceneNode, SceneNodeId};
use super::import_state::SceneImports;
use super::internal::{gizmos, lights, rendering, transform_gizmos};
use super::loader::SceneImportDevice;
use super::render_bridge::RenderBridge;
use super::runtime_state::SceneRuntime;
use super::state::{
    EnvironmentState, GizmoState, TransformGizmoHandle, TransformGizmoMode, TransformGizmoSpace,
};
use crate::asset::{Assets, Handle, MeshData};
use crate::environment::Environment;
use crate::renderer::{CustomRenderRequest, RenderBatcher, Renderer};
use crate::scene::components::{
    CameraComponent, SelectedInEditor, TransformComponent, WorldTransform,
};
use crate::scene::internal::lights::safe_normalize;
use crate::scene::transform::Transform;
use crate::scene::Camera;
use crate::scripting::ScriptingState;
use crate::time::Instant;
use glam::Vec3;
use hecs::{Entity, World};
use log::error;
use std::collections::HashMap;

pub struct Scene {
    pub assets: Assets,
    environment: EnvironmentState,
    camera: Camera,
    nodes: Vec<Option<SceneNode>>,
    free_list: Vec<SceneNodeId>,
    root: SceneNodeId,
    main_scene: SceneNodeId,
    runtime: SceneRuntime,
    gizmos: GizmoState,
    imports: SceneImports,
}

impl Scene {
    pub fn new() -> Self {
        let mut nodes = Vec::new();
        let root_id = SceneNodeId::new(0);
        let root_node = SceneNode::new(root_id, "Root", SceneInstance::new());
        nodes.push(Some(root_node));

        Self {
            assets: Assets::default(),
            environment: EnvironmentState::new(),
            camera: Camera::default(),
            nodes,
            free_list: Vec::new(),
            root: root_id,
            main_scene: root_id,
            runtime: SceneRuntime::new(),
            gizmos: GizmoState::new(),
            imports: SceneImports::new(),
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

    pub(super) fn nodes_iter(&self) -> impl Iterator<Item = &SceneNode> {
        self.nodes.iter().filter_map(|n| n.as_ref())
    }

    pub(super) fn nodes_iter_mut(&mut self) -> impl Iterator<Item = &mut SceneNode> {
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
        self.runtime.scripting()
    }

    pub fn scripting_mut(&mut self) -> &mut ScriptingState {
        self.runtime.scripting_mut()
    }

    pub fn transform_gizmo_mode(&self) -> TransformGizmoMode {
        self.gizmos.mode()
    }

    pub fn set_transform_gizmo_mode(&mut self, mode: TransformGizmoMode) {
        self.gizmos.set_mode(mode);
    }

    pub fn transform_gizmo_space(&self) -> TransformGizmoSpace {
        self.gizmos.space()
    }

    pub fn set_transform_gizmo_space(&mut self, space: TransformGizmoSpace) {
        self.gizmos.set_space(space);
    }

    pub fn transform_gizmo_hover(&self) -> Option<TransformGizmoHandle> {
        self.gizmos.hover()
    }

    pub fn set_transform_gizmo_hover(&mut self, hover: Option<TransformGizmoHandle>) {
        self.gizmos.set_hover(hover);
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
            fov_y: camera.fov_y_radians(),
        };

        transform_gizmos::hit_test(
            camera_vectors,
            transform,
            self.transform_gizmo_mode(),
            self.transform_gizmo_space(),
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
        self.runtime.init_timer();
    }

    pub fn reset_script_runtime(&mut self) {
        let mut runtime = std::mem::take(&mut self.runtime);
        runtime.reset_script_runtime(self.main_world_mut());
        self.runtime = runtime;
    }

    pub fn set_animation_playback(&mut self, playing: bool) {
        for node in self.nodes_iter_mut() {
            let instance = node.instance_mut();

            if playing {
                instance.begin_playback();
            } else {
                instance.restore_rest_pose();
            }

            let states = instance.animation_states_mut();
            for state in states {
                state.playing = playing;
                if !playing {
                    state.time = 0.0;
                }
            }
        }

        if !playing {
            // Ensure transforms are fully propagated at both entity and node levels
            self.propagate_transforms(); // Propagates Parent/Children in ECS world
            self.update_world_transforms(); // Updates scene node transforms

            // Force one more propagation to catch any missed updates
            for node in self.nodes_iter_mut() {
                node.instance_mut().propagate_transforms();
            }
        }
    }

    pub fn time(&self) -> f64 {
        self.runtime.time()
    }

    pub(crate) fn set_time(&mut self, time: f64) {
        self.runtime.set_time(time);
    }

    pub fn last_frame(&self) -> Instant {
        self.runtime.last_frame()
    }

    pub fn last_frame_instant(&self) -> Option<Instant> {
        self.runtime.last_frame_instant()
    }

    pub fn set_last_frame(&mut self, instant: Instant) {
        self.runtime.set_last_frame(instant);
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

    pub fn active_camera_entity(&self) -> Option<Entity> {
        self.main_instance().active_camera()
    }

    pub fn set_active_camera_entity(&mut self, entity: Option<Entity>) {
        let entity = match entity {
            Some(candidate) if self.main_world().contains(candidate) => Some(candidate),
            _ => None,
        };

        self.main_instance_mut().set_active_camera(entity);
        self.refresh_active_camera();
    }

    pub fn environment(&self) -> &Environment {
        self.environment.environment()
    }

    pub fn environment_mut(&mut self) -> &mut Environment {
        self.environment.environment_mut()
    }

    pub fn set_environment(&mut self, environment: Environment) {
        let mut state = std::mem::take(&mut self.environment);
        {
            let world = self.main_world_mut();
            state.set_environment(environment, world);
        }
        self.environment = state;
    }

    pub(crate) fn refresh_environment_state(&mut self) {
        let mut state = std::mem::take(&mut self.environment);
        {
            let world = self.main_world_mut();
            state.refresh(world);
        }
        self.environment = state;
    }

    pub(crate) fn replace_assets(&mut self, assets: Assets) {
        self.assets = assets;
        self.gizmos.clear_resources();
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

    pub(super) fn main_instance(&self) -> &SceneInstance {
        self.node(self.main_scene).instance()
    }

    pub(super) fn main_instance_mut(&mut self) -> &mut SceneInstance {
        self.node_mut(self.main_scene).instance_mut()
    }

    pub fn main_world(&self) -> &World {
        self.main_instance().world()
    }

    pub fn main_world_mut(&mut self) -> &mut World {
        self.main_instance_mut().world_mut()
    }

    pub(crate) fn gizmo_resources(&self) -> Option<gizmos::GizmoResources> {
        self.gizmos.gizmo_resources()
    }

    pub(crate) fn set_gizmo_resources(&mut self, resources: Option<gizmos::GizmoResources>) {
        self.gizmos.set_gizmo_resources(resources);
    }

    pub(super) fn ensure_gizmo_resources(
        &mut self,
        renderer: &mut Renderer,
    ) -> gizmos::GizmoResources {
        self.gizmos
            .ensure_gizmo_resources(renderer, &mut self.assets)
    }

    pub(crate) fn transform_gizmo_resources(
        &self,
    ) -> Option<transform_gizmos::TransformGizmoResources> {
        self.gizmos.transform_gizmo_resources()
    }

    pub(crate) fn set_transform_gizmo_resources(
        &mut self,
        resources: Option<transform_gizmos::TransformGizmoResources>,
    ) {
        self.gizmos.set_transform_gizmo_resources(resources);
    }

    pub(super) fn ensure_transform_gizmo_resources(
        &mut self,
        renderer: &mut Renderer,
    ) -> transform_gizmos::TransformGizmoResources {
        self.gizmos
            .ensure_transform_gizmo_resources(renderer, &mut self.assets)
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
        self.refresh_environment_state();
        let absolute_time = self.runtime.advance_time(dt);
        for node in self.nodes_iter_mut() {
            node.instance_mut().update(dt, absolute_time);
        }

        self.update_world_transforms();

        let main_scene_index = self.main_scene.index();
        if let Some(Some(main_node)) = self.nodes.get_mut(main_scene_index) {
            let world = main_node.instance_mut().world_mut();
            self.runtime.run_scripts(world, dt);
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

        self.refresh_active_camera();
    }

    fn refresh_active_camera(&mut self) {
        let active = self.active_camera_entity();
        if let Some(entity) = active {
            if !self.apply_camera_from_entity(entity) {
                self.main_instance_mut().set_active_camera(None);
            }
        }
    }

    fn apply_camera_from_entity(&mut self, entity: Entity) -> bool {
        let data = {
            let world = self.main_world();
            if !world.contains(entity) {
                return false;
            }

            let projection = world
                .get::<&CameraComponent>(entity)
                .ok()
                .map(|component| *component);
            let transform = world
                .get::<&WorldTransform>(entity)
                .map(|component| component.0)
                .ok()
                .or_else(|| {
                    world
                        .get::<&TransformComponent>(entity)
                        .map(|component| component.0)
                        .ok()
                });

            Some((projection, transform))
        };

        let Some((projection, transform)) = data else {
            return false;
        };

        let mut camera = *self.camera();

        if let Some(component) = projection {
            component.apply_to_camera(&mut camera);
        }

        if let Some(transform) = transform {
            let eye = transform.translation;
            let forward = safe_normalize(transform.rotation * Vec3::NEG_Z, Vec3::NEG_Z);
            let up = safe_normalize(transform.rotation * Vec3::Y, Vec3::Y);

            camera.eye = eye;
            camera.target = eye + forward;
            camera.up = up;
        }

        self.camera = camera;
        true
    }

    pub fn render<'a>(
        &'a mut self,
        renderer: &mut Renderer,
        batcher: &mut RenderBatcher,
        custom_render: Option<&'a mut CustomRenderRequest<'a>>,
        gizmos_enabled: bool,
    ) -> Result<crate::renderer::RenderFrame, wgpu::SurfaceError> {
        RenderBridge::render(self, renderer, batcher, custom_render, gizmos_enabled)
    }

    pub fn add_default_lighting(&mut self) -> usize {
        lights::add_default_lighting(self.main_world_mut())
    }

    pub fn has_any_lights(&self) -> bool {
        lights::has_any_lights(self.main_world())
    }

    pub fn merge_as_child(
        &mut self,
        parent_entity: hecs::Entity,
        other: Scene,
        renderer: &mut dyn SceneImportDevice,
    ) {
        let mut imports = std::mem::take(&mut self.imports);
        imports.merge_as_child(self, parent_entity, other, renderer);
        self.imports = imports;
    }

    pub fn process_pending_gltf_imports(&mut self, renderer: &mut Renderer) {
        let mut imports = std::mem::take(&mut self.imports);
        imports.process_pending_gltf_imports(self, renderer);
        self.imports = imports;
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

    fn instantiate_asset_internal(
        &mut self,
        asset: &SceneAsset,
        name: String,
        parent: Option<SceneNodeId>,
        renderer: Option<&mut dyn SceneImportDevice>,
    ) -> SceneNodeId {
        let parent_id = parent.unwrap_or(self.root);
        assert!(self.is_valid_node(parent_id), "Invalid parent node");
        let instance = match renderer {
            Some(device) => asset.instantiate(Some(device), &mut self.assets),
            None => asset.instantiate(None, &mut self.assets),
        };
        let should_apply_camera = parent.is_none() && self.main_scene == self.root;
        let active_camera_entity = if should_apply_camera {
            instance.active_camera()
        } else {
            None
        };
        let id = self.allocate_node(name, instance);
        {
            let node = self.node_mut(id);
            node.set_local_transform(Transform::from(asset.root_transform.clone()));
        }
        self.attach_node(id, parent_id);
        self.update_world_transforms();
        if should_apply_camera {
            self.set_active_camera_entity(active_camera_entity);
        }
        self.refresh_environment_state();
        id
    }

    pub fn instantiate_asset_named(
        &mut self,
        asset: &SceneAsset,
        name: impl Into<String>,
        parent: Option<SceneNodeId>,
    ) -> SceneNodeId {
        self.instantiate_asset_internal(asset, name.into(), parent, None)
    }

    pub fn instantiate_asset_named_with_renderer(
        &mut self,
        asset: &SceneAsset,
        name: impl Into<String>,
        parent: Option<SceneNodeId>,
        renderer: &mut dyn SceneImportDevice,
    ) -> SceneNodeId {
        self.instantiate_asset_internal(asset, name.into(), parent, Some(renderer))
    }

    pub fn instantiate_asset(
        &mut self,
        asset: &SceneAsset,
        parent: Option<SceneNodeId>,
    ) -> SceneNodeId {
        self.instantiate_asset_named(asset, asset.name.clone(), parent)
    }

    pub fn instantiate_asset_with_renderer(
        &mut self,
        asset: &SceneAsset,
        parent: Option<SceneNodeId>,
        renderer: &mut dyn SceneImportDevice,
    ) -> SceneNodeId {
        self.instantiate_asset_named_with_renderer(asset, asset.name.clone(), parent, renderer)
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
        self.export_node_asset_internal(self.main_scene, Some(name.into()), true)
        // REMAP FOR SAVE
    }

    pub fn export_node_asset(&self, node: SceneNodeId) -> Option<SceneAsset> {
        self.export_node_asset_internal(node, None, true) // REMAP FOR SAVE
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
            self.export_node_asset_internal(node, None, false), // DON'T REMAP FOR SNAPSHOTS
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

        let asset = self.export_node_asset_internal(node, None, true);
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
        remap_handles: bool, // NEW PARAMETER
    ) -> Option<SceneAsset> {
        if !self.is_valid_node(node) {
            return None;
        }

        let node_ref = self.node(node);
        let (mut entities, index_map) = serialize_world(node_ref.instance().world());
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

        let mut mesh_map: HashMap<usize, usize> = HashMap::new();
        let mut mesh_data: Vec<MeshData> = Vec::new();

        if remap_handles {
            // ONLY REMAP IF REQUESTED
            for entity in &mut entities {
                if entity.primitive_mesh.is_some() {
                    continue;
                }

                if let Some(handle_index) = entity.mesh_handle {
                    let mapped = if let Some(&existing) = mesh_map.get(&handle_index) {
                        Some(existing)
                    } else if let Some(mesh) = self.assets.meshes.get(Handle::new(handle_index)) {
                        let next_index = mesh_data.len();
                        mesh_data.push(mesh.data().clone());
                        mesh_map.insert(handle_index, next_index);
                        Some(next_index)
                    } else {
                        log::warn!(
                            "Skipping missing mesh handle {} while exporting asset '{}'",
                            handle_index,
                            name
                        );
                        None
                    };

                    entity.mesh_handle = mapped;
                }
            }
        } else { // FOR SNAPSHOTS: KEEP ORIGINAL HANDLES, NO MESH DATA
             // Don't modify mesh handles, don't extract mesh data
             // The full asset pool is preserved separately in the snapshot
        }

        let active_camera = if node == self.main_scene {
            node_ref
                .instance()
                .active_camera()
                .and_then(|entity| index_map.get(&entity).copied())
                .or_else(|| {
                    let target = CameraComponent::from(self.camera());
                    entities.iter().enumerate().find_map(|(index, entity)| {
                        entity
                            .camera
                            .as_ref()
                            .and_then(|camera| (camera == &target).then_some(index))
                    })
                })
        } else {
            None
        };

        Some(SceneAsset {
            name,
            root_transform: SerializedTransform::from(*node_ref.local_transform()),
            entities,
            animations,
            animation_states,
            mesh_data,
            active_camera,
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
    use crate::scene::SceneSnapshot;
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
    fn stopping_playback_restores_rest_pose_without_snapshot() {
        use crate::scene::animation::{
            AnimationChannel, AnimationClip, AnimationInterpolation, AnimationOutput,
            AnimationSampler, AnimationTarget, TransformProperty,
        };
        use crate::scene::components::Visible;

        let mut scene = Scene::new();
        let entity = scene.main_world_mut().spawn((
            TransformComponent(Transform::from_trs(
                glam::Vec3::new(1.0, 2.0, 3.0),
                glam::Quat::IDENTITY,
                glam::Vec3::splat(1.5),
            )),
            Visible(true),
        ));

        let mut clip = AnimationClip::new("Translate");
        clip.add_channel(AnimationChannel {
            sampler: AnimationSampler {
                times: vec![0.0, 1.0],
                output: AnimationOutput::Vec3(vec![
                    glam::Vec3::new(1.0, 2.0, 3.0),
                    glam::Vec3::new(10.0, 20.0, 30.0),
                ]),
                interpolation: AnimationInterpolation::Linear,
            },
            target: AnimationTarget::Transform {
                entity,
                property: TransformProperty::Translation,
            },
        });

        let clip_index = scene.add_animation_clip(clip);
        scene.play_animation(clip_index, true);

        scene.set_animation_playback(false);

        let initial_transform = scene
            .main_world()
            .get::<&TransformComponent>(entity)
            .unwrap()
            .0;

        scene.set_animation_playback(true);
        scene.update(0.5);

        scene.set_animation_playback(false);
        let restored_transform = scene
            .main_world()
            .get::<&TransformComponent>(entity)
            .unwrap()
            .0;

        assert!(restored_transform
            .translation
            .abs_diff_eq(initial_transform.translation, 1e-5));
        assert!(restored_transform
            .rotation
            .abs_diff_eq(initial_transform.rotation, 1e-5));
        assert!(restored_transform
            .scale
            .abs_diff_eq(initial_transform.scale, 1e-5));
    }

    #[test]
    fn autoplayed_animation_restores_rest_pose_after_stop() {
        use crate::scene::animation::{
            AnimationChannel, AnimationClip, AnimationInterpolation, AnimationOutput,
            AnimationSampler, AnimationTarget, TransformProperty,
        };
        use crate::scene::components::Visible;

        let mut scene = Scene::new();
        let entity = scene.main_world_mut().spawn((
            TransformComponent(Transform::from_trs(
                glam::Vec3::new(-4.0, 1.0, 2.0),
                glam::Quat::IDENTITY,
                glam::Vec3::splat(0.75),
            )),
            Visible(true),
        ));

        let baseline = scene
            .main_world()
            .get::<&TransformComponent>(entity)
            .unwrap()
            .0;

        let mut clip = AnimationClip::new("AutoPlay");
        clip.add_channel(AnimationChannel {
            sampler: AnimationSampler {
                times: vec![0.0, 1.0],
                output: AnimationOutput::Vec3(vec![
                    glam::Vec3::new(-4.0, 1.0, 2.0),
                    glam::Vec3::new(5.0, 7.0, -3.0),
                ]),
                interpolation: AnimationInterpolation::Linear,
            },
            target: AnimationTarget::Transform {
                entity,
                property: TransformProperty::Translation,
            },
        });

        let clip_index = scene.add_animation_clip(clip);
        scene.play_animation(clip_index, true);

        scene.update(0.5);

        let animated = scene
            .main_world()
            .get::<&TransformComponent>(entity)
            .unwrap()
            .0;
        assert!(!animated.translation.abs_diff_eq(baseline.translation, 1e-5));

        scene.set_animation_playback(false);

        let restored = scene
            .main_world()
            .get::<&TransformComponent>(entity)
            .unwrap()
            .0;

        assert!(restored.translation.abs_diff_eq(baseline.translation, 1e-5));
        assert!(restored.rotation.abs_diff_eq(baseline.rotation, 1e-5));
        assert!(restored.scale.abs_diff_eq(baseline.scale, 1e-5));
    }

    #[test]
    fn snapshot_restores_default_lighting() {
        let mut scene = Scene::new();
        assert!(!scene.has_any_lights());
        let added = scene.add_default_lighting();
        assert!(added >= 1);
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

        let mut scratch_assets = Assets::default();
        let instance = root_asset.instantiate(None, &mut scratch_assets);
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

    #[test]
    fn snapshot_restores_transforms_after_playback() {
        use crate::scene::animation::{
            AnimationChannel, AnimationClip, AnimationInterpolation, AnimationOutput,
            AnimationSampler, AnimationTarget, TransformProperty,
        };
        use crate::scene::components::{Name, Visible};

        let mut scene = Scene::new();
        let entity = scene.main_world_mut().spawn((
            Name::new("AnimatedEntity"),
            TransformComponent(Transform::from_trs(
                glam::Vec3::new(1.0, 2.0, -3.0),
                glam::Quat::IDENTITY,
                glam::Vec3::new(0.5, 1.5, 2.5),
            )),
            Visible(true),
        ));

        let mut clip = AnimationClip::new("ScaleAnim");
        clip.add_channel(AnimationChannel {
            sampler: AnimationSampler {
                times: vec![0.0, 1.0],
                output: AnimationOutput::Vec3(vec![
                    glam::Vec3::new(0.5, 1.5, 2.5),
                    glam::Vec3::splat(3.0),
                ]),
                interpolation: AnimationInterpolation::Linear,
            },
            target: AnimationTarget::Transform {
                entity,
                property: TransformProperty::Scale,
            },
        });

        let clip_index = scene.add_animation_clip(clip);
        scene.play_animation(clip_index, true);

        let initial_scale = scene
            .main_world()
            .get::<&TransformComponent>(entity)
            .unwrap()
            .0
            .scale;

        let snapshot = SceneSnapshot::capture(&scene);

        scene.set_animation_playback(true);
        scene.update(0.5);

        let animated_scale = scene
            .main_world()
            .get::<&TransformComponent>(entity)
            .unwrap()
            .0
            .scale;
        assert_ne!(animated_scale, initial_scale);

        let mut restored = snapshot.into_scene();
        restored.set_animation_playback(false);
        restored.update(0.0);

        let world = restored.main_world();
        let mut query = world.query::<(&Name, &TransformComponent)>();
        let restored_scale = query
            .iter()
            .find(|(_, (name, _))| name.0 == "AnimatedEntity")
            .map(|(_, (_, transform))| transform.0.scale)
            .expect("restored entity not found");

        assert!(restored_scale.abs_diff_eq(initial_scale, 1e-6));
    }

    #[test]
    fn snapshot_restores_hierarchical_transforms_after_playback() {
        use crate::scene::animation::{
            AnimationChannel, AnimationClip, AnimationInterpolation, AnimationOutput,
            AnimationSampler, AnimationTarget, TransformProperty,
        };
        use crate::scene::components::{Children, Name, Parent, Visible};

        let mut scene = Scene::new();
        let world = scene.main_world_mut();

        let parent = world.spawn((
            Name::new("Parent"),
            TransformComponent(Transform::from_trs(
                glam::Vec3::new(5.0, -3.0, 2.0),
                glam::Quat::from_rotation_y(0.5),
                glam::Vec3::new(1.5, 2.0, 0.75),
            )),
            Visible(true),
        ));

        let child_a = world.spawn((
            Name::new("ChildA"),
            TransformComponent(Transform::from_trs(
                glam::Vec3::new(-2.0, 1.0, 0.5),
                glam::Quat::from_rotation_x(-0.25),
                glam::Vec3::new(0.5, 0.75, 1.25),
            )),
            Parent(parent),
            Visible(true),
        ));

        let child_b = world.spawn((
            Name::new("ChildB"),
            TransformComponent(Transform::from_trs(
                glam::Vec3::new(1.0, -2.5, 3.0),
                glam::Quat::from_rotation_z(0.8),
                glam::Vec3::new(0.9, 1.1, 0.6),
            )),
            Parent(parent),
            Visible(true),
        ));

        let grandchild = world.spawn((
            Name::new("Grandchild"),
            TransformComponent(Transform::from_trs(
                glam::Vec3::new(0.25, 0.5, -0.75),
                glam::Quat::from_rotation_x(0.35),
                glam::Vec3::new(1.25, 0.8, 0.95),
            )),
            Parent(child_a),
            Visible(true),
        ));

        world
            .insert_one(parent, Children(vec![child_a, child_b]))
            .unwrap();
        world
            .insert_one(child_a, Children(vec![grandchild]))
            .unwrap();

        let mut clip = AnimationClip::new("HierarchyAnim");
        clip.add_channel(AnimationChannel {
            sampler: AnimationSampler {
                times: vec![0.0, 1.0],
                output: AnimationOutput::Vec3(vec![
                    glam::Vec3::new(5.0, -3.0, 2.0),
                    glam::Vec3::new(10.0, -1.0, -4.0),
                ]),
                interpolation: AnimationInterpolation::Linear,
            },
            target: AnimationTarget::Transform {
                entity: parent,
                property: TransformProperty::Translation,
            },
        });
        clip.add_channel(AnimationChannel {
            sampler: AnimationSampler {
                times: vec![0.0, 1.0],
                output: AnimationOutput::Quat(vec![
                    glam::Quat::from_rotation_y(0.5),
                    glam::Quat::from_rotation_y(1.0),
                ]),
                interpolation: AnimationInterpolation::Linear,
            },
            target: AnimationTarget::Transform {
                entity: parent,
                property: TransformProperty::Rotation,
            },
        });
        clip.add_channel(AnimationChannel {
            sampler: AnimationSampler {
                times: vec![0.0, 1.0],
                output: AnimationOutput::Vec3(vec![
                    glam::Vec3::new(0.5, 0.75, 1.25),
                    glam::Vec3::splat(1.5),
                ]),
                interpolation: AnimationInterpolation::Linear,
            },
            target: AnimationTarget::Transform {
                entity: child_a,
                property: TransformProperty::Scale,
            },
        });
        clip.add_channel(AnimationChannel {
            sampler: AnimationSampler {
                times: vec![0.0, 1.0],
                output: AnimationOutput::Vec3(vec![
                    glam::Vec3::new(1.0, -2.5, 3.0),
                    glam::Vec3::new(-1.0, -3.0, 4.0),
                ]),
                interpolation: AnimationInterpolation::Linear,
            },
            target: AnimationTarget::Transform {
                entity: child_b,
                property: TransformProperty::Translation,
            },
        });
        clip.add_channel(AnimationChannel {
            sampler: AnimationSampler {
                times: vec![0.0, 1.0],
                output: AnimationOutput::Quat(vec![
                    glam::Quat::from_rotation_x(0.35),
                    glam::Quat::from_rotation_x(-0.8),
                ]),
                interpolation: AnimationInterpolation::Linear,
            },
            target: AnimationTarget::Transform {
                entity: grandchild,
                property: TransformProperty::Rotation,
            },
        });

        let clip_index = scene.add_animation_clip(clip);
        let _ = scene.play_animation(clip_index, true);

        scene.set_animation_playback(false);

        let mut initial_transforms: std::collections::HashMap<String, Transform> =
            std::collections::HashMap::new();
        {
            let world_ref = scene.main_world();
            for (_entity, (name, transform)) in
                world_ref.query::<(&Name, &TransformComponent)>().iter()
            {
                initial_transforms.insert(name.0.clone(), transform.0);
            }
        }

        let snapshot = SceneSnapshot::capture(&scene);

        scene.set_animation_playback(true);
        scene.update(0.5);

        let mut restored = snapshot.into_scene();
        restored.set_animation_playback(false);
        restored.update(0.0);

        let world = restored.main_world();
        for (_entity, (name, transform)) in world.query::<(&Name, &TransformComponent)>().iter() {
            let expected = initial_transforms
                .get(&name.0)
                .expect("initial transform missing for entity");
            assert!(transform
                .0
                .translation
                .abs_diff_eq(expected.translation, 1e-5));
            assert!(transform.0.rotation.abs_diff_eq(expected.rotation, 1e-5));
            assert!(transform.0.scale.abs_diff_eq(expected.scale, 1e-5));
        }
    }

    #[test]
    fn stopping_preview_restores_rest_pose() {
        use crate::scene::animation::{
            AnimationChannel, AnimationClip, AnimationInterpolation, AnimationOutput,
            AnimationSampler, AnimationTarget, TransformProperty,
        };
        use crate::scene::components::{Name, Visible};

        let mut scene = Scene::new();
        let entity = scene.main_world_mut().spawn((
            Name::new("PreviewEntity"),
            TransformComponent(Transform::from_translation(glam::Vec3::new(0.0, 1.0, 0.0))),
            Visible(true),
        ));

        let mut clip = AnimationClip::new("Translate");
        clip.add_channel(AnimationChannel {
            sampler: AnimationSampler {
                times: vec![0.0, 1.0],
                output: AnimationOutput::Vec3(vec![
                    glam::Vec3::new(0.0, 1.0, 0.0),
                    glam::Vec3::new(5.0, 10.0, -3.0),
                ]),
                interpolation: AnimationInterpolation::Linear,
            },
            target: AnimationTarget::Transform {
                entity,
                property: TransformProperty::Translation,
            },
        });

        let clip_index = scene.add_animation_clip(clip);
        scene.play_animation(clip_index, true);

        let initial_translation = scene
            .main_world()
            .get::<&TransformComponent>(entity)
            .unwrap()
            .0
            .translation;

        scene.set_animation_playback(true);
        scene.update(0.5);

        let animated_translation = scene
            .main_world()
            .get::<&TransformComponent>(entity)
            .unwrap()
            .0
            .translation;
        assert_ne!(animated_translation, initial_translation);

        scene.set_animation_playback(false);
        scene.update(0.0);

        let restored_translation = scene
            .main_world()
            .get::<&TransformComponent>(entity)
            .unwrap()
            .0
            .translation;
        assert!(restored_translation.abs_diff_eq(initial_translation, 1e-5));
    }
}
