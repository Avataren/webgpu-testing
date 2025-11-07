# Plugin Architecture Improvement Plan

> Generated: 2025-11-06
> Based on: Comprehensive architectural review of RuneScript plugin system

## Executive Summary

The plugin architecture is well-designed with clear patterns (command pattern, ECS integration, sandboxed execution) but suffers from state management complexity and lacks dynamic loading capabilities. The main bottleneck is the `EditorSharedState` god object, and the most impactful improvement would be implementing plugin hot reload.

---

## Current Architecture Assessment

### ✅ Strengths
1. **Annotation-based mode system** (`@editor`, `@tool`) - Simple and declarative
2. **Command pattern throughout** - Good separation of concerns
3. **ECS integration** - Plugins are first-class entities
4. **Sandboxed VM execution** - Scripts can't crash editor
5. **Manifest-based configuration** - Easy plugin management via TOML

### ❌ Weaknesses
1. **God object pattern** - `EditorSharedState` has 18+ fields (violates SRP)
2. **Thread-local state access** - Implicit dependencies, hard to test
3. **One-frame UI input lag** - Command recording creates noticeable latency
4. **No plugin hot reload** - Requires editor restart for changes
5. **Fragile annotation parsing** - Limited validation, fails silently
6. **Timing dependencies** - Plugin loading coupled to project system

### 📊 Architecture Metrics
- **Lifecycle functions**: 3 (`on_created`, `update`, `on_ui`)
- **Plugin modes**: 3 (RuntimeOnly, EditorOnly, Both)
- **UI command types**: 10+ (Button, Label, TextEdit, Slider, etc.)
- **State layers**: 4 (per-script, frame-to-frame, config, global)
- **Code locations**:
  - Core: `src/scripting/rune/`
  - Integration: `src/bin/editor/application/`
  - Examples: `examples/scripts/`

---

## Improvement Roadmap

### 🔥 Phase 1: Foundation (High Priority)

#### 1.1 Refactor EditorSharedState
**Impact**: High | **Effort**: Medium | **Risk**: Medium

**Current Problem**:
- File: `src/bin/editor/application/core.rs` (lines 50-79)
- Contains 18+ unrelated fields
- Changes ripple across entire editor
- Violates Single Responsibility Principle

**Solution**:
```rust
// Break into cohesive modules
pub struct EditorSharedState {
    ui_state: Box<UIState>,
    plugin_state: Box<PluginState>,
    system_state: Box<SystemState>,
    command_queue: VecDeque<EditorCommand>,
}

pub struct UIState {
    viewports: ViewportState,
    theme: Theme,
    dpi_factor: f32,
}

pub struct PluginState {
    ui_plugin_manager: Option<UiPluginManager>,
    ui_plugins_loaded: bool,
    script_ui_commands: HashMap<Entity, Vec<UiCommand>>,
    script_ui_responses: HashMap<Entity, Vec<UiResponse>>,
}

pub struct SystemState {
    shader_watcher: ShaderWatcher,
    runtime_state: RuntimeStateHandle,
    inspector: InspectorState,
}
```

**Benefits**:
- Clear ownership boundaries
- Easier to test individual components
- Reduces cognitive load
- Enables parallel refactoring

**Migration Strategy**:
1. Create new module structure
2. Move fields incrementally
3. Update access patterns (`.shared.ui_state.theme` vs `.shared.theme`)
4. Add builder pattern for initialization
5. Test after each field migration

**Acceptance Criteria**:
- [ ] No struct > 10 fields
- [ ] Each module has single responsibility
- [ ] All tests pass
- [ ] No performance regression

---

#### 1.2 Implement Plugin Hot Reload
**Impact**: Very High | **Effort**: High | **Risk**: Medium

**Current Problem**:
- Plugins load once at startup
- Requires full editor restart for changes
- Slow iteration cycle for plugin developers

**Solution Architecture**:

```rust
pub struct PluginWatcher {
    watcher: notify::RecommendedWatcher,
    modified_plugins: Arc<Mutex<HashSet<PathBuf>>>,
}

pub trait HotReloadable {
    fn on_before_reload(&mut self) -> SerializedState;
    fn on_after_reload(&mut self, state: SerializedState);
}

impl UiPluginManager {
    pub fn reload_plugin(&mut self, entity: Entity, scene: &mut Scene) -> Result<(), Error> {
        // 1. Extract state from old script
        let old_state = self.extract_state(entity)?;

        // 2. Reload script source
        let new_source = self.load_script_source(entity)?;

        // 3. Recompile script
        scene.reload_script(entity, new_source)?;

        // 4. Restore state
        self.restore_state(entity, old_state)?;

        // 5. Re-trigger on_created
        self.reinitialize_plugin(entity)?;

        Ok(())
    }
}
```

**Implementation Steps**:

1. **Add file watching** (using `notify` crate)
   - Watch `examples/scripts/*.rn`
   - Debounce changes (500ms)
   - Collect modified files per frame

2. **Implement state preservation**
   - Serialize script state before reload
   - Store in temporary buffer
   - Restore after recompilation

3. **Add reload command**
   ```rust
   pub enum EditorCommand {
       // ... existing
       ReloadPlugin { entity: Entity, preserve_state: bool },
   }
   ```

4. **Add UI controls**
   - "Reload Plugin" button in plugin windows
   - Keyboard shortcut (Ctrl+R in plugin window)
   - Visual indicator during reload

5. **Error handling**
   - Show compilation errors in UI
   - Keep old version on error
   - Allow manual retry

**Benefits**:
- 10x faster iteration for plugin development
- No lost editor state
- Encourages experimentation

**Risks**:
- State migration issues if API changes
- Potential memory leaks if cleanup incomplete
- Race conditions if reload during execution

**Mitigation**:
- Clear state migration strategy
- Proper Drop implementations
- Reload only when script not executing

**Acceptance Criteria**:
- [ ] Script changes detected within 1 second
- [ ] State preserved across reloads (80% cases)
- [ ] Compilation errors shown in UI
- [ ] No memory leaks after 100 reloads
- [ ] Works with all example plugins

---

#### 1.3 Enhanced Error Reporting
**Impact**: Medium | **Effort**: Low | **Risk**: Low

**Current Problem**:
- Annotation parsing fails silently
- Runtime errors logged but not shown to user
- Compile errors hidden

**Solution**:

```rust
// Add validation to annotation parsing
pub fn parse_script_mode(source: &str) -> Result<ScriptMode, ParseError> {
    let mut annotations = Vec::new();

    for (line_num, line) in source.lines().enumerate() {
        let trimmed = line.trim();

        if trimmed.starts_with("//") {
            let comment = trimmed[2..].trim();

            if comment.starts_with('@') {
                annotations.push((line_num, comment));
            }
        } else if !trimmed.is_empty() {
            break; // Stop at first non-comment
        }
    }

    // Validate annotations
    if annotations.len() > 1 {
        return Err(ParseError::MultipleAnnotations {
            found: annotations,
        });
    }

    match annotations.first() {
        Some((_, "@tool")) => Ok(ScriptMode::Both),
        Some((_, "@editor")) => Ok(ScriptMode::EditorOnly),
        Some((line, unknown)) => Err(ParseError::UnknownAnnotation {
            line: *line,
            annotation: unknown.to_string(),
        }),
        None => Ok(ScriptMode::RuntimeOnly),
    }
}

// Add error display in UI
pub struct ScriptError {
    pub entity: Entity,
    pub error_type: ErrorType,
    pub message: String,
    pub source_location: Option<(PathBuf, usize)>,
    pub timestamp: Instant,
}

pub enum ErrorType {
    Compilation,
    Runtime,
    Annotation,
}
```

**UI Implementation**:
- Add "Console" panel showing errors
- Red border on plugin window with error
- Click error to jump to source location
- Filter by error type

**Benefits**:
- Faster debugging
- Better developer experience
- Prevents silent failures

**Acceptance Criteria**:
- [ ] All parse errors reported with line number
- [ ] Runtime errors shown with stack trace
- [ ] Errors persist until fixed or dismissed
- [ ] Click to jump to error location

---

### ⚡ Phase 2: Enhancement (Medium Priority)

#### 2.1 Reduce UI Input Lag
**Impact**: Medium | **Effort**: High | **Risk**: High

**Current Problem**:
- Command recording pattern causes one-frame delay
- Noticeable in fast interactions (sliders, drag values)
- Root cause: `UiContext` records commands, rendered next frame

**Options**:

**Option A: Immediate Mode (High Risk)**
```rust
// Allow scripts to hold egui::Ui reference directly
pub fn on_ui(ui: &mut egui::Ui) {
    if ui.button("Click").clicked() {
        // Immediate response
    }
}
```
- ✅ Zero latency
- ❌ Breaks lifetime safety
- ❌ Requires unsafe code
- ❌ Scripts could crash editor

**Option B: Double Buffering (Complex)**
```rust
pub struct UiContext {
    current_commands: Vec<UiCommand>,
    next_commands: Vec<UiCommand>,
    current_responses: HashMap<String, UiResponse>,
}
```
- ✅ Maintains safety
- ✅ Reduces latency to half-frame
- ❌ Complex implementation
- ❌ Higher memory usage

**Option C: Hybrid Mode (Recommended)**
```rust
pub enum UiMode {
    Deferred,   // Current command recording
    Immediate,  // Direct egui access (unsafe)
}

// Allow plugins to opt-in to immediate mode
// @immediate
// @editor
pub fn on_ui(ui: &mut egui::Ui) { ... }
```
- ✅ Backwards compatible
- ✅ Opt-in risk
- ❌ Two code paths to maintain
- ✅ Allows experimentation

**Recommendation**:
1. Ship Option C (Hybrid Mode)
2. Mark immediate mode as "experimental"
3. Require explicit annotation
4. Add safety guardrails (timeout, catch panics)

**Acceptance Criteria**:
- [ ] Immediate mode available for opt-in
- [ ] No editor crashes from plugin panics
- [ ] Performance benchmarks show improvement
- [ ] Documentation warns about risks

---

#### 2.2 Plugin Dependencies System
**Impact**: Medium | **Effort**: Medium | **Risk**: Low

**Current Problem**:
- Plugins load in arbitrary order
- No way to declare dependencies
- No version compatibility checks

**Solution**:

```toml
# ui_plugins.toml
[[plugin]]
name = "advanced_tool"
script = "advanced_tool.rn"
enabled = true
default_visible = true
category = "Tools"

# New: Dependencies
dependencies = [
    { name = "core_lib", version = ">=1.0.0" },
    { name = "ui_utils", version = "^2.1" },
]

# New: Provides
provides = [
    { service = "asset_browser", version = "1.0.0" },
]

# New: Conflicts
conflicts = ["old_asset_browser"]
```

**Implementation**:

```rust
pub struct PluginDependency {
    pub name: String,
    pub version_requirement: semver::VersionReq,
}

pub struct PluginService {
    pub name: String,
    pub version: semver::Version,
    pub provider: Entity,
}

impl UiPluginManager {
    pub fn load_plugins_with_dependencies(&mut self) -> Result<LoadOrder, DependencyError> {
        // 1. Parse dependencies
        let mut graph = DependencyGraph::new();
        for plugin in &self.plugins {
            graph.add_node(plugin);
        }

        // 2. Topological sort
        let load_order = graph.topological_sort()?;

        // 3. Check version compatibility
        for plugin in &load_order {
            self.verify_dependencies(plugin)?;
        }

        // 4. Load in order
        for plugin in load_order {
            self.load_plugin(plugin)?;
        }

        Ok(LoadOrder { plugins: load_order })
    }
}
```

**Benefits**:
- Predictable load order
- Clearer plugin relationships
- Version compatibility enforcement

**Acceptance Criteria**:
- [ ] Circular dependencies detected and rejected
- [ ] Missing dependencies show clear error
- [ ] Plugins load in dependency order
- [ ] Version conflicts reported before load

---

#### 2.3 Explicit Context Passing (Remove Thread-Locals)
**Impact**: Medium | **Effort**: Very High | **Risk**: Very High

**Current Problem**:
- State accessed via thread-local storage
- File: `src/scripting/rune/guards.rs`
- Implicit dependencies hard to track
- Blocks parallel script execution

**Solution**:

```rust
// Replace thread-locals with explicit context
pub struct ScriptExecutionContext {
    pub state: Rc<RefCell<ScriptStateMap>>,
    pub commands: Rc<RefCell<ScriptCommands>>,
    pub event_queue: Rc<RefCell<Vec<ScriptEvent>>>,
    pub entity: Entity,
    pub world: *const World,
}

// Inject context into Rune module
impl Module for ScriptApiModule {
    fn install(self, context: &mut rune::compile::Context) -> Result<()> {
        // Register context type
        context.install(&ScriptExecutionContext::module())?;

        // All API functions take context
        context.function_meta(get_state_with_context)?;

        Ok(())
    }
}

// Rune API functions now take context explicitly
#[rune::function]
pub fn get_state(ctx: &ScriptExecutionContext, key: String, default: Value) -> Value {
    ctx.state.borrow_mut().get_or_insert(key, default)
}
```

**Challenges**:
- Rune API constraints (can't pass arbitrary Rust types)
- All API functions must be updated
- Existing scripts need migration
- Performance impact (passing context everywhere)

**Recommendation**:
- **Defer this to Phase 3**
- Current thread-local approach works
- High effort, marginal benefit
- Focus on more impactful improvements first

---

### 🚀 Phase 3: Advanced Features (Low Priority)

#### 3.1 Plugin Sandboxing
**Impact**: Low | **Effort**: Very High | **Risk**: High

**Features**:
- CPU/memory limits per plugin
- Permission system (file access, network, etc.)
- Isolated failure domains
- Resource accounting

**Implementation**:
- Requires VM-level support
- May need custom Rune fork
- Complex to test
- Only needed for untrusted plugins

**Recommendation**: Only pursue if:
- Planning plugin marketplace
- Allowing third-party plugins
- Running untrusted code

---

#### 3.2 Visual Plugin Editor
**Impact**: Low | **Effort**: Very High | **Risk**: Medium

**Features**:
- GUI for creating basic plugins
- Template library
- Live preview
- Visual scripting (node-based)

**Recommendation**:
- Nice-to-have, not essential
- Better to focus on improving text-based workflow
- Consider after core improvements done

---

#### 3.3 Plugin Marketplace
**Impact**: Low | **Effort**: Very High | **Risk**: High

**Features**:
- Discovery and installation
- Version management
- Update notifications
- Ratings and reviews

**Recommendation**:
- Only after Phase 1 & 2 complete
- Requires stable plugin API
- Needs authentication/payment infrastructure
- Consider Obsidian-style community plugins

---

## Implementation Timeline

### Sprint 1 (2 weeks): Foundation Prep
- [x] Fix `on_ui()` signature bug ✅
- [ ] Write comprehensive tests for current architecture
- [ ] Document all data flows
- [ ] Set up benchmarking infrastructure

### Sprint 2-3 (4 weeks): Refactor EditorSharedState
- [ ] Design new module structure
- [ ] Migrate UIState
- [ ] Migrate PluginState
- [ ] Migrate SystemState
- [ ] Update all access patterns
- [ ] Comprehensive testing

### Sprint 4-5 (4 weeks): Plugin Hot Reload
- [ ] Add file watching
- [ ] Implement state serialization
- [ ] Add reload command
- [ ] Build UI controls
- [ ] Error handling
- [ ] Testing with all example plugins

### Sprint 6 (2 weeks): Error Reporting
- [ ] Improve annotation parser
- [ ] Add error display UI
- [ ] Stack trace formatting
- [ ] Source location linking

### Sprint 7-8 (4 weeks): Buffer (Polish & Bugs)
- [ ] Address issues from previous sprints
- [ ] Performance optimization
- [ ] Documentation updates
- [ ] User testing

### Future Sprints (Phase 2+)
- [ ] Plugin dependencies
- [ ] UI lag reduction (if needed)
- [ ] Advanced features (as priority dictates)

---

## Metrics for Success

### Developer Experience
- **Plugin iteration time**: < 3 seconds (from save to reload)
- **Error clarity**: 90% of errors have clear fix
- **Setup time**: < 5 minutes for new plugin developer

### Performance
- **Hot reload time**: < 500ms
- **UI responsiveness**: < 16ms frame time
- **Plugin overhead**: < 1ms per plugin per frame

### Stability
- **Crash rate**: 0 crashes from plugins per 1000 reloads
- **Memory leaks**: 0 after 100 reload cycles
- **Test coverage**: > 80% for core plugin system

### Code Quality
- **Max struct fields**: 10
- **Max function lines**: 50
- **Cyclomatic complexity**: < 10
- **Module coupling**: < 30%

---

## Risk Mitigation

### High-Risk Changes

1. **EditorSharedState Refactor**
   - **Risk**: Breaking existing code
   - **Mitigation**: Incremental migration, comprehensive tests

2. **Hot Reload Implementation**
   - **Risk**: State corruption, memory leaks
   - **Mitigation**: State validation, memory profiling, rollback on error

3. **Immediate Mode UI**
   - **Risk**: Editor crashes from plugin bugs
   - **Mitigation**: Opt-in, panic catching, timeouts

### Technical Debt

Current debt to address:
1. Thread-local state (Phase 3)
2. God object anti-pattern (Phase 1) ✅
3. Fragile annotation parsing (Phase 1) ✅
4. Lack of error boundaries (Phase 1) ✅

---

## Conclusion

### Priority Order
1. **Refactor EditorSharedState** (enables everything else)
2. **Implement hot reload** (biggest impact on dev experience)
3. **Enhanced error reporting** (quick win, high value)
4. **Plugin dependencies** (enables complex plugins)
5. **Reduce UI lag** (only if users complain)
6. **Advanced features** (nice-to-have)

### Success Criteria
After Phase 1 completion:
- ✅ Plugin development 10x faster (hot reload)
- ✅ Code maintainability improved (refactored state)
- ✅ Developer experience better (error reporting)
- ✅ No performance regressions
- ✅ All tests passing
- ✅ Documentation complete

### Next Steps
1. Review this plan with team
2. Validate priorities
3. Set up project tracking (GitHub issues)
4. Begin Sprint 1 (test infrastructure)
5. Start Sprint 2 (EditorSharedState refactor)

---

*This plan is a living document. Update as priorities shift and new information emerges.*
