use crate::scene::components::{CameraComponent, EnvironmentComponent};
use super::entity::{SceneMaterialField, SceneMaterialHandle};
use crate::renderer::primitives::PrimitiveMeshDescriptor;
use super::serialization::{
    SerializedBillboard, SerializedDirectionalLight, SerializedMaterial, SerializedMeshBounds,
    SerializedParticleBehavior, SerializedParticleEmitter, SerializedParticleSystem,
    SerializedPointLight, SerializedRuneScript, SerializedSpotLight, SerializedTransform,
};
use serde::{Deserialize, Serialize};

/// Reference to a prefab entity stored in another scene document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenePrefabRef {
    /// Identifier of the scene document containing the prefab definition.
    pub document: String,
    /// Hierarchical path to the prefab node within the referenced document.
    pub node_path: Vec<String>,
    /// Local overrides that should be applied when instantiating the prefab.
    #[serde(default)]
    pub overrides: ScenePrefabOverrides,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(from = "ScenePrefabOverridesData", into = "ScenePrefabOverridesData")]
pub struct ScenePrefabOverrides {
    #[serde(default)]
    pub transform: Option<SerializedTransform>,
    #[serde(default)]
    pub visible: Option<bool>,
    #[serde(default)]
    pub mesh_handle: Option<usize>,
    #[serde(default)]
    pub primitive_mesh: Option<PrimitiveMeshDescriptor>,
    #[serde(default)]
    pub mesh_bounds: Option<SerializedMeshBounds>,
    #[serde(default)]
    pub material: Option<SceneMaterialHandle>,
    #[serde(skip)]
    pub material_data: Option<SerializedMaterial>,
    #[serde(default)]
    pub script: Option<SerializedRuneScript>,
    #[serde(default)]
    pub directional_light: Option<SerializedDirectionalLight>,
    #[serde(default)]
    pub point_light: Option<SerializedPointLight>,
    #[serde(default)]
    pub spot_light: Option<SerializedSpotLight>,
    #[serde(default)]
    pub casts_shadow: Option<bool>,
    #[serde(default)]
    pub billboard: Option<SerializedBillboard>,
    #[serde(default)]
    pub particle_system: Option<SerializedParticleSystem>,
    #[serde(default)]
    pub particle_emitter: Option<SerializedParticleEmitter>,
    #[serde(default)]
    pub particle_behavior: Option<SerializedParticleBehavior>,
    #[serde(default)]
    pub environment: Option<EnvironmentComponent>,
    #[serde(default)]
    pub camera: Option<CameraComponent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ScenePrefabOverridesData {
    #[serde(default)]
    transform: Option<SerializedTransform>,
    #[serde(default)]
    visible: Option<bool>,
    #[serde(default)]
    mesh_handle: Option<usize>,
    #[serde(default)]
    primitive_mesh: Option<PrimitiveMeshDescriptor>,
    #[serde(default)]
    mesh_bounds: Option<SerializedMeshBounds>,
    #[serde(default)]
    material: Option<SceneMaterialField>,
    #[serde(default)]
    script: Option<SerializedRuneScript>,
    #[serde(default)]
    directional_light: Option<SerializedDirectionalLight>,
    #[serde(default)]
    point_light: Option<SerializedPointLight>,
    #[serde(default)]
    spot_light: Option<SerializedSpotLight>,
    #[serde(default)]
    casts_shadow: Option<bool>,
    #[serde(default)]
    billboard: Option<SerializedBillboard>,
    #[serde(default)]
    particle_system: Option<SerializedParticleSystem>,
    #[serde(default)]
    particle_emitter: Option<SerializedParticleEmitter>,
    #[serde(default)]
    particle_behavior: Option<SerializedParticleBehavior>,
    #[serde(default)]
    environment: Option<EnvironmentComponent>,
    #[serde(default)]
    camera: Option<CameraComponent>,
}

impl From<ScenePrefabOverridesData> for ScenePrefabOverrides {
    fn from(data: ScenePrefabOverridesData) -> Self {
        let (material, material_data) = match data.material {
            Some(SceneMaterialField::Handle(handle)) => (Some(handle), None),
            Some(SceneMaterialField::Legacy(legacy)) => (None, Some(*legacy)),
            None => (None, None),
        };

        ScenePrefabOverrides {
            transform: data.transform,
            visible: data.visible,
            mesh_handle: data.mesh_handle,
            primitive_mesh: data.primitive_mesh,
            mesh_bounds: data.mesh_bounds,
            material,
            material_data,
            script: data.script,
            directional_light: data.directional_light,
            point_light: data.point_light,
            spot_light: data.spot_light,
            casts_shadow: data.casts_shadow,
            billboard: data.billboard,
            particle_system: data.particle_system,
            particle_emitter: data.particle_emitter,
            particle_behavior: data.particle_behavior,
            environment: data.environment,
            camera: data.camera,
        }
    }
}

impl From<ScenePrefabOverrides> for ScenePrefabOverridesData {
    fn from(overrides: ScenePrefabOverrides) -> Self {
        let ScenePrefabOverrides {
            transform,
            visible,
            mesh_handle,
            primitive_mesh,
            mesh_bounds,
            material,
            material_data,
            script,
            directional_light,
            point_light,
            spot_light,
            casts_shadow,
            billboard,
            particle_system,
            particle_emitter,
            particle_behavior,
            environment,
            camera,
        } = overrides;

        let material = material
            .map(SceneMaterialField::Handle)
            .or_else(|| material_data.map(|data| SceneMaterialField::Legacy(Box::new(data))));

        ScenePrefabOverridesData {
            transform,
            visible,
            mesh_handle,
            primitive_mesh,
            mesh_bounds,
            material,
            script,
            directional_light,
            point_light,
            spot_light,
            casts_shadow,
            billboard,
            particle_system,
            particle_emitter,
            particle_behavior,
            environment,
            camera,
        }
    }
}
