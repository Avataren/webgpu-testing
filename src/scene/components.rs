// scene/components.rs
// Pure hecs components - no custom entity system

use crate::asset::{Handle, MaterialAsset, Mesh};
use crate::environment::{ColorGrading, Environment, HdrBackground};
use crate::gpu_particles::behaviors::{
    BoidsBehavior, OptimizedBoidsBehavior, PhysicsBehavior, StarfieldBehavior,
};
use crate::gpu_particles::{
    ColorGradient, EmissionShape, ParticleEmitter, ParticleRenderMode, SizeCurve,
};
use crate::renderer::primitives::PrimitiveMeshDescriptor;
use crate::renderer::Material;
use crate::renderer::Vertex;
use crate::scene::camera::{Camera, CameraProjection};
use crate::scene::Transform;
use glam::Vec3;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use wgpu::Color;

// ============================================================================
// Camera Component
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
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

/// Marker component identifying a built-in primitive mesh and its descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrimitiveMeshComponent {
    pub descriptor: PrimitiveMeshDescriptor,
}

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
pub struct MaterialComponent(pub Handle<MaterialAsset>);

/// Visibility component
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Visible(pub bool);

impl Default for Visible {
    fn default() -> Self {
        Self(true)
    }
}

/// Marker component that indicates an entity is currently selected in the editor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelectedInEditor;

/// Marker component that indicates an entity is an editor UI plugin.
/// Entities with this component are excluded from the scene hierarchy and scene serialization.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EditorPlugin;

/// Persistent editor-scoped identifier used for undo/redo bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EditorEntityId(pub u128);

impl EditorEntityId {
    /// Mixes the 128-bit identifier into a 64-bit value suitable for GPU picking.
    pub fn pick_identifier(self) -> u64 {
        let low = self.0 as u64;
        let high = (self.0 >> 64) as u64;
        low ^ high.wrapping_mul(0x9E3779B185EBCA87)
    }
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParticleRenderBlendMode {
    Auto,
    Opaque,
    AlphaBlend,
    Additive,
}

impl Default for ParticleRenderBlendMode {
    fn default() -> Self {
        Self::Auto
    }
}

impl ParticleRenderBlendMode {
    pub const fn display_name(self) -> &'static str {
        match self {
            ParticleRenderBlendMode::Auto => "Auto",
            ParticleRenderBlendMode::Opaque => "Opaque",
            ParticleRenderBlendMode::AlphaBlend => "Alpha Blend",
            ParticleRenderBlendMode::Additive => "Additive",
        }
    }

    pub const fn variants() -> [ParticleRenderBlendMode; 4] {
        [
            ParticleRenderBlendMode::Auto,
            ParticleRenderBlendMode::Opaque,
            ParticleRenderBlendMode::AlphaBlend,
            ParticleRenderBlendMode::Additive,
        ]
    }

    pub fn resolve(self, material: &Material) -> ParticleRenderMode {
        match self {
            ParticleRenderBlendMode::Auto => {
                if material.requires_separate_pass() {
                    ParticleRenderMode::AlphaBlend
                } else {
                    ParticleRenderMode::Opaque
                }
            }
            ParticleRenderBlendMode::Opaque => ParticleRenderMode::Opaque,
            ParticleRenderBlendMode::AlphaBlend => ParticleRenderMode::AlphaBlend,
            ParticleRenderBlendMode::Additive => ParticleRenderMode::Additive,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ParticleFloatRange {
    pub min: f32,
    pub max: f32,
}

impl ParticleFloatRange {
    pub const fn new(min: f32, max: f32) -> Self {
        Self { min, max }
    }
}

impl Default for ParticleFloatRange {
    fn default() -> Self {
        Self::new(0.0, 0.0)
    }
}

impl From<(f32, f32)> for ParticleFloatRange {
    fn from(range: (f32, f32)) -> Self {
        Self {
            min: range.0,
            max: range.1,
        }
    }
}

impl From<ParticleFloatRange> for (f32, f32) {
    fn from(range: ParticleFloatRange) -> Self {
        (range.min, range.max)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ParticleVec3Range {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl ParticleVec3Range {
    pub const fn new(min: [f32; 3], max: [f32; 3]) -> Self {
        Self { min, max }
    }

    pub const fn splat(value: f32) -> Self {
        Self {
            min: [value; 3],
            max: [value; 3],
        }
    }
}

impl Default for ParticleVec3Range {
    fn default() -> Self {
        Self::new([0.0; 3], [0.0; 3])
    }
}

impl From<(Vec3, Vec3)> for ParticleVec3Range {
    fn from(range: (Vec3, Vec3)) -> Self {
        Self {
            min: range.0.to_array(),
            max: range.1.to_array(),
        }
    }
}

impl From<ParticleVec3Range> for (Vec3, Vec3) {
    fn from(range: ParticleVec3Range) -> Self {
        (Vec3::from_array(range.min), Vec3::from_array(range.max))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleColorKeyframe {
    pub color: [f32; 4],
    pub time: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleColorGradient {
    pub keyframes: Vec<ParticleColorKeyframe>,
}

impl Default for ParticleColorGradient {
    fn default() -> Self {
        Self {
            keyframes: vec![ParticleColorKeyframe {
                color: [1.0, 1.0, 1.0, 1.0],
                time: 0.0,
            }],
        }
    }
}

impl From<ColorGradient> for ParticleColorGradient {
    fn from(gradient: ColorGradient) -> Self {
        Self {
            keyframes: gradient
                .keyframes
                .into_iter()
                .map(|(color, time)| ParticleColorKeyframe { color, time })
                .collect(),
        }
    }
}

impl From<ParticleColorGradient> for ColorGradient {
    fn from(gradient: ParticleColorGradient) -> Self {
        ColorGradient {
            keyframes: gradient
                .keyframes
                .into_iter()
                .map(|key| (key.color, key.time))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleSizeKeyframe {
    pub size: f32,
    pub time: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleSizeCurve {
    pub keyframes: Vec<ParticleSizeKeyframe>,
}

impl Default for ParticleSizeCurve {
    fn default() -> Self {
        Self {
            keyframes: vec![ParticleSizeKeyframe {
                size: 1.0,
                time: 0.0,
            }],
        }
    }
}

impl From<SizeCurve> for ParticleSizeCurve {
    fn from(curve: SizeCurve) -> Self {
        Self {
            keyframes: curve
                .keyframes
                .into_iter()
                .map(|(size, time)| ParticleSizeKeyframe { size, time })
                .collect(),
        }
    }
}

impl From<ParticleSizeCurve> for SizeCurve {
    fn from(curve: ParticleSizeCurve) -> Self {
        SizeCurve {
            keyframes: curve
                .keyframes
                .into_iter()
                .map(|key| (key.size, key.time))
                .collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParticleEmissionShape {
    Point,
    Sphere { radius: f32 },
    Box { half_extents: [f32; 3] },
    Cone { angle: f32, radius: f32 },
    Disc { radius: f32 },
    Ring { radius: f32, thickness: f32 },
    RadialBurst,
}

impl Default for ParticleEmissionShape {
    fn default() -> Self {
        Self::Point
    }
}

impl From<EmissionShape> for ParticleEmissionShape {
    fn from(shape: EmissionShape) -> Self {
        match shape {
            EmissionShape::Point => Self::Point,
            EmissionShape::Sphere { radius } => Self::Sphere { radius },
            EmissionShape::Box { half_extents } => Self::Box {
                half_extents: half_extents.to_array(),
            },
            EmissionShape::Cone { angle, radius } => Self::Cone { angle, radius },
            EmissionShape::Disc { radius } => Self::Disc { radius },
            EmissionShape::Ring { radius, thickness } => Self::Ring { radius, thickness },
            EmissionShape::RadialBurst => Self::RadialBurst,
        }
    }
}

impl From<ParticleEmissionShape> for EmissionShape {
    fn from(shape: ParticleEmissionShape) -> Self {
        match shape {
            ParticleEmissionShape::Point => EmissionShape::Point,
            ParticleEmissionShape::Sphere { radius } => EmissionShape::Sphere { radius },
            ParticleEmissionShape::Box { half_extents } => EmissionShape::Box {
                half_extents: Vec3::from_array(half_extents),
            },
            ParticleEmissionShape::Cone { angle, radius } => EmissionShape::Cone { angle, radius },
            ParticleEmissionShape::Disc { radius } => EmissionShape::Disc { radius },
            ParticleEmissionShape::Ring { radius, thickness } => {
                EmissionShape::Ring { radius, thickness }
            }
            ParticleEmissionShape::RadialBurst => EmissionShape::RadialBurst,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleEmitterComponent {
    pub spawn_rate: f32,
    #[serde(default)]
    pub burst_count: Option<u32>,
    #[serde(default)]
    pub position: [f32; 3],
    #[serde(default)]
    pub emission_shape: ParticleEmissionShape,
    #[serde(default)]
    pub initial_velocity_range: ParticleVec3Range,
    #[serde(default = "ParticleEmitterComponent::default_scale_range")]
    pub initial_scale_range: ParticleVec3Range,
    #[serde(default = "ParticleEmitterComponent::default_lifetime_range")]
    pub lifetime_range: ParticleFloatRange,
    #[serde(default)]
    pub color_gradient: ParticleColorGradient,
    #[serde(default)]
    pub size_curve: ParticleSizeCurve,
    #[serde(default)]
    pub radial_velocity: ParticleFloatRange,
    #[serde(default)]
    pub auto_respawn: bool,
}

impl ParticleEmitterComponent {
    const fn default_scale_range() -> ParticleVec3Range {
        ParticleVec3Range::splat(1.0)
    }

    const fn default_lifetime_range() -> ParticleFloatRange {
        ParticleFloatRange::new(5.0, 5.0)
    }

    pub fn to_runtime(&self) -> ParticleEmitter {
        let mut emitter = ParticleEmitter::new(Vec3::from_array(self.position), self.spawn_rate);
        emitter.burst_count = self.burst_count;
        emitter.emission_shape = self.emission_shape.clone().into();
        emitter.initial_velocity_range = self.initial_velocity_range.into();
        emitter.initial_scale_range = self.initial_scale_range.into();
        emitter.lifetime_range = self.lifetime_range.into();
        emitter.color_gradient = self.color_gradient.clone().into();
        emitter.size_curve = self.size_curve.clone().into();
        emitter.radial_velocity = self.radial_velocity.into();
        emitter.auto_respawn = self.auto_respawn;
        emitter
    }
}

impl Default for ParticleEmitterComponent {
    fn default() -> Self {
        Self {
            spawn_rate: 0.0,
            burst_count: None,
            position: [0.0; 3],
            emission_shape: ParticleEmissionShape::default(),
            initial_velocity_range: ParticleVec3Range::default(),
            initial_scale_range: Self::default_scale_range(),
            lifetime_range: Self::default_lifetime_range(),
            color_gradient: ParticleColorGradient::default(),
            size_curve: ParticleSizeCurve::default(),
            radial_velocity: ParticleFloatRange::default(),
            auto_respawn: false,
        }
    }
}

impl From<&ParticleEmitter> for ParticleEmitterComponent {
    fn from(emitter: &ParticleEmitter) -> Self {
        Self {
            spawn_rate: emitter.spawn_rate,
            burst_count: emitter.burst_count,
            position: emitter.position().to_array(),
            emission_shape: emitter.emission_shape.into(),
            initial_velocity_range: emitter.initial_velocity_range.into(),
            initial_scale_range: emitter.initial_scale_range.into(),
            lifetime_range: emitter.lifetime_range.into(),
            color_gradient: emitter.color_gradient.clone().into(),
            size_curve: emitter.size_curve.clone().into(),
            radial_velocity: emitter.radial_velocity.into(),
            auto_respawn: emitter.auto_respawn,
        }
    }
}

impl From<ParticleEmitterComponent> for ParticleEmitter {
    fn from(component: ParticleEmitterComponent) -> Self {
        component.to_runtime()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PhysicsBehaviorConfig {
    pub drag: f32,
    pub turbulence_strength: f32,
    pub turbulence_frequency: f32,
    pub gravity: [f32; 3],
    pub ground_level: f32,
    pub bounce_factor: f32,
    pub velocity_damping: f32,
}

impl Default for PhysicsBehaviorConfig {
    fn default() -> Self {
        let behavior = PhysicsBehavior::default();
        Self {
            drag: behavior.drag,
            turbulence_strength: behavior.turbulence_strength,
            turbulence_frequency: behavior.turbulence_frequency,
            gravity: behavior.gravity.to_array(),
            ground_level: behavior.ground_level,
            bounce_factor: behavior.bounce_factor,
            velocity_damping: behavior.velocity_damping,
        }
    }
}

impl From<&PhysicsBehavior> for PhysicsBehaviorConfig {
    fn from(behavior: &PhysicsBehavior) -> Self {
        Self {
            drag: behavior.drag,
            turbulence_strength: behavior.turbulence_strength,
            turbulence_frequency: behavior.turbulence_frequency,
            gravity: behavior.gravity.to_array(),
            ground_level: behavior.ground_level,
            bounce_factor: behavior.bounce_factor,
            velocity_damping: behavior.velocity_damping,
        }
    }
}

impl PhysicsBehaviorConfig {
    pub fn to_behavior(&self) -> PhysicsBehavior {
        PhysicsBehavior {
            drag: self.drag,
            turbulence_strength: self.turbulence_strength,
            turbulence_frequency: self.turbulence_frequency,
            gravity: Vec3::from_array(self.gravity),
            ground_level: self.ground_level,
            bounce_factor: self.bounce_factor,
            velocity_damping: self.velocity_damping,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StarfieldBehaviorConfig {
    pub near_plane: f32,
    pub far_plane: f32,
    pub far_reset_band: f32,
    pub field_half_size: f32,
    pub min_radius: f32,
}

impl Default for StarfieldBehaviorConfig {
    fn default() -> Self {
        Self {
            near_plane: 0.05,
            far_plane: 200.0,
            far_reset_band: 5.0,
            field_half_size: 60.0,
            min_radius: 0.25,
        }
    }
}

impl From<&StarfieldBehavior> for StarfieldBehaviorConfig {
    fn from(behavior: &StarfieldBehavior) -> Self {
        Self {
            near_plane: behavior.near_plane,
            far_plane: behavior.far_plane,
            far_reset_band: behavior.far_reset_band,
            field_half_size: behavior.field_half_size,
            min_radius: behavior.min_radius,
        }
    }
}

impl StarfieldBehaviorConfig {
    pub fn to_behavior(&self) -> StarfieldBehavior {
        StarfieldBehavior {
            near_plane: self.near_plane,
            far_plane: self.far_plane,
            far_reset_band: self.far_reset_band,
            field_half_size: self.field_half_size,
            min_radius: self.min_radius,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoidsBehaviorConfig {
    pub separation_radius: f32,
    pub alignment_radius: f32,
    pub cohesion_radius: f32,
    pub separation_weight: f32,
    pub alignment_weight: f32,
    pub cohesion_weight: f32,
    pub max_speed: f32,
    pub max_force: f32,
    pub bounds: f32,
    pub particle_count: u32,
}

impl Default for BoidsBehaviorConfig {
    fn default() -> Self {
        let behavior = BoidsBehavior::default();
        Self {
            separation_radius: behavior.separation_radius,
            alignment_radius: behavior.alignment_radius,
            cohesion_radius: behavior.cohesion_radius,
            separation_weight: behavior.separation_weight,
            alignment_weight: behavior.alignment_weight,
            cohesion_weight: behavior.cohesion_weight,
            max_speed: behavior.max_speed,
            max_force: behavior.max_force,
            bounds: behavior.bounds,
            particle_count: behavior.particle_count,
        }
    }
}

impl From<&BoidsBehavior> for BoidsBehaviorConfig {
    fn from(behavior: &BoidsBehavior) -> Self {
        Self {
            separation_radius: behavior.separation_radius,
            alignment_radius: behavior.alignment_radius,
            cohesion_radius: behavior.cohesion_radius,
            separation_weight: behavior.separation_weight,
            alignment_weight: behavior.alignment_weight,
            cohesion_weight: behavior.cohesion_weight,
            max_speed: behavior.max_speed,
            max_force: behavior.max_force,
            bounds: behavior.bounds,
            particle_count: behavior.particle_count,
        }
    }
}

impl BoidsBehaviorConfig {
    pub fn to_behavior(&self) -> BoidsBehavior {
        BoidsBehavior {
            separation_radius: self.separation_radius,
            alignment_radius: self.alignment_radius,
            cohesion_radius: self.cohesion_radius,
            separation_weight: self.separation_weight,
            alignment_weight: self.alignment_weight,
            cohesion_weight: self.cohesion_weight,
            max_speed: self.max_speed,
            max_force: self.max_force,
            bounds: self.bounds,
            particle_count: self.particle_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OptimizedBoidsBehaviorConfig {
    pub separation_radius: f32,
    pub alignment_radius: f32,
    pub cohesion_radius: f32,
    pub separation_weight: f32,
    pub alignment_weight: f32,
    pub cohesion_weight: f32,
    pub max_speed: f32,
    pub max_force: f32,
    pub bounds: f32,
    pub particle_count: u32,
}

impl Default for OptimizedBoidsBehaviorConfig {
    fn default() -> Self {
        let behavior = BoidsBehavior::default();
        Self {
            separation_radius: behavior.separation_radius,
            alignment_radius: behavior.alignment_radius,
            cohesion_radius: behavior.cohesion_radius,
            separation_weight: behavior.separation_weight,
            alignment_weight: behavior.alignment_weight,
            cohesion_weight: behavior.cohesion_weight,
            max_speed: behavior.max_speed,
            max_force: behavior.max_force,
            bounds: behavior.bounds,
            particle_count: behavior.particle_count,
        }
    }
}

impl From<&OptimizedBoidsBehavior> for OptimizedBoidsBehaviorConfig {
    fn from(behavior: &OptimizedBoidsBehavior) -> Self {
        Self {
            separation_radius: behavior.separation_radius,
            alignment_radius: behavior.alignment_radius,
            cohesion_radius: behavior.cohesion_radius,
            separation_weight: behavior.separation_weight,
            alignment_weight: behavior.alignment_weight,
            cohesion_weight: behavior.cohesion_weight,
            max_speed: behavior.max_speed,
            max_force: behavior.max_force,
            bounds: behavior.bounds,
            particle_count: behavior.particle_count,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ParticleBehaviorConfig {
    Physics(PhysicsBehaviorConfig),
    Starfield(StarfieldBehaviorConfig),
    Boids(BoidsBehaviorConfig),
    OptimizedBoids(OptimizedBoidsBehaviorConfig),
}

impl ParticleBehaviorConfig {
    pub fn from_preset(preset: ParticleBehaviorPreset) -> Self {
        match preset {
            ParticleBehaviorPreset::Physics => Self::Physics(PhysicsBehaviorConfig::default()),
            ParticleBehaviorPreset::Starfield => {
                Self::Starfield(StarfieldBehaviorConfig::default())
            }
            ParticleBehaviorPreset::Boids => Self::Boids(BoidsBehaviorConfig::default()),
            ParticleBehaviorPreset::OptimizedBoids => {
                Self::OptimizedBoids(OptimizedBoidsBehaviorConfig::default())
            }
        }
    }

    pub const fn preset(&self) -> ParticleBehaviorPreset {
        match self {
            ParticleBehaviorConfig::Physics(_) => ParticleBehaviorPreset::Physics,
            ParticleBehaviorConfig::Starfield(_) => ParticleBehaviorPreset::Starfield,
            ParticleBehaviorConfig::Boids(_) => ParticleBehaviorPreset::Boids,
            ParticleBehaviorConfig::OptimizedBoids(_) => ParticleBehaviorPreset::OptimizedBoids,
        }
    }

    pub fn ensure_variant(self, preset: ParticleBehaviorPreset) -> Self {
        if self.preset() == preset {
            self
        } else {
            Self::from_preset(preset)
        }
    }
}

impl Default for ParticleBehaviorConfig {
    fn default() -> Self {
        Self::from_preset(ParticleBehaviorPreset::default())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParticleSystemComponent {
    pub spawn_rate: f32,
    #[serde(default)]
    pub behavior: ParticleBehaviorPreset,
    #[serde(default)]
    pub behavior_config: ParticleBehaviorConfig,
    #[serde(default)]
    pub render_mode: ParticleRenderBlendMode,
}

impl ParticleSystemComponent {
    pub fn new(spawn_rate: f32, behavior: ParticleBehaviorPreset) -> Self {
        Self {
            spawn_rate,
            behavior,
            behavior_config: ParticleBehaviorConfig::from_preset(behavior),
            render_mode: ParticleRenderBlendMode::default(),
        }
    }

    pub fn set_behavior(&mut self, behavior: ParticleBehaviorPreset) {
        self.behavior = behavior;
        self.behavior_config = ParticleBehaviorConfig::from_preset(behavior);
    }

    pub fn with_behavior_config(mut self, config: ParticleBehaviorConfig) -> Self {
        self.behavior_config = config.ensure_variant(self.behavior);
        self
    }
}

impl Default for ParticleSystemComponent {
    fn default() -> Self {
        let behavior = ParticleBehaviorPreset::default();
        Self {
            spawn_rate: 10.0,
            behavior,
            behavior_config: ParticleBehaviorConfig::from_preset(behavior),
            render_mode: ParticleRenderBlendMode::default(),
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

impl Default for PointLight {
    fn default() -> Self {
        Self {
            color: Vec3::splat(1.0),
            intensity: 120.0,
            range: 12.0,
        }
    }
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

impl Default for DirectionalLight {
    fn default() -> Self {
        Self {
            color: Vec3::new(0.9, 0.95, 1.0),
            intensity: 3.0,
            shadow_size: Self::DEFAULT_SHADOW_SIZE,
        }
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

impl Default for SpotLight {
    fn default() -> Self {
        Self {
            color: Vec3::new(1.0, 0.95, 0.9),
            intensity: 15.0,
            inner_angle: 0.35,
            outer_angle: 0.6,
            range: 18.0,
        }
    }
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
