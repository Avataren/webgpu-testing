// renderer/material.rs (PBR version)

use super::shader_builder::SamplerFilterMode;
use crate::renderer::texture::DEFAULT_CHECKER_TEXTURE_INDEX;

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
    pub metallic_factor: u8,  // 0-255 -> 0.0-1.0
    pub roughness_factor: u8, // 0-255 -> 0.0-1.0
    /// 255 is 1.0; larger values give HDR emission for bloom, up to `MAX_EMISSIVE`.
    pub emissive_strength: u16,
    /// How much light passes through the surface, 0-255 -> 0.0-1.0 (glass, gems, water).
    /// Transmitted light is refracted by `ior` and tinted by the base colour.
    pub transmission_factor: u8,
    /// Index of refraction as `(ior - 1) * 100`: 50 is 1.5 (glass), 142 is 2.42
    /// (diamond); covers 1.00 to 3.55.
    pub ior: u8,
    /// How much the index of refraction varies with wavelength (prisms, the "fire" of a
    /// cut gem), in hundredths: as glTF KHR_materials_dispersion, 20 / Abbe number (36 for
    /// diamond, about 34 for crown glass); 0 for none.
    pub dispersion: u8,
    /// Alpha below which the surface is cut away (glTF `MASK`: foliage, fences), 1-255
    /// for 0.0-1.0; 0 for none. A masked surface is otherwise opaque.
    pub alpha_cutoff: u8,
    /// Interior mapping (with [`MaterialFlags::INTERIOR`]): the room images in the
    /// emissive texture, as columns minus one (high 4 bits) and rows minus one (low 4
    /// bits). 0 is a single room; 0x31 is four columns by two rows.
    pub interior_atlas: u8,
    /// Interior mapping: how deep the rooms are, in room widths times 64 (64 is as deep
    /// as wide; 1-255 for 1/64 to about 4).
    pub interior_depth: u8,
    /// Interior mapping: the share of rooms with their lights on, 0-255 for 0.0-1.0.
    pub interior_lights: u8,
    /// Interior mapping: the share of rooms with curtains or a blind, 0-255 for 0.0-1.0.
    pub interior_curtains: u8,
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
    pub const UNLIT: Self = Self(1 << 7);
    pub const USE_NEAREST_FILTERING: Self = Self(1 << 8);
    pub const BILLBOARDED: Self = Self(1 << 9);
    /// Leave the object out of shadow maps. Set per object at render time from
    /// `CanCastShadow(false)`; not stored in material assets.
    pub const NO_SHADOW: Self = Self(1 << 10);
    /// Interior mapping: the surface is a window onto rooms behind it, traced in the
    /// shader (interior.wgsl). The emissive texture holds the rooms' images
    /// (`Material::interior_atlas`), lit by the emissive strength; base colour texture
    /// alpha marks the frame (opaque) apart from the glass, and the base colour tints
    /// the glass.
    pub const INTERIOR: Self = Self(1 << 11);
    // Bits 24-31 are taken on the GPU by the alpha cutoff (`MaterialData`).

    pub const fn bits(&self) -> u32 {
        self.0
    }

    pub const fn contains(&self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }

    pub fn insert(&mut self, other: Self) {
        self.0 |= other.0;
    }

    pub fn remove(&mut self, other: Self) {
        self.0 &= !other.0;
    }

    pub const fn from_bits(bits: u32) -> Self {
        Self(bits)
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
    /// Largest emissive strength; values above 1 are HDR and make bloom pick the surface up.
    pub const MAX_EMISSIVE: f32 = 64.0;
    /// Stored index of refraction for 1.5 (glass); see [`Material::ior`].
    pub const DEFAULT_IOR: u8 = 50;
    /// Stored room depth for rooms as deep as wide; see [`Material::interior_depth`].
    pub const DEFAULT_INTERIOR_DEPTH: u8 = 64;
    /// Stored share of lit rooms (three in four); see [`Material::interior_lights`].
    pub const DEFAULT_INTERIOR_LIGHTS: u8 = 191;
    /// Stored share of curtained rooms (half); see [`Material::interior_curtains`].
    pub const DEFAULT_INTERIOR_CURTAINS: u8 = 128;
    /// Most room images an interior atlas holds across or down.
    pub const MAX_INTERIOR_ATLAS: u32 = 16;

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
            transmission_factor: 0,
            ior: Self::DEFAULT_IOR,
            dispersion: 0,
            alpha_cutoff: 0,
            interior_atlas: 0,
            interior_depth: Self::DEFAULT_INTERIOR_DEPTH,
            interior_lights: Self::DEFAULT_INTERIOR_LIGHTS,
            interior_curtains: Self::DEFAULT_INTERIOR_CURTAINS,
        }
    }

    pub fn pbr() -> Self {
        Self::new([255, 255, 255, 255])
            .with_metallic(0.0)
            .with_roughness(0.5)
    }

    pub fn with_billboarding(mut self) -> Self {
        self.flags.insert(MaterialFlags::BILLBOARDED);
        self
    }

    /// Disable billboarding (use 3D rotation)
    pub fn without_billboarding(mut self) -> Self {
        self.flags.remove(MaterialFlags::BILLBOARDED);
        self
    }

    /// Check if billboarding is enabled
    pub fn is_billboarded(&self) -> bool {
        self.flags.contains(MaterialFlags::BILLBOARDED)
    }

    pub fn with_metallic(mut self, metallic: f32) -> Self {
        self.metallic_factor = (metallic.clamp(0.0, 1.0) * 255.0) as u8;
        self
    }

    pub fn with_roughness(mut self, roughness: f32) -> Self {
        self.roughness_factor = (roughness.clamp(0.0, 1.0) * 255.0) as u8;
        self
    }

    /// How much light passes through (0-1); see [`Material::transmission_factor`].
    pub fn with_transmission(mut self, transmission: f32) -> Self {
        self.transmission_factor = (transmission.clamp(0.0, 1.0) * 255.0).round() as u8;
        self
    }

    /// Index of refraction (1.0-3.55); see [`Material::ior`].
    pub fn with_ior(mut self, ior: f32) -> Self {
        self.ior = Self::encode_ior(ior);
        self
    }

    /// Dispersion as glTF KHR_materials_dispersion (20 / Abbe number, 0-2.55).
    pub fn with_dispersion(mut self, dispersion: f32) -> Self {
        self.dispersion = Self::encode_dispersion(dispersion);
        self
    }

    pub fn encode_dispersion(dispersion: f32) -> u8 {
        (dispersion.clamp(0.0, 2.55) * 100.0).round() as u8
    }

    pub fn dispersion_f32(&self) -> f32 {
        self.dispersion as f32 / 100.0
    }

    pub fn encode_ior(ior: f32) -> u8 {
        ((ior.clamp(1.0, 3.55) - 1.0) * 100.0).round() as u8
    }

    pub fn transmission_f32(&self) -> f32 {
        self.transmission_factor as f32 / 255.0
    }

    pub fn ior_f32(&self) -> f32 {
        1.0 + self.ior as f32 / 100.0
    }

    /// Whether light passes through the surface (refracting glass).
    pub fn is_transmissive(&self) -> bool {
        self.transmission_factor > 0
    }

    pub fn with_emissive(mut self, strength: f32) -> Self {
        self.emissive_strength = Self::encode_emissive(strength);
        self
    }

    /// Strength to its stored form: 255 per 1.0, clamped to `0..=MAX_EMISSIVE`.
    pub fn encode_emissive(strength: f32) -> u16 {
        (strength.clamp(0.0, Self::MAX_EMISSIVE) * 255.0).round() as u16
    }

    pub fn with_alpha(mut self) -> Self {
        self.flags |= MaterialFlags::ALPHA_BLEND;
        self
    }

    /// Cut the surface away where its alpha is below `cutoff` (0-1); 0 turns it off.
    /// The rest stays opaque (depth, shadows, no sorting), unlike [`Self::with_alpha`].
    pub fn with_alpha_cutoff(mut self, cutoff: f32) -> Self {
        self.alpha_cutoff = Self::encode_alpha_cutoff(cutoff);
        self
    }

    /// `cutoff` (0-1) as stored; a cutoff above 0 never rounds to 0 (which means none).
    pub fn encode_alpha_cutoff(cutoff: f32) -> u8 {
        if cutoff.is_nan() || cutoff <= 0.0 {
            return 0;
        }
        ((cutoff.min(1.0) * 255.0).round() as u8).max(1)
    }

    pub fn alpha_cutoff_f32(&self) -> f32 {
        self.alpha_cutoff as f32 / 255.0
    }

    /// Whether parts of the surface are cut away by alpha (see `alpha_cutoff`).
    pub fn is_alpha_masked(&self) -> bool {
        self.alpha_cutoff > 0 && !self.requires_separate_pass()
    }

    /// Interior mapping onto `columns` x `rows` room images in the emissive texture (set
    /// with [`Self::with_emissive_texture`]; without one the rooms are plain); see
    /// [`MaterialFlags::INTERIOR`]. One room fills each unit of the surface's uvs.
    pub fn with_interior(mut self, columns: u32, rows: u32) -> Self {
        self.flags.insert(MaterialFlags::INTERIOR);
        self.interior_atlas = Self::encode_interior_atlas(columns, rows);
        self
    }

    pub fn is_interior(&self) -> bool {
        self.flags.contains(MaterialFlags::INTERIOR)
    }

    /// `columns` x `rows` (each 1-16) as stored in [`Material::interior_atlas`].
    pub fn encode_interior_atlas(columns: u32, rows: u32) -> u8 {
        let max = Self::MAX_INTERIOR_ATLAS;
        (((columns.clamp(1, max) - 1) << 4) | (rows.clamp(1, max) - 1)) as u8
    }

    /// The interior atlas's columns and rows of room images.
    pub fn interior_atlas_size(&self) -> (u32, u32) {
        (
            u32::from(self.interior_atlas >> 4) + 1,
            u32::from(self.interior_atlas & 15) + 1,
        )
    }

    /// Room depth in room widths (1/64 to about 4) as stored in [`Material::interior_depth`].
    pub fn encode_interior_depth(depth: f32) -> u8 {
        if depth.is_nan() {
            return Self::DEFAULT_INTERIOR_DEPTH;
        }
        (depth * 64.0).round().clamp(1.0, 255.0) as u8
    }

    pub fn interior_depth_f32(&self) -> f32 {
        f32::from(self.interior_depth.max(1)) / 64.0
    }

    pub fn interior_lights_f32(&self) -> f32 {
        f32::from(self.interior_lights) / 255.0
    }

    pub fn interior_curtains_f32(&self) -> f32 {
        f32::from(self.interior_curtains) / 255.0
    }

    /// The interior parameters packed for the GPU, as interior.wgsl unpacks them: atlas
    /// (bits 0-7), depth (8-15), lit share (16-23), curtained share (24-31).
    pub fn interior_bits(&self) -> u32 {
        u32::from(self.interior_atlas)
            | (u32::from(self.interior_depth.max(1)) << 8)
            | (u32::from(self.interior_lights) << 16)
            | (u32::from(self.interior_curtains) << 24)
    }

    pub fn with_unlit(mut self) -> Self {
        self.flags.insert(MaterialFlags::UNLIT);
        self
    }

    pub fn with_lit(mut self) -> Self {
        self.flags.remove(MaterialFlags::UNLIT);
        self
    }

    pub fn with_nearest_filtering(mut self) -> Self {
        self.flags.insert(MaterialFlags::USE_NEAREST_FILTERING);
        self
    }

    pub fn with_linear_filtering(mut self) -> Self {
        self.flags.remove(MaterialFlags::USE_NEAREST_FILTERING);
        self
    }

    pub fn uses_nearest_filtering(&self) -> bool {
        self.flags.contains(MaterialFlags::USE_NEAREST_FILTERING)
    }

    pub fn sampler_filtering(&self) -> SamplerFilterMode {
        if self.uses_nearest_filtering() {
            SamplerFilterMode::Nearest
        } else {
            SamplerFilterMode::Linear
        }
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
        Self::new([255, 255, 255, 255]).with_base_color_texture(DEFAULT_CHECKER_TEXTURE_INDEX)
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

    pub fn is_unlit(&self) -> bool {
        self.flags.contains(MaterialFlags::UNLIT)
    }

    pub fn with_shadow_casting(mut self, casts_shadow: bool) -> Self {
        if casts_shadow {
            self.flags.remove(MaterialFlags::NO_SHADOW);
        } else {
            self.flags.insert(MaterialFlags::NO_SHADOW);
        }
        self
    }

    /// Whether objects drawn with this material appear in shadow maps.
    /// Whether the material shows up in shadow maps. Transmissive materials (glass) do
    /// not: light passes them (ray-traced shadows skip them too).
    pub fn casts_shadow(&self) -> bool {
        (!self.is_unlit() && !self.flags.contains(MaterialFlags::NO_SHADOW))
            && !self.is_transmissive()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interior_settings_pack_as_interior_wgsl_reads_them() {
        let mut material = Material::pbr().with_interior(4, 2);
        material.interior_depth = Material::encode_interior_depth(1.5);
        material.interior_lights = 200;
        material.interior_curtains = 10;
        assert!(material.is_interior());
        assert_eq!(material.interior_atlas_size(), (4, 2));
        assert_eq!(material.interior_depth_f32(), 1.5);

        let bits = material.interior_bits();
        // interior_columns / interior_rows / interior_back and the shares.
        assert_eq!(((bits >> 4) & 15) + 1, 4);
        assert_eq!((bits & 15) + 1, 2);
        assert_eq!(((bits >> 8) & 255) as f32 / 32.0, 2.0 * 1.5);
        assert_eq!((bits >> 16) & 255, 200);
        assert_eq!(bits >> 24, 10);
    }

    #[test]
    fn interior_encodings_clamp() {
        assert_eq!(Material::encode_interior_atlas(0, 99), 0x0f);
        assert_eq!(Material::encode_interior_atlas(16, 1), 0xf0);
        assert_eq!(Material::encode_interior_depth(0.0), 1);
        assert_eq!(Material::encode_interior_depth(10.0), 255);
        assert_eq!(
            Material::encode_interior_depth(f32::NAN),
            Material::DEFAULT_INTERIOR_DEPTH
        );
        // A depth of 0 stored by hand still reads as the shallowest room, not none.
        let material = Material {
            interior_depth: 0,
            ..Material::pbr()
        };
        assert_eq!((material.interior_bits() >> 8) & 255, 1);
    }
}
