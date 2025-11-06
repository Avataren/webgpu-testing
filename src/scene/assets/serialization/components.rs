use crate::scene::components::{
    Billboard, BillboardOrientation, BillboardSpace, DirectionalLight, MeshBounds,
    ParticleBehaviorConfig, ParticleBehaviorPreset, ParticleColorGradient, ParticleEmissionShape,
    ParticleEmitterComponent, ParticleFloatRange, ParticleRenderBlendMode, ParticleSizeCurve,
    ParticleSystemComponent, ParticleVec3Range, PointLight, SpotLight,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedParticleSystem {
    pub spawn_rate: f32,
    #[serde(default)]
    pub behavior: Option<ParticleBehaviorPreset>,
    #[serde(default)]
    pub behavior_config: Option<ParticleBehaviorConfig>,
    #[serde(default)]
    pub render_mode: ParticleRenderBlendMode,
}

impl From<&ParticleSystemComponent> for SerializedParticleSystem {
    fn from(component: &ParticleSystemComponent) -> Self {
        Self {
            spawn_rate: component.spawn_rate,
            behavior: Some(component.behavior),
            behavior_config: Some(component.behavior_config.clone()),
            render_mode: component.render_mode,
        }
    }
}

impl From<ParticleSystemComponent> for SerializedParticleSystem {
    fn from(component: ParticleSystemComponent) -> Self {
        Self::from(&component)
    }
}

impl From<SerializedParticleSystem> for ParticleSystemComponent {
    fn from(serialized: SerializedParticleSystem) -> Self {
        let preset = serialized.behavior.unwrap_or_default();
        let mut component = ParticleSystemComponent::new(serialized.spawn_rate, preset);
        if let Some(config) = serialized.behavior_config {
            component.behavior_config = config.ensure_variant(preset);
        }
        component.render_mode = serialized.render_mode;
        component
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedParticleBehavior {
    pub preset: ParticleBehaviorPreset,
    #[serde(default)]
    pub config: ParticleBehaviorConfig,
}

impl SerializedParticleBehavior {
    pub(crate) fn config_for_preset(&self) -> ParticleBehaviorConfig {
        self.config.clone().ensure_variant(self.preset)
    }

    pub fn apply_to_component(&self, component: &mut ParticleSystemComponent) {
        component.set_behavior(self.preset);
        component.behavior_config = self.config_for_preset();
    }

    pub fn into_component_with_spawn_rate(self, spawn_rate: f32) -> ParticleSystemComponent {
        let mut component = ParticleSystemComponent::new(spawn_rate, self.preset);
        component.behavior_config = self.config.ensure_variant(self.preset);
        component
    }
}

impl From<&ParticleSystemComponent> for SerializedParticleBehavior {
    fn from(component: &ParticleSystemComponent) -> Self {
        Self {
            preset: component.behavior,
            config: component.behavior_config.clone(),
        }
    }
}

impl From<ParticleSystemComponent> for SerializedParticleBehavior {
    fn from(component: ParticleSystemComponent) -> Self {
        Self::from(&component)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerializedParticleEmitter {
    pub spawn_rate: f32,
    #[serde(default)]
    pub burst_count: Option<u32>,
    #[serde(default)]
    pub position: [f32; 3],
    #[serde(default)]
    pub emission_shape: ParticleEmissionShape,
    #[serde(default)]
    pub initial_velocity_range: ParticleVec3Range,
    #[serde(default = "SerializedParticleEmitter::default_scale_range")]
    pub initial_scale_range: ParticleVec3Range,
    #[serde(default = "SerializedParticleEmitter::default_lifetime_range")]
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

impl SerializedParticleEmitter {
    const fn default_scale_range() -> ParticleVec3Range {
        ParticleVec3Range::splat(1.0)
    }

    const fn default_lifetime_range() -> ParticleFloatRange {
        ParticleFloatRange::new(5.0, 5.0)
    }
}

impl From<&ParticleEmitterComponent> for SerializedParticleEmitter {
    fn from(component: &ParticleEmitterComponent) -> Self {
        Self {
            spawn_rate: component.spawn_rate,
            burst_count: component.burst_count,
            position: component.position,
            emission_shape: component.emission_shape.clone(),
            initial_velocity_range: component.initial_velocity_range,
            initial_scale_range: component.initial_scale_range,
            lifetime_range: component.lifetime_range,
            color_gradient: component.color_gradient.clone(),
            size_curve: component.size_curve.clone(),
            radial_velocity: component.radial_velocity,
            auto_respawn: component.auto_respawn,
        }
    }
}

impl From<ParticleEmitterComponent> for SerializedParticleEmitter {
    fn from(component: ParticleEmitterComponent) -> Self {
        Self::from(&component)
    }
}

impl From<SerializedParticleEmitter> for ParticleEmitterComponent {
    fn from(serialized: SerializedParticleEmitter) -> Self {
        ParticleEmitterComponent {
            spawn_rate: serialized.spawn_rate,
            burst_count: serialized.burst_count,
            position: serialized.position,
            emission_shape: serialized.emission_shape,
            initial_velocity_range: serialized.initial_velocity_range,
            initial_scale_range: serialized.initial_scale_range,
            lifetime_range: serialized.lifetime_range,
            color_gradient: serialized.color_gradient,
            size_curve: serialized.size_curve,
            radial_velocity: serialized.radial_velocity,
            auto_respawn: serialized.auto_respawn,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SerializedBillboardOrientation {
    FaceCamera,
    FaceCameraYAxis,
}

impl From<BillboardOrientation> for SerializedBillboardOrientation {
    fn from(orientation: BillboardOrientation) -> Self {
        match orientation {
            BillboardOrientation::FaceCamera => SerializedBillboardOrientation::FaceCamera,
            BillboardOrientation::FaceCameraYAxis => {
                SerializedBillboardOrientation::FaceCameraYAxis
            }
        }
    }
}

impl From<SerializedBillboardOrientation> for BillboardOrientation {
    fn from(orientation: SerializedBillboardOrientation) -> Self {
        match orientation {
            SerializedBillboardOrientation::FaceCamera => BillboardOrientation::FaceCamera,
            SerializedBillboardOrientation::FaceCameraYAxis => {
                BillboardOrientation::FaceCameraYAxis
            }
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default)]
pub enum SerializedBillboardSpace {
    #[default]
    World,
    View {
        offset: [f32; 3],
    },
}

impl From<BillboardSpace> for SerializedBillboardSpace {
    fn from(space: BillboardSpace) -> Self {
        match space {
            BillboardSpace::World => SerializedBillboardSpace::World,
            BillboardSpace::View { offset } => SerializedBillboardSpace::View {
                offset: offset.to_array(),
            },
        }
    }
}

impl From<SerializedBillboardSpace> for BillboardSpace {
    fn from(space: SerializedBillboardSpace) -> Self {
        match space {
            SerializedBillboardSpace::World => BillboardSpace::World,
            SerializedBillboardSpace::View { offset } => BillboardSpace::View {
                offset: glam::Vec3::from_array(offset),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SerializedBillboard {
    pub orientation: SerializedBillboardOrientation,
    #[serde(default)]
    pub space: SerializedBillboardSpace,
    #[serde(default)]
    pub lit: bool,
}

impl From<Billboard> for SerializedBillboard {
    fn from(component: Billboard) -> Self {
        SerializedBillboard {
            orientation: SerializedBillboardOrientation::from(component.orientation),
            space: SerializedBillboardSpace::from(component.space),
            lit: component.lit,
        }
    }
}

impl From<SerializedBillboard> for Billboard {
    fn from(serialized: SerializedBillboard) -> Self {
        let mut billboard = Billboard::new(serialized.orientation.into());
        billboard = billboard.with_space(serialized.space.into());
        billboard.with_lighting(serialized.lit)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SerializedMeshBounds {
    pub min: [f32; 3],
    pub max: [f32; 3],
}

impl From<MeshBounds> for SerializedMeshBounds {
    fn from(bounds: MeshBounds) -> Self {
        Self {
            min: bounds.min.to_array(),
            max: bounds.max.to_array(),
        }
    }
}

impl From<SerializedMeshBounds> for MeshBounds {
    fn from(serialized: SerializedMeshBounds) -> Self {
        MeshBounds::new(
            glam::Vec3::from_array(serialized.min),
            glam::Vec3::from_array(serialized.max),
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SerializedDirectionalLight {
    pub color: [f32; 3],
    pub intensity: f32,
    pub shadow_size: f32,
}

impl From<DirectionalLight> for SerializedDirectionalLight {
    fn from(light: DirectionalLight) -> Self {
        Self {
            color: light.color.to_array(),
            intensity: light.intensity,
            shadow_size: light.shadow_size,
        }
    }
}

impl From<SerializedDirectionalLight> for DirectionalLight {
    fn from(serialized: SerializedDirectionalLight) -> Self {
        let mut light = DirectionalLight::new(
            glam::Vec3::from_array(serialized.color),
            serialized.intensity,
        );
        light.shadow_size = serialized.shadow_size;
        light
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SerializedPointLight {
    pub color: [f32; 3],
    pub intensity: f32,
    pub range: f32,
}

impl From<PointLight> for SerializedPointLight {
    fn from(light: PointLight) -> Self {
        Self {
            color: light.color.to_array(),
            intensity: light.intensity,
            range: light.range,
        }
    }
}

impl From<SerializedPointLight> for PointLight {
    fn from(serialized: SerializedPointLight) -> Self {
        PointLight {
            color: glam::Vec3::from_array(serialized.color),
            intensity: serialized.intensity,
            range: serialized.range,
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SerializedSpotLight {
    pub color: [f32; 3],
    pub intensity: f32,
    pub inner_angle: f32,
    pub outer_angle: f32,
    pub range: f32,
}

impl From<SpotLight> for SerializedSpotLight {
    fn from(light: SpotLight) -> Self {
        Self {
            color: light.color.to_array(),
            intensity: light.intensity,
            inner_angle: light.inner_angle,
            outer_angle: light.outer_angle,
            range: light.range,
        }
    }
}

impl From<SerializedSpotLight> for SpotLight {
    fn from(serialized: SerializedSpotLight) -> Self {
        SpotLight {
            color: glam::Vec3::from_array(serialized.color),
            intensity: serialized.intensity,
            inner_angle: serialized.inner_angle,
            outer_angle: serialized.outer_angle,
            range: serialized.range,
        }
    }
}
