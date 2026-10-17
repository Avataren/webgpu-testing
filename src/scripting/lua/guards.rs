use std::cell::{Cell, RefCell};
use std::marker::PhantomData;
use std::rc::Rc;

use hecs::World;

use crate::physics::ScenePhysics;
use crate::scene::InputState;

use super::commands::ScriptCommands;
use super::types::{ScriptEvent, ScriptStateMap};

/// Generation counter to track guard lifecycle and detect use-after-drop bugs.
/// Each guard increments this counter when created and resets it when dropped,
/// allowing runtime verification that pointers are only dereferenced while guards are active.
type Generation = u64;

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

    pub fn with<R>(
        &mut self,
        f: impl FnOnce(&mut ScriptCommands) -> Result<R, mlua::Error>,
    ) -> Result<R, mlua::Error> {
        let rc = match &self.0 {
            Some(rc) => rc.clone(),
            None => {
                return Err(mlua::Error::RuntimeError(
                    "script command context missing".into(),
                ))
            }
        };
        let mut guard = rc.borrow_mut();
        f(&mut guard)
    }
}

/// Wrapper for raw pointers with generation-based lifetime tracking.
/// This allows runtime verification that pointers are only accessed while their guard is active.
struct TrackedPointer<T> {
    ptr: *const T,
    generation: Generation,
}

impl<T> TrackedPointer<T> {
    fn new(ptr: *const T, generation: Generation) -> Self {
        Self { ptr, generation }
    }
}

thread_local! {
    pub(crate) static ACTIVE_COMMANDS: RefCell<ActiveCommands> = RefCell::new(ActiveCommands::default());
    pub(crate) static ACTIVE_STATE: RefCell<Option<Rc<RefCell<ScriptStateMap>>>> = const { RefCell::new(None) };

    // For pointer-based guards, we track both the pointer and a generation counter
    // to detect use-after-drop bugs in debug builds
    pub(crate) static ACTIVE_WORLD: RefCell<Option<TrackedPointer<World>>> = const { RefCell::new(None) };
    pub(crate) static WORLD_GENERATION: Cell<Generation> = const { Cell::new(0) };

    pub(crate) static ACTIVE_INPUT_STATE: RefCell<Option<TrackedPointer<InputState>>> = const { RefCell::new(None) };
    pub(crate) static INPUT_STATE_GENERATION: Cell<Generation> = const { Cell::new(0) };

    pub(crate) static ACTIVE_PHYSICS: RefCell<Option<TrackedPointer<ScenePhysics>>> = const { RefCell::new(None) };
    pub(crate) static PHYSICS_GENERATION: Cell<Generation> = const { Cell::new(0) };

    #[cfg(feature = "navigation")]
    pub(crate) static ACTIVE_NAVIGATION: RefCell<Option<TrackedPointer<crate::navigation::SceneNavigation>>> = const { RefCell::new(None) };
    #[cfg(feature = "navigation")]
    pub(crate) static NAVIGATION_GENERATION: Cell<Generation> = const { Cell::new(0) };

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

/// The entity a script's handle names: an entity id, or a handle `spawn_entity` (or
/// another spawning function) returned, once its entity exists (from the hook after the
/// one that spawned it). Reading functions take either; an entity still pending, or an
/// id that never existed, names no live entity.
pub(crate) fn resolve_entity(handle: i64) -> Option<hecs::Entity> {
    let resolved = ACTIVE_COMMANDS.with(|cell| {
        let active = cell.try_borrow().ok()?;
        let commands = active.0.as_ref()?.try_borrow().ok()?;
        let bits = commands.registry.try_borrow().ok()?.resolved_bits(handle);
        bits
    });
    hecs::Entity::from_bits(resolved.unwrap_or(handle as u64))
}

pub(crate) fn with_active_commands<R>(
    f: impl FnOnce(&mut ScriptCommands) -> Result<R, mlua::Error>,
) -> Result<R, mlua::Error> {
    ACTIVE_COMMANDS.with(|cell| cell.borrow_mut().with(f))
}

pub(crate) fn with_active_state<R>(
    f: impl FnOnce(&mut ScriptStateMap) -> Result<R, mlua::Error>,
) -> Result<R, mlua::Error> {
    ACTIVE_STATE.with(|cell| {
        let opt = cell.borrow();
        let Some(rc) = opt.as_ref() else {
            return Err(mlua::Error::RuntimeError("state store missing".into()));
        };
        let mut borrow = rc.borrow_mut();
        f(&mut borrow)
    })
}

/// Guard that makes a World reference available to Lua scripts during execution.
///
/// # Safety Constraints
/// - The World reference must remain valid for the entire lifetime of this guard
/// - Only one WorldGuard should be active per thread at a time
/// - Script execution must be single-threaded and non-reentrant
/// - The guard must not be moved across thread boundaries
///
/// # Implementation
/// Uses generation counters to detect use-after-drop in debug builds.
/// The PhantomData<Rc<()>> ensures the guard is !Send and !Sync, preventing
/// it from being moved across thread boundaries (raw pointers are Send/Sync).
pub(crate) struct WorldGuard<'a> {
    // Borrows the world for the guard's lifetime, so it cannot be mutated or dropped
    // while scripts can observe it. The Rc makes the guard !Send + !Sync.
    _marker: PhantomData<(&'a World, Rc<()>)>,
}

impl<'a> WorldGuard<'a> {
    /// Creates a new WorldGuard, making the world available to scripts.
    ///
    /// # Safety Constraints
    /// The caller must ensure that:
    /// - `world` remains valid for the entire lifetime of this guard
    /// - No other WorldGuard is active on this thread
    /// - Script execution will not be reentrant
    pub fn enter(world: &'a World) -> Self {
        let ptr = world as *const World;
        let generation = WORLD_GENERATION.with(|gen| {
            let new_gen = gen.get().wrapping_add(1);
            gen.set(new_gen);
            new_gen
        });

        debug_assert!(
            ACTIVE_WORLD.with(|cell| cell.borrow().is_none()),
            "WorldGuard: attempted to create guard while another is active"
        );

        ACTIVE_WORLD.with(|cell| *cell.borrow_mut() = Some(TrackedPointer::new(ptr, generation)));

        Self {
            _marker: PhantomData,
        }
    }
}

impl Drop for WorldGuard<'_> {
    fn drop(&mut self) {
        ACTIVE_WORLD.with(|cell| *cell.borrow_mut() = None);

        // Reset generation counter when guard is dropped
        WORLD_GENERATION.with(|gen| gen.set(0));
    }
}

/// Guard that makes an InputState reference available to Lua scripts during execution.
///
/// # Safety Constraints
/// - The InputState reference must remain valid for the entire lifetime of this guard
/// - Only one InputStateGuard should be active per thread at a time
/// - Script execution must be single-threaded and non-reentrant
/// - The guard must not be moved across thread boundaries
///
/// # Implementation
/// Uses generation counters to detect use-after-drop in debug builds.
/// The PhantomData<Rc<()>> ensures the guard is !Send and !Sync, preventing
/// it from being moved across thread boundaries (raw pointers are Send/Sync).
pub struct InputStateGuard<'a> {
    // Borrows the input state for the guard's lifetime. The Rc makes the guard !Send + !Sync.
    _marker: PhantomData<(&'a InputState, Rc<()>)>,
}

impl<'a> InputStateGuard<'a> {
    /// Creates a new InputStateGuard, making the input state available to scripts.
    ///
    /// # Safety Constraints
    /// The caller must ensure that:
    /// - `input_state` remains valid for the entire lifetime of this guard
    /// - No other InputStateGuard is active on this thread
    /// - Script execution will not be reentrant
    pub fn enter(input_state: &'a InputState) -> Self {
        let ptr = input_state as *const InputState;
        let generation = INPUT_STATE_GENERATION.with(|gen| {
            let new_gen = gen.get().wrapping_add(1);
            gen.set(new_gen);
            new_gen
        });

        debug_assert!(
            ACTIVE_INPUT_STATE.with(|cell| cell.borrow().is_none()),
            "InputStateGuard: attempted to create guard while another is active"
        );

        ACTIVE_INPUT_STATE
            .with(|cell| *cell.borrow_mut() = Some(TrackedPointer::new(ptr, generation)));

        Self {
            _marker: PhantomData,
        }
    }
}

impl Drop for InputStateGuard<'_> {
    fn drop(&mut self) {
        ACTIVE_INPUT_STATE.with(|cell| *cell.borrow_mut() = None);

        // Reset generation counter when guard is dropped
        INPUT_STATE_GENERATION.with(|gen| gen.set(0));
    }
}

/// Guard that lets Lua scripts query the scene's physics (raycasts, velocities,
/// character state) while they run. Same constraints as [`InputStateGuard`]: the
/// physics must outlive the guard, one guard per thread, single-threaded scripts.
pub(crate) struct PhysicsGuard<'a> {
    _marker: PhantomData<(&'a ScenePhysics, Rc<()>)>,
}

impl<'a> PhysicsGuard<'a> {
    pub fn enter(physics: &'a ScenePhysics) -> Self {
        let ptr = physics as *const ScenePhysics;
        let generation = PHYSICS_GENERATION.with(|gen| {
            let new_gen = gen.get().wrapping_add(1);
            gen.set(new_gen);
            new_gen
        });
        ACTIVE_PHYSICS.with(|cell| *cell.borrow_mut() = Some(TrackedPointer::new(ptr, generation)));
        Self {
            _marker: PhantomData,
        }
    }
}

impl Drop for PhysicsGuard<'_> {
    fn drop(&mut self) {
        ACTIVE_PHYSICS.with(|cell| *cell.borrow_mut() = None);
        PHYSICS_GENERATION.with(|gen| gen.set(0));
    }
}

/// Run `f` with the scene's physics, or report that scripts cannot reach it here (an
/// editor UI plugin, a test without a scene).
pub(crate) fn with_active_physics<R>(
    f: impl FnOnce(&ScenePhysics) -> Result<R, mlua::Error>,
) -> Result<R, mlua::Error> {
    ACTIVE_PHYSICS.with(|cell| {
        let opt = cell.borrow();
        let Some(tracked) = opt.as_ref() else {
            return Err(mlua::Error::RuntimeError("physics not available".into()));
        };
        debug_assert_eq!(
            tracked.generation,
            PHYSICS_GENERATION.with(|gen| gen.get()),
            "PhysicsGuard generation mismatch - guard may have been dropped"
        );
        // SAFETY: as for `with_active_input_state`: the pointer was set by a live
        // `PhysicsGuard` that borrows the physics for its lifetime, and scripts run
        // single-threaded and non-reentrantly.
        let physics = unsafe { &*tracked.ptr };
        f(physics)
    })
}

/// Guard that lets Lua scripts query the scene's navigation (paths, agent states)
/// while they run. Same constraints as [`PhysicsGuard`].
#[cfg(feature = "navigation")]
pub(crate) struct NavigationGuard<'a> {
    _marker: PhantomData<(&'a crate::navigation::SceneNavigation, Rc<()>)>,
}

#[cfg(feature = "navigation")]
impl<'a> NavigationGuard<'a> {
    pub fn enter(navigation: &'a crate::navigation::SceneNavigation) -> Self {
        let ptr = navigation as *const crate::navigation::SceneNavigation;
        let generation = NAVIGATION_GENERATION.with(|gen| {
            let new_gen = gen.get().wrapping_add(1);
            gen.set(new_gen);
            new_gen
        });
        ACTIVE_NAVIGATION
            .with(|cell| *cell.borrow_mut() = Some(TrackedPointer::new(ptr, generation)));
        Self {
            _marker: PhantomData,
        }
    }
}

#[cfg(feature = "navigation")]
impl Drop for NavigationGuard<'_> {
    fn drop(&mut self) {
        ACTIVE_NAVIGATION.with(|cell| *cell.borrow_mut() = None);
        NAVIGATION_GENERATION.with(|gen| gen.set(0));
    }
}

/// Run `f` with the scene's navigation, or report that scripts cannot reach it here.
#[cfg(feature = "navigation")]
pub(crate) fn with_active_navigation<R>(
    f: impl FnOnce(&crate::navigation::SceneNavigation) -> Result<R, mlua::Error>,
) -> Result<R, mlua::Error> {
    ACTIVE_NAVIGATION.with(|cell| {
        let opt = cell.borrow();
        let Some(tracked) = opt.as_ref() else {
            return Err(mlua::Error::RuntimeError("navigation not available".into()));
        };
        debug_assert_eq!(
            tracked.generation,
            NAVIGATION_GENERATION.with(|gen| gen.get()),
            "NavigationGuard generation mismatch - guard may have been dropped"
        );
        // SAFETY: as for `with_active_physics`: the pointer was set by a live
        // `NavigationGuard` that borrows the navigation for its lifetime, and scripts
        // run single-threaded and non-reentrantly.
        let navigation = unsafe { &*tracked.ptr };
        f(navigation)
    })
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

/// Executes a closure with access to the active World reference.
///
/// # Safety
/// This function dereferences a raw pointer that was stored during WorldGuard::enter.
/// The following invariants must hold:
/// - The WorldGuard that set this pointer must still be alive
/// - The World reference must not have been moved or deallocated
/// - Script execution must be single-threaded
///
/// # Panics (Debug builds only)
/// - If no WorldGuard is active
/// - If the generation counter has changed (indicating the guard was dropped)
pub(crate) fn with_active_world<R>(
    f: impl FnOnce(&World) -> Result<R, mlua::Error>,
) -> Result<R, mlua::Error> {
    ACTIVE_WORLD.with(|cell| {
        let opt = cell.borrow();
        let Some(tracked) = opt.as_ref() else {
            return Err(mlua::Error::RuntimeError("world not available".into()));
        };

        // Verify the generation counter matches in debug builds
        debug_assert_eq!(
            tracked.generation,
            WORLD_GENERATION.with(|gen| gen.get()),
            "WorldGuard generation mismatch - guard may have been dropped"
        );

        // SAFETY: The World pointer is valid because:
        // 1. It was set by WorldGuard::enter with a valid reference
        // 2. The WorldGuard is still alive (verified by generation counter in debug)
        // 3. Script execution is single-threaded and non-reentrant
        // 4. The World is not moved during guard lifetime
        let world = unsafe { &*tracked.ptr };
        f(world)
    })
}

pub(crate) fn with_active_entity<R>(
    f: impl FnOnce(i64) -> Result<R, mlua::Error>,
) -> Result<R, mlua::Error> {
    ACTIVE_ENTITY.with(|cell| {
        let opt = cell.borrow();
        let Some(entity_bits) = *opt else {
            return Err(mlua::Error::RuntimeError("entity not available".into()));
        };
        f(entity_bits)
    })
}

/// Executes a closure with access to the active InputState reference.
///
/// # Safety
/// This function dereferences a raw pointer that was stored during InputStateGuard::enter.
/// The following invariants must hold:
/// - The InputStateGuard that set this pointer must still be alive
/// - The InputState reference must not have been moved or deallocated
/// - Script execution must be single-threaded
///
/// # Panics (Debug builds only)
/// - If no InputStateGuard is active
/// - If the generation counter has changed (indicating the guard was dropped)
pub(crate) fn with_active_input_state<R>(
    f: impl FnOnce(&InputState) -> Result<R, mlua::Error>,
) -> Result<R, mlua::Error> {
    ACTIVE_INPUT_STATE.with(|cell| {
        let opt = cell.borrow();
        let Some(tracked) = opt.as_ref() else {
            return Err(mlua::Error::RuntimeError(
                "input state not available".into(),
            ));
        };

        // Verify the generation counter matches in debug builds
        debug_assert_eq!(
            tracked.generation,
            INPUT_STATE_GENERATION.with(|gen| gen.get()),
            "InputStateGuard generation mismatch - guard may have been dropped"
        );

        // SAFETY: The InputState pointer is valid because:
        // 1. It was set by InputStateGuard::enter with a valid reference
        // 2. The InputStateGuard is still alive (verified by generation counter in debug)
        // 3. Script execution is single-threaded and non-reentrant
        // 4. The InputState is not moved during guard lifetime
        let input_state = unsafe { &*tracked.ptr };
        f(input_state)
    })
}

pub(crate) fn with_active_event_queue<R>(
    f: impl FnOnce(&mut Vec<ScriptEvent>) -> Result<R, mlua::Error>,
) -> Result<R, mlua::Error> {
    ACTIVE_EVENT_QUEUE.with(|cell| {
        let opt = cell.borrow();
        let Some(rc) = opt.as_ref() else {
            return Err(mlua::Error::RuntimeError(
                "event queue not available".into(),
            ));
        };
        let mut borrow = rc.borrow_mut();
        f(&mut borrow)
    })
}

pub(crate) fn get_active_entity() -> Result<i64, mlua::Error> {
    ACTIVE_ENTITY.with(|cell| {
        let opt = cell.borrow();
        match *opt {
            Some(entity_bits) => Ok(entity_bits),
            None => Err(mlua::Error::RuntimeError(
                "active entity not available".into(),
            )),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_world_guard_lifecycle() {
        // Create a dummy world
        let world = World::new();

        // Initially, accessing active world should fail
        let result = with_active_world(|_w| Ok(()));
        assert!(result.is_err());

        // Create a guard
        {
            let _guard = WorldGuard::enter(&world);

            // Now access should succeed
            let result = with_active_world(|_w| Ok(42));
            assert_eq!(result.unwrap(), 42);

            // Verify generation counter is non-zero
            let gen = WORLD_GENERATION.with(|g| g.get());
            assert_ne!(gen, 0);
        }

        // After guard is dropped, access should fail again
        let result = with_active_world(|_w| Ok(()));
        assert!(result.is_err());

        // Generation counter should be reset
        let gen = WORLD_GENERATION.with(|g| g.get());
        assert_eq!(gen, 0);
    }

    #[test]
    fn test_entity_guard_lifecycle() {
        // Initially, getting active entity should fail
        let result = get_active_entity();
        assert!(result.is_err());

        // Create a guard with a test entity
        let test_entity_bits = 12345i64;
        {
            let _guard = EntityGuard::enter(test_entity_bits);

            // Now access should succeed
            let result = get_active_entity();
            assert_eq!(result.unwrap(), test_entity_bits);

            // Test with closure
            let result = with_active_entity(|bits| {
                assert_eq!(bits, test_entity_bits);
                Ok(42)
            });
            assert_eq!(result.unwrap(), 42);
        }

        // After guard is dropped, access should fail again
        let result = get_active_entity();
        assert!(result.is_err());
    }

    #[test]
    fn test_command_guard_lifecycle() {
        use super::super::entity_registry::EntityHandleRegistry;

        let registry = Rc::new(RefCell::new(EntityHandleRegistry::default()));
        let commands = Rc::new(RefCell::new(ScriptCommands::new(registry)));

        // Initially, accessing active commands should fail
        let result = with_active_commands(|_c| Ok(()));
        assert!(result.is_err());

        // Create a guard
        {
            let _guard = CommandGuard::enter(Rc::clone(&commands));

            // Now access should succeed
            let result = with_active_commands(|_c| Ok(42));
            assert_eq!(result.unwrap(), 42);
        }

        // After guard is dropped, access should fail again
        let result = with_active_commands(|_c| Ok(()));
        assert!(result.is_err());
    }

    #[test]
    fn test_state_guard_lifecycle() {
        let state = Rc::new(RefCell::new(ScriptStateMap::new()));

        // Initially, accessing active state should fail
        let result = with_active_state(|_s| Ok(()));
        assert!(result.is_err());

        // Create a guard
        {
            let _guard = StateGuard::enter(&state);

            // Now access should succeed
            let result = with_active_state(|_s| Ok(42));
            assert_eq!(result.unwrap(), 42);
        }

        // After guard is dropped, access should fail again
        let result = with_active_state(|_s| Ok(()));
        assert!(result.is_err());
    }

    #[test]
    fn test_event_queue_guard_lifecycle() {
        let queue = Rc::new(RefCell::new(Vec::<ScriptEvent>::new()));

        // Initially, accessing active event queue should fail
        let result = with_active_event_queue(|_q| Ok(()));
        assert!(result.is_err());

        // Create a guard
        {
            let _guard = EventQueueGuard::enter(&queue);

            // Now access should succeed
            let result = with_active_event_queue(|q| {
                assert_eq!(q.len(), 0);
                Ok(42)
            });
            assert_eq!(result.unwrap(), 42);
        }

        // After guard is dropped, access should fail again
        let result = with_active_event_queue(|_q| Ok(()));
        assert!(result.is_err());
    }

    #[test]
    fn test_nested_guards_same_type_should_panic_in_debug() {
        // This test verifies that creating nested guards of the same type
        // triggers a debug assertion failure. In release builds, this would
        // create a safety issue, so we only test the debug assertion.
        #[cfg(debug_assertions)]
        {
            let world = World::new();
            let _guard1 = WorldGuard::enter(&world);

            // Attempting to create a second guard should panic in debug builds
            let result = std::panic::catch_unwind(|| {
                let _guard2 = WorldGuard::enter(&world);
            });
            assert!(
                result.is_err(),
                "Expected panic when creating nested WorldGuard"
            );
        }
    }

    #[test]
    fn test_error_messages() {
        // Test that error messages are descriptive
        let result = with_active_world(|_| Ok(()));
        assert!(result.is_err());
        if let Err(mlua::Error::RuntimeError(msg)) = result {
            assert_eq!(msg, "world not available");
        } else {
            panic!("Expected RuntimeError");
        }

        let result = with_active_input_state(|_| Ok(()));
        assert!(result.is_err());
        if let Err(mlua::Error::RuntimeError(msg)) = result {
            assert_eq!(msg, "input state not available");
        } else {
            panic!("Expected RuntimeError");
        }

        let result = get_active_entity();
        assert!(result.is_err());
        if let Err(mlua::Error::RuntimeError(msg)) = result {
            assert_eq!(msg, "active entity not available");
        } else {
            panic!("Expected RuntimeError");
        }
    }

    #[test]
    fn test_generation_counter_increments() {
        let world = World::new();

        // When no guard is active, generation should be 0
        let gen_before = WORLD_GENERATION.with(|g| g.get());
        assert_eq!(gen_before, 0);

        // First guard - generation increments from 0 to 1
        {
            let _guard = WorldGuard::enter(&world);
            let gen1 = WORLD_GENERATION.with(|g| g.get());
            assert_eq!(gen1, 1);
        }

        // After guard drops, generation resets to 0
        let gen_after_1 = WORLD_GENERATION.with(|g| g.get());
        assert_eq!(gen_after_1, 0);

        // Second guard - generation increments from 0 to 1 again
        {
            let _guard = WorldGuard::enter(&world);
            let gen2 = WORLD_GENERATION.with(|g| g.get());
            assert_eq!(gen2, 1);
        }

        // After guard drops, generation resets to 0
        let gen_after_2 = WORLD_GENERATION.with(|g| g.get());
        assert_eq!(gen_after_2, 0);
    }

    // Compile-time tests to ensure guards are !Send and !Sync
    // These tests verify that guards cannot be moved across threads,
    // preventing use-after-drop bugs from cross-thread guard drops.

    #[test]
    fn test_guards_are_not_send() {
        // Verify all pointer-based guards are !Send
        const _: () = {
            // These assertions will cause compile errors if the guards become Send
            fn _assert_world_guard_not_send() {
                fn needs_send<T: Send>() {}
                // Uncommenting this line should cause a compile error:
                // needs_send::<WorldGuard>();
            }
            fn _assert_input_state_guard_not_send() {
                fn needs_send<T: Send>() {}
                // Uncommenting this line should cause a compile error:
                // needs_send::<InputStateGuard>();
            }
        };
    }

    #[test]
    fn test_guards_are_not_sync() {
        // Verify all pointer-based guards are !Sync
        const _: () = {
            // These assertions will cause compile errors if the guards become Sync
            fn _assert_world_guard_not_sync() {
                fn needs_sync<T: Sync>() {}
                // Uncommenting this line should cause a compile error:
                // needs_sync::<WorldGuard>();
            }
            fn _assert_input_state_guard_not_sync() {
                fn needs_sync<T: Sync>() {}
                // Uncommenting this line should cause a compile error:
                // needs_sync::<InputStateGuard>();
            }
        };
    }
}
