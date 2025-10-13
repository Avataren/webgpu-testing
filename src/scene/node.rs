use super::asset::SerializedTransform;
use super::instance::SceneInstance;
use super::transform::Transform;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SceneNodeId(pub(crate) u32);

impl SceneNodeId {
    pub(crate) fn index(self) -> usize {
        self.0 as usize
    }
}

pub(super) struct SceneNode {
    name: String,
    parent: Option<SceneNodeId>,
    children: Vec<SceneNodeId>,
    local_transform: Transform,
    world_transform: Transform,
    instance: SceneInstance,
}

impl SceneNode {
    pub(super) fn new(_id: SceneNodeId, name: impl Into<String>, instance: SceneInstance) -> Self {
        Self {
            name: name.into(),
            parent: None,
            children: Vec::new(),
            local_transform: Transform::IDENTITY,
            world_transform: Transform::IDENTITY,
            instance,
        }
    }

    pub(super) fn name(&self) -> &str {
        &self.name
    }

    pub(super) fn set_name(&mut self, name: impl Into<String>) {
        self.name = name.into();
    }

    pub(super) fn parent(&self) -> Option<SceneNodeId> {
        self.parent
    }

    pub(super) fn set_parent(&mut self, parent: Option<SceneNodeId>) {
        self.parent = parent;
    }

    pub(super) fn children(&self) -> &[SceneNodeId] {
        &self.children
    }

    pub(super) fn add_child(&mut self, child: SceneNodeId) {
        self.children.push(child);
    }

    pub(super) fn remove_child(&mut self, child: SceneNodeId) {
        if let Some(index) = self.children.iter().position(|&id| id == child) {
            self.children.swap_remove(index);
        }
    }

    pub(super) fn local_transform(&self) -> &Transform {
        &self.local_transform
    }

    pub(super) fn local_transform_mut(&mut self) -> &mut Transform {
        &mut self.local_transform
    }

    pub(super) fn set_local_transform(&mut self, transform: Transform) {
        self.local_transform = transform;
    }

    pub(super) fn world_transform(&self) -> &Transform {
        &self.world_transform
    }

    pub(super) fn set_world_transform(&mut self, transform: Transform) {
        self.world_transform = transform;
    }

    pub(super) fn instance(&self) -> &SceneInstance {
        &self.instance
    }

    pub(super) fn instance_mut(&mut self) -> &mut SceneInstance {
        &mut self.instance
    }

    pub(super) fn world(&self) -> &hecs::World {
        self.instance.world()
    }

    pub(super) fn propagate_transforms(&mut self) {
        self.instance.propagate_transforms();
    }

    pub(super) fn take_children(&self) -> Vec<SceneNodeId> {
        self.children.clone()
    }

    pub(super) fn serialized_transform(&self) -> SerializedTransform {
        SerializedTransform::from(self.local_transform)
    }
}
