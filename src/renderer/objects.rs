// renderer/objects.rs (PBR version)
use bytemuck::{Pod, Zeroable};
use glam::Mat4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct ObjectData {
    pub model: [[f32; 4]; 4],           // 64 bytes
    pub color: [f32; 4],                 // 16 bytes
    pub base_color_texture: u32,         // 4 bytes
    pub metallic_roughness_texture: u32, // 4 bytes
    pub normal_texture: u32,             // 4 bytes
    pub emissive_texture: u32,           // 4 bytes
    pub occlusion_texture: u32,          // 4 bytes
    pub material_flags: u32,             // 4 bytes
    pub metallic_factor: f32,            // 4 bytes
    pub roughness_factor: f32,           // 4 bytes
    pub emissive_strength: f32,          // 4 bytes
    pub _padding: u32,                   // 4 bytes
}

impl ObjectData {
    pub fn from_material(model: Mat4, material: &crate::renderer::Material) -> Self {
        Self {
            model: model.to_cols_array_2d(),
            color: material.color_f32(),
            base_color_texture: material.base_color_texture,
            metallic_roughness_texture: material.metallic_roughness_texture,
            normal_texture: material.normal_texture,
            emissive_texture: material.emissive_texture,
            occlusion_texture: material.occlusion_texture,
            material_flags: material.flags_bits(),
            metallic_factor: material.metallic_f32(),
            roughness_factor: material.roughness_f32(),
            emissive_strength: material.emissive_f32(),
            _padding: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn object_data_size() {
        // 64 + 16 + 5*4 + 4 + 3*4 + 4 = 64 + 16 + 20 + 4 + 12 + 4 = 120 bytes
        assert_eq!(std::mem::size_of::<ObjectData>(), 120);
    }
}