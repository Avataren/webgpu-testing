// renderer/objects.rs (PBR version)
use bytemuck::{Pod, Zeroable};
use glam::Mat4;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable, Debug)]
pub struct ObjectData {
    pub model: [[f32; 4]; 4], // 64 bytes
    pub material_index: u32,  // 4 bytes
    pub _padding: [u32; 3],   // 12 bytes (ensures 80 byte stride)
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
    pub _padding: f32,                   // 4 bytes
    pub _padding2: [u32; 2],             // 8 bytes (ensures 64 byte stride)
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
            _padding: 0.0,
            _padding2: [0, 0],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn object_data_size() {
        // 64 (model) + 4 (index) + 12 padding = 80 bytes
        assert_eq!(std::mem::size_of::<ObjectData>(), 80);
    }

    #[test]
    fn material_data_pbr_factors() {
        use crate::renderer::Material;

        let material = Material::new([255, 255, 255, 255])
            .with_metallic(0.75)
            .with_roughness(0.25)
            .with_base_color_texture(0);

        assert_eq!(material.metallic_factor, 191);
        assert_eq!(material.roughness_factor, 63);

        let material_gpu = MaterialData::from_material(&material);

        assert!((material_gpu.metallic_factor - 0.75).abs() < 0.01);
        assert!((material_gpu.roughness_factor - 0.25).abs() < 0.01);
        assert_eq!(material_gpu.material_flags & 0b10, 0); // MR texture flag should be off
        assert_eq!(material_gpu.material_flags & 0b1, 0b1); // Base color flag on
    }

    #[test]
    fn pbr_grid_material_values() {
        use crate::renderer::Material;

        let grid_size = 5usize;
        let mut metallic_values = Vec::new();
        let mut roughness_values = Vec::new();

        for row in 0..grid_size {
            for col in 0..grid_size {
                let metallic = col as f32 / (grid_size - 1) as f32;
                let roughness = row as f32 / (grid_size - 1) as f32;

                let material = Material::new([220, 220, 220, 255])
                    .with_metallic(metallic)
                    .with_roughness(roughness)
                    .with_base_color_texture(0);

                let material_gpu = MaterialData::from_material(&material);
                metallic_values.push(material_gpu.metallic_factor);
                roughness_values.push(material_gpu.roughness_factor);
            }
        }

        assert!(metallic_values.iter().any(|&m| m < 0.1));
        assert!(metallic_values.iter().any(|&m| (m - 1.0).abs() < 0.01));
        assert!(roughness_values.iter().any(|&r| r < 0.1));
        assert!(roughness_values.iter().any(|&r| (r - 1.0).abs() < 0.01));
    }
}
