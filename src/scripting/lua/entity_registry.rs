use std::collections::HashMap;

use glam::{Quat, Vec3};
use hecs::Entity;

use super::types::LuaScriptSource;

#[derive(Debug)]
pub(crate) struct EntityHandleRegistry {
    next_handle: i64,
    pub handles: HashMap<i64, Option<u64>>,
}

impl Default for EntityHandleRegistry {
    fn default() -> Self {
        Self {
            // Start from -1 to avoid collision with entity bits 0
            next_handle: -1,
            handles: HashMap::new(),
        }
    }
}

impl EntityHandleRegistry {
    pub fn allocate(&mut self) -> i64 {
        // Keep decrementing until we find an unused handle
        // This ensures we don't overwrite pre-registered entries (e.g., self_entity)
        while self.handles.contains_key(&self.next_handle) {
            self.next_handle -= 1;
        }
        let handle = self.next_handle;
        self.next_handle -= 1;
        self.handles.insert(handle, None);
        handle
    }

    pub fn resolve(&mut self, handle: i64, entity: Entity) {
        self.handles.insert(handle, Some(entity.to_bits().get()));
    }

    /// Directly resolve a handle to entity bits without needing an Entity object.
    /// This is useful for pre-registering entity_bits to prevent "not yet available" errors.
    pub fn resolve_bits(&mut self, handle: i64, bits: u64) {
        self.handles.insert(handle, Some(bits));
    }

    pub fn resolved_bits(&self, handle: i64) -> Option<u64> {
        self.handles.get(&handle).and_then(|bits| *bits)
    }

    pub fn contains(&self, handle: i64) -> bool {
        self.handles.contains_key(&handle)
    }
}

#[derive(Default)]
pub(crate) struct PendingEntity {
    pub name: Option<String>,
    pub translation: Option<Vec3>,
    pub rotation: Option<Quat>,
    pub scale: Option<Vec3>,
    pub parent: Option<i64>,
    pub scripts: Vec<LuaScriptSource>,
    pub components: HashMap<String, serde_json::Value>,
}

pub(crate) enum ExistingCommand {
    SetName {
        entity_bits: u64,
        name: String,
    },
    SetParent {
        entity_bits: u64,
        parent_bits: Option<u64>,
    },
    Translate {
        entity_bits: u64,
        delta: Vec3,
    },
    SetTranslation {
        entity_bits: u64,
        position: Vec3,
    },
    Rotate {
        entity_bits: u64,
        axis: Vec3,
        angle: f32,
    },
    SetRotation {
        entity_bits: u64,
        rotation: Quat,
    },
    SetScale {
        entity_bits: u64,
        scale: Vec3,
    },
    LookAt {
        entity_bits: u64,
        target: Vec3,
    },
    AttachScript {
        entity_bits: u64,
        source: LuaScriptSource,
    },
    ImportGltf {
        entity_bits: u64,
        path: String,
        scale: f32,
    },
    SetComponent {
        entity_bits: u64,
        component_name: String,
        #[allow(dead_code)]
        value: serde_json::Value,
    },
    AddComponent {
        entity_bits: u64,
        component_name: String,
        #[allow(dead_code)]
        value: serde_json::Value,
    },
    RemoveComponent {
        entity_bits: u64,
        component_name: String,
    },
    SubscribeEvent {
        entity_bits: u64,
        event_name: String,
        callback_name: String,
    },
    UnsubscribeEvent {
        entity_bits: u64,
        event_name: String,
    },
}
