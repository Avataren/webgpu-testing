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

#[test]
fn despawn_entity_removes_the_subtree_and_is_idempotent() {
    use crate::scene::{Children, Name};

    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let parent = world.spawn((Name::new("Parent"),));
    let child = world.spawn((Name::new("Child"), Parent(parent)));
    world.insert_one(parent, Children(vec![child])).unwrap();
    let bystander = world.spawn((Name::new("Bystander"),));

    let script = spawn_script(&mut world, "despawner", "function noop() end");
    state.process_scripts(&mut world, DT, false).unwrap();
    state
        .eval(
            &mut world,
            &format!(
                "despawn_entity({}); despawn_entity({})",
                parent.to_bits().get() as i64,
                parent.to_bits().get() as i64
            ),
            Some(script),
        )
        .unwrap();

    assert!(!world.contains(parent), "the entity is gone");
    assert!(!world.contains(child), "its children go with it");
    assert!(world.contains(bystander));

    let exists = state
        .eval(
            &mut world,
            &format!(
                "return entity_exists({}), entity_exists({})",
                parent.to_bits().get() as i64,
                bystander.to_bits().get() as i64
            ),
            Some(script),
        )
        .unwrap();
    assert_eq!(
        exists.values,
        vec![serde_json::json!(false), serde_json::json!(true)]
    );
}

#[test]
fn despawn_detaches_the_entity_from_its_parent() {
    use crate::scene::{Children, Name};

    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let parent = world.spawn((Name::new("Parent"),));
    let first = world.spawn((Name::new("First"), Parent(parent)));
    let second = world.spawn((Name::new("Second"), Parent(parent)));
    world
        .insert_one(parent, Children(vec![first, second]))
        .unwrap();

    state
        .eval(
            &mut world,
            &format!("despawn_entity({})", first.to_bits().get() as i64),
            None,
        )
        .unwrap();

    assert_eq!(world.get::<&Children>(parent).unwrap().0, vec![second]);
}

#[test]
fn despawning_an_entity_spawned_in_the_same_chunk_creates_nothing() {
    use crate::scene::Name;

    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    state
        .eval(
            &mut world,
            r#"
                local keep = spawn_entity("Keep")
                local drop = spawn_entity("Drop")
                set_translation(drop, 1, 2, 3)
                despawn_entity(drop)
            "#,
            None,
        )
        .unwrap();

    let names: Vec<String> = world
        .query::<&Name>()
        .iter()
        .map(|(_, name)| name.0.clone())
        .collect();
    assert_eq!(names, vec!["Keep".to_string()]);
}

#[test]
fn get_time_counts_script_time_and_restarts_on_reset() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let entity = spawn_script(
        &mut world,
        "clock",
        "function update(self_entity, dt) now = get_time() end",
    );
    for _ in 0..30 {
        state.process_scripts(&mut world, 0.1, false).unwrap();
    }
    let now = match state.script_global(&world, entity, "now") {
        Some(mlua::Value::Number(value)) => value,
        other => panic!("now: {other:?}"),
    };
    assert!((now - 3.0).abs() < 1e-6, "got {now}");

    state.reset_runtime();
    state.process_scripts(&mut world, 0.1, false).unwrap();
    let now = match state.script_global(&world, entity, "now") {
        Some(mlua::Value::Number(value)) => value,
        other => panic!("now: {other:?}"),
    };
    assert!((now - 0.1).abs() < 1e-6, "got {now}");
}

#[test]
fn direction_helpers_follow_the_entity_rotation() {
    use glam::Quat;

    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let ship = spawn_script(&mut world, "ship", "function noop() end");
    // A quarter turn about Y (counter-clockwise from above) turns -Z toward -X.
    world
        .insert_one(
            ship,
            TransformComponent(Transform {
                rotation: Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
                ..Transform::default()
            }),
        )
        .unwrap();
    state.process_scripts(&mut world, DT, false).unwrap();

    let bits = ship.to_bits().get() as i64;
    let result = state
        .eval(
            &mut world,
            &format!(
                "local f = get_forward({bits}); local r = get_right({bits}); \
                 local u = get_up({bits}); return f.x, f.z, r.z, u.y"
            ),
            Some(ship),
        )
        .unwrap();
    let values: Vec<f64> = result.values.iter().filter_map(|v| v.as_f64()).collect();
    assert_eq!(values.len(), 4);
    assert!((values[0] + 1.0).abs() < 1e-5, "forward.x {}", values[0]);
    assert!(values[1].abs() < 1e-5, "forward.z {}", values[1]);
    assert!((values[2] + 1.0).abs() < 1e-5, "right.z {}", values[2]);
    assert!((values[3] - 1.0).abs() < 1e-5, "up.y {}", values[3]);
}

#[test]
fn hot_reload_runs_on_destroyed_with_a_reason_before_recreating() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pool.lua");
    std::fs::write(
        &path,
        r#"
            function on_created(self_entity)
                created = (created or 0) + 1
                pool = spawn_entity("PoolMember")
            end
            function on_destroyed(self_entity, reason)
                despawn_entity(pool)
                destroyed_because = reason
            end
        "#,
    )
    .unwrap();

    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let entity = world.spawn((LuaScriptComponent::new(
        crate::scripting::LuaScriptSource::file(&path),
    ),));
    state.process_scripts(&mut world, DT, false).unwrap();
    assert_eq!(count_named(&world, "PoolMember"), 1);

    // Editing the script reloads it: the old instance cleans up, the new one spawns its
    // own pool, so exactly one set exists afterwards.
    assert_eq!(state.reload_path(&path), 1);
    state.process_scripts(&mut world, DT, false).unwrap();
    state.process_scripts(&mut world, DT, false).unwrap();
    assert_eq!(state.last_error(entity), None);
    assert_eq!(count_named(&world, "PoolMember"), 1, "no leaked pool");

    // The new instance has not been destroyed, so remove the component to see the
    // "removed" reason.
    world.remove_one::<LuaScriptComponent>(entity).unwrap();
    state.process_scripts(&mut world, DT, false).unwrap();
    assert_eq!(count_named(&world, "PoolMember"), 0);
}

fn count_named(world: &World, wanted: &str) -> usize {
    world
        .query::<&crate::scene::Name>()
        .iter()
        .filter(|(_, name)| name.0 == wanted)
        .count()
}

#[test]
fn parenting_in_the_spawning_chunk_keeps_child_lists_so_despawn_takes_the_children() {
    use crate::scene::Children;

    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    state
        .eval(
            &mut world,
            r#"
                local parent = spawn_entity("P")
                local child = spawn_entity("C")
                set_parent(child, parent)
            "#,
            None,
        )
        .unwrap();

    let parent = world
        .query::<&crate::scene::Name>()
        .iter()
        .find(|(_, name)| name.0 == "P")
        .map(|(entity, _)| entity)
        .unwrap();
    assert_eq!(world.get::<&Children>(parent).unwrap().0.len(), 1);

    state
        .eval(&mut world, "despawn_entity(find_entity_by_name('P'))", None)
        .unwrap();
    assert_eq!(world.query::<&crate::scene::Name>().iter().count(), 0);
}

#[test]
fn hot_reload_hands_on_destroyed_result_to_the_new_on_created_and_follows_modules() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("game.lua");
    let module = dir.path().join("waves.lua");
    std::fs::write(&module, "return { first = 1 }").unwrap();
    std::fs::write(
        &path,
        r#"
            local waves = require("waves")
            function on_created(self_entity, previous)
                GAME = previous or { wave = waves.first, created = 0 }
                GAME.created = GAME.created + 1
            end
            function on_destroyed(self_entity, reason)
                if reason == "reload" then return GAME end
            end
        "#,
    )
    .unwrap();

    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let entity = world.spawn((LuaScriptComponent::new(
        crate::scripting::LuaScriptSource::file(&path),
    ),));
    state.process_scripts(&mut world, DT, false).unwrap();
    state
        .eval(&mut world, "GAME.wave = 7", Some(entity))
        .unwrap();

    // Editing the required module reloads the script that uses it, and the game state
    // survives because on_destroyed returned it.
    assert_eq!(state.reload_path(&module), 1);
    state.process_scripts(&mut world, DT, false).unwrap();
    state.process_scripts(&mut world, DT, false).unwrap();
    assert_eq!(state.last_error(entity), None);
    let result = state
        .eval(&mut world, "return GAME.wave, GAME.created", Some(entity))
        .unwrap();
    assert_eq!(
        result.values,
        vec![serde_json::json!(7), serde_json::json!(2)]
    );

    // A reset (play start or stop) starts fresh.
    state.reset_runtime();
    state.process_scripts(&mut world, DT, false).unwrap();
    let result = state
        .eval(&mut world, "return GAME.wave, GAME.created", Some(entity))
        .unwrap();
    assert_eq!(
        result.values,
        vec![serde_json::json!(1), serde_json::json!(1)]
    );
}

#[test]
fn hot_reload_keeps_spawned_handles_mutable_and_new_handles_distinct() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let entity = spawn_script(
        &mut world,
        "handle_reload",
        r#"
        function on_created(self_entity, previous)
            GAME = previous or {ticks = 0, pool = {
                spawn_entity("PooledA"), spawn_entity("PooledB")
            }}
            if previous then
                GAME.extra = spawn_entity("AfterReload" .. GAME.ticks)
            end
        end
        function on_destroyed(self_entity, reason)
            if reason == "reload" then return GAME end
        end
        function update(self_entity, dt)
            GAME.ticks = GAME.ticks + 1
            set_translation(GAME.pool[1], GAME.ticks, 1, 0)
            set_translation(GAME.pool[2], GAME.ticks, 2, 0)
            if GAME.extra then set_translation(GAME.extra, GAME.ticks, 3, 0) end
        end
    "#,
    );
    state.process_scripts(&mut world, DT, false).unwrap();
    let pool = world
        .query::<&crate::scene::Name>()
        .iter()
        .filter(|(_, name)| name.0.starts_with("Pooled"))
        .map(|(entity, name)| (entity, name.0.clone()))
        .collect::<Vec<_>>();
    assert_eq!(pool.len(), 2);
    for reload in 1..=2 {
        assert!(state.reload_entity(entity));
        for _ in 0..3 {
            state.process_scripts(&mut world, DT, false).unwrap();
        }
        assert_eq!(state.last_error(entity), None);
        for (pooled, name) in &pool {
            let pos = world
                .get::<&TransformComponent>(*pooled)
                .unwrap()
                .0
                .translation;
            assert_eq!(
                pos.x,
                (1 + reload * 3) as f32,
                "preserved handle no longer mutates {name}"
            );
            assert_eq!(pos.y, if name == "PooledA" { 1.0 } else { 2.0 });
        }
        let newest = format!("AfterReload{}", 1 + (reload - 1) * 3);
        let extra = world
            .query::<(&crate::scene::Name, &TransformComponent)>()
            .iter()
            .find(|(_, (name, _))| name.0 == newest)
            .map(|(_, (_, t))| t.0.translation)
            .unwrap();
        assert_eq!(extra, Vec3::new((1 + reload * 3) as f32, 3.0, 0.0));
        assert_eq!(world.len(), 3 + reload);
    }
}

#[test]
fn spawn_primitive_can_share_a_material() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    state
        .eval(
            &mut world,
            r#"
                local a = spawn_primitive("cube", "A")
                local b = spawn_primitive("cube", "B", { material_of = a })
                share_material(spawn_entity("C"), a)
            "#,
            None,
        )
        .unwrap();
    let commands = state.take_pending_asset_commands();
    let shares: Vec<_> = commands
        .iter()
        .filter_map(|pending| match pending.command {
            crate::scripting::lua::AssetCommand::ShareMaterial(source) => Some(source),
            _ => None,
        })
        .collect();
    assert_eq!(shares.len(), 2);
    let a = world
        .query::<&crate::scene::Name>()
        .iter()
        .find(|(_, name)| name.0 == "A")
        .map(|(entity, _)| entity)
        .unwrap();
    // Queued handles are resolved to the source's entity id.
    assert!(shares
        .iter()
        .all(|&source| source as u64 == a.to_bits().get()));
}

#[test]
fn a_bare_spawned_group_node_carries_its_children_along() {
    use crate::scene::internal::transforms::propagate_transforms;
    use crate::scene::Name;

    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    state
        .eval(
            &mut world,
            r#"
                local root = spawn_entity("Root")
                set_translation(root, 5, 0, 0)
                local group = spawn_entity("Group") -- never moved: still needs a transform
                set_parent(group, root)
                local leaf = spawn_entity("Leaf")
                set_parent(leaf, group)
                set_translation(leaf, 0, 1, 0)
            "#,
            None,
        )
        .unwrap();
    propagate_transforms(&mut world);

    let leaf = world
        .query::<&Name>()
        .iter()
        .find(|(_, name)| name.0 == "Leaf")
        .map(|(entity, _)| entity)
        .unwrap();
    let position = world.get::<&WorldTransform>(leaf).unwrap().0.translation;
    assert_eq!(position, Vec3::new(5.0, 1.0, 0.0));
}

#[test]
fn voxel_models_mesh_per_key_and_spawn_one_child_per_part() {
    use crate::scene::Name;
    use crate::scripting::lua::AssetCommand;

    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let result = state
        .eval(
            &mut world,
            r##"
                local m = voxel_model{
                    rows = { "RRR", "RGR", "RRR" }, depth = 2, voxel = 0.5,
                    legend = { R = "red", G = "#00ff00" },
                }
                local shared = spawn_primitive("cube", "Shared")
                local root, parts = spawn_voxels(m, "Box", {
                    materials = { red = shared }, cast_shadow = false,
                })
                -- A fill function: a 4x4x4 ball of key 1 (true).
                local ball = voxel_model{ size = {4, 4, 4}, fill = function(x, y, z)
                    return (x - 1.5)^2 + (y - 1.5)^2 + (z - 1.5)^2 < 4
                end }
                return #m.parts, m.parts[1].key, m.parts[2].key, m.voxels, m.quads,
                    m.cube_quads, parts.red ~= nil and parts["#00ff00"] ~= nil,
                    ball.parts[1].key, ball.voxels, m.max[2], tostring(m.parts[1].mesh)
            "##,
            None,
        )
        .unwrap();
    let values = result.values;
    assert_eq!(values[0], serde_json::json!(2));
    assert_eq!(values[1], serde_json::json!("red"));
    assert_eq!(values[2], serde_json::json!("#00ff00"));
    assert_eq!(values[3], serde_json::json!(18));
    // The green voxels sit inside the red frame but show on the front and back faces.
    assert!(values[4].as_u64().unwrap() < values[5].as_u64().unwrap() / 4);
    assert_eq!(values[6], serde_json::json!(true));
    assert_eq!(values[7], serde_json::json!(1));
    assert_eq!(values[8], serde_json::json!(32));
    assert_eq!(values[9], serde_json::json!(1.5));
    assert!(values[10].as_str().unwrap().starts_with("GeneratedMesh("));

    let names: Vec<String> = world
        .query::<&Name>()
        .iter()
        .map(|(_, name)| name.0.clone())
        .collect();
    for expected in ["Box", "Box.red", "Box.#00ff00"] {
        assert!(
            names.iter().any(|n| n == expected),
            "{expected} missing from {names:?}"
        );
    }
    let commands = state.take_pending_asset_commands();
    let meshes = commands
        .iter()
        .filter(|pending| matches!(pending.command, AssetCommand::GeneratedMesh(_)))
        .count();
    let shares = commands
        .iter()
        .filter(|pending| matches!(pending.command, AssetCommand::ShareMaterial(_)))
        .count();
    let patches: Vec<_> = commands
        .iter()
        .filter_map(|pending| match &pending.command {
            AssetCommand::Material(patch) => Some(patch.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(meshes, 2);
    assert_eq!(shares, 1, "red shares Shared's material");
    // The green key is a colour, so it colours its own part.
    assert_eq!(patches, vec![serde_json::json!({ "color": "#00ff00" })]);
    let no_shadow = world
        .query::<(&Name, &crate::scene::CanCastShadow)>()
        .iter()
        .filter(|(_, (name, cast))| name.0.starts_with("Box.") && !cast.0)
        .count();
    assert_eq!(no_shadow, 2);
}

#[test]
fn voxel_look_groups_keys_and_colours_vertices() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let result = state
        .eval(
            &mut world,
            r##"
                local m = voxel_model{
                    rows = { "ABC" }, ao = 0,
                    colors = { A = "#ff0000", B = { 0, 0, 1 } },
                    parts = { C = "trim" }, part = "body",
                }
                local root, parts = spawn_voxels(m, "Look")
                local keys = m.parts[1].keys
                local bad = pcall(voxel_model, { rows = { "A" }, colors = { A = "red" } })
                return #m.parts, m.parts[1].key, m.parts[2].key, keys[1], keys[2],
                    m.parts[1].voxels, parts.body ~= nil and parts.trim ~= nil, bad
            "##,
            None,
        )
        .unwrap();
    assert_eq!(
        result.values,
        vec![
            serde_json::json!(2),
            serde_json::json!("body"),
            serde_json::json!("trim"),
            serde_json::json!("A"),
            serde_json::json!("B"),
            serde_json::json!(2),
            serde_json::json!(true),
            serde_json::json!(false),
        ]
    );
}

#[test]
fn voxel_model_errors_name_the_problem() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    for (source, expected) in [
        (r#"voxel_model{ rows = { "AA", "A" } }"#, "row 2"),
        (
            r#"voxel_model{ rows = { "AX" }, legend = { A = 1 } }"#,
            "'X'",
        ),
        (
            r#"voxel_model{ size = {2, 2, 2}, fill = function() error("boom") end }"#,
            "fill(0, 0, 0)",
        ),
        (
            r#"voxel_model{ size = {2, 2, 2} }"#,
            "needs rows, layers, voxels or fill",
        ),
        (r#"set_mesh(spawn_entity("E"), true)"#, "set_mesh expects"),
    ] {
        let err = state
            .eval(&mut world, source, None)
            .unwrap_err()
            .to_string();
        assert!(err.contains(expected), "{source}: {err}");
    }
}

#[test]
fn animator_functions_queue_playback_and_find_bones() {
    use crate::scene::components::{Children, Name};
    use crate::scene::skeletal::Animator;

    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let character = spawn_script(
        &mut world,
        "animated",
        r#"
            function on_created(self_entity)
                animator_play(self_entity, "Walk", { fade = 0.5, speed = 2 })
                animator_set_layer(self_entity, 1, { weight = 0.5, mask = "Spine" })
                animator_set_root_motion(self_entity, "apply")
                hand = find_bone(self_entity, "LeftHand")
                missing = find_bone(self_entity, "Tail")
            end
        "#,
    );
    let hand = world.spawn((
        Name::new("mixamorig:LeftHand"),
        TransformComponent(Transform::default()),
        Parent(character),
    ));
    world.insert_one(character, Children(vec![hand])).unwrap();
    world
        .insert_one(character, Animator::with_library("walk.glb"))
        .unwrap();

    state.process_scripts(&mut world, DT, false).unwrap();

    let animator = world.get::<&Animator>(character).unwrap();
    let layer = &animator.state.layers[0];
    assert_eq!(layer.clips[0].name, "Walk");
    assert_eq!(layer.clips[0].speed, 2.0);
    assert_eq!(animator.state.layers[1].mask.as_deref(), Some("spine"));
    assert_eq!(animator.root_motion, crate::scene::RootMotion::Apply);
    assert_eq!(
        global_i64(&state, &world, character, "hand"),
        Some(hand.to_bits().get() as i64)
    );
    assert!(matches!(
        state.script_global(&world, character, "missing"),
        None | Some(mlua::Value::Nil)
    ));
}

#[test]
fn clone_entity_copies_a_character_that_plays_in_the_same_chunk() {
    use crate::scene::skeletal::{Animator, SkinnedMesh};
    use crate::scene::{Children, Name, Visible};
    use std::sync::Arc;

    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let stage = world.spawn((Name::new("Stage"),));
    let dancer = world.spawn((
        Name::new("Dancer"),
        Parent(stage),
        TransformComponent(Transform::from_translation(Vec3::Y)),
        Visible(false),
        Animator::with_library("dance.glb"),
    ));
    let hips = world.spawn((Name::new("Hips"), Parent(dancer)));
    let body = world.spawn((
        Name::new("Body"),
        Parent(dancer),
        SkinnedMesh {
            joints: vec![hips],
            inverse_bind: Arc::from(vec![glam::Mat4::IDENTITY]),
        },
    ));
    world.insert_one(stage, Children(vec![dancer])).unwrap();
    world
        .insert_one(dancer, Children(vec![hips, body]))
        .unwrap();

    state
        .eval(
            &mut world,
            &format!(
                r#"
                    local a = clone_entity({dancer}, "Copy A")
                    set_translation(a, 5, 0, 0)
                    set_component(a, "Visible", true)
                    animator_play(a, "Dance", {{ fade = 0 }})
                    local b = clone_entity({dancer}, "Copy B")
                    set_parent(b, nil)
                "#,
                dancer = dancer.to_bits().get() as i64
            ),
            None,
        )
        .unwrap();

    let named = |name: &str| {
        world
            .query::<&Name>()
            .iter()
            .find(|(_, n)| n.0 == name)
            .map(|(entity, _)| entity)
            .unwrap_or_else(|| panic!("no entity named {name}"))
    };
    let (a, b) = (named("Copy A"), named("Copy B"));
    assert_eq!(
        world.get::<&Parent>(a).unwrap().0,
        stage,
        "a sibling of the source"
    );
    assert!(
        world.get::<&Parent>(b).is_err(),
        "set_parent(nil) unparents a clone"
    );
    assert_eq!(world.get::<&Children>(stage).unwrap().0, vec![dancer, a]);
    assert_eq!(
        world.get::<&TransformComponent>(a).unwrap().0.translation,
        Vec3::new(5.0, 0.0, 0.0)
    );
    assert_eq!(
        world.get::<&TransformComponent>(b).unwrap().0.translation,
        Vec3::Y,
        "a clone starts from its source's transform"
    );
    assert!(world.get::<&Visible>(a).unwrap().0);
    assert!(!world.get::<&Visible>(b).unwrap().0);

    let a_children = world.get::<&Children>(a).unwrap().0.clone();
    assert_eq!(a_children.len(), 2);
    let a_body = a_children
        .iter()
        .copied()
        .find(|&child| world.get::<&SkinnedMesh>(child).is_ok())
        .unwrap();
    let joint = world.get::<&SkinnedMesh>(a_body).unwrap().joints[0];
    assert!(
        a_children.contains(&joint),
        "the copy deforms with its own bones"
    );
    assert_eq!(
        world.get::<&Animator>(a).unwrap().status(0).clip.as_deref(),
        Some("Dance"),
        "animator_play reached the clone made in the same chunk"
    );
    assert!(world
        .get::<&Animator>(dancer)
        .unwrap()
        .state
        .layers
        .is_empty());
}

#[test]
fn reading_functions_accept_spawned_handles_in_later_hooks() {
    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let script = spawn_script(&mut world, "spawner", "function noop() end");
    state.process_scripts(&mut world, DT, false).unwrap();
    state
        .eval(
            &mut world,
            r#"
                PARENT = spawn_entity("Parent")
                CHILD = spawn_entity("Child")
                set_parent(CHILD, PARENT)
                set_translation(PARENT, 1, 2, 3)
                READ_NOW = entity_exists(PARENT)
            "#,
            Some(script),
        )
        .unwrap();
    let output = state
        .eval(
            &mut world,
            "return READ_NOW, entity_exists(PARENT), #get_children(PARENT), get_component(PARENT, 'Transform').translation[2], get_parent(CHILD) ~= nil",
            Some(script),
        )
        .unwrap();
    assert_eq!(
        output.values,
        vec![
            serde_json::json!(false),
            serde_json::json!(true),
            serde_json::json!(1),
            serde_json::json!(2.0),
            serde_json::json!(true)
        ],
        "a handle reads like the entity id from the next hook on"
    );
}

#[test]
fn despawning_many_subtrees_in_one_hook_removes_every_descendant() {
    use crate::scene::{Children, Name};

    let mut state = ScriptingState::new().unwrap();
    let mut world = World::new();
    let mut roots = Vec::new();
    let mut everything = Vec::new();
    for i in 0..20 {
        let root = world.spawn((Name::new(format!("Actor {i}")),));
        let listed = world.spawn((Parent(root),));
        // Linked only by its Parent: the root's child list misses it.
        let unlisted = world.spawn((Parent(root),));
        let grandchild = world.spawn((Parent(listed),));
        world.insert_one(root, Children(vec![listed])).unwrap();
        world
            .insert_one(listed, Children(vec![grandchild]))
            .unwrap();
        roots.push(root.to_bits().get() as i64);
        everything.extend([root, listed, unlisted, grandchild]);
    }
    let bystander = world.spawn((Name::new("Bystander"),));

    let list = roots
        .iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    state
        .eval(
            &mut world,
            &format!("for _, e in ipairs({{ {list} }}) do despawn_entity(e) end"),
            None,
        )
        .unwrap();

    assert!(everything.iter().all(|&entity| !world.contains(entity)));
    assert!(world.contains(bystander));
}
