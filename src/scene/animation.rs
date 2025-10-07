use glam::{Quat, Vec3, Vec4};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationInterpolation {
    Step,
    Linear,
    CubicSpline,
}

#[derive(Debug, Clone)]
pub enum AnimationOutput {
    Vec3(Vec<Vec3>),
    Quat(Vec<Quat>),
    Vec4(Vec<Vec4>),
}

#[derive(Debug, Clone)]
pub struct AnimationSampler {
    pub times: Vec<f32>,
    pub output: AnimationOutput,
    pub interpolation: AnimationInterpolation,
}

impl AnimationSampler {
    pub fn end_time(&self) -> f32 {
        self.times.last().copied().unwrap_or(0.0)
    }

    fn sample_indices(&self, time: f32) -> Option<(usize, usize, f32)> {
        if self.times.is_empty() {
            return None;
        }

        if self.times.len() == 1 {
            return Some((0, 0, 0.0));
        }

        let first = self.times[0];
        if time <= first {
            return Some((0, 0, 0.0));
        }

        let last_index = self.times.len() - 1;
        let last = self.times[last_index];
        if time >= last {
            return Some((last_index, last_index, 0.0));
        }

        match self
            .times
            .binary_search_by(|probe| probe.partial_cmp(&time).unwrap())
        {
            Ok(index) => Some((index, index, 0.0)),
            Err(upper) => {
                if upper == 0 || upper >= self.times.len() {
                    return None;
                }
                let lower = upper - 1;
                let start = self.times[lower];
                let end = self.times[upper];
                let span = end - start;
                let factor = if span.abs() < f32::EPSILON {
                    0.0
                } else {
                    ((time - start) / span).clamp(0.0, 1.0)
                };
                Some((lower, upper, factor))
            }
        }
    }

    /// For cubic spline, output array contains [in_tangent, value, out_tangent] for each keyframe
    fn get_cubic_spline_segment_vec3(
        &self,
        values: &[Vec3],
        lower: usize,
        upper: usize,
    ) -> Option<(Vec3, Vec3, Vec3, Vec3)> {
        if lower == upper {
            let idx = lower * 3 + 1; // Get the value component
            if idx >= values.len() {
                return None;
            }
            return Some((values[idx], values[idx], values[idx], values[idx]));
        }

        let lower_value_idx = lower * 3 + 1;
        let lower_out_tangent_idx = lower * 3 + 2;
        let upper_in_tangent_idx = upper * 3;
        let upper_value_idx = upper * 3 + 1;

        if upper_value_idx >= values.len() {
            return None;
        }

        Some((
            values[lower_value_idx],
            values[lower_out_tangent_idx],
            values[upper_in_tangent_idx],
            values[upper_value_idx],
        ))
    }

    fn get_cubic_spline_segment_vec4(
        &self,
        values: &[Vec4],
        lower: usize,
        upper: usize,
    ) -> Option<(Vec4, Vec4, Vec4, Vec4)> {
        if lower == upper {
            let idx = lower * 3 + 1;
            if idx >= values.len() {
                return None;
            }
            return Some((values[idx], values[idx], values[idx], values[idx]));
        }

        let lower_value_idx = lower * 3 + 1;
        let lower_out_tangent_idx = lower * 3 + 2;
        let upper_in_tangent_idx = upper * 3;
        let upper_value_idx = upper * 3 + 1;

        if upper_value_idx >= values.len() {
            return None;
        }

        Some((
            values[lower_value_idx],
            values[lower_out_tangent_idx],
            values[upper_in_tangent_idx],
            values[upper_value_idx],
        ))
    }

    fn get_cubic_spline_segment_quat(
        &self,
        values: &[Quat],
        lower: usize,
        upper: usize,
    ) -> Option<(Quat, Quat, Quat, Quat)> {
        if lower == upper {
            let idx = lower * 3 + 1;
            if idx >= values.len() {
                return None;
            }
            return Some((values[idx], values[idx], values[idx], values[idx]));
        }

        let lower_value_idx = lower * 3 + 1;
        let lower_out_tangent_idx = lower * 3 + 2;
        let upper_in_tangent_idx = upper * 3;
        let upper_value_idx = upper * 3 + 1;

        if upper_value_idx >= values.len() {
            return None;
        }

        Some((
            values[lower_value_idx],
            values[lower_out_tangent_idx],
            values[upper_in_tangent_idx],
            values[upper_value_idx],
        ))
    }

    /// Hermite cubic spline interpolation
    fn cubic_hermite_vec3(p0: Vec3, m0: Vec3, m1: Vec3, p1: Vec3, t: f32, dt: f32) -> Vec3 {
        let t2 = t * t;
        let t3 = t2 * t;

        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;

        p0 * h00 + m0 * h10 * dt + p1 * h01 + m1 * h11 * dt
    }

    fn cubic_hermite_vec4(p0: Vec4, m0: Vec4, m1: Vec4, p1: Vec4, t: f32, dt: f32) -> Vec4 {
        let t2 = t * t;
        let t3 = t2 * t;

        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;

        p0 * h00 + m0 * h10 * dt + p1 * h01 + m1 * h11 * dt
    }

    fn cubic_hermite_quat(p0: Quat, m0: Quat, m1: Quat, p1: Quat, t: f32, dt: f32) -> Quat {
        let t2 = t * t;
        let t3 = t2 * t;

        let h00 = 2.0 * t3 - 3.0 * t2 + 1.0;
        let h10 = t3 - 2.0 * t2 + t;
        let h01 = -2.0 * t3 + 3.0 * t2;
        let h11 = t3 - t2;

        // Component-wise interpolation for quaternions in cubic spline
        let result = Quat::from_xyzw(
            p0.x * h00 + m0.x * h10 * dt + p1.x * h01 + m1.x * h11 * dt,
            p0.y * h00 + m0.y * h10 * dt + p1.y * h01 + m1.y * h11 * dt,
            p0.z * h00 + m0.z * h10 * dt + p1.z * h01 + m1.z * h11 * dt,
            p0.w * h00 + m0.w * h10 * dt + p1.w * h01 + m1.w * h11 * dt,
        );

        result.normalize()
    }

    pub fn sample_vec3(&self, time: f32) -> Option<Vec3> {
        let values = match &self.output {
            AnimationOutput::Vec3(values) => values,
            _ => return None,
        };

        let (lower, upper, factor) = self.sample_indices(time)?;

        match self.interpolation {
            AnimationInterpolation::Step => Some(values[lower]),
            AnimationInterpolation::Linear => {
                if lower == upper {
                    Some(values[lower])
                } else {
                    Some(values[lower].lerp(values[upper], factor))
                }
            }
            AnimationInterpolation::CubicSpline => {
                let (p0, m0, m1, p1) = self.get_cubic_spline_segment_vec3(values, lower, upper)?;
                if lower == upper {
                    Some(p0)
                } else {
                    let dt = self.times[upper] - self.times[lower];
                    Some(Self::cubic_hermite_vec3(p0, m0, m1, p1, factor, dt))
                }
            }
        }
    }

    pub fn sample_vec4(&self, time: f32) -> Option<Vec4> {
        let values = match &self.output {
            AnimationOutput::Vec4(values) => values,
            _ => return None,
        };

        let (lower, upper, factor) = self.sample_indices(time)?;

        match self.interpolation {
            AnimationInterpolation::Step => Some(values[lower]),
            AnimationInterpolation::Linear => {
                if lower == upper {
                    Some(values[lower])
                } else {
                    Some(values[lower].lerp(values[upper], factor))
                }
            }
            AnimationInterpolation::CubicSpline => {
                let (p0, m0, m1, p1) = self.get_cubic_spline_segment_vec4(values, lower, upper)?;
                if lower == upper {
                    Some(p0)
                } else {
                    let dt = self.times[upper] - self.times[lower];
                    Some(Self::cubic_hermite_vec4(p0, m0, m1, p1, factor, dt))
                }
            }
        }
    }

    pub fn sample_quat(&self, time: f32) -> Option<Quat> {
        let values = match &self.output {
            AnimationOutput::Quat(values) => values,
            _ => return None,
        };

        let (lower, upper, factor) = self.sample_indices(time)?;

        match self.interpolation {
            AnimationInterpolation::Step => Some(values[lower]),
            AnimationInterpolation::Linear => {
                if lower == upper {
                    Some(values[lower])
                } else {
                    let a = values[lower].normalize();
                    let b = values[upper].normalize();
                    Some(a.slerp(b, factor).normalize())
                }
            }
            AnimationInterpolation::CubicSpline => {
                let (p0, m0, m1, p1) = self.get_cubic_spline_segment_quat(values, lower, upper)?;
                if lower == upper {
                    Some(p0.normalize())
                } else {
                    let dt = self.times[upper] - self.times[lower];
                    Some(Self::cubic_hermite_quat(p0, m0, m1, p1, factor, dt))
                }
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransformProperty {
    Translation,
    Rotation,
    Scale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialProperty {
    BaseColorFactor,
}

#[derive(Debug, Clone, Copy)]
pub enum AnimationTarget {
    Transform {
        entity: hecs::Entity,
        property: TransformProperty,
    },
    Material {
        material_index: usize,
        property: MaterialProperty,
    },
}

#[derive(Debug, Clone)]
pub struct AnimationChannel {
    pub sampler: AnimationSampler,
    pub target: AnimationTarget,
}

#[derive(Debug, Clone)]
pub struct AnimationClip {
    pub name: String,
    pub duration: f32,
    pub channels: Vec<AnimationChannel>,
}

impl AnimationClip {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            duration: 0.0,
            channels: Vec::new(),
        }
    }

    pub fn add_channel(&mut self, channel: AnimationChannel) {
        self.duration = self.duration.max(channel.sampler.end_time());
        self.channels.push(channel);
    }

    pub fn sample(
        &self,
        time: f32,
        transform_updates: &mut HashMap<hecs::Entity, TransformUpdate>,
        material_updates: &mut HashMap<usize, MaterialUpdate>,
    ) {
        for channel in &self.channels {
            match channel.target {
                AnimationTarget::Transform { entity, property } => {
                    let entry = transform_updates.entry(entity).or_default();
                    match property {
                        TransformProperty::Translation => {
                            if let Some(value) = channel.sampler.sample_vec3(time) {
                                entry.translation = Some(value);
                            }
                        }
                        TransformProperty::Rotation => {
                            if let Some(value) = channel.sampler.sample_quat(time) {
                                entry.rotation = Some(value);
                            }
                        }
                        TransformProperty::Scale => {
                            if let Some(value) = channel.sampler.sample_vec3(time) {
                                entry.scale = Some(value);
                            }
                        }
                    }
                }
                AnimationTarget::Material {
                    material_index,
                    property,
                } => {
                    let entry = material_updates.entry(material_index).or_default();
                    match property {
                        MaterialProperty::BaseColorFactor => {
                            if let Some(value) = channel.sampler.sample_vec4(time) {
                                entry.base_color = Some(value);
                            }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnimationState {
    pub clip_index: usize,
    pub time: f32,
    pub speed: f32,
    pub looping: bool,
    pub playing: bool,
}

impl AnimationState {
    pub fn new(clip_index: usize) -> Self {
        Self {
            clip_index,
            time: 0.0,
            speed: 1.0,
            looping: true,
            playing: true,
        }
    }

    pub fn advance(&mut self, dt: f32, duration: f32) -> f32 {
        if !self.playing {
            return self.time;
        }

        let mut time = self.time + dt * self.speed;
        let duration = duration.max(0.0);

        if duration > 0.0 {
            if self.looping {
                time = time.rem_euclid(duration);
                if time < 0.0 {
                    time += duration;
                }
            } else {
                if time >= duration {
                    time = duration;
                    self.playing = false;
                } else if time < 0.0 {
                    time = 0.0;
                }
            }
        }

        self.time = time;
        time
    }
}

#[derive(Debug, Default, Clone)]
pub struct TransformUpdate {
    pub translation: Option<Vec3>,
    pub rotation: Option<Quat>,
    pub scale: Option<Vec3>,
}

#[derive(Debug, Default, Clone)]
pub struct MaterialUpdate {
    pub base_color: Option<Vec4>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{vec3, vec4};
    use hecs::World;

    #[test]
    fn sampler_vec3_linear_interpolation() {
        let sampler = AnimationSampler {
            times: vec![0.0, 1.0],
            output: AnimationOutput::Vec3(vec![Vec3::ZERO, Vec3::ONE]),
            interpolation: AnimationInterpolation::Linear,
        };

        assert_eq!(sampler.sample_vec3(-0.5).unwrap(), Vec3::ZERO);
        assert_eq!(sampler.sample_vec3(0.0).unwrap(), Vec3::ZERO);
        assert_eq!(sampler.sample_vec3(1.0).unwrap(), Vec3::ONE);

        let mid = sampler.sample_vec3(0.5).unwrap();
        assert!((mid - vec3(0.5, 0.5, 0.5)).length() < 1e-6);
    }

    #[test]
    fn sampler_quat_spherical_interpolation() {
        let sampler = AnimationSampler {
            times: vec![0.0, 1.0],
            output: AnimationOutput::Quat(vec![
                Quat::IDENTITY,
                Quat::from_rotation_y(std::f32::consts::PI),
            ]),
            interpolation: AnimationInterpolation::Linear,
        };

        let half = sampler.sample_quat(0.5).unwrap();
        let rotated_half = (half * Vec3::Z).normalize();
        assert!(
            rotated_half.z.abs() < 1e-4,
            "unexpected slerp result: {:?}",
            half
        );
        assert!(
            (rotated_half.x.abs() - 1.0).abs() < 1e-4,
            "unexpected slerp result: {:?}",
            half
        );
    }

    #[test]
    fn sampler_step_mode_picks_exact_key() {
        let sampler = AnimationSampler {
            times: vec![0.0, 1.0, 2.0],
            output: AnimationOutput::Vec4(vec![
                vec4(1.0, 0.0, 0.0, 1.0),
                vec4(0.0, 1.0, 0.0, 1.0),
                vec4(0.0, 0.0, 1.0, 1.0),
            ]),
            interpolation: AnimationInterpolation::Step,
        };

        assert_eq!(sampler.sample_vec4(0.1).unwrap(), vec4(1.0, 0.0, 0.0, 1.0));
        assert_eq!(sampler.sample_vec4(1.5).unwrap(), vec4(0.0, 1.0, 0.0, 1.0));
        assert_eq!(sampler.sample_vec4(2.0).unwrap(), vec4(0.0, 0.0, 1.0, 1.0));
    }

    #[test]
    fn animation_clip_writes_transform_and_material_updates() {
        let mut world = World::new();
        let entity = world.spawn(());

        let translation_sampler = AnimationSampler {
            times: vec![0.0, 1.0],
            output: AnimationOutput::Vec3(vec![Vec3::ZERO, Vec3::splat(2.0)]),
            interpolation: AnimationInterpolation::Linear,
        };
        let color_sampler = AnimationSampler {
            times: vec![0.0, 1.0],
            output: AnimationOutput::Vec4(vec![vec4(0.2, 0.2, 0.2, 1.0), vec4(0.8, 0.4, 0.6, 1.0)]),
            interpolation: AnimationInterpolation::Linear,
        };

        let mut clip = AnimationClip::new("test");
        clip.add_channel(AnimationChannel {
            sampler: translation_sampler,
            target: AnimationTarget::Transform {
                entity,
                property: TransformProperty::Translation,
            },
        });
        clip.add_channel(AnimationChannel {
            sampler: color_sampler,
            target: AnimationTarget::Material {
                material_index: 3,
                property: MaterialProperty::BaseColorFactor,
            },
        });

        let mut transform_updates = HashMap::new();
        let mut material_updates = HashMap::new();
        clip.sample(0.5, &mut transform_updates, &mut material_updates);

        let transform = transform_updates.get(&entity).expect("missing transform");
        assert!(transform.rotation.is_none());
        assert_eq!(transform.translation.unwrap(), vec3(1.0, 1.0, 1.0));

        let material = material_updates
            .get(&3)
            .expect("missing material update for index 3");
        let base_color = material.base_color.unwrap();
        let expected = vec4(0.5, 0.3, 0.4, 1.0);
        assert!((base_color - expected).length() < 1e-5);
    }

    #[test]
    fn animation_state_looping_and_clamp_behaviour() {
        let mut looping = AnimationState::new(0);
        looping.looping = true;
        looping.time = 1.5;
        let advanced = looping.advance(1.0, 2.0);
        assert!((advanced - 0.5).abs() < 1e-6);
        assert!(looping.playing);

        let mut once = AnimationState::new(0);
        once.looping = false;
        let advanced = once.advance(5.0, 2.0);
        assert!((advanced - 2.0).abs() < 1e-6);
        assert!(!once.playing);
        let advanced = once.advance(1.0, 2.0);
        assert!((advanced - 2.0).abs() < 1e-6);
    }

    #[test]
    fn cubic_spline_vec3_interpolation() {
        // Data format: [in_tangent_0, value_0, out_tangent_0, in_tangent_1, value_1, out_tangent_1]
        let sampler = AnimationSampler {
            times: vec![0.0, 1.0],
            output: AnimationOutput::Vec3(vec![
                vec3(0.0, 0.0, 0.0), // in-tangent for keyframe 0
                vec3(0.0, 0.0, 0.0), // value for keyframe 0
                vec3(1.0, 1.0, 1.0), // out-tangent for keyframe 0
                vec3(1.0, 1.0, 1.0), // in-tangent for keyframe 1
                vec3(2.0, 2.0, 2.0), // value for keyframe 1
                vec3(0.0, 0.0, 0.0), // out-tangent for keyframe 1
            ]),
            interpolation: AnimationInterpolation::CubicSpline,
        };

        let start = sampler.sample_vec3(0.0).unwrap();
        assert!((start - vec3(0.0, 0.0, 0.0)).length() < 1e-5);

        let end = sampler.sample_vec3(1.0).unwrap();
        assert!((end - vec3(2.0, 2.0, 2.0)).length() < 1e-5);

        // Middle should be smoothly interpolated
        let mid = sampler.sample_vec3(0.5).unwrap();
        assert!(mid.x > 0.5 && mid.x < 1.5);
    }

    #[test]
    fn cubic_spline_vec4_color_animation() {
        let sampler = AnimationSampler {
            times: vec![0.0, 1.0],
            output: AnimationOutput::Vec4(vec![
                vec4(0.0, 0.0, 0.0, 0.0),  // in-tangent
                vec4(1.0, 0.0, 0.0, 1.0),  // red
                vec4(0.0, 0.0, 1.0, 0.0),  // out-tangent
                vec4(0.0, 0.0, -1.0, 0.0), // in-tangent
                vec4(0.0, 0.0, 1.0, 1.0),  // blue
                vec4(0.0, 0.0, 0.0, 0.0),  // out-tangent
            ]),
            interpolation: AnimationInterpolation::CubicSpline,
        };

        let color = sampler.sample_vec4(0.5).unwrap();
        // Should transition smoothly from red to blue
        assert!(color.x >= 0.0 && color.z >= 0.0);
        assert!((color.w - 1.0).abs() < 1e-5); // Alpha stays at 1
    }
}
