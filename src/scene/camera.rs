use glam::{Mat4, Vec3};

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fov_y_radians: f32,
    pub near: f32,
    pub far: f32,
}

impl Camera {
    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye, self.target, self.up)
    }
    pub fn proj(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(self.fov_y_radians, aspect, self.near, self.far)
    }
    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }
    pub fn position(&self) -> Vec3 {
        self.eye
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            eye: Vec3::new(0.0, 0.0, 3.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            fov_y_radians: 60f32.to_radians(),
            near: 0.1,
            far: 100.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn view_proj_is_reasonable() {
        let cam = Camera::default();
        let vp = cam.view_proj(16.0 / 9.0);
        // Just ensure it's invertible and finite
        let inv = vp.inverse();
        let id = vp * inv;
        let eps = 1e-4;
        assert!(id.abs_diff_eq(Mat4::IDENTITY, eps));
    }
}
