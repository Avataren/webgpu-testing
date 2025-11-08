use glam::Vec2;
use std::collections::HashSet;

/// Represents the state of a key or button
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonState {
    /// Not pressed
    Up,
    /// Just pressed this frame
    JustPressed,
    /// Held down
    Down,
    /// Just released this frame
    JustReleased,
}

impl ButtonState {
    /// Returns true if the button is down (either just pressed or held)
    pub fn is_down(&self) -> bool {
        matches!(self, ButtonState::JustPressed | ButtonState::Down)
    }

    /// Returns true if the button was just pressed this frame
    pub fn is_just_pressed(&self) -> bool {
        matches!(self, ButtonState::JustPressed)
    }

    /// Returns true if the button was just released this frame
    pub fn is_just_released(&self) -> bool {
        matches!(self, ButtonState::JustReleased)
    }
}

/// Input state resource that tracks keyboard and mouse input
#[derive(Debug, Clone)]
pub struct InputState {
    // Keyboard state
    keys_down: HashSet<String>,
    keys_just_pressed: HashSet<String>,
    keys_just_released: HashSet<String>,

    // Mouse button state
    mouse_buttons_down: HashSet<u32>,
    mouse_buttons_just_pressed: HashSet<u32>,
    mouse_buttons_just_released: HashSet<u32>,

    // Mouse position and movement
    mouse_position: Vec2,
    mouse_delta: Vec2,

    // Mouse scroll
    scroll_delta: Vec2,
}

impl Default for InputState {
    fn default() -> Self {
        Self::new()
    }
}

impl InputState {
    /// Create a new empty input state
    pub fn new() -> Self {
        Self {
            keys_down: HashSet::new(),
            keys_just_pressed: HashSet::new(),
            keys_just_released: HashSet::new(),
            mouse_buttons_down: HashSet::new(),
            mouse_buttons_just_pressed: HashSet::new(),
            mouse_buttons_just_released: HashSet::new(),
            mouse_position: Vec2::ZERO,
            mouse_delta: Vec2::ZERO,
            scroll_delta: Vec2::ZERO,
        }
    }

    /// Update at the start of each frame - transitions "just pressed" to "down" and "just released" to "up"
    pub fn update(&mut self) {
        // Transition keyboard states
        for key in self.keys_just_pressed.drain() {
            self.keys_down.insert(key);
        }
        self.keys_just_released.clear();

        // Transition mouse button states
        for button in self.mouse_buttons_just_pressed.drain() {
            self.mouse_buttons_down.insert(button);
        }
        self.mouse_buttons_just_released.clear();

        // Reset deltas
        self.mouse_delta = Vec2::ZERO;
        self.scroll_delta = Vec2::ZERO;
    }

    // === Keyboard Methods ===

    /// Called when a key is pressed
    pub fn press_key(&mut self, key: String) {
        if !self.keys_down.contains(&key) {
            self.keys_just_pressed.insert(key);
        }
    }

    /// Called when a key is released
    pub fn release_key(&mut self, key: &str) {
        self.keys_down.remove(key);
        self.keys_just_pressed.remove(key);
        self.keys_just_released.insert(key.to_string());
    }

    /// Check if a key is currently pressed
    pub fn is_key_pressed(&self, key: &str) -> bool {
        self.keys_down.contains(key) || self.keys_just_pressed.contains(key)
    }

    /// Check if a key was just pressed this frame
    pub fn is_key_just_pressed(&self, key: &str) -> bool {
        self.keys_just_pressed.contains(key)
    }

    /// Check if a key was just released this frame
    pub fn is_key_just_released(&self, key: &str) -> bool {
        self.keys_just_released.contains(key)
    }

    /// Get the state of a key
    pub fn key_state(&self, key: &str) -> ButtonState {
        if self.keys_just_pressed.contains(key) {
            ButtonState::JustPressed
        } else if self.keys_down.contains(key) {
            ButtonState::Down
        } else if self.keys_just_released.contains(key) {
            ButtonState::JustReleased
        } else {
            ButtonState::Up
        }
    }

    // === Mouse Button Methods ===

    /// Called when a mouse button is pressed
    pub fn press_mouse_button(&mut self, button: u32) {
        if !self.mouse_buttons_down.contains(&button) {
            self.mouse_buttons_just_pressed.insert(button);
        }
    }

    /// Called when a mouse button is released
    pub fn release_mouse_button(&mut self, button: u32) {
        self.mouse_buttons_down.remove(&button);
        self.mouse_buttons_just_pressed.remove(&button);
        self.mouse_buttons_just_released.insert(button);
    }

    /// Check if a mouse button is currently pressed
    pub fn is_mouse_button_pressed(&self, button: u32) -> bool {
        self.mouse_buttons_down.contains(&button)
            || self.mouse_buttons_just_pressed.contains(&button)
    }

    /// Check if a mouse button was just pressed this frame
    pub fn is_mouse_button_just_pressed(&self, button: u32) -> bool {
        self.mouse_buttons_just_pressed.contains(&button)
    }

    /// Check if a mouse button was just released this frame
    pub fn is_mouse_button_just_released(&self, button: u32) -> bool {
        self.mouse_buttons_just_released.contains(&button)
    }

    /// Get the state of a mouse button
    pub fn mouse_button_state(&self, button: u32) -> ButtonState {
        if self.mouse_buttons_just_pressed.contains(&button) {
            ButtonState::JustPressed
        } else if self.mouse_buttons_down.contains(&button) {
            ButtonState::Down
        } else if self.mouse_buttons_just_released.contains(&button) {
            ButtonState::JustReleased
        } else {
            ButtonState::Up
        }
    }

    // === Mouse Position and Movement Methods ===

    /// Set the current mouse position
    pub fn set_mouse_position(&mut self, position: Vec2) {
        self.mouse_position = position;
    }

    /// Get the current mouse position
    pub fn mouse_position(&self) -> Vec2 {
        self.mouse_position
    }

    /// Add to the mouse delta for this frame
    pub fn add_mouse_delta(&mut self, delta: Vec2) {
        self.mouse_delta += delta;
    }

    /// Get the mouse movement delta for this frame
    pub fn mouse_delta(&self) -> Vec2 {
        self.mouse_delta
    }

    /// Add to the scroll delta for this frame
    pub fn add_scroll_delta(&mut self, delta: Vec2) {
        self.scroll_delta += delta;
    }

    /// Get the mouse scroll delta for this frame
    pub fn scroll_delta(&self) -> Vec2 {
        self.scroll_delta
    }

    /// Reset all input state (useful for focus changes)
    pub fn reset(&mut self) {
        self.keys_down.clear();
        self.keys_just_pressed.clear();
        self.keys_just_released.clear();
        self.mouse_buttons_down.clear();
        self.mouse_buttons_just_pressed.clear();
        self.mouse_buttons_just_released.clear();
        self.mouse_delta = Vec2::ZERO;
        self.scroll_delta = Vec2::ZERO;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_press_cycle() {
        let mut input = InputState::new();

        // Initially not pressed
        assert!(!input.is_key_pressed("W"));
        assert!(!input.is_key_just_pressed("W"));

        // Press key
        input.press_key("W".to_string());
        assert!(input.is_key_pressed("W"));
        assert!(input.is_key_just_pressed("W"));
        assert_eq!(input.key_state("W"), ButtonState::JustPressed);

        // Update (transitions to Down)
        input.update();
        assert!(input.is_key_pressed("W"));
        assert!(!input.is_key_just_pressed("W"));
        assert_eq!(input.key_state("W"), ButtonState::Down);

        // Release key
        input.release_key("W");
        assert!(!input.is_key_pressed("W"));
        assert!(input.is_key_just_released("W"));
        assert_eq!(input.key_state("W"), ButtonState::JustReleased);

        // Update (transitions to Up)
        input.update();
        assert!(!input.is_key_pressed("W"));
        assert!(!input.is_key_just_released("W"));
        assert_eq!(input.key_state("W"), ButtonState::Up);
    }

    #[test]
    fn test_mouse_button_cycle() {
        let mut input = InputState::new();

        // Initially not pressed
        assert!(!input.is_mouse_button_pressed(0));
        assert!(!input.is_mouse_button_just_pressed(0));

        // Press button
        input.press_mouse_button(0);
        assert!(input.is_mouse_button_pressed(0));
        assert!(input.is_mouse_button_just_pressed(0));
        assert_eq!(input.mouse_button_state(0), ButtonState::JustPressed);

        // Update (transitions to Down)
        input.update();
        assert!(input.is_mouse_button_pressed(0));
        assert!(!input.is_mouse_button_just_pressed(0));
        assert_eq!(input.mouse_button_state(0), ButtonState::Down);

        // Release button
        input.release_mouse_button(0);
        assert!(!input.is_mouse_button_pressed(0));
        assert!(input.is_mouse_button_just_released(0));
        assert_eq!(input.mouse_button_state(0), ButtonState::JustReleased);

        // Update (transitions to Up)
        input.update();
        assert!(!input.is_mouse_button_pressed(0));
        assert!(!input.is_mouse_button_just_released(0));
        assert_eq!(input.mouse_button_state(0), ButtonState::Up);
    }

    #[test]
    fn test_mouse_position() {
        let mut input = InputState::new();

        assert_eq!(input.mouse_position(), Vec2::ZERO);

        input.set_mouse_position(Vec2::new(100.0, 200.0));
        assert_eq!(input.mouse_position(), Vec2::new(100.0, 200.0));
    }

    #[test]
    fn test_mouse_delta() {
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
    fn test_scroll_delta() {
        let mut input = InputState::new();

        assert_eq!(input.scroll_delta(), Vec2::ZERO);

        input.add_scroll_delta(Vec2::new(0.0, 1.0));
        assert_eq!(input.scroll_delta(), Vec2::new(0.0, 1.0));

        // Update resets scroll delta
        input.update();
        assert_eq!(input.scroll_delta(), Vec2::ZERO);
    }

    #[test]
    fn test_reset() {
        let mut input = InputState::new();

        input.press_key("W".to_string());
        input.press_mouse_button(0);
        input.set_mouse_position(Vec2::new(100.0, 100.0));
        input.add_mouse_delta(Vec2::new(5.0, 5.0));

        input.reset();

        assert!(!input.is_key_pressed("W"));
        assert!(!input.is_mouse_button_pressed(0));
        assert_eq!(input.mouse_delta(), Vec2::ZERO);
        // Note: reset doesn't clear mouse_position, only button states and deltas
    }
}
