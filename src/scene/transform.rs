// scene/transform.rs - Verified transform composition
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
    pub const IDENTITY: Self = Self {
        translation: Vec3::ZERO,
        rotation: Quat::IDENTITY,
        scale: Vec3::ONE,
    };

    /// Convert transform to 4x4 matrix
    /// Order: Scale -> Rotate -> Translate (standard TRS)
    pub fn matrix(&self) -> Mat4 {
        Mat4::from_scale_rotation_translation(self.scale, self.rotation, self.translation)
    }

    /// Create transform from translation, rotation, and scale
    pub fn from_trs(t: Vec3, r: Quat, s: Vec3) -> Self {
        Self {
            translation: t,
            rotation: r,
            scale: s,
        }
    }

    /// Combine two transforms: self (parent) * other (child) = world transform of child
    ///
    /// This computes the world transform of a child given:
    /// - self: the parent's world transform
    /// - other: the child's local transform
    ///
    /// The math follows standard transform composition:
    /// 1. Scale is multiplicative: parent.scale * child.scale
    /// 2. Rotation is composed: parent.rotation * child.rotation
    /// 3. Translation is: parent.translation + parent.rotation * (parent.scale * child.translation)
    ///    This means the child's local translation is first scaled by parent's scale,
    ///    then rotated by parent's rotation, then offset by parent's translation.
    pub fn mul_transform(&self, other: &Transform) -> Transform {
        Transform {
            // Translation: parent_translation + parent_rotation * (parent_scale * child_translation)
            translation: self.translation + self.rotation * (self.scale * other.translation),

            // Rotation: parent_rotation * child_rotation
            rotation: self.rotation * other.rotation,

            // Scale: parent_scale * child_scale (component-wise)
            scale: self.scale * other.scale,
        }
    }

    /// Alternative: Compute using matrix multiplication (for verification)
    pub fn mul_transform_via_matrix(&self, other: &Transform) -> Transform {
        let m = self.matrix() * other.matrix();
        let (scale, rotation, translation) = m.to_scale_rotation_translation();
        Transform {
            translation,
            rotation,
            scale,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::PI;

    #[test]
    fn default_is_identity() {
        let m = Transform::default().matrix();
        assert!(m.abs_diff_eq(Mat4::IDENTITY, 1e-6));
    }

    #[test]
    fn identity_transform_composition() {
        let t = Transform::from_trs(Vec3::new(1.0, 2.0, 3.0), Quat::IDENTITY, Vec3::ONE);
        let result = Transform::IDENTITY.mul_transform(&t);

        assert!(result.translation.abs_diff_eq(t.translation, 1e-6));
        assert!(result.rotation.abs_diff_eq(t.rotation, 1e-6));
        assert!(result.scale.abs_diff_eq(t.scale, 1e-6));
    }

    #[test]
    fn simple_translation_chain() {
        // Parent at (5, 0, 0)
        let parent = Transform::from_trs(Vec3::new(5.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ONE);

        // Child offset by (2, 0, 0)
        let child = Transform::from_trs(Vec3::new(2.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ONE);

        // World position should be (7, 0, 0)
        let world = parent.mul_transform(&child);

        assert!(world
            .translation
            .abs_diff_eq(Vec3::new(7.0, 0.0, 0.0), 1e-5));
        assert!(world.scale.abs_diff_eq(Vec3::ONE, 1e-5));
    }

    #[test]
    fn translation_and_scale() {
        // Parent at origin with 2x scale
        let parent = Transform::from_trs(Vec3::ZERO, Quat::IDENTITY, Vec3::splat(2.0));

        // Child at local (1, 0, 0) with 0.5x scale
        let child = Transform::from_trs(Vec3::new(1.0, 0.0, 0.0), Quat::IDENTITY, Vec3::splat(0.5));

        let world = parent.mul_transform(&child);

        // Child's translation (1, 0, 0) is scaled by parent (2x) = (2, 0, 0)
        assert!(world
            .translation
            .abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 1e-5));

        // Child's scale (0.5) * parent's scale (2) = 1.0
        assert!(world.scale.abs_diff_eq(Vec3::ONE, 1e-5));
    }

    #[test]
    fn rotation_affects_child_position() {
        // Parent rotated 90 degrees around Y
        let parent = Transform::from_trs(Vec3::ZERO, Quat::from_rotation_y(PI / 2.0), Vec3::ONE);

        // Child at (1, 0, 0) in local space
        let child = Transform::from_trs(Vec3::new(1.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ONE);

        let world = parent.mul_transform(&child);

        // After 90 degree Y rotation, X becomes -Z
        // So (1, 0, 0) becomes approximately (0, 0, -1)
        assert!(world
            .translation
            .abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), 1e-5));
    }

    #[test]
    fn three_level_hierarchy() {
        // Grandparent at (10, 0, 0)
        let grandparent = Transform::from_trs(Vec3::new(10.0, 0.0, 0.0), Quat::IDENTITY, Vec3::ONE);

        // Parent offset by (0, 5, 0)
        let parent = Transform::from_trs(Vec3::new(0.0, 5.0, 0.0), Quat::IDENTITY, Vec3::ONE);

        // Child offset by (0, 0, 3)
        let child = Transform::from_trs(Vec3::new(0.0, 0.0, 3.0), Quat::IDENTITY, Vec3::ONE);

        // Compute world transforms step by step
        let parent_world = grandparent.mul_transform(&parent);
        let child_world = parent_world.mul_transform(&child);

        // Parent should be at (10, 5, 0)
        assert!(parent_world
            .translation
            .abs_diff_eq(Vec3::new(10.0, 5.0, 0.0), 1e-5));

        // Child should be at (10, 5, 3)
        assert!(child_world
            .translation
            .abs_diff_eq(Vec3::new(10.0, 5.0, 3.0), 1e-5));
    }

    #[test]
    fn complex_hierarchy_with_rotation_and_scale() {
        // Parent: translated (5, 0, 0), rotated 90° Y, scaled 2x
        let parent = Transform::from_trs(
            Vec3::new(5.0, 0.0, 0.0),
            Quat::from_rotation_y(PI / 2.0),
            Vec3::splat(2.0),
        );

        // Child: translated (1, 0, 0) locally, scaled 0.5x
        let child = Transform::from_trs(Vec3::new(1.0, 0.0, 0.0), Quat::IDENTITY, Vec3::splat(0.5));

        let world = parent.mul_transform(&child);

        // Child's local translation (1, 0, 0):
        // - Scaled by parent scale (2x): (2, 0, 0)
        // - Rotated 90° Y: (0, 0, -2)
        // - Translated by parent: (5, 0, -2)
        assert!(world
            .translation
            .abs_diff_eq(Vec3::new(5.0, 0.0, -2.0), 1e-4));

        // Scale: 2.0 * 0.5 = 1.0
        assert!(world.scale.abs_diff_eq(Vec3::ONE, 1e-5));

        // Rotation: 90° Y
        assert!(world.rotation.abs_diff_eq(parent.rotation, 1e-5));
    }

    #[test]
    fn verify_against_matrix_multiplication() {
        // Create various transforms and verify our TRS composition matches matrix multiplication
        let parent = Transform::from_trs(
            Vec3::new(3.0, 2.0, 1.0),
            Quat::from_rotation_y(PI / 4.0),
            Vec3::splat(1.5),
        );

        let child = Transform::from_trs(
            Vec3::new(1.0, -1.0, 2.0),
            Quat::from_rotation_x(PI / 6.0),
            Vec3::splat(0.75),
        );

        let world_trs = parent.mul_transform(&child);
        let world_matrix = parent.mul_transform_via_matrix(&child);

        assert!(world_trs
            .translation
            .abs_diff_eq(world_matrix.translation, 1e-4));
        assert!(world_trs.rotation.abs_diff_eq(world_matrix.rotation, 1e-4));
        assert!(world_trs.scale.abs_diff_eq(world_matrix.scale, 1e-4));
    }

    #[test]
    fn matrix_conversion_roundtrip() {
        let t = Transform::from_trs(
            Vec3::new(1.0, 2.0, 3.0),
            Quat::from_rotation_y(PI / 3.0),
            Vec3::new(1.0, 2.0, 1.5),
        );

        let m = t.matrix();
        let (s, r, pos) = m.to_scale_rotation_translation();

        assert!(pos.abs_diff_eq(t.translation, 1e-5));
        assert!(r.abs_diff_eq(t.rotation, 1e-5));
        assert!(s.abs_diff_eq(t.scale, 1e-5));
    }
}
