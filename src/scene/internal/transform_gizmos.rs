//! The editor's move / rotate / scale gizmo, drawn as screen-space shapes
//! ([`GizmoShape`]) of constant pixel size, and the hit test that picks its handles.
//!
//! Drawing and picking share one layout ([`GizmoLayout`]), so what is under the
//! pointer is always what is drawn there: handles that face the camera edge-on fade
//! out and stop being pickable together.

use super::lights::safe_normalize;
use super::rendering::CameraVectors;
use crate::renderer::gizmo_shapes::{GizmoColor, GizmoShape};
use crate::scene::transform::Transform;
use crate::scene::{
    TransformGizmoAxis, TransformGizmoHandle, TransformGizmoMode, TransformGizmoSpace,
};
use glam::{Quat, Vec3};

/// Viewport height assumed until the gizmos have been drawn once.
pub(crate) const DEFAULT_VIEWPORT_HEIGHT: f32 = 720.0;

const AXES: [TransformGizmoAxis; 3] = [
    TransformGizmoAxis::X,
    TransformGizmoAxis::Y,
    TransformGizmoAxis::Z,
];

const TRANSLATE_PLANES: [(TransformGizmoAxis, TransformGizmoAxis); 3] = [
    (TransformGizmoAxis::X, TransformGizmoAxis::Y),
    (TransformGizmoAxis::Y, TransformGizmoAxis::Z),
    (TransformGizmoAxis::Z, TransformGizmoAxis::X),
];

// Colors (sRGB, straight alpha).
const AXIS_X_COLOR: GizmoColor = [0.96, 0.29, 0.36, 1.0];
const AXIS_Y_COLOR: GizmoColor = [0.49, 0.83, 0.25, 1.0];
const AXIS_Z_COLOR: GizmoColor = [0.24, 0.56, 1.0, 1.0];
const NEUTRAL_COLOR: GizmoColor = [0.94, 0.95, 0.97, 1.0];
const PLANE_FILL_ALPHA: f32 = 0.28;
const PLANE_HOVER_FILL_ALPHA: f32 = 0.55;
const SCREEN_RING_ALPHA: f32 = 0.55;
/// A dark rim under the handles keeps them readable over bright or same-colored
/// objects.
const HALO_COLOR: GizmoColor = [0.0, 0.0, 0.0, 0.32];
const HALO_PX: f32 = 1.25;
const TRACKBALL_ALPHA: f32 = 0.05;
/// How far a hovered handle's color moves towards white.
const HOVER_LIGHTEN: f32 = 0.45;

// Sizes, in pixels.
/// From the center to the tip of a move arrow or a scale handle.
const AXIS_LENGTH_PX: f32 = 110.0;
/// Axis lines start this far out, leaving the center handle clear.
const AXIS_START_PX: f32 = 18.0;
const LINE_WIDTH_PX: f32 = 2.5;
const HOVER_LINE_WIDTH_PX: f32 = 3.5;
const ARROW_LENGTH_PX: f32 = 19.0;
const ARROW_HALF_WIDTH_PX: f32 = 7.0;
const PLANE_START_PX: f32 = 28.0;
const PLANE_END_PX: f32 = 52.0;
const PLANE_OUTLINE_PX: f32 = 1.5;
const CENTER_RING_RADIUS_PX: f32 = 8.0;
const CENTER_RING_WIDTH_PX: f32 = 2.0;
const SCALE_HANDLE_HALF_PX: f32 = 6.0;
const SCALE_CENTER_HALF_PX: f32 = 8.5;
const SQUARE_CORNER_PX: f32 = 1.5;
const ROTATE_RADIUS_PX: f32 = 94.0;
const SCREEN_RING_RADIUS_PX: f32 = 112.0;
const SCREEN_RING_WIDTH_PX: f32 = 1.75;
const RING_SEGMENTS: usize = 72;

// Pick tolerances, in pixels from the drawn shape.
const AXIS_PICK_PX: f32 = 8.0;
const CENTER_PICK_PX: f32 = 11.0;
const RING_PICK_PX: f32 = 7.0;
const SCALE_HANDLE_PICK_PX: f32 = 10.0;

/// The world-space size of one pixel at a point, and the camera's view direction.
struct ScreenScale {
    camera: CameraVectors,
    forward: Vec3,
}

impl ScreenScale {
    fn new(camera: CameraVectors) -> Self {
        let forward = safe_normalize(camera.target - camera.position, Vec3::NEG_Z);
        Self { camera, forward }
    }

    fn world_per_pixel(&self, position: Vec3) -> f32 {
        let height = self.camera.viewport_height.max(1.0);
        match self.camera.ortho_height {
            Some(view_height) => view_height / height,
            None => {
                let depth = (position - self.camera.position)
                    .dot(self.forward)
                    .max(1e-3);
                let half_fov = (self.camera.fov_y * 0.5).clamp(1e-4, 1.5);
                2.0 * depth * half_fov.tan() / height
            }
        }
    }

    /// Direction from `position` towards the eye (the view direction for orthographic
    /// cameras).
    fn to_eye(&self, position: Vec3) -> Vec3 {
        match self.camera.ortho_height {
            Some(_) => -self.forward,
            None => safe_normalize(self.camera.position - position, -self.forward),
        }
    }
}

/// Where everything of one gizmo goes this frame.
struct GizmoLayout {
    origin: Vec3,
    rotation: Quat,
    /// World units per pixel at the gizmo's center.
    unit: f32,
    to_eye: Vec3,
}

impl GizmoLayout {
    fn new(camera: CameraVectors, transform: Transform, space: TransformGizmoSpace) -> Self {
        let scale = ScreenScale::new(camera);
        let origin = transform.translation;
        let rotation = match space {
            TransformGizmoSpace::Local => transform.rotation,
            TransformGizmoSpace::World => Quat::IDENTITY,
        };
        Self {
            origin,
            rotation,
            unit: scale.world_per_pixel(origin),
            to_eye: scale.to_eye(origin),
        }
    }

    fn axis(&self, axis: TransformGizmoAxis) -> Vec3 {
        axis_direction(self.rotation, axis)
    }

    /// The point `pixels` out along `direction` from the center.
    fn along(&self, direction: Vec3, pixels: f32) -> Vec3 {
        self.origin + direction * (pixels * self.unit)
    }

    /// 0 for an axis pointing straight at the camera (it would be a dot), 1 once it
    /// is clearly visible on screen.
    fn axis_visibility(&self, axis: TransformGizmoAxis) -> f32 {
        let sin = self.axis(axis).cross(self.to_eye).length();
        smoothstep(0.12, 0.25, sin)
    }

    /// 0 for a plane seen edge-on, 1 once it is clearly open to the camera.
    fn plane_visibility(&self, a: TransformGizmoAxis, b: TransformGizmoAxis) -> f32 {
        let normal = self.axis(a).cross(self.axis(b));
        smoothstep(0.15, 0.35, normal.dot(self.to_eye).abs())
    }

    fn plane_corners(&self, a: TransformGizmoAxis, b: TransformGizmoAxis) -> [Vec3; 4] {
        let (da, db) = (self.axis(a), self.axis(b));
        let (near, far) = (PLANE_START_PX * self.unit, PLANE_END_PX * self.unit);
        [
            self.origin + da * near + db * near,
            self.origin + da * far + db * near,
            self.origin + da * far + db * far,
            self.origin + da * near + db * far,
        ]
    }

    /// Points around the rotation ring of `axis`, with whether each faces the camera
    /// (the far half is hidden, as it would be behind the object).
    fn ring_points(&self, axis: TransformGizmoAxis) -> Vec<(Vec3, bool)> {
        let normal = self.axis(axis);
        let (u, v) = orthonormal_basis(normal);
        let radius = ROTATE_RADIUS_PX * self.unit;
        (0..=RING_SEGMENTS)
            .map(|i| {
                let angle = i as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU;
                let offset = (u * angle.cos() + v * angle.sin()) * radius;
                (
                    self.origin + offset,
                    offset.dot(self.to_eye) >= -1e-4 * radius,
                )
            })
            .collect()
    }

    /// The circle around the view direction for free rotation.
    fn screen_ring_points(&self) -> Vec<Vec3> {
        let (u, v) = orthonormal_basis(self.to_eye);
        let radius = SCREEN_RING_RADIUS_PX * self.unit;
        (0..=RING_SEGMENTS)
            .map(|i| {
                let angle = i as f32 / RING_SEGMENTS as f32 * std::f32::consts::TAU;
                self.origin + (u * angle.cos() + v * angle.sin()) * radius
            })
            .collect()
    }
}

/// Shapes collected for one gizmo: halos go under everything else.
#[derive(Default)]
struct Canvas {
    halos: Vec<GizmoShape>,
    shapes: Vec<GizmoShape>,
}

impl Canvas {
    fn push(&mut self, shape: GizmoShape) {
        self.shapes.push(shape);
    }

    /// Push a shape with a dark rim drawn beneath it, as strong as the shape is opaque.
    fn push_with_halo(&mut self, shape: GizmoShape, opacity: f32) {
        let halo = with_alpha(HALO_COLOR, opacity);
        self.halos.push(shape.grown(HALO_PX, halo));
        self.shapes.push(shape);
    }

    fn finish(self) -> Vec<GizmoShape> {
        let mut shapes = self.halos;
        shapes.extend(self.shapes);
        shapes
    }
}

pub(crate) fn build_transform_gizmos(
    camera: CameraVectors,
    gizmo_transform: Transform,
    mode: TransformGizmoMode,
    space: TransformGizmoSpace,
    hovered: Option<TransformGizmoHandle>,
) -> Vec<GizmoShape> {
    let layout = GizmoLayout::new(camera, gizmo_transform, space);
    let mut canvas = Canvas::default();
    match mode {
        TransformGizmoMode::Translate => build_translate(&layout, hovered, &mut canvas),
        TransformGizmoMode::Rotate => build_rotate(&layout, hovered, &mut canvas),
        TransformGizmoMode::Scale => build_scale(&layout, hovered, &mut canvas),
    }
    canvas.finish()
}

/// World length of a gizmo axis at `position`, for turning drags into amounts.
pub(crate) fn gizmo_world_size(camera: CameraVectors, position: Vec3) -> f32 {
    ScreenScale::new(camera).world_per_pixel(position) * AXIS_LENGTH_PX
}

/// Axes in back-to-front order, so nearer ones are drawn over farther ones.
fn axes_back_to_front(layout: &GizmoLayout) -> [TransformGizmoAxis; 3] {
    let mut axes = AXES;
    axes.sort_by(|a, b| {
        let da = layout.axis(*a).dot(layout.to_eye);
        let db = layout.axis(*b).dot(layout.to_eye);
        da.total_cmp(&db)
    });
    axes
}

fn build_translate(
    layout: &GizmoLayout,
    hovered: Option<TransformGizmoHandle>,
    canvas: &mut Canvas,
) {
    for (a, b) in TRANSLATE_PLANES {
        let visibility = layout.plane_visibility(a, b);
        if visibility <= 0.0 {
            continue;
        }
        let hover = hovered == Some(TransformGizmoHandle::TranslatePlane(a, b));
        // A plane takes the color of the axis it is perpendicular to.
        let color = handle_color(axis_color(third_axis(a, b)), hover);
        let fill_alpha = if hover {
            PLANE_HOVER_FILL_ALPHA
        } else {
            PLANE_FILL_ALPHA
        };
        canvas.push(
            GizmoShape::quad(
                layout.plane_corners(a, b),
                with_alpha(color, fill_alpha * visibility),
            )
            .with_outline(PLANE_OUTLINE_PX, with_alpha(color, visibility)),
        );
    }

    for axis in axes_back_to_front(layout) {
        let visibility = layout.axis_visibility(axis);
        if visibility <= 0.0 {
            continue;
        }
        let hover = hovered == Some(TransformGizmoHandle::TranslateAxis(axis));
        let color = with_alpha(handle_color(axis_color(axis), hover), visibility);
        let direction = layout.axis(axis);
        let start = layout.along(direction, AXIS_START_PX);
        let tip = layout.along(direction, AXIS_LENGTH_PX);
        // The line ends inside the arrow head so the two join without a seam.
        let line_end = layout.along(direction, AXIS_LENGTH_PX - ARROW_LENGTH_PX * 0.6);
        canvas.push_with_halo(
            GizmoShape::line(start, line_end, line_width(hover), color),
            visibility,
        );
        let head_scale = if hover { 1.15 } else { 1.0 };
        canvas.push_with_halo(
            GizmoShape::arrow_head(
                layout.origin,
                tip,
                ARROW_LENGTH_PX * head_scale,
                ARROW_HALF_WIDTH_PX * head_scale,
                color,
            ),
            visibility,
        );
    }

    let hover = hovered == Some(TransformGizmoHandle::TranslateCenter);
    let color = handle_color(NEUTRAL_COLOR, hover);
    if hover {
        canvas.push(GizmoShape::disc(
            layout.origin,
            CENTER_RING_RADIUS_PX + CENTER_RING_WIDTH_PX * 0.5,
            with_alpha(color, 0.35),
        ));
    }
    canvas.push_with_halo(
        GizmoShape::ring(
            layout.origin,
            CENTER_RING_RADIUS_PX,
            if hover {
                CENTER_RING_WIDTH_PX + 1.0
            } else {
                CENTER_RING_WIDTH_PX
            },
            color,
        ),
        1.0,
    );
}

fn build_rotate(layout: &GizmoLayout, hovered: Option<TransformGizmoHandle>, canvas: &mut Canvas) {
    // A faint disc behind the rings reads as the sphere they lie on.
    canvas.push(GizmoShape::disc(
        layout.origin,
        ROTATE_RADIUS_PX,
        with_alpha(NEUTRAL_COLOR, TRACKBALL_ALPHA),
    ));

    let screen_hover = hovered == Some(TransformGizmoHandle::RotateScreen);
    let screen_color = handle_color(NEUTRAL_COLOR, screen_hover);
    let screen_alpha = if screen_hover { 1.0 } else { SCREEN_RING_ALPHA };
    push_polyline(
        canvas,
        &layout.screen_ring_points(),
        if screen_hover {
            HOVER_LINE_WIDTH_PX
        } else {
            SCREEN_RING_WIDTH_PX
        },
        with_alpha(screen_color, screen_alpha),
    );

    // Rings seen more edge-on are drawn last: they cross the others' visible halves.
    let mut axes = AXES;
    axes.sort_by(|a, b| {
        let da = layout.axis(*a).dot(layout.to_eye).abs();
        let db = layout.axis(*b).dot(layout.to_eye).abs();
        db.total_cmp(&da)
    });
    for axis in axes {
        let hover = hovered == Some(TransformGizmoHandle::RotateAxis(axis));
        let color = handle_color(axis_color(axis), hover);
        let points = layout.ring_points(axis);
        for pair in points.windows(2) {
            let ((a, a_front), (b, b_front)) = (pair[0], pair[1]);
            if a_front && b_front {
                canvas.push_with_halo(
                    GizmoShape::line(a, b, line_width(hover), color).butt_caps(),
                    1.0,
                );
            }
        }
    }
}

fn build_scale(layout: &GizmoLayout, hovered: Option<TransformGizmoHandle>, canvas: &mut Canvas) {
    for axis in axes_back_to_front(layout) {
        let visibility = layout.axis_visibility(axis);
        if visibility <= 0.0 {
            continue;
        }
        let hover = hovered == Some(TransformGizmoHandle::ScaleAxis(axis));
        let color = with_alpha(handle_color(axis_color(axis), hover), visibility);
        let direction = layout.axis(axis);
        let end = layout.along(direction, AXIS_LENGTH_PX);
        canvas.push_with_halo(
            GizmoShape::line(
                layout.along(direction, AXIS_START_PX),
                end,
                line_width(hover),
                color,
            ),
            visibility,
        );
        let half = if hover {
            SCALE_HANDLE_HALF_PX + 1.0
        } else {
            SCALE_HANDLE_HALF_PX
        };
        canvas.push_with_halo(
            GizmoShape::square(end, half, SQUARE_CORNER_PX, color),
            visibility,
        );
    }

    let hover = hovered == Some(TransformGizmoHandle::ScaleUniform);
    let color = handle_color(NEUTRAL_COLOR, hover);
    let fill_alpha = if hover { 0.55 } else { 0.2 };
    canvas.push_with_halo(
        GizmoShape::square(
            layout.origin,
            SCALE_CENTER_HALF_PX,
            SQUARE_CORNER_PX,
            with_alpha(color, fill_alpha),
        )
        .with_outline(CENTER_RING_WIDTH_PX, color),
        1.0,
    );
}

/// A closed polyline as separate segments meeting flush.
fn push_polyline(canvas: &mut Canvas, points: &[Vec3], width: f32, color: GizmoColor) {
    for pair in points.windows(2) {
        canvas.push(GizmoShape::line(pair[0], pair[1], width, color).butt_caps());
    }
}

pub(crate) fn hit_test(
    camera: CameraVectors,
    gizmo_transform: Transform,
    mode: TransformGizmoMode,
    space: TransformGizmoSpace,
    ray_origin: Vec3,
    ray_dir: Vec3,
) -> Option<TransformGizmoHandle> {
    let ray_dir = safe_normalize(ray_dir, Vec3::ZERO);
    if ray_dir.length_squared() <= 1e-6 {
        return None;
    }
    let layout = GizmoLayout::new(camera, gizmo_transform, space);
    // Distances below are world units at the gizmo; divide by `unit` for pixels.
    let unit = layout.unit.max(1e-9);

    let mut best: Option<(TransformGizmoHandle, f32)> = None;
    let mut consider = |handle: TransformGizmoHandle, pixels: f32, tolerance: f32| {
        if pixels > tolerance {
            return;
        }
        // Rank by distance relative to the handle's tolerance, so a small handle
        // just under the pointer beats a generous one nearby.
        let score = pixels / tolerance;
        if best.is_none_or(|(_, best_score)| score < best_score) {
            best = Some((handle, score));
        }
    };

    match mode {
        TransformGizmoMode::Translate => {
            if let Some(distance) = distance_point_to_ray(ray_origin, ray_dir, layout.origin) {
                consider(
                    TransformGizmoHandle::TranslateCenter,
                    distance / unit,
                    CENTER_PICK_PX,
                );
            }
            for axis in AXES {
                if layout.axis_visibility(axis) < 0.5 {
                    continue;
                }
                let direction = layout.axis(axis);
                if let Some(distance) = distance_to_segment(
                    ray_origin,
                    ray_dir,
                    layout.along(direction, AXIS_START_PX),
                    layout.along(direction, AXIS_LENGTH_PX),
                ) {
                    consider(
                        TransformGizmoHandle::TranslateAxis(axis),
                        distance / unit,
                        AXIS_PICK_PX,
                    );
                }
            }
            for (a, b) in TRANSLATE_PLANES {
                if layout.plane_visibility(a, b) < 0.5 {
                    continue;
                }
                if ray_hits_quad(ray_origin, ray_dir, layout.plane_corners(a, b)) {
                    // Inside the square counts as right on it, below an axis line
                    // right under the pointer.
                    consider(TransformGizmoHandle::TranslatePlane(a, b), 0.5, 1.0);
                }
            }
        }
        TransformGizmoMode::Rotate => {
            for axis in AXES {
                let points = layout.ring_points(axis);
                for pair in points.windows(2) {
                    let ((a, a_front), (b, b_front)) = (pair[0], pair[1]);
                    if !(a_front && b_front) {
                        continue;
                    }
                    if let Some(distance) = distance_to_segment(ray_origin, ray_dir, a, b) {
                        consider(
                            TransformGizmoHandle::RotateAxis(axis),
                            distance / unit,
                            RING_PICK_PX,
                        );
                    }
                }
            }
            for pair in layout.screen_ring_points().windows(2) {
                if let Some(distance) = distance_to_segment(ray_origin, ray_dir, pair[0], pair[1]) {
                    consider(
                        TransformGizmoHandle::RotateScreen,
                        distance / unit,
                        RING_PICK_PX,
                    );
                }
            }
        }
        TransformGizmoMode::Scale => {
            for axis in AXES {
                if layout.axis_visibility(axis) < 0.5 {
                    continue;
                }
                let end = layout.along(layout.axis(axis), AXIS_LENGTH_PX);
                if let Some(distance) = distance_point_to_ray(ray_origin, ray_dir, end) {
                    consider(
                        TransformGizmoHandle::ScaleAxis(axis),
                        distance / unit,
                        SCALE_HANDLE_PICK_PX,
                    );
                }
            }
            if let Some(distance) = distance_point_to_ray(ray_origin, ray_dir, layout.origin) {
                consider(
                    TransformGizmoHandle::ScaleUniform,
                    distance / unit,
                    CENTER_PICK_PX,
                );
            }
        }
    }

    best.map(|(handle, _)| handle)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn line_width(hover: bool) -> f32 {
    if hover {
        HOVER_LINE_WIDTH_PX
    } else {
        LINE_WIDTH_PX
    }
}

fn with_alpha(mut color: GizmoColor, alpha: f32) -> GizmoColor {
    color[3] *= alpha;
    color
}

fn handle_color(color: GizmoColor, hovered: bool) -> GizmoColor {
    if !hovered {
        return color;
    }
    let lighten = |c: f32| c + (1.0 - c) * HOVER_LIGHTEN;
    [
        lighten(color[0]),
        lighten(color[1]),
        lighten(color[2]),
        color[3],
    ]
}

fn axis_base(axis: TransformGizmoAxis) -> Vec3 {
    match axis {
        TransformGizmoAxis::X => Vec3::X,
        TransformGizmoAxis::Y => Vec3::Y,
        TransformGizmoAxis::Z => Vec3::Z,
    }
}

fn axis_direction(rotation: Quat, axis: TransformGizmoAxis) -> Vec3 {
    safe_normalize(rotation * axis_base(axis), axis_base(axis))
}

fn axis_color(axis: TransformGizmoAxis) -> GizmoColor {
    match axis {
        TransformGizmoAxis::X => AXIS_X_COLOR,
        TransformGizmoAxis::Y => AXIS_Y_COLOR,
        TransformGizmoAxis::Z => AXIS_Z_COLOR,
    }
}

fn third_axis(a: TransformGizmoAxis, b: TransformGizmoAxis) -> TransformGizmoAxis {
    AXES.into_iter()
        .find(|axis| *axis != a && *axis != b)
        .unwrap_or(TransformGizmoAxis::Y)
}

fn orthonormal_basis(normal: Vec3) -> (Vec3, Vec3) {
    let normal = safe_normalize(normal, Vec3::Z);
    let helper = if normal.y.abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let u = normal.cross(helper).normalize();
    let v = normal.cross(u);
    (u, v)
}

fn distance_point_to_ray(origin: Vec3, direction: Vec3, point: Vec3) -> Option<f32> {
    let diff = point - origin;
    let t = diff.dot(direction);
    if t < 0.0 {
        return None;
    }
    Some((origin + direction * t - point).length())
}

/// Shortest distance between a ray and the segment `a`-`b`.
fn distance_to_segment(ray_origin: Vec3, ray_dir: Vec3, a: Vec3, b: Vec3) -> Option<f32> {
    let segment = b - a;
    let length = segment.length();
    if length < 1e-9 {
        return distance_point_to_ray(ray_origin, ray_dir, a);
    }
    let axis = segment / length;
    let w0 = ray_origin - a;
    let along = ray_dir.dot(axis);
    let d = ray_dir.dot(w0);
    let e = axis.dot(w0);
    let denom = 1.0 - along * along;
    // Closest point of the segment's line to the ray's line, kept on the segment.
    let t = if denom.abs() > 1e-6 {
        (e - along * d) / denom
    } else {
        e
    };
    let on_segment = a + axis * t.clamp(0.0, length);
    distance_point_to_ray(ray_origin, ray_dir, on_segment)
}

fn ray_hits_quad(ray_origin: Vec3, ray_dir: Vec3, corners: [Vec3; 4]) -> bool {
    let edge_a = corners[1] - corners[0];
    let edge_b = corners[3] - corners[0];
    let normal = edge_a.cross(edge_b);
    let denom = ray_dir.dot(normal);
    if denom.abs() < 1e-12 {
        return false;
    }
    let t = (corners[0] - ray_origin).dot(normal) / denom;
    if !t.is_finite() || t < 0.0 {
        return false;
    }
    let local = ray_origin + ray_dir * t - corners[0];
    let u = local.dot(edge_a) / edge_a.length_squared();
    let v = local.dot(edge_b) / edge_b.length_squared();
    (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> CameraVectors {
        CameraVectors {
            position: Vec3::new(0.0, 0.0, 10.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            fov_y: 60f32.to_radians(),
            viewport_height: 800.0,
            ortho_height: None,
        }
    }

    fn ray_through(point: Vec3) -> (Vec3, Vec3) {
        let origin = camera().position;
        (origin, (point - origin).normalize())
    }

    fn hit(mode: TransformGizmoMode, point: Vec3) -> Option<TransformGizmoHandle> {
        let (origin, dir) = ray_through(point);
        hit_test(
            camera(),
            Transform::IDENTITY,
            mode,
            TransformGizmoSpace::World,
            origin,
            dir,
        )
    }

    #[test]
    fn picks_the_handles_where_they_are_drawn() {
        let unit = ScreenScale::new(camera()).world_per_pixel(Vec3::ZERO);
        let px = |x: f32, y: f32| Vec3::new(x * unit, y * unit, 0.0);
        let plane_middle = (PLANE_START_PX + PLANE_END_PX) * 0.5;
        assert_eq!(
            hit(TransformGizmoMode::Translate, px(60.0, 0.0)),
            Some(TransformGizmoHandle::TranslateAxis(TransformGizmoAxis::X))
        );
        assert_eq!(
            hit(TransformGizmoMode::Translate, px(0.0, AXIS_LENGTH_PX - 4.0)),
            Some(TransformGizmoHandle::TranslateAxis(TransformGizmoAxis::Y))
        );
        assert_eq!(
            hit(TransformGizmoMode::Translate, px(2.0, -3.0)),
            Some(TransformGizmoHandle::TranslateCenter)
        );
        assert_eq!(
            hit(
                TransformGizmoMode::Translate,
                px(plane_middle, plane_middle)
            ),
            Some(TransformGizmoHandle::TranslatePlane(
                TransformGizmoAxis::X,
                TransformGizmoAxis::Y
            ))
        );
        assert_eq!(
            hit(
                TransformGizmoMode::Translate,
                px(AXIS_LENGTH_PX + 30.0, 0.0)
            ),
            None
        );
        // The Z axis points at the camera, so it is hidden and not pickable.
        assert_ne!(
            hit(TransformGizmoMode::Translate, px(0.0, 0.0)),
            Some(TransformGizmoHandle::TranslateAxis(TransformGizmoAxis::Z))
        );
        assert_eq!(
            hit(TransformGizmoMode::Scale, px(AXIS_LENGTH_PX - 1.0, 2.0)),
            Some(TransformGizmoHandle::ScaleAxis(TransformGizmoAxis::X))
        );
        // At 45 degrees: the X and Y rings are edge-on lines through the center.
        let diagonal = ROTATE_RADIUS_PX * std::f32::consts::FRAC_1_SQRT_2;
        assert_eq!(
            hit(TransformGizmoMode::Rotate, px(diagonal, diagonal)),
            Some(TransformGizmoHandle::RotateAxis(TransformGizmoAxis::Z))
        );
        assert_eq!(
            hit(TransformGizmoMode::Rotate, px(SCREEN_RING_RADIUS_PX, 0.0)),
            Some(TransformGizmoHandle::RotateScreen)
        );
    }

    #[test]
    fn gizmo_keeps_its_pixel_size_with_distance() {
        let near = gizmo_world_size(camera(), Vec3::ZERO);
        let far = gizmo_world_size(camera(), Vec3::new(0.0, 0.0, -10.0));
        assert!((far / near - 2.0).abs() < 1e-3);
    }
}
