use glam::{Mat4, Vec3};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CameraProjection {
    Perspective {
        fov_y_radians: f32,
        near: f32,
        far: f32,
    },
    Orthographic {
        left: f32,
        right: f32,
        bottom: f32,
        top: f32,
        near: f32,
        far: f32,
    },
}

impl CameraProjection {
    pub const fn perspective(fov_y_radians: f32, near: f32, far: f32) -> Self {
        Self::Perspective {
            fov_y_radians,
            near,
            far,
        }
    }

    pub const fn orthographic(
        left: f32,
        right: f32,
        bottom: f32,
        top: f32,
        near: f32,
        far: f32,
    ) -> Self {
        Self::Orthographic {
            left,
            right,
            bottom,
            top,
            near,
            far,
        }
    }

    pub fn near(self) -> f32 {
        match self {
            CameraProjection::Perspective { near, .. }
            | CameraProjection::Orthographic { near, .. } => near,
        }
    }

    pub fn far(self) -> f32 {
        match self {
            CameraProjection::Perspective { far, .. }
            | CameraProjection::Orthographic { far, .. } => far,
        }
    }
}

impl Default for CameraProjection {
    fn default() -> Self {
        Self::perspective(60f32.to_radians(), 0.1, 100.0)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub eye: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    projection: CameraProjection,
}

impl Camera {
    pub fn view(&self) -> Mat4 {
        Mat4::look_at_rh(self.eye, self.target, self.up)
    }

    pub fn proj(&self, aspect: f32) -> Mat4 {
        match self.projection {
            CameraProjection::Perspective {
                fov_y_radians,
                near,
                far,
            } => Mat4::perspective_rh(fov_y_radians, aspect, near, far),
            CameraProjection::Orthographic {
                left,
                right,
                bottom,
                top,
                near,
                far,
            } => Mat4::orthographic_rh(left, right, bottom, top, near, far),
        }
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        self.proj(aspect) * self.view()
    }

    pub fn position(&self) -> Vec3 {
        self.eye
    }

    pub fn projection(&self) -> CameraProjection {
        self.projection
    }

    pub fn set_projection(&mut self, projection: CameraProjection) {
        self.projection = projection;
    }

    pub fn fov_y_radians(&self) -> f32 {
        match self.projection {
            CameraProjection::Perspective { fov_y_radians, .. } => fov_y_radians,
            CameraProjection::Orthographic { .. } => 1e-4,
        }
    }

    pub fn near(&self) -> f32 {
        self.projection.near()
    }

    pub fn far(&self) -> f32 {
        self.projection.far()
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            eye: Vec3::new(0.0, 0.0, 3.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            projection: CameraProjection::default(),
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
