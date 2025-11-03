use super::animation::{AnimationClip, AnimationState};
use super::internal::{animations, transforms};
use crate::asset::Assets;
use crate::scene::components::TransformComponent;
use crate::scene::transform::Transform;
use hecs::{Entity, World};
use slotmap::new_key_type;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::ops::{Deref, DerefMut};

new_key_type! {
    pub struct SceneNodeId;
}

pub(crate) struct SceneNode {
    name: String,
    parent: Option<SceneNodeId>,
    children: Vec<SceneNodeId>,
    local_transform: Transform,
    world_transform: Transform,
    instance: SceneInstance,
    prefab: PrefabInstanceInfo,
    prefab_origin: Option<PrefabOriginMetadata>,
}

impl SceneNode {
    pub(crate) fn new(name: impl Into<String>, instance: SceneInstance) -> Self {
        Self {
            name: name.into(),
            parent: None,
            children: Vec::new(),
            local_transform: Transform::IDENTITY,
            world_transform: Transform::IDENTITY,
            instance,
            prefab: PrefabInstanceInfo::new(),
            prefab_origin: None,
        }
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    pub(crate) fn parent(&self) -> Option<SceneNodeId> {
        self.parent
    }

    pub(crate) fn set_parent(&mut self, parent: Option<SceneNodeId>) {
        self.parent = parent;
    }

    pub(crate) fn children(&self) -> &[SceneNodeId] {
        &self.children
    }

    pub(crate) fn add_child(&mut self, child: SceneNodeId) {
        self.children.push(child);
    }

    pub(crate) fn remove_child(&mut self, child: SceneNodeId) {
        if let Some(index) = self.children.iter().position(|&id| id == child) {
            self.children.swap_remove(index);
        }
    }

    pub(crate) fn local_transform(&self) -> &Transform {
        &self.local_transform
    }

    pub(crate) fn local_transform_mut(&mut self) -> SceneNodeLocalTransformMut<'_> {
        SceneNodeLocalTransformMut::new(&mut self.local_transform, &mut self.prefab)
    }

    pub(crate) fn set_local_transform(&mut self, transform: Transform) {
        self.prefab.set_local_baseline(transform);
        self.local_transform = transform;
    }

    pub(crate) fn clear_local_transform_override(&mut self) {
        self.local_transform = self.prefab.baseline_local_transform();
        self.prefab.clear_local_override();
    }

    pub(crate) fn world_transform(&self) -> &Transform {
        &self.world_transform
    }

    pub(crate) fn set_world_transform(&mut self, transform: Transform) {
        self.world_transform = transform;
    }

    pub(crate) fn instance(&self) -> &SceneInstance {
        &self.instance
    }

    pub(crate) fn instance_mut(&mut self) -> &mut SceneInstance {
        &mut self.instance
    }

    pub(crate) fn prefab_origin(&self) -> Option<&PrefabOriginMetadata> {
        self.prefab_origin.as_ref()
    }

    pub(crate) fn set_prefab_origin(&mut self, origin: Option<PrefabOriginMetadata>) {
        self.prefab_origin = origin;
    }
}

pub(crate) struct SceneInstance {
    world: World,
    animations: Vec<AnimationClip>,
    animation_states: Vec<AnimationState>,
    rest_pose: Option<HashMap<hecs::Entity, Transform>>,
    active_camera: Option<Entity>,
    prefab_overrides: PrefabOverrideState,
    asset_entity_map: Vec<Option<Entity>>,
}

impl SceneInstance {
    pub(crate) fn new() -> Self {
        Self {
            world: World::new(),
            animations: Vec::new(),
            animation_states: Vec::new(),
            rest_pose: None,
            active_camera: None,
            prefab_overrides: PrefabOverrideState::default(),
            asset_entity_map: Vec::new(),
        }
    }

    pub(crate) fn update(&mut self, assets: &mut Assets, dt: f64, absolute_time: f64) {
        let world = &mut self.world;
        let animations = &self.animations;
        let animation_states = &mut self.animation_states;

        animations::advance_animations(world, assets, animations, animation_states, dt);
        animations::update_rotate_animations(world, dt);
        animations::update_orbit_animations(world, absolute_time);
    }

    pub(crate) fn propagate_transforms(&mut self) {
        transforms::propagate_transforms(&mut self.world);
    }

    pub(crate) fn add_animation_clip(&mut self, clip: AnimationClip) -> usize {
        let index = self.animations.len();
        self.animations.push(clip);
        index
    }

    pub(crate) fn play_animation(&mut self, clip_index: usize, looping: bool) -> Option<usize> {
        if clip_index >= self.animations.len() {
            return None;
        }

        self.capture_rest_pose();

        let mut state = AnimationState::new(clip_index);
        state.looping = looping;
        let index = self.animation_states.len();
        self.animation_states.push(state);
        Some(index)
    }

    pub(crate) fn animations(&self) -> &[AnimationClip] {
        &self.animations
    }

    pub(crate) fn animation_states(&self) -> &[AnimationState] {
        &self.animation_states
    }

    pub(crate) fn animation_states_mut(&mut self) -> &mut [AnimationState] {
        &mut self.animation_states
    }

    pub(crate) fn set_animation_data(
        &mut self,
        animations: Vec<AnimationClip>,
        animation_states: Vec<AnimationState>,
    ) {
        self.animations = animations;
        self.animation_states = animation_states;
    }

    pub(crate) fn take_animation_data(&mut self) -> (Vec<AnimationClip>, Vec<AnimationState>) {
        (
            std::mem::take(&mut self.animations),
            std::mem::take(&mut self.animation_states),
        )
    }

    pub(crate) fn begin_playback(&mut self) {
        self.rest_pose = None;
        self.capture_rest_pose();
    }

    pub(crate) fn capture_rest_pose(&mut self) {
        if let Some(rest) = self.rest_pose.as_mut() {
            for (entity, transform) in self.world.query::<&TransformComponent>().iter() {
                rest.entry(entity).or_insert(transform.0);
            }
            return;
        }

        let mut rest = HashMap::new();
        for (entity, transform) in self.world.query::<&TransformComponent>().iter() {
            rest.insert(entity, transform.0);
        }
        self.rest_pose = Some(rest);
    }

    pub(crate) fn restore_rest_pose(&mut self) {
        let Some(rest) = self.rest_pose.take() else {
            return;
        };

        for (entity, transform) in rest {
            if let Ok(mut component) = self.world.get::<&mut TransformComponent>(entity) {
                component.0 = transform;
            }
        }

        self.prefab_overrides.reapply(&mut self.world);
        self.propagate_transforms();
    }

    pub(crate) fn push_animation_state(&mut self, state: AnimationState) -> Option<usize> {
        if state.clip_index >= self.animations.len() {
            return None;
        }

        if state.playing {
            self.capture_rest_pose();
        }

        let index = self.animation_states.len();
        self.animation_states.push(state);
        Some(index)
    }

    pub(crate) fn world(&self) -> &World {
        &self.world
    }

    pub(crate) fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    pub(crate) fn set_asset_entity_map(&mut self, map: Vec<Entity>) {
        self.asset_entity_map = map.into_iter().map(Some).collect();
    }

    pub(crate) fn asset_entity(&self, index: usize) -> Option<Entity> {
        self.asset_entity_map.get(index).copied().flatten()
    }

    pub(crate) fn asset_entity_count(&self) -> usize {
        self.asset_entity_map.len()
    }

    pub(crate) fn clear_asset_entity(&mut self, index: usize) {
        if let Some(entry) = self.asset_entity_map.get_mut(index) {
            *entry = None;
        }
    }

    pub(crate) fn mutate_entity_transform(
        &mut self,
        entity: Entity,
        transform: Transform,
    ) -> Result<(), hecs::ComponentError> {
        let baseline = {
            let component = self.world.get::<&TransformComponent>(entity)?;
            component.0
        };

        {
            let mut component = self.world.get::<&mut TransformComponent>(entity)?;
            component.0 = transform;
        }

        self.prefab_overrides
            .set_transform_override(entity, baseline, transform);
        Ok(())
    }

    pub(crate) fn clear_entity_transform_override(
        &mut self,
        entity: Entity,
    ) -> Result<(), hecs::ComponentError> {
        if let Some(baseline) = self.prefab_overrides.clear_transform_override(entity) {
            if let Ok(mut component) = self.world.get::<&mut TransformComponent>(entity) {
                component.0 = baseline;
            }
        }

        Ok(())
    }

    pub(crate) fn mutate_component<T>(
        &mut self,
        entity: Entity,
        value: T,
    ) -> Result<(), hecs::ComponentError>
    where
        T: Clone + PartialEq + Send + Sync + 'static,
    {
        let baseline = {
            let component = self.world.get::<&T>(entity)?;
            (*component).clone()
        };

        {
            let mut component = self.world.get::<&mut T>(entity)?;
            *component = value.clone();
        }

        self.prefab_overrides
            .set_component_override(entity, baseline, value);
        Ok(())
    }

    pub(crate) fn clear_component_override<T>(
        &mut self,
        entity: Entity,
    ) -> Result<(), hecs::ComponentError>
    where
        T: Clone + Send + Sync + 'static,
    {
        if let Some(entry) = self.prefab_overrides.remove_component_override::<T>(entity) {
            entry.apply_baseline(&mut self.world, entity);
        }

        Ok(())
    }

    pub(crate) fn reapply_prefab_overrides(&mut self) {
        self.prefab_overrides.reapply(&mut self.world);
    }

    pub(crate) fn active_camera(&self) -> Option<Entity> {
        self.active_camera
    }

    pub(crate) fn set_active_camera(&mut self, camera: Option<Entity>) {
        self.active_camera = camera;
    }

    pub(crate) fn into_world(self) -> World {
        self.world
    }
}

#[derive(Debug, Clone)]
pub(crate) struct PrefabInstanceInfo {
    baseline_local_transform: Transform,
    local_override: Option<Transform>,
}

impl PrefabInstanceInfo {
    pub(crate) fn new() -> Self {
        Self {
            baseline_local_transform: Transform::IDENTITY,
            local_override: None,
        }
    }

    pub(crate) fn baseline_local_transform(&self) -> Transform {
        self.baseline_local_transform
    }

    pub(crate) fn set_local_baseline(&mut self, transform: Transform) {
        self.baseline_local_transform = transform;
        if self.local_override == Some(transform) {
            self.local_override = None;
        }
    }

    pub(crate) fn set_local_override(&mut self, transform: Transform) {
        if transform == self.baseline_local_transform {
            self.local_override = None;
        } else {
            self.local_override = Some(transform);
        }
    }

    pub(crate) fn clear_local_override(&mut self) {
        self.local_override = None;
    }
}

#[derive(Clone, Debug)]
pub(crate) struct PrefabOriginMetadata {
    document_id: String,
    entity_paths: Vec<Option<Vec<String>>>,
}

impl PrefabOriginMetadata {
    pub(crate) fn new(
        document_id: impl Into<String>,
        entity_paths: Vec<Option<Vec<String>>>,
    ) -> Self {
        Self {
            document_id: document_id.into(),
            entity_paths,
        }
    }

    pub(crate) fn document_id(&self) -> &str {
        &self.document_id
    }

    pub(crate) fn entity_paths(&self) -> &[Option<Vec<String>>] {
        &self.entity_paths
    }
}

trait DynValue: Any {
    fn as_any(&self) -> &dyn Any;
}

impl<T> DynValue for T
where
    T: Any + Clone,
{
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Default)]
pub(crate) struct PrefabOverrideState {
    entity_transforms: HashMap<Entity, TransformOverride>,
    entity_components: HashMap<Entity, HashMap<TypeId, ComponentOverrideEntry>>,
}

impl PrefabOverrideState {
    fn set_transform_override(&mut self, entity: Entity, baseline: Transform, value: Transform) {
        let mut remove = false;

        self.entity_transforms
            .entry(entity)
            .and_modify(|entry| {
                entry.set_override(value);
                remove = !entry.has_override();
            })
            .or_insert_with(|| {
                let mut entry = TransformOverride::new(baseline);
                entry.set_override(value);
                if !entry.has_override() {
                    remove = true;
                }
                entry
            });

        if remove {
            self.entity_transforms.remove(&entity);
        }
    }

    fn clear_transform_override(&mut self, entity: Entity) -> Option<Transform> {
        self.entity_transforms
            .remove(&entity)
            .map(|entry| entry.baseline())
    }

    fn set_component_override<T>(&mut self, entity: Entity, baseline: T, value: T)
    where
        T: Clone + PartialEq + Send + Sync + 'static,
    {
        let map = self.entity_components.entry(entity).or_default();
        let type_id = TypeId::of::<T>();
        let mut remove_entry = false;

        map.entry(type_id)
            .and_modify(|entry| {
                entry.set_override(value.clone());
                remove_entry = !entry.has_override();
            })
            .or_insert_with(|| {
                let mut entry = ComponentOverrideEntry::new::<T>(baseline.clone());
                entry.set_override(value);
                if !entry.has_override() {
                    remove_entry = true;
                }
                entry
            });

        if remove_entry {
            map.remove(&type_id);
        }

        if map.is_empty() {
            self.entity_components.remove(&entity);
        }
    }

    fn remove_component_override<T>(&mut self, entity: Entity) -> Option<ComponentOverrideEntry>
    where
        T: Clone + Send + Sync + 'static,
    {
        let type_id = TypeId::of::<T>();
        let map = self.entity_components.get_mut(&entity)?;
        let entry = map.remove(&type_id);
        if map.is_empty() {
            self.entity_components.remove(&entity);
        }
        entry
    }

    fn reapply(&self, world: &mut World) {
        for (entity, entry) in &self.entity_transforms {
            if let Some(value) = entry.override_value() {
                if let Ok(mut component) = world.get::<&mut TransformComponent>(*entity) {
                    component.0 = value;
                }
            }
        }

        for (entity, components) in &self.entity_components {
            for entry in components.values() {
                if entry.has_override() {
                    entry.apply(world, *entity);
                }
            }
        }
    }
}

struct ComponentOverrideEntry {
    type_id: TypeId,
    baseline: Box<dyn DynValue>,
    override_value: Option<Box<dyn DynValue>>,
    apply_fn: fn(&mut World, Entity, &dyn DynValue),
}

impl ComponentOverrideEntry {
    fn new<T>(baseline: T) -> Self
    where
        T: Clone + Send + Sync + 'static,
    {
        Self {
            type_id: TypeId::of::<T>(),
            baseline: Box::new(baseline),
            override_value: None,
            apply_fn: apply_component_override::<T>,
        }
    }

    fn set_override<T>(&mut self, value: T)
    where
        T: Clone + PartialEq + Send + Sync + 'static,
    {
        debug_assert_eq!(self.type_id, TypeId::of::<T>());
        let baseline = self
            .baseline
            .as_any()
            .downcast_ref::<T>()
            .expect("component baseline type mismatch");

        if &value == baseline {
            self.override_value = None;
        } else {
            self.override_value = Some(Box::new(value));
        }
    }

    fn has_override(&self) -> bool {
        self.override_value.is_some()
    }

    fn apply(&self, world: &mut World, entity: Entity) {
        if let Some(value) = self.override_value.as_ref() {
            (self.apply_fn)(world, entity, value.as_ref());
        }
    }

    fn apply_baseline(&self, world: &mut World, entity: Entity) {
        (self.apply_fn)(world, entity, self.baseline.as_ref());
    }
}

fn apply_component_override<T>(world: &mut World, entity: Entity, value: &dyn DynValue)
where
    T: Clone + Send + Sync + 'static,
{
    let Some(value) = value.as_any().downcast_ref::<T>() else {
        return;
    };

    let clone = value.clone();
    if let Ok(mut component) = world.get::<&mut T>(entity) {
        *component = clone;
        return;
    }

    let _ = world.insert_one(entity, clone);
}

#[derive(Clone, Copy)]
struct TransformOverride {
    baseline: Transform,
    override_value: Option<Transform>,
}

impl TransformOverride {
    fn new(baseline: Transform) -> Self {
        Self {
            baseline,
            override_value: None,
        }
    }

    fn set_override(&mut self, value: Transform) {
        if value == self.baseline {
            self.override_value = None;
        } else {
            self.override_value = Some(value);
        }
    }

    fn has_override(&self) -> bool {
        self.override_value.is_some()
    }

    fn override_value(&self) -> Option<Transform> {
        self.override_value
    }

    fn baseline(&self) -> Transform {
        self.baseline
    }
}

pub struct SceneNodeLocalTransformMut<'a> {
    transform: &'a mut Transform,
    prefab: &'a mut PrefabInstanceInfo,
}

impl<'a> SceneNodeLocalTransformMut<'a> {
    fn new(transform: &'a mut Transform, prefab: &'a mut PrefabInstanceInfo) -> Self {
        Self { transform, prefab }
    }
}

impl Deref for SceneNodeLocalTransformMut<'_> {
    type Target = Transform;

    fn deref(&self) -> &Self::Target {
        self.transform
    }
}

impl DerefMut for SceneNodeLocalTransformMut<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.transform
    }
}

impl Drop for SceneNodeLocalTransformMut<'_> {
    fn drop(&mut self) {
        let current = *self.transform;
        if current == self.prefab.baseline_local_transform() {
            self.prefab.clear_local_override();
        } else {
            self.prefab.set_local_override(current);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::components::{TransformComponent, Visible};

    #[test]
    fn capture_rest_pose_extends_to_new_entities() {
        let mut instance = SceneInstance::new();

        let entity_a =
            instance
                .world_mut()
                .spawn((TransformComponent(Transform::from_translation(
                    glam::Vec3::new(1.0, 0.0, 0.0),
                )),));

        instance.capture_rest_pose();

        let entity_b =
            instance
                .world_mut()
                .spawn((TransformComponent(Transform::from_translation(
                    glam::Vec3::new(-2.0, 0.0, 0.0),
                )),));

        instance.capture_rest_pose();

        {
            let mut transform_a = instance
                .world_mut()
                .get::<&mut TransformComponent>(entity_a)
                .unwrap();
            transform_a.0.translation = glam::Vec3::new(5.0, 5.0, 5.0);
        }

        {
            let mut transform_b = instance
                .world_mut()
                .get::<&mut TransformComponent>(entity_b)
                .unwrap();
            transform_b.0.translation = glam::Vec3::new(-8.0, 1.0, 3.0);
        }

        instance.restore_rest_pose();

        let restored_a = instance
            .world()
            .get::<&TransformComponent>(entity_a)
            .unwrap()
            .0;
        let restored_b = instance
            .world()
            .get::<&TransformComponent>(entity_b)
            .unwrap()
            .0;

        assert!(restored_a
            .translation
            .abs_diff_eq(glam::Vec3::new(1.0, 0.0, 0.0), 1e-5));
        assert!(restored_b
            .translation
            .abs_diff_eq(glam::Vec3::new(-2.0, 0.0, 0.0), 1e-5));
    }

    #[test]
    fn begin_playback_refreshes_existing_rest_pose() {
        let mut instance = SceneInstance::new();
        let entity = instance
            .world_mut()
            .spawn((TransformComponent(Transform::from_translation(
                glam::Vec3::new(1.0, 2.0, 3.0),
            )),));

        instance.capture_rest_pose();

        {
            let mut transform = instance
                .world_mut()
                .get::<&mut TransformComponent>(entity)
                .unwrap();
            transform.0.translation = glam::Vec3::new(-4.0, 5.0, -6.0);
        }

        instance.begin_playback();

        {
            let mut transform = instance
                .world_mut()
                .get::<&mut TransformComponent>(entity)
                .unwrap();
            transform.0.translation = glam::Vec3::new(42.0, 0.0, 0.0);
        }

        instance.restore_rest_pose();

        let restored = instance
            .world()
            .get::<&TransformComponent>(entity)
            .unwrap()
            .0;

        assert!(restored
            .translation
            .abs_diff_eq(glam::Vec3::new(-4.0, 5.0, -6.0), 1e-5));
    }

    #[test]
    fn node_local_transform_overrides_track_and_clear() {
        let mut node = SceneNode::new("Test", SceneInstance::new());
        let baseline = Transform::from_translation(glam::Vec3::new(1.0, 0.0, 0.0));
        node.set_local_transform(baseline);

        {
            let mut local = node.local_transform_mut();
            local.translation = glam::Vec3::new(4.0, 0.0, 0.0);
        }

        assert_eq!(
            node.local_transform().translation,
            glam::Vec3::new(4.0, 0.0, 0.0)
        );

        node.clear_local_transform_override();

        assert_eq!(node.local_transform().translation, baseline.translation);
    }

    #[test]
    fn overrides_persist_through_restore_and_clear() {
        let mut instance = SceneInstance::new();
        let entity = instance
            .world_mut()
            .spawn((TransformComponent(Transform::IDENTITY), Visible(true)));

        instance.capture_rest_pose();

        instance
            .mutate_entity_transform(
                entity,
                Transform::from_translation(glam::Vec3::new(3.0, 0.0, 0.0)),
            )
            .unwrap();
        instance.mutate_component(entity, Visible(false)).unwrap();

        {
            let mut visible = instance.world_mut().get::<&mut Visible>(entity).unwrap();
            visible.0 = true;
        }

        instance.reapply_prefab_overrides();

        instance.restore_rest_pose();

        let transform = instance
            .world()
            .get::<&TransformComponent>(entity)
            .unwrap()
            .0;
        assert_eq!(transform.translation, glam::Vec3::new(3.0, 0.0, 0.0));

        {
            let visible = instance.world().get::<&Visible>(entity).unwrap();
            assert!(!visible.0);
        }

        instance.clear_entity_transform_override(entity).unwrap();
        instance
            .clear_component_override::<Visible>(entity)
            .unwrap();

        instance.reapply_prefab_overrides();

        instance.restore_rest_pose();

        let transform = instance
            .world()
            .get::<&TransformComponent>(entity)
            .unwrap()
            .0;
        assert_eq!(transform.translation, glam::Vec3::ZERO);

        {
            let visible = instance.world().get::<&Visible>(entity).unwrap();
            assert!(visible.0);
        }
    }
}
