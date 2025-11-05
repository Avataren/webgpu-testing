use glam::Vec2;
use wgpu_cube::scene::{InputState, Scene};
use wgpu_cube::scene::components::{Name, TransformComponent};
use wgpu_cube::scene::Transform;
use wgpu_cube::scripting::RuneScriptComponent;

#[test]
fn test_input_state_keyboard_cycle() {
    let mut input = InputState::new();

    // Initially not pressed
    assert!(!input.is_key_pressed("W"));
    assert!(!input.is_key_just_pressed("W"));

    // Press key
    input.press_key("W".to_string());
    assert!(input.is_key_pressed("W"));
    assert!(input.is_key_just_pressed("W"));

    // Update (transitions to Down)
    input.update();
    assert!(input.is_key_pressed("W"));
    assert!(!input.is_key_just_pressed("W"));

    // Release key
    input.release_key("W");
    assert!(!input.is_key_pressed("W"));
    assert!(input.is_key_just_released("W"));

    // Update (transitions to Up)
    input.update();
    assert!(!input.is_key_pressed("W"));
    assert!(!input.is_key_just_released("W"));
}

#[test]
fn test_input_state_mouse_button_cycle() {
    let mut input = InputState::new();

    // Initially not pressed
    assert!(!input.is_mouse_button_pressed(0));
    assert!(!input.is_mouse_button_just_pressed(0));

    // Press button
    input.press_mouse_button(0);
    assert!(input.is_mouse_button_pressed(0));
    assert!(input.is_mouse_button_just_pressed(0));

    // Update (transitions to Down)
    input.update();
    assert!(input.is_mouse_button_pressed(0));
    assert!(!input.is_mouse_button_just_pressed(0));

    // Release button
    input.release_mouse_button(0);
    assert!(!input.is_mouse_button_pressed(0));
    assert!(input.is_mouse_button_just_released(0));

    // Update (transitions to Up)
    input.update();
    assert!(!input.is_mouse_button_pressed(0));
    assert!(!input.is_mouse_button_just_released(0));
}

#[test]
fn test_input_state_mouse_position() {
    let mut input = InputState::new();

    assert_eq!(input.mouse_position(), Vec2::ZERO);

    input.set_mouse_position(Vec2::new(100.0, 200.0));
    assert_eq!(input.mouse_position(), Vec2::new(100.0, 200.0));

    input.set_mouse_position(Vec2::new(150.0, 250.0));
    assert_eq!(input.mouse_position(), Vec2::new(150.0, 250.0));
}

#[test]
fn test_input_state_mouse_delta() {
    let mut input = InputState::new();

    assert_eq!(input.mouse_delta(), Vec2::ZERO);

    input.add_mouse_delta(Vec2::new(5.0, -3.0));
    assert_eq!(input.mouse_delta(), Vec2::new(5.0, -3.0));

    input.add_mouse_delta(Vec2::new(2.0, 1.0));
    assert_eq!(input.mouse_delta(), Vec2::new(7.0, -2.0));

    // Update resets delta
    input.update();
    assert_eq!(input.mouse_delta(), Vec2::ZERO);
}

#[test]
fn test_input_state_scroll_delta() {
    let mut input = InputState::new();

    assert_eq!(input.scroll_delta(), Vec2::ZERO);

    input.add_scroll_delta(Vec2::new(0.0, 1.0));
    assert_eq!(input.scroll_delta(), Vec2::new(0.0, 1.0));

    input.add_scroll_delta(Vec2::new(0.0, 0.5));
    assert_eq!(input.scroll_delta(), Vec2::new(0.0, 1.5));

    // Update resets scroll delta
    input.update();
    assert_eq!(input.scroll_delta(), Vec2::ZERO);
}

#[test]
fn test_input_state_multiple_keys() {
    let mut input = InputState::new();

    input.press_key("W".to_string());
    input.press_key("A".to_string());
    input.press_key("Space".to_string());

    assert!(input.is_key_just_pressed("W"));
    assert!(input.is_key_just_pressed("A"));
    assert!(input.is_key_just_pressed("Space"));

    input.update();

    assert!(input.is_key_pressed("W"));
    assert!(input.is_key_pressed("A"));
    assert!(input.is_key_pressed("Space"));
    assert!(!input.is_key_just_pressed("W"));

    input.release_key("W");
    assert!(!input.is_key_pressed("W"));
    assert!(input.is_key_pressed("A"));
    assert!(input.is_key_pressed("Space"));
}

#[test]
fn test_input_state_multiple_mouse_buttons() {
    let mut input = InputState::new();

    input.press_mouse_button(0);
    input.press_mouse_button(1);

    assert!(input.is_mouse_button_just_pressed(0));
    assert!(input.is_mouse_button_just_pressed(1));

    input.update();

    assert!(input.is_mouse_button_pressed(0));
    assert!(input.is_mouse_button_pressed(1));
    assert!(!input.is_mouse_button_just_pressed(0));

    input.release_mouse_button(0);
    assert!(!input.is_mouse_button_pressed(0));
    assert!(input.is_mouse_button_pressed(1));
}

#[test]
fn test_input_state_reset() {
    let mut input = InputState::new();

    input.press_key("W".to_string());
    input.press_mouse_button(0);
    input.set_mouse_position(Vec2::new(100.0, 100.0));
    input.add_mouse_delta(Vec2::new(5.0, 5.0));
    input.add_scroll_delta(Vec2::new(0.0, 1.0));

    input.reset();

    assert!(!input.is_key_pressed("W"));
    assert!(!input.is_mouse_button_pressed(0));
    assert_eq!(input.mouse_delta(), Vec2::ZERO);
    assert_eq!(input.scroll_delta(), Vec2::ZERO);
    // Note: mouse_position is NOT reset by reset()
    assert_eq!(input.mouse_position(), Vec2::new(100.0, 100.0));
}

// Note: The following tests would require integrating InputState with the Scene's
// script execution system. For now, they serve as documentation of the expected API.
// Full integration would require:
// 1. Adding InputState as a resource to the World or Scene
// 2. Setting up InputStateGuard in the script execution path
// 3. Wiring up winit events to update the InputState

#[test]
#[ignore] // Ignored until InputState is integrated with Scene
fn test_keyboard_input_in_script() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    // This test demonstrates the expected API once InputState is fully integrated
    let _test_entity = scene.main_world_mut().spawn((
        Name("InputTest".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "InputTest",
            r####"
            pub fn update(self_entity, dt) {
                if is_key_pressed("W") {
                    translate(self_entity, 0.0, 0.0, -1.0 * dt);
                    log_info("Moving forward");
                }

                if is_key_just_pressed("Space") {
                    log_info("Jump!");
                }
            }
            "####,
        ),
    ));

    // Would need to:
    // 1. Add InputState to scene
    // 2. Simulate key press
    // 3. Run update
    // 4. Verify entity moved
}

#[test]
#[ignore] // Ignored until InputState is integrated with Scene
fn test_mouse_input_in_script() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let _test_entity = scene.main_world_mut().spawn((
        Name("MouseTest".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "MouseTest",
            r####"
            pub fn update(self_entity, dt) {
                if is_mouse_button_pressed(0) {
                    log_info("Left mouse button is down");
                }

                if is_mouse_button_just_pressed(1) {
                    log_info("Right click!");
                }

                let pos = get_mouse_position();
                let delta = get_mouse_delta();
                let scroll = get_mouse_scroll_delta();

                if delta[0].abs() > 0.0 || delta[1].abs() > 0.0 {
                    log_info("Mouse moved");
                }
            }
            "####,
        ),
    ));
}

#[test]
#[ignore] // Ignored until InputState is integrated with Scene
fn test_wasd_movement_controller() {
    let _ = env_logger::builder().is_test(true).try_init();
    let mut scene = Scene::new();

    let _player = scene.main_world_mut().spawn((
        Name("Player".to_string()),
        TransformComponent(Transform::default()),
        RuneScriptComponent::new_inline(
            "PlayerController",
            r####"
            pub fn on_created(self_entity) {
                set_state(self_entity, "speed", 5.0);
            }

            pub fn update(self_entity, dt) {
                let speed = get_state(self_entity, "speed", 5.0);
                let mut dx = 0.0;
                let mut dz = 0.0;

                if is_key_pressed("W") {
                    dz -= 1.0;
                }
                if is_key_pressed("S") {
                    dz += 1.0;
                }
                if is_key_pressed("A") {
                    dx -= 1.0;
                }
                if is_key_pressed("D") {
                    dx += 1.0;
                }

                if dx != 0.0 || dz != 0.0 {
                    // Normalize diagonal movement
                    let len = (dx * dx + dz * dz).sqrt();
                    dx = dx / len * speed * dt;
                    dz = dz / len * speed * dt;
                    translate(self_entity, dx, 0.0, dz);
                }
            }
            "####,
        ),
    ));

    // Would simulate WASD input and verify player moves correctly
}

#[test]
fn test_input_state_button_state_enum() {
    use wgpu_cube::scene::ButtonState;

    let just_pressed = ButtonState::JustPressed;
    assert!(just_pressed.is_down());
    assert!(just_pressed.is_just_pressed());
    assert!(!just_pressed.is_just_released());

    let down = ButtonState::Down;
    assert!(down.is_down());
    assert!(!down.is_just_pressed());
    assert!(!down.is_just_released());

    let just_released = ButtonState::JustReleased;
    assert!(!just_released.is_down());
    assert!(!just_released.is_just_pressed());
    assert!(just_released.is_just_released());

    let up = ButtonState::Up;
    assert!(!up.is_down());
    assert!(!up.is_just_pressed());
    assert!(!up.is_just_released());
}

#[test]
fn test_input_state_key_state_queries() {
    use wgpu_cube::scene::ButtonState;
    let mut input = InputState::new();

    // Up state
    assert_eq!(input.key_state("W"), ButtonState::Up);

    // JustPressed state
    input.press_key("W".to_string());
    assert_eq!(input.key_state("W"), ButtonState::JustPressed);

    // Down state
    input.update();
    assert_eq!(input.key_state("W"), ButtonState::Down);

    // JustReleased state
    input.release_key("W");
    assert_eq!(input.key_state("W"), ButtonState::JustReleased);

    // Back to Up state
    input.update();
    assert_eq!(input.key_state("W"), ButtonState::Up);
}

#[test]
fn test_input_state_mouse_button_state_queries() {
    use wgpu_cube::scene::ButtonState;
    let mut input = InputState::new();

    // Up state
    assert_eq!(input.mouse_button_state(0), ButtonState::Up);

    // JustPressed state
    input.press_mouse_button(0);
    assert_eq!(input.mouse_button_state(0), ButtonState::JustPressed);

    // Down state
    input.update();
    assert_eq!(input.mouse_button_state(0), ButtonState::Down);

    // JustReleased state
    input.release_mouse_button(0);
    assert_eq!(input.mouse_button_state(0), ButtonState::JustReleased);

    // Back to Up state
    input.update();
    assert_eq!(input.mouse_button_state(0), ButtonState::Up);
}
