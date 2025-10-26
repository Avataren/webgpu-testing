use std::collections::{HashMap, HashSet};

use glam::Vec3;
use hecs::{ComponentError, Entity};
use log::{debug, warn};

use super::system::{EditorContext, EditorSystem};
use wgpu_cube::asset::{Handle, Mesh};
use wgpu_cube::gpu_particles::behaviors::{
    BoidsBehavior, OptimizedBoidsBehavior, PhysicsBehavior, StarfieldBehavior,
};
use wgpu_cube::gpu_particles::{GpuParticleSystem, ParticleBehavior};
use wgpu_cube::renderer::{CustomRenderContext, Renderer};
use wgpu_cube::scene::components::{
    Billboard, CanCastShadow, DepthState, GpuParticleInstance, MaterialComponent, MeshComponent,
    ParticleEmitterComponent, ParticleSystemComponent, TransformComponent, WorldTransform,
};
use wgpu_cube::scene::transform::Transform;
use wgpu_cube::scene::Scene;

#[derive(Clone, PartialEq)]
struct ParticleSystemDescriptor {
    mesh: Handle<Mesh>,
    material: wgpu_cube::renderer::Material,
    emitter: ParticleEmitterComponent,
    system: ParticleSystemComponent,
    depth_state: Option<DepthState>,
    casts_shadows: bool,
}

struct ParticleSceneData {
    entity: Entity,
    descriptor: ParticleSystemDescriptor,
    emitter_origin: Vec3,
    existing_gpu_instance: Option<GpuParticleInstance>,
}

enum BehaviorRuntime {
    Physics(PhysicsBehavior),
    Starfield(StarfieldBehavior),
    Boids(BoidsBehavior),
    OptimizedBoids(OptimizedBoidsBehavior),
}

impl BehaviorRuntime {
    fn new(
        descriptor: &ParticleSystemDescriptor,
        device: &wgpu::Device,
        max_particles: u32,
    ) -> Self {
        match &descriptor.system.behavior_config {
            wgpu_cube::scene::components::ParticleBehaviorConfig::Physics(config) => {
                Self::Physics(config.to_behavior())
            }
            wgpu_cube::scene::components::ParticleBehaviorConfig::Starfield(config) => {
                Self::Starfield(config.to_behavior())
            }
            wgpu_cube::scene::components::ParticleBehaviorConfig::Boids(config) => {
                let mut behavior = config.to_behavior();
                behavior.set_particle_count(max_particles);
                Self::Boids(behavior)
            }
            wgpu_cube::scene::components::ParticleBehaviorConfig::OptimizedBoids(config) => {
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

                Self::OptimizedBoids(behavior)
            }
        }
    }

    fn as_behavior(&self) -> &dyn ParticleBehavior {
        match self {
            Self::Physics(behavior) => behavior,
            Self::Starfield(behavior) => behavior,
            Self::Boids(behavior) => behavior,
            Self::OptimizedBoids(behavior) => behavior,
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

#[derive(Default)]
pub(crate) struct EditorParticleSystem {
    systems: HashMap<Entity, ParticleSystemEntry>,
    next_gpu_index: u32,
}

impl EditorParticleSystem {
    pub(crate) fn render(&mut self, ctx: &mut CustomRenderContext<'_>) {
        for entry in self.systems.values_mut() {
            if let Some(mesh) = ctx.scene.assets.meshes.get(entry.descriptor.mesh) {
                entry.system.render(ctx, mesh);
            }
        }
    }

    pub(crate) fn has_shadow_casters(&self) -> bool {
        self.systems
            .values()
            .any(|entry| entry.system.casts_shadows())
    }

    fn sync_with_scene(&mut self, ctx: &mut wgpu_cube::app::GpuUpdateContext<'_>) {
        let data = Self::collect_scene_data(ctx.scene);
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

        self.update_gpu(ctx.renderer, ctx.dt as f32);
    }

    fn collect_scene_data(scene: &mut Scene) -> Vec<ParticleSceneData> {
        let mut data = Vec::new();

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
                let emitter_origin = transform
                    .map(|t| Self::transform_point(t, Vec3::from_array(emitter.position)))
                    .unwrap_or_else(|| Vec3::from_array(emitter.position));

                let mut material = material_component.0;
                material = if billboard.is_some() {
                    material.with_billboarding()
                } else {
                    material.without_billboarding()
                };

                data.push(ParticleSceneData {
                    entity,
                    descriptor: ParticleSystemDescriptor {
                        mesh: mesh.0,
                        material,
                        emitter: emitter.clone(),
                        system: (*system).clone(),
                        depth_state: depth_state.copied(),
                        casts_shadows: casts_shadow.map(|s| s.0).unwrap_or(false),
                    },
                    emitter_origin,
                    existing_gpu_instance: gpu_instance.copied(),
                });
            }
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
                        data.emitter_origin,
                        renderer,
                        gpu_index,
                    );
                }

                entry.descriptor = descriptor;
                Self::refresh_entry(entry, data.emitter_origin);
                entry.gpu_index
            }
            Entry::Vacant(vacant) => {
                let gpu_index = self.next_gpu_index;
                self.next_gpu_index = self.next_gpu_index.saturating_add(1);
                let entry =
                    Self::create_entry(descriptor, data.emitter_origin, renderer, gpu_index);
                vacant.insert(entry);
                gpu_index
            }
        }
    }

    fn create_entry(
        descriptor: ParticleSystemDescriptor,
        emitter_origin: Vec3,
        renderer: &mut Renderer,
        gpu_index: u32,
    ) -> ParticleSystemEntry {
        let max_particles = Self::estimate_capacity(&descriptor);
        let device = renderer.get_device();
        let queue = renderer.get_queue();
        let behavior = BehaviorRuntime::new(&descriptor, device, max_particles);

        let mut system = GpuParticleSystem::new(
            device,
            queue,
            renderer,
            max_particles,
            descriptor.material,
            behavior.as_behavior(),
        );

        system.set_casts_shadows(descriptor.casts_shadows);
        system.set_depth_write_enabled(
            descriptor
                .depth_state
                .map(|state| state.depth_write)
                .unwrap_or(true),
        );

        let mut emitter = descriptor.emitter.to_runtime();
        emitter.position = emitter_origin;
        system.add_emitter(emitter);

        ParticleSystemEntry {
            system,
            behavior,
            descriptor,
            gpu_index,
        }
    }

    fn refresh_entry(entry: &mut ParticleSystemEntry, emitter_origin: Vec3) {
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
            emitter.position = emitter_origin;
        } else {
            let mut emitter = entry.descriptor.emitter.to_runtime();
            emitter.position = emitter_origin;
            entry.system.add_emitter(emitter);
        }
    }

    fn update_gpu(&mut self, renderer: &mut Renderer, dt: f32) {
        if self.systems.is_empty() {
            return;
        }

        let device = renderer.get_device();
        let queue = renderer.get_queue();
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("EditorParticleSystemUpdate"),
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
            wgpu_cube::scene::components::ParticleBehaviorConfig::Boids(config) => {
                estimate = estimate.max(config.particle_count);
            }
            wgpu_cube::scene::components::ParticleBehaviorConfig::OptimizedBoids(config) => {
                estimate = estimate.max(config.particle_count);
            }
            _ => {}
        }

        estimate.max(64)
    }

    fn transform_point(transform: Transform, point: Vec3) -> Vec3 {
        transform.translation + transform.rotation * (transform.scale * point)
    }
}

impl EditorSystem for EditorParticleSystem {
    fn gpu_update<'app, 'ctx, 'scene>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'scene>) {
        let Some(gpu_ctx) = ctx.gpu_context_mut() else {
            return;
        };

        self.sync_with_scene(gpu_ctx);
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}
