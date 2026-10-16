//! Rigid-body physics, walking characters and spring-arm cameras, on
//! [Rapier](https://rapier.rs).
//!
//! Components describe what an entity is made of: [`Collider`] (its shape; on its own,
//! fixed scenery), [`RigidBody`] (how it moves), [`CharacterController`] (a walking
//! capsule steered by scripts) and [`SpringCamera`] (a third-person camera that follows
//! a target). They are saved with the scene and edited in the inspector, over the remote
//! API and from Lua.
//!
//! [`ScenePhysics`] is the simulation built from them, one per scene. It is derived
//! state: nothing in it is saved, it is rebuilt from the components when play starts
//! (or a scene is restored), it steps only in play mode after scripts have run, and it
//! writes moved bodies back to their transforms. Scripts read it (raycasts, velocities,
//! character state) during their hooks and change it through queued commands.

mod character;
mod components;
mod debug;
pub mod edit;
mod shapes;
mod spring_camera;
mod world;

pub use character::CharacterStatus;
pub use components::{
    BodyKind, CharacterController, Collider, ColliderShape, RigidBody, SpringCamera,
};
pub(crate) use debug::{selected_collider_gizmos, simulation_gizmos};
pub use spring_camera::SpringCameraStatus;
pub use world::{PhysicsCommand, PhysicsEvent, QueryOptions, RayHit, ScenePhysics};

use glam::{Quat, Vec3};
use rapier3d::prelude as rp;

// Rapier has its own glam; values cross over component by component.

pub(crate) fn to_rp(v: Vec3) -> rp::Vector {
    rp::Vector::new(v.x, v.y, v.z)
}

pub(crate) fn from_rp(v: rp::Vector) -> Vec3 {
    Vec3::new(v.x, v.y, v.z)
}

pub(crate) fn quat_to_rp(q: Quat) -> rp::Rotation {
    rp::Rotation::from_xyzw(q.x, q.y, q.z, q.w)
}

pub(crate) fn quat_from_rp(q: rp::Rotation) -> Quat {
    Quat::from_xyzw(q.x, q.y, q.z, q.w)
}

pub(crate) fn pose(translation: Vec3, rotation: Quat) -> rp::Pose {
    rp::Pose::from_parts(to_rp(translation), quat_to_rp(rotation.normalize()))
}
