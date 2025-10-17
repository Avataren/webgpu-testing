use glam::{Vec2, Vec3};

use wgpu_cube::app::UpdateContext;

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
        let camera = ctx.scene.camera_mut();

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
            self.apply_look_delta(camera, &mut forward, &mut up);
        }

        let dt = ctx.dt as f32;
        if !self.looking || dt <= 0.0 {
            return;
        }

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

        if movement.length_squared() < 1e-6 {
            return;
        }

        movement = movement.normalize();
        let mut speed = self.base_speed * dt;
        if self.boost {
            speed *= self.boost_multiplier;
        }
        let delta = movement * speed;
        camera.eye += delta;
        camera.target += delta;
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

    fn apply_look_delta(
        &mut self,
        camera: &mut wgpu_cube::scene::Camera,
        forward: &mut Vec3,
        up: &mut Vec3,
    ) {
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
}
