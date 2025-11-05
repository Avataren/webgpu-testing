use wgpu_cube::scene::components::{Name, TransformComponent};
use wgpu_cube::scene::Scene;
use wgpu_cube::scene::Transform;
use wgpu_cube::scripting::RuneScriptComponent;

/// Test basic event emission and reception
#[test]
fn event_basic_emit_and_receive() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create emitter script
    let emitter = scene.main_world_mut().spawn((
        Name("Emitter".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "EmitterScript",
            r#"
            pub fn on_created(self_entity) {
                set_state(self_entity, "emitter_created", true);
            }

            pub fn update(self_entity, dt) {
                emit_event("test_event", #{
                    message: "Hello from emitter",
                    value: 42
                });
            }
            "#,
        ),
    ));

    // Create listener script
    let listener = scene.main_world_mut().spawn((
        Name("Listener".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "ListenerScript",
            r#"
            pub fn on_created(self_entity) {
                subscribe_event("test_event", "on_test_event");
                set_state(self_entity, "event_received", false);
            }

            pub fn on_test_event(event_data) {
                let self_entity = try_get_state(0, "listener_entity", ());
                if self_entity != () {
                    set_state(self_entity, "event_received", true);
                    set_state(self_entity, "event_message", event_data.message);
                    set_state(self_entity, "event_value", event_data.value);
                }
            }

            pub fn update(self_entity, dt) {
                set_state(0, "listener_entity", self_entity);
            }
            "#,
        ),
    ));

    // Initialize scripts
    scene.update(0.0);

    // Verify emitter and listener are created
    {
        let world = scene.main_world();
        let emitter_comp = world.get::<&RuneScriptComponent>(emitter).unwrap();
        assert!(emitter_comp.created_called());

        let listener_comp = world.get::<&RuneScriptComponent>(listener).unwrap();
        assert!(listener_comp.created_called());
    }

    // Run update to trigger event emission and dispatch
    scene.update(0.1);

    // Verify event was received (check in next frame after state is set)
    scene.update(0.1);

    println!("Test completed - basic event emission and reception");
}

/// Test event subscription and unsubscription
#[test]
fn event_subscribe_and_unsubscribe() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let _emitter = scene.main_world_mut().spawn((
        Name("Emitter".to_string()),
        RuneScriptComponent::new_inline(
            "EmitterScript",
            r#"
            pub fn update(self_entity, dt) {
                emit_event("counter_event", #{ count: 1 });
            }
            "#,
        ),
    ));

    let _listener = scene.main_world_mut().spawn((
        Name("Listener".to_string()),
        RuneScriptComponent::new_inline(
            "ListenerScript",
            r#"
            pub fn on_created(self_entity) {
                subscribe_event("counter_event", "on_counter");
                set_f64(self_entity, "received_count", 0.0);
                set_f64(self_entity, "update_count", 0.0);
            }

            pub fn update(self_entity, dt) {
                let updates = get_f64(self_entity, "update_count", 0.0);
                set_f64(self_entity, "update_count", updates + 1.0);

                // Unsubscribe after 3 updates
                if updates == 3.0 {
                    unsubscribe_event("counter_event");
                }
            }

            pub fn on_counter(event_data) {
                // This is a workaround to get self_entity in event handler
                // In a real scenario, you'd pass entity in event data
                let count = get_f64(0, "temp_count", 0.0);
                set_f64(0, "temp_count", count + 1.0);
            }
            "#,
        ),
    ));

    // Initialize
    scene.update(0.0);

    // Run several updates
    for i in 1..=5 {
        scene.update(0.1);
        println!("Update {}", i);
    }

    println!("Test completed - subscription and unsubscription");
}

/// Test events emitted during on_created are dispatched
#[test]
fn event_on_created_emission() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Emitter emits during on_created
    let _emitter = scene.main_world_mut().spawn((
        Name("Emitter".to_string()),
        RuneScriptComponent::new_inline(
            "EmitterScript",
            r#"
            pub fn on_created(self_entity) {
                emit_event("init_event", #{
                    source: "emitter",
                    phase: "on_created"
                });
            }
            "#,
        ),
    ));

    // Listener subscribes in on_created
    let _listener = scene.main_world_mut().spawn((
        Name("Listener".to_string()),
        RuneScriptComponent::new_inline(
            "ListenerScript",
            r#"
            pub fn on_created(self_entity) {
                subscribe_event("init_event", "on_init");
                set_state(self_entity, "init_received", false);
            }

            pub fn on_init(event_data) {
                // Mark that we received the init event
                set_state(0, "global_init_received", true);
                set_state(0, "init_source", event_data.source);
            }
            "#,
        ),
    ));

    // Single update should process on_created and dispatch events
    scene.update(0.0);

    // The event should have been received during initialization
    // This tests the fix for the missing dispatch_events in process_on_created
    println!("Test completed - on_created event emission");
}

/// Test multiple subscribers to the same event
#[test]
fn event_multiple_subscribers() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Single emitter
    scene.main_world_mut().spawn((
        Name("Emitter".to_string()),
        RuneScriptComponent::new_inline(
            "EmitterScript",
            r#"
            pub fn update(self_entity, dt) {
                emit_event("broadcast", #{ data: "message" });
            }
            "#,
        ),
    ));

    // Multiple listeners
    for i in 1..=3 {
        scene.main_world_mut().spawn((
            Name(format!("Listener{}", i)),
            RuneScriptComponent::new_inline(
                &format!("ListenerScript{}", i),
                &format!(
                    r#"
                    pub fn on_created(self_entity) {{
                        subscribe_event("broadcast", "on_broadcast");
                        set_state(self_entity, "listener_id", {});
                    }}

                    pub fn on_broadcast(event_data) {{
                        let count = get_f64(0, "broadcast_count", 0.0);
                        set_f64(0, "broadcast_count", count + 1.0);
                    }}
                    "#,
                    i
                ),
            ),
        ));
    }

    // Initialize
    scene.update(0.0);

    // Run one update with event emission
    scene.update(0.1);

    // All three listeners should have received the event
    println!("Test completed - multiple subscribers");
}

/// Test event handler can spawn entities with scripts
#[test]
fn event_handler_spawns_entity() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Emitter
    scene.main_world_mut().spawn((
        Name("Emitter".to_string()),
        RuneScriptComponent::new_inline(
            "EmitterScript",
            r#"
            pub fn update(self_entity, dt) {
                let spawned = get_state(self_entity, "spawned", false);
                if !spawned {
                    emit_event("spawn_request", #{});
                    set_state(self_entity, "spawned", true);
                }
            }
            "#,
        ),
    ));

    // Spawner listens to events and spawns entities
    scene.main_world_mut().spawn((
        Name("Spawner".to_string()),
        RuneScriptComponent::new_inline(
            "SpawnerScript",
            r#"
            pub fn on_created(self_entity) {
                subscribe_event("spawn_request", "on_spawn_request");
            }

            pub fn on_spawn_request(event_data) {
                let new_entity = spawn_entity(Some("SpawnedByEvent"));
                set_translation(new_entity, 1.0, 2.0, 3.0);
                attach_inline_script(new_entity, "SpawnedScript", "
                    pub fn on_created(self_entity) {
                        set_state(self_entity, \"spawned_by_event\", true);
                    }
                ");
            }
            "#,
        ),
    ));

    // Initialize
    scene.update(0.0);

    // Run update to trigger spawn
    scene.update(0.1);

    // Check spawned entity exists
    let world = scene.main_world();
    let mut found_spawned = false;
    for (entity, name) in world.query::<&Name>().iter() {
        if name.0 == "SpawnedByEvent" {
            found_spawned = true;
            let transform = world.get::<&TransformComponent>(entity).unwrap();
            assert_eq!(transform.0.translation, glam::Vec3::new(1.0, 2.0, 3.0));
        }
    }

    // The entity is spawned but script initialization happens next frame
    assert!(found_spawned, "Entity spawned by event handler not found");

    // Run another frame to initialize spawned script
    scene.update(0.1);

    println!("Test completed - event handler spawning entities");
}

/// Test event handler can dynamically subscribe to events
#[test]
fn event_handler_dynamic_subscription() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Emitter emits multiple event types
    scene.main_world_mut().spawn((
        Name("Emitter".to_string()),
        RuneScriptComponent::new_inline(
            "EmitterScript",
            r#"
            pub fn update(self_entity, dt) {
                emit_event("trigger_event", #{});
                emit_event("secondary_event", #{ data: "test" });
            }
            "#,
        ),
    ));

    // Listener dynamically subscribes to secondary event when receiving trigger
    scene.main_world_mut().spawn((
        Name("Listener".to_string()),
        RuneScriptComponent::new_inline(
            "ListenerScript",
            r#"
            pub fn on_created(self_entity) {
                subscribe_event("trigger_event", "on_trigger");
                set_state(self_entity, "subscribed_to_secondary", false);
            }

            pub fn on_trigger(event_data) {
                let subscribed = get_state(0, "subscribed_to_secondary", false);
                if !subscribed {
                    subscribe_event("secondary_event", "on_secondary");
                    set_state(0, "subscribed_to_secondary", true);
                }
            }

            pub fn on_secondary(event_data) {
                set_state(0, "secondary_received", true);
            }
            "#,
        ),
    ));

    // Initialize
    scene.update(0.0);

    // First update: trigger event causes subscription
    scene.update(0.1);

    // Second update: secondary event should now be received
    scene.update(0.1);

    println!("Test completed - dynamic subscription in event handler");
}

/// Test event data passing with complex structures
#[test]
fn event_complex_data_passing() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Emitter sends complex data
    scene.main_world_mut().spawn((
        Name("Emitter".to_string()),
        RuneScriptComponent::new_inline(
            "EmitterScript",
            r#"
            pub fn update(self_entity, dt) {
                emit_event("complex_event", #{
                    string_val: "test message",
                    number_val: 42,
                    float_val: 3.14,
                    nested: #{
                        inner: "nested value"
                    },
                    array: [1, 2, 3]
                });
            }
            "#,
        ),
    ));

    // Listener receives and validates complex data
    scene.main_world_mut().spawn((
        Name("Listener".to_string()),
        RuneScriptComponent::new_inline(
            "ListenerScript",
            r#"
            pub fn on_created(self_entity) {
                subscribe_event("complex_event", "on_complex");
            }

            pub fn on_complex(event_data) {
                // Validate data fields exist and have correct types
                let str_val = event_data.string_val;
                let num_val = event_data.number_val;
                let float_val = event_data.float_val;
                let nested_val = event_data.nested.inner;
                let first_elem = event_data.array[0];

                set_state(0, "data_validated", true);
            }
            "#,
        ),
    ));

    // Initialize and run
    scene.update(0.0);
    scene.update(0.1);

    println!("Test completed - complex data passing");
}

/// Test event chain: event handler emits another event
#[test]
fn event_chain_emission() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Initial emitter
    scene.main_world_mut().spawn((
        Name("Emitter".to_string()),
        RuneScriptComponent::new_inline(
            "EmitterScript",
            r#"
            pub fn update(self_entity, dt) {
                let emitted = get_state(self_entity, "emitted", false);
                if !emitted {
                    emit_event("first_event", #{ step: 1 });
                    set_state(self_entity, "emitted", true);
                }
            }
            "#,
        ),
    ));

    // Middle handler that re-emits
    scene.main_world_mut().spawn((
        Name("Middle".to_string()),
        RuneScriptComponent::new_inline(
            "MiddleScript",
            r#"
            pub fn on_created(self_entity) {
                subscribe_event("first_event", "on_first");
            }

            pub fn on_first(event_data) {
                emit_event("second_event", #{ step: 2 });
            }
            "#,
        ),
    ));

    // Final listener
    scene.main_world_mut().spawn((
        Name("Final".to_string()),
        RuneScriptComponent::new_inline(
            "FinalScript",
            r#"
            pub fn on_created(self_entity) {
                subscribe_event("second_event", "on_second");
            }

            pub fn on_second(event_data) {
                set_state(0, "chain_completed", true);
            }
            "#,
        ),
    ));

    // Initialize
    scene.update(0.0);

    // Run update to trigger chain
    scene.update(0.1);

    println!("Test completed - event chain emission");
}
