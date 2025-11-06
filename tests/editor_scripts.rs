use glam::{Quat, Vec3};
use hecs::World;
use std::path::Path;
use wgpu_cube::scene::components::{Name, TransformComponent};
use wgpu_cube::scene::Transform;
use wgpu_cube::scripting::{RuneScriptComponent, RuneScriptSource, ScriptingState};

#[test]
fn editor_cube_spin_script_runs_without_compile_errors() {
    let _ = env_logger::builder().is_test(true).try_init();

    let mut scripting = ScriptingState::new().expect("failed to create scripting state");
    scripting
        .runtime_mut()
        .set_script_root(env!("CARGO_MANIFEST_DIR"));

    let mut world = World::new();
    let entity = world.spawn((
        Name("Cube".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new(RuneScriptSource::file("scripts/editor_cube_spin.rn")),
    ));

    scripting
        .update_scripts(&mut world, 0.0)
        .expect("cube spin on_created should compile");
    scripting
        .update_scripts(&mut world, 0.5)
        .expect("cube spin update should run");
    scripting
        .update_scripts(&mut world, 0.25)
        .expect("cube spin update should run again");

    let transform = world
        .get::<&TransformComponent>(entity)
        .expect("transform component should be inserted by script");
    let expected_angle = (0.5 + 0.25) * 1.5;
    let expected_rotation = Quat::from_rotation_y(expected_angle as f32);
    assert!(transform.0.rotation.abs_diff_eq(expected_rotation, 1e-3));
}

#[test]
fn editor_startup_script_spawns_cube_and_attaches_spin() {
    let _ = env_logger::builder().is_test(true).try_init();

    let mut scripting = ScriptingState::new().expect("failed to create scripting state");
    scripting
        .runtime_mut()
        .set_script_root(env!("CARGO_MANIFEST_DIR"));

    let mut world = World::new();
    world.spawn((
        Name("Editor Root".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new(RuneScriptSource::file("scripts/editor_startup.rn")),
    ));

    scripting
        .update_scripts(&mut world, 0.0)
        .expect("editor startup on_created should compile");
    scripting
        .update_scripts(&mut world, 0.016)
        .expect("editor startup update should run");

    let mut cube_found = false;
    {
        let mut query = world.query::<(&Name, &TransformComponent)>();
        for (_entity, (name, transform)) in query.iter() {
            if name.0 == "Editor Cube" {
                cube_found = true;
                assert!(transform
                    .0
                    .translation
                    .abs_diff_eq(Vec3::new(0.0, 0.5, 0.0), 1e-5));
            }
        }
    }
    assert!(cube_found, "Editor Cube entity was not spawned");

    let mut spin_script_attached = false;
    {
        let mut query = world.query::<&RuneScriptComponent>();
        for (_entity, script) in query.iter() {
            if let RuneScriptSource::File { path } = script.source() {
                if path == Path::new("scripts/editor_cube_spin.rn") {
                    spin_script_attached = true;
                    break;
                }
            }
        }
    }
    assert!(
        spin_script_attached,
        "Spin script was not attached to the spawned cube"
    );
}

#[test]
fn spawn_cube_fractal_runs_without_errors() {
    let _ = env_logger::builder().is_test(true).try_init();

    let mut scripting = ScriptingState::new().expect("failed to create scripting state");
    scripting
        .runtime_mut()
        .set_script_root(env!("CARGO_MANIFEST_DIR"));

    let mut world = World::new();
    world.spawn((
        Name("Fractal Spawner".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new(RuneScriptSource::file("scripts/spawn_cube_fractal.rn")),
    ));

    // First update: spawn_cube_fractal.rn on_created runs
    // This should spawn the fractal root entity and attach cube_fractal.rn to it
    scripting
        .update_scripts(&mut world, 0.0)
        .expect("spawn_cube_fractal on_created should run without errors");

    // Second update: cube_fractal.rn on_created runs
    // This is where the "entity handle is not yet available" error would occur
    // because cube_fractal.rn calls add_component, set_scale, set_translation on self_entity
    scripting
        .update_scripts(&mut world, 0.0)
        .expect("cube_fractal on_created should run without entity handle errors");

    // Third update: fractal_cube_rotate.rn on_created runs for all the child cubes
    scripting
        .update_scripts(&mut world, 0.0)
        .expect("fractal_cube_rotate on_created should run without errors");

    // Fourth update: run all update functions
    scripting
        .update_scripts(&mut world, 0.016)
        .expect("all scripts should update without errors");

    // Verify the fractal root was created
    let mut fractal_found = false;
    {
        let mut query = world.query::<&Name>();
        for (_entity, name) in query.iter() {
            if name.0 == "Cube Fractal" {
                fractal_found = true;
                break;
            }
        }
    }
    assert!(fractal_found, "Cube Fractal entity was not spawned");

    // Verify cube_fractal.rn script was attached
    let mut fractal_script_attached = false;
    {
        let mut query = world.query::<&RuneScriptComponent>();
        for (_entity, script) in query.iter() {
            if let RuneScriptSource::File { path } = script.source() {
                if path == Path::new("scripts/cube_fractal.rn") {
                    fractal_script_attached = true;
                    break;
                }
            }
        }
    }
    assert!(
        fractal_script_attached,
        "cube_fractal script was not attached"
    );
}
