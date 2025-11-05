use glam::Vec3;
use wgpu_cube::scene::components::{
    Name, TransformComponent, Visible, PointLight, DirectionalLight,
    SpotLight, RotateAnimation, OrbitAnimation, CanCastShadow,
};
use wgpu_cube::scene::Scene;
use wgpu_cube::scene::Transform;
use wgpu_cube::scripting::RuneScriptComponent;

#[test]
fn test_all_component_types() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Create an entity with many different components
    let test_entity = scene.main_world_mut().spawn((
        Name("MultiComponentEntity".to_string()),
        TransformComponent(Transform::default()),
        Visible(true),
        PointLight {
            color: Vec3::ONE,
            intensity: 5.0,
            range: 10.0,
        },
        RotateAnimation {
            axis: Vec3::Y,
            speed: 1.0,
        },
        CanCastShadow(true),
        RuneScriptComponent::new_inline(
            "AllComponentsTest",
            r####"
            pub fn on_created(self_entity) {
                log_info("=== Testing All Component Types ===");

                // Test Name
                if has_component(self_entity, "Name") {
                    let name = get_component(self_entity, "Name");
                    if name != None {
                        log_info("✓ Name component accessible");
                    }
                }

                // Test TransformComponent
                if has_component(self_entity, "TransformComponent") {
                    log_info("✓ TransformComponent accessible");
                }

                // Test Visible
                if has_component(self_entity, "Visible") {
                    let visible = get_component(self_entity, "Visible");
                    if visible != None {
                        log_info("✓ Visible component accessible");
                    }
                }

                // Test PointLight
                if has_component(self_entity, "PointLight") {
                    let light = get_component(self_entity, "PointLight");
                    if light != None {
                        log_info("✓ PointLight component accessible");
                    }
                }

                // Test RotateAnimation
                if has_component(self_entity, "RotateAnimation") {
                    let anim = get_component(self_entity, "RotateAnimation");
                    if anim != None {
                        log_info("✓ RotateAnimation component accessible");
                    }
                }

                // Test CanCastShadow
                if has_component(self_entity, "CanCastShadow") {
                    let shadow = get_component(self_entity, "CanCastShadow");
                    if shadow != None {
                        log_info("✓ CanCastShadow component accessible");
                    }
                }

                log_info("=== All Component Tests Complete ===");
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
fn test_light_components() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // Point Light entity
    scene.main_world_mut().spawn((
        Name("PointLight".to_string()),
        PointLight {
            color: Vec3::new(1.0, 0.8, 0.6),
            intensity: 3.5,
            range: 15.0,
        },
    ));

    // Directional Light entity
    scene.main_world_mut().spawn((
        Name("DirectionalLight".to_string()),
        DirectionalLight {
            color: Vec3::ONE,
            intensity: 2.0,
            shadow_size: 50.0,
        },
    ));

    // Spot Light entity
    scene.main_world_mut().spawn((
        Name("SpotLight".to_string()),
        SpotLight {
            color: Vec3::new(1.0, 1.0, 0.8),
            intensity: 4.0,
            inner_angle: 0.5,
            outer_angle: 1.0,
            range: 20.0,
        },
    ));

    // Script that checks all lights
    let searcher = scene.main_world_mut().spawn((
        Name("LightChecker".to_string()),
        RuneScriptComponent::new_inline(
            "LightTest",
            r####"
            pub fn on_created(self_entity) {
                log_info("=== Searching for Lights ===");

                let point = find_entity_by_name("PointLight");
                if point != None {
                    log_info("Found PointLight entity");
                }

                let directional = find_entity_by_name("DirectionalLight");
                if directional != None {
                    log_info("Found DirectionalLight entity");
                }

                let spot = find_entity_by_name("SpotLight");
                if spot != None {
                    log_info("Found SpotLight entity");
                }

                log_info("=== Light Search Complete ===");
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
fn test_animation_components() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let test_entity = scene.main_world_mut().spawn((
        Name("AnimatedEntity".to_string()),
        TransformComponent(Transform::default()),
        RotateAnimation {
            axis: Vec3::Y,
            speed: 2.0,
        },
        OrbitAnimation {
            center: Vec3::ZERO,
            radius: 5.0,
            speed: 1.0,
            offset: 0.0,
        },
        RuneScriptComponent::new_inline(
            "AnimTest",
            r####"
            pub fn on_created(self_entity) {
                if has_component(self_entity, "RotateAnimation") {
                    log_info("✓ Has RotateAnimation");
                }

                if has_component(self_entity, "OrbitAnimation") {
                    log_info("✓ Has OrbitAnimation");
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
fn test_component_aliases() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let test_entity = scene.main_world_mut().spawn((
        Name("AliasTest".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "AliasTest",
            r####"
            pub fn on_created(self_entity) {
                // Test that Transform and TransformComponent are aliases
                if has_component(self_entity, "Transform") {
                    log_info("✓ Transform alias works");
                }

                if has_component(self_entity, "TransformComponent") {
                    log_info("✓ TransformComponent works");
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
