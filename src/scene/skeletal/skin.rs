//! Skinned meshes: a mesh deformed by a skeleton of joint entities.

use std::sync::Arc;

use glam::Mat4;
use hecs::{Entity, World};

use crate::scene::components::WorldTransform;

/// Most joints one skinned mesh may use.
pub const MAX_JOINTS: usize = 1024;

/// Deforms the entity's mesh (which must carry a skin stream, see
/// [`MeshData::skin`](crate::asset::MeshData::skin)) with a skeleton.
///
/// Joints are ordinary entities, usually descendants of the character's root, so they
/// can be animated, inspected and have things parented to them. `joints[i]` pairs with
/// `inverse_bind[i]`, which maps the mesh's bind-pose space into joint `i`'s space. As
/// glTF prescribes, the skinned mesh entity's own transform does not move the vertices;
/// the joints place them.
#[derive(Debug, Clone)]
pub struct SkinnedMesh {
    pub joints: Vec<Entity>,
    pub inverse_bind: Arc<[Mat4]>,
}

impl SkinnedMesh {
    pub fn new(joints: Vec<Entity>, inverse_bind: Arc<[Mat4]>) -> Self {
        Self {
            joints,
            inverse_bind,
        }
    }

    /// Point the joints at copies, e.g. after duplicating a character. Joints `map`
    /// does not know are kept.
    pub fn remap(&mut self, map: impl Fn(Entity) -> Option<Entity>) {
        for joint in &mut self.joints {
            if let Some(mapped) = map(*joint) {
                *joint = mapped;
            }
        }
    }

    /// The joint matrices for the skinning shader, in the mesh entity's local space:
    /// `inverse(mesh_world) * joint_world * inverse_bind`. A joint that no longer exists
    /// (or has no world transform yet) leaves its vertices in the bind pose.
    pub fn palette(&self, world: &World, mesh_world: Mat4) -> Vec<Mat4> {
        let to_local = mesh_world.inverse();
        self.joints
            .iter()
            .zip(self.inverse_bind.iter())
            .map(
                |(joint, inverse_bind)| match world.get::<&WorldTransform>(*joint) {
                    Ok(joint_world) => to_local * joint_world.0.matrix() * *inverse_bind,
                    Err(_) => Mat4::IDENTITY,
                },
            )
            .collect()
    }
}

/// Every skinned mesh's bones as (parent joint, joint) world positions within one
/// world, for drawing skeletons.
pub fn skeleton_segments(world: &World) -> Vec<(glam::Vec3, glam::Vec3)> {
    use crate::scene::components::Parent;
    let mut segments = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for (_, skin) in world.query::<&SkinnedMesh>().iter() {
        for &joint in &skin.joints {
            if !seen.insert(joint) {
                continue;
            }
            let Some(parent) = world.get::<&Parent>(joint).ok().map(|p| p.0) else {
                continue;
            };
            if !skin.joints.contains(&parent) {
                continue;
            }
            if let (Ok(a), Ok(b)) = (
                world.get::<&WorldTransform>(parent),
                world.get::<&WorldTransform>(joint),
            ) {
                segments.push((a.0.translation, b.0.translation));
            }
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::transform::Transform;
    use glam::{Quat, Vec3};

    #[test]
    fn bind_pose_palette_is_identity_and_follows_joints() {
        let mut world = World::new();
        // Joint at y = 1 in the bind pose; the mesh sits at x = 5.
        let joint_bind = Transform::from_translation(Vec3::Y);
        let joint = world.spawn((WorldTransform(joint_bind),));
        let mesh_world = Mat4::from_translation(Vec3::X * 5.0);
        // Bind matrices are relative to the mesh: the joint is at (-5, 1, 0) from it.
        let inverse_bind: Arc<[Mat4]> =
            Arc::from([(mesh_world.inverse() * joint_bind.matrix()).inverse()]);
        let skin = SkinnedMesh::new(vec![joint], inverse_bind);

        let palette = skin.palette(&world, mesh_world);
        assert!(palette[0].abs_diff_eq(Mat4::IDENTITY, 1e-5));

        // Rotate the joint a quarter turn: a vertex 1 m above it swings to the side.
        world.get::<&mut WorldTransform>(joint).unwrap().0.rotation =
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let palette = skin.palette(&world, mesh_world);
        let vertex = Vec3::new(-5.0, 2.0, 0.0);
        let skinned = palette[0].transform_point3(vertex);
        assert!(
            skinned.abs_diff_eq(Vec3::new(-6.0, 1.0, 0.0), 1e-5),
            "{skinned}"
        );

        // A despawned joint falls back to the bind pose.
        world.despawn(joint).unwrap();
        assert_eq!(skin.palette(&world, mesh_world)[0], Mat4::IDENTITY);
    }
}
