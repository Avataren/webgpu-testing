// scene/components.rs
// Pure hecs components - no custom entity system

use crate::asset::Handle;
use crate::asset::Mesh;
use crate::renderer::Material;
use crate::scene::Transform;
use glam::Vec3;

// ============================================================================
// Core Rendering Components
// ============================================================================

/// Transform component (position, rotation, scale)
#[derive(Debug, Clone, Copy)]
pub struct TransformComponent(pub Transform);

/// World-space transform (computed from hierarchy)
#[derive(Debug, Clone, Copy)]
pub struct WorldTransform(pub Transform);

/// Mesh component
#[derive(Debug, Clone, Copy)]
pub struct MeshComponent(pub Handle<Mesh>);

/// Material component
#[derive(Debug, Clone, Copy)]
pub struct MaterialComponent(pub Material);

/// Visibility component
#[derive(Debug, Clone, Copy)]
pub struct Visible(pub bool);

impl Default for Visible {
    fn default() -> Self {
        Self(true)
    }
}

// ============================================================================
// Lighting Components
// ============================================================================

/// Point light component
#[derive(Debug, Clone, Copy)]
pub struct PointLight {
    pub color: Vec3,
    pub intensity: f32,
    pub range: f32,
}

/// Directional light component
#[derive(Debug, Clone, Copy)]
pub struct DirectionalLight {
    pub color: Vec3,
    pub intensity: f32,
}

/// Spot light component
#[derive(Debug, Clone, Copy)]
pub struct SpotLight {
    pub color: Vec3,
    pub intensity: f32,
    pub inner_angle: f32,
    pub outer_angle: f32,
    pub range: f32,
}

// ============================================================================
// Utility Components
// ============================================================================

/// Name component for debugging
#[derive(Debug, Clone)]
pub struct Name(pub String);

impl Name {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }
}

// ============================================================================
// Animation Components
// ============================================================================

/// Rotation animation component
#[derive(Debug, Clone, Copy)]
pub struct RotateAnimation {
    pub axis: Vec3,
    pub speed: f32,
}

/// Orbit animation component
#[derive(Debug, Clone, Copy)]
pub struct OrbitAnimation {
    pub center: Vec3,
    pub radius: f32,
    pub speed: f32,
    pub offset: f32,
}

// ============================================================================
// Hierarchy Components (for future use)
// ============================================================================

/// Parent entity reference
#[derive(Debug, Clone, Copy)]
pub struct Parent(pub hecs::Entity);

/// List of children entities
#[derive(Debug, Clone)]
pub struct Children(pub Vec<hecs::Entity>);
