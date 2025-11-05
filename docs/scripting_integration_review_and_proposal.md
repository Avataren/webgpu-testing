# Scripting Integration Review & Improvement Proposal

**Author**: Claude (Doba Review)
**Date**: 2025-11-05
**Status**: Proposal

---

## Executive Summary

The current Rune scripting integration provides basic entity manipulation but lacks the flexibility and power of modern game engines like Godot or Unity. This document analyzes the current limitations and proposes a comprehensive architecture to achieve feature parity with professional game engine scripting systems.

---

## 1. Current System Analysis

### 1.1 What Works Well

✅ **Basic Entity Lifecycle**
- `on_created()` and `update(dt)` callbacks
- Script hot-reloading in editor
- Per-entity script instances

✅ **Entity Creation**
- `spawn_entity()` with optional names
- Basic transform manipulation (translation, rotation)
- Dynamic script attachment
- GLTF model importing

✅ **State Management**
- Per-entity key-value state storage
- Type-safe f64 accessors

✅ **Developer Experience**
- Syntax highlighting in editor
- Inline and file-based scripts
- Logging functions

### 1.2 Critical Limitations

❌ **Component System Access**
- **No component queries**: Cannot read/write any ECS components
- **No component addition/removal**: Can't add lights, cameras, meshes, materials
- **Available components not exposed**: 40+ component types exist but scripts can't touch them:
  - `CameraComponent`, `MeshComponent`, `MaterialComponent`
  - `PointLight`, `DirectionalLight`, `SpotLight`
  - `ParticleEmitterComponent`, `ParticleSystemComponent`
  - `Visible`, `Billboard`, `DepthState`
  - `RotateAnimation`, `OrbitAnimation`
  - etc.

❌ **Transform System**
- **No relative transforms**: Can only set absolute position/rotation
- **No scale manipulation**: Scripts can't scale entities
- **No world transforms**: Can't read computed world-space transforms
- **No hierarchy manipulation**: Can't parent/unparent at runtime
- **Missing math**: No access to Parent/Children components

❌ **Entity Queries**
- **No entity finding**: Can't find entities by name, component, or criteria
- **No spatial queries**: Can't find entities near a position
- **No component iteration**: Can't iterate all entities with specific components

❌ **Input System**
- **No input events**: Scripts can't respond to keyboard/mouse
- **No event callbacks**: No click, hover, drag handlers

❌ **Physics & Collision**
- **No collision detection**: Can't detect overlaps or contacts
- **No raycasting**: Can't cast rays for line-of-sight, picking
- **No physics queries**: No sphere casts, box casts

❌ **Time & Frame Management**
- **Only delta time**: No total elapsed time, frame count
- **No fixed timestep**: Can't run physics at fixed rate
- **No coroutines**: No yield/wait functionality

❌ **Math & Utility**
- **Limited math types**: Only exposed via floats, no Vector3, Quat, Matrix4
- **Euler-only rotations**: No quaternion operations for complex rotations
- **No interpolation**: No lerp, slerp helpers

❌ **Asset & Scene Management**
- **No prefab spawning**: Can't instantiate saved entity templates
- **No scene loading**: Can't switch or layer scenes
- **No asset handles**: Can't reference/load materials, meshes, textures

❌ **Communication**
- **No event system**: Scripts can't emit/listen to events
- **No script messaging**: Scripts can't communicate directly
- **No signals**: No observer pattern support

---

## 2. Godot & Unity Comparison

### 2.1 Godot GDScript Features

```gdscript
# Component access
var camera = $Camera3D
var mesh = $MeshInstance3D

# Input handling
if Input.is_action_pressed("move_forward"):
    position += transform.basis.z * speed * delta

# Physics queries
var hit = raycast(position, direction, max_distance)
if hit:
    print("Hit: ", hit.collider.name)

# Scene tree manipulation
var new_node = preload("res://enemy.tscn").instantiate()
add_child(new_node)

# Signals (events)
signal health_changed(new_health)
health_changed.emit(100)

# Rich built-in types
var velocity = Vector3(1, 0, 0)
var rotation = Quat.from_euler(Vector3(0, PI/2, 0))
```

### 2.2 Unity C# Features

```csharp
// Component access
var camera = GetComponent<Camera>();
var rb = GetComponent<Rigidbody>();

// Input handling
if (Input.GetKey(KeyCode.Space)) {
    rb.AddForce(Vector3.up * jumpForce);
}

// Physics queries
if (Physics.Raycast(origin, direction, out RaycastHit hit)) {
    Debug.Log($"Hit: {hit.collider.name}");
}

// Component manipulation
gameObject.AddComponent<Light>();
Destroy(GetComponent<MeshRenderer>());

// Prefab instantiation
var enemy = Instantiate(enemyPrefab, position, rotation);

// Events
public UnityEvent onPlayerDeath;
onPlayerDeath.Invoke();

// Coroutines
StartCoroutine(DelayedAction(2.0f));
```

---

## 3. Proposed Architecture

### 3.1 Core Principles

1. **Expose the ECS**: Make Hecs entities and components accessible to scripts
2. **Type Safety**: Leverage Rune's type system for compile-time safety
3. **Performance**: Use command buffering to avoid runtime ECS conflicts
4. **Gradual Migration**: Extend existing API, don't break current scripts
5. **Rust-Rune Bridge**: Create clean FFI layer for component marshalling

### 3.2 API Design Overview

```rune
// ============================================================================
// ENTITY & COMPONENT QUERIES
// ============================================================================

// Find entities
let entities = query()
    .with_component("MeshComponent")
    .with_component("TransformComponent")
    .find();

let entity = find_entity_by_name("Player");

// Component access
let transform = get_component(entity, "TransformComponent");
let camera = get_component(entity, "CameraComponent");

// Component manipulation
set_component(entity, "Visible", { visible: false });
add_component(entity, "PointLight", {
    color: [1.0, 0.8, 0.6],
    intensity: 5.0,
    range: 10.0,
});
remove_component(entity, "RotateAnimation");

// ============================================================================
// TRANSFORM SYSTEM
// ============================================================================

// Local transform (relative to parent)
let local_pos = get_local_translation(entity);
set_local_translation(entity, [x, y, z]);
set_local_rotation(entity, yaw, pitch, roll);
set_local_scale(entity, [sx, sy, sz]);

// World transform (computed)
let world_pos = get_world_translation(entity);
let world_rot = get_world_rotation(entity);

// Relative transforms
translate(entity, [dx, dy, dz]); // Add to position
rotate(entity, axis, angle);      // Rotate around axis
look_at(entity, target_pos);      // Orient towards position

// Hierarchy
set_parent(child, parent);
let parent = get_parent(entity);
let children = get_children(entity);

// ============================================================================
// MATH TYPES
// ============================================================================

// Vector3
struct Vec3 {
    x, y, z,

    fn length(self) { ... }
    fn normalize(self) { ... }
    fn dot(self, other) { ... }
    fn cross(self, other) { ... }
    fn lerp(self, other, t) { ... }
}

// Quaternion
struct Quat {
    x, y, z, w,

    fn from_euler(yaw, pitch, roll) { ... }
    fn from_axis_angle(axis, angle) { ... }
    fn slerp(self, other, t) { ... }
    fn to_euler(self) { ... }
}

// ============================================================================
// INPUT SYSTEM
// ============================================================================

pub fn update(self_entity, dt) {
    if is_key_pressed("W") {
        translate(self_entity, [0.0, 0.0, -1.0 * dt]);
    }

    if is_mouse_button_pressed(0) { // Left click
        let (x, y) = get_mouse_position();
        on_click(x, y);
    }
}

// ============================================================================
// PHYSICS & RAYCASTING
// ============================================================================

// Raycast
let result = raycast(origin, direction, max_distance);
if result.hit {
    log_info(`Hit ${result.entity} at ${result.position}`);
}

// Collision queries
let nearby = overlap_sphere(center, radius);
let collisions = overlap_box(center, half_extents);

// ============================================================================
// TIME & SCHEDULING
// ============================================================================

let total_time = get_time();
let frame = get_frame_count();

// Timers (one-shot or repeating)
let timer_id = set_timeout(self_entity, 2.0, "on_delayed_spawn");
let timer_id = set_interval(self_entity, 0.5, "on_pulse");
cancel_timer(timer_id);

// Coroutine-style (future enhancement)
yield wait_for_seconds(2.0);
yield wait_until(condition);

// ============================================================================
// ASSET & PREFAB SYSTEM
// ============================================================================

// Instantiate saved entity templates
let enemy = spawn_prefab("prefabs/enemy.entity");
set_translation(enemy, [x, y, z]);

// Load assets
let mesh = load_mesh("models/cube.obj");
let material = load_material("materials/metal.mat");
set_component(entity, "MeshComponent", { mesh: mesh });
set_component(entity, "MaterialComponent", { material: material });

// ============================================================================
// EVENT SYSTEM
// ============================================================================

// Emit events
emit_event("player_damaged", { damage: 10, attacker: enemy });

// Listen to events (register in on_created)
pub fn on_created(self_entity) {
    subscribe_event(self_entity, "player_damaged", "on_player_damaged");
}

pub fn on_player_damaged(self_entity, event_data) {
    let damage = event_data.damage;
    log_info(`Player took ${damage} damage!`);
}

// ============================================================================
// LIFECYCLE HOOKS (EXTENDED)
// ============================================================================

pub fn on_created(self_entity) { ... }     // Current
pub fn update(self_entity, dt) { ... }      // Current

pub fn fixed_update(self_entity) { ... }    // NEW: Physics timestep
pub fn on_enabled(self_entity) { ... }      // NEW: Entity enabled
pub fn on_disabled(self_entity) { ... }     // NEW: Entity disabled
pub fn on_destroyed(self_entity) { ... }    // NEW: Before removal
pub fn on_collision(self_entity, other) { ... } // NEW: Collision detected
```

---

## 4. Implementation Plan

### Phase 1: Foundation (Week 1-2)

**Goal**: Establish core infrastructure for component access

✅ **Task 1.1**: Component Registry System
- Create `ComponentTypeRegistry` to map string names to Rust types
- Implement reflection traits for all components
- Add `to_rune_value()` and `from_rune_value()` for each component type

✅ **Task 1.2**: Query System API
```rust
// In src/scripting/rune.rs
fn query_entities(world: &World, component_names: Vec<String>) -> Vec<i64>
fn get_component_value(world: &World, entity: Entity, component_name: &str) -> Option<Value>
fn set_component_value(world: &mut World, entity: Entity, component_name: &str, value: Value)
```

✅ **Task 1.3**: Expose to Rune
- Add `query()`, `get_component()`, `set_component()` functions
- Add `find_entity_by_name()`, `entity_exists()`

**Deliverable**: Scripts can read/write basic components (Name, TransformComponent, Visible)

---

### Phase 2: Transform & Hierarchy (Week 2-3)

**Goal**: Rich transform manipulation and hierarchy control

✅ **Task 2.1**: Extended Transform API
- Add `translate()`, `rotate()`, `set_scale()`
- Add `get_world_translation()`, `get_world_rotation()`
- Add `look_at()`, `rotate_around()`

✅ **Task 2.2**: Hierarchy Manipulation
- Add `set_parent()`, `get_parent()`, `get_children()`
- Update Parent/Children components from scripts
- Handle transform propagation

✅ **Task 2.3**: Math Types
- Create `ScriptVec3`, `ScriptQuat` Rust types with Rune bindings
- Implement arithmetic, interpolation, utility methods
- Export constructors and common operations

**Deliverable**: Scripts have full transform control similar to Unity/Godot

---

### Phase 3: Input System (Week 3-4)

**Goal**: Scripts can respond to user input

✅ **Task 3.1**: Input State Capture
- Create `InputState` resource with keyboard/mouse state
- Update each frame before script execution
- Store key/button state, mouse position, deltas

✅ **Task 3.2**: Rune Input API
- Add `is_key_pressed()`, `is_key_just_pressed()`, `is_key_just_released()`
- Add `is_mouse_button_pressed()`, `get_mouse_position()`, `get_mouse_delta()`
- Add `get_mouse_scroll_delta()`

**Deliverable**: Scripts can create interactive behaviors

---

### Phase 4: Entity Queries & Raycasting (Week 4-5)

**Goal**: Find and interact with entities spatially

✅ **Task 4.1**: Entity Query Builder
- Implement `QueryBuilder` in Rust
- Support `.with_component()`, `.without_component()`, `.with_name()`
- Return filtered entity lists to Rune

✅ **Task 4.2**: Spatial Queries
- Add `get_entities_in_radius()`, `get_nearest_entity()`
- Implement using brute-force iteration (optimize later)

✅ **Task 4.3**: Raycasting (Basic)
- Add `raycast()` function
- Ray-AABB intersection using MeshBounds
- Return entity, position, normal

**Deliverable**: Scripts can find and interact with nearby entities

---

### Phase 5: Events & Communication (Week 5-6)

**Goal**: Scripts can communicate via events

✅ **Task 5.1**: Event Bus System
- Create `ScriptEventBus` resource
- Store event subscriptions per entity
- Queue events during frame, dispatch after updates

✅ **Task 5.2**: Rune Event API
- Add `emit_event(event_name, data)`
- Add `subscribe_event(self_entity, event_name, callback_name)`
- Call script functions when events match

✅ **Task 5.3**: Built-in Events
- Emit `entity_created`, `entity_destroyed`
- Emit `collision_started`, `collision_ended` (if physics added)

**Deliverable**: Scripts have signal/event system like Godot

---

### Phase 6: Asset System & Prefabs (Week 6-7)

**Goal**: Scripts can spawn prefabs and load assets

✅ **Task 6.1**: Prefab Serialization
- Add entity template saving/loading
- Store components in JSON format
- Add `prefabs/` directory support

✅ **Task 6.2**: Rune Prefab API
- Add `spawn_prefab(path)` function
- Instantiate all components from template
- Maintain entity hierarchy

✅ **Task 6.3**: Asset Loading Helpers
- Add `load_mesh(path)`, `load_material(path)` wrappers
- Return handle IDs usable in `set_component()`

**Deliverable**: Scripts can spawn complex entity graphs

---

### Phase 7: Advanced Features (Week 7-8)

**Goal**: Time management, lifecycle hooks, polish

✅ **Task 7.1**: Extended Lifecycle
- Add `fixed_update()` callback at fixed timestep
- Add `on_enabled()`, `on_disabled()`, `on_destroyed()`

✅ **Task 7.2**: Timer System
- Add `set_timeout()`, `set_interval()`, `cancel_timer()`
- Store timers in script state
- Tick and dispatch callbacks

✅ **Task 7.3**: Time Utilities
- Add `get_time()`, `get_frame_count()`
- Add `get_delta_time()` (alternative to parameter)

**Deliverable**: Feature-complete scripting system

---

### Phase 8: Documentation & Examples (Week 8+)

✅ **Task 8.1**: API Documentation
- Generate comprehensive API reference
- Document all functions with examples
- Create migration guide from old API

✅ **Task 8.2**: Example Scripts
- First-person controller
- Third-person camera
- Enemy AI with patrol and chase
- Particle system spawner
- UI interaction
- Door/button/trigger system

✅ **Task 8.3**: Editor Improvements
- Autocomplete for script API
- Inline documentation tooltips
- Script templates for common patterns

---

## 5. Example Use Cases

### 5.1 Player Controller

```rune
pub fn on_created(self_entity) {
    set_state(self_entity, "speed", 5.0);
}

pub fn update(self_entity, dt) {
    let speed = get_state(self_entity, "speed", 5.0);
    let mut velocity = Vec3::new(0.0, 0.0, 0.0);

    if is_key_pressed("W") {
        velocity.z -= 1.0;
    }
    if is_key_pressed("S") {
        velocity.z += 1.0;
    }
    if is_key_pressed("A") {
        velocity.x -= 1.0;
    }
    if is_key_pressed("D") {
        velocity.x += 1.0;
    }

    if velocity.length() > 0.0 {
        velocity = velocity.normalize();
        translate(self_entity, velocity * speed * dt);
    }
}
```

### 5.2 Enemy AI

```rune
pub fn on_created(self_entity) {
    set_state(self_entity, "state", "patrol");
    set_state(self_entity, "target", None);
}

pub fn update(self_entity, dt) {
    let state = get_state(self_entity, "state", "patrol");

    if state == "patrol" {
        patrol(self_entity, dt);

        // Check for player in range
        let player = find_entity_by_name("Player");
        if player != None {
            let my_pos = get_world_translation(self_entity);
            let player_pos = get_world_translation(player);
            let distance = Vec3::distance(my_pos, player_pos);

            if distance < 10.0 {
                set_state(self_entity, "state", "chase");
                set_state(self_entity, "target", player);
            }
        }
    } else if state == "chase" {
        chase_target(self_entity, dt);
    }
}

fn chase_target(self_entity, dt) {
    let target = get_state(self_entity, "target", None);
    if target == None {
        set_state(self_entity, "state", "patrol");
        return;
    }

    let my_pos = get_world_translation(self_entity);
    let target_pos = get_world_translation(target);
    let direction = Vec3::normalize(target_pos - my_pos);

    translate(self_entity, direction * 3.0 * dt);
    look_at(self_entity, target_pos);
}
```

### 5.3 Interactive Door

```rune
pub fn on_created(self_entity) {
    subscribe_event(self_entity, "door_trigger", "on_door_trigger");
    set_state(self_entity, "is_open", false);
}

pub fn on_door_trigger(self_entity, event_data) {
    let is_open = get_state(self_entity, "is_open", false);

    if !is_open {
        // Open door
        rotate(self_entity, Vec3::new(0.0, 1.0, 0.0), 1.57); // 90 degrees
        set_state(self_entity, "is_open", true);

        // Close after 3 seconds
        set_timeout(self_entity, 3.0, "close_door");
    }
}

pub fn close_door(self_entity) {
    rotate(self_entity, Vec3::new(0.0, 1.0, 0.0), -1.57);
    set_state(self_entity, "is_open", false);
}
```

---

## 6. Technical Challenges

### 6.1 ECS Access from Scripts

**Challenge**: Rune scripts run in a VM, can't directly access Rust ECS
**Solution**: Command buffering pattern (already used), extend to reads with snapshot pattern

```rust
// Read snapshot at start of frame
let component_snapshot = create_component_snapshot(world, entity);
// Script reads from snapshot
// Script writes to command buffer
// Apply commands after all scripts run
apply_script_commands(world);
```

### 6.2 Type Marshalling

**Challenge**: Convert between Rust components and Rune Values
**Solution**: Trait-based serialization

```rust
trait ToRuneValue {
    fn to_rune_value(&self) -> Result<Value, RuneScriptingError>;
}

trait FromRuneValue {
    fn from_rune_value(value: &Value) -> Result<Self, RuneScriptingError>
    where Self: Sized;
}

// Implement for all components
impl ToRuneValue for TransformComponent { ... }
impl FromRuneValue for TransformComponent { ... }
```

### 6.3 Performance

**Challenge**: Heavy scripting can slow frame rate
**Solutions**:
- **Script budgeting**: Limit script execution time per frame
- **Selective updates**: Only run scripts for visible/active entities
- **Optimization hints**: Mark hot-path scripts for caching
- **Profiling**: Add script performance metrics to editor

### 6.4 Error Handling

**Challenge**: Script errors shouldn't crash engine
**Solutions**:
- Catch all script errors, log to console
- Disable malfunctioning scripts automatically
- Show script errors in editor UI
- Add script validation before execution

---

## 7. Migration Strategy

### 7.1 Backward Compatibility

All existing scripts continue to work unchanged:
- Keep current `spawn_entity()`, `set_name()`, `set_translation()`, etc.
- New API is purely additive
- Old functions call new implementation under the hood

### 7.2 Deprecation Path (Optional)

If old API should eventually be removed:
1. Mark old functions as deprecated in docs (Year 1)
2. Add runtime warnings when old API used (Year 2)
3. Provide migration tool to update scripts (Year 2)
4. Remove deprecated functions (Year 3+)

**Recommendation**: Keep old API indefinitely for simplicity

---

## 8. Success Metrics

### 8.1 Feature Parity

- ✅ Can replicate any Unity/Godot tutorial script
- ✅ Can build complete game mechanics in scripts (player, enemies, items)
- ✅ Can create reusable script libraries

### 8.2 Performance

- ⏱️ 1000 active scripted entities at 60 FPS
- ⏱️ <1ms total script execution time per frame
- ⏱️ Hot-reload in <100ms

### 8.3 Developer Experience

- 📝 Comprehensive documentation with examples
- 🎓 Tutorial series for common patterns
- 🛠️ Editor support (autocomplete, inline docs)
- 🐛 Clear error messages with line numbers

---

## 9. Conclusion

The current scripting system provides a solid foundation but is severely limited compared to Godot and Unity. This proposal outlines a comprehensive path to achieve feature parity through:

1. **Full ECS access** via component queries and manipulation
2. **Rich transform system** with hierarchy control
3. **Input handling** for interactive behaviors
4. **Entity queries** and spatial searches
5. **Event system** for script communication
6. **Asset/prefab system** for complex entity spawning
7. **Extended lifecycle** with timers and fixed updates

**Estimated Timeline**: 8-10 weeks for full implementation
**Priority**: High - Scripting is core to rapid iteration and gameplay development

The proposed architecture maintains backward compatibility while providing the flexibility needed for professional game development. Implementation can proceed incrementally, delivering value at each phase.

---

## Appendix A: API Reference Template

*(Full API documentation to be generated during Phase 8)*

### Entity Functions
- `spawn_entity(name: Option<String>) -> i64`
- `destroy_entity(entity: i64)`
- `entity_exists(entity: i64) -> bool`
- `find_entity_by_name(name: String) -> Option<i64>`

### Component Functions
- `get_component(entity: i64, component_name: String) -> Option<Value>`
- `set_component(entity: i64, component_name: String, value: Value)`
- `add_component(entity: i64, component_name: String, value: Value)`
- `remove_component(entity: i64, component_name: String)`
- `has_component(entity: i64, component_name: String) -> bool`

### Transform Functions
- `get_local_translation(entity: i64) -> Vec3`
- `set_local_translation(entity: i64, position: Vec3)`
- `get_local_rotation(entity: i64) -> Quat`
- `set_local_rotation(entity: i64, rotation: Quat)`
- `get_local_scale(entity: i64) -> Vec3`
- `set_local_scale(entity: i64, scale: Vec3)`
- `translate(entity: i64, delta: Vec3)`
- `rotate(entity: i64, axis: Vec3, angle: f64)`
- `look_at(entity: i64, target: Vec3)`

*(Continues...)*

---

## Appendix B: Comparison Matrix

| Feature | Current | Proposed | Godot | Unity |
|---------|---------|----------|-------|-------|
| Component Access | ❌ | ✅ | ✅ | ✅ |
| Transform Control | ⚠️ Basic | ✅ Full | ✅ | ✅ |
| Input Handling | ❌ | ✅ | ✅ | ✅ |
| Entity Queries | ❌ | ✅ | ✅ | ✅ |
| Physics/Raycasting | ❌ | ✅ | ✅ | ✅ |
| Event System | ❌ | ✅ | ✅ Signals | ✅ UnityEvents |
| Hierarchy Control | ❌ | ✅ | ✅ | ✅ |
| Prefabs | ❌ | ✅ | ✅ Scenes | ✅ Prefabs |
| Coroutines | ❌ | ⚠️ Timers | ✅ yield | ✅ Coroutines |
| Hot Reload | ✅ | ✅ | ✅ | ⚠️ Limited |
| Math Types | ❌ | ✅ | ✅ | ✅ |
| Lifecycle Hooks | ⚠️ 2 | ✅ 6+ | ✅ Many | ✅ Many |

**Legend**: ✅ Full Support | ⚠️ Partial | ❌ Missing
