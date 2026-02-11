# Lua Scripting Phase 5 Test Instructions

## Overview

This directory contains comprehensive test scripts for verifying the Lua scripting integration. Each test script focuses on specific functionality and can be run independently.

## Test Scripts

### 1. test_lifecycle.lua
**Purpose**: Verify lifecycle hooks work correctly

**What it tests**:
- `on_created()` called exactly once
- `update()` called every frame
- State persists between frames
- dt parameter is correct

**How to run**:
1. Open the editor
2. Create a new entity (e.g., a cube)
3. Attach `scripts/test_lifecycle.lua` to the entity
4. Observe the logs in the console

**Expected behavior**:
- On creation: "Lifecycle Test: State initialized"
- Every 60 frames: Progress log with frame count and time
- Frame 20: "Lifecycle Test PASSED: State persists correctly"
- Entity should rotate slowly around Y axis

---

### 2. test_editor_mode.lua
**Purpose**: Verify @editor annotation works

**What it tests**:
- Script only runs in editor mode
- Script does NOT run in runtime/game mode

**How to run**:
1. Open the editor
2. Create an entity and attach `scripts/test_editor_mode.lua`
3. Observe logs while in editor mode (should see "Editor Mode Test" logs)
4. Enter play/runtime mode
5. Verify logs STOP appearing in runtime mode

**Expected behavior**:
- Editor mode: Entity at position (-2, oscillating Y, 0), logs every 60 frames
- Runtime mode: No logs, entity should not animate

---

### 3. test_runtime_mode.lua
**Purpose**: Verify runtime-only scripts work

**What it tests**:
- Script does NOT run in editor mode
- Script ONLY runs in runtime/game mode

**How to run**:
1. Open the editor
2. Create an entity and attach `scripts/test_runtime_mode.lua`
3. Verify NO logs appear while in editor mode
4. Enter play/runtime mode
5. Observe logs should now appear

**Expected behavior**:
- Editor mode: No logs, entity static
- Runtime mode: Entity at position (2, 0, 0), spinning fast, logs every 60 frames

---

### 4. test_isolation.lua
**Purpose**: Verify script isolation between entities

**What it tests**:
- Multiple entities with same script have isolated state
- Global variables are isolated per script instance
- No state leakage between entities

**How to run**:
1. Open the editor
2. Create 3-5 entities
3. Attach `scripts/test_isolation.lua` to ALL of them
4. Observe logs from each entity

**Expected behavior**:
- Each entity logs with its own entity ID
- Each entity has independent counter
- Global `my_global_counter` matches state counter for each entity
- No "ISOLATION FAILURE" messages
- Entities spin at different speeds

---

### 5. test_event_emitter.lua & test_event_listener.lua
**Purpose**: Verify event system works

**What it tests**:
- Events can be emitted
- Events can be subscribed to
- Event data passes correctly between scripts

**How to run**:
1. Create entity A with `scripts/test_event_emitter.lua`
2. Create entity B with `scripts/test_event_listener.lua`
3. Observe logs showing events being emitted and received

**Expected behavior**:
- Emitter logs: "Emitting 'test_tick' event" every 60 frames
- Listener logs: "Received 'test_tick'" matching emitter
- All event types should be emitted and received
- Event data should be intact

---

### 6. test_hotreload.lua
**Purpose**: Verify hot-reload preserves state

**What it tests**:
- State survives script reload
- on_created() runs after reload
- Restored state overwrites defaults

**How to run**:
1. Create an entity with `scripts/test_hotreload.lua`
2. Let it run for ~3 seconds (180 frames)
3. Trigger script reload (Scene → Reset Script Runtime)
4. Observe logs

**Expected behavior**:
- Before reload: "Checkpoint 1" at frame 10, "Checkpoint 2" at frame 180
- After reload: "RELOAD DETECTED! Counter was at X"
- Counter continues from previous value (not reset to 0)
- Reload count increments

---

### 7. test_comprehensive.lua
**Purpose**: Integration test for multiple API categories

**What it tests**:
- Logging API
- State Management API
- Entity Management API
- Transform API
- Input API
- Event API

**How to run**:
1. Create an entity with `scripts/test_comprehensive.lua`
2. Let it run for at least 300 frames (~5 seconds)
3. Optionally interact with keyboard/mouse during input test phase

**Expected behavior**:
- Phase 1 (frames 1-60): State management test
- Phase 2 (frames 60-120): Transform animation
- Phase 3 (frames 120-180): Entity spawn/despawn
- Phase 4 (frames 180-240): Input detection (press keys to test)
- Phase 5 (frames 240-300): Event system
- Frame 300: Final report with all test results

---

## Test Scene Setup

To test all scripts at once:

1. Create a new scene in the editor
2. Add 7 entities (cubes, spheres, or any mesh)
3. Attach each test script to a different entity:
   - Entity 1: test_lifecycle.lua
   - Entity 2: test_editor_mode.lua
   - Entity 3: test_runtime_mode.lua
   - Entities 4-6: test_isolation.lua (same script on multiple entities!)
   - Entity 7: test_event_emitter.lua
   - Entity 8: test_event_listener.lua
   - Entity 9: test_hotreload.lua
   - Entity 10: test_comprehensive.lua

4. Space entities out so you can see them all
5. Observe logs and visual feedback

## Expected Results

All tests should PASS with no errors. Look for:
- ✓ "PASSED" messages in logs
- ✗ No "FAILED" messages
- No Lua errors or panics
- Smooth visual animations
- Correct mode behavior (editor vs runtime)

## Troubleshooting

### "ISOLATION FAILURE" message
- Problem: Script isolation is broken
- Cause: Per-script environments not working correctly
- Fix: Check `component.rs` registry key implementation

### Hot-reload counter resets to 0
- Problem: State not being preserved
- Cause: State restoration happens before on_created()
- Fix: Check `state.rs` process_scripts() order

### Script runs in wrong mode
- Problem: Mode annotations not respected
- Cause: Mode checking not implemented correctly
- Fix: Check `state.rs` call_on_created() and call_update()

### Events not received
- Problem: Event subscription not working
- Cause: Event API implementation issue
- Fix: Check `api/events.rs` and event dispatch mechanism

## Success Criteria

Phase 5 is complete when:
- [ ] All lifecycle hooks work in both editor and runtime modes
- [ ] Script mode annotations (@editor) are respected
- [ ] Script isolation verified (no collisions between entities)
- [ ] Hot-reload preserves state correctly
- [ ] Event system works end-to-end
- [ ] All test scripts pass without errors
- [ ] Visual feedback confirms correct behavior
- [ ] No crashes or panics during testing
