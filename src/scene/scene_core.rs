use super::animation::{AnimationClip, AnimationState};
use super::assets::{
    build_tree_asset_node, serialize_world, SceneAsset, SceneTreeAsset, SceneTreeAssetNode,
    SerializedAnimationClip, SerializedTransform,
};
use super::graph::{SceneInstance, SceneNode, SceneNodeId};
use super::internal::{lights, rendering};
use crate::asset::Assets;
use crate::environment::Environment;
use crate::renderer::{CustomRenderRequest, RenderBatcher, Renderer};
use crate::scene::transform::Transform;
use crate::scene::Camera;
use crate::scripting::ScriptingState;
use crate::time::Instant;
use hecs::World;
use log::error;
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

    pub fn time(&self) -> f64 {
        self.time
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

        {
            let mut scripting = std::mem::take(&mut self.scripting);
            if let Err(err) = scripting.update_scripts(self.main_world_mut(), dt) {
                error!("Rune scripting error: {err}");
            }
            self.scripting = scripting;
        }
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
    ) -> Result<crate::renderer::RenderFrame, wgpu::SurfaceError> {
        batcher.clear();
        let camera_vectors = rendering::CameraVectors::from_renderer(renderer);

        for node in self.nodes_iter() {
            let world = node.instance().world();
            let world_transform = *node.world_transform();

            for mut object in rendering::build_render_objects(world, camera_vectors).into_iter() {
                object.transform = world_transform.mul_transform(&object.transform);
                batcher.add(object);
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
            let instance = asset.instantiate();
            super::internal::composition::merge_world_as_child(
                self.main_world_mut(),
                parent_entity,
                instance.into_world(),
            );
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
    use super::super::components::{Children, Name, Parent, TransformComponent, Visible};
    use super::*;
    use glam::Vec3;

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
}
