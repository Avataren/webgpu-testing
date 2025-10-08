// scene/components.rs
// Pure hecs components - no custom entity system

use crate::asset::Handle;
use crate::asset::Mesh;
use crate::renderer::Material;
use crate::scene::Transform;
use glam::Vec3;

// ============================================================================
// Billboard Components
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BillboardOrientation {
    /// Rotate freely so the quad faces the camera.
    FaceCamera,
    /// Only rotate around the world Y axis to face the camera.
    FaceCameraYAxis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BillboardProjection {
    /// Use the scene camera's projection (default behaviour).
    Perspective,
    /// Render in a dedicated orthographic pass that treats the transform
    /// translation as screen-space units, keeping the quad facing the camera.
    Orthographic,
}

impl Default for BillboardProjection {
    fn default() -> Self {
        Self::Perspective
    }
}

#[derive(Debug, Clone, Copy)]
pub enum BillboardSpace {
    /// Use the transform's translation directly in world space.
    World,
    /// Treat the transform's translation as an offset in view space
    /// (x = right, y = up, z = forward).
    View { offset: Vec3 },
}

impl Default for BillboardSpace {
    fn default() -> Self {
        Self::World
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Billboard {
    pub orientation: BillboardOrientation,
    pub space: BillboardSpace,
    pub lit: bool,
    pub projection: BillboardProjection,
}

impl Billboard {
    pub fn new(orientation: BillboardOrientation) -> Self {
        Self {
            orientation,
            space: BillboardSpace::World,
            lit: false,
            projection: BillboardProjection::default(),
        }
    }

    pub fn with_space(mut self, space: BillboardSpace) -> Self {
        self.space = space;
        self
    }

    pub fn with_lighting(mut self, enabled: bool) -> Self {
        self.lit = enabled;
        self
    }

    pub fn with_projection(mut self, projection: BillboardProjection) -> Self {
        self.projection = projection;
        self
    }
}

// ============================================================================
// Depth State Component
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DepthState {
    pub depth_test: bool,
    pub depth_write: bool,
}

impl DepthState {
    pub const fn new(depth_test: bool, depth_write: bool) -> Self {
        Self {
            depth_test,
            depth_write,
        }
    }
}

impl Default for DepthState {
    fn default() -> Self {
        Self {
            depth_test: true,
            depth_write: true,
        }
    }
}

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

/// Marker/flag component indicating a light should cast shadows
#[derive(Debug, Clone, Copy)]
pub struct CanCastShadow(pub bool);

impl Default for CanCastShadow {
    fn default() -> Self {
        Self(true)
    }
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
// glTF Metadata Components
// ============================================================================

/// Stores the originating glTF node index for an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GltfNode(pub usize);

/// Stores the originating glTF material index for an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GltfMaterial(pub usize);

// ============================================================================
// Hierarchy Components (for future use)
// ============================================================================

/// Parent entity reference
#[derive(Debug, Clone, Copy)]
pub struct Parent(pub hecs::Entity);

/// List of children entities
#[derive(Debug, Clone)]
pub struct Children(pub Vec<hecs::Entity>);
