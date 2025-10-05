use bytemuck::{Pod, Zeroable};
use glam::Vec3;

pub const MAX_DIRECTIONAL_LIGHTS: usize = 4;
pub const MAX_POINT_LIGHTS: usize = 16;
pub const MAX_SPOT_LIGHTS: usize = 8;

#[derive(Clone, Default)]
pub struct LightsData {
    directional: Vec<DirectionalLightRaw>,
    point: Vec<PointLightRaw>,
    spot: Vec<SpotLightRaw>,
}

impl LightsData {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.directional.clear();
        self.point.clear();
        self.spot.clear();
    }

    pub fn add_directional(&mut self, direction: Vec3, color: Vec3, intensity: f32) {
        self.directional
            .push(DirectionalLightRaw::new(direction, color, intensity));
    }

    pub fn add_point(&mut self, position: Vec3, color: Vec3, intensity: f32, range: f32) {
        self.point
            .push(PointLightRaw::new(position, color, intensity, range));
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
}

#[repr(C)]
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

#[repr(C)]
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

#[repr(C)]
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

#[repr(C)]
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
