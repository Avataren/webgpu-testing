//! The simulation of one scene's main world, built from its physics components.

use std::collections::{BTreeSet, HashMap};
use std::hash::{Hash, Hasher};
use std::sync::Mutex;

use glam::{Mat4, Quat, Vec3};
use hecs::{Entity, World};
use rapier3d::parry::query::ShapeCastOptions;
use rapier3d::prelude as rp;

use super::character::{self, CharacterRuntime, CharacterStatus};
use super::components::{BodyKind, CharacterController, Collider, RigidBody, SpringCamera};
use super::shapes::{build_shape, ShapeSource};
use super::spring_camera::{self, CameraRuntime, SpringCameraStatus};
use super::{from_rp, pose, quat_from_rp, to_rp};
use crate::asset::Assets;
use crate::scene::components::{
    Children, MeshComponent, Name, Parent, PrimitiveMeshComponent, TransformComponent,
};
use crate::scene::skeletal::SkinnedMesh;
use crate::scene::transform::LookAtTarget;
use crate::scene::{InputState, Transform};

/// Longest single simulation step (s); longer frames run several.
const MAX_STEP: f32 = 1.0 / 60.0;
/// Most steps in one frame; a slower frame slows the simulation down instead.
const MAX_STEPS_PER_FRAME: u32 = 6;
/// A kinematic body moving from one pose to another over a frame's steps.
type Glide = (rp::RigidBodyHandle, (Vec3, Quat), (Vec3, Quat));

/// A transform that moved less than this since we wrote it was not moved by anyone else.
const MOVED_EPSILON: f32 = 1.0e-4;

/// A change scripts (or tools) ask of the simulation, applied at the next step.
#[derive(Debug, Clone, PartialEq)]
pub enum PhysicsCommand {
    /// Set a body's or character's velocity (m/s).
    SetVelocity {
        entity: Entity,
        velocity: Vec3,
    },
    /// Push a dynamic body (kg·m/s), or add to a character's velocity (m/s).
    ApplyImpulse {
        entity: Entity,
        impulse: Vec3,
    },
    /// Set a dynamic body's spin (radians per second about each axis).
    SetAngularVelocity {
        entity: Entity,
        velocity: Vec3,
    },
    ApplyTorqueImpulse {
        entity: Entity,
        impulse: Vec3,
    },
    /// Steer a character: a horizontal world-space direction whose length (0..1) is the
    /// share of its speed.
    CharacterMove {
        entity: Entity,
        direction: Vec3,
        run: bool,
    },
    CharacterJump {
        entity: Entity,
    },
    /// Face a character this way (radians about +Y; 0 faces +Z), at once or at its turn
    /// speed; `None` hands its facing back to its movement.
    CharacterFace {
        entity: Entity,
        yaw: Option<f32>,
        instant: bool,
    },
    /// Turn a spring camera (degrees).
    CameraRotate {
        entity: Entity,
        yaw: f32,
        pitch: f32,
    },
    /// Set a spring camera's view; `None` keeps a value. `target: Some(None)` goes back
    /// to the component's target.
    CameraSet {
        entity: Entity,
        yaw: Option<f32>,
        pitch: Option<f32>,
        distance: Option<f32>,
        target: Option<Option<Entity>>,
        snap: bool,
    },
    SetGravity(Vec3),
    SetDebugDraw(bool),
}

/// Something scripts hear about: `callback(self, data)` on `entity`'s script.
#[derive(Debug, Clone, PartialEq)]
pub struct PhysicsEvent {
    pub entity: Entity,
    pub callback: &'static str,
    pub data: serde_json::Value,
}

/// What a scene query hit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RayHit {
    pub entity: Entity,
    pub point: Vec3,
    pub normal: Vec3,
    pub distance: f32,
}

/// Which colliders a query sees.
#[derive(Debug, Clone, Default)]
pub struct QueryOptions {
    pub exclude: Vec<Entity>,
    /// See sensors (trigger volumes) too.
    pub sensors: bool,
    /// Skip dynamic bodies (props), seeing only scenery and characters.
    pub ignore_dynamic: bool,
}

/// How the simulation sees an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Dynamic,
    Fixed,
    Kinematic,
    Character,
}

/// Everything a body was built from; a change rebuilds it.
#[derive(Debug, Clone, PartialEq)]
struct Signature {
    role: Role,
    body: Option<RigidBody>,
    collider: Option<Collider>,
    /// Radius, height and step height of a character's capsule.
    character: Option<[f32; 3]>,
    /// World scale in tenths of a millimetre.
    scale: [i64; 3],
    geometry: u64,
}

struct Entry {
    body: rp::RigidBodyHandle,
    signature: Signature,
    /// The world pose we last put the body at (fixed and kinematic bodies follow the
    /// entity when it moves).
    placed: (Vec3, Quat),
    /// The local transform we last wrote (dynamic bodies and characters): anything else
    /// there means someone moved the entity.
    written: Option<Transform>,
}

/// Collision events Rapier reports during a step.
#[derive(Default)]
struct EventCollector(Mutex<Vec<rp::CollisionEvent>>);

impl rp::EventHandler for EventCollector {
    fn handle_collision_event(
        &self,
        _bodies: &rp::RigidBodySet,
        _colliders: &rp::ColliderSet,
        event: rp::CollisionEvent,
        _contact_pair: Option<&rp::ContactPair>,
    ) {
        if let Ok(mut events) = self.0.lock() {
            events.push(event);
        }
    }

    fn handle_contact_force_event(
        &self,
        _dt: f32,
        _bodies: &rp::RigidBodySet,
        _colliders: &rp::ColliderSet,
        _contact_pair: &rp::ContactPair,
        _total_force_magnitude: f32,
    ) {
    }

    fn handle_soft_body_tear_event(
        &self,
        _soft_bodies: &rp::SoftBodySet,
        _event: &rp::SoftBodyTearEvent,
    ) {
    }
}

/// The physics of a scene: a Rapier world mirroring the entities that have a
/// [`RigidBody`], [`Collider`] or [`CharacterController`], plus the [`SpringCamera`]s
/// that look at them.
///
/// It is rebuilt from the components (nothing in it is saved), steps only in play mode
/// after scripts have run, and writes moved bodies back to their transforms.
pub struct ScenePhysics {
    world: rp::PhysicsWorld,
    entries: HashMap<Entity, Entry>,
    characters: HashMap<Entity, CharacterRuntime>,
    cameras: HashMap<Entity, CameraRuntime>,
    commands: Vec<PhysicsCommand>,
    events: Vec<PhysicsEvent>,
    collector: EventCollector,
    debug_draw: bool,
    steps: u64,
}

impl Default for ScenePhysics {
    fn default() -> Self {
        Self::new()
    }
}

/// `entity`'s world transform from its own and its ancestors' local transforms (correct
/// even before this frame's transform propagation).
pub(crate) fn world_transform(world: &World, entity: Entity) -> Transform {
    let local = |entity| {
        world
            .get::<&TransformComponent>(entity)
            .map(|transform| transform.0)
            .unwrap_or_default()
    };
    let mut result = local(entity);
    let mut current = entity;
    // Bounded, in case a broken hierarchy has a cycle.
    for _ in 0..256 {
        let Ok(parent) = world.get::<&Parent>(current).map(|parent| parent.0) else {
            break;
        };
        result = local(parent).mul_transform(&result);
        current = parent;
    }
    result
}

/// The local transform that puts `entity` at `world_pose` (keeping its scale).
fn local_for_world(world: &World, entity: Entity, position: Vec3, rotation: Quat) -> Transform {
    let scale = world
        .get::<&TransformComponent>(entity)
        .map(|transform| transform.0.scale)
        .unwrap_or(Vec3::ONE);
    match world.get::<&Parent>(entity).map(|parent| parent.0) {
        Ok(parent) => {
            let parent_world = world_transform(world, parent);
            let inverse_rotation = parent_world.rotation.inverse();
            let parent_scale = parent_world.scale.max(Vec3::splat(1.0e-6));
            Transform {
                translation: inverse_rotation * (position - parent_world.translation)
                    / parent_scale,
                rotation: inverse_rotation * rotation,
                scale,
            }
        }
        Err(_) => Transform {
            translation: position,
            rotation,
            scale,
        },
    }
}

fn entity_bits(entity: Entity) -> u128 {
    u128::from(entity.to_bits().get())
}

fn entity_from_user_data(data: u128) -> Option<Entity> {
    Entity::from_bits(data as u64)
}

fn transform_changed(a: &Transform, b: &Transform) -> bool {
    a.translation.distance_squared(b.translation) > MOVED_EPSILON * MOVED_EPSILON
        || a.rotation.dot(b.rotation).abs() < 1.0 - 1.0e-6
        || a.scale.distance_squared(b.scale) > MOVED_EPSILON * MOVED_EPSILON
}

fn quantized_scale(scale: Vec3) -> [i64; 3] {
    let q = |v: f32| (v.abs() * 10_000.0).round() as i64;
    [q(scale.x), q(scale.y), q(scale.z)]
}

fn lerp_pose(from: (Vec3, Quat), to: (Vec3, Quat), t: f32) -> rp::Pose {
    pose(from.0.lerp(to.0, t), from.1.slerp(to.1, t))
}

/// A descendant mesh a `include_children` collider takes in.
pub(super) struct ChildMesh {
    pub mesh: MeshComponent,
    pub world: Mat4,
}

/// Meshes below `root` for an `include_children` collider: not skinned, and not under a
/// collider or body of their own.
pub(super) fn child_meshes(world: &World, root: Entity) -> Vec<ChildMesh> {
    let mut meshes = Vec::new();
    let mut stack: Vec<Entity> = world
        .get::<&Children>(root)
        .map(|children| children.0.clone())
        .unwrap_or_default();
    while let Some(entity) = stack.pop() {
        let has_own = world
            .satisfies::<hecs::Or<&Collider, hecs::Or<&RigidBody, &CharacterController>>>(entity)
            .unwrap_or(false);
        if has_own {
            continue;
        }
        if let Ok(mesh) = world.get::<&MeshComponent>(entity) {
            if !world.satisfies::<&SkinnedMesh>(entity).unwrap_or(false) {
                meshes.push(ChildMesh {
                    mesh: *mesh,
                    world: world_transform(world, entity).matrix(),
                });
            }
        }
        if let Ok(children) = world.get::<&Children>(entity) {
            stack.extend(children.0.iter().copied());
        }
    }
    meshes
}

impl ScenePhysics {
    pub fn new() -> Self {
        let world = rp::PhysicsWorld {
            gravity: rp::Vector::new(0.0, -9.81, 0.0),
            ..Default::default()
        };
        Self {
            world,
            entries: HashMap::new(),
            characters: HashMap::new(),
            cameras: HashMap::new(),
            commands: Vec::new(),
            events: Vec::new(),
            collector: EventCollector::default(),
            debug_draw: false,
            steps: 0,
        }
    }

    /// Forget the simulation; it is rebuilt from the components at the next step.
    pub fn reset(&mut self) {
        let debug_draw = self.debug_draw;
        *self = Self::new();
        self.debug_draw = debug_draw;
    }

    pub fn queue(&mut self, commands: impl IntoIterator<Item = PhysicsCommand>) {
        self.commands.extend(commands);
    }

    /// Script callbacks the last steps produced.
    pub fn take_events(&mut self) -> Vec<PhysicsEvent> {
        std::mem::take(&mut self.events)
    }

    pub fn debug_draw(&self) -> bool {
        self.debug_draw
    }

    pub fn set_debug_draw(&mut self, enabled: bool) {
        self.debug_draw = enabled;
    }

    pub fn gravity(&self) -> Vec3 {
        from_rp(self.world.gravity)
    }

    pub fn body_count(&self) -> usize {
        self.entries.len()
    }

    /// Simulation steps run since the last reset.
    pub fn steps(&self) -> u64 {
        self.steps
    }

    /// The entity a collider was built for.
    pub(crate) fn collider_entity(&self, handle: rp::ColliderHandle) -> Option<Entity> {
        self.world
            .colliders
            .get(handle)
            .and_then(|collider| entity_from_user_data(collider.user_data))
    }

    /// The role of the body a collider belongs to, for debug colours.
    pub(crate) fn collider_kind(
        &self,
        handle: rp::ColliderHandle,
    ) -> Option<(BodyKind, bool, bool)> {
        let entity = self.collider_entity(handle)?;
        let entry = self.entries.get(&entity)?;
        let sensor = self.world.colliders.get(handle)?.is_sensor();
        let kind = match entry.signature.role {
            Role::Dynamic => BodyKind::Dynamic,
            Role::Fixed => BodyKind::Fixed,
            Role::Kinematic | Role::Character => BodyKind::Kinematic,
        };
        Some((kind, sensor, entry.signature.role == Role::Character))
    }

    fn has_physics(world: &World) -> bool {
        world.query::<&Collider>().iter().next().is_some()
            || world.query::<&RigidBody>().iter().next().is_some()
            || world
                .query::<&CharacterController>()
                .iter()
                .next()
                .is_some()
            || world.query::<&SpringCamera>().iter().next().is_some()
    }

    /// Advance the simulation by `dt` seconds: build and update bodies from the
    /// components, step (kinematic bodies and characters glide to their targets), move
    /// characters by move-and-slide, write positions back, then place spring cameras.
    pub fn step(&mut self, world: &mut World, assets: &Assets, input: &InputState, dt: f32) {
        if dt <= 0.0 || !dt.is_finite() {
            return;
        }
        if self.entries.is_empty() && self.cameras.is_empty() && !Self::has_physics(world) {
            self.commands.clear();
            return;
        }
        let dt = dt.min(MAX_STEP * MAX_STEPS_PER_FRAME as f32);
        self.sync(world, assets);
        self.apply_commands();

        let steps = ((dt / MAX_STEP).ceil() as u32).clamp(1, MAX_STEPS_PER_FRAME);
        let step_dt = dt / steps as f32;

        // Kinematic bodies and characters glide to where they should be over the frame's
        // steps (characters to where last frame's move took them), so the bodies they
        // push see a smooth motion.
        let glides: Vec<Glide> = self
            .entries
            .values()
            .filter(|entry| matches!(entry.signature.role, Role::Kinematic | Role::Character))
            .filter_map(|entry| {
                let body = self.world.bodies.get(entry.body)?;
                let from = (from_rp(body.translation()), quat_from_rp(*body.rotation()));
                (from != entry.placed).then_some((entry.body, from, entry.placed))
            })
            .collect();
        for index in 0..steps {
            let t = (index + 1) as f32 / steps as f32;
            for (body, from, to) in &glides {
                if let Some(body) = self.world.bodies.get_mut(*body) {
                    body.set_next_kinematic_position(lerp_pose(*from, *to, t));
                }
            }
            self.world.integration_parameters.dt = step_dt;
            self.world.step_with_events(&(), &self.collector);
            self.steps += 1;
        }

        // Characters move once per frame (move-and-slide is not a solver), against the
        // world as it is after the step.
        let mut moved_characters: Vec<(Entity, Vec3, Quat)> = Vec::new();
        let mut character_entities: Vec<Entity> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.signature.role == Role::Character)
            .map(|(entity, _)| *entity)
            .collect();
        character_entities.sort_unstable_by_key(|entity| entity.to_bits());
        for entity in character_entities {
            let Ok(config) = world
                .get::<&CharacterController>(entity)
                .map(|c| (*c).clone())
            else {
                continue;
            };
            let body = self.entries[&entity].body;
            let feet = from_rp(self.world.bodies[body].translation());
            let state = self.characters.entry(entity).or_default();
            let result = character::step(
                &mut self.world,
                body,
                &config,
                state,
                feet,
                dt,
                |handle, colliders| {
                    colliders
                        .get(handle)
                        .and_then(|collider| entity_from_user_data(collider.user_data))
                },
            );
            if result.landed {
                self.events.push(PhysicsEvent {
                    entity,
                    callback: "on_landed",
                    data: serde_json::json!(state.landing_speed),
                });
            }
            if result.jumped {
                self.events.push(PhysicsEvent {
                    entity,
                    callback: "on_jumped",
                    data: serde_json::Value::Null,
                });
            }
            moved_characters.push((entity, result.position, result.rotation));
        }

        self.write_back(world, &moved_characters);
        self.feed_animation_trees(world);
        self.collect_collision_events();
        self.place_cameras(world, input, dt);
    }

    /// Create, rebuild, move and remove bodies so they match the components.
    fn sync(&mut self, world: &World, assets: &Assets) {
        let mut wanted = BTreeSet::new();
        wanted.extend(world.query::<&Collider>().iter().map(|(entity, _)| entity));
        wanted.extend(world.query::<&RigidBody>().iter().map(|(entity, _)| entity));
        wanted.extend(
            world
                .query::<&CharacterController>()
                .iter()
                .map(|(entity, _)| entity),
        );

        let stale: Vec<Entity> = self
            .entries
            .keys()
            .filter(|entity| !wanted.contains(entity))
            .copied()
            .collect();
        for entity in stale {
            self.remove_entry(entity);
            self.characters.remove(&entity);
        }

        for entity in wanted {
            let transform = world_transform(world, entity);
            let body = world
                .get::<&RigidBody>(entity)
                .ok()
                .map(|body| (*body).clone());
            let collider = world.get::<&Collider>(entity).ok().map(|c| (*c).clone());
            let character = world
                .get::<&CharacterController>(entity)
                .ok()
                .map(|controller| (*controller).clone());
            let role = match (&character, &body) {
                (Some(_), _) => Role::Character,
                (None, Some(body)) => match body.kind {
                    BodyKind::Dynamic => Role::Dynamic,
                    BodyKind::Fixed => Role::Fixed,
                    BodyKind::Kinematic => Role::Kinematic,
                },
                (None, None) => Role::Fixed,
            };
            let include_children = collider.as_ref().is_some_and(|c| c.include_children);
            let own_mesh = world.get::<&MeshComponent>(entity).ok().map(|mesh| *mesh);
            let children = if include_children && role != Role::Character {
                child_meshes(world, entity)
            } else {
                Vec::new()
            };
            let geometry = {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                own_mesh.map(|mesh| mesh.0.index()).hash(&mut hasher);
                for child in &children {
                    child.mesh.0.index().hash(&mut hasher);
                    for value in (transform_inverse(&transform) * child.world).to_cols_array() {
                        ((value * 10_000.0).round() as i64).hash(&mut hasher);
                    }
                }
                hasher.finish()
            };
            let signature = Signature {
                role,
                body,
                collider,
                character: character
                    .as_ref()
                    .map(|c| [c.radius, c.height, c.step_height]),
                scale: quantized_scale(transform.scale),
                geometry,
            };

            let rebuild = match self.entries.get(&entity) {
                Some(entry) => entry.signature != signature,
                None => true,
            };
            if rebuild {
                let carried = self.remove_entry(entity);
                self.create_entry(
                    world, assets, entity, signature, &transform, own_mesh, &children, carried,
                );
                if role == Role::Character {
                    let state = self.characters.entry(entity).or_default();
                    if carried.is_none() {
                        let (yaw, _, _) = transform.rotation.to_euler(glam::EulerRot::YXZ);
                        state.yaw = yaw;
                    }
                }
                continue;
            }

            let entry = self.entries.get_mut(&entity).expect("checked above");
            let pose_now = (transform.translation, transform.rotation);
            match role {
                Role::Fixed => {
                    if pose_now != entry.placed {
                        if let Some(body) = self.world.bodies.get_mut(entry.body) {
                            body.set_position(pose(pose_now.0, pose_now.1), true);
                        }
                        entry.placed = pose_now;
                    }
                }
                Role::Kinematic => entry.placed = pose_now,
                Role::Dynamic | Role::Character => {
                    let local = world
                        .get::<&TransformComponent>(entity)
                        .map(|t| t.0)
                        .unwrap_or_default();
                    if entry
                        .written
                        .is_none_or(|written| transform_changed(&written, &local))
                    {
                        // Moved by a script or the editor: teleport.
                        if let Some(body) = self.world.bodies.get_mut(entry.body) {
                            body.set_position(pose(pose_now.0, pose_now.1), true);
                        }
                        entry.placed = pose_now;
                        entry.written = Some(local);
                        if role == Role::Character {
                            if let Some(state) = self.characters.get_mut(&entity) {
                                let (yaw, _, _) = pose_now.1.to_euler(glam::EulerRot::YXZ);
                                state.yaw = yaw;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Remove an entity's body; returns its velocities, so a rebuilt body keeps moving.
    fn remove_entry(&mut self, entity: Entity) -> Option<(Vec3, Vec3)> {
        let entry = self.entries.remove(&entity)?;
        let body = self.world.remove_body_with_colliders(entry.body, true)?;
        Some((from_rp(body.linvel()), from_rp(body.angvel())))
    }

    #[allow(clippy::too_many_arguments)]
    fn create_entry(
        &mut self,
        world: &World,
        assets: &Assets,
        entity: Entity,
        signature: Signature,
        transform: &Transform,
        own_mesh: Option<MeshComponent>,
        children: &[ChildMesh],
        carried: Option<(Vec3, Vec3)>,
    ) {
        let position = pose(transform.translation, transform.rotation);
        let mut builder = match signature.role {
            Role::Dynamic => rp::RigidBodyBuilder::dynamic(),
            Role::Fixed => rp::RigidBodyBuilder::fixed(),
            Role::Kinematic | Role::Character => rp::RigidBodyBuilder::kinematic_position_based(),
        }
        .pose(position)
        .user_data(entity_bits(entity));
        if let Some(body) = signature
            .body
            .as_ref()
            .filter(|_| signature.role == Role::Dynamic)
        {
            let (linear, angular) =
                carried.unwrap_or((Vec3::from(body.initial_velocity), Vec3::ZERO));
            builder = builder
                .linear_damping(body.linear_damping.max(0.0))
                .angular_damping(body.angular_damping.max(0.0))
                .gravity_scale(body.gravity_scale)
                .ccd_enabled(body.ccd)
                .can_sleep(body.can_sleep)
                .linvel(to_rp(linear))
                .angvel(to_rp(angular));
            if body.lock_rotation {
                builder = builder.lock_rotations();
            }
        } else if signature.role == Role::Dynamic {
            let (linear, angular) = carried.unwrap_or_default();
            builder = builder.linvel(to_rp(linear)).angvel(to_rp(angular));
        }
        let handle = self.world.bodies.insert(builder);

        let dynamic = signature.role == Role::Dynamic;
        let shape = match &signature.character {
            Some([radius, height, step_height]) => {
                let (shape, center) = character::body_capsule(&CharacterController {
                    radius: *radius,
                    height: *height,
                    step_height: *step_height,
                    ..Default::default()
                });
                Some((shape, pose(Vec3::new(0.0, center, 0.0), Quat::IDENTITY)))
            }
            None => {
                let collider = signature.collider.clone().unwrap_or_default();
                let body_frame = transform_inverse(transform);
                let own = own_mesh.and_then(|mesh| assets.meshes.get(mesh.0));
                let mut source = ShapeSource::default();
                if let Some(mesh) = own {
                    source
                        .meshes
                        .push((mesh.data(), body_frame * transform.matrix()));
                }
                for child in children {
                    if let Some(mesh) = assets.meshes.get(child.mesh.0) {
                        source.meshes.push((mesh.data(), body_frame * child.world));
                    }
                }
                if children.is_empty() {
                    source.primitive = world
                        .get::<&PrimitiveMeshComponent>(entity)
                        .ok()
                        .map(|primitive| primitive.descriptor)
                        .filter(|_| own.is_some());
                }
                let built = build_shape(&collider, transform.scale, &source, dynamic);
                if built.is_none() {
                    log::warn!(
                        "{}: cannot build its collider ({}); it does not collide",
                        entity_label(world, entity),
                        collider.shape.kind_name()
                    );
                }
                built
            }
        };

        if let Some((shape, offset)) = shape {
            let config = signature.collider.clone().unwrap_or_default();
            let mut builder = rp::ColliderBuilder::new(shape)
                .position(offset)
                .user_data(entity_bits(entity))
                .active_events(rp::ActiveEvents::COLLISION_EVENTS);
            if signature.character.is_some() {
                builder = builder.friction(0.0);
            } else {
                builder = builder
                    .friction(config.friction.max(0.0))
                    .restitution(config.restitution.clamp(0.0, 1.0))
                    .density(config.density.max(0.0))
                    .sensor(config.sensor);
                if config.sensor {
                    // Sensors notice characters (kinematic) too, not only dynamic bodies;
                    // scenery touching scenery is never news.
                    builder = builder.active_collision_types(
                        rp::ActiveCollisionTypes::all() - rp::ActiveCollisionTypes::FIXED_FIXED,
                    );
                }
                if let Some(mass) = signature.body.as_ref().map(|b| b.mass).filter(|m| *m > 0.0) {
                    builder = builder.mass(mass);
                }
            }
            self.world
                .colliders
                .insert_with_parent(builder, handle, &mut self.world.bodies);
        }

        let written = matches!(signature.role, Role::Dynamic | Role::Character).then(|| {
            world
                .get::<&TransformComponent>(entity)
                .map(|t| t.0)
                .unwrap_or_default()
        });
        self.entries.insert(
            entity,
            Entry {
                body: handle,
                signature,
                placed: (transform.translation, transform.rotation),
                written,
            },
        );
    }

    fn apply_commands(&mut self) {
        for command in std::mem::take(&mut self.commands) {
            match command {
                PhysicsCommand::SetVelocity { entity, velocity } => {
                    if let Some(state) = self.characters.get_mut(&entity) {
                        state.velocity_override = Some(velocity);
                    } else if let Some(body) = self.dynamic_body_mut(entity) {
                        body.set_linvel(to_rp(velocity), true);
                    }
                }
                PhysicsCommand::ApplyImpulse { entity, impulse } => {
                    if let Some(state) = self.characters.get_mut(&entity) {
                        state.impulse += impulse;
                    } else if let Some(body) = self.dynamic_body_mut(entity) {
                        body.apply_impulse(to_rp(impulse), true);
                    }
                }
                PhysicsCommand::SetAngularVelocity { entity, velocity } => {
                    if let Some(body) = self.dynamic_body_mut(entity) {
                        body.set_angvel(to_rp(velocity), true);
                    }
                }
                PhysicsCommand::ApplyTorqueImpulse { entity, impulse } => {
                    if let Some(body) = self.dynamic_body_mut(entity) {
                        body.apply_torque_impulse(to_rp(impulse), true);
                    }
                }
                PhysicsCommand::CharacterMove {
                    entity,
                    direction,
                    run,
                } => {
                    let state = self.characters.entry(entity).or_default();
                    state.move_input = Vec3::new(direction.x, 0.0, direction.z);
                    state.run = run;
                }
                PhysicsCommand::CharacterJump { entity } => {
                    // The buffer length comes from the component at the next step.
                    self.characters.entry(entity).or_default().jump_buffer = f32::INFINITY;
                }
                PhysicsCommand::CharacterFace {
                    entity,
                    yaw,
                    instant,
                } => {
                    self.characters.entry(entity).or_default().facing =
                        yaw.map(|yaw| (yaw, instant));
                }
                PhysicsCommand::CameraRotate { entity, yaw, pitch } => {
                    let state = self.cameras.entry(entity).or_default();
                    state.yaw += yaw;
                    state.pitch += pitch;
                }
                PhysicsCommand::CameraSet {
                    entity,
                    yaw,
                    pitch,
                    distance,
                    target,
                    snap,
                } => {
                    let state = self.cameras.entry(entity).or_default();
                    if !state.initialized
                        && (yaw.is_some() || pitch.is_some() || distance.is_some())
                    {
                        // Values from a script before the camera's first frame win over
                        // the component's starting values.
                        state.initialized = true;
                        state.extent = 1.0;
                        state.snap = true;
                        state.distance = distance.unwrap_or(4.5);
                    }
                    if let Some(yaw) = yaw {
                        state.yaw = yaw;
                    }
                    if let Some(pitch) = pitch {
                        state.pitch = pitch;
                    }
                    if let Some(distance) = distance {
                        state.distance = distance;
                    }
                    if let Some(target) = target {
                        state.target = target;
                    }
                    state.snap |= snap;
                }
                PhysicsCommand::SetGravity(gravity) => self.world.gravity = to_rp(gravity),
                PhysicsCommand::SetDebugDraw(enabled) => self.debug_draw = enabled,
            }
        }
    }

    fn dynamic_body_mut(&mut self, entity: Entity) -> Option<&mut rp::RigidBody> {
        let entry = self.entries.get(&entity)?;
        self.world
            .bodies
            .get_mut(entry.body)
            .filter(|body| body.is_dynamic())
    }

    fn write_back(&mut self, world: &mut World, characters: &[(Entity, Vec3, Quat)]) {
        let mut moves: Vec<(Entity, Vec3, Quat)> = characters.to_vec();
        for (entity, entry) in &self.entries {
            if entry.signature.role != Role::Dynamic {
                continue;
            }
            let Some(body) = self.world.bodies.get(entry.body) else {
                continue;
            };
            if body.is_sleeping() && entry.written.is_some() {
                continue;
            }
            moves.push((
                *entity,
                from_rp(body.translation()),
                quat_from_rp(*body.rotation()),
            ));
        }
        for (entity, position, rotation) in moves {
            let local = local_for_world(world, entity, position, rotation);
            if let Ok(mut transform) = world.get::<&mut TransformComponent>(entity) {
                transform.0 = local;
            }
            if let Some(entry) = self.entries.get_mut(&entity) {
                entry.written = Some(local);
                entry.placed = (position, rotation);
            }
        }
    }

    /// Hand each animation tree that follows a character (`from_character`) the
    /// character's movement: `speed`, `forward_speed` and `side_speed` (relative to its
    /// facing), `vertical_speed`, `grounded`, `air_time`, `landing_speed`, and the
    /// triggers `jumped` and `landed`. The trees use them at the next animation update.
    fn feed_animation_trees(&self, world: &mut World) {
        use crate::scene::skeletal::AnimationTree;
        if self.characters.is_empty() {
            return;
        }
        let trees: Vec<Entity> = world
            .query::<&AnimationTree>()
            .iter()
            .filter(|(_, tree)| tree.from_character)
            .map(|(entity, _)| entity)
            .collect();
        for entity in trees {
            // The character: the tree's entity, or the nearest above it.
            let mut current = Some(entity);
            let mut character = None;
            for _ in 0..64 {
                let Some(candidate) = current else { break };
                if self.characters.contains_key(&candidate) {
                    character = Some(candidate);
                    break;
                }
                current = world.get::<&Parent>(candidate).ok().map(|parent| parent.0);
            }
            let Some(state) = character.and_then(|character| self.characters.get(&character))
            else {
                continue;
            };
            let Ok(mut tree) = world.get::<&mut AnimationTree>(entity) else {
                continue;
            };
            let forward = Vec3::new(state.yaw.sin(), 0.0, state.yaw.cos());
            let right = Vec3::new(-state.yaw.cos(), 0.0, state.yaw.sin());
            let horizontal = Vec3::new(state.velocity.x, 0.0, state.velocity.z);
            tree.set("speed", horizontal.length());
            tree.set("forward_speed", horizontal.dot(forward));
            tree.set("side_speed", horizontal.dot(right));
            tree.set("vertical_speed", state.velocity.y);
            tree.set("grounded", if state.grounded { 1.0 } else { 0.0 });
            tree.set("air_time", state.air_time);
            tree.set("landing_speed", state.landing_speed);
            if state.just_landed {
                tree.trigger("landed");
            }
            if state.just_jumped {
                tree.trigger("jumped");
            }
        }
    }

    fn collect_collision_events(&mut self) {
        let events = match self.collector.0.lock() {
            Ok(mut events) => std::mem::take(&mut *events),
            Err(_) => return,
        };
        for event in events {
            let (a, b, started) = match event {
                rp::CollisionEvent::Started(a, b, _) => (a, b, true),
                rp::CollisionEvent::Stopped(a, b, _) => (a, b, false),
            };
            let (Some(entity_a), Some(entity_b)) =
                (self.collider_entity(a), self.collider_entity(b))
            else {
                continue;
            };
            let sensor = event.sensor();
            let callback = match (sensor, started) {
                (true, true) => "on_trigger_enter",
                (true, false) => "on_trigger_exit",
                (false, true) => "on_collision_enter",
                (false, false) => "on_collision_exit",
            };
            for (entity, other) in [(entity_a, entity_b), (entity_b, entity_a)] {
                self.events.push(PhysicsEvent {
                    entity,
                    callback,
                    data: serde_json::json!(other.to_bits().get() as i64),
                });
            }
        }
    }

    /// The entity a spring camera follows: the script's choice, else the named entity,
    /// else its parent.
    fn camera_target(
        &self,
        world: &World,
        entity: Entity,
        config: &SpringCamera,
    ) -> Option<Entity> {
        if let Some(target) = self
            .cameras
            .get(&entity)
            .and_then(|state| state.target)
            .filter(|target| world.contains(*target))
        {
            return Some(target);
        }
        if !config.target.is_empty() {
            return world
                .query::<&Name>()
                .iter()
                .filter(|(candidate, _)| *candidate != entity)
                .find(|(_, name)| name.0 == config.target)
                .map(|(candidate, _)| candidate);
        }
        world.get::<&Parent>(entity).ok().map(|parent| parent.0)
    }

    fn place_cameras(&mut self, world: &mut World, input: &InputState, dt: f32) {
        let cameras: Vec<(Entity, SpringCamera)> = world
            .query::<&SpringCamera>()
            .iter()
            .map(|(entity, camera)| (entity, camera.clone()))
            .collect();
        self.cameras
            .retain(|entity, _| cameras.iter().any(|(camera, _)| camera == entity));
        for (entity, config) in cameras {
            let target = self.camera_target(world, entity, &config);
            let Some(target) = target else {
                continue;
            };
            let target_position = world_transform(world, target).translation;
            let exclude = self.entries.get(&target).map(|entry| entry.body);
            let state = self.cameras.entry(entity).or_default();
            let placement = spring_camera::update(
                &self.world,
                &config,
                state,
                target_position,
                exclude,
                Some(input),
                input.cursor_locked(),
                dt,
            );
            let local = local_for_world(world, entity, placement.position, placement.rotation);
            if let Ok(mut transform) = world.get::<&mut TransformComponent>(entity) {
                transform.0 = local;
            }
            // Keeps directional shadows centred on the character the camera looks at.
            let _ = world.insert_one(entity, LookAtTarget(placement.focus));
        }
    }

    /// The first collider a ray from `origin` along `direction` hits within `max_distance`.
    pub fn raycast(
        &self,
        origin: Vec3,
        direction: Vec3,
        max_distance: f32,
        options: &QueryOptions,
    ) -> Option<RayHit> {
        let direction = direction.try_normalize()?;
        let ray = rp::Ray::new(to_rp(origin), to_rp(direction));
        let predicate = |_: rp::ColliderHandle, collider: &rp::Collider| {
            entity_from_user_data(collider.user_data)
                .is_some_and(|entity| !options.exclude.contains(&entity))
        };
        let filter = query_filter(options).predicate(&predicate);
        let (handle, hit) = self
            .world
            .query_pipeline_with_filter(filter)
            .cast_ray_and_get_normal(&ray, max_distance.max(0.0), true)?;
        Some(RayHit {
            entity: self.collider_entity(handle)?,
            point: from_rp(ray.point_at(hit.time_of_impact)),
            normal: from_rp(hit.normal),
            distance: hit.time_of_impact,
        })
    }

    /// The first collider a ball of `radius` swept from `origin` along `direction` hits.
    pub fn spherecast(
        &self,
        origin: Vec3,
        direction: Vec3,
        radius: f32,
        max_distance: f32,
        options: &QueryOptions,
    ) -> Option<RayHit> {
        let direction = direction.try_normalize()?;
        let predicate = |_: rp::ColliderHandle, collider: &rp::Collider| {
            entity_from_user_data(collider.user_data)
                .is_some_and(|entity| !options.exclude.contains(&entity))
        };
        let filter = query_filter(options).predicate(&predicate);
        let ball = rp::Ball::new(radius.max(1.0e-3));
        let cast = ShapeCastOptions {
            max_time_of_impact: max_distance.max(0.0),
            target_distance: 0.0,
            stop_at_penetration: true,
            compute_impact_geometry_on_penetration: true,
        };
        let (handle, hit) = self.world.query_pipeline_with_filter(filter).cast_shape(
            &pose(origin, Quat::IDENTITY),
            to_rp(direction),
            &ball,
            cast,
        )?;
        let center = origin + direction * hit.time_of_impact;
        let normal = -from_rp(hit.normal2);
        Some(RayHit {
            entity: self.collider_entity(handle)?,
            point: center - normal.normalize_or(-direction) * ball.radius,
            normal,
            distance: hit.time_of_impact,
        })
    }

    /// Entities whose colliders overlap a ball at `center`.
    pub fn overlap_sphere(&self, center: Vec3, radius: f32, options: &QueryOptions) -> Vec<Entity> {
        let ball = rp::Ball::new(radius.max(1.0e-3));
        let filter = query_filter(options);
        let mut entities: Vec<Entity> = self
            .world
            .query_pipeline_with_filter(filter)
            .intersect_shape(pose(center, Quat::IDENTITY), &ball)
            .filter_map(|(_, collider)| entity_from_user_data(collider.user_data))
            .filter(|entity| !options.exclude.contains(entity))
            .collect();
        entities.sort_unstable_by_key(|entity| entity.to_bits());
        entities.dedup();
        entities
    }

    /// A body's or character's velocity (m/s).
    pub fn velocity(&self, entity: Entity) -> Option<Vec3> {
        if let Some(state) = self.characters.get(&entity) {
            return Some(state.velocity);
        }
        let entry = self.entries.get(&entity)?;
        self.world
            .bodies
            .get(entry.body)
            .map(|body| from_rp(body.linvel()))
    }

    pub fn angular_velocity(&self, entity: Entity) -> Option<Vec3> {
        let entry = self.entries.get(&entity)?;
        self.world
            .bodies
            .get(entry.body)
            .map(|body| from_rp(body.angvel()))
    }

    /// Mass of a body (kg).
    pub fn mass(&self, entity: Entity) -> Option<f32> {
        let entry = self.entries.get(&entity)?;
        self.world.bodies.get(entry.body).map(|body| body.mass())
    }

    pub fn character_status(&self, entity: Entity) -> Option<CharacterStatus> {
        self.characters.get(&entity).map(CharacterRuntime::status)
    }

    pub fn spring_camera_status(
        &self,
        world: &World,
        entity: Entity,
    ) -> Option<SpringCameraStatus> {
        let config = world.get::<&SpringCamera>(entity).ok()?;
        let state = self.cameras.get(&entity)?;
        Some(state.status(self.camera_target(world, entity, &config)))
    }

    /// The collider handles of every simulated entity with its body's pose, for debug
    /// drawing.
    pub(crate) fn colliders(&self) -> impl Iterator<Item = (rp::ColliderHandle, &rp::Collider)> {
        self.world.colliders.iter()
    }
}

fn query_filter(options: &QueryOptions) -> rp::QueryFilter<'_> {
    let mut flags = rp::QueryFilterFlags::empty();
    if !options.sensors {
        flags |= rp::QueryFilterFlags::EXCLUDE_SENSORS;
    }
    if options.ignore_dynamic {
        flags |= rp::QueryFilterFlags::EXCLUDE_DYNAMIC;
    }
    rp::QueryFilter::from(flags)
}

/// Inverse of the rotation and translation of `transform` (its scale is left in the
/// shapes, not the body).
fn transform_inverse(transform: &Transform) -> Mat4 {
    Mat4::from_rotation_translation(transform.rotation, transform.translation).inverse()
}

fn entity_label(world: &World, entity: Entity) -> String {
    world
        .get::<&Name>(entity)
        .map(|name| format!("{:?}", name.0))
        .unwrap_or_else(|_| format!("entity {}", entity.to_bits().get()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::physics::ColliderShape;

    const DT: f32 = 1.0 / 60.0;

    fn boxed(size: [f32; 3]) -> Collider {
        Collider {
            shape: ColliderShape::Box { size },
            ..Default::default()
        }
    }

    fn at(position: Vec3) -> TransformComponent {
        TransformComponent(Transform::from_translation(position))
    }

    /// A 40 m floor whose top is at y = 0.
    fn floor(world: &mut World) -> Entity {
        world.spawn((at(Vec3::new(0.0, -0.5, 0.0)), boxed([40.0, 1.0, 40.0])))
    }

    fn run(physics: &mut ScenePhysics, world: &mut World, seconds: f32) {
        let assets = Assets::default();
        let input = InputState::new();
        for _ in 0..(seconds / DT).round() as usize {
            physics.step(world, &assets, &input, DT);
        }
    }

    fn position(world: &World, entity: Entity) -> Vec3 {
        world
            .get::<&TransformComponent>(entity)
            .unwrap()
            .0
            .translation
    }

    #[test]
    fn dynamic_bodies_fall_and_rest_on_the_floor() {
        let mut world = World::new();
        floor(&mut world);
        let ball = world.spawn((
            at(Vec3::new(0.0, 5.0, 0.0)),
            RigidBody::default(),
            Collider {
                shape: ColliderShape::Sphere { radius: 0.5 },
                ..Default::default()
            },
        ));
        let mut physics = ScenePhysics::new();
        run(&mut physics, &mut world, 0.5);
        let falling = position(&world, ball);
        assert!(falling.y < 4.0 && falling.y > 0.5, "{falling:?}");
        run(&mut physics, &mut world, 3.0);
        let rest = position(&world, ball);
        assert!((rest.y - 0.5).abs() < 0.05, "{rest:?}");
        assert_eq!(physics.body_count(), 2);
    }

    #[test]
    fn moving_an_entity_teleports_its_body() {
        let mut world = World::new();
        floor(&mut world);
        let crate_entity = world.spawn((
            at(Vec3::new(0.0, 0.5, 0.0)),
            RigidBody::default(),
            boxed([1.0; 3]),
        ));
        let mut physics = ScenePhysics::new();
        run(&mut physics, &mut world, 0.5);
        world
            .get::<&mut TransformComponent>(crate_entity)
            .unwrap()
            .0
            .translation = Vec3::new(10.0, 3.0, 0.0);
        run(&mut physics, &mut world, 2.0);
        let moved = position(&world, crate_entity);
        assert!(
            (moved.x - 10.0).abs() < 0.05 && (moved.y - 0.5).abs() < 0.05,
            "{moved:?}"
        );
    }

    #[test]
    fn characters_walk_jump_and_land() {
        let mut world = World::new();
        floor(&mut world);
        let hero = world.spawn((
            at(Vec3::new(0.0, 0.05, 0.0)),
            CharacterController::default(),
        ));
        let mut physics = ScenePhysics::new();
        run(&mut physics, &mut world, 0.3);
        assert!(physics.character_status(hero).unwrap().grounded);

        physics.queue([PhysicsCommand::CharacterMove {
            entity: hero,
            direction: Vec3::new(1.0, 0.0, 0.0),
            run: true,
        }]);
        run(&mut physics, &mut world, 1.0);
        let walked = position(&world, hero);
        let speed = CharacterController::default().run_speed;
        assert!(
            walked.x > speed * 0.8 && walked.x < speed * 1.05,
            "{walked:?}"
        );
        assert!(walked.y.abs() < 0.05, "{walked:?}");
        // It turned to face +X (its model's +Z).
        let yaw = physics.character_status(hero).unwrap().yaw;
        assert!((yaw - std::f32::consts::FRAC_PI_2).abs() < 0.05, "{yaw}");

        physics.queue([
            PhysicsCommand::CharacterMove {
                entity: hero,
                direction: Vec3::ZERO,
                run: false,
            },
            PhysicsCommand::CharacterJump { entity: hero },
        ]);
        run(&mut physics, &mut world, 0.25);
        let status = physics.character_status(hero).unwrap();
        assert!(!status.grounded);
        let peak_time = (2.0 * 1.3 / 22.0f32).sqrt();
        run(&mut physics, &mut world, peak_time - 0.25);
        let top = position(&world, hero).y;
        assert!((top - 1.3).abs() < 0.15, "jump peak {top}");
        run(&mut physics, &mut world, 1.0);
        let landed = physics.character_status(hero).unwrap();
        assert!(landed.grounded && position(&world, hero).y.abs() < 0.05);
        let events = physics.take_events();
        assert!(events.iter().any(|event| event.callback == "on_jumped"));
        assert!(events.iter().any(|event| event.callback == "on_landed"));
    }

    #[test]
    fn a_facing_from_a_script_holds_while_strafing() {
        let mut world = World::new();
        floor(&mut world);
        let hero = world.spawn((
            at(Vec3::new(0.0, 0.05, 0.0)),
            CharacterController::default(),
        ));
        let mut physics = ScenePhysics::new();
        // Face -Z and strafe toward +X: it slides sideways without turning.
        physics.queue([
            PhysicsCommand::CharacterFace {
                entity: hero,
                yaw: Some(std::f32::consts::PI),
                instant: true,
            },
            PhysicsCommand::CharacterMove {
                entity: hero,
                direction: Vec3::X,
                run: false,
            },
        ]);
        run(&mut physics, &mut world, 1.0);
        let status = physics.character_status(hero).unwrap();
        assert!(
            (status.yaw - std::f32::consts::PI).abs() < 1e-4,
            "{}",
            status.yaw
        );
        assert!(position(&world, hero).x > 1.5);
        // Handing the facing back turns it toward its movement again.
        physics.queue([PhysicsCommand::CharacterFace {
            entity: hero,
            yaw: None,
            instant: false,
        }]);
        run(&mut physics, &mut world, 0.5);
        let yaw = physics.character_status(hero).unwrap().yaw;
        assert!((yaw - std::f32::consts::FRAC_PI_2).abs() < 0.05, "{yaw}");
    }

    #[test]
    fn characters_climb_steps_but_not_walls() {
        let mut world = World::new();
        floor(&mut world);
        // A 0.25 m step at x = 2, and a 2 m wall at z = 3.
        world.spawn((at(Vec3::new(4.0, 0.125, 0.0)), boxed([4.0, 0.25, 4.0])));
        world.spawn((at(Vec3::new(0.0, 1.0, 3.5)), boxed([4.0, 2.0, 1.0])));
        let hero = world.spawn((
            at(Vec3::new(0.0, 0.05, 0.0)),
            CharacterController::default(),
        ));
        let mut physics = ScenePhysics::new();
        physics.queue([PhysicsCommand::CharacterMove {
            entity: hero,
            direction: Vec3::X,
            run: false,
        }]);
        run(&mut physics, &mut world, 2.0);
        let up = position(&world, hero);
        assert!(
            up.x > 2.5 && (up.y - 0.25).abs() < 0.06,
            "did not climb the step: {up:?}"
        );

        let walker = world.spawn((
            at(Vec3::new(-1.0, 0.05, 0.0)),
            CharacterController::default(),
        ));
        physics.queue([PhysicsCommand::CharacterMove {
            entity: walker,
            direction: Vec3::Z,
            run: true,
        }]);
        run(&mut physics, &mut world, 2.0);
        let blocked = position(&world, walker);
        assert!(
            blocked.z < 3.0 - 0.25 && blocked.y < 0.1,
            "walked through the wall: {blocked:?}"
        );
    }

    /// Eight 0.25 m steps, 0.4 m deep, up to a 2 m landing (toward -Z from z = -2).
    fn staircase(world: &mut World) {
        for i in 0..8 {
            let h = 0.25 * (i + 1) as f32;
            world.spawn((
                at(Vec3::new(0.0, h / 2.0, -2.0 - 0.4 * i as f32)),
                boxed([3.0, h, 0.4]),
            ));
        }
        world.spawn((at(Vec3::new(0.0, 1.0, -6.0)), boxed([3.0, 2.0, 2.0])));
    }

    #[test]
    fn characters_climb_stairs_at_any_frame_rate_and_speed() {
        for (fps, run) in [
            (60.0f32, false),
            (60.0, true),
            (180.0, false),
            (180.0, true),
        ] {
            let mut world = World::new();
            floor(&mut world);
            staircase(&mut world);
            let hero = world.spawn((at(Vec3::ZERO), CharacterController::default()));
            let mut physics = ScenePhysics::new();
            physics.queue([PhysicsCommand::CharacterMove {
                entity: hero,
                direction: Vec3::NEG_Z,
                run,
            }]);
            let assets = Assets::default();
            let input = InputState::new();
            let mut top_reached = None;
            let mut landings = 0;
            for frame in 0..(fps * 5.0) as usize {
                physics.step(&mut world, &assets, &input, 1.0 / fps);
                let events = physics.take_events();
                if frame > 0 && top_reached.is_none() {
                    landings += events
                        .iter()
                        .filter(|event| event.callback == "on_landed")
                        .count();
                }
                let p = position(&world, hero);
                if top_reached.is_none() && p.z < -5.5 {
                    top_reached = Some((frame as f32 / fps, p.y));
                }
            }
            let (time, height) = top_reached.unwrap_or_else(|| {
                panic!(
                    "stuck on the stairs at {fps} fps (run {run}): {:?}",
                    position(&world, hero)
                )
            });
            // Every step is walked onto, never landed on: no airborne frames on the way.
            assert_eq!(landings, 0, "{fps} fps run {run}: landed on the steps");
            assert!(
                (height - 2.0).abs() < 0.05,
                "{fps} fps run {run}: height {height}"
            );
            // About the time walking the distance takes: the steps barely slow it down.
            let speed = if run { 6.0 } else { 2.2 };
            assert!(
                time < 5.5 / speed + 0.6,
                "{fps} fps run {run}: took {time} s"
            );
        }
    }

    #[test]
    fn slopes_up_to_the_limit_are_walkable() {
        for (degrees, climbable) in [(30.0f32, true), (60.0, false)] {
            let mut world = World::new();
            floor(&mut world);
            // A ramp rising toward -Z from z = -1 to 2 m high.
            let angle = degrees.to_radians();
            let length = 2.0 / angle.sin() + 0.3;
            let run = length * angle.cos();
            let mut ramp = Transform::from_translation(Vec3::new(
                0.0,
                length * 0.5 * angle.sin() - 0.15 * angle.cos(),
                -1.0 - run * 0.5,
            ));
            ramp.rotation = Quat::from_rotation_x(angle);
            world.spawn((TransformComponent(ramp), boxed([3.0, 0.3, length])));
            let hero = world.spawn((at(Vec3::ZERO), CharacterController::default()));
            let mut physics = ScenePhysics::new();
            physics.queue([PhysicsCommand::CharacterMove {
                entity: hero,
                direction: Vec3::NEG_Z,
                run: false,
            }]);
            // The highest it gets (it walks off the far end of the ramp).
            let mut top = 0.0f32;
            for _ in 0..40 {
                run_for(&mut physics, &mut world, 0.1);
                top = top.max(position(&world, hero).y);
            }
            if climbable {
                assert!(top > 1.5, "{degrees} degrees: only reached {top}");
            } else {
                assert!(top < 0.6, "{degrees} degrees: climbed to {top}");
            }
        }
    }

    fn run_for(physics: &mut ScenePhysics, world: &mut World, seconds: f32) {
        run(physics, world, seconds);
    }

    #[test]
    fn sensors_report_characters_and_queries_see_colliders() {
        let mut world = World::new();
        floor(&mut world);
        // A flat pad on the floor: the character's body reaches down to its feet.
        let zone = world.spawn((
            at(Vec3::new(3.0, 0.05, 0.0)),
            Collider {
                sensor: true,
                ..boxed([1.0, 0.1, 1.0])
            },
        ));
        let hero = world.spawn((
            at(Vec3::new(0.0, 0.05, 0.0)),
            CharacterController::default(),
        ));
        let mut physics = ScenePhysics::new();
        physics.queue([PhysicsCommand::CharacterMove {
            entity: hero,
            direction: Vec3::X,
            run: false,
        }]);
        run(&mut physics, &mut world, 1.6);
        let events = physics.take_events();
        assert!(
            events
                .iter()
                .any(|event| event.entity == zone && event.callback == "on_trigger_enter"),
            "{events:?}"
        );
        // Only the character: the floor the pad lies on is scenery, like the pad.
        assert!(
            events
                .iter()
                .filter(|event| event.entity == zone)
                .all(|event| event.data == serde_json::json!(hero.to_bits().get() as i64)),
            "{events:?}"
        );

        let hit = physics
            .raycast(
                Vec3::new(-5.0, 5.0, 0.0),
                Vec3::NEG_Y,
                20.0,
                &QueryOptions::default(),
            )
            .unwrap();
        assert!(
            (hit.point.y).abs() < 1e-3 && (hit.normal.y - 1.0).abs() < 1e-3,
            "{hit:?}"
        );
        // Sensors are skipped unless asked for.
        let through = physics.raycast(
            Vec3::new(3.0, 5.0, 0.0),
            Vec3::NEG_Y,
            20.0,
            &QueryOptions::default(),
        );
        assert_ne!(through.map(|hit| hit.entity), Some(zone));
        let sensor = physics.raycast(
            Vec3::new(3.0, 5.0, 0.0),
            Vec3::NEG_Y,
            20.0,
            &QueryOptions {
                sensors: true,
                ..Default::default()
            },
        );
        assert_eq!(sensor.map(|hit| hit.entity), Some(zone));
        let near = physics.overlap_sphere(position(&world, hero), 0.5, &QueryOptions::default());
        assert!(near.contains(&hero));
    }

    #[test]
    fn spring_cameras_follow_a_named_target() {
        let mut world = World::new();
        floor(&mut world);
        let hero = world.spawn((
            at(Vec3::new(0.0, 0.05, 0.0)),
            Name("Hero".into()),
            CharacterController::default(),
        ));
        let camera = world.spawn((
            at(Vec3::ZERO),
            SpringCamera {
                target: "Hero".into(),
                follow_lag: 0.0,
                vertical_lag: 0.0,
                pitch: 0.0,
                ..Default::default()
            },
        ));
        let mut physics = ScenePhysics::new();
        physics.queue([PhysicsCommand::CharacterMove {
            entity: hero,
            direction: Vec3::NEG_Z,
            run: false,
        }]);
        run(&mut physics, &mut world, 1.0);
        let hero_position = position(&world, hero);
        let camera_position = position(&world, camera);
        // Behind (+Z) the hero at the arm's length, level with the pivot.
        let expected = hero_position + Vec3::new(0.0, 1.5, 4.5);
        assert!(
            camera_position.distance(expected) < 0.1,
            "{camera_position:?} vs {expected:?}"
        );
        let status = physics.spring_camera_status(&world, camera).unwrap();
        assert_eq!(status.target, Some(hero));
        assert!(world.satisfies::<&LookAtTarget>(camera).unwrap());
    }
}
