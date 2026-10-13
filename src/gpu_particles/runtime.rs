//! Runs GPU particles for a scene, in the editor and the player alike:
//!
//! - entities with a particle system and emitter component each get a
//!   [`GpuParticleSystem`], kept in step with the component;
//! - one-off bursts scripts request with `emit_particles` ([`ParticleBurst`]) go to
//!   shared effect systems, one per physics setting and blend mode.

use std::collections::{HashMap, HashSet};

use glam::{Affine3A, Vec3};
use hecs::{ComponentError, Entity};
use log::{debug, warn};

use super::behaviors::{BoidsBehavior, OptimizedBoidsBehavior, PhysicsBehavior, StarfieldBehavior};
use super::{ColorGradient, EmissionShape, GpuParticleSystem, ParticleBehavior, ParticleEmitter};
use super::{ParticleRenderMode, SizeCurve};
use crate::app::GpuUpdateContext;
use crate::asset::{Handle, MaterialAsset, Mesh};
use crate::renderer::{CustomRenderContext, Material, PrimitiveMeshDescriptor, Renderer};
use crate::scene::components::{
    Billboard, CanCastShadow, DepthState, GpuParticleInstance, MaterialComponent, MeshComponent,
    ParticleBehaviorConfig, ParticleEmitterComponent, ParticleSystemComponent, TransformComponent,
    WorldTransform,
};
use crate::scene::transform::Transform;
use crate::scene::Scene;

/// Particles one effect system holds; bursts beyond this are dropped.
const EFFECT_CAPACITY: u32 = 16_384;
/// Longest simulation step. A stalled frame (a screenshot readback, a debug-build
/// hitch) would otherwise age every particle at once and end whole bursts early.
const MAX_STEP: f32 = 1.0 / 20.0;

/// A one-off burst of particles, as `emit_particles` requests it.
#[derive(Debug, Clone, PartialEq)]
pub struct ParticleBurst {
    pub position: Vec3,
    pub count: u32,
    /// Color at spawn and at the end of life; above 1 is HDR (blooms).
    pub color: [f32; 4],
    pub color_end: [f32; 4],
    /// Outward speed range in m/s.
    pub speed: (f32, f32),
    /// Lifetime range in seconds.
    pub life: (f32, f32),
    /// Quad size in meters at spawn and at the end of life.
    pub size: f32,
    pub size_end: f32,
    /// Added to every particle's velocity.
    pub velocity: Vec3,
    /// Vertical acceleration (negative falls).
    pub gravity: f32,
    /// Air drag: velocity loss per second, as a fraction.
    pub drag: f32,
    /// Particles start this far from the position (0 = a point).
    pub radius: f32,
    /// Fly out in all directions (`false`) or flat in the XZ plane (`true`, a ring).
    pub flat: bool,
    /// Additive (glow) or alpha blending.
    pub additive: bool,
}

impl Default for ParticleBurst {
    fn default() -> Self {
        Self {
            position: Vec3::ZERO,
            count: 24,
            color: [1.0, 0.8, 0.3, 1.0],
            color_end: [0.0, 0.0, 0.0, 0.0],
            speed: (2.0, 6.0),
            life: (0.4, 0.8),
            size: 0.15,
            size_end: 0.0,
            velocity: Vec3::ZERO,
            gravity: 0.0,
            drag: 1.0,
            radius: 0.0,
            flat: false,
            additive: true,
        }
    }
}

impl ParticleBurst {
    fn emitter(&self) -> ParticleEmitter {
        let shape = if self.flat {
            // A ring gives each particle an outward direction in the XZ plane. The
            // emitter takes the direction from the start offset, which it ignores
            // below 1 mm, so the ring is never smaller than 1 cm.
            EmissionShape::Ring {
                radius: self.radius.max(0.01),
                thickness: 0.0,
            }
        } else if self.radius > 0.0 {
            EmissionShape::Sphere {
                radius: self.radius,
            }
        } else {
            EmissionShape::RadialBurst
        };
        let size = self.size.max(0.001);
        ParticleEmitter::new(self.position, 0.0)
            .with_burst(self.count)
            .with_emission_shape(shape)
            .with_velocity(self.velocity, self.velocity)
            .with_radial_velocity(
                self.speed.0.min(self.speed.1),
                self.speed.0.max(self.speed.1).max(1e-4),
            )
            .with_lifetime(self.life.0.min(self.life.1), self.life.0.max(self.life.1))
            .with_color_gradient(
                ColorGradient::new()
                    .with_keyframe(self.color, 0.0)
                    .with_keyframe(self.color_end, 1.0),
            )
            .with_size_curve(SizeCurve::new(size).with_keyframe(self.size_end.max(0.0), 1.0))
    }

    /// Bursts with the same physics and blending share a system.
    fn effect_key(&self) -> EffectKey {
        EffectKey {
            gravity_cm: (self.gravity * 100.0).round() as i32,
            drag_pct: (self.drag * 100.0).round() as i32,
            additive: self.additive,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct EffectKey {
    gravity_cm: i32,
    drag_pct: i32,
    additive: bool,
}

struct EffectSystem {
    system: GpuParticleSystem,
    behavior: PhysicsBehavior,
}

#[derive(Clone, PartialEq)]
struct ParticleSystemDescriptor {
    mesh: Handle<Mesh>,
    material_handle: Handle<MaterialAsset>,
    material: Material,
    emitter: ParticleEmitterComponent,
    system: ParticleSystemComponent,
    depth_state: Option<DepthState>,
    casts_shadows: bool,
}

struct ParticleSceneData {
    entity: Entity,
    descriptor: ParticleSystemDescriptor,
    emitter_transform: Affine3A,
    existing_gpu_instance: Option<GpuParticleInstance>,
}

enum BehaviorRuntime {
    Physics(PhysicsBehavior),
    Starfield(StarfieldBehavior),
    Boids(BoidsBehavior),
    OptimizedBoids(Box<OptimizedBoidsBehavior>),
}

impl BehaviorRuntime {
    fn new(
        descriptor: &ParticleSystemDescriptor,
        device: &wgpu::Device,
        max_particles: u32,
    ) -> Self {
        match &descriptor.system.behavior_config {
            ParticleBehaviorConfig::Physics(config) => Self::Physics(config.to_behavior()),
            ParticleBehaviorConfig::Starfield(config) => Self::Starfield(config.to_behavior()),
            ParticleBehaviorConfig::Boids(config) => {
                let mut behavior = config.to_behavior();
                behavior.set_particle_count(max_particles);
                Self::Boids(behavior)
            }
            ParticleBehaviorConfig::OptimizedBoids(config) => {
                let mut behavior = OptimizedBoidsBehavior::new(
                    device,
                    max_particles,
                    config.bounds,
                    config
                        .separation_radius
                        .max(config.alignment_radius)
                        .max(config.cohesion_radius),
                );

                behavior.separation_radius = config.separation_radius;
                behavior.alignment_radius = config.alignment_radius;
                behavior.cohesion_radius = config.cohesion_radius;
                behavior.separation_weight = config.separation_weight;
                behavior.alignment_weight = config.alignment_weight;
                behavior.cohesion_weight = config.cohesion_weight;
                behavior.max_speed = config.max_speed;
                behavior.max_force = config.max_force;
                behavior.bounds = config.bounds;
                behavior.particle_count = max_particles;

                Self::OptimizedBoids(Box::new(behavior))
            }
        }
    }

    fn as_behavior(&self) -> &dyn ParticleBehavior {
        match self {
            Self::Physics(behavior) => behavior,
            Self::Starfield(behavior) => behavior,
            Self::Boids(behavior) => behavior,
            Self::OptimizedBoids(behavior) => behavior.as_ref(),
        }
    }

    fn prepare_update(
        &mut self,
        system: &mut GpuParticleSystem,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
    ) {
        if let Self::OptimizedBoids(behavior) = self {
            behavior.build_spatial_grid(
                device,
                queue,
                encoder,
                system.particles_buffer(),
                system.active_particle_count(),
            );
        }
    }
}

struct ParticleSystemEntry {
    system: GpuParticleSystem,
    behavior: BehaviorRuntime,
    descriptor: ParticleSystemDescriptor,
    gpu_index: u32,
}

/// GPU particles of the active scene: entity particle systems and script bursts.
#[derive(Default)]
pub struct ParticleRuntime {
    systems: HashMap<Entity, ParticleSystemEntry>,
    next_gpu_index: u32,
    effects: HashMap<EffectKey, EffectSystem>,
    /// The quad script bursts are drawn with.
    effect_mesh: Option<Handle<Mesh>>,
}

impl ParticleRuntime {
    /// Draw every particle system (call from the application's custom render hook).
    pub fn render(&mut self, ctx: &mut CustomRenderContext<'_>) {
        for entry in self.systems.values_mut() {
            if let Some(mesh) = ctx.scene.assets.meshes.get(entry.descriptor.mesh) {
                entry.system.render(ctx, mesh);
            }
        }
        if let Some(mesh) = self
            .effect_mesh
            .and_then(|handle| ctx.scene.assets.meshes.get(handle))
        {
            for effect in self.effects.values_mut() {
                effect.system.render(ctx, mesh);
            }
        }
    }

    pub fn has_shadow_casters(&self) -> bool {
        self.systems
            .values()
            .any(|entry| entry.system.casts_shadows())
    }

    /// Particles alive in script effects, roughly (for diagnostics).
    pub fn effect_particles(&self) -> u32 {
        self.effects
            .values()
            .map(|effect| effect.system.active_particle_count())
            .sum()
    }

    /// Bring the particle systems in line with the scene, start requested bursts and
    /// simulate one step (call once per frame from a GPU update).
    pub fn update(&mut self, ctx: &mut GpuUpdateContext<'_>) {
        self.sync_with_scene(ctx);
        // Entering or leaving play starts over: bursts from the last session vanish
        // with the rest of what play changed.
        if ctx.scene.take_script_effects_reset() {
            self.effects.clear();
        }
        let bursts = ctx.scene.take_particle_bursts();
        if !bursts.is_empty() {
            self.start_bursts(&mut ctx.scene, ctx.renderer, bursts);
        }
        // Step with the game (sped up by time_scale, frozen while paused), but clamp the
        // real frame time first so a stall does not end every burst at once.
        let stepped = ctx.scene.take_stepped_time().max(0.0);
        let real = ctx.dt.max(0.0);
        let speed = if real > 0.0 { stepped / real } else { 0.0 };
        let dt = (real.min(f64::from(MAX_STEP)) * speed) as f32;
        self.update_gpu(ctx.renderer, dt);
    }

    fn start_bursts(
        &mut self,
        scene: &mut Scene,
        renderer: &mut Renderer,
        bursts: Vec<ParticleBurst>,
    ) {
        // Bursts are drawn as camera-facing quads.
        if self
            .effect_mesh
            .filter(|handle| scene.assets.meshes.get(*handle).is_some())
            .is_none()
        {
            self.effect_mesh = Some(
                scene
                    .assets
                    .ensure_primitive_mesh(renderer, PrimitiveMeshDescriptor::Plane),
            );
        }
        for burst in bursts {
            let key = burst.effect_key();
            let effect = self.effects.entry(key).or_insert_with(|| {
                let behavior = PhysicsBehavior::default()
                    .with_gravity(Vec3::new(0.0, burst.gravity, 0.0))
                    .with_drag(burst.drag)
                    // No ground: effects in a game happen at any height.
                    .with_ground_collision(-1.0e9, 0.0, 0.0);
                let material = Material::new([255, 255, 255, 255])
                    .with_unlit()
                    .with_billboarding();
                let mode = if burst.additive {
                    ParticleRenderMode::Additive
                } else {
                    ParticleRenderMode::AlphaBlend
                };
                let system = GpuParticleSystem::new_with_mode(
                    renderer.get_device(),
                    renderer.get_queue(),
                    renderer,
                    EFFECT_CAPACITY,
                    material,
                    &behavior,
                    mode,
                );
                EffectSystem { system, behavior }
            });
            effect.system.add_emitter(burst.emitter());
        }
    }

    fn sync_with_scene(&mut self, ctx: &mut GpuUpdateContext<'_>) {
        let data = Self::collect_scene_data(&mut ctx.scene);
        let mut seen = HashSet::new();
        let mut pending_gpu_instance_updates = Vec::new();

        for scene_data in &data {
            seen.insert(scene_data.entity);
            let index = self.ensure_entry(scene_data, ctx.renderer);

            if scene_data.existing_gpu_instance.map(|inst| inst.index) != Some(index) {
                pending_gpu_instance_updates.push((scene_data.entity, index));
            }
        }

        let removed: Vec<_> = self
            .systems
            .keys()
            .copied()
            .filter(|entity| !seen.contains(entity))
            .collect();

        for entity in &removed {
            self.systems.remove(entity);
        }

        if !pending_gpu_instance_updates.is_empty() || !removed.is_empty() {
            let world = ctx.scene.main_world_mut();

            for (entity, index) in pending_gpu_instance_updates {
                let mut updated = false;
                if let Ok(mut existing) = world.get::<&mut GpuParticleInstance>(entity) {
                    existing.index = index;
                    updated = true;
                }

                if !updated {
                    if let Err(err) = world.insert(entity, (GpuParticleInstance { index },)) {
                        warn!(
                            "failed to tag {:?} as GPU-driven particle system: {}",
                            entity, err
                        );
                    }
                }
            }

            for entity in removed {
                if let Err(err) = world.remove_one::<GpuParticleInstance>(entity) {
                    if !matches!(
                        err,
                        ComponentError::NoSuchEntity | ComponentError::MissingComponent(_)
                    ) {
                        debug!(
                            "failed to remove GPU particle marker from {:?}: {}",
                            entity, err
                        );
                    }
                }
            }
        }
    }

    fn collect_scene_data(scene: &mut Scene) -> Vec<ParticleSceneData> {
        let mut data = Vec::new();

        let mut pending = Vec::new();

        {
            let world = scene.main_world_mut();
            let mut query = world.query::<(
                &ParticleSystemComponent,
                &ParticleEmitterComponent,
                &MaterialComponent,
                &MeshComponent,
                Option<&DepthState>,
                Option<&CanCastShadow>,
                Option<&Billboard>,
                Option<&GpuParticleInstance>,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
            )>();

            for (
                entity,
                (
                    system,
                    emitter,
                    material_component,
                    mesh,
                    depth_state,
                    casts_shadow,
                    billboard,
                    gpu_instance,
                    world_transform,
                    local_transform,
                ),
            ) in query.iter()
            {
                let transform = world_transform
                    .map(|wt| wt.0)
                    .or_else(|| local_transform.map(|lt| lt.0));
                let entity_affine = transform
                    .map(Self::transform_affine)
                    .unwrap_or(Affine3A::IDENTITY);
                let emitter_affine = Affine3A::from_translation(Vec3::from_array(emitter.position));
                let emitter_transform = entity_affine * emitter_affine;
                pending.push((
                    entity,
                    mesh.0,
                    material_component.0,
                    emitter.clone(),
                    (*system).clone(),
                    depth_state.copied(),
                    casts_shadow.map(|s| s.0).unwrap_or(false),
                    billboard.is_some(),
                    emitter_transform,
                    gpu_instance.copied(),
                ));
            }
        }

        for (
            entity,
            mesh_handle,
            material_handle,
            emitter,
            system,
            depth_state,
            casts_shadows,
            is_billboard,
            emitter_transform,
            existing_gpu_instance,
        ) in pending
        {
            let Some(asset) = scene.assets.material(material_handle) else {
                warn!(
                    "Particle system {:?} references missing material handle {}",
                    entity,
                    material_handle.index()
                );
                continue;
            };

            let mut material = *asset.material();
            material = if is_billboard {
                material.with_billboarding()
            } else {
                material.without_billboarding()
            };

            data.push(ParticleSceneData {
                entity,
                descriptor: ParticleSystemDescriptor {
                    mesh: mesh_handle,
                    material_handle,
                    material,
                    emitter,
                    system,
                    depth_state,
                    casts_shadows,
                },
                emitter_transform,
                existing_gpu_instance,
            });
        }

        data
    }

    fn ensure_entry(&mut self, data: &ParticleSceneData, renderer: &mut Renderer) -> u32 {
        use std::collections::hash_map::Entry;

        let descriptor = data.descriptor.clone();
        match self.systems.entry(data.entity) {
            Entry::Occupied(mut occupied) => {
                let entry = occupied.get_mut();
                if entry.descriptor != descriptor {
                    let gpu_index = entry.gpu_index;
                    *entry = Self::create_entry(
                        descriptor.clone(),
                        data.emitter_transform,
                        renderer,
                        gpu_index,
                    );
                }

                entry.descriptor = descriptor;
                Self::refresh_entry(entry, data.emitter_transform);
                entry.gpu_index
            }
            Entry::Vacant(vacant) => {
                let gpu_index = self.next_gpu_index;
                self.next_gpu_index = self.next_gpu_index.saturating_add(1);
                let entry =
                    Self::create_entry(descriptor, data.emitter_transform, renderer, gpu_index);
                vacant.insert(entry);
                gpu_index
            }
        }
    }

    fn create_entry(
        descriptor: ParticleSystemDescriptor,
        emitter_transform: Affine3A,
        renderer: &mut Renderer,
        gpu_index: u32,
    ) -> ParticleSystemEntry {
        let max_particles = Self::estimate_capacity(&descriptor);
        let device = renderer.get_device();
        let queue = renderer.get_queue();
        let behavior = BehaviorRuntime::new(&descriptor, device, max_particles);

        let render_mode = descriptor.system.render_mode.resolve(&descriptor.material);

        let mut system = GpuParticleSystem::new_with_mode(
            device,
            queue,
            renderer,
            max_particles,
            descriptor.material,
            behavior.as_behavior(),
            render_mode,
        );

        system.set_casts_shadows(descriptor.casts_shadows);
        system.set_depth_write_enabled(
            descriptor
                .depth_state
                .map(|state| state.depth_write)
                .unwrap_or(true),
        );

        let mut emitter = descriptor.emitter.to_runtime();
        emitter.set_world_transform(emitter_transform);
        system.add_emitter(emitter);

        ParticleSystemEntry {
            system,
            behavior,
            descriptor,
            gpu_index,
        }
    }

    fn refresh_entry(entry: &mut ParticleSystemEntry, emitter_transform: Affine3A) {
        entry
            .system
            .set_casts_shadows(entry.descriptor.casts_shadows);
        entry.system.set_depth_write_enabled(
            entry
                .descriptor
                .depth_state
                .map(|state| state.depth_write)
                .unwrap_or(true),
        );

        if let Some(emitter) = entry.system.emitters_mut().first_mut() {
            emitter.set_world_transform(emitter_transform);
        } else {
            let mut emitter = entry.descriptor.emitter.to_runtime();
            emitter.set_world_transform(emitter_transform);
            entry.system.add_emitter(emitter);
        }
    }

    fn update_gpu(&mut self, renderer: &mut Renderer, dt: f32) {
        if self.systems.is_empty() && self.effects.is_empty() {
            return;
        }
        // Paused (time_scale 0, or a stalled frame): nothing moves, and the dead-list
        // readback each update waits on the GPU.
        if dt <= 0.0
            && self
                .effects
                .values_mut()
                .all(|effect| effect.system.emitters_mut().is_empty())
            && self.systems.is_empty()
        {
            return;
        }

        let device = renderer.get_device();
        let queue = renderer.get_queue();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("ParticleRuntimeUpdate"),
        });

        for entry in self.systems.values_mut() {
            entry
                .behavior
                .prepare_update(&mut entry.system, device, queue, &mut encoder);
            entry.system.update(
                device,
                queue,
                &mut encoder,
                entry.behavior.as_behavior(),
                dt,
            );
        }
        for effect in self.effects.values_mut() {
            // An idle effect system costs a dispatch and a GPU readback wait per frame.
            if effect.system.active_particle_count() == 0 && effect.system.emitters_mut().is_empty()
            {
                continue;
            }
            effect
                .system
                .update(device, queue, &mut encoder, &effect.behavior, dt);
        }

        queue.submit(Some(encoder.finish()));
    }

    fn estimate_capacity(descriptor: &ParticleSystemDescriptor) -> u32 {
        let emitter = &descriptor.emitter;
        let burst = emitter.burst_count.unwrap_or(0);
        let lifetime_max = emitter.lifetime_range.max.max(0.0);
        let spawn_rate = emitter.spawn_rate.max(0.0);
        let mut estimate = if lifetime_max > 0.0 && spawn_rate > 0.0 {
            (spawn_rate * lifetime_max).ceil() as u32 + burst
        } else {
            burst
        };

        match &descriptor.system.behavior_config {
            ParticleBehaviorConfig::Boids(config) => {
                estimate = estimate.max(config.particle_count);
            }
            ParticleBehaviorConfig::OptimizedBoids(config) => {
                estimate = estimate.max(config.particle_count);
            }
            _ => {}
        }

        estimate.max(64)
    }

    fn transform_affine(transform: Transform) -> Affine3A {
        Affine3A::from_mat4(transform.matrix())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bursts_share_a_system_by_physics_and_blend() {
        let spark = ParticleBurst::default();
        let other_spot = ParticleBurst {
            position: Vec3::new(4.0, 0.0, 2.0),
            count: 5,
            ..ParticleBurst::default()
        };
        let falling = ParticleBurst {
            gravity: -9.8,
            ..ParticleBurst::default()
        };
        assert!(spark.effect_key() == other_spot.effect_key());
        assert!(spark.effect_key() != falling.effect_key());
    }

    #[test]
    fn burst_emitter_spawns_count_particles_at_the_position() {
        let burst = ParticleBurst {
            position: Vec3::new(1.0, 2.0, 3.0),
            count: 10,
            speed: (5.0, 5.0),
            flat: true,
            ..ParticleBurst::default()
        };
        let mut emitter = burst.emitter();
        let mut particles = Vec::new();
        emitter.emit_into(0.016, &mut particles);
        assert_eq!(particles.len(), 10);
        assert!(emitter.is_complete());
        for particle in &particles {
            let velocity = Vec3::from_array(particle.velocity);
            assert!(velocity.y.abs() < 1e-4, "a ring stays flat: {velocity}");
            assert!((velocity.length() - 5.0).abs() < 0.01);
            assert!(Vec3::from_array(particle.position).distance(burst.position) < 0.02);
        }
    }
}
