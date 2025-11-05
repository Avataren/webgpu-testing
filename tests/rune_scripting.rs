use glam::Quat;
use std::f32::consts::FRAC_PI_2;
use wgpu_cube::scene::components::{Name, Parent, TransformComponent};
use wgpu_cube::scene::Scene;
use wgpu_cube::scene::Transform;
use wgpu_cube::scripting::RuneScriptComponent;

#[test]
fn rune_script_spawns_and_updates_entities() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let root_entity = scene.main_world_mut().spawn((
        Name("Root".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "RootScript",
            r####"
            pub fn on_created(self_entity) {
                log_info("root script spawned");
                set_name(self_entity, "RootFromScript");
                set_translation(self_entity, 0.0, 1.0, 0.0);
                set_rotation(self_entity, 1.57079632679, 0.0, 0.0);
                let child = spawn_entity(Some("ChildFromScript"));
                set_translation(child, 2.0, 3.0, 4.0);
                attach_inline_script(child, "ChildScript", "
                    pub fn on_created(self_entity) {
                        log_info(\"child script spawned\");
                        set_translation(self_entity, 5.0, 6.0, 7.0);
                    }
                ");
            }

            pub fn update(self_entity, dt) {
                log_debug("root script update running");
                set_translation(self_entity, dt, dt * 2.0, dt * 3.0);
            }
            "####,
        ),
    ));

    // Run startup frame.
    scene.update(0.0);

    {
        let world = scene.main_world();
        println!(
            "scripts in world: {}",
            world.query::<&RuneScriptComponent>().iter().count()
        );
        let name = world.get::<&Name>(root_entity).unwrap();
        assert_eq!(name.0, "RootFromScript");
        let transform = world.get::<&TransformComponent>(root_entity).unwrap();
        assert_eq!(transform.0.translation, glam::Vec3::new(0.0, 1.0, 0.0));

        let mut child_found = false;
        for (entity, name) in world.query::<&Name>().iter() {
            if name.0 == "ChildFromScript" {
                child_found = true;
                let transform = world.get::<&TransformComponent>(entity).unwrap();
                assert_eq!(transform.0.translation, glam::Vec3::new(5.0, 6.0, 7.0));
            }
        }
        assert!(child_found, "child entity spawned by script was not found");
    }

    // Advance simulation to run update callback.
    scene.update(0.5);

    let world = scene.main_world();
    let transform = world.get::<&TransformComponent>(root_entity).unwrap();
    assert_eq!(transform.0.translation, glam::Vec3::new(0.5, 1.0, 1.5));
    assert!(transform
        .0
        .rotation
        .abs_diff_eq(Quat::from_rotation_y(FRAC_PI_2), 1e-3));
}

#[test]
fn rune_script_recursive_spawning_with_parents() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let root_entity = scene.main_world_mut().spawn((
        Name("Root".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "RecursiveSpawnTest",
            r####"
            pub fn on_created(self_entity) {
                log_info("Creating recursive structure with pending entities");

                // Spawn first level child (pending entity -1)
                let child1 = spawn_entity(Some("Child1"));
                set_parent(child1, Some(self_entity));
                set_translation(child1, 1.0, 0.0, 0.0);

                // Spawn second level child (pending entity -2, parent is pending -1)
                let child2 = spawn_entity(Some("Child2"));
                set_parent(child2, Some(child1));
                set_translation(child2, 1.0, 0.0, 0.0);

                // Spawn third level child (pending entity -3, parent is pending -2)
                let child3 = spawn_entity(Some("Child3"));
                set_parent(child3, Some(child2));
                set_translation(child3, 1.0, 0.0, 0.0);

                log_info("Recursive structure created");
            }
            "####,
        ),
    ));

    // Run startup frame to execute on_created
    scene.update(0.0);

    // Verify the hierarchy was created correctly
    let world = scene.main_world();

    // Count total entities
    let entity_count = world.iter().count();
    assert_eq!(entity_count, 4, "Should have 4 entities (root + 3 children)");

    // Count parent relationships
    let parent_count = world.query::<&Parent>().iter().count();
    assert_eq!(parent_count, 3, "Should have 3 parent relationships");

    // Verify the hierarchy structure
    let mut child1 = None;
    let mut child2 = None;
    let mut child3 = None;

    for (entity, name) in world.query::<&Name>().iter() {
        match name.0.as_str() {
            "Child1" => child1 = Some(entity),
            "Child2" => child2 = Some(entity),
            "Child3" => child3 = Some(entity),
            _ => {}
        }
    }

    let child1 = child1.expect("Child1 should exist");
    let child2 = child2.expect("Child2 should exist");
    let child3 = child3.expect("Child3 should exist");

    // Verify parent-child relationships
    let child1_parent = world.get::<&Parent>(child1).expect("Child1 should have parent");
    assert_eq!(child1_parent.0, root_entity, "Child1's parent should be root");

    let child2_parent = world.get::<&Parent>(child2).expect("Child2 should have parent");
    assert_eq!(child2_parent.0, child1, "Child2's parent should be Child1");

    let child3_parent = world.get::<&Parent>(child3).expect("Child3 should have parent");
    assert_eq!(child3_parent.0, child2, "Child3's parent should be Child2");

    println!("Recursive spawning with parents test passed!");
}
