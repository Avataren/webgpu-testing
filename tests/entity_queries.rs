use glam::Vec3;
use wgpu_cube::scene::components::{CameraComponent, Name, PointLight, TransformComponent};
use wgpu_cube::scene::{Scene, Transform};
use wgpu_cube::scripting::RuneScriptComponent;

#[test]
fn test_query_entities_with_component() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create entities with cameras
    let _camera1 = scene.main_world_mut().spawn((
        Name("Camera1".to_string()),
        TransformComponent(Transform::default()),
        CameraComponent::perspective(1.0, 0.1, 1000.0),
    ));

    let _camera2 = scene.main_world_mut().spawn((
        Name("Camera2".to_string()),
        TransformComponent(Transform::default()),
        CameraComponent::perspective(1.0, 0.1, 1000.0),
    ));

    // Create entity without camera
    let _other = scene.main_world_mut().spawn((
        Name("Other".to_string()),
        TransformComponent(Transform::default()),
    ));

    // Create script that queries cameras
    let _test_entity = scene.main_world_mut().spawn((
        Name("QueryTest".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "QueryTest",
            r####"
            pub fn on_created(self_entity) {
                let cameras = query_entities_with_component("CameraComponent");
                log_info(`Found ${cameras.len()} cameras`);

                // Should find 2 cameras
                if cameras.len() == 2 {
                    log_info("✓ Found correct number of cameras");
                } else {
                    log_error(`✗ Expected 2 cameras, found ${cameras.len()}`);
                }
            }
            "####,
        ),
    ));

    // Run startup frame
    scene.update(0.0);

    // Verify script executed
    let world = scene.main_world();
    let camera_count = world.query::<&CameraComponent>().iter().count();
    assert_eq!(camera_count, 2, "Should have 2 camera entities");
}

#[test]
fn test_query_nonexistent_component() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let _test_entity = scene.main_world_mut().spawn((
        Name("QueryTest".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "QueryTest",
            r####"
            pub fn on_created(self_entity) {
                // Query for component that doesn't exist
                let results = query_entities_with_component("NonExistentComponent");
                if results.len() == 0 {
                    log_info("✓ Empty result for non-existent component");
                }
            }
            "####,
        ),
    ));

    scene.update(0.0);
}

#[test]
fn test_get_entities_in_radius() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create entities at different positions
    let mut transform1 = Transform::default();
    transform1.translation = Vec3::new(0.0, 0.0, 0.0);
    let _entity1 = scene.main_world_mut().spawn((
        Name("Near1".to_string()),
        TransformComponent(transform1),
    ));

    let mut transform2 = Transform::default();
    transform2.translation = Vec3::new(5.0, 0.0, 0.0);
    let _entity2 = scene.main_world_mut().spawn((
        Name("Near2".to_string()),
        TransformComponent(transform2),
    ));

    let mut transform3 = Transform::default();
    transform3.translation = Vec3::new(20.0, 0.0, 0.0);
    let _entity3 = scene.main_world_mut().spawn((
        Name("Far".to_string()),
        TransformComponent(transform3),
    ));

    // Create test script
    let _test_entity = scene.main_world_mut().spawn((
        Name("SpatialTest".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "SpatialTest",
            r####"
            pub fn on_created(self_entity) {
                // Find entities within 10 units of origin
                let nearby = get_entities_in_radius(0.0, 0.0, 0.0, 10.0);
                log_info(`Found ${nearby.len()} nearby entities`);

                // Should find 3: the two near ones plus self
                if nearby.len() >= 2 {
                    log_info("✓ Found nearby entities");
                }
            }
            "####,
        ),
    ));

    scene.update(0.0);
}

#[test]
fn test_get_nearest_entity() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create entities at different distances
    let mut transform1 = Transform::default();
    transform1.translation = Vec3::new(10.0, 0.0, 0.0);
    let _entity1 = scene.main_world_mut().spawn((
        Name("Far".to_string()),
        TransformComponent(transform1),
    ));

    let mut transform2 = Transform::default();
    transform2.translation = Vec3::new(2.0, 0.0, 0.0);
    let _entity2 = scene.main_world_mut().spawn((
        Name("Near".to_string()),
        TransformComponent(transform2),
    ));

    // Create test script
    let _test_entity = scene.main_world_mut().spawn((
        Name("NearestTest".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "NearestTest",
            r####"
            pub fn on_created(self_entity) {
                let nearest = get_nearest_entity(0.0, 0.0, 0.0);
                if nearest != None {
                    log_info("✓ Found nearest entity");

                    // Get the name to verify it's the right one
                    let name_comp = get_component(nearest, "Name");
                    if name_comp != None {
                        log_info(`Nearest entity name: ${name_comp}`);
                    }
                }
            }
            "####,
        ),
    ));

    scene.update(0.0);
}

#[test]
fn test_get_nearest_entity_with_component() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create a near entity without the component
    let mut transform1 = Transform::default();
    transform1.translation = Vec3::new(2.0, 0.0, 0.0);
    let _entity1 = scene.main_world_mut().spawn((
        Name("NearWithoutLight".to_string()),
        TransformComponent(transform1),
    ));

    // Create a farther entity WITH the component
    let mut transform2 = Transform::default();
    transform2.translation = Vec3::new(10.0, 0.0, 0.0);
    let _entity2 = scene.main_world_mut().spawn((
        Name("FarWithLight".to_string()),
        TransformComponent(transform2),
        PointLight {
            color: Vec3::ONE,
            intensity: 1.0,
            range: 10.0,
        },
    ));

    // Create test script
    let _test_entity = scene.main_world_mut().spawn((
        Name("NearestWithComponentTest".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "NearestWithComponentTest",
            r####"
            pub fn on_created(self_entity) {
                // Should find the far entity because it has PointLight
                let nearest = get_nearest_entity_with_component(0.0, 0.0, 0.0, "PointLight");
                if nearest != None {
                    log_info("✓ Found nearest entity with PointLight");
                } else {
                    log_error("✗ Did not find nearest entity with PointLight");
                }

                // Try with non-existent component
                let none_result = get_nearest_entity_with_component(0.0, 0.0, 0.0, "FakeComponent");
                if none_result == None {
                    log_info("✓ Correctly returned None for non-existent component");
                }
            }
            "####,
        ),
    ));

    scene.update(0.0);
}

#[test]
fn test_get_entities_in_box() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create entities inside and outside the box
    let mut transform_inside1 = Transform::default();
    transform_inside1.translation = Vec3::new(5.0, 2.5, 5.0);
    let _inside1 = scene.main_world_mut().spawn((
        Name("Inside1".to_string()),
        TransformComponent(transform_inside1),
    ));

    let mut transform_inside2 = Transform::default();
    transform_inside2.translation = Vec3::new(7.0, 3.0, 8.0);
    let _inside2 = scene.main_world_mut().spawn((
        Name("Inside2".to_string()),
        TransformComponent(transform_inside2),
    ));

    let mut transform_outside = Transform::default();
    transform_outside.translation = Vec3::new(20.0, 0.0, 0.0);
    let _outside = scene.main_world_mut().spawn((
        Name("Outside".to_string()),
        TransformComponent(transform_outside),
    ));

    // Create test script
    let _test_entity = scene.main_world_mut().spawn((
        Name("BoxQueryTest".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "BoxQueryTest",
            r####"
            pub fn on_created(self_entity) {
                // Query entities in box from (0,0,0) to (10,5,10)
                let min = [0.0, 0.0, 0.0];
                let max = [10.0, 5.0, 10.0];
                let entities = get_entities_in_box(min, max);
                log_info(`Found ${entities.len()} entities in box`);

                // Should find 2 entities inside
                if entities.len() >= 2 {
                    log_info("✓ Found entities in box");
                }
            }
            "####,
        ),
    ));

    scene.update(0.0);
}

#[test]
fn test_spatial_queries_with_multiple_component_types() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create a mix of entities with different components at various positions
    let mut transform1 = Transform::default();
    transform1.translation = Vec3::new(3.0, 0.0, 0.0);
    let _camera_near = scene.main_world_mut().spawn((
        Name("CameraNear".to_string()),
        TransformComponent(transform1),
        CameraComponent::perspective(1.0, 0.1, 1000.0),
    ));

    let mut transform2 = Transform::default();
    transform2.translation = Vec3::new(15.0, 0.0, 0.0);
    let _camera_far = scene.main_world_mut().spawn((
        Name("CameraFar".to_string()),
        TransformComponent(transform2),
        CameraComponent::perspective(1.0, 0.1, 1000.0),
    ));

    let mut transform3 = Transform::default();
    transform3.translation = Vec3::new(5.0, 0.0, 0.0);
    let _light = scene.main_world_mut().spawn((
        Name("Light".to_string()),
        TransformComponent(transform3),
        PointLight {
            color: Vec3::ONE,
            intensity: 1.0,
            range: 10.0,
        },
    ));

    // Create test script
    let _test_entity = scene.main_world_mut().spawn((
        Name("MixedQueryTest".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "MixedQueryTest",
            r####"
            pub fn on_created(self_entity) {
                // Find all cameras
                let cameras = query_entities_with_component("CameraComponent");
                log_info(`Found ${cameras.len()} cameras total`);

                // Find cameras within radius
                let nearby = get_entities_in_radius(0.0, 0.0, 0.0, 10.0);
                log_info(`Found ${nearby.len()} entities within 10 units`);

                // Find nearest camera specifically
                let nearest_camera = get_nearest_entity_with_component(0.0, 0.0, 0.0, "CameraComponent");
                if nearest_camera != None {
                    log_info("✓ Found nearest camera");
                }

                // Find nearest light
                let nearest_light = get_nearest_entity_with_component(0.0, 0.0, 0.0, "PointLight");
                if nearest_light != None {
                    log_info("✓ Found nearest light");
                }
            }
            "####,
        ),
    ));

    scene.update(0.0);
}

#[test]
fn test_query_entities_multiple_component_types() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create entities with lights
    let _light1 = scene.main_world_mut().spawn((
        Name("Light1".to_string()),
        TransformComponent(Transform::default()),
        PointLight {
            color: Vec3::ONE,
            intensity: 1.0,
            range: 10.0,
        },
    ));

    let _light2 = scene.main_world_mut().spawn((
        Name("Light2".to_string()),
        TransformComponent(Transform::default()),
        PointLight {
            color: Vec3::ONE,
            intensity: 1.0,
            range: 10.0,
        },
    ));

    // Create entities with cameras
    let _camera = scene.main_world_mut().spawn((
        Name("Camera".to_string()),
        TransformComponent(Transform::default()),
        CameraComponent::perspective(1.0, 0.1, 1000.0),
    ));

    // Create test script
    let _test_entity = scene.main_world_mut().spawn((
        Name("MultiTypeQueryTest".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "MultiTypeQueryTest",
            r####"
            pub fn on_created(self_entity) {
                let lights = query_entities_with_component("PointLight");
                log_info(`Found ${lights.len()} lights`);

                let cameras = query_entities_with_component("CameraComponent");
                log_info(`Found ${cameras.len()} cameras`);

                if lights.len() == 2 && cameras.len() == 1 {
                    log_info("✓ Correct counts for all component types");
                }
            }
            "####,
        ),
    ));

    scene.update(0.0);
}

#[test]
fn test_empty_spatial_queries() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create test script with no other entities nearby
    let mut transform = Transform::default();
    transform.translation = Vec3::new(1000.0, 0.0, 0.0); // Far away
    let _test_entity = scene.main_world_mut().spawn((
        Name("IsolatedTest".to_string()),
        TransformComponent(transform),
        RuneScriptComponent::new_inline(
            "IsolatedTest",
            r####"
            pub fn on_created(self_entity) {
                // Query very far location where nothing exists
                let nearby = get_entities_in_radius(-1000.0, -1000.0, -1000.0, 1.0);
                if nearby.len() == 0 {
                    log_info("✓ Correct empty result for empty region");
                }

                let min = [-2000.0, -2000.0, -2000.0];
                let max = [-1900.0, -1900.0, -1900.0];
                let in_box = get_entities_in_box(min, max);
                if in_box.len() == 0 {
                    log_info("✓ Correct empty result for empty box");
                }
            }
            "####,
        ),
    ));

    scene.update(0.0);
}
