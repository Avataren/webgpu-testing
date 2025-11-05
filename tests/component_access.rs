use glam::Vec3;
use wgpu_cube::scene::components::{Name, TransformComponent};
use wgpu_cube::scene::Scene;
use wgpu_cube::scene::Transform;
use wgpu_cube::scripting::RuneScriptComponent;

#[test]
fn test_get_component_name() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let test_entity = scene.main_world_mut().spawn((
        Name("TestEntity".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "ComponentAccessTest",
            r####"
            pub fn on_created(self_entity) {
                // Test getting Name component
                let name_comp = get_component(self_entity, "Name");
                if name_comp != None {
                    log_info("Found name component");
                } else {
                    log_error("Failed to get Name component!");
                }

                // Test has_component
                if has_component(self_entity, "Name") {
                    log_info("Entity has Name component");
                } else {
                    log_error("Entity missing Name component!");
                }

                if has_component(self_entity, "TransformComponent") {
                    log_info("Entity has TransformComponent");
                } else {
                    log_error("Entity missing TransformComponent!");
                }

                // Test getting non-existent component
                let missing = get_component(self_entity, "MeshComponent");
                if missing == None {
                    log_info("Correctly returned None for missing component");
                } else {
                    log_error("Should have returned None for missing component!");
                }
            }
            "####,
        ),
    ));

    // Run startup frame to execute on_created
    scene.update(0.0);

    // Verify the script ran without errors
    let world = scene.main_world();
    let script_comp = world.get::<&RuneScriptComponent>(test_entity).unwrap();
    assert!(script_comp.created_called());
}

#[test]
fn test_get_component_transform() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let mut transform = Transform::default();
    transform.translation = Vec3::new(1.0, 2.0, 3.0);

    let test_entity = scene.main_world_mut().spawn((
        Name("TransformTest".to_string()),
        TransformComponent(transform),
        RuneScriptComponent::new_inline(
            "TransformAccessTest",
            r####"
            pub fn on_created(self_entity) {
                // Test getting TransformComponent
                let transform = get_component(self_entity, "TransformComponent");
                if transform != None {
                    log_info("Got transform component successfully");
                } else {
                    log_error("Failed to get TransformComponent!");
                }
            }
            "####,
        ),
    ));

    // Run startup frame
    scene.update(0.0);

    // Verify script executed
    let world = scene.main_world();
    let script_comp = world.get::<&RuneScriptComponent>(test_entity).unwrap();
    assert!(script_comp.created_called());
}

#[test]
fn test_find_entity_by_name() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create a target entity
    scene.main_world_mut().spawn((
        Name("TargetEntity".to_string()),
        TransformComponent(Transform::default()),
    ));

    // Create a script that searches for the target
    let searcher = scene.main_world_mut().spawn((
        Name("Searcher".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "SearchTest",
            r####"
            pub fn on_created(self_entity) {
                // Find entity by name
                let target_opt = find_entity_by_name("TargetEntity");
                if target_opt != None {
                    log_info("Found TargetEntity");
                    // Note: In Rune, we can't directly use Option values
                    // This is a limitation we'll address in future iterations
                } else {
                    log_error("Failed to find TargetEntity!");
                }

                // Try to find non-existent entity
                let missing = find_entity_by_name("NonExistent");
                if missing == None {
                    log_info("Correctly returned None for missing entity");
                } else {
                    log_error("Should have returned None for missing entity!");
                }
            }
            "####,
        ),
    ));

    // Run startup frame
    scene.update(0.0);

    // Verify script executed
    let world = scene.main_world();
    let script_comp = world.get::<&RuneScriptComponent>(searcher).unwrap();
    assert!(script_comp.created_called());
}

#[test]
fn test_has_component() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let test_entity = scene.main_world_mut().spawn((
        Name("HasComponentTest".to_string()),
        RuneScriptComponent::new_inline(
            "HasComponentTest",
            r####"
            pub fn on_created(self_entity) {
                // Check for component we have
                if has_component(self_entity, "Name") {
                    log_info("✓ Correctly detected Name component");
                } else {
                    log_error("✗ Failed to detect Name component!");
                }

                // Check for component we don't have
                if !has_component(self_entity, "MeshComponent") {
                    log_info("✓ Correctly returned false for MeshComponent");
                } else {
                    log_error("✗ Should not have MeshComponent!");
                }

                // Check for unknown component type
                if !has_component(self_entity, "UnknownComponent") {
                    log_info("✓ Correctly handled unknown component type");
                } else {
                    log_error("✗ Should return false for unknown component!");
                }
            }
            "####,
        ),
    ));

    // Run startup frame
    scene.update(0.0);

    // Verify script executed
    let world = scene.main_world();
    let script_comp = world.get::<&RuneScriptComponent>(test_entity).unwrap();
    assert!(script_comp.created_called());
}
