# Application System Separation - Phase 2 Progress

**Date**: 2025-11-06
**Branch**: `claude/refactor-application-systems-011CUsAGnsdiNFyCXn5hK6Ue`
**Status**: Phase 2 In Progress (27% Complete)

---

## Progress Summary

### Handlers Extracted: 7 of 26 (27%)

✅ **Completed**:
1. Transform (handle_update_transform) - 24 lines
2. Camera (handle_update_camera) - 37 lines
3. Point Light (handle_update_point_light) - 23 lines
4. Directional Light (handle_update_directional_light) - 23 lines
5. Spot Light (handle_update_spot_light) - 23 lines
6. Set Can Cast Shadow (handle_set_can_cast_shadow) - 50 lines
7. Rename Entity (handle_rename_entity) - 18 lines

**Total Extracted**: ~200 lines into focused handler functions

---

## Commit History (Phase 2)

1. **38eef70** - Create action_handlers foundation (Phase 1)
   - ActionContext, ActionResult infrastructure
   - Transform & camera handlers

2. **0b5948d** - Add action dispatcher (Phase 1)
   - dispatch_action() routing mechanism

3. **ff9cd86** - Documentation (Phase 1)
   - Comprehensive Phase 1 summary

4. **81534a0** - Extract light action handlers
   - 3 light types (point, directional, spot)

5. **7ac3220** - Extract misc action handlers
   - SetCanCastShadow, RenameEntity

---

## Remaining Work: 19 of 26 handlers (73%)

### Materials (5 handlers, ~180 lines)
- [ ] UpdateMaterial (~40 lines)
- [ ] SetMaterialKind (~40 lines)
- [ ] CreateShaderMaterial (~30 lines)
- [ ] AssignShaderSource (~35 lines)
- [ ] CreateShaderSource (~35 lines)

**Location**: Lines 693-856 in application/mod.rs
**Complexity**: Medium (handle validation, asset updates)

### Environment (1 handler, ~66 lines)
- [ ] UpdateEnvironment (~66 lines)

**Location**: Lines 1055-1123 in application/mod.rs
**Complexity**: High (calls copy_environment_asset_if_needed, complex logic)
**Note**: May need to extract helper functions

### Particles (3 handlers, ~220 lines)
- [ ] UpdateParticleSystem (~60 lines)
- [ ] UpdateParticleEmitter (~80 lines)
- [ ] UpdateParticleBehavior (~80 lines)

**Location**: Lines 1124-1284 in application/mod.rs
**Complexity**: High (complex component setup, mesh/material creation)

### Billboard (1 handler, ~30 lines)
- [ ] SetBillboard (~30 lines)

**Location**: ~Line 1260 in application/mod.rs
**Complexity**: Low

### Scripts (3 handlers, ~120 lines)
- [ ] AddScript (~60 lines)
- [ ] ChangeScriptSource (~30 lines)
- [ ] EditScript (handled in UI, ~30 lines fallback)

**Location**: Lines 1333-1453 in application/mod.rs
**Complexity**: Medium (component insertion, path handling)

### Add Components (7 handlers, ~140 lines)
- [ ] AddCamera (~20 lines)
- [ ] AddMesh (~20 lines)
- [ ] AddPointLight (~20 lines)
- [ ] AddDirectionalLight (~20 lines)
- [ ] AddSpotLight (~20 lines)
- [ ] AddEnvironment (~20 lines)
- [ ] AddParticleSystem (~20 lines)

**Location**: Lines 1454-1543 in application/mod.rs
**Complexity**: Low (simple component insertion)

### Shader Actions (1 handler, special case)
- [ ] EditShader (handled elsewhere, ~10 lines)

**Location**: Lines 1562-1568 in application/mod.rs
**Complexity**: Low (just logs, no action needed)

---

## Extraction Pattern

All handlers follow this pattern:

```rust
pub fn handle_<action_name>(
    ctx: &mut ActionContext,
    entity: Entity,
    <parameters>
) -> ActionResult {
    // 1. Get component from world
    let world = ctx.scene.main_world_mut();
    match world.get::<&mut Component>(entity) {
        Ok(mut component) => {
            // 2. Update component
            *component = new_value;

            // 3. Record change
            ctx.app.record_scene_change(ctx.scene);

            // 4. Return result
            ActionResult::scene_changed()
        }
        Err(err) => {
            log::warn!("Failed to update: {}", err);
            ActionResult::no_change()
        }
    }
}
```

### Steps for Each Handler:

1. **Find in original**: Search for `InspectorAction::<Name>` in application/mod.rs
2. **Copy logic**: Extract the match arm code
3. **Create handler file**: Add to appropriate module (or create new one)
4. **Update imports**: Add to action_handlers/mod.rs
5. **Update dispatcher**: Add match arm in dispatch.rs
6. **Build**: Verify with `cargo build`
7. **Commit**: One commit per handler or group

---

## File Structure

```
action_handlers/
├── mod.rs              # ActionContext, ActionResult, re-exports
├── dispatch.rs         # dispatch_action() router
├── transform.rs        # Transform handler
├── camera.rs           # Camera handler
├── lights.rs           # Light handlers (3 types)
├── misc.rs             # SetCanCastShadow, RenameEntity
└── (to be added)
    ├── materials.rs    # 5 material handlers
    ├── environment.rs  # 1 environment handler
    ├── particles.rs    # 3 particle handlers
    ├── scripts.rs      # 3 script handlers
    └── components.rs   # 7 add component handlers
```

---

## Complexity Notes

### Simple Handlers (Low Complexity)
- Most "Add*" actions: Just insert component
- RenameEntity: Simple field update
- SetBillboard: Toggle component

**Pattern**: 15-25 lines, straightforward

### Medium Handlers (Medium Complexity)
- Material actions: Validate handle matches, update asset
- Light actions: Update component, record change
- Script actions: Path handling, component insertion

**Pattern**: 30-50 lines, some validation

### Complex Handlers (High Complexity)
- UpdateEnvironment: Asset copying, HDR handling, path comparison
- Particle actions: Mesh/material creation, complex setup
- UpdateParticleEmitter: Spawner creation, buffer setup

**Pattern**: 60-80 lines, may need helper functions

---

## Estimated Completion Time

**Remaining Work**:
- Simple handlers (8): ~2 hours
- Medium handlers (6): ~3 hours
- Complex handlers (5): ~4 hours

**Total**: ~9 hours to complete all handlers

**Strategy**:
1. Start with simple handlers (momentum)
2. Group by domain (all materials together)
3. Tackle complex ones last
4. Test frequently

---

## Testing Strategy

After all handlers extracted:

1. **Compile verification**: `cargo build`
2. **Replace original match**: Update apply_pending_inspector_actions() to use dispatcher
3. **Run editor**: Manual testing of inspector actions
4. **Unit tests**: Add tests for each handler module

---

## Benefits Achieved So Far

- ✅ **7 handlers extracted** (198 lines)
- ✅ **Pattern established** and proven
- ✅ **Infrastructure complete** (ActionContext, ActionResult, Dispatcher)
- ✅ **Build verified** at each step
- ✅ **Clear roadmap** for remaining work

---

## Next Steps

### Immediate:
1. Extract material handlers (5 types) → materials.rs
2. Extract add component handlers (7 types) → components.rs
3. Extract billboard handler → particles.rs or misc.rs
4. Extract script handlers (3 types) → scripts.rs

### Then:
5. Extract complex particle handlers → particles.rs
6. Extract complex environment handler → environment.rs

### Finally:
7. Replace apply_pending_inspector_actions() match with dispatcher
8. Add unit tests
9. Remove old implementation
10. Update documentation

---

## Command Reference

```bash
# Build
cargo build

# Find handler in original
grep -n "InspectorAction::<Name>" src/bin/editor/application/mod.rs

# Extract lines
sed -n '<start>,<end>p' src/bin/editor/application/mod.rs

# Commit
git add -A
git commit -m "Refactor: Extract <name> handlers"

# Push
git push
```

---

**Status**: Checkpoint reached - 27% complete with clear path forward
**Quality**: All extracted handlers build and follow consistent pattern
**Risk**: Low - incremental extraction with frequent verification
