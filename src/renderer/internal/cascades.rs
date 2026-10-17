//! Cascaded shadow maps for directional lights: where each cascade's map lies.
//!
//! The camera's view is cut by depth into up to [`MAX_SHADOW_CASCADES`] slices, near ones
//! short and far ones long ([`split_depths`]), each covered by its own map of the same
//! size, so shadows near the camera get small texels and distant ones still get some.
//! The slices span what the camera sees: from the nearest visible receiver to the
//! farthest, within the light's shadow distance ([`fit_receivers`]).
//!
//! A cascade covers the sphere around its slice rather than the slice itself. The sphere
//! keeps its size however the camera turns ([`slice_sphere`]), and its map is moved by
//! whole texels in a light space that only rotates with the light ([`light_rotation`]),
//! so a world point keeps its place within its texel as the camera moves and shadow
//! edges do not shimmer ([`cascade_view`]). The map's depth range reaches from the
//! sphere's far side back toward the light past every caster, so casters behind the
//! camera or above the view still cast into it.

use glam::{Mat4, Vec3, Vec4};

use crate::renderer::internal::culling::Sphere;
use crate::renderer::lights::MAX_SHADOW_CASCADES;

/// Between even (0) and logarithmic (1) cascade splits: mostly logarithmic, so the first
/// cascades stay small near the camera and the far ones share out the distance.
pub(crate) const SPLIT_LAMBDA: f32 = 0.9;

/// Where the logarithmic splits start at the closest: a camera near plane of a few
/// millimeters would otherwise give the first cascade almost nothing to cover.
const MIN_LOG_NEAR: f32 = 0.2;

/// A cascade's fitted radius is kept while the slice's sphere stays within this share
/// below it, so float noise in the camera's matrices does not resize the map; a new
/// radius gets the same headroom.
const RADIUS_HYSTERESIS: f32 = 0.92;
const RADIUS_HEADROOM: f32 = 1.03;

/// Fitted receiver depths move in steps of 2^(1/8) (about 9%), so the cascades change
/// size only when what the camera sees changes noticeably.
const DEPTH_STEPS_PER_OCTAVE: f32 = 8.0;

/// What the cascades are fitted to: the camera's view and projection without TAA jitter.
#[derive(Clone, Copy, Debug)]
pub(crate) struct CascadeCamera {
    pub view: Mat4,
    pub projection: Mat4,
    pub near: f32,
    pub far: f32,
    pub orthographic: bool,
}

impl CascadeCamera {
    /// The camera's position and unit view direction in world space.
    pub(crate) fn eye_and_forward(&self) -> (Vec3, Vec3) {
        let world = self.view.inverse();
        let eye = world.w_axis.truncate();
        let forward = (-world.z_axis.truncate())
            .try_normalize()
            .unwrap_or(Vec3::NEG_Z);
        (eye, forward)
    }
}

/// The view depths cascades cover: `near` to `far`, and whether `far` was cut short by
/// the shadow distance (then shadows fade out toward it instead of ending in a line).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CascadeRange {
    pub near: f32,
    pub far: f32,
    pub fade: bool,
}

/// The view depths the receivers of shadows in view span.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ReceiverDepths {
    /// Some receiver's extent is not known: the whole view counts.
    Unknown,
    /// Spheres around them reach from `.0` to `.1` (empty when `.0 > .1`).
    Span(f32, f32),
}

impl ReceiverDepths {
    /// The span of `receivers` (their spheres) along the camera's view.
    pub(crate) fn of<'a>(
        camera: &CascadeCamera,
        receivers: impl IntoIterator<Item = &'a Sphere>,
    ) -> Self {
        let (eye, forward) = camera.eye_and_forward();
        let mut lo = f32::INFINITY;
        let mut hi = f32::NEG_INFINITY;
        for sphere in receivers {
            if !sphere.radius.is_finite() || !sphere.center.is_finite() {
                return Self::Unknown;
            }
            let depth = (sphere.center - eye).dot(forward);
            lo = lo.min(depth - sphere.radius);
            hi = hi.max(depth + sphere.radius);
        }
        Self::Span(lo, hi)
    }
}

/// Fit the cascades' depth range to the receivers the camera sees (`receivers`), within
/// `distance` of the camera (the light's shadow distance) and the camera's own range.
/// The fitted ends are rounded outward in small steps so they hold still while the view
/// barely changes. None when nothing in view can receive a shadow.
pub(crate) fn fit_receivers(
    camera: &CascadeCamera,
    receivers: ReceiverDepths,
    distance: f32,
) -> Option<CascadeRange> {
    let near = camera.near.max(1e-4);
    let limit = camera.far.min(distance.max(near * 2.0));
    let ReceiverDepths::Span(lo, hi) = receivers else {
        return Some(CascadeRange {
            near,
            far: limit,
            fade: true,
        });
    };
    if !(hi > near && lo < limit) {
        return None;
    }
    let near = quantize_down(lo.max(near)).max(near);
    let fitted_far = quantize_up(hi.max(near * 1.01));
    let far = fitted_far.min(limit);
    Some(CascadeRange {
        near: near.min(far * 0.5),
        far,
        fade: fitted_far > limit,
    })
}

fn quantize_up(value: f32) -> f32 {
    step_to_value((value.max(1e-4).log2() * DEPTH_STEPS_PER_OCTAVE).ceil())
}

fn quantize_down(value: f32) -> f32 {
    step_to_value((value.max(1e-4).log2() * DEPTH_STEPS_PER_OCTAVE).floor())
}

fn step_to_value(step: f32) -> f32 {
    (step / DEPTH_STEPS_PER_OCTAVE).exp2()
}

/// The view depths where `count` cascades meet over `range`: `count + 1` values from
/// `range.near` to `range.far` (the rest of the array repeats `range.far`).
pub(crate) fn split_depths(range: CascadeRange, count: usize) -> [f32; MAX_SHADOW_CASCADES + 1] {
    let count = count.clamp(1, MAX_SHADOW_CASCADES);
    let (near, far) = (range.near, range.far.max(range.near * 1.001));
    let log_near = near.max(MIN_LOG_NEAR.min(far * 0.1));
    let mut splits = [far; MAX_SHADOW_CASCADES + 1];
    splits[0] = near;
    for (index, split) in splits.iter_mut().enumerate().take(count).skip(1) {
        let t = index as f32 / count as f32;
        let logarithmic = log_near * (far / log_near).powf(t);
        let even = near + (far - near) * t;
        *split = (SPLIT_LAMBDA * logarithmic + (1.0 - SPLIT_LAMBDA) * even).clamp(near, far);
    }
    splits
}

/// The sphere around the camera's view between view depths `near` and `far`, in view
/// space: its centre lies on the slice's axis, where the near and far corners are equally
/// far (or on the far plane when the near corners are inside the far corners' sphere),
/// so it depends only on the projection and keeps its size as the camera turns.
pub(crate) fn slice_sphere(camera: &CascadeCamera, near: f32, far: f32) -> (Vec3, f32) {
    let p = camera.projection;
    // A view-space point (x, y, -d) lands on clip x = P00 x + P20 (-d) + P30, w = d (or 1
    // for an orthographic camera); solve for x at the edges of clip space.
    let corner = |sx: f32, sy: f32, depth: f32| {
        let w = if camera.orthographic { 1.0 } else { depth };
        let x = (sx * w + p.z_axis.x * depth - p.w_axis.x) / p.x_axis.x;
        let y = (sy * w + p.z_axis.y * depth - p.w_axis.y) / p.y_axis.y;
        Vec3::new(x, y, -depth)
    };
    let corners: [Vec3; 8] = std::array::from_fn(|index| {
        let sx = if index & 1 == 0 { -1.0 } else { 1.0 };
        let sy = if index & 2 == 0 { -1.0 } else { 1.0 };
        corner(sx, sy, if index & 4 == 0 { near } else { far })
    });
    let near_center = (corners[0] + corners[3]) * 0.5;
    let far_center = (corners[4] + corners[7]) * 0.5;
    let reach = |center: Vec3, points: &[Vec3]| {
        points
            .iter()
            .map(|point| (*point - center).length_squared())
            .fold(0.0_f32, f32::max)
    };
    let a2 = reach(near_center, &corners[..4]);
    let b2 = reach(far_center, &corners[4..]);
    let length = (far_center - near_center).length().max(1e-6);
    // Distance along the axis from the near centre where both corner sets are equally
    // far: a² + t² = b² + (L - t)².
    let t = ((b2 - a2 + length * length) / (2.0 * length)).clamp(0.0, length);
    let center = near_center + (far_center - near_center) * (t / length);
    let radius = corners
        .iter()
        .map(|point| point.distance(center))
        .fold(0.0_f32, f32::max);
    (center, radius)
}

/// `raw` with hysteresis against the radius used last (`previous`, 0 for none): kept
/// while the new sphere fits in it with little to spare, else the new one with headroom.
pub(crate) fn settle_radius(raw: f32, previous: f32) -> f32 {
    if previous > 0.0 && raw <= previous && raw >= previous * RADIUS_HYSTERESIS {
        previous
    } else {
        raw * RADIUS_HEADROOM
    }
}

/// World to light space for a light shining along `direction`: a rotation only, so a
/// world point's light-space position never changes as the cascades move.
pub(crate) fn light_rotation(direction: Vec3) -> Mat4 {
    let direction = direction.try_normalize().unwrap_or(Vec3::NEG_Y);
    let up = if direction.dot(Vec3::Y).abs() > 0.99 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    Mat4::look_at_rh(Vec3::ZERO, direction, up)
}

/// One cascade's map: how light space maps onto it, and the projection it is drawn with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CascadeView {
    /// Light-space x and y (meters) to map coordinates: `uv = xy * [x, y] + [z, w]`.
    pub uv_transform: [f32; 4],
    /// Light-space z to depth (0 at the near plane, toward the light): `z * [0] + [1]`.
    pub depth_transform: [f32; 2],
    /// A texel's width in meters.
    pub texel: f32,
    /// World space to clip space, for drawing the map.
    pub view_proj: Mat4,
}

/// The view of a cascade covering `sphere` (world space) with `size` texels across, in
/// light space `light_view`: centred on whole texels, from the sphere's far side back to
/// `caster_reach` (the light-space distance of the caster nearest the light, if any),
/// both ends rounded outward so small changes leave the view as it is.
pub(crate) fn cascade_view(
    light_view: Mat4,
    sphere: Sphere,
    size: u32,
    caster_reach: Option<f32>,
) -> CascadeView {
    let radius = sphere.radius.max(1e-3);
    let texel = 2.0 * radius / size.max(1) as f32;
    let center = light_view.transform_point3(sphere.center);
    let cx = (center.x / texel).round() * texel;
    let cy = (center.y / texel).round() * texel;
    // Distances along the light, from the light space origin. The near end moves in
    // coarse steps: it follows the caster nearest the light, which may be one moving
    // about, and depth precision to spare makes a deep range cost nothing.
    let step = (radius * 0.25).max(texel);
    let near_step = (radius * 2.0).max(8.0);
    let middle = -center.z;
    let far = ((middle + radius) / step).ceil() * step;
    let mut near = middle - radius;
    if let Some(reach) = caster_reach.filter(|reach| reach.is_finite()) {
        near = near.min(reach);
    }
    let near = (near / near_step).floor() * near_step;
    let depth_scale = -1.0 / (far - near);
    let depth_offset = -near / (far - near);
    let projection = Mat4::from_cols(
        Vec4::new(1.0 / radius, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 1.0 / radius, 0.0, 0.0),
        Vec4::new(0.0, 0.0, depth_scale, 0.0),
        Vec4::new(-cx / radius, -cy / radius, depth_offset, 1.0),
    );
    CascadeView {
        uv_transform: [
            0.5 / radius,
            -0.5 / radius,
            0.5 - 0.5 * cx / radius,
            0.5 + 0.5 * cy / radius,
        ],
        depth_transform: [depth_scale, depth_offset],
        texel,
        view_proj: projection * light_view,
    }
}

/// The light-space distance (along the light, as [`cascade_view`] measures it) of the
/// caster nearest the light among `casters` (unbounded ones skipped).
pub(crate) fn caster_reach<'a>(
    light_view: Mat4,
    casters: impl IntoIterator<Item = &'a Sphere>,
) -> Option<f32> {
    let mut reach: Option<f32> = None;
    for sphere in casters {
        if !sphere.radius.is_finite() || !sphere.center.is_finite() {
            continue;
        }
        let distance = -light_view.transform_point3(sphere.center).z - sphere.radius;
        reach = Some(reach.map_or(distance, |reach| reach.min(distance)));
    }
    reach
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera(eye: Vec3, target: Vec3) -> CascadeCamera {
        CascadeCamera {
            view: Mat4::look_at_rh(eye, target, Vec3::Y),
            projection: Mat4::perspective_rh(60f32.to_radians(), 16.0 / 9.0, 0.1, 500.0),
            near: 0.1,
            far: 500.0,
            orthographic: false,
        }
    }

    #[test]
    fn splits_grow_from_near_to_far() {
        let range = CascadeRange {
            near: 0.1,
            far: 150.0,
            fade: true,
        };
        let splits = split_depths(range, 4);
        assert_eq!(splits[0], 0.1);
        assert_eq!(splits[4], 150.0);
        for pair in splits.windows(2) {
            assert!(pair[1] > pair[0], "{splits:?}");
        }
        // The first cascade stays within a few meters, the last takes the far half.
        assert!(splits[1] < 5.0 && splits[3] < 75.0, "{splits:?}");
        // Fewer cascades: the rest repeat the far end.
        let two = split_depths(range, 2);
        assert_eq!(&two[2..], &[150.0; 3]);
    }

    #[test]
    fn slice_spheres_hold_their_slice_and_ignore_the_camera_turning() {
        let a = camera(Vec3::new(0.0, 2.0, 0.0), Vec3::new(0.0, 2.0, -1.0));
        let b = camera(Vec3::new(3.0, 5.0, 1.0), Vec3::new(-4.0, 0.0, 2.5));
        for (near, far) in [(0.1, 2.0), (2.0, 9.0), (9.0, 40.0), (40.0, 150.0)] {
            let (center, radius) = slice_sphere(&a, near, far);
            let (_, turned) = slice_sphere(&b, near, far);
            assert!((radius - turned).abs() < radius * 1e-4);
            // Every corner of the slice is inside.
            for corner in crate::renderer::internal::culling::frustum_corners(a.projection) {
                let depth = -corner.z;
                let scale = |d: f32| corner * (d / depth);
                for point in [scale(near), scale(far)] {
                    assert!(
                        point.distance(center) <= radius * 1.0001,
                        "{point} {center} {radius}"
                    );
                }
            }
            // And it is not much bigger than the slice's far corners need.
            assert!(radius < far * 1.4 + 1.0, "{near}..{far}: {radius}");
        }
    }

    #[test]
    fn orthographic_slices_are_boxes() {
        let camera = CascadeCamera {
            view: Mat4::IDENTITY,
            projection: Mat4::orthographic_rh(-8.0, 8.0, -4.5, 4.5, 0.1, 100.0),
            near: 0.1,
            far: 100.0,
            orthographic: true,
        };
        let (center, radius) = slice_sphere(&camera, 10.0, 20.0);
        assert!(
            center.abs_diff_eq(Vec3::new(0.0, 0.0, -15.0), 1e-4),
            "{center}"
        );
        let expected = (8.0f32 * 8.0 + 4.5 * 4.5 + 5.0 * 5.0).sqrt();
        assert!((radius - expected).abs() < 1e-3, "{radius} vs {expected}");
    }

    #[test]
    fn cascades_keep_world_points_on_their_texels_as_they_move() {
        let light = light_rotation(Vec3::new(0.3, -1.0, 0.2));
        let point = Vec3::new(3.3, 0.7, -5.1);
        let fraction = |focus: Vec3| {
            let view = cascade_view(
                light,
                Sphere {
                    center: focus,
                    radius: 12.0,
                },
                2048,
                None,
            );
            let [sx, sy, ox, oy] = view.uv_transform;
            let local = light.transform_point3(point);
            let texels = glam::Vec2::new(local.x * sx + ox, local.y * sy + oy) * 2048.0;
            texels - texels.floor()
        };
        let a = fraction(Vec3::ZERO);
        let b = fraction(Vec3::new(0.123, 0.4, 0.457));
        assert!((a - b).abs().max_element() < 1e-2, "{a:?} vs {b:?}");
    }

    #[test]
    fn cascade_views_draw_what_their_transforms_sample() {
        let light = light_rotation(Vec3::new(-0.4, -0.8, 0.45));
        let sphere = Sphere {
            center: Vec3::new(10.0, 1.0, -7.0),
            radius: 9.0,
        };
        let view = cascade_view(light, sphere, 1024, Some(-40.0));
        for point in [
            sphere.center,
            sphere.center + Vec3::new(5.0, -3.0, 2.0),
            sphere.center + Vec3::new(-6.0, 4.0, -1.0),
        ] {
            let clip = view.view_proj * point.extend(1.0);
            let local = light.transform_point3(point);
            let [sx, sy, ox, oy] = view.uv_transform;
            let uv = glam::Vec2::new(local.x * sx + ox, local.y * sy + oy);
            let expected = glam::Vec2::new(clip.x * 0.5 + 0.5, -clip.y * 0.5 + 0.5);
            assert!(uv.abs_diff_eq(expected, 1e-5), "{uv} vs {expected}");
            let depth = local.z * view.depth_transform[0] + view.depth_transform[1];
            assert!((depth - clip.z).abs() < 1e-5);
            assert!((0.0..=1.0).contains(&depth), "{depth}");
        }
        // The caster reach pulls the near plane toward the light, past a caster 40 m
        // beyond the light space origin.
        let caster = light.inverse().transform_point3(Vec3::new(0.0, 0.0, 39.9));
        let depth = (view.view_proj * caster.extend(1.0)).z;
        assert!((0.0..0.5).contains(&depth), "{depth}");
    }

    #[test]
    fn receivers_fit_the_range_and_hold_still() {
        let camera = camera(Vec3::new(0.0, 1.5, 4.0), Vec3::new(0.0, 0.5, 0.0));
        let room = [
            Sphere {
                center: Vec3::new(0.0, 0.5, 0.0),
                radius: 1.0,
            },
            Sphere {
                center: Vec3::new(0.0, 0.0, -2.0),
                radius: 3.0,
            },
        ];
        let range = fit_receivers(&camera, ReceiverDepths::of(&camera, &room), 150.0).unwrap();
        // The nearest receiver is about 3.1 m away (its sphere's front).
        assert!(range.near > 2.5 && range.near <= 3.12, "{range:?}");
        assert!(range.far >= 9.0 && range.far < 11.0, "{range:?}");
        assert!(!range.fade, "the receivers end before the shadow distance");
        // A small change keeps the same steps.
        let moved = [
            room[0],
            Sphere {
                radius: 3.02,
                ..room[1]
            },
        ];
        assert_eq!(
            fit_receivers(&camera, ReceiverDepths::of(&camera, &moved), 150.0),
            Some(range)
        );
        // Past the shadow distance the range stops there and fades.
        let big = [Sphere {
            center: Vec3::ZERO,
            radius: 1000.0,
        }];
        let range = fit_receivers(&camera, ReceiverDepths::of(&camera, &big), 150.0).unwrap();
        assert_eq!(range.far, 150.0);
        assert!(range.fade);
        // Unknown receivers: the whole view.
        let unknown = ReceiverDepths::of(&camera, &[Sphere::UNBOUNDED]);
        assert_eq!(unknown, ReceiverDepths::Unknown);
        assert_eq!(fit_receivers(&camera, unknown, 150.0).unwrap().near, 0.1);
        // Nothing in front of the camera: nothing to shadow.
        let behind = [Sphere {
            center: Vec3::new(0.0, 1.5, 30.0),
            radius: 1.0,
        }];
        assert_eq!(
            fit_receivers(&camera, ReceiverDepths::of(&camera, &behind), 150.0),
            None
        );
        assert_eq!(
            fit_receivers(&camera, ReceiverDepths::of(&camera, &[]), 150.0),
            None
        );
    }

    #[test]
    fn radius_hysteresis_keeps_small_changes() {
        let first = settle_radius(10.0, 0.0);
        assert!(first > 10.0);
        assert_eq!(settle_radius(10.01, first), first);
        assert_eq!(settle_radius(9.8, first), first);
        assert!(settle_radius(11.0, first) > 11.0);
        assert!(settle_radius(5.0, first) < 6.0);
    }
}
