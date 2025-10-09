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
    pub fn from_instance(model: Mat4, material_index: u32) -> Self {
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
    pub color: [f32; 4],           // 16 bytes
    pub texture_indices: [u32; 4], // 16 bytes
    pub extras: [u32; 4],          // 16 bytes
    pub factors: [f32; 4],         // 16 bytes
}

impl MaterialData {
    pub fn from_material(material: &crate::renderer::Material) -> Self {
        Self {
            color: material.color_f32(),
            texture_indices: [
                material.base_color_texture,
                material.metallic_roughness_texture,
                material.normal_texture,
                material.emissive_texture,
            ],
            extras: [material.occlusion_texture, material.flags_bits(), 0, 0],
            factors: [
                material.metallic_f32(),
                material.roughness_f32(),
                material.emissive_f32(),
                0.0,
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn object_data_size() {
        // 64 + 4 + 12 padding = 80 bytes
        assert_eq!(std::mem::size_of::<ObjectData>(), 80);
    }

    #[test]
    fn object_data_material_index() {
        use glam::{Mat4, Vec3};

        let object = ObjectData::from_instance(Mat4::from_scale(Vec3::ONE), 7);

        assert_eq!(object.material_index, 7);
        assert_eq!(object._padding, [0, 0, 0]);
    }

    #[test]
    fn material_data_conversions() {
        use crate::renderer::Material;

        let material = Material::new([220, 180, 140, 200])
            .with_metallic(0.25)
            .with_roughness(0.75)
            .with_emissive(0.5)
            .with_base_color_texture(3)
            .with_normal_texture(5)
            .with_occlusion_texture(7);

        let data = MaterialData::from_material(&material);

        assert_eq!(
            data.color,
            [220.0 / 255.0, 180.0 / 255.0, 140.0 / 255.0, 200.0 / 255.0]
        );
        assert_eq!(data.texture_indices[0], 3);
        assert_eq!(data.texture_indices[2], 5);
        assert_eq!(data.extras[0], 7);
        assert_eq!(data.extras[1], material.flags_bits());
        assert!((data.factors[0] - 0.25).abs() < 0.01);
        assert!((data.factors[1] - 0.75).abs() < 0.01);
        assert!((data.factors[2] - 0.5).abs() < 0.01);
    }
}
