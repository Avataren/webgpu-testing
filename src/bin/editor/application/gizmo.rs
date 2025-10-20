use glam::{Quat, Vec2, Vec3};
use log::warn;
use wgpu_cube::app::{RuntimeMode, UpdateContext};
use wgpu_cube::scene::{
    Parent, Transform, TransformComponent, TransformGizmoAxis, TransformGizmoHandle,
    TransformGizmoSpace, WorldTransform,
};

use super::core::{EditorApplication, GizmoDragKind, GizmoDragState};

impl EditorApplication {
    pub(super) fn update_gizmo_drag(&mut self, ctx: &mut UpdateContext) {
        if !matches!(ctx.runtime, RuntimeMode::Editor) {
            self.gizmo_drag = None;
            self.pointer.press_uv = None;
            return;
        }

        if let Some(uv) = self.pointer.press_uv.take() {
            if self.try_begin_gizmo_drag(ctx, uv) {
                self.pointer.selection_press_uv = None;
            }
        }

        let mut transforms_dirty = false;
        let mut end_drag = false;

        if let Some(drag) = self.gizmo_drag.as_mut() {
            if !self.pointer.primary_down {
                end_drag = true;
            } else if let Some(region) = self.scene_viewport.region() {
                let width = region.width().max(1) as f32;
                let height = region.height().max(1) as f32;
                if width > 0.0 && height > 0.0 {
                    let aspect = width / height;
                    let camera = ctx.scene.camera();
                    let uv = self
                        .pointer
                        .scene_uv
                        .filter(|uv| uv.is_finite())
                        .unwrap_or(drag.last_pointer_uv);
                    let (origin, direction) = Self::ray_from_uv(camera, uv, aspect);
                    match Self::apply_gizmo_drag(ctx, drag, origin, direction) {
                        Ok(updated) => {
                            if updated {
                                drag.last_pointer_uv = uv;
                                drag.any_change = true;
                                transforms_dirty = true;
                            }
                        }
                        Err(_) => {
                            end_drag = true;
                        }
                    }
                }
            }
        }

        if transforms_dirty {
            ctx.scene.propagate_transforms();
        }

        let mut record_history = false;
        if end_drag {
            if let Some(drag) = self.gizmo_drag.take() {
                record_history = drag.any_change;
            }
        }

        if record_history {
            self.record_scene_change(ctx.scene);
        }
    }

    pub(super) fn try_begin_gizmo_drag(&mut self, ctx: &mut UpdateContext, press_uv: Vec2) -> bool {
        let Some(entity) = self.selection.selected else {
            return false;
        };

        let Some(region) = self.scene_viewport.region() else {
            return false;
        };
        let width = region.width().max(1) as f32;
        let height = region.height().max(1) as f32;
        if width <= 0.0 || height <= 0.0 {
            return false;
        }
        let aspect = width / height;

        let camera = *ctx.scene.camera();
        let (origin, direction) = Self::ray_from_uv(&camera, press_uv, aspect);
        let Some(handle) = ctx.scene.transform_gizmo_hit(origin, direction) else {
            return false;
        };

        {
            let world = ctx.scene.main_world();
            if world.entity(entity).is_err() {
                return false;
            }
        }

        ctx.scene.propagate_transforms();

        let (initial_local, parent_world, initial_world_opt) = {
            let world = ctx.scene.main_world();
            let local = world
                .get::<&TransformComponent>(entity)
                .map(|c| c.0)
                .unwrap_or(Transform::IDENTITY);
            let parent_world = world
                .get::<&Parent>(entity)
                .ok()
                .and_then(|parent| world.get::<&WorldTransform>(parent.0).ok())
                .map(|wt| wt.0)
                .unwrap_or(Transform::IDENTITY);
            let world_transform = world.get::<&WorldTransform>(entity).ok().map(|wt| wt.0);
            (local, parent_world, world_transform)
        };

        let initial_world =
            initial_world_opt.unwrap_or_else(|| parent_world.mul_transform(&initial_local));

        let mut camera_forward = camera.target - camera.eye;
        camera_forward = Self::safe_normalize(camera_forward, Vec3::NEG_Z);
        let mut camera_up = camera.up;
        camera_up = Self::safe_normalize(camera_up, Vec3::Y);

        let origin_point = initial_world.translation;
        let gizmo_rotation = match self.transform_gizmo_space {
            TransformGizmoSpace::Local => initial_world.rotation,
            TransformGizmoSpace::World => Quat::IDENTITY,
        };
        let kind = match handle {
            TransformGizmoHandle::TranslateAxis(axis) => {
                let axis_dir = Self::axis_direction(gizmo_rotation, axis);
                let mut start_param =
                    Self::ray_axis_parameter(origin, direction, origin_point, axis_dir)
                        .unwrap_or(0.0);
                if start_param.abs() < 1e-4 {
                    let Some(plane_normal) =
                        Self::translation_plane_normal(axis_dir, camera_forward, camera_up)
                    else {
                        return false;
                    };
                    let Some(point) =
                        Self::ray_plane_intersection(origin, direction, origin_point, plane_normal)
                    else {
                        return false;
                    };
                    start_param = (point - origin_point).dot(axis_dir);
                }
                if start_param.abs() < 1e-4 {
                    start_param = 1.0;
                }
                GizmoDragKind::TranslateAxis {
                    axis_dir,
                    origin: origin_point,
                    start_param,
                }
            }
            TransformGizmoHandle::TranslatePlane(axis_a, axis_b) => {
                let axis_a_dir = Self::axis_direction(gizmo_rotation, axis_a);
                let axis_b_dir = Self::axis_direction(gizmo_rotation, axis_b);
                let mut plane_normal = axis_a_dir.cross(axis_b_dir);
                if plane_normal.length_squared() < 1e-6 {
                    return false;
                }
                plane_normal = plane_normal.normalize();
                let Some(point) =
                    Self::ray_plane_intersection(origin, direction, origin_point, plane_normal)
                else {
                    return false;
                };
                GizmoDragKind::TranslatePlane {
                    plane_normal,
                    origin: origin_point,
                    start_point: point,
                }
            }
            TransformGizmoHandle::TranslateCenter => {
                let plane_normal = camera_forward;
                let Some(point) =
                    Self::ray_plane_intersection(origin, direction, origin_point, plane_normal)
                else {
                    return false;
                };
                GizmoDragKind::TranslatePlane {
                    plane_normal,
                    origin: origin_point,
                    start_point: point,
                }
            }
            TransformGizmoHandle::RotateAxis(axis) => {
                let axis_dir = Self::axis_direction(gizmo_rotation, axis);
                let Some(point) =
                    Self::ray_plane_intersection(origin, direction, origin_point, axis_dir)
                else {
                    return false;
                };
                let start_vector = point - origin_point;
                if start_vector.length_squared() < 1e-6 {
                    return false;
                }
                GizmoDragKind::Rotate {
                    axis_dir,
                    origin: origin_point,
                    start_vector,
                }
            }
            TransformGizmoHandle::RotateScreen => {
                let axis_dir = -camera_forward;
                let Some(point) =
                    Self::ray_plane_intersection(origin, direction, origin_point, axis_dir)
                else {
                    return false;
                };
                let start_vector = point - origin_point;
                if start_vector.length_squared() < 1e-6 {
                    return false;
                }
                GizmoDragKind::Rotate {
                    axis_dir,
                    origin: origin_point,
                    start_vector,
                }
            }
            TransformGizmoHandle::ScaleAxis(axis) => {
                let axis_dir = Self::axis_direction(gizmo_rotation, axis);
                let mut start_param =
                    Self::ray_axis_parameter(origin, direction, origin_point, axis_dir)
                        .unwrap_or(0.0);

                if start_param.abs() < 1e-4 {
                    let plane_normal =
                        match Self::translation_plane_normal(axis_dir, camera_forward, camera_up) {
                            Some(normal) => normal,
                            None => return false,
                        };
                    let Some(point) =
                        Self::ray_plane_intersection(origin, direction, origin_point, plane_normal)
                    else {
                        return false;
                    };
                    start_param = (point - origin_point).dot(axis_dir);
                    if start_param.abs() < 1e-4 {
                        start_param = 1.0;
                    }
                }
                if start_param.abs() < 1e-4 {
                    start_param = 1.0;
                }

                GizmoDragKind::ScaleAxis {
                    axis,
                    axis_dir,
                    origin: origin_point,
                    start_param,
                    initial_scale: initial_world.scale,
                }
            }
            TransformGizmoHandle::ScaleUniform => {
                let plane_normal = camera_forward;
                let right = {
                    let r = camera_forward.cross(camera_up);
                    if r.length_squared() < 1e-6 {
                        Vec3::X
                    } else {
                        r.normalize()
                    }
                };
                let up = {
                    let u = right.cross(camera_forward);
                    if u.length_squared() < 1e-6 {
                        camera_up
                    } else {
                        u.normalize()
                    }
                };
                let base_scale = Self::gizmo_screen_scale(&camera, origin_point);
                GizmoDragKind::ScaleUniform {
                    plane_normal,
                    origin: origin_point,
                    right,
                    up,
                    base_scale,
                    initial_scale: initial_world.scale,
                }
            }
        };

        self.gizmo_drag = Some(GizmoDragState {
            entity,
            handle,
            parent_world,
            initial_world,
            last_pointer_uv: press_uv,
            any_change: false,
            kind,
        });

        true
    }

    pub(super) fn apply_gizmo_drag(
        ctx: &mut UpdateContext,
        drag: &mut GizmoDragState,
        ray_origin: Vec3,
        ray_dir: Vec3,
    ) -> Result<bool, ()> {
        let mut new_world = drag.initial_world;
        let mut updated = false;

        match &mut drag.kind {
            GizmoDragKind::TranslateAxis {
                axis_dir,
                origin,
                start_param,
            } => {
                let Some(param) = Self::ray_axis_parameter(ray_origin, ray_dir, *origin, *axis_dir)
                else {
                    return Ok(false);
                };
                let delta = param - *start_param;
                if delta.is_finite() {
                    new_world.translation = drag.initial_world.translation + *axis_dir * delta;
                    updated = true;
                }
            }
            GizmoDragKind::TranslatePlane {
                plane_normal,
                origin,
                start_point,
            } => {
                let Some(point) =
                    Self::ray_plane_intersection(ray_origin, ray_dir, *origin, *plane_normal)
                else {
                    return Ok(false);
                };
                let delta = point - *start_point;
                if delta.is_finite() {
                    new_world.translation = drag.initial_world.translation + delta;
                    updated = true;
                }
            }
            GizmoDragKind::Rotate {
                axis_dir,
                origin,
                start_vector,
            } => {
                let Some(point) =
                    Self::ray_plane_intersection(ray_origin, ray_dir, *origin, *axis_dir)
                else {
                    return Ok(false);
                };
                let current_vector = point - *origin;
                let Some(angle) = Self::signed_angle(*start_vector, current_vector, *axis_dir)
                else {
                    return Ok(false);
                };
                let rotation = Quat::from_axis_angle(*axis_dir, angle);
                new_world.rotation = rotation * drag.initial_world.rotation;
                updated = true;
            }
            GizmoDragKind::ScaleAxis {
                axis,
                axis_dir,
                origin,
                start_param,
                initial_scale,
            } => {
                let Some(param) = Self::ray_axis_parameter(ray_origin, ray_dir, *origin, *axis_dir)
                else {
                    return Ok(false);
                };
                let mut ratio = if start_param.abs() < 1e-4 {
                    1.0 + (param - *start_param)
                } else {
                    param / *start_param
                };
                if !ratio.is_finite() {
                    return Ok(false);
                }
                if ratio.abs() < 0.01 {
                    ratio = 0.01 * ratio.signum();
                    if !ratio.is_finite() || ratio == 0.0 {
                        ratio = 0.01;
                    }
                }

                let mut scale = *initial_scale;
                match axis {
                    TransformGizmoAxis::X => scale.x = initial_scale.x * ratio,
                    TransformGizmoAxis::Y => scale.y = initial_scale.y * ratio,
                    TransformGizmoAxis::Z => scale.z = initial_scale.z * ratio,
                }

                new_world.scale = scale;
                updated = true;
            }
            GizmoDragKind::ScaleUniform {
                plane_normal,
                origin,
                right,
                up,
                base_scale,
                initial_scale,
            } => {
                let Some(point) =
                    Self::ray_plane_intersection(ray_origin, ray_dir, *origin, *plane_normal)
                else {
                    return Ok(false);
                };
                let delta = point - *origin;
                let right_amt = delta.dot(*right);
                let up_amt = delta.dot(*up);
                let dominant = if right_amt.abs() >= up_amt.abs() {
                    right_amt
                } else {
                    up_amt
                };
                let scale_ref = base_scale.max(1e-3);
                let mut ratio = 1.0 + dominant / scale_ref;
                if !ratio.is_finite() {
                    return Ok(false);
                }
                if ratio.abs() < 0.01 {
                    ratio = 0.01 * ratio.signum();
                    if !ratio.is_finite() || ratio == 0.0 {
                        ratio = 0.01;
                    }
                }
                new_world.scale = *initial_scale * ratio;
                updated = true;
            }
        }

        if !updated {
            return Ok(false);
        }

        let new_local = Self::world_to_local(drag.parent_world, new_world);

        {
            let world = ctx.scene.main_world_mut();
            let updated_existing = {
                if let Ok(mut transform) = world.get::<&mut TransformComponent>(drag.entity) {
                    transform.0 = new_local;
                    true
                } else {
                    false
                }
            };

            if !updated_existing {
                if let Err(err) = world.insert_one(drag.entity, TransformComponent(new_local)) {
                    warn!(
                        "failed to insert TransformComponent for {:?}: {err}",
                        drag.entity
                    );
                    return Err(());
                }
            }
        }

        Ok(true)
    }

    pub(super) fn basis_from_up_forward(up_hint: Vec3, forward: Vec3) -> (Vec3, Vec3) {
        let mut right = up_hint.cross(forward);
        if right.length_squared() < 1e-6 {
            right = Vec3::X;
        } else {
            right = right.normalize();
        }

        let mut up = forward.cross(right);
        if up.length_squared() < 1e-6 {
            up = Vec3::Y;
        } else {
            up = up.normalize();
        }

        (right, up)
    }

    pub(super) fn safe_normalize(vec: Vec3, fallback: Vec3) -> Vec3 {
        if vec.length_squared() < 1e-6 {
            fallback
        } else {
            vec.normalize()
        }
    }

    pub(super) fn axis_basis(axis: TransformGizmoAxis) -> Vec3 {
        match axis {
            TransformGizmoAxis::X => Vec3::X,
            TransformGizmoAxis::Y => Vec3::Y,
            TransformGizmoAxis::Z => Vec3::Z,
        }
    }

    pub(super) fn axis_direction(rotation: Quat, axis: TransformGizmoAxis) -> Vec3 {
        let dir = rotation * Self::axis_basis(axis);
        if dir.length_squared() < 1e-6 {
            Self::axis_basis(axis)
        } else {
            dir.normalize()
        }
    }

    pub(super) fn translation_plane_normal(
        axis_dir: Vec3,
        view_dir: Vec3,
        view_up: Vec3,
    ) -> Option<Vec3> {
        let mut normal = axis_dir * axis_dir.dot(view_dir) - view_dir;
        if normal.length_squared() < 1e-6 {
            normal = axis_dir.cross(view_up);
        }
        if normal.length_squared() < 1e-6 {
            let fallback = if axis_dir.x.abs() < 0.9 {
                Vec3::X
            } else {
                Vec3::Y
            };
            normal = axis_dir.cross(fallback);
        }
        if normal.length_squared() < 1e-6 {
            normal = axis_dir.cross(Vec3::Z);
        }
        if normal.length_squared() < 1e-6 {
            return None;
        }
        Some(normal.normalize())
    }

    pub(super) fn ray_plane_intersection(
        ray_origin: Vec3,
        ray_dir: Vec3,
        plane_origin: Vec3,
        plane_normal: Vec3,
    ) -> Option<Vec3> {
        let denom = ray_dir.dot(plane_normal);
        if denom.abs() < 1e-6 {
            return None;
        }
        let t = (plane_origin - ray_origin).dot(plane_normal) / denom;
        if !t.is_finite() || t < 0.0 {
            return None;
        }
        Some(ray_origin + ray_dir * t)
    }

    pub(super) fn ray_axis_parameter(
        ray_origin: Vec3,
        ray_dir: Vec3,
        axis_origin: Vec3,
        axis_dir: Vec3,
    ) -> Option<f32> {
        if axis_dir.length_squared() < 1e-6 {
            return None;
        }
        let axis_dir = axis_dir.normalize();
        let a = ray_dir.dot(ray_dir);
        if a < 1e-6 {
            return None;
        }
        let b = ray_dir.dot(axis_dir);
        let w0 = ray_origin - axis_origin;
        let d = ray_dir.dot(w0);
        let e = axis_dir.dot(w0);
        let denom = a - b * b;
        let t = if denom.abs() > 1e-6 {
            (a * e - b * d) / denom
        } else {
            e
        };
        t.is_finite().then_some(t)
    }

    pub(super) fn gizmo_screen_scale(camera: &wgpu_cube::scene::Camera, position: Vec3) -> f32 {
        let distance = (camera.eye - position).length().max(0.1);
        let half_fov = (camera.fov_y_radians * 0.5).max(1e-4);
        if half_fov <= 1e-3 {
            1.0
        } else {
            2.0 * distance * half_fov.tan() * 0.16
        }
    }

    pub(super) fn signed_angle(start: Vec3, current: Vec3, axis: Vec3) -> Option<f32> {
        if start.length_squared() < 1e-6
            || current.length_squared() < 1e-6
            || axis.length_squared() < 1e-6
        {
            return None;
        }

        let start_norm = start.normalize();
        let current_norm = current.normalize();
        let axis_norm = axis.normalize();

        let cross = start_norm.cross(current_norm);
        let sin = cross.dot(axis_norm);
        let cos = start_norm.dot(current_norm).clamp(-1.0, 1.0);
        Some(sin.atan2(cos))
    }

    pub(super) fn world_to_local(parent_world: Transform, world: Transform) -> Transform {
        let parent_matrix = parent_world.matrix();
        let parent_inverse = parent_matrix.inverse();
        let world_matrix = world.matrix();
        let local_matrix = parent_inverse * world_matrix;
        let (scale, rotation, translation) = local_matrix.to_scale_rotation_translation();
        Transform::from_trs(translation, rotation, scale)
    }
}
