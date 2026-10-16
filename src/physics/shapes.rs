//! Turning a [`Collider`] and the entity's geometry into a Rapier shape.

use glam::{Mat4, Quat, Vec3};
use rapier3d::prelude as rp;

use super::components::{Collider, ColliderShape};
use super::{pose, to_rp};
use crate::asset::MeshData;
use crate::renderer::primitives::PrimitiveMeshDescriptor;

/// Thickness given to a flat plane's collider, behind its face (m).
const PLANE_THICKNESS: f32 = 0.1;

/// Geometry a collider can be fitted to: meshes placed in the body's frame (the entity's
/// rotation and translation removed, its scale kept).
#[derive(Default)]
pub(crate) struct ShapeSource<'a> {
    /// The entity's own built-in primitive, when it is one and nothing else is included.
    pub primitive: Option<PrimitiveMeshDescriptor>,
    pub meshes: Vec<(&'a MeshData, Mat4)>,
}

impl ShapeSource<'_> {
    fn points(&self) -> Vec<rp::Vector> {
        self.meshes
            .iter()
            .flat_map(|(mesh, transform)| {
                mesh.vertices
                    .iter()
                    .map(move |vertex| to_rp(transform.transform_point3(vertex.pos.into())))
            })
            .collect()
    }

    fn triangles(&self) -> (Vec<rp::Vector>, Vec<[u32; 3]>) {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        for (mesh, transform) in &self.meshes {
            let base = vertices.len() as u32;
            vertices.extend(
                mesh.vertices
                    .iter()
                    .map(|vertex| to_rp(transform.transform_point3(vertex.pos.into()))),
            );
            // A mirroring transform flips the winding; keep triangles facing outward.
            let flip = transform.determinant() < 0.0;
            indices.extend(mesh.indices.as_chunks::<3>().0.iter().map(|[a, b, c]| {
                if flip {
                    [base + a, base + c, base + b]
                } else {
                    [base + a, base + b, base + c]
                }
            }));
        }
        (vertices, indices)
    }
}

fn convex_hull(source: &ShapeSource<'_>) -> Option<rp::SharedShape> {
    let points = source.points();
    if points.len() < 4 {
        return None;
    }
    rp::SharedShape::convex_hull(&points)
}

fn triangle_mesh(source: &ShapeSource<'_>) -> Option<rp::SharedShape> {
    let (vertices, indices) = source.triangles();
    if indices.is_empty() {
        return None;
    }
    // Merged duplicate vertices and fixed internal edges stop characters and rolling
    // bodies from catching on the seams between triangles.
    rp::SharedShape::trimesh_with_flags(vertices, indices, rp::TriMeshFlags::FIX_INTERNAL_EDGES)
        .map_err(|err| log::warn!("cannot build a triangle mesh collider: {err}"))
        .ok()
}

fn nearly_equal(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1e-3 * a.abs().max(b.abs()).max(1.0)
}

/// Capsule of full height `height`; one too short for its caps is a ball.
pub(crate) fn upright_capsule(radius: f32, height: f32) -> rp::SharedShape {
    let radius = radius.max(0.01);
    let half_segment = (height * 0.5 - radius).max(0.0);
    if half_segment <= 1e-4 {
        rp::SharedShape::ball(radius)
    } else {
        rp::SharedShape::capsule_y(half_segment, radius)
    }
}

/// The shape of a built-in primitive at `scale`, and where it sits in the body frame.
fn primitive_shape(
    primitive: PrimitiveMeshDescriptor,
    scale: Vec3,
    source: &ShapeSource<'_>,
    moving: bool,
) -> Option<(rp::SharedShape, Mat4)> {
    let s = scale.abs();
    match primitive {
        PrimitiveMeshDescriptor::Cube => Some((
            rp::SharedShape::cuboid(0.5 * s.x, 0.5 * s.y, 0.5 * s.z),
            Mat4::IDENTITY,
        )),
        PrimitiveMeshDescriptor::Sphere if nearly_equal(s.x, s.y) && nearly_equal(s.y, s.z) => {
            Some((rp::SharedShape::ball(s.x), Mat4::IDENTITY))
        }
        // The plane faces +Z; it gets a thin slab behind its face so fast bodies do not
        // tunnel through.
        PrimitiveMeshDescriptor::Plane => Some((
            rp::SharedShape::cuboid(0.5 * s.x, 0.5 * s.y, PLANE_THICKNESS * 0.5),
            Mat4::from_translation(Vec3::new(0.0, 0.0, -PLANE_THICKNESS * 0.5)),
        )),
        // Unit radius, length 1 along Z; Rapier's cylinders stand along Y.
        PrimitiveMeshDescriptor::Cylinder if nearly_equal(s.x, s.y) => Some((
            rp::SharedShape::cylinder(0.5 * s.z, s.x),
            Mat4::from_quat(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
        )),
        _ => mesh_shape(source, moving).map(|shape| (shape, Mat4::IDENTITY)),
    }
}

/// The mesh itself for scenery, its hull for bodies that move.
fn mesh_shape(source: &ShapeSource<'_>, moving: bool) -> Option<rp::SharedShape> {
    if moving {
        convex_hull(source)
    } else {
        triangle_mesh(source)
    }
}

/// Build `collider`'s shape for an entity at `scale` (its world scale), and the shape's
/// pose in the body's frame. `dynamic` bodies never get triangle meshes (Rapier cannot
/// give a hollow mesh a sensible mass), only convex hulls.
pub(crate) fn build_shape(
    collider: &Collider,
    scale: Vec3,
    source: &ShapeSource<'_>,
    dynamic: bool,
) -> Option<(rp::SharedShape, rp::Pose)> {
    let s = scale.abs().max(Vec3::splat(1e-4));
    let (shape, local) = match &collider.shape {
        ColliderShape::Auto => match source.primitive {
            Some(primitive) => primitive_shape(primitive, scale, source, dynamic)?,
            None if !source.meshes.is_empty() => (mesh_shape(source, dynamic)?, Mat4::IDENTITY),
            None => (
                rp::SharedShape::cuboid(0.5 * s.x, 0.5 * s.y, 0.5 * s.z),
                Mat4::IDENTITY,
            ),
        },
        ColliderShape::Box { size } => {
            let half = Vec3::from(*size).abs() * s * 0.5;
            (
                rp::SharedShape::cuboid(half.x.max(1e-3), half.y.max(1e-3), half.z.max(1e-3)),
                Mat4::IDENTITY,
            )
        }
        ColliderShape::Sphere { radius } => (
            rp::SharedShape::ball((radius.abs() * s.max_element()).max(1e-3)),
            Mat4::IDENTITY,
        ),
        ColliderShape::Capsule { radius, height } => (
            upright_capsule(radius.abs() * s.x.max(s.z), height.abs() * s.y),
            Mat4::IDENTITY,
        ),
        ColliderShape::Cylinder { radius, height } => (
            rp::SharedShape::cylinder(
                (height.abs() * s.y * 0.5).max(1e-3),
                (radius.abs() * s.x.max(s.z)).max(1e-3),
            ),
            Mat4::IDENTITY,
        ),
        ColliderShape::Mesh => (mesh_shape(source, dynamic)?, Mat4::IDENTITY),
        ColliderShape::ConvexHull => (convex_hull(source)?, Mat4::IDENTITY),
    };
    let offset = Vec3::from(collider.offset) * scale;
    let (_, rotation, translation) = local.to_scale_rotation_translation();
    Some((shape, pose(translation + offset, rotation)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cube_source() -> (MeshData, Option<PrimitiveMeshDescriptor>) {
        (
            PrimitiveMeshDescriptor::Cube.mesh_data(),
            Some(PrimitiveMeshDescriptor::Cube),
        )
    }

    #[test]
    fn auto_fits_primitives_to_their_scale() {
        let (mesh, primitive) = cube_source();
        let scale = Vec3::new(4.0, 0.5, 2.0);
        let source = ShapeSource {
            primitive,
            meshes: vec![(&mesh, Mat4::from_scale(scale))],
        };
        let (shape, _) = build_shape(&Collider::default(), scale, &source, false).unwrap();
        let cuboid = shape.as_cuboid().expect("a cube fits a box");
        assert!((cuboid.half_extents.x - 2.0).abs() < 1e-5);
        assert!((cuboid.half_extents.y - 0.25).abs() < 1e-5);

        let sphere = PrimitiveMeshDescriptor::Sphere.mesh_data();
        let source = ShapeSource {
            primitive: Some(PrimitiveMeshDescriptor::Sphere),
            meshes: vec![(&sphere, Mat4::from_scale(Vec3::splat(0.5)))],
        };
        let (shape, _) =
            build_shape(&Collider::default(), Vec3::splat(0.5), &source, true).unwrap();
        assert!((shape.as_ball().unwrap().radius - 0.5).abs() < 1e-5);
    }

    #[test]
    fn meshes_become_triangles_for_scenery_and_hulls_for_bodies() {
        let mesh = PrimitiveMeshDescriptor::Torus.mesh_data();
        let source = ShapeSource {
            primitive: None,
            meshes: vec![(&mesh, Mat4::IDENTITY)],
        };
        let (fixed, _) = build_shape(&Collider::default(), Vec3::ONE, &source, false).unwrap();
        assert!(fixed.as_trimesh().is_some());
        let (moving, _) = build_shape(&Collider::default(), Vec3::ONE, &source, true).unwrap();
        assert!(moving.as_convex_polyhedron().is_some());
    }

    #[test]
    fn offsets_scale_with_the_entity() {
        let collider = Collider {
            shape: ColliderShape::Sphere { radius: 0.5 },
            offset: [0.0, 1.0, 0.0],
            ..Default::default()
        };
        let (_, pose) =
            build_shape(&collider, Vec3::splat(2.0), &ShapeSource::default(), true).unwrap();
        assert!((pose.translation.y - 2.0).abs() < 1e-5);
    }

    #[test]
    fn short_capsules_become_balls() {
        assert!(upright_capsule(0.5, 0.8).as_ball().is_some());
        assert!(upright_capsule(0.3, 1.8).as_capsule().is_some());
    }
}
