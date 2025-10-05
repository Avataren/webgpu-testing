use bytemuck::{Pod, Zeroable};
use glam::Vec3;

pub const MAX_DIRECTIONAL_LIGHTS: usize = 4;
pub const MAX_POINT_LIGHTS: usize = 16;
pub const MAX_SPOT_LIGHTS: usize = 8;

#[derive(Clone, Copy, Debug)]
pub struct DirectionalLightData {
    pub direction: Vec3,
    pub color: Vec3,
    pub intensity: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct PointLightData {
    pub position: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    pub range: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct SpotLightData {
    pub position: Vec3,
    pub direction: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    pub range: f32,
    pub inner_angle: f32,
    pub outer_angle: f32,
}

#[derive(Clone, Default)]
pub struct LightsData {
    directional: Vec<DirectionalLightData>,
    point: Vec<PointLightData>,
    spot: Vec<SpotLightData>,
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

    pub fn add_directional(&mut self, light: DirectionalLightData) {
        self.directional.push(light);
    }

    pub fn add_point(&mut self, light: PointLightData) {
        self.point.push(light);
    }

    pub fn add_spot(&mut self, light: SpotLightData) {
        self.spot.push(light);
    }

    pub fn directional_lights(&self) -> &[DirectionalLightData] {
        &self.directional
    }

    pub fn point_lights(&self) -> &[PointLightData] {
        &self.point
    }

    pub fn spot_lights(&self) -> &[SpotLightData] {
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
    pub fn from_data(data: &DirectionalLightData) -> Self {
        Self {
            direction: [data.direction.x, data.direction.y, data.direction.z, 0.0],
            color_intensity: [data.color.x, data.color.y, data.color.z, data.intensity],
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
    pub fn from_data(data: &PointLightData) -> Self {
        Self {
            position_range: [
                data.position.x,
                data.position.y,
                data.position.z,
                data.range,
            ],
            color_intensity: [data.color.x, data.color.y, data.color.z, data.intensity],
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
    pub fn from_data(data: &SpotLightData) -> Self {
        let mut inner = data.inner_angle;
        let mut outer = data.outer_angle;
        if inner > outer {
            std::mem::swap(&mut inner, &mut outer);
        }
        let cos_inner = inner.cos();
        let cos_outer = outer.cos();

        Self {
            position_range: [
                data.position.x,
                data.position.y,
                data.position.z,
                data.range,
            ],
            direction: [data.direction.x, data.direction.y, data.direction.z, 0.0],
            color_intensity: [data.color.x, data.color.y, data.color.z, data.intensity],
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
            *dst = DirectionalLightRaw::from_data(src);
        }

        let point_count = data.point_lights().len().min(MAX_POINT_LIGHTS) as u32;
        uniform.counts[1] = point_count;
        for (dst, src) in uniform
            .points
            .iter_mut()
            .zip(data.point_lights().iter())
            .take(point_count as usize)
        {
            *dst = PointLightRaw::from_data(src);
        }

        let spot_count = data.spot_lights().len().min(MAX_SPOT_LIGHTS) as u32;
        uniform.counts[2] = spot_count;
        for (dst, src) in uniform
            .spots
            .iter_mut()
            .zip(data.spot_lights().iter())
            .take(spot_count as usize)
        {
            *dst = SpotLightRaw::from_data(src);
        }

        uniform
    }
}
