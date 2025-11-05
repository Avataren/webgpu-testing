use rune::runtime::VmResult;

use crate::scripting::rune::guards::with_active_input_state;

/// Check if a keyboard key is currently pressed (down).
///
/// # Arguments
/// * `key` - The key name as a string (e.g., "W", "Space", "Escape", "A", "D", "S")
///
/// Returns `true` if the key is currently held down, `false` otherwise.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_key_pressed("W") {
///         translate(self_entity, 0.0, 0.0, -5.0 * dt);
///     }
/// }
/// ```
#[rune::function]
pub(crate) fn is_key_pressed(key: String) -> VmResult<bool> {
    with_active_input_state(|input_state| {
        VmResult::Ok(input_state.is_key_pressed(&key))
    })
}

/// Check if a keyboard key was just pressed this frame.
///
/// # Arguments
/// * `key` - The key name as a string
///
/// Returns `true` only on the frame the key transitions from up to down.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_key_just_pressed("Space") {
///         log_info("Jump!");
///     }
/// }
/// ```
#[rune::function]
pub(crate) fn is_key_just_pressed(key: String) -> VmResult<bool> {
    with_active_input_state(|input_state| {
        VmResult::Ok(input_state.is_key_just_pressed(&key))
    })
}

/// Check if a keyboard key was just released this frame.
///
/// # Arguments
/// * `key` - The key name as a string
///
/// Returns `true` only on the frame the key transitions from down to up.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_key_just_released("W") {
///         log_info("Stopped moving forward");
///     }
/// }
/// ```
#[rune::function]
pub(crate) fn is_key_just_released(key: String) -> VmResult<bool> {
    with_active_input_state(|input_state| {
        VmResult::Ok(input_state.is_key_just_released(&key))
    })
}

/// Check if a mouse button is currently pressed (down).
///
/// # Arguments
/// * `button` - The mouse button index (0 = left, 1 = right, 2 = middle)
///
/// Returns `true` if the button is currently held down, `false` otherwise.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_mouse_button_pressed(0) {
///         log_info("Left mouse button is down");
///     }
/// }
/// ```
#[rune::function]
pub(crate) fn is_mouse_button_pressed(button: i64) -> VmResult<bool> {
    with_active_input_state(|input_state| {
        VmResult::Ok(input_state.is_mouse_button_pressed(button as u32))
    })
}

/// Check if a mouse button was just pressed this frame.
///
/// # Arguments
/// * `button` - The mouse button index (0 = left, 1 = right, 2 = middle)
///
/// Returns `true` only on the frame the button transitions from up to down.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_mouse_button_just_pressed(0) {
///         log_info("Click!");
///     }
/// }
/// ```
#[rune::function]
pub(crate) fn is_mouse_button_just_pressed(button: i64) -> VmResult<bool> {
    with_active_input_state(|input_state| {
        VmResult::Ok(input_state.is_mouse_button_just_pressed(button as u32))
    })
}

/// Check if a mouse button was just released this frame.
///
/// # Arguments
/// * `button` - The mouse button index (0 = left, 1 = right, 2 = middle)
///
/// Returns `true` only on the frame the button transitions from down to up.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     if is_mouse_button_just_released(0) {
///         log_info("Released left click");
///     }
/// }
/// ```
#[rune::function]
pub(crate) fn is_mouse_button_just_released(button: i64) -> VmResult<bool> {
    with_active_input_state(|input_state| {
        VmResult::Ok(input_state.is_mouse_button_just_released(button as u32))
    })
}

/// Get the current mouse position.
///
/// Returns an array `[x, y]` with the mouse position in screen coordinates.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     let pos = get_mouse_position();
///     log_info(`Mouse at ${pos[0]}, ${pos[1]}`);
/// }
/// ```
#[rune::function]
pub(crate) fn get_mouse_position() -> VmResult<rune::alloc::Vec<f64>> {
    with_active_input_state(|input_state| {
        let pos = input_state.mouse_position();
        let mut vec = rune::alloc::Vec::new();
        if let Err(e) = vec.try_push(pos.x as f64) {
            return VmResult::Err(e.into());
        }
        if let Err(e) = vec.try_push(pos.y as f64) {
            return VmResult::Err(e.into());
        }
        VmResult::Ok(vec)
    })
}

/// Get the mouse movement delta for this frame.
///
/// Returns an array `[dx, dy]` with the mouse movement since last frame.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     let delta = get_mouse_delta();
///     if delta[0].abs() > 0.0 || delta[1].abs() > 0.0 {
///         log_info(`Mouse moved by ${delta[0]}, ${delta[1]}`);
///     }
/// }
/// ```
#[rune::function]
pub(crate) fn get_mouse_delta() -> VmResult<rune::alloc::Vec<f64>> {
    with_active_input_state(|input_state| {
        let delta = input_state.mouse_delta();
        let mut vec = rune::alloc::Vec::new();
        if let Err(e) = vec.try_push(delta.x as f64) {
            return VmResult::Err(e.into());
        }
        if let Err(e) = vec.try_push(delta.y as f64) {
            return VmResult::Err(e.into());
        }
        VmResult::Ok(vec)
    })
}

/// Get the mouse scroll delta for this frame.
///
/// Returns an array `[dx, dy]` with the scroll amount. Typically `dy` is used for vertical scrolling.
///
/// # Example
/// ```rune
/// pub fn update(self_entity, dt) {
///     let scroll = get_mouse_scroll_delta();
///     if scroll[1] > 0.0 {
///         log_info("Scrolled up");
///     } else if scroll[1] < 0.0 {
///         log_info("Scrolled down");
///     }
/// }
/// ```
#[rune::function]
pub(crate) fn get_mouse_scroll_delta() -> VmResult<rune::alloc::Vec<f64>> {
    with_active_input_state(|input_state| {
        let scroll = input_state.scroll_delta();
        let mut vec = rune::alloc::Vec::new();
        if let Err(e) = vec.try_push(scroll.x as f64) {
            return VmResult::Err(e.into());
        }
        if let Err(e) = vec.try_push(scroll.y as f64) {
            return VmResult::Err(e.into());
        }
        VmResult::Ok(vec)
    })
}
