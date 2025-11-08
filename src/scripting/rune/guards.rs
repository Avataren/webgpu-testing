use std::cell::RefCell;
use std::rc::Rc;

use hecs::World;
use rune::runtime::VmResult;

use crate::scene::InputState;
use crate::scripting::component_registry::ComponentRegistry;

use super::commands::ScriptCommands;
use super::types::{ScriptEvent, ScriptStateMap};

/// Tracks the active command queue while executing a script.
#[derive(Default)]
pub(crate) struct ActiveCommands(Option<Rc<RefCell<ScriptCommands>>>);

impl ActiveCommands {
    pub fn set(&mut self, commands: Rc<RefCell<ScriptCommands>>) {
        self.0 = Some(commands);
    }

    pub fn clear(&mut self) {
        self.0 = None;
    }

    pub fn with<R>(&mut self, f: impl FnOnce(&mut ScriptCommands) -> VmResult<R>) -> VmResult<R> {
        let rc = match &self.0 {
            Some(rc) => rc.clone(),
            None => return VmResult::panic("script command context missing"),
        };
        let mut guard = rc.borrow_mut();
        f(&mut guard)
    }
}

thread_local! {
    pub(crate) static ACTIVE_COMMANDS: RefCell<ActiveCommands> = RefCell::new(ActiveCommands::default());
    pub(crate) static ACTIVE_STATE: RefCell<Option<Rc<RefCell<ScriptStateMap>>>> = const { RefCell::new(None) };
    pub(crate) static ACTIVE_WORLD: RefCell<Option<*const World>> = const { RefCell::new(None) };
    pub(crate) static ACTIVE_REGISTRY: RefCell<Option<*const ComponentRegistry>> = const { RefCell::new(None) };
    pub(crate) static ACTIVE_INPUT_STATE: RefCell<Option<*const InputState>> = const { RefCell::new(None) };
    pub(crate) static ACTIVE_EVENT_QUEUE: RefCell<Option<Rc<RefCell<Vec<ScriptEvent>>>>> = const { RefCell::new(None) };
    pub(crate) static ACTIVE_ENTITY: RefCell<Option<i64>> = const { RefCell::new(None) };
}

pub(crate) struct CommandGuard;

impl CommandGuard {
    pub fn enter(commands: Rc<RefCell<ScriptCommands>>) -> Self {
        ACTIVE_COMMANDS.with(|cell| cell.borrow_mut().set(commands));
        Self
    }
}

impl Drop for CommandGuard {
    fn drop(&mut self) {
        ACTIVE_COMMANDS.with(|cell| cell.borrow_mut().clear());
    }
}

pub(crate) struct StateGuard {
    // Keep an Rc clone around so the state remains available while the guard
    // exists. We don't hold a RefMut here to avoid double-borrow issues when
    // `with_active_state` borrows the map later.
    _state: Rc<RefCell<ScriptStateMap>>,
}

impl StateGuard {
    pub fn enter(state: &Rc<RefCell<ScriptStateMap>>) -> Self {
        let state_clone = Rc::clone(state);
        ACTIVE_STATE.with(|cell| *cell.borrow_mut() = Some(Rc::clone(&state_clone)));
        Self {
            _state: state_clone,
        }
    }
}

impl Drop for StateGuard {
    fn drop(&mut self) {
        ACTIVE_STATE.with(|cell| *cell.borrow_mut() = None);
    }
}

pub(crate) fn with_active_state<R>(
    f: impl FnOnce(&mut ScriptStateMap) -> VmResult<R>,
) -> VmResult<R> {
    ACTIVE_STATE.with(|cell| {
        let opt = cell.borrow();
        let Some(rc) = opt.as_ref() else {
            return VmResult::panic("state store missing");
        };
        let mut borrow = rc.borrow_mut();
        f(&mut borrow)
    })
}

pub(crate) struct WorldGuard;

impl WorldGuard {
    pub fn enter(world: &World) -> Self {
        let ptr = world as *const World;
        ACTIVE_WORLD.with(|cell| *cell.borrow_mut() = Some(ptr));
        Self
    }
}

impl Drop for WorldGuard {
    fn drop(&mut self) {
        ACTIVE_WORLD.with(|cell| *cell.borrow_mut() = None);
    }
}

pub(crate) struct RegistryGuard;

impl RegistryGuard {
    pub fn enter(registry: &ComponentRegistry) -> Self {
        let ptr = registry as *const ComponentRegistry;
        ACTIVE_REGISTRY.with(|cell| *cell.borrow_mut() = Some(ptr));
        Self
    }
}

impl Drop for RegistryGuard {
    fn drop(&mut self) {
        ACTIVE_REGISTRY.with(|cell| *cell.borrow_mut() = None);
    }
}

pub(crate) struct EventQueueGuard {
    _queue: Rc<RefCell<Vec<ScriptEvent>>>,
}

impl EventQueueGuard {
    pub fn enter(queue: &Rc<RefCell<Vec<ScriptEvent>>>) -> Self {
        let queue_clone = Rc::clone(queue);
        ACTIVE_EVENT_QUEUE.with(|cell| *cell.borrow_mut() = Some(Rc::clone(&queue_clone)));
        Self {
            _queue: queue_clone,
        }
    }
}

impl Drop for EventQueueGuard {
    fn drop(&mut self) {
        ACTIVE_EVENT_QUEUE.with(|cell| *cell.borrow_mut() = None);
    }
}

pub(crate) struct EntityGuard;

impl EntityGuard {
    pub fn enter(entity_bits: i64) -> Self {
        ACTIVE_ENTITY.with(|cell| *cell.borrow_mut() = Some(entity_bits));
        Self
    }
}

impl Drop for EntityGuard {
    fn drop(&mut self) {
        ACTIVE_ENTITY.with(|cell| *cell.borrow_mut() = None);
    }
}

pub(crate) fn with_active_world<R>(f: impl FnOnce(&World) -> VmResult<R>) -> VmResult<R> {
    ACTIVE_WORLD.with(|cell| {
        let opt = cell.borrow();
        let Some(ptr) = *opt else {
            return VmResult::panic("world not available");
        };
        // SAFETY: The World pointer is only set during script execution and cleared after.
        // We control script execution to be single-threaded and non-reentrant.
        let world = unsafe { &*ptr };
        f(world)
    })
}

pub(crate) fn with_active_entity<R>(f: impl FnOnce(i64) -> VmResult<R>) -> VmResult<R> {
    ACTIVE_ENTITY.with(|cell| {
        let opt = cell.borrow();
        let Some(entity_bits) = *opt else {
            return VmResult::panic("entity not available");
        };
        f(entity_bits)
    })
}

pub(crate) fn with_active_registry<R>(
    f: impl FnOnce(&ComponentRegistry) -> VmResult<R>,
) -> VmResult<R> {
    ACTIVE_REGISTRY.with(|cell| {
        let opt = cell.borrow();
        let Some(ptr) = *opt else {
            return VmResult::panic("component registry not available");
        };
        // SAFETY: The ComponentRegistry pointer is only set during script execution and cleared after.
        // We control script execution to be single-threaded and non-reentrant.
        let registry = unsafe { &*ptr };
        f(registry)
    })
}

pub(crate) fn with_active_input_state<R>(
    f: impl FnOnce(&InputState) -> VmResult<R>,
) -> VmResult<R> {
    ACTIVE_INPUT_STATE.with(|cell| {
        let opt = cell.borrow();
        let Some(ptr) = *opt else {
            return VmResult::panic("input state not available");
        };
        // SAFETY: The InputState pointer is only set during script execution and cleared after.
        // We control script execution to be single-threaded and non-reentrant.
        let input_state = unsafe { &*ptr };
        f(input_state)
    })
}

pub(crate) fn with_active_event_queue<R>(
    f: impl FnOnce(&mut Vec<ScriptEvent>) -> VmResult<R>,
) -> VmResult<R> {
    ACTIVE_EVENT_QUEUE.with(|cell| {
        let opt = cell.borrow();
        let Some(rc) = opt.as_ref() else {
            return VmResult::panic("event queue not available");
        };
        let mut borrow = rc.borrow_mut();
        f(&mut borrow)
    })
}

pub(crate) fn get_active_entity() -> VmResult<i64> {
    ACTIVE_ENTITY.with(|cell| {
        let opt = cell.borrow();
        match *opt {
            Some(entity_bits) => VmResult::Ok(entity_bits),
            None => VmResult::panic("active entity not available"),
        }
    })
}
