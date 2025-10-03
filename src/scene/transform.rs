use glam::{Mat4, Quat, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct Transform {
    pub translation: Vec3,
    pub rotation: Quat,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            translation: Vec3::ZERO,
            rotation: Quat::IDENTITY,
            scale: Vec3::ONE,
        }
    }
}

impl Transform {
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    pub fn from_trs(t: Vec3, r: Quat, s: Vec3) -> Self {
        Self {
            translation: t,
            rotation: r,
            scale: s,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn default_is_identity() {
        let m = Transform::default().matrix();
        assert!(m.abs_diff_eq(Mat4::IDENTITY, 1e-6));
    }

    #[test]
    fn translate_then_scale_ok() {
        let tr = Transform::from_trs(Vec3::new(1.0, 2.0, 3.0), Quat::IDENTITY, Vec3::splat(2.0));
        let m = tr.matrix();
        let p = m.transform_point3(Vec3::new(1.0, 0.0, 0.0));
        // Scale happens about origin, then translation
        // (1,0,0) -> (2,0,0) -> (3,2,3)
        assert!(p.abs_diff_eq(Vec3::new(3.0, 2.0, 3.0), 1e-6));
    }
}
