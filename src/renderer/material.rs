// renderer/material.rs
use super::assets::Handle;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Material {
    pub color: [u8; 4], // RGBA color
    // Future: texture handles, shader variant, etc.
}

impl Material {
    pub fn new(color: [u8; 4]) -> Self {
        Self { color }
    }

    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::new([r, g, b, 255])
    }

    pub fn white() -> Self {
        Self::rgb(255, 255, 255)
    }

    pub fn red() -> Self {
        Self::rgb(255, 0, 0)
    }

    pub fn green() -> Self {
        Self::rgb(0, 255, 0)
    }

    pub fn blue() -> Self {
        Self::rgb(0, 0, 255)
    }

    /// Get normalized color as [f32; 4]
    pub fn color_f32(&self) -> [f32; 4] {
        [
            self.color[0] as f32 / 255.0,
            self.color[1] as f32 / 255.0,
            self.color[2] as f32 / 255.0,
            self.color[3] as f32 / 255.0,
        ]
    }
}

impl Default for Material {
    fn default() -> Self {
        Self::white()
    }
}