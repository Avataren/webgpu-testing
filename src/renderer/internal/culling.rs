//! Frustum culling of batch instances against a camera or shadow view.
//!
//! Every instance gets a world-space bounding sphere from its mesh's local box and its
//! transform (`world_sphere`); a view's frustum comes from its view-projection matrix.
//! A sphere the frustum does not touch cannot produce a fragment, since the GPU clips to
//! the same planes, so skipping its draw changes nothing in the image.

use glam::{Mat4, Vec3, Vec4};

use crate::asset::{LocalBounds, MeshData};
use crate::scene::transform::Transform;

/// A bounding sphere in world space.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sphere {
    pub center: Vec3,
    pub radius: f32,
}

impl Sphere {
    /// A sphere no frustum rejects, for instances whose extent is not known (skinned
    /// meshes, vertices moved by a shader, transforms written by the GPU).
    pub(crate) const UNBOUNDED: Self = Self {
        center: Vec3::ZERO,
        radius: f32::INFINITY,
    };

    /// The smallest axis-aligned box around every sphere, as a sphere.
    pub(crate) fn around(spheres: &[Sphere]) -> Option<Sphere> {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for sphere in spheres {
            if !sphere.radius.is_finite() || !sphere.center.is_finite() {
                return Some(Sphere::UNBOUNDED);
            }
            min = min.min(sphere.center - Vec3::splat(sphere.radius));
            max = max.max(sphere.center + Vec3::splat(sphere.radius));
        }
        (!spheres.is_empty()).then(|| Sphere {
            center: (min + max) * 0.5,
            radius: ((max - min) * 0.5).length(),
        })
    }
}

/// The sphere around `bounds` placed by `transform` (scale, then rotation, then
/// translation, as `Transform::matrix`): the box's centre moved with it, and the radius
/// of its farthest corner after scaling.
pub(crate) fn world_sphere(bounds: &LocalBounds, transform: &Transform) -> Sphere {
    let center = transform.translation + transform.rotation * (transform.scale * bounds.center());
    let radius = (bounds.half_extents() * transform.scale).length();
    Sphere { center, radius }
}

/// Each joint's box around the vertices it moves (by any weight) in a skinned mesh's bind
/// pose, by joint index; `None` for joints that move none.
pub(crate) fn joint_bounds(data: &MeshData) -> Vec<Option<LocalBounds>> {
    let mut joints: Vec<Option<LocalBounds>> = Vec::new();
    let Some(skin) = data.skin.as_ref().filter(|_| data.is_skinned()) else {
        return joints;
    };
    for (vertex, influence) in data.vertices.iter().zip(skin) {
        let position = Vec3::from_array(vertex.pos);
        for (joint, weight) in influence.joints.iter().zip(influence.weights) {
            if weight == 0 {
                continue;
            }
            let joint = *joint as usize;
            if joints.len() <= joint {
                joints.resize(joint + 1, None);
            }
            let bounds = joints[joint].get_or_insert(LocalBounds {
                min: position,
                max: position,
            });
            bounds.min = bounds.min.min(position);
            bounds.max = bounds.max.max(position);
        }
    }
    joints
}

/// The box around a skinned mesh posed by `palette` (mesh-space joint matrices), from its
/// [`joint_bounds`]: each joint's bind-pose box moved by its matrix. A skinned vertex is a
/// weighted mean of its joints' matrices applied to it, and each of those lies in that
/// joint's moved box, so the vertex lies in the box around them all.
pub(crate) fn skinned_bounds(
    joints: &[Option<LocalBounds>],
    palette: &[Mat4],
) -> Option<LocalBounds> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for (bounds, matrix) in joints.iter().zip(palette) {
        let Some(bounds) = bounds else {
            continue;
        };
        // The box of a box moved by an affine matrix: its moved centre, and its half
        // extents through the matrix's absolute values.
        let center = matrix.transform_point3(bounds.center());
        let half = bounds.half_extents();
        let extent = Vec3::new(
            matrix.x_axis.x.abs() * half.x
                + matrix.y_axis.x.abs() * half.y
                + matrix.z_axis.x.abs() * half.z,
            matrix.x_axis.y.abs() * half.x
                + matrix.y_axis.y.abs() * half.y
                + matrix.z_axis.y.abs() * half.z,
            matrix.x_axis.z.abs() * half.x
                + matrix.y_axis.z.abs() * half.y
                + matrix.z_axis.z.abs() * half.z,
        );
        min = min.min(center - extent);
        max = max.max(center + extent);
    }
    (min.cmple(max).all() && min.is_finite() && max.is_finite()).then_some(LocalBounds { min, max })
}

/// The six clip planes of a view-projection matrix (wgpu clip space: -w..w in x and y,
/// 0..w in z).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Frustum {
    /// `(n, d)` with `n` a unit normal pointing inward: a point `p` is inside when
    /// `dot(n, p) + d >= 0` for every plane.
    planes: [Vec4; 6],
}

impl Frustum {
    pub(crate) fn from_view_proj(view_proj: Mat4) -> Self {
        let row = |i: usize| {
            Vec4::new(
                view_proj.x_axis[i],
                view_proj.y_axis[i],
                view_proj.z_axis[i],
                view_proj.w_axis[i],
            )
        };
        let (x, y, z, w) = (row(0), row(1), row(2), row(3));
        Self {
            planes: [w + x, w - x, w + y, w - y, z, w - z].map(normalize_plane),
        }
    }

    /// Whether any part of `sphere` may be inside the frustum. Conservative: a sphere
    /// near a corner can pass while being outside, never the reverse.
    pub(crate) fn intersects(&self, sphere: Sphere) -> bool {
        if !sphere.radius.is_finite() || !sphere.center.is_finite() {
            return true;
        }
        // A little slack for the float error of the projection and the plane normals.
        let margin = sphere.radius * 1.001 + 1e-3;
        self.planes
            .iter()
            .all(|plane| plane.truncate().dot(sphere.center) + plane.w >= -margin)
    }
}

impl Frustum {
    /// Whether every one of `points` lies outside one of the planes, so a convex shape
    /// spanned by them cannot touch the frustum. Conservative: shapes it keeps may still
    /// miss the frustum.
    pub(crate) fn excludes(&self, points: &[Vec3]) -> bool {
        self.planes.iter().any(|plane| {
            points
                .iter()
                .all(|point| plane.truncate().dot(*point) + plane.w < -1e-4)
        })
    }
}

/// The corners of the box `view_proj` maps to clip space (wgpu: x and y -1..1, z 0..1), in
/// world space.
pub(crate) fn frustum_corners(view_proj: Mat4) -> [Vec3; 8] {
    let inverse = view_proj.inverse();
    std::array::from_fn(|corner| {
        let ndc = Vec4::new(
            if corner & 1 == 0 { -1.0 } else { 1.0 },
            if corner & 2 == 0 { -1.0 } else { 1.0 },
            if corner & 4 == 0 { 0.0 } else { 1.0 },
            1.0,
        );
        let world = inverse * ndc;
        world.truncate() / world.w
    })
}

/// Scale a plane so its normal is a unit vector; a degenerate plane (a projection
/// without that constraint, or a broken matrix) rejects nothing.
fn normalize_plane(plane: Vec4) -> Vec4 {
    let length = plane.truncate().length();
    if length.is_finite() && length > 1e-12 {
        plane / length
    } else {
        Vec4::new(0.0, 0.0, 0.0, 1.0)
    }
}

/// Which instances of a list of batches a frustum sees. Reused across views to keep its
/// allocations.
#[derive(Default)]
pub(crate) struct Visibility {
    /// Per batch: whether any of its instances is visible.
    batches: Vec<bool>,
    /// Per batch: its instances' flags in `instances`, or an empty range when every
    /// instance is visible (or the whole batch is not).
    ranges: Vec<(u32, u32)>,
    instances: Vec<bool>,
}

impl Visibility {
    /// Test every batch of `batches` (and the instances of those with bounds) against
    /// `frustum`, keeping only the instances `keep(batch, instance)` accepts (instances
    /// without bounds pass the frustum).
    pub(crate) fn compute_filtered(
        &mut self,
        batches: &crate::renderer::internal::PreparedBatches,
        frustum: &Frustum,
        keep: impl Fn(usize, usize) -> bool,
    ) {
        self.batches.clear();
        self.ranges.clear();
        self.instances.clear();
        for (index, batch) in batches.all().iter().enumerate() {
            let start = self.instances.len() as u32;
            if batch
                .bounding_sphere
                .is_some_and(|sphere| !frustum.intersects(sphere))
            {
                self.batches.push(false);
                self.ranges.push((start, 0));
                continue;
            }
            let mut any = false;
            for instance in 0..batch.instances.len() {
                let visible = keep(index, instance)
                    && batch
                        .bounds
                        .get(instance)
                        .is_none_or(|sphere| frustum.intersects(*sphere));
                any |= visible;
                self.instances.push(visible);
            }
            self.batches.push(any);
            self.ranges.push((start, batch.instances.len() as u32));
        }
    }

    /// Whether batch `index` has anything to draw.
    pub(crate) fn batch(&self, index: usize) -> bool {
        self.batches.get(index).copied().unwrap_or(true)
    }

    /// Batch `index`'s per-instance flags; empty when every instance is visible (or the
    /// batch is not: check [`Self::batch`] first).
    pub(crate) fn instances(&self, index: usize) -> &[bool] {
        match self.ranges.get(index) {
            Some(&(start, len)) => &self.instances[start as usize..(start + len) as usize],
            None => &[],
        }
    }
}

/// Whether instance `index` is drawn, given a batch's per-instance flags (`visible`
/// empty: all are).
#[inline]
pub(crate) fn instance_visible(visible: &[bool], index: usize) -> bool {
    visible.get(index).copied().unwrap_or(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Quat;

    fn camera() -> Mat4 {
        let view = Mat4::look_at_rh(Vec3::new(0.0, 1.0, 5.0), Vec3::ZERO, Vec3::Y);
        Mat4::perspective_rh(60f32.to_radians(), 16.0 / 9.0, 0.1, 100.0) * view
    }

    #[test]
    fn spheres_in_front_pass_and_behind_fail() {
        let frustum = Frustum::from_view_proj(camera());
        assert!(frustum.intersects(Sphere {
            center: Vec3::ZERO,
            radius: 0.5
        }));
        assert!(!frustum.intersects(Sphere {
            center: Vec3::new(0.0, 1.0, 20.0),
            radius: 0.5
        }));
        assert!(!frustum.intersects(Sphere {
            center: Vec3::new(0.0, 0.0, -200.0),
            radius: 0.5
        }));
        assert!(!frustum.intersects(Sphere {
            center: Vec3::new(50.0, 0.0, 0.0),
            radius: 0.5
        }));
        // A sphere around the camera is always in view.
        assert!(frustum.intersects(Sphere {
            center: Vec3::new(0.0, 1.0, 5.0),
            radius: 1.0
        }));
        assert!(frustum.intersects(Sphere::UNBOUNDED));
    }

    #[test]
    fn orthographic_shadow_frustum_clips_to_its_box() {
        let view = Mat4::look_at_rh(Vec3::new(0.0, 10.0, 0.0), Vec3::ZERO, Vec3::Z);
        let proj = Mat4::orthographic_rh(-5.0, 5.0, -5.0, 5.0, 0.1, 20.0);
        let frustum = Frustum::from_view_proj(proj * view);
        assert!(frustum.intersects(Sphere {
            center: Vec3::new(4.0, 0.0, 4.0),
            radius: 0.5
        }));
        assert!(!frustum.intersects(Sphere {
            center: Vec3::new(7.0, 0.0, 0.0),
            radius: 0.5
        }));
        // Beyond the far plane (below the ground the light looks at).
        assert!(!frustum.intersects(Sphere {
            center: Vec3::new(0.0, -12.0, 0.0),
            radius: 0.5
        }));
    }

    /// Anything the GPU would draw (a point of the sphere inside clip space) is kept.
    #[test]
    fn culling_is_conservative() {
        let view_proj = camera();
        let frustum = Frustum::from_view_proj(view_proj);
        let mut seed = 12345u32;
        let mut random = || {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        for _ in 0..2000 {
            let center = Vec3::new(
                random() * 40.0 - 20.0,
                random() * 40.0 - 20.0,
                random() * 40.0 - 20.0,
            );
            let radius = random() * 3.0;
            let sphere = Sphere { center, radius };
            // Sample the sphere's surface and centre.
            let mut any_inside = false;
            for i in 0..64 {
                let t = i as f32 / 64.0;
                let dir = Vec3::new(
                    (t * 37.0).sin() * (t * 11.0).cos(),
                    (t * 37.0).sin() * (t * 11.0).sin(),
                    (t * 37.0).cos(),
                );
                let point = if i == 0 {
                    center
                } else {
                    center + dir * radius
                };
                let clip = view_proj * point.extend(1.0);
                if clip.w > 0.0
                    && clip.x.abs() <= clip.w
                    && clip.y.abs() <= clip.w
                    && clip.z >= 0.0
                    && clip.z <= clip.w
                {
                    any_inside = true;
                }
            }
            if any_inside {
                assert!(frustum.intersects(sphere), "{sphere:?} was culled");
            }
        }
    }

    #[test]
    fn world_sphere_follows_scale_rotation_and_translation() {
        let bounds = LocalBounds {
            min: Vec3::new(-1.0, 0.0, -1.0),
            max: Vec3::new(1.0, 4.0, 1.0),
        };
        let transform = Transform::from_trs(
            Vec3::new(10.0, 0.0, 0.0),
            Quat::from_rotation_z(std::f32::consts::FRAC_PI_2),
            Vec3::new(2.0, 1.0, 2.0),
        );
        let sphere = world_sphere(&bounds, &transform);
        // The centre (0, 2, 0) scaled to (0, 2, 0), rotated to (-2, 0, 0), moved to x 10.
        assert!(sphere.center.abs_diff_eq(Vec3::new(8.0, 0.0, 0.0), 1e-5));
        // Half extents (1, 2, 1) scaled to (2, 2, 2).
        assert!((sphere.radius - 12f32.sqrt()).abs() < 1e-5);
        // Every corner lies within the sphere.
        let matrix = transform.matrix();
        for corner in 0..8 {
            let local = Vec3::new(
                if corner & 1 == 0 {
                    bounds.min.x
                } else {
                    bounds.max.x
                },
                if corner & 2 == 0 {
                    bounds.min.y
                } else {
                    bounds.max.y
                },
                if corner & 4 == 0 {
                    bounds.min.z
                } else {
                    bounds.max.z
                },
            );
            let world = matrix.transform_point3(local);
            assert!(world.distance(sphere.center) <= sphere.radius + 1e-4);
        }
    }

    #[test]
    fn skinned_bounds_follow_the_joints() {
        use crate::renderer::{SkinVertex, Vertex};
        let vertex = |x: f32, y: f32| Vertex {
            pos: [x, y, 0.0],
            ..bytemuck::Zeroable::zeroed()
        };
        // Two bones: the lower half follows joint 0, the upper half joint 1, the middle
        // both.
        let data = MeshData {
            vertices: vec![vertex(0.0, 0.0), vertex(0.0, 1.0), vertex(0.0, 2.0)],
            indices: vec![0, 1, 2],
            skin: Some(vec![
                SkinVertex {
                    joints: [0, 0, 0, 0],
                    weights: [u16::MAX, 0, 0, 0],
                },
                SkinVertex {
                    joints: [0, 1, 0, 0],
                    weights: [u16::MAX / 2, u16::MAX / 2, 0, 0],
                },
                SkinVertex {
                    joints: [1, 0, 0, 0],
                    weights: [u16::MAX, 0, 0, 0],
                },
            ]),
        };
        let joints = joint_bounds(&data);
        assert_eq!(joints.len(), 2);
        assert_eq!(joints[0].unwrap().max.y, 1.0);
        assert_eq!(joints[1].unwrap().min.y, 1.0);
        // Joint 1 raises the top by 3 meters: the box reaches it, the bottom stays.
        let palette = [
            Mat4::IDENTITY,
            Mat4::from_translation(Vec3::new(0.0, 3.0, 0.0)),
        ];
        let posed = skinned_bounds(&joints, &palette).unwrap();
        assert!(posed.min.abs_diff_eq(Vec3::ZERO, 1e-6));
        assert!(posed.max.abs_diff_eq(Vec3::new(0.0, 5.0, 0.0), 1e-6));
    }

    #[test]
    fn frusta_that_do_not_meet_are_told_apart() {
        let camera_view = camera();
        let frustum = Frustum::from_view_proj(camera_view);
        let corners = frustum_corners(camera_view);
        // A light's face looking away from the camera's view, behind it.
        let behind = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 5.0)
            * Mat4::look_at_rh(Vec3::new(0.0, 1.0, 8.0), Vec3::new(0.0, 1.0, 9.0), Vec3::Y);
        let behind_corners = frustum_corners(behind);
        assert!(
            frustum.excludes(&behind_corners) || Frustum::from_view_proj(behind).excludes(&corners)
        );
        // One looking into the view is kept.
        let ahead = Mat4::perspective_rh(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 5.0)
            * Mat4::look_at_rh(Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 1.0, -1.0), Vec3::Y);
        assert!(!frustum.excludes(&frustum_corners(ahead)));
        assert!(!Frustum::from_view_proj(ahead).excludes(&corners));
    }

    #[test]
    fn instances_without_flags_are_visible() {
        assert!(instance_visible(&[], 7));
        assert!(!instance_visible(&[false], 0));
        assert!(instance_visible(&[false, true], 1));
    }
}
