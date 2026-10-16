//! Collider wireframes: the selected entity's colliders in the editor, and every
//! simulated collider while physics debug drawing is on.

use std::collections::HashSet;

use glam::{Mat4, Vec3};
use hecs::World;
use rapier3d::prelude as rp;

use super::components::{BodyKind, CharacterController, Collider, RigidBody};
use super::shapes::{build_shape, ShapeSource};
use super::world::{world_transform, ScenePhysics};
use super::{from_rp, quat_from_rp};
use crate::asset::Assets;
use crate::renderer::gizmo_shapes::{GizmoColor, GizmoShape};
use crate::scene::components::{MeshComponent, PrimitiveMeshComponent, SelectedInEditor};
use crate::scene::Transform;

const FIXED_COLOR: GizmoColor = [0.35, 0.85, 0.45, 0.85];
const DYNAMIC_COLOR: GizmoColor = [1.0, 0.62, 0.2, 0.9];
const KINEMATIC_COLOR: GizmoColor = [0.35, 0.65, 1.0, 0.9];
const CHARACTER_COLOR: GizmoColor = [0.3, 0.95, 0.95, 0.95];
const SENSOR_COLOR: GizmoColor = [1.0, 0.9, 0.25, 0.8];
/// Line width in pixels.
const LINE_WIDTH: f32 = 1.5;
/// Segment budget for all wireframes in a frame (big level meshes are cut short).
const MAX_SEGMENTS: usize = 40_000;
const CURVE_SUBDIVISIONS: u32 = 16;

/// The edges of `shape` placed at `pose` (body-frame shapes moved into the world).
fn shape_segments(shape: &dyn rp::Shape, pose: Mat4, out: &mut Vec<(Vec3, Vec3)>) {
    let mut push = |points: &[rp::Vector], edges: &mut dyn Iterator<Item = [u32; 2]>| {
        for [a, b] in edges {
            if out.len() >= MAX_SEGMENTS {
                return;
            }
            let (Some(a), Some(b)) = (points.get(a as usize), points.get(b as usize)) else {
                continue;
            };
            out.push((
                pose.transform_point3(from_rp(*a)),
                pose.transform_point3(from_rp(*b)),
            ));
        }
    };
    if let Some(ball) = shape.as_ball() {
        let (points, edges) = ball.to_outline(CURVE_SUBDIVISIONS);
        push(&points, &mut edges.into_iter());
    } else if let Some(cuboid) = shape.as_cuboid() {
        let (points, edges) = cuboid.to_outline();
        push(&points, &mut edges.into_iter());
    } else if let Some(capsule) = shape.as_capsule() {
        let (points, edges) = capsule.to_outline(CURVE_SUBDIVISIONS);
        push(&points, &mut edges.into_iter());
    } else if let Some(cylinder) = shape.as_cylinder() {
        let (points, edges) = cylinder.to_outline(CURVE_SUBDIVISIONS);
        push(&points, &mut edges.into_iter());
    } else if let Some(hull) = shape.as_convex_polyhedron() {
        push(
            hull.points(),
            &mut hull.edges().iter().map(|edge| edge.vertices),
        );
    } else if let Some(mesh) = shape.as_trimesh() {
        let mut seen = HashSet::new();
        let mut edges = mesh
            .indices()
            .iter()
            .flat_map(|[a, b, c]| [[*a, *b], [*b, *c], [*c, *a]]);
        let mut unique = std::iter::from_fn(|| {
            for [a, b] in edges.by_ref() {
                if seen.insert((a.min(b), a.max(b))) {
                    return Some([a, b]);
                }
            }
            None
        });
        push(mesh.vertices(), &mut unique);
    } else if let Some(compound) = shape.as_compound() {
        for (part_pose, part) in compound.shapes() {
            let part_pose = Mat4::from_rotation_translation(
                quat_from_rp(part_pose.rotation),
                from_rp(part_pose.translation),
            );
            shape_segments(part.as_ref(), pose * part_pose, out);
        }
    }
}

fn lines(segments: Vec<(Vec3, Vec3)>, color: GizmoColor, node: &Transform) -> Vec<GizmoShape> {
    let node = node.matrix();
    segments
        .into_iter()
        .map(|(a, b)| {
            GizmoShape::line(
                node.transform_point3(a),
                node.transform_point3(b),
                LINE_WIDTH,
                color,
            )
        })
        .collect()
}

fn pose_matrix(pose: &rp::Pose) -> Mat4 {
    Mat4::from_rotation_translation(quat_from_rp(pose.rotation), from_rp(pose.translation))
}

/// Every collider the simulation has, coloured by body kind.
pub(crate) fn simulation_gizmos(physics: &ScenePhysics, node: &Transform) -> Vec<GizmoShape> {
    let mut shapes = Vec::new();
    let mut budget = 0usize;
    for (handle, collider) in physics.colliders() {
        let color = match physics.collider_kind(handle) {
            Some((_, true, _)) => SENSOR_COLOR,
            Some((_, _, true)) => CHARACTER_COLOR,
            Some((BodyKind::Dynamic, ..)) => DYNAMIC_COLOR,
            Some((BodyKind::Kinematic, ..)) => KINEMATIC_COLOR,
            _ => FIXED_COLOR,
        };
        let mut segments = Vec::new();
        shape_segments(
            collider.shape(),
            pose_matrix(collider.position()),
            &mut segments,
        );
        budget += segments.len();
        shapes.extend(lines(segments, color, node));
        if budget >= MAX_SEGMENTS {
            break;
        }
    }
    shapes
}

/// Wireframes of the colliders the selected entities would get, built from their
/// components (the editor has no running simulation).
pub(crate) fn selected_collider_gizmos(
    world: &World,
    assets: &Assets,
    node: &Transform,
) -> Vec<GizmoShape> {
    let mut shapes = Vec::new();
    let mut query = world.query::<&SelectedInEditor>();
    for (entity, _) in query.iter() {
        let collider = world.get::<&Collider>(entity).ok().map(|c| (*c).clone());
        let body = world.get::<&RigidBody>(entity).ok().map(|b| b.kind);
        let character = world
            .get::<&CharacterController>(entity)
            .ok()
            .map(|c| (*c).clone());
        if collider.is_none() && body.is_none() && character.is_none() {
            continue;
        }
        let transform = world_transform(world, entity);
        let body_pose = Mat4::from_rotation_translation(transform.rotation, transform.translation);
        let mut segments = Vec::new();
        let color = if let Some(controller) = character {
            let (capsule, center) = super::character::body_capsule(&controller);
            let pose = body_pose * Mat4::from_translation(Vec3::Y * center);
            shape_segments(capsule.as_ref(), pose, &mut segments);
            CHARACTER_COLOR
        } else {
            let collider = collider.unwrap_or_default();
            let dynamic = body == Some(BodyKind::Dynamic);
            let body_frame = body_pose.inverse();
            let mut source = ShapeSource::default();
            let own = world
                .get::<&MeshComponent>(entity)
                .ok()
                .and_then(|mesh| assets.meshes.get(mesh.0));
            if let Some(mesh) = own {
                source
                    .meshes
                    .push((mesh.data(), body_frame * transform.matrix()));
                source.primitive = world
                    .get::<&PrimitiveMeshComponent>(entity)
                    .ok()
                    .map(|primitive| primitive.descriptor);
            }
            let children = if collider.include_children {
                super::world::child_meshes(world, entity)
            } else {
                Vec::new()
            };
            if !children.is_empty() {
                source.primitive = None;
            }
            for child in &children {
                if let Some(mesh) = assets.meshes.get(child.mesh.0) {
                    source.meshes.push((mesh.data(), body_frame * child.world));
                }
            }
            if let Some((shape, pose)) = build_shape(&collider, transform.scale, &source, dynamic) {
                shape_segments(
                    shape.as_ref(),
                    body_pose * pose_matrix(&pose),
                    &mut segments,
                );
            }
            match (collider.sensor, body) {
                (true, _) => SENSOR_COLOR,
                (_, Some(BodyKind::Dynamic)) => DYNAMIC_COLOR,
                (_, Some(BodyKind::Kinematic)) => KINEMATIC_COLOR,
                _ => FIXED_COLOR,
            }
        };
        shapes.extend(lines(segments, color, node));
    }
    shapes
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    #[test]
    fn boxes_draw_twelve_edges() {
        let mut segments = Vec::new();
        shape_segments(
            rp::SharedShape::cuboid(1.0, 2.0, 3.0).as_ref(),
            Mat4::from_rotation_translation(Quat::IDENTITY, Vec3::new(10.0, 0.0, 0.0)),
            &mut segments,
        );
        assert_eq!(segments.len(), 12);
        assert!(segments
            .iter()
            .all(|(a, _)| (a.x - 10.0).abs() <= 1.0 + 1e-5));
    }
}
