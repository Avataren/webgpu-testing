//! Regression tests for script lifecycle bugs.

use glam::Vec3;
use hecs::{Entity, World};

use crate::scene::transform::Transform;
use crate::scene::{Parent, TransformComponent, WorldTransform};
use crate::scripting::lua::component::LuaScriptComponent;
use crate::scripting::lua::state::ScriptingState;

const DT: f64 = 1.0 / 60.0;

fn spawn_script(world: &mut World, name: &str, source: &str) -> Entity {
    world.spawn((
        TransformComponent(Transform::default()),
        LuaScriptComponent::new_inline(name, source),
    ))
}

fn global_i64(state: &ScriptingState, world: &World, entity: Entity, name: &str) -> Option<i64> {
    match state.script_global(world, entity, name)? {
        mlua::Value::Integer(value) => Some(value),
        mlua::Value::Number(value) => Some(value as i64),
        _ => None,
    }
}

fn global_string(
    state: &ScriptingState,
    world: &World,
    entity: Entity,
    name: &str,
) -> Option<String> {
    match state.script_global(world, entity, name)? {
        mlua::Value::String(value) => Some(value.to_string_lossy()),
        _ => None,
    }
}

#[test]
fn update_can_read_world_state() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let entity = spawn_script(
        &mut world,
        "world_read",
        r#"
            function update(self_entity, dt)
                local pos = get_world_translation(self_entity)
                seen_x = pos and pos.x or -1
            end
        "#,
    );
    let transform = Transform {
        translation: Vec3::new(7.0, 0.0, 0.0),
        ..Transform::default()
    };
    world.insert_one(entity, WorldTransform(transform)).unwrap();

    state.process_scripts(&mut world, DT, false).unwrap();

    assert_eq!(global_i64(&state, &world, entity, "seen_x"), Some(7));
}

#[test]
fn coroutine_wait_suspends_and_resumes() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let entity = spawn_script(
        &mut world,
        "coroutine_wait",
        r#"
            step = 0
            function on_created(self_entity)
                coroutine_create(function()
                    step = 1
                    wait(0.04)
                    step = 2
                    wait_frames(2)
                    step = 3
                end)
            end
        "#,
    );

    state.process_scripts(&mut world, DT, false).unwrap();
    assert_eq!(global_i64(&state, &world, entity, "step"), Some(1));

    // 0.04s at 60Hz elapses within three frames; wait_frames(2) needs two more.
    for _ in 0..3 {
        state.process_scripts(&mut world, DT, false).unwrap();
    }
    assert_eq!(global_i64(&state, &world, entity, "step"), Some(2));

    for _ in 0..3 {
        state.process_scripts(&mut world, DT, false).unwrap();
    }
    assert_eq!(global_i64(&state, &world, entity, "step"), Some(3));
}

#[test]
fn self_triggering_event_is_bounded() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let entity = spawn_script(
        &mut world,
        "event_loop",
        r#"
            count = 0
            function on_created(self_entity)
                subscribe_event("ping", "on_ping")
                emit_event("ping", {})
            end
            function on_ping(data)
                count = count + 1
                emit_event("ping", {})
            end
        "#,
    );

    state.process_scripts(&mut world, DT, false).unwrap();

    let count = global_i64(&state, &world, entity, "count").unwrap();
    assert!(count > 0, "handler never ran");
    assert!(count <= 10_000, "event cascade was not bounded: {count}");
}

#[test]
fn replacing_component_recompiles_script() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let entity = spawn_script(
        &mut world,
        "reload",
        r#"function on_created(self_entity) version = "a" end"#,
    );
    state.process_scripts(&mut world, DT, false).unwrap();
    assert_eq!(
        global_string(&state, &world, entity, "version").as_deref(),
        Some("a")
    );

    world
        .insert_one(
            entity,
            LuaScriptComponent::new_inline(
                "reload",
                r#"function on_created(self_entity) version = "b" end"#,
            ),
        )
        .unwrap();
    state.process_scripts(&mut world, DT, false).unwrap();

    assert_eq!(
        global_string(&state, &world, entity, "version").as_deref(),
        Some("b")
    );
}

#[test]
fn cloned_component_runs_on_created_in_fresh_runtime() {
    // Mirrors play-mode snapshot/restore: components are copied into a new
    // world that is driven by a brand-new scripting state.
    let source = "-- @tool\nfunction on_created(self_entity) created = 1 end";
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let entity = spawn_script(&mut world, "snapshot", source);
    state.process_scripts(&mut world, DT, true).unwrap();

    let component = LuaScriptComponent::clone(&world.get::<&LuaScriptComponent>(entity).unwrap());
    let mut restored_state = ScriptingState::new().unwrap();
    let mut restored_world = World::new();
    let restored = restored_world.spawn((TransformComponent(Transform::default()), component));
    restored_state
        .process_scripts(&mut restored_world, DT, false)
        .unwrap();

    assert_eq!(
        global_i64(&restored_state, &restored_world, restored, "created"),
        Some(1)
    );
}

#[test]
fn reset_runtime_reruns_on_created() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let entity = spawn_script(
        &mut world,
        "reset",
        r#"
            -- @tool
            created = (created or 0) + 1
            function on_created(self_entity) runs = 1 end
        "#,
    );
    state.process_scripts(&mut world, DT, true).unwrap();
    state.reset_runtime();
    state.process_scripts(&mut world, DT, true).unwrap();

    assert_eq!(global_i64(&state, &world, entity, "runs"), Some(1));
}

#[test]
fn reset_runtime_preserves_script_state() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let entity = spawn_script(
        &mut world,
        "persist",
        r#"
            function on_created(self_entity)
                set_f64("counter", 0)
            end
            function update(self_entity, dt)
                set_f64("counter", get_f64("counter", 0) + 1)
                counter = get_f64("counter", 0)
            end
        "#,
    );
    for _ in 0..3 {
        state.process_scripts(&mut world, DT, false).unwrap();
    }
    state.reset_runtime();
    state.process_scripts(&mut world, DT, false).unwrap();

    // on_created reset the counter to 0, then restored state (3) won, then update ran.
    assert_eq!(global_i64(&state, &world, entity, "counter"), Some(4));
}

#[test]
fn compile_error_is_recorded_and_other_scripts_keep_running() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let broken = spawn_script(&mut world, "broken", "function update(self_entity, dt)");
    let healthy = spawn_script(
        &mut world,
        "healthy",
        "function update(self_entity, dt) ticks = (ticks or 0) + 1 end",
    );

    state.process_scripts(&mut world, DT, false).unwrap();
    state.process_scripts(&mut world, DT, false).unwrap();

    let error = state.last_error(broken).expect("compile error recorded");
    assert!(error.starts_with("compile"), "{error}");
    assert_eq!(global_i64(&state, &world, healthy, "ticks"), Some(2));
}

#[test]
fn stale_entity_command_does_not_block_other_commands() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let ghost = world.spawn(());
    world.despawn(ghost).unwrap();
    let ghost_bits = ghost.to_bits().get();

    let entity = spawn_script(
        &mut world,
        "stale",
        &format!(
            r#"
            function update(self_entity, dt)
                set_translation({ghost_bits}, 1, 1, 1)
                set_translation(self_entity, 5, 0, 0)
            end
            "#
        ),
    );
    let other = spawn_script(
        &mut world,
        "other",
        "function update(self_entity, dt) set_translation(self_entity, 9, 0, 0) end",
    );

    state.process_scripts(&mut world, DT, false).unwrap();

    let x = |e: Entity| world.get::<&TransformComponent>(e).unwrap().0.translation.x;
    assert_eq!(x(entity), 5.0);
    assert_eq!(x(other), 9.0);
}

#[test]
fn on_ui_commands_are_applied_next_update() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let entity = spawn_script(
        &mut world,
        "ui_cmd",
        r#"
            -- @tool
            function on_ui(self_entity, ui)
                set_translation(self_entity, 3, 0, 0)
            end
        "#,
    );
    state.process_scripts(&mut world, DT, true).unwrap();
    state.process_ui(&world, true, None, None);
    state.process_scripts(&mut world, DT, true).unwrap();

    let x = world
        .get::<&TransformComponent>(entity)
        .unwrap()
        .0
        .translation
        .x;
    assert_eq!(x, 3.0);
}

#[test]
fn set_parent_rejects_cycles() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let a = world.spawn((TransformComponent(Transform::default()),));
    let a_bits = a.to_bits().get();
    let b = spawn_script(
        &mut world,
        "cycle",
        &format!(
            r#"
            function on_created(self_entity)
                set_parent({a_bits}, self_entity)
                set_parent(self_entity, {a_bits})
            end
            "#
        ),
    );

    state.process_scripts(&mut world, DT, false).unwrap();

    assert_eq!(world.get::<&Parent>(a).unwrap().0, b);
    assert!(world.get::<&Parent>(b).is_err(), "cycle was created");
}

#[test]
fn reload_path_swaps_code_and_keeps_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hot.lua");
    std::fs::write(
        &path,
        r#"
            function update(self_entity, dt)
                set_f64("n", get_f64("n", 0) + 1)
                version, n = "old", get_f64("n", 0)
            end
        "#,
    )
    .unwrap();

    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let file = crate::scripting::LuaScriptSource::file(&path);
    let entity = world.spawn((LuaScriptComponent::new(file.clone()),));
    let untouched = spawn_script(
        &mut world,
        "untouched",
        "function on_created(self_entity) created = (created or 0) + 1 end",
    );
    state.process_scripts(&mut world, DT, false).unwrap();
    state.process_scripts(&mut world, DT, false).unwrap();

    std::fs::write(
        &path,
        r#"
            function update(self_entity, dt)
                set_f64("n", get_f64("n", 0) + 1)
                version, n = "new", get_f64("n", 0)
            end
        "#,
    )
    .unwrap();
    assert_eq!(state.reload_path(&path), 1);
    state.process_scripts(&mut world, DT, false).unwrap();
    assert_eq!(state.last_error(entity), None);

    assert_eq!(
        global_string(&state, &world, entity, "version").as_deref(),
        Some("new")
    );
    assert_eq!(global_i64(&state, &world, entity, "n"), Some(3));
    // Unrelated scripts are not restarted.
    assert_eq!(global_i64(&state, &world, untouched, "created"), Some(1));
}

#[test]
fn numeric_state_getters_honour_defaults() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let entity = spawn_script(
        &mut world,
        "defaults",
        r#"
            function on_created(self_entity)
                speed = get_f64("speed", 2.5)
                hp = get_i32("hp", 100)
                set_i32("hp", 42)
                hp_after = get_i32("hp", 0)
                ok, err = pcall(get_f64, "missing")
            end
        "#,
    );
    state.process_scripts(&mut world, DT, false).unwrap();

    let number = |name| match state.script_global(&world, entity, name) {
        Some(mlua::Value::Number(n)) => n,
        Some(mlua::Value::Integer(n)) => n as f64,
        other => panic!("{name}: {other:?}"),
    };
    assert_eq!(number("speed"), 2.5);
    assert_eq!(number("hp"), 100.0);
    assert_eq!(number("hp_after"), 42.0);
    assert!(matches!(
        state.script_global(&world, entity, "ok"),
        Some(mlua::Value::Boolean(false))
    ));
}

#[test]
fn bundled_lua_scripts_compile() {
    let runtime = crate::scripting::LuaScriptingRuntime::new().unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut checked = 0;
    for dir in ["examples/scripts", "scripts"] {
        let Ok(entries) = std::fs::read_dir(root.join(dir)) else {
            continue;
        };
        for path in entries.filter_map(Result::ok).map(|entry| entry.path()) {
            if path.extension().is_some_and(|ext| ext == "lua") {
                let source = crate::scripting::LuaScriptSource::file(&path);
                if let Err(err) = runtime.compile(&source) {
                    panic!("{} failed to compile: {err}", path.display());
                }
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "no bundled scripts found");
}
