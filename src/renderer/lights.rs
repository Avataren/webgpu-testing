use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

pub const MAX_DIRECTIONAL_LIGHTS: usize = 4;
pub const MAX_POINT_LIGHTS: usize = 4;
pub const MAX_SPOT_LIGHTS: usize = 4;

#[derive(Clone, Default)]
pub struct LightsData {
    directional: Vec<DirectionalLightRaw>,
    point: Vec<PointLightRaw>,
    spot: Vec<SpotLightRaw>,
    directional_shadows: Vec<DirectionalShadowRaw>,
    point_shadows: Vec<PointShadowRaw>,
    spot_shadows: Vec<SpotShadowRaw>,
}

impl LightsData {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.directional.clear();
        self.point.clear();
        self.spot.clear();
        self.directional_shadows.clear();
        self.point_shadows.clear();
        self.spot_shadows.clear();
    }

    pub fn add_directional(
        &mut self,
        direction: Vec3,
        color: Vec3,
        intensity: f32,
        shadow: Option<DirectionalShadowData>,
    ) {
        self.directional
            .push(DirectionalLightRaw::new(direction, color, intensity));
        self.directional_shadows
            .push(DirectionalShadowRaw::from_data(shadow));
    }

    pub fn add_point(
        &mut self,
        position: Vec3,
        color: Vec3,
        intensity: f32,
        range: f32,
        shadow: Option<PointShadowData>,
    ) {
        self.point
            .push(PointLightRaw::new(position, color, intensity, range));
        self.point_shadows.push(PointShadowRaw::from_data(shadow));
    }

    pub fn add_spot(
        &mut self,
        position: Vec3,
        direction: Vec3,
        color: Vec3,
        intensity: f32,
        range: f32,
        inner_angle: f32,
        outer_angle: f32,
        shadow: Option<SpotShadowData>,
    ) {
        self.spot.push(SpotLightRaw::new(
            position,
            direction,
            color,
            intensity,
            range,
            inner_angle,
            outer_angle,
        ));
        self.spot_shadows.push(SpotShadowRaw::from_data(shadow));
    }

    pub fn directional_lights(&self) -> &[DirectionalLightRaw] {
        &self.directional
    }

    pub fn point_lights(&self) -> &[PointLightRaw] {
        &self.point
    }

    pub fn spot_lights(&self) -> &[SpotLightRaw] {
        &self.spot
    }

    pub fn directional_shadows(&self) -> &[DirectionalShadowRaw] {
        &self.directional_shadows
    }

    pub fn point_shadows(&self) -> &[PointShadowRaw] {
        &self.point_shadows
    }

    pub fn spot_shadows(&self) -> &[SpotShadowRaw] {
        &self.spot_shadows
    }
}

// All raw light/shadow structs are uploaded directly to GPU buffers.  WebGPU
// follows WGSL's std140/std430 layout rules which require 16 byte alignment for
// anything containing vectors/matrices.  The default C representation would only
// guarantee 4 byte alignment for arrays of `f32`, which in turn caused
// subsequent array elements to become misaligned once more than one entry was
// present.  Explicitly enforcing 16 byte alignment on these structs keeps the
// CPU layout in lock-step with the shader expectations.

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct DirectionalLightRaw {
    pub direction: [f32; 4],
    pub color_intensity: [f32; 4],
}

impl DirectionalLightRaw {
    pub fn new(direction: Vec3, color: Vec3, intensity: f32) -> Self {
        Self {
            direction: [direction.x, direction.y, direction.z, 0.0],
            color_intensity: [color.x, color.y, color.z, intensity],
        }
    }
}

#[derive(Clone, Copy)]
pub struct DirectionalShadowData {
    pub view_proj: Mat4,
    pub bias: f32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct DirectionalShadowRaw {
    pub view_proj: [[f32; 4]; 4],
    pub params: [f32; 4],
    pub _padding: [f32; 4],
}

impl DirectionalShadowRaw {
    fn disabled() -> Self {
        Self {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            params: [0.0, 0.0, 0.0, 0.0],
            _padding: [0.0; 4],
        }
    }

    fn from_data(data: Option<DirectionalShadowData>) -> Self {
        if let Some(data) = data {
            Self {
                view_proj: data.view_proj.to_cols_array_2d(),
                params: [1.0, data.bias, 0.0, 0.0],
                _padding: [0.0; 4],
            }
        } else {
            Self::disabled()
        }
    }
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct PointLightRaw {
    pub position_range: [f32; 4],
    pub color_intensity: [f32; 4],
}

impl PointLightRaw {
    pub fn new(position: Vec3, color: Vec3, intensity: f32, range: f32) -> Self {
        Self {
            position_range: [position.x, position.y, position.z, range],
            color_intensity: [color.x, color.y, color.z, intensity],
        }
    }
}

#[derive(Clone, Copy)]
pub struct PointShadowData {
    pub view_proj: [Mat4; 6],
    pub bias: f32,
    pub near: f32,
    pub far: f32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct PointShadowRaw {
    pub view_proj: [[[f32; 4]; 4]; 6],
    pub params: [f32; 4],
}

impl PointShadowRaw {
    fn disabled() -> Self {
        Self {
            view_proj: [Mat4::IDENTITY.to_cols_array_2d(); 6],
            params: [0.0, 0.0, 0.0, 0.0],
        }
    }

    fn from_data(data: Option<PointShadowData>) -> Self {
        if let Some(data) = data {
            Self {
                view_proj: data.view_proj.map(|mat| mat.to_cols_array_2d()),
                params: [1.0, data.bias, data.near, data.far],
            }
        } else {
            Self::disabled()
        }
    }
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SpotLightRaw {
    pub position_range: [f32; 4],
    pub direction: [f32; 4],
    pub color_intensity: [f32; 4],
    pub cone_params: [f32; 4],
}

impl SpotLightRaw {
    pub fn new(
        position: Vec3,
        direction: Vec3,
        color: Vec3,
        intensity: f32,
        range: f32,
        inner_angle: f32,
        outer_angle: f32,
    ) -> Self {
        let (mut inner, mut outer) = (inner_angle, outer_angle);
        if inner > outer {
            std::mem::swap(&mut inner, &mut outer);
        }
        let cos_inner = inner.cos();
        let cos_outer = outer.cos();

        Self {
            position_range: [position.x, position.y, position.z, range],
            direction: [direction.x, direction.y, direction.z, 0.0],
            color_intensity: [color.x, color.y, color.z, intensity],
            cone_params: [cos_inner, cos_outer, 0.0, 0.0],
        }
    }
}

#[derive(Clone, Copy)]
pub struct SpotShadowData {
    pub view_proj: Mat4,
    pub bias: f32,
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct SpotShadowRaw {
    pub view_proj: [[f32; 4]; 4],
    pub params: [f32; 4],
}

impl SpotShadowRaw {
    fn disabled() -> Self {
        Self {
            view_proj: Mat4::IDENTITY.to_cols_array_2d(),
            params: [0.0, 0.0, 0.0, 0.0],
        }
    }

    fn from_data(data: Option<SpotShadowData>) -> Self {
        if let Some(data) = data {
            Self {
                view_proj: data.view_proj.to_cols_array_2d(),
                params: [1.0, data.bias, 0.0, 0.0],
            }
        } else {
            Self::disabled()
        }
    }
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct LightsUniform {
    pub counts: [u32; 4],
    pub directionals: [DirectionalLightRaw; MAX_DIRECTIONAL_LIGHTS],
    pub points: [PointLightRaw; MAX_POINT_LIGHTS],
    pub spots: [SpotLightRaw; MAX_SPOT_LIGHTS],
}

impl LightsUniform {
    pub fn from_data(data: &LightsData) -> Self {
        let mut uniform = Self::zeroed();

        let dir_count = data.directional_lights().len().min(MAX_DIRECTIONAL_LIGHTS) as u32;
        uniform.counts[0] = dir_count;
        for (dst, src) in uniform
            .directionals
            .iter_mut()
            .zip(data.directional_lights().iter())
            .take(dir_count as usize)
        {
            *dst = *src;
        }

        let point_count = data.point_lights().len().min(MAX_POINT_LIGHTS) as u32;
        uniform.counts[1] = point_count;
        for (dst, src) in uniform
            .points
            .iter_mut()
            .zip(data.point_lights().iter())
            .take(point_count as usize)
        {
            *dst = *src;
        }

        let spot_count = data.spot_lights().len().min(MAX_SPOT_LIGHTS) as u32;
        uniform.counts[2] = spot_count;
        for (dst, src) in uniform
            .spots
            .iter_mut()
            .zip(data.spot_lights().iter())
            .take(spot_count as usize)
        {
            *dst = *src;
        }

        uniform
    }
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct ShadowsUniform {
    pub counts: [u32; 4],
    pub directionals: [DirectionalShadowRaw; MAX_DIRECTIONAL_LIGHTS],
    pub points: [PointShadowRaw; MAX_POINT_LIGHTS],
    pub spots: [SpotShadowRaw; MAX_SPOT_LIGHTS],
}

impl ShadowsUniform {
    pub fn from_data(data: &LightsData) -> Self {
        let mut uniform = Self::zeroed();

        let dir_count = data.directional_shadows().len().min(MAX_DIRECTIONAL_LIGHTS) as u32;
        uniform.counts[0] = dir_count;
        for (dst, src) in uniform
            .directionals
            .iter_mut()
            .zip(data.directional_shadows().iter())
            .take(dir_count as usize)
        {
            *dst = *src;
        }

        // DEBUG: Log what we're putting in the uniform
        // if dir_count > 0 {
        //     log::info!("ShadowsUniform created - first dir shadow:");
        //     log::info!("  view_proj[0]: {:?}", uniform.directionals[0].view_proj[0]);
        //     log::info!("  params: {:?}", uniform.directionals[0].params);
        // }

        let point_count = data.point_shadows().len().min(MAX_POINT_LIGHTS) as u32;
        uniform.counts[1] = point_count;
        for (dst, src) in uniform
            .points
            .iter_mut()
            .zip(data.point_shadows().iter())
            .take(point_count as usize)
        {
            *dst = *src;
        }

        let spot_count = data.spot_shadows().len().min(MAX_SPOT_LIGHTS) as u32;
        uniform.counts[2] = spot_count;
        for (dst, src) in uniform
            .spots
            .iter_mut()
            .zip(data.spot_shadows().iter())
            .take(spot_count as usize)
        {
            *dst = *src;
        }

        uniform
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spot_light_uniform_includes_shadow_data() {
        let mut data = LightsData::new();
        let position = Vec3::new(1.0, 2.0, 3.0);
        let direction = Vec3::new(-0.2, -0.9, 0.3).normalize();
        let color = Vec3::new(0.8, 0.7, 0.6);
        let intensity = 4.0;
        let range = 10.0;
        let inner = 0.3;
        let outer = 0.6;

        let view = Mat4::look_at_rh(position, position + direction, Vec3::Y);
        let proj = Mat4::perspective_rh(outer * 2.0, 1.0, 0.1, range);
        let shadow = SpotShadowData {
            view_proj: proj * view,
            bias: 0.005,
        };

        data.add_spot(
            position,
            direction,
            color,
            intensity,
            range,
            inner,
            outer,
            Some(shadow),
        );

        let lights = LightsUniform::from_data(&data);
        assert_eq!(lights.counts[2], 1);
        let stored = lights.spots[0];
        assert_eq!(stored.position_range, [1.0, 2.0, 3.0, range]);
        assert_eq!(stored.color_intensity, [0.8, 0.7, 0.6, intensity]);
        let stored_dir = Vec3::new(
            stored.direction[0],
            stored.direction[1],
            stored.direction[2],
        );
        assert!(stored_dir.abs_diff_eq(direction, 1e-6));

        let shadows = ShadowsUniform::from_data(&data);
        assert_eq!(shadows.counts[2], 1);
        assert_eq!(shadows.spots[0].params[0], 1.0);
        assert_eq!(shadows.spots[0].params[1], 0.005);
        let stored_view = Mat4::from_cols_array_2d(&shadows.spots[0].view_proj);
        assert!(stored_view.abs_diff_eq(proj * view, 1e-6));
    }

    #[test]
    fn gpu_structs_are_16_byte_aligned() {
        use std::mem::{align_of, size_of};

        assert_eq!(align_of::<DirectionalLightRaw>(), 16);
        assert_eq!(align_of::<PointLightRaw>(), 16);
        assert_eq!(align_of::<SpotLightRaw>(), 16);
        assert_eq!(align_of::<DirectionalShadowRaw>(), 16);
        assert_eq!(align_of::<PointShadowRaw>(), 16);
        assert_eq!(align_of::<SpotShadowRaw>(), 16);
        assert_eq!(align_of::<LightsUniform>(), 16);
        assert_eq!(align_of::<ShadowsUniform>(), 16);

        // Sanity check that the size of each struct remains a multiple of the
        // required alignment so arrays keep matching WGSL's expected stride.
        assert_eq!(size_of::<DirectionalShadowRaw>() % 16, 0);
        assert_eq!(size_of::<PointShadowRaw>() % 16, 0);
        assert_eq!(size_of::<SpotShadowRaw>() % 16, 0);
        assert_eq!(size_of::<LightsUniform>() % 16, 0);
        assert_eq!(size_of::<ShadowsUniform>() % 16, 0);
    }
}
