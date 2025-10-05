// renderer/material.rs (PBR version)

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Material {
    pub base_color: [u8; 4],
    pub flags: MaterialFlags,

    // PBR texture indices
    pub base_color_texture: u32,
    pub metallic_roughness_texture: u32,
    pub normal_texture: u32,
    pub emissive_texture: u32,
    pub occlusion_texture: u32,

    // PBR parameters (stored as u8, converted to f32 in shader)
    pub metallic_factor: u8,   // 0-255 -> 0.0-1.0
    pub roughness_factor: u8,  // 0-255 -> 0.0-1.0
    pub emissive_strength: u8, // 0-255 -> 0.0-1.0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MaterialFlags(u32);

impl MaterialFlags {
    pub const NONE: Self = Self(0);
    pub const USE_BASE_COLOR_TEXTURE: Self = Self(1 << 0);
    pub const USE_METALLIC_ROUGHNESS_TEXTURE: Self = Self(1 << 1);
    pub const USE_NORMAL_TEXTURE: Self = Self(1 << 2);
    pub const USE_EMISSIVE_TEXTURE: Self = Self(1 << 3);
    pub const USE_OCCLUSION_TEXTURE: Self = Self(1 << 4);
    pub const ALPHA_BLEND: Self = Self(1 << 5);
    pub const DOUBLE_SIDED: Self = Self(1 << 6);

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
            base_color: color,
            flags: MaterialFlags::NONE,
            base_color_texture: 0,
            metallic_roughness_texture: 0,
            normal_texture: 0,
            emissive_texture: 0,
            occlusion_texture: 0,
            metallic_factor: 0,
            roughness_factor: 255, // Default to rough
            emissive_strength: 0,
        }
    }

    pub fn pbr() -> Self {
        Self::new([255, 255, 255, 255])
            .with_metallic(0.0)
            .with_roughness(0.5)
    }

    pub fn with_metallic(mut self, metallic: f32) -> Self {
        self.metallic_factor = (metallic.clamp(0.0, 1.0) * 255.0) as u8;
        self
    }

    pub fn with_roughness(mut self, roughness: f32) -> Self {
        self.roughness_factor = (roughness.clamp(0.0, 1.0) * 255.0) as u8;
        self
    }

    pub fn with_emissive(mut self, strength: f32) -> Self {
        self.emissive_strength = (strength.clamp(0.0, 1.0) * 255.0) as u8;
        self
    }

    pub fn with_alpha(mut self) -> Self {
        self.flags |= MaterialFlags::ALPHA_BLEND;
        self
    }

    pub fn with_base_color_texture(mut self, index: u32) -> Self {
        self.base_color_texture = index;
        self.flags |= MaterialFlags::USE_BASE_COLOR_TEXTURE;
        self
    }

    pub fn with_metallic_roughness_texture(mut self, index: u32) -> Self {
        self.metallic_roughness_texture = index;
        self.flags |= MaterialFlags::USE_METALLIC_ROUGHNESS_TEXTURE;
        self
    }

    pub fn with_normal_texture(mut self, index: u32) -> Self {
        self.normal_texture = index;
        self.flags |= MaterialFlags::USE_NORMAL_TEXTURE;
        self
    }

    pub fn with_emissive_texture(mut self, index: u32) -> Self {
        self.emissive_texture = index;
        self.flags |= MaterialFlags::USE_EMISSIVE_TEXTURE;
        self
    }

    pub fn with_occlusion_texture(mut self, index: u32) -> Self {
        self.occlusion_texture = index;
        self.flags |= MaterialFlags::USE_OCCLUSION_TEXTURE;
        self
    }

    // Legacy compatibility
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

    pub fn with_texture(self, index: u32) -> Self {
        self.with_base_color_texture(index)
    }

    pub fn checker() -> Self {
        Self::new([255, 255, 255, 255]).with_base_color_texture(0)
    }

    pub fn color_f32(&self) -> [f32; 4] {
        [
            self.base_color[0] as f32 / 255.0,
            self.base_color[1] as f32 / 255.0,
            self.base_color[2] as f32 / 255.0,
            self.base_color[3] as f32 / 255.0,
        ]
    }

    pub fn metallic_f32(&self) -> f32 {
        self.metallic_factor as f32 / 255.0
    }

    pub fn roughness_f32(&self) -> f32 {
        self.roughness_factor as f32 / 255.0
    }

    pub fn emissive_f32(&self) -> f32 {
        self.emissive_strength as f32 / 255.0
    }

    pub fn flags_bits(&self) -> u32 {
        self.flags.bits()
    }

    pub fn requires_separate_pass(&self) -> bool {
        self.flags.contains(MaterialFlags::ALPHA_BLEND)
    }
}

impl Default for Material {
    fn default() -> Self {
        Self::white()
    }
}
