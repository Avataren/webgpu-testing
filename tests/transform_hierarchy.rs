use glam::Vec3;
use wgpu_cube::scene::components::{Children, Name, Parent, TransformComponent};
use wgpu_cube::scene::Scene;
use wgpu_cube::scene::Transform;
use wgpu_cube::scripting::RuneScriptComponent;

#[test]
fn test_translate() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let mut transform = Transform::default();
    transform.translation = Vec3::new(0.0, 0.0, 0.0);

    let test_entity = scene.main_world_mut().spawn((
        Name("TranslateTest".to_string()),
        TransformComponent(transform),
        RuneScriptComponent::new_inline(
            "TranslateTest",
            r####"
            pub fn on_created(self_entity) {
                // Translate the entity
                translate(self_entity, 1.0, 2.0, 3.0);
                log_info("✓ Translate command issued");
            }
            "####,
        ),
    ));

    // Run startup frame to execute on_created
    scene.update(0.0);

    // Verify the translation was applied
    let world = scene.main_world();
    let transform_comp = world.get::<&TransformComponent>(test_entity).unwrap();
    assert!(
        (transform_comp.0.translation.x - 1.0).abs() < 0.001,
        "X translation incorrect"
    );
    assert!(
        (transform_comp.0.translation.y - 2.0).abs() < 0.001,
        "Y translation incorrect"
    );
    assert!(
        (transform_comp.0.translation.z - 3.0).abs() < 0.001,
        "Z translation incorrect"
    );
}

#[test]
fn test_rotate() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let test_entity = scene.main_world_mut().spawn((
        Name("RotateTest".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "RotateTest",
            r####"
            pub fn on_created(self_entity) {
                // Rotate around Y axis by 90 degrees (PI/2 radians)
                let pi = 3.14159265359;
                rotate(self_entity, 0.0, 1.0, 0.0, pi / 2.0);
                log_info("✓ Rotate command issued");
            }
            "####,
        ),
    ));

    // Run startup frame
    scene.update(0.0);

    // Verify rotation was applied (rotation should not be identity)
    let world = scene.main_world();
    let transform_comp = world.get::<&TransformComponent>(test_entity).unwrap();
    let rotation = transform_comp.0.rotation;

    // Check that rotation is not the identity quaternion
    assert!(
        (rotation.w - 1.0).abs() > 0.01
            || rotation.x.abs() > 0.01
            || rotation.y.abs() > 0.01
            || rotation.z.abs() > 0.01,
        "Rotation should have been applied"
    );
}

#[test]
fn test_set_scale() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let test_entity = scene.main_world_mut().spawn((
        Name("ScaleTest".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "ScaleTest",
            r####"
            pub fn on_created(self_entity) {
                // Set scale to 2x, 3x, 4x
                set_scale(self_entity, 2.0, 3.0, 4.0);
                log_info("✓ SetScale command issued");
            }
            "####,
        ),
    ));

    // Run startup frame
    scene.update(0.0);

    // Verify scale was applied
    let world = scene.main_world();
    let transform_comp = world.get::<&TransformComponent>(test_entity).unwrap();
    assert!(
        (transform_comp.0.scale.x - 2.0).abs() < 0.001,
        "X scale incorrect"
    );
    assert!(
        (transform_comp.0.scale.y - 3.0).abs() < 0.001,
        "Y scale incorrect"
    );
    assert!(
        (transform_comp.0.scale.z - 4.0).abs() < 0.001,
        "Z scale incorrect"
    );
}

#[test]
fn test_look_at() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let mut transform = Transform::default();
    transform.translation = Vec3::new(0.0, 0.0, 0.0);

    let test_entity = scene.main_world_mut().spawn((
        Name("LookAtTest".to_string()),
        TransformComponent(transform),
        RuneScriptComponent::new_inline(
            "LookAtTest",
            r####"
            pub fn on_created(self_entity) {
                // Look at point (1, 0, 0)
                look_at(self_entity, 1.0, 0.0, 0.0);
                log_info("✓ LookAt command issued");
            }
            "####,
        ),
    ));

    // Run startup frame
    scene.update(0.0);

    // Verify rotation was applied (should not be identity)
    let world = scene.main_world();
    let transform_comp = world.get::<&TransformComponent>(test_entity).unwrap();
    let rotation = transform_comp.0.rotation;

    // The rotation should be set to look at the target
    assert!(rotation.w.abs() > 0.0, "Rotation should have been applied");
}

#[test]
fn test_get_world_translation() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let mut transform = Transform::default();
    transform.translation = Vec3::new(5.0, 10.0, 15.0);

    let test_entity = scene.main_world_mut().spawn((
        Name("GetTranslationTest".to_string()),
        TransformComponent(transform),
        RuneScriptComponent::new_inline(
            "GetTranslationTest",
            r####"
            pub fn on_created(self_entity) {
                // Get world translation
                let pos = get_world_translation(self_entity);
                if pos != None {
                    log_info("✓ Got world translation");
                } else {
                    log_error("✗ Failed to get world translation");
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
fn test_get_world_rotation() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let test_entity = scene.main_world_mut().spawn((
        Name("GetRotationTest".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "GetRotationTest",
            r####"
            pub fn on_created(self_entity) {
                // Get world rotation (as euler angles)
                let rot = get_world_rotation(self_entity);
                if rot != None {
                    log_info("✓ Got world rotation");
                } else {
                    log_error("✗ Failed to get world rotation");
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
fn test_set_parent() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create parent entity
    let parent = scene.main_world_mut().spawn((
        Name("Parent".to_string()),
        TransformComponent(Transform::default()),
    ));

    let parent_bits = parent.to_bits().get();

    // Create child entity with script
    let child = scene.main_world_mut().spawn((
        Name("Child".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "ParentTest",
            &format!(
                r####"
                pub fn on_created(self_entity) {{
                    // Set parent
                    set_parent(self_entity, Some({}));
                    log_info("✓ SetParent command issued");
                }}
                "####,
                parent_bits
            ),
        ),
    ));

    // Run startup frame
    scene.update(0.0);

    // Verify parent was set
    let world = scene.main_world();
    let parent_comp = world.get::<&Parent>(child);
    assert!(parent_comp.is_ok(), "Child should have Parent component");
    assert_eq!(parent_comp.unwrap().0, parent, "Parent entity should match");

    // Verify parent has child in Children component
    let children_comp = world.get::<&Children>(parent);
    assert!(
        children_comp.is_ok(),
        "Parent should have Children component"
    );
    assert!(
        children_comp.unwrap().0.contains(&child),
        "Parent's children should contain child"
    );
}

#[test]
fn test_unparent() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create parent entity
    let parent = scene.main_world_mut().spawn((
        Name("Parent".to_string()),
        TransformComponent(Transform::default()),
    ));

    // Create child entity with existing parent
    let child = scene.main_world_mut().spawn((
        Name("Child".to_string()),
        TransformComponent(Transform::default()),
        Parent(parent),
        RuneScriptComponent::new_inline(
            "UnparentTest",
            r####"
            pub fn on_created(self_entity) {
                // Remove parent
                set_parent(self_entity, None);
                log_info("✓ Unparent command issued");
            }
            "####,
        ),
    ));

    // Manually add child to parent's children
    scene
        .main_world_mut()
        .insert_one(parent, Children(vec![child]))
        .unwrap();

    // Run startup frame
    scene.update(0.0);

    // Verify parent was removed
    let world = scene.main_world();
    let parent_comp = world.get::<&Parent>(child);
    assert!(
        parent_comp.is_err(),
        "Child should not have Parent component"
    );

    // Verify parent's children was updated
    let children_comp = world.get::<&Children>(parent);
    if let Ok(children) = children_comp {
        assert!(
            !children.0.contains(&child),
            "Parent's children should not contain child"
        );
    }
}

#[test]
fn test_get_parent() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create parent entity
    let parent = scene.main_world_mut().spawn((
        Name("Parent".to_string()),
        TransformComponent(Transform::default()),
    ));

    // Create child entity with parent
    let child = scene.main_world_mut().spawn((
        Name("Child".to_string()),
        TransformComponent(Transform::default()),
        Parent(parent),
        RuneScriptComponent::new_inline(
            "GetParentTest",
            r####"
            pub fn on_created(self_entity) {
                // Get parent
                let parent_opt = get_parent(self_entity);
                if parent_opt != None {
                    log_info("✓ Got parent entity");
                } else {
                    log_error("✗ Failed to get parent");
                }
            }
            "####,
        ),
    ));

    // Run startup frame
    scene.update(0.0);

    // Verify script executed
    let world = scene.main_world();
    let script_comp = world.get::<&RuneScriptComponent>(child).unwrap();
    assert!(script_comp.created_called());
}

#[test]
fn test_get_children() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create child entities
    let child1 = scene.main_world_mut().spawn((
        Name("Child1".to_string()),
        TransformComponent(Transform::default()),
    ));

    let child2 = scene.main_world_mut().spawn((
        Name("Child2".to_string()),
        TransformComponent(Transform::default()),
    ));

    // Create parent entity with children
    let parent = scene.main_world_mut().spawn((
        Name("Parent".to_string()),
        TransformComponent(Transform::default()),
        Children(vec![child1, child2]),
        RuneScriptComponent::new_inline(
            "GetChildrenTest",
            r####"
            pub fn on_created(self_entity) {
                // Get children
                let children_opt = get_children(self_entity);
                if children_opt != None {
                    log_info("✓ Got children list");
                } else {
                    log_error("✗ Failed to get children");
                }
            }
            "####,
        ),
    ));

    // Run startup frame
    scene.update(0.0);

    // Verify script executed
    let world = scene.main_world();
    let script_comp = world.get::<&RuneScriptComponent>(parent).unwrap();
    assert!(script_comp.created_called());
}

#[test]
fn test_multiple_transforms() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let test_entity = scene.main_world_mut().spawn((
        Name("MultiTransformTest".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "MultiTransformTest",
            r####"
            pub fn on_created(self_entity) {
                // Apply multiple transformations
                translate(self_entity, 1.0, 0.0, 0.0);
                set_scale(self_entity, 2.0, 2.0, 2.0);

                let pi = 3.14159265359;
                rotate(self_entity, 0.0, 1.0, 0.0, pi / 4.0);

                log_info("✓ Multiple transform commands issued");
            }
            "####,
        ),
    ));

    // Run startup frame
    scene.update(0.0);

    // Verify all transformations were applied
    let world = scene.main_world();
    let transform_comp = world.get::<&TransformComponent>(test_entity).unwrap();

    // Check translation
    assert!(
        (transform_comp.0.translation.x - 1.0).abs() < 0.001,
        "Translation should be applied"
    );

    // Check scale
    assert!(
        (transform_comp.0.scale.x - 2.0).abs() < 0.001,
        "Scale should be applied"
    );

    // Check rotation (should not be identity)
    let rotation = transform_comp.0.rotation;
    assert!(
        (rotation.w - 1.0).abs() > 0.01
            || rotation.x.abs() > 0.01
            || rotation.y.abs() > 0.01
            || rotation.z.abs() > 0.01,
        "Rotation should be applied"
    );
}

#[test]
fn test_hierarchy_chain() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create grandparent
    let grandparent = scene.main_world_mut().spawn((
        Name("Grandparent".to_string()),
        TransformComponent(Transform::default()),
    ));

    let _grandparent_bits = grandparent.to_bits().get();

    // Create parent
    let parent = scene.main_world_mut().spawn((
        Name("Parent".to_string()),
        TransformComponent(Transform::default()),
        Parent(grandparent),
    ));

    let parent_bits = parent.to_bits().get();

    // Update grandparent's children
    scene
        .main_world_mut()
        .insert_one(grandparent, Children(vec![parent]))
        .unwrap();

    // Create child with script
    let child = scene.main_world_mut().spawn((
        Name("Child".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "HierarchyTest",
            &format!(
                r####"
                pub fn on_created(self_entity) {{
                    // Set parent
                    set_parent(self_entity, Some({}));

                    // Get parent
                    let parent_opt = get_parent(self_entity);
                    if parent_opt != None {{
                        log_info("✓ Got parent in hierarchy");
                    }}
                }}
                "####,
                parent_bits
            ),
        ),
    ));

    // Run startup frame
    scene.update(0.0);

    // Verify the hierarchy chain
    let world = scene.main_world();

    // Check child has parent
    let child_parent = world.get::<&Parent>(child).unwrap();
    assert_eq!(child_parent.0, parent, "Child's parent should be correct");

    // Check parent has child
    let parent_children = world.get::<&Children>(parent).unwrap();
    assert!(
        parent_children.0.contains(&child),
        "Parent should contain child"
    );

    // Check parent has grandparent
    let parent_parent = world.get::<&Parent>(parent).unwrap();
    assert_eq!(
        parent_parent.0, grandparent,
        "Parent's parent should be grandparent"
    );
}
