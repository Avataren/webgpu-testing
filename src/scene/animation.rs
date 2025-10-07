use glam::{Quat, Vec3, Vec4};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationInterpolation {
    Step,
    Linear,
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

    pub fn sample_vec3(&self, time: f32) -> Option<Vec3> {
        let values = match &self.output {
            AnimationOutput::Vec3(values) => values,
            _ => return None,
        };

        let (lower, upper, factor) = self.sample_indices(time)?;

        if lower == upper || matches!(self.interpolation, AnimationInterpolation::Step) {
            return Some(values[lower]);
        }

        Some(values[lower].lerp(values[upper], factor))
    }

    pub fn sample_vec4(&self, time: f32) -> Option<Vec4> {
        let values = match &self.output {
            AnimationOutput::Vec4(values) => values,
            _ => return None,
        };

        let (lower, upper, factor) = self.sample_indices(time)?;

        if lower == upper || matches!(self.interpolation, AnimationInterpolation::Step) {
            return Some(values[lower]);
        }

        Some(values[lower].lerp(values[upper], factor))
    }

    pub fn sample_quat(&self, time: f32) -> Option<Quat> {
        let values = match &self.output {
            AnimationOutput::Quat(values) => values,
            _ => return None,
        };

        let (lower, upper, factor) = self.sample_indices(time)?;

        if lower == upper || matches!(self.interpolation, AnimationInterpolation::Step) {
            return Some(values[lower]);
        }

        let a = values[lower].normalize();
        let b = values[upper].normalize();
        Some(a.slerp(b, factor).normalize())
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
}
