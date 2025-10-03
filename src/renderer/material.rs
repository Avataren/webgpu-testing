// renderer/material.rs (Bindless version)
use super::assets::Handle;
use super::texture::Texture;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Material {
    pub color: [u8; 4],
    pub texture_index: u32,  // Index into texture array (0 = no texture)
    pub flags: MaterialFlags,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaterialFlags(u32);

impl MaterialFlags {
    pub const NONE: Self = Self(0);
    pub const USE_TEXTURE: Self = Self(1 << 0);
    pub const ALPHA_BLEND: Self = Self(1 << 1);
    pub const DOUBLE_SIDED: Self = Self(1 << 2);
    pub const EMISSIVE: Self = Self(1 << 3);

    pub const fn bits(&self) -> u32 {
        self.0
    }

    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

impl std::ops::BitOr for MaterialFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for MaterialFlags {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

impl Material {
    pub fn new(color: [u8; 4]) -> Self {
        Self {
            color,
            texture_index: 0,
            flags: MaterialFlags::NONE,
        }
    }

    pub fn with_texture(mut self, texture_index: u32) -> Self {
        self.texture_index = texture_index;
        self.flags |= MaterialFlags::USE_TEXTURE;
        self
    }

    pub fn with_alpha(mut self) -> Self {
        self.flags |= MaterialFlags::ALPHA_BLEND;
        self
    }

    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self::new([r, g, b, 255])
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

    pub fn white() -> Self {
        Self::rgb(255, 255, 255)
    }

    pub fn checker() -> Self {
        Self::new([255, 255, 255, 0])
    }

    pub fn color_f32(&self) -> [f32; 4] {
        [
            self.color[0] as f32 / 255.0,
            self.color[1] as f32 / 255.0,
            self.color[2] as f32 / 255.0,
            self.color[3] as f32 / 255.0,
        ]
    }

    pub fn flags_bits(&self) -> u32 {
        self.flags.bits()
    }

    /// Check if this material requires a different pipeline (transparency, etc.)
    pub fn requires_separate_pass(&self) -> bool {
        self.flags.contains(MaterialFlags::ALPHA_BLEND)
    }
}

impl Default for Material {
    fn default() -> Self {
        Self::white()
    }
}