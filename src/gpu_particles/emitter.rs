// src/gpu_particles/emitter.rs
use glam::Vec3;
use rand::{rngs::SmallRng, Rng, SeedableRng};

use super::particle::{Particle, MAX_COLOR_KEYS};

#[derive(Clone, Copy, Debug)]
pub enum EmissionShape {
    Point,
    Sphere { radius: f32 },
    Box { half_extents: Vec3 },
    Cone { angle: f32, radius: f32 },
    Disc { radius: f32 },
    Ring { radius: f32, thickness: f32 },
    RadialBurst,
}

#[derive(Clone, Debug)]
pub struct ColorGradient {
    pub keyframes: Vec<([f32; 4], f32)>, // (color, time_point)
}

impl ColorGradient {
    pub fn new() -> Self {
        Self {
            keyframes: vec![([1.0, 1.0, 1.0, 1.0], 0.0)],
        }
    }

    pub fn with_keyframe(mut self, color: [f32; 4], time: f32) -> Self {
        self.keyframes.push((color, time.clamp(0.0, 1.0)));
        self.keyframes
            .sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        self
    }

    pub fn sample(&self, t: f32) -> [f32; 4] {
        if self.keyframes.is_empty() {
            return [1.0; 4];
        }
        if self.keyframes.len() == 1 {
            return self.keyframes[0].0;
        }

        let t = t.clamp(0.0, 1.0);

        // Find surrounding keyframes
        let mut before = &self.keyframes[0];
        let mut after = &self.keyframes[self.keyframes.len() - 1];

        for i in 0..self.keyframes.len() - 1 {
            if self.keyframes[i].1 <= t && self.keyframes[i + 1].1 >= t {
                before = &self.keyframes[i];
                after = &self.keyframes[i + 1];
                break;
            }
        }

        if (after.1 - before.1).abs() < 1e-6 {
            return before.0;
        }

        let factor = (t - before.1) / (after.1 - before.1);
        [
            before.0[0] + (after.0[0] - before.0[0]) * factor,
            before.0[1] + (after.0[1] - before.0[1]) * factor,
            before.0[2] + (after.0[2] - before.0[2]) * factor,
            before.0[3] + (after.0[3] - before.0[3]) * factor,
        ]
    }
}

impl Default for ColorGradient {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct SizeCurve {
    pub keyframes: Vec<(f32, f32)>, // (size, time_point)
}

impl SizeCurve {
    pub fn new(size: f32) -> Self {
        Self {
            keyframes: vec![(size, 0.0)],
        }
    }

    pub fn with_keyframe(mut self, size: f32, time: f32) -> Self {
        self.keyframes.push((size, time));
        self.keyframes
            .sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        self
    }

    pub fn sample(&self, t: f32) -> f32 {
        if self.keyframes.is_empty() {
            return 1.0;
        }
        if self.keyframes.len() == 1 {
            return self.keyframes[0].0;
        }

        let t = t.clamp(0.0, 1.0);

        let mut before = &self.keyframes[0];
        let mut after = &self.keyframes[self.keyframes.len() - 1];

        for i in 0..self.keyframes.len() - 1 {
            if self.keyframes[i].1 <= t && self.keyframes[i + 1].1 >= t {
                before = &self.keyframes[i];
                after = &self.keyframes[i + 1];
                break;
            }
        }

        if (after.1 - before.1).abs() < 1e-6 {
            return before.0;
        }

        let factor = (t - before.1) / (after.1 - before.1);
        before.0 + (after.0 - before.0) * factor
    }
}

pub struct ParticleEmitter {
    pub spawn_rate: f32,
    pub burst_count: Option<u32>,
    pub position: Vec3,
    pub emission_shape: EmissionShape,
    pub initial_velocity_range: (Vec3, Vec3),
    pub initial_scale_range: (Vec3, Vec3),
    pub lifetime_range: (f32, f32),
    pub color_gradient: ColorGradient,
    pub size_curve: SizeCurve,
    pub radial_velocity: (f32, f32), // For radial burst effects
    spawn_accumulator: f32,
    pub total_spawned: u32,
    pub auto_respawn: bool, // Whether to reset after burst completes
    rng: SmallRng,
}

impl ParticleEmitter {
    pub fn new(position: Vec3, spawn_rate: f32) -> Self {
        Self {
            spawn_rate,
            burst_count: None,
            position,
            emission_shape: EmissionShape::Point,
            initial_velocity_range: (Vec3::ZERO, Vec3::ZERO),
            initial_scale_range: (Vec3::ONE, Vec3::ONE),
            lifetime_range: (5.0, 5.0),
            color_gradient: ColorGradient::new(),
            size_curve: SizeCurve::new(1.0),
            radial_velocity: (0.0, 0.0),
            spawn_accumulator: 0.0,
            total_spawned: 0,
            auto_respawn: false,
            rng: SmallRng::from_entropy(),
        }
    }

    pub fn with_burst(mut self, count: u32) -> Self {
        self.burst_count = Some(count);
        self
    }

    pub fn with_auto_respawn(mut self, enabled: bool) -> Self {
        self.auto_respawn = enabled;
        self
    }

    pub fn with_emission_shape(mut self, shape: EmissionShape) -> Self {
        self.emission_shape = shape;
        self
    }

    pub fn with_velocity(mut self, min: Vec3, max: Vec3) -> Self {
        self.initial_velocity_range = (min, max);
        self
    }

    pub fn with_radial_velocity(mut self, min: f32, max: f32) -> Self {
        self.radial_velocity = (min, max);
        self
    }

    pub fn with_scale(mut self, min: Vec3, max: Vec3) -> Self {
        self.initial_scale_range = (min, max);
        self
    }

    pub fn with_lifetime(mut self, min: f32, max: f32) -> Self {
        self.lifetime_range = (min, max);
        self
    }

    pub fn with_color_gradient(mut self, gradient: ColorGradient) -> Self {
        self.color_gradient = gradient;
        self
    }

    pub fn with_size_curve(mut self, curve: SizeCurve) -> Self {
        self.size_curve = curve;
        self
    }

    /// Seed the emitter's random number generator for deterministic spawning.
    pub fn with_seed(mut self, seed: u64) -> Self {
        self.rng = SmallRng::seed_from_u64(seed);
        self
    }

    pub fn reset(&mut self) {
        self.total_spawned = 0;
        self.spawn_accumulator = 0.0;
    }

    /// Reseed the emitter's random generator without rebuilding the struct.
    pub fn reseed(&mut self, seed: u64) {
        self.rng = SmallRng::seed_from_u64(seed);
    }

    pub fn is_complete(&self) -> bool {
        // Check if it's a burst emitter
        if let Some(burst_count) = self.burst_count {
            // Burst is complete if we've spawned all particles and not auto-respawning
            self.total_spawned >= burst_count && !self.auto_respawn
        } else {
            // Continuous emitters (spawn_rate > 0) are never "complete"
            false
        }
    }

    pub fn emit_into(&mut self, dt: f32, output: &mut Vec<Particle>) {
        // Check if burst is complete
        if let Some(burst_count) = self.burst_count {
            if self.total_spawned >= burst_count {
                if self.auto_respawn {
                    self.reset();
                } else {
                    return;
                }
            }
        }

        if self.spawn_rate <= 0.0 {
            if let Some(burst_count) = self.burst_count {
                let remaining = burst_count.saturating_sub(self.total_spawned);
                if remaining == 0 {
                    return;
                }

                self.total_spawned += remaining;
                output.reserve(remaining as usize);
                for _ in 0..remaining {
                    output.push(self.spawn_particle());
                }
                return;
            }

            return;
        }

        self.spawn_accumulator += dt * self.spawn_rate;
        let to_spawn = self.spawn_accumulator.floor() as u32;
        self.spawn_accumulator -= to_spawn as f32;

        let actual_spawn = if let Some(burst_count) = self.burst_count {
            let remaining = burst_count.saturating_sub(self.total_spawned);
            let spawn = to_spawn.min(remaining);
            self.total_spawned += spawn;
            spawn
        } else {
            to_spawn
        };

        if actual_spawn == 0 {
            return;
        }

        output.reserve(actual_spawn as usize);
        for _ in 0..actual_spawn {
            output.push(self.spawn_particle());
        }
    }

    fn spawn_particle(&mut self) -> Particle {
        fn sample_range(rng: &mut impl Rng, min: f32, max: f32) -> f32 {
            let (lo, hi) = if max < min { (max, min) } else { (min, max) };
            if hi <= lo {
                lo
            } else {
                rng.gen_range(lo..hi)
            }
        }

        fn sample_vec3(rng: &mut impl Rng, min: Vec3, max: Vec3) -> Vec3 {
            Vec3::new(
                sample_range(rng, min.x, max.x),
                sample_range(rng, min.y, max.y),
                sample_range(rng, min.z, max.z),
            )
        }

        let (offset, direction) = match self.emission_shape {
            EmissionShape::Point => (Vec3::ZERO, Vec3::ZERO),
            EmissionShape::Sphere { radius } => {
                let theta = self.rng.gen_range(0.0..std::f32::consts::TAU);
                let phi = self.rng.gen_range(0.0..std::f32::consts::PI);
                let r = sample_range(&mut self.rng, 0.0, radius.max(0.0));
                let pos = Vec3::new(
                    r * phi.sin() * theta.cos(),
                    r * phi.sin() * theta.sin(),
                    r * phi.cos(),
                );
                (pos, Vec3::ZERO)
            }
            EmissionShape::Box { half_extents } => {
                let pos = Vec3::new(
                    sample_range(&mut self.rng, -half_extents.x, half_extents.x),
                    sample_range(&mut self.rng, -half_extents.y, half_extents.y),
                    sample_range(&mut self.rng, -half_extents.z, half_extents.z),
                );
                (pos, Vec3::ZERO)
            }
            EmissionShape::Cone { angle, radius } => {
                let theta = self.rng.gen_range(0.0..std::f32::consts::TAU);
                let phi = sample_range(&mut self.rng, 0.0, angle.max(0.0));
                let r = sample_range(&mut self.rng, 0.0, radius.max(0.0));
                let pos = Vec3::new(
                    r * phi.sin() * theta.cos(),
                    r * phi.cos(),
                    r * phi.sin() * theta.sin(),
                );
                (pos, Vec3::ZERO)
            }
            EmissionShape::Disc { radius } => {
                let theta = self.rng.gen_range(0.0..std::f32::consts::TAU);
                let r = sample_range(&mut self.rng, 0.0, radius.max(0.0));
                let pos = Vec3::new(r * theta.cos(), 0.0, r * theta.sin());
                (pos, Vec3::ZERO)
            }
            EmissionShape::Ring { radius, thickness } => {
                let theta = self.rng.gen_range(0.0..std::f32::consts::TAU);
                let half_thickness = (thickness / 2.0).max(0.0);
                let r = radius + sample_range(&mut self.rng, -half_thickness, half_thickness);
                let pos = Vec3::new(r * theta.cos(), 0.0, r * theta.sin());
                (pos, Vec3::ZERO)
            }
            EmissionShape::RadialBurst => {
                // Sphere direction for burst
                let theta = self.rng.gen_range(0.0..std::f32::consts::TAU);
                let phi = self.rng.gen_range(0.0..std::f32::consts::PI);
                let dir = Vec3::new(phi.sin() * theta.cos(), phi.sin() * theta.sin(), phi.cos());
                (Vec3::ZERO, dir)
            }
        };

        let position = self.position + offset;

        // Calculate velocity
        let base_velocity = sample_vec3(
            &mut self.rng,
            self.initial_velocity_range.0,
            self.initial_velocity_range.1,
        );

        let velocity = if self.radial_velocity.1 > 0.0 {
            // Add radial component if specified
            let radial_speed = sample_range(
                &mut self.rng,
                self.radial_velocity.0,
                self.radial_velocity.1,
            );
            let radial_dir = if direction.length_squared() > 1e-6 {
                direction.normalize()
            } else {
                // Use position offset as direction
                if offset.length_squared() > 1e-6 {
                    offset.normalize()
                } else {
                    Vec3::Y
                }
            };
            base_velocity + radial_dir * radial_speed
        } else {
            base_velocity
        };

        // ✅ Use uniform scale (single value for all axes)
        let scale_range = (
            (self.initial_scale_range.0.x
                + self.initial_scale_range.0.y
                + self.initial_scale_range.0.z)
                / 3.0,
            (self.initial_scale_range.1.x
                + self.initial_scale_range.1.y
                + self.initial_scale_range.1.z)
                / 3.0,
        );
        let scale_uniform = sample_range(&mut self.rng, scale_range.0, scale_range.1);

        let lifetime = sample_range(&mut self.rng, self.lifetime_range.0, self.lifetime_range.1);

        // Prepare gradient key data
        let mut color_keys = [[1.0; 4]; MAX_COLOR_KEYS];
        let mut color_key_times = [0.0_f32; MAX_COLOR_KEYS];

        let mut key_count = self.color_gradient.keyframes.len().min(MAX_COLOR_KEYS);

        if key_count == 0 {
            color_key_times = [0.0, 1.0, 1.0, 1.0];
            key_count = 1;
        } else {
            for (i, (color, time)) in self
                .color_gradient
                .keyframes
                .iter()
                .take(MAX_COLOR_KEYS)
                .enumerate()
            {
                color_keys[i] = *color;
                color_key_times[i] = time.clamp(0.0, 1.0);
            }

            let last_index = key_count - 1;
            for slot in key_count..MAX_COLOR_KEYS {
                color_keys[slot] = color_keys[last_index];
                color_key_times[slot] = color_key_times[last_index];
            }
        }

        let initial_color = color_keys[0];

        let start_size = self.size_curve.sample(0.0);
        let end_size = self.size_curve.sample(1.0);

        // ✅ Store spawn scale with start_size applied
        let spawn_scale = scale_uniform * start_size;

        Particle {
            position: position.into(),
            lifetime: 0.0,
            velocity: velocity.into(),
            max_lifetime: lifetime,
            rotation: Particle::AXIS_ANGLE_IDENTITY,
            // ✅ Uniform scale on all axes
            scale: [spawn_scale, spawn_scale, spawn_scale],
            angular_velocity: self.rng.gen_range(-1.0..1.0),
            color: initial_color,
            color_keys,
            color_key_times,
            // ✅ Clean user_data layout:
            // [0] = spawn_scale (single value)
            // [1] = end_size / start_size (size ratio)
            // [2] = color key count
            // [3] = reserved
            user_data: [
                spawn_scale,
                end_size / start_size.max(0.001),
                key_count as f32,
                0.0,
            ],
        }
    }

    // =========================================================================
    // PRESET CONSTRUCTORS
    // =========================================================================

    pub fn fountain(position: Vec3) -> Self {
        Self::new(position, 50.0)
            .with_emission_shape(EmissionShape::Cone {
                angle: std::f32::consts::PI / 8.0,
                radius: 0.3,
            })
            .with_velocity(Vec3::ZERO, Vec3::new(0.5, 0.5, 0.5))
            .with_radial_velocity(8.0, 12.0)
            .with_lifetime(2.0, 3.0)
            .with_scale(Vec3::splat(0.05), Vec3::splat(0.15))
            .with_color_gradient(
                ColorGradient::new()
                    // Blue water, opaque
                    .with_keyframe([0.3, 0.5, 1.0, 0.9], 0.0)
                    // Still blue, fading
                    .with_keyframe([0.3, 0.5, 1.0, 0.6], 0.7)
                    // Fade to transparent at end
                    .with_keyframe([0.2, 0.4, 0.9, 0.0], 1.0),
            )
            .with_size_curve(
                SizeCurve::new(1.0)
                    .with_keyframe(0.8, 0.5)
                    .with_keyframe(0.3, 1.0),
            )
    }

    pub fn firework(position: Vec3) -> Self {
        Self::new(position, 0.0)
            .with_burst(300)
            .with_emission_shape(EmissionShape::RadialBurst)
            .with_radial_velocity(10.0, 15.0)
            .with_lifetime(1.5, 2.5)
            .with_scale(Vec3::splat(0.08), Vec3::splat(0.12))
            .with_color_gradient(
                ColorGradient::new()
                    .with_keyframe([1.0, 0.8, 0.2, 1.0], 0.0)
                    .with_keyframe([1.0, 0.5, 0.0, 1.0], 0.3)
                    .with_keyframe([1.0, 0.2, 0.0, 0.5], 0.7)
                    .with_keyframe([0.5, 0.1, 0.0, 0.0], 1.0),
            )
            .with_size_curve(
                SizeCurve::new(1.2)
                    .with_keyframe(1.5, 0.2)
                    .with_keyframe(0.2, 1.0),
            )
    }

    pub fn smoke(position: Vec3) -> Self {
        Self::new(position, 15.0)
            .with_emission_shape(EmissionShape::Sphere { radius: 0.25 })
            .with_velocity(Vec3::new(-0.3, 0.5, -0.3), Vec3::new(0.3, 1.2, 0.3))
            .with_lifetime(4.0, 6.0)
            .with_scale(Vec3::splat(0.15), Vec3::splat(0.3))
            .with_color_gradient(
                ColorGradient::new()
                    // Start: Medium gray, fairly visible
                    .with_keyframe([0.6, 0.6, 0.6, 0.7], 0.0)
                    // Middle: Darker, more transparent
                    .with_keyframe([0.45, 0.45, 0.45, 0.45], 0.5)
                    // End: Very dark and transparent
                    .with_keyframe([0.25, 0.25, 0.25, 0.0], 1.0),
            )
            .with_size_curve(
                SizeCurve::new(0.4)
                    .with_keyframe(1.2, 0.5)
                    .with_keyframe(2.0, 1.0),
            )
    }

    pub fn explosion(position: Vec3) -> Self {
        Self::new(position, 0.0)
            .with_burst(500)
            .with_emission_shape(EmissionShape::RadialBurst)
            .with_radial_velocity(5.0, 20.0)
            .with_lifetime(0.5, 1.5)
            .with_scale(Vec3::splat(0.1), Vec3::splat(0.2))
            .with_color_gradient(
                ColorGradient::new()
                    // Flash: Bright yellow-white
                    .with_keyframe([1.0, 1.0, 0.9, 1.0], 0.0)
                    // Hot: Orange
                    .with_keyframe([1.0, 0.6, 0.1, 1.0], 0.15)
                    // Fire: Red-orange
                    .with_keyframe([0.9, 0.3, 0.0, 0.8], 0.4)
                    // Smoke: Dark and fading
                    .with_keyframe([0.3, 0.2, 0.2, 0.3], 0.7)
                    // Gone
                    .with_keyframe([0.2, 0.2, 0.2, 0.0], 1.0),
            )
            .with_size_curve(
                SizeCurve::new(1.5)
                    .with_keyframe(2.0, 0.3)
                    .with_keyframe(0.5, 1.0),
            )
    }
}
