use env_logger;
use glam::Quat;
use wgpu_cube::scene::components::{Name, TransformComponent};
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
        .abs_diff_eq(Quat::from_rotation_y(1.57079632679_f32), 1e-3));
}
