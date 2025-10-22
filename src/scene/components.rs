// scene/components.rs
// Pure hecs components - no custom entity system

use crate::asset::Handle;
use crate::asset::Mesh;
use crate::environment::{ColorGrading, Environment, HdrBackground};
use crate::renderer::{Material, Vertex};
use crate::scene::camera::{Camera, CameraProjection};
use crate::scene::Transform;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use wgpu::Color;

// ============================================================================
// Camera Component
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CameraComponent {
    pub projection: CameraProjection,
}

impl CameraComponent {
    pub fn perspective(fov_y_radians: f32, near: f32, far: f32) -> Self {
        Self {
            projection: CameraProjection::perspective(fov_y_radians, near, far),
        }
    }

    pub fn orthographic(left: f32, right: f32, bottom: f32, top: f32, near: f32, far: f32) -> Self {
        Self {
            projection: CameraProjection::orthographic(left, right, bottom, top, near, far),
        }
    }

    pub fn apply_to_camera(&self, camera: &mut Camera) {
        camera.set_projection(self.projection);
    }

    pub fn near(&self) -> f32 {
        self.projection.near()
    }

    pub fn far(&self) -> f32 {
        self.projection.far()
    }
}

impl Default for CameraComponent {
    fn default() -> Self {
        Self {
            projection: CameraProjection::default(),
        }
    }
}

impl From<Camera> for CameraComponent {
    fn from(camera: Camera) -> Self {
        Self {
            projection: camera.projection(),
        }
    }
}

impl From<&Camera> for CameraComponent {
    fn from(camera: &Camera) -> Self {
        Self {
            projection: camera.projection(),
        }
    }
}

// ============================================================================
// Environment Component
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentComponent {
    pub clear_color: [f32; 4],
    pub ambient_intensity: f32,
    #[serde(default)]
    pub color_grading: EnvironmentColorGrading,
    #[serde(default)]
    pub hdr: Option<EnvironmentHdrSettings>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentColorGrading {
    pub exposure: f32,
    pub saturation: f32,
    pub contrast: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentHdrSettings {
    pub enabled: bool,
    pub intensity: f32,
    pub path: Option<PathBuf>,
}

impl EnvironmentComponent {
    pub fn from_environment(environment: &Environment) -> Self {
        let color = environment.clear_color();
        let grading = environment.color_grading();
        let hdr = environment
            .hdr_background()
            .map(EnvironmentHdrSettings::from);

        Self {
            clear_color: [
                color.r as f32,
                color.g as f32,
                color.b as f32,
                color.a as f32,
            ],
            ambient_intensity: environment.ambient_intensity(),
            color_grading: EnvironmentColorGrading::from(grading),
            hdr,
        }
    }

    pub fn to_environment(&self) -> Environment {
        let mut environment = Environment::new(Color {
            r: self.clear_color[0] as f64,
            g: self.clear_color[1] as f64,
            b: self.clear_color[2] as f64,
            a: self.clear_color[3] as f64,
        });
        environment.set_ambient_intensity(self.ambient_intensity);
        environment.set_color_grading(self.color_grading.into());

        match self
            .hdr
            .as_ref()
            .and_then(EnvironmentHdrSettings::to_background)
        {
            Some(background) => environment.set_hdr_background(Some(background)),
            None => environment.set_hdr_background(None),
        }

        environment
    }

    pub fn apply_to_environment(&self, environment: &mut Environment) {
        *environment = self.to_environment();
    }

    pub fn update_from_environment(&mut self, environment: &Environment) {
        *self = Self::from_environment(environment);
    }

    pub fn hdr_settings_mut(&mut self) -> &mut EnvironmentHdrSettings {
        self.hdr.get_or_insert_with(EnvironmentHdrSettings::default)
    }
}

impl Default for EnvironmentComponent {
    fn default() -> Self {
        Self::from_environment(&Environment::default())
    }
}

impl EnvironmentColorGrading {
    pub fn apply_to(&self, grading: &mut ColorGrading) {
        grading.set_exposure(self.exposure);
        grading.set_saturation(self.saturation);
        grading.set_contrast(self.contrast);
    }
}

impl Default for EnvironmentColorGrading {
    fn default() -> Self {
        Self {
            exposure: 1.0,
            saturation: 1.0,
            contrast: 1.0,
        }
    }
}

impl From<ColorGrading> for EnvironmentColorGrading {
    fn from(grading: ColorGrading) -> Self {
        Self {
            exposure: grading.exposure(),
            saturation: grading.saturation(),
            contrast: grading.contrast(),
        }
    }
}

impl From<EnvironmentColorGrading> for ColorGrading {
    fn from(settings: EnvironmentColorGrading) -> Self {
        let mut grading = ColorGrading::default();
        grading.set_exposure(settings.exposure);
        grading.set_saturation(settings.saturation);
        grading.set_contrast(settings.contrast);
        grading
    }
}

impl EnvironmentHdrSettings {
    pub fn to_background(&self) -> Option<HdrBackground> {
        let path = self.path.clone()?;
        let mut background = HdrBackground::new(path);
        background.set_enabled(self.enabled);
        background.set_intensity(self.intensity);
        Some(background)
    }
}

impl Default for EnvironmentHdrSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            intensity: 1.0,
            path: None,
        }
    }
}

impl From<&HdrBackground> for EnvironmentHdrSettings {
    fn from(background: &HdrBackground) -> Self {
        Self {
            enabled: background.enabled(),
            intensity: background.intensity(),
            path: Some(background.path().to_path_buf()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn environment_component_round_trips_hdr_background() {
        let mut component = EnvironmentComponent::default();
        let hdr = component.hdr_settings_mut();
        hdr.path = Some(Path::new("assets/test_environment.hdr").to_path_buf());
        hdr.enabled = true;
        hdr.intensity = 2.5;

        let environment = component.to_environment();
        let background = environment
            .active_hdr_background()
            .expect("HDR background should be active");

        assert!(environment.is_hdr_enabled());
        assert_eq!(background.path(), Path::new("assets/test_environment.hdr"));
        assert!((background.intensity() - 2.5).abs() < f32::EPSILON);
    }
}

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
}

impl Billboard {
    pub fn new(orientation: BillboardOrientation) -> Self {
        Self {
            orientation,
            space: BillboardSpace::World,
            lit: false,
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

/// Axis-aligned bounding box for a mesh in local space.
#[derive(Debug, Clone, Copy)]
pub struct MeshBounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl MeshBounds {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    pub fn from_vertices(vertices: &[Vertex]) -> Option<Self> {
        if vertices.is_empty() {
            return None;
        }

        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);

        for vertex in vertices {
            let pos = Vec3::from_array(vertex.pos);
            min = min.min(pos);
            max = max.max(pos);
        }

        Some(Self { min, max })
    }
}

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

/// Marker component that indicates an entity is currently selected in the editor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelectedInEditor;

/// Persistent editor-scoped identifier used for undo/redo bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EditorEntityId(pub u128);

/// Source glTF asset used to spawn this entity.
#[derive(Debug, Clone)]
pub struct GltfSource(pub PathBuf);

/// Primitive index within a glTF node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GltfPrimitive(pub usize);

// ============================================================================
// GPU-driven instance components
// ============================================================================

/// Marker for instances whose transforms are driven entirely on the GPU.
///
/// The `index` refers to the absolute object buffer slot that the GPU system
/// updates each frame (including any base instance offset).
#[derive(Debug, Clone, Copy)]
pub struct GpuParticleInstance {
    pub index: u32,
}

// ============================================================================
// Particle System Components
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ParticleBehaviorPreset {
    #[default]
    Physics,
    Starfield,
    Boids,
    OptimizedBoids,
}

impl ParticleBehaviorPreset {
    pub const fn display_name(self) -> &'static str {
        match self {
            ParticleBehaviorPreset::Physics => "Physics",
            ParticleBehaviorPreset::Starfield => "Starfield",
            ParticleBehaviorPreset::Boids => "Boids",
            ParticleBehaviorPreset::OptimizedBoids => "Optimized Boids",
        }
    }

    pub const fn variants() -> [ParticleBehaviorPreset; 4] {
        [
            ParticleBehaviorPreset::Physics,
            ParticleBehaviorPreset::Starfield,
            ParticleBehaviorPreset::Boids,
            ParticleBehaviorPreset::OptimizedBoids,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ParticleSystemComponent {
    pub spawn_rate: f32,
    #[serde(default)]
    pub behavior: ParticleBehaviorPreset,
}

impl ParticleSystemComponent {
    pub const fn new(spawn_rate: f32, behavior: ParticleBehaviorPreset) -> Self {
        Self {
            spawn_rate,
            behavior,
        }
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
    pub shadow_size: f32,
}

impl DirectionalLight {
    pub const DEFAULT_SHADOW_SIZE: f32 = 30.0;
    pub const DEFAULT_SHADOW_DISTANCE: f32 = 30.0;

    pub fn new(color: Vec3, intensity: f32) -> Self {
        Self {
            color,
            intensity,
            shadow_size: Self::DEFAULT_SHADOW_SIZE,
        }
    }

    pub fn with_shadow_size(mut self, shadow_size: f32) -> Self {
        self.shadow_size = shadow_size;
        self
    }
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
