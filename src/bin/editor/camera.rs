use glam::{Mat3, Quat, Vec2, Vec3};
use hecs::Entity;

use wgpu_cube::app::UpdateContext;
use wgpu_cube::scene::{Camera, Parent, Transform, TransformComponent, WorldTransform};

/// Controls the editor camera using familiar FPS-style inputs.
pub struct EditorCameraController {
    move_forward: bool,
    move_back: bool,
    move_left: bool,
    move_right: bool,
    boost: bool,
    looking: bool,
    viewport_rect: Option<egui::Rect>,
    look_delta: Vec2,
    base_speed: f32,
    boost_multiplier: f32,
    look_sensitivity: f32,
}

impl Default for EditorCameraController {
    fn default() -> Self {
        Self {
            move_forward: false,
            move_back: false,
            move_left: false,
            move_right: false,
            boost: false,
            looking: false,
            viewport_rect: None,
            look_delta: Vec2::ZERO,
            base_speed: 5.0,
            boost_multiplier: 3.0,
            look_sensitivity: 0.0025,
        }
    }
}

impl EditorCameraController {
    pub fn set_viewport_rect(&mut self, rect: Option<egui::Rect>) {
        self.viewport_rect = rect;
        if rect.is_none() {
            self.reset_movement();
            self.look_delta = Vec2::ZERO;
        }
    }

    pub fn is_looking(&self) -> bool {
        self.looking
    }

    pub fn capture_input(&mut self, ctx: &egui::Context) {
        let viewport_rect = self.viewport_rect;
        let wants_keyboard = ctx.wants_keyboard_input();
        let was_looking = self.looking;
        self.look_delta = Vec2::ZERO;

        ctx.input(|input| {
            if wants_keyboard || viewport_rect.is_none() {
                self.looking = false;
                self.reset_movement();
                return;
            }

            let rect = viewport_rect.unwrap();

            if input.pointer.secondary_pressed() {
                let press_pos = input
                    .pointer
                    .press_origin()
                    .or_else(|| input.pointer.hover_pos());
                self.looking = press_pos.is_some_and(|pos| rect.contains(pos));
                if !self.looking {
                    self.reset_movement();
                }
            } else if !input.pointer.secondary_down() {
                self.looking = false;
                self.reset_movement();
            }

            if !self.looking {
                return;
            }

            self.move_forward = input.key_down(egui::Key::W);
            self.move_back = input.key_down(egui::Key::S);
            self.move_left = input.key_down(egui::Key::A);
            self.move_right = input.key_down(egui::Key::D);
            self.boost = input.modifiers.shift;

            let motion = input
                .pointer
                .motion()
                .unwrap_or_else(|| input.pointer.delta());
            self.look_delta = Vec2::new(motion.x, motion.y);
        });

        self.sync_cursor_capture(ctx, was_looking);
    }

    pub fn update_camera(&mut self, ctx: &mut UpdateContext) {
        let mut camera = *ctx.scene.camera();

        let mut forward = camera.target - camera.eye;
        if forward.length_squared() < f32::EPSILON {
            forward = Vec3::NEG_Z;
        }
        forward = forward.normalize();

        let mut up = camera.up;
        if up.length_squared() < f32::EPSILON {
            up = Vec3::Y;
        }
        up = up.normalize();

        if self.looking && self.look_delta.length_squared() > 0.0 {
            self.apply_look_delta(&mut camera, &mut forward, &mut up);
        }

        let dt = ctx.dt as f32;
        if self.looking && dt > 0.0 {
            let mut right = forward.cross(up);
            if right.length_squared() < 1e-6 {
                right = Vec3::X;
            }
            right = right.normalize();

            let mut movement = Vec3::ZERO;
            if self.move_forward {
                movement += forward;
            }
            if self.move_back {
                movement -= forward;
            }
            if self.move_left {
                movement -= right;
            }
            if self.move_right {
                movement += right;
            }

            if movement.length_squared() >= 1e-6 {
                movement = movement.normalize();
                let mut speed = self.base_speed * dt;
                if self.boost {
                    speed *= self.boost_multiplier;
                }
                let delta = movement * speed;
                camera.eye += delta;
                camera.target += delta;
            }
        }

        self.apply_camera_state(ctx, camera);
    }

    fn reset_movement(&mut self) {
        self.move_forward = false;
        self.move_back = false;
        self.move_left = false;
        self.move_right = false;
        self.boost = false;
    }

    fn sync_cursor_capture(&self, ctx: &egui::Context, was_looking: bool) {
        if self.looking == was_looking {
            return;
        }

        if self.looking {
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(
                egui::viewport::CursorGrab::Locked,
            ));
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorGrab(
                egui::viewport::CursorGrab::None,
            ));
            ctx.send_viewport_cmd(egui::ViewportCommand::CursorVisible(true));
        }
    }

    fn apply_look_delta(&mut self, camera: &mut Camera, forward: &mut Vec3, up: &mut Vec3) {
        let delta = self.look_delta * self.look_sensitivity;
        self.look_delta = Vec2::ZERO;

        let mut yaw = forward.x.atan2(-forward.z);
        let mut pitch = forward.y.clamp(-1.0, 1.0).asin();
        yaw += delta.x;
        let max_pitch = std::f32::consts::FRAC_PI_2 - 0.01;
        pitch = (pitch - delta.y).clamp(-max_pitch, max_pitch);

        let cos_pitch = pitch.cos();
        let sin_pitch = pitch.sin();
        let sin_yaw = yaw.sin();
        let cos_yaw = yaw.cos();
        *forward = Vec3::new(cos_pitch * sin_yaw, sin_pitch, -cos_pitch * cos_yaw);

        let mut right = forward.cross(Vec3::Y);
        if right.length_squared() < 1e-6 {
            right = Vec3::X;
        }
        right = right.normalize();
        *up = right.cross(*forward).normalize();

        camera.target = camera.eye + *forward;
        camera.up = *up;
    }

    fn apply_camera_state(&self, ctx: &mut UpdateContext, camera: Camera) {
        let active = ctx.scene.active_camera_entity();
        let mut camera_to_apply = Some(camera);

        if let Some(entity) = active {
            if self.write_transform_from_camera(ctx, entity, &camera) {
                ctx.scene.propagate_transforms();
                camera_to_apply = None;
            }
        }

        if let Some(camera) = camera_to_apply {
            ctx.scene.set_camera(camera);
        }
    }

    fn write_transform_from_camera(
        &self,
        ctx: &mut UpdateContext,
        entity: Entity,
        camera: &Camera,
    ) -> bool {
        let mut updated = false;
        {
            let world = ctx.scene.main_world_mut();

            let parent_world = world
                .get::<&Parent>(entity)
                .ok()
                .and_then(|parent| world.get::<&WorldTransform>(parent.0).ok().map(|wt| wt.0));

            if let Ok(mut transform) = world.get::<&mut TransformComponent>(entity) {
                if Self::update_local_transform(&mut transform.0, parent_world, camera) {
                    updated = true;
                }
            }

            if let Ok(mut world_transform) = world.get::<&mut WorldTransform>(entity) {
                if Self::update_world_transform(&mut world_transform.0, camera) {
                    updated = true;
                }
            }
        }

        updated
    }

    fn update_local_transform(
        transform: &mut Transform,
        parent_world: Option<Transform>,
        camera: &Camera,
    ) -> bool {
        let desired_rotation = Self::camera_rotation(camera);
        let desired_translation = camera.eye;

        let (target_translation, target_rotation) = if let Some(parent) = parent_world {
            let parent_rotation = parent.rotation;
            let parent_inverse = parent_rotation.conjugate();
            let offset = desired_translation - parent.translation;
            let rotated = parent_inverse * offset;
            let local_translation = Self::safe_divide(rotated, parent.scale);
            let local_rotation = parent_inverse * desired_rotation;
            (local_translation, local_rotation)
        } else {
            (desired_translation, desired_rotation)
        };

        let mut changed = false;
        if !transform.translation.abs_diff_eq(target_translation, 1e-5) {
            transform.translation = target_translation;
            changed = true;
        }

        if !transform.rotation.abs_diff_eq(target_rotation, 1e-5) {
            transform.rotation = target_rotation;
            changed = true;
        }

        changed
    }

    fn update_world_transform(transform: &mut Transform, camera: &Camera) -> bool {
        let desired_rotation = Self::camera_rotation(camera);
        let mut changed = false;

        if !transform.translation.abs_diff_eq(camera.eye, 1e-5) {
            transform.translation = camera.eye;
            changed = true;
        }

        if !transform.rotation.abs_diff_eq(desired_rotation, 1e-5) {
            transform.rotation = desired_rotation;
            changed = true;
        }

        changed
    }

    fn camera_rotation(camera: &Camera) -> Quat {
        let forward = (camera.target - camera.eye)
            .try_normalize()
            .unwrap_or(Vec3::NEG_Z);
        let raw_up = camera.up.try_normalize().unwrap_or(Vec3::Y);
        let right = forward.cross(raw_up).try_normalize().unwrap_or(Vec3::X);
        let up = right.cross(forward).try_normalize().unwrap_or(Vec3::Y);
        Quat::from_mat3(&Mat3::from_cols(right, up, -forward))
    }

    fn safe_divide(value: Vec3, divisor: Vec3) -> Vec3 {
        Vec3::new(
            if divisor.x.abs() > f32::EPSILON {
                value.x / divisor.x
            } else {
                value.x
            },
            if divisor.y.abs() > f32::EPSILON {
                value.y / divisor.y
            } else {
                value.y
            },
            if divisor.z.abs() > f32::EPSILON {
                value.z / divisor.z
            } else {
                value.z
            },
        )
    }
}
