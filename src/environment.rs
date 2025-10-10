use wgpu::Color;

/// Describes high-level environment settings applied while rendering a scene.
///
/// The environment controls global rendering parameters such as the clear
/// color, sky rendering, and image-based lighting. The current implementation
/// keeps things minimal with only a configurable clear color.
#[derive(Debug, Clone, Copy)]
pub struct Environment {
    clear_color: Color,
}

impl Environment {
    /// Creates a new environment with the provided clear color.
    pub fn new(clear_color: Color) -> Self {
        Self { clear_color }
    }

    /// Returns the clear color that should be used when starting a frame.
    pub fn clear_color(&self) -> Color {
        self.clear_color
    }

    /// Sets the clear color used for rendering.
    pub fn set_clear_color(&mut self, color: Color) {
        self.clear_color = color;
    }

    /// Returns a copy of the environment with the given clear color.
    pub fn with_clear_color(mut self, color: Color) -> Self {
        self.clear_color = color;
        self
    }
}

impl Default for Environment {
    fn default() -> Self {
        Self::new(Color {
            r: 0.231,
            g: 0.269,
            b: 0.338,
            a: 1.0,
        })
    }
}
