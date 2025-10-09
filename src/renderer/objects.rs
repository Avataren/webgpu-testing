// renderer/objects.rs (PBR version)
use bytemuck::{Pod, Zeroable};
use glam::Mat4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct ObjectData {
    pub model: [[f32; 4]; 4], // 64 bytes
    pub material_index: u32,  // 4 bytes
    pub _padding: [u32; 3],   // 12 bytes (ensures 16-byte alignment)
}

impl ObjectData {
    pub fn from_material_index(model: Mat4, material_index: u32) -> Self {
        Self {
            model: model.to_cols_array_2d(),
            material_index,
            _padding: [0; 3],
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct MaterialData {
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
    pub _padding: [u32; 3],              // 12 bytes (ensures 16-byte stride)
}

impl MaterialData {
    pub fn from_material(material: &crate::renderer::Material) -> Self {
        Self {
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
            _padding: [0; 3],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn object_data_size() {
        // 64 bytes for model + 16 bytes for index + padding = 80 bytes total
        assert_eq!(std::mem::size_of::<ObjectData>(), 80);
    }

    #[test]
    fn material_data_conversion() {
        use crate::renderer::Material;

        let material = Material::new([255, 200, 100, 255])
            .with_metallic(0.5)
            .with_roughness(0.25)
            .with_emissive(0.75)
            .with_base_color_texture(1)
            .with_metallic_roughness_texture(2)
            .with_normal_texture(3)
            .with_emissive_texture(4)
            .with_occlusion_texture(5);

        let data = MaterialData::from_material(&material);

        assert_eq!(data.base_color_texture, 1);
        assert_eq!(data.metallic_roughness_texture, 2);
        assert_eq!(data.normal_texture, 3);
        assert_eq!(data.emissive_texture, 4);
        assert_eq!(data.occlusion_texture, 5);
        assert!((data.color[0] - (255.0 / 255.0)).abs() < f32::EPSILON);
        assert!((data.metallic_factor - 0.5).abs() < 0.01);
        assert!((data.roughness_factor - 0.25).abs() < 0.01);
        assert!((data.emissive_strength - 0.75).abs() < 0.01);
    }
}
