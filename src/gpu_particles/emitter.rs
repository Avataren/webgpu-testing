use glam::{Quat, Vec3};
use rand::Rng;

use super::particle::Particle;

#[derive(Clone, Copy, Debug)]
pub enum EmissionShape {
    Point,
    Sphere { radius: f32 },
    Box { half_extents: Vec3 },
    Cone { angle: f32, radius: f32 },
}

pub struct ParticleEmitter {
    pub spawn_rate: f32,
    pub burst_count: Option<u32>,
    pub position: Vec3,
    pub emission_shape: EmissionShape,
    pub initial_velocity_range: (Vec3, Vec3),
    pub initial_scale_range: (Vec3, Vec3),
    pub lifetime_range: (f32, f32),
    pub initial_color: [f32; 4],
    spawn_accumulator: f32,
    total_spawned: u32,
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
            initial_color: [1.0; 4],
            spawn_accumulator: 0.0,
            total_spawned: 0,
        }
    }

    pub fn with_burst(mut self, count: u32) -> Self {
        self.burst_count = Some(count);
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

    pub fn with_scale(mut self, min: Vec3, max: Vec3) -> Self {
        self.initial_scale_range = (min, max);
        self
    }

    pub fn with_lifetime(mut self, min: f32, max: f32) -> Self {
        self.lifetime_range = (min, max);
        self
    }

    pub fn with_color(mut self, color: [f32; 4]) -> Self {
        self.initial_color = color;
        self
    }

    pub fn update(&mut self, dt: f32) -> Vec<Particle> {
        if let Some(burst_count) = self.burst_count {
            if self.total_spawned >= burst_count {
                return Vec::new();
            }
        }

        self.spawn_accumulator += dt * self.spawn_rate;
        let to_spawn = self.spawn_accumulator.floor() as u32;
        self.spawn_accumulator -= to_spawn as f32;

        if let Some(burst_count) = self.burst_count {
            let remaining = burst_count.saturating_sub(self.total_spawned);
            let actual_spawn = to_spawn.min(remaining);
            self.total_spawned += actual_spawn;
            (0..actual_spawn).map(|_| self.spawn_particle()).collect()
        } else {
            (0..to_spawn).map(|_| self.spawn_particle()).collect()
        }
    }

    fn spawn_particle(&self) -> Particle {
        let mut rng = rand::thread_rng();

        let offset = match self.emission_shape {
            EmissionShape::Point => Vec3::ZERO,
            EmissionShape::Sphere { radius } => {
                let theta = rng.gen_range(0.0..std::f32::consts::TAU);
                let phi = rng.gen_range(0.0..std::f32::consts::PI);
                let r = rng.gen_range(0.0..radius);
                Vec3::new(
                    r * phi.sin() * theta.cos(),
                    r * phi.sin() * theta.sin(),
                    r * phi.cos(),
                )
            }
            EmissionShape::Box { half_extents } => Vec3::new(
                rng.gen_range(-half_extents.x..half_extents.x),
                rng.gen_range(-half_extents.y..half_extents.y),
                rng.gen_range(-half_extents.z..half_extents.z),
            ),
            EmissionShape::Cone { angle, radius } => {
                let theta = rng.gen_range(0.0..std::f32::consts::TAU);
                let phi = rng.gen_range(0.0..angle);
                let r = rng.gen_range(0.0..radius);
                Vec3::new(
                    r * phi.sin() * theta.cos(),
                    r * phi.cos(),
                    r * phi.sin() * theta.sin(),
                )
            }
        };

        let position = self.position + offset;
        let velocity = Vec3::new(
            rng.gen_range(self.initial_velocity_range.0.x..self.initial_velocity_range.1.x),
            rng.gen_range(self.initial_velocity_range.0.y..self.initial_velocity_range.1.y),
            rng.gen_range(self.initial_velocity_range.0.z..self.initial_velocity_range.1.z),
        );

        let scale = Vec3::new(
            rng.gen_range(self.initial_scale_range.0.x..self.initial_scale_range.1.x),
            rng.gen_range(self.initial_scale_range.0.y..self.initial_scale_range.1.y),
            rng.gen_range(self.initial_scale_range.0.z..self.initial_scale_range.1.z),
        );

        let lifetime = rng.gen_range(self.lifetime_range.0..self.lifetime_range.1);

        Particle {
            position: position.into(),
            lifetime: 0.0,
            velocity: velocity.into(),
            max_lifetime: lifetime,
            rotation: Quat::IDENTITY.into(),
            scale: scale.into(),
            angular_velocity: rng.gen_range(-1.0..1.0),
            color: self.initial_color,
            user_data: [0.0; 4],
        }
    }
}
