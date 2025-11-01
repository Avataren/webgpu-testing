use std::fs;

use tempfile::tempdir;
use wgpu_cube::project::{ProjectManifest, ProjectMetadata};
use wgpu_cube::scene::{
    Name, ParticleBehaviorConfig, ParticleBehaviorPreset, ParticleEmissionShape,
    ParticleEmitterComponent, ParticleFloatRange, ParticleSizeCurve, ParticleSizeKeyframe,
    ParticleSystemComponent, ParticleVec3Range, Scene, SceneAsset, StarfieldBehaviorConfig,
    Transform, TransformComponent,
};

#[test]
fn particle_system_asset_roundtrip() {
    let mut scene = Scene::new();
    let particle_system = ParticleSystemComponent::new(120.0, ParticleBehaviorPreset::Starfield)
        .with_behavior_config(ParticleBehaviorConfig::Starfield(StarfieldBehaviorConfig {
            near_plane: 0.05,
            far_plane: 180.0,
            far_reset_band: 6.0,
            field_half_size: 75.0,
            min_radius: 0.4,
        }));
    let particle_emitter = ParticleEmitterComponent {
        spawn_rate: 180.0,
        burst_count: Some(32),
        auto_respawn: true,
        emission_shape: ParticleEmissionShape::Sphere { radius: 2.5 },
        initial_velocity_range: ParticleVec3Range::new([-0.5, 1.0, -0.5], [0.5, 2.0, 0.5]),
        lifetime_range: ParticleFloatRange::new(1.25, 2.75),
        size_curve: ParticleSizeCurve {
            keyframes: vec![
                ParticleSizeKeyframe {
                    size: 0.5,
                    time: 0.0,
                },
                ParticleSizeKeyframe {
                    size: 1.5,
                    time: 0.8,
                },
            ],
        },
        ..Default::default()
    };
    scene.main_world_mut().spawn((
        Name::new("Particle Emitter"),
        TransformComponent(Transform::IDENTITY),
        particle_system.clone(),
        particle_emitter.clone(),
    ));

    let asset = scene
        .export_main_asset("Particles")
        .expect("scene should export particle asset");
    assert!(asset
        .entities
        .iter()
        .any(|entity| entity.particle_system.is_some()));
    assert!(asset
        .entities
        .iter()
        .any(|entity| entity.particle_emitter.is_some()));
    assert!(asset
        .entities
        .iter()
        .any(|entity| entity.particle_behavior.is_some()));

    let json = asset.to_json().expect("asset should serialize");
    let restored = SceneAsset::from_json(&json).expect("asset should deserialize");

    let mut restored_scene = Scene::new();
    let node = restored_scene.instantiate_asset(&restored, None);
    restored_scene.set_main_scene(node);

    let mut system_query = restored_scene
        .main_world()
        .query::<&ParticleSystemComponent>();
    let restored_system = system_query
        .iter()
        .map(|(_, component)| component.clone())
        .next()
        .expect("particle system component should restore");
    assert_eq!(restored_system, particle_system);

    let mut emitter_query = restored_scene
        .main_world()
        .query::<&ParticleEmitterComponent>();
    let restored_emitter = emitter_query
        .iter()
        .map(|(_, component)| component.clone())
        .next()
        .expect("particle emitter component should restore");
    assert_eq!(restored_emitter, particle_emitter);
}

#[test]
fn project_manifest_scene_json_roundtrip_includes_particles() {
    let mut scene = Scene::new();
    let particle_emitter = ParticleEmitterComponent {
        spawn_rate: 60.0,
        emission_shape: ParticleEmissionShape::Cone {
            angle: 0.5,
            radius: 1.0,
        },
        ..Default::default()
    };
    scene.main_world_mut().spawn((
        Name::new("Burst"),
        TransformComponent(Transform::IDENTITY),
        ParticleSystemComponent::new(45.0, ParticleBehaviorPreset::Physics),
        particle_emitter,
    ));

    let metadata = ProjectMetadata::default();
    let manifest = ProjectManifest::capture(&scene, metadata, None)
        .expect("manifest capture should succeed for particle scene");

    let document = manifest
        .scenes()
        .first()
        .expect("manifest should contain a primary scene");
    let asset = manifest
        .scene_asset(&document.id)
        .expect("captured manifest should include scene asset");

    assert!(asset
        .entities
        .iter()
        .any(|entity| entity.particle_system.is_some()));
    assert!(asset
        .entities
        .iter()
        .any(|entity| entity.particle_emitter.is_some()));
    assert!(asset
        .entities
        .iter()
        .any(|entity| entity.particle_behavior.is_some()));

    let temp_dir = tempdir().expect("temp dir should create");
    manifest
        .save_to_dir(temp_dir.path())
        .expect("manifest should save");

    let scene_path = temp_dir.path().join(&document.relative_path);
    assert!(scene_path.exists(), "scene document should be written");

    let scene_json = fs::read_to_string(&scene_path).expect("scene document should read");
    let saved_scene = SceneAsset::from_json(&scene_json).expect("scene asset should load");
    assert!(saved_scene
        .entities
        .iter()
        .any(|entity| entity.particle_system.is_some()));
    assert!(saved_scene
        .entities
        .iter()
        .any(|entity| entity.particle_emitter.is_some()));
    assert!(saved_scene
        .entities
        .iter()
        .any(|entity| entity.particle_behavior.is_some()));

    let loaded_manifest =
        ProjectManifest::load_from_dir(temp_dir.path()).expect("manifest should load from disk");
    let loaded_document = loaded_manifest
        .scenes()
        .first()
        .expect("loaded manifest should contain a scene");
    let loaded_asset = loaded_manifest
        .scene_asset(&loaded_document.id)
        .expect("loaded manifest should provide scene asset");

    assert!(loaded_asset
        .entities
        .iter()
        .any(|entity| entity.particle_system.is_some()));
    assert!(asset
        .entities
        .iter()
        .any(|entity| entity.particle_emitter.is_some()));
    assert!(loaded_asset
        .entities
        .iter()
        .any(|entity| entity.particle_emitter.is_some()));
    assert!(loaded_asset
        .entities
        .iter()
        .any(|entity| entity.particle_behavior.is_some()));
    assert!(asset
        .entities
        .iter()
        .any(|entity| entity.particle_behavior.is_some()));
}

#[test]
fn particle_spawn_rate_edit_serializes_consistently() {
    let mut scene = Scene::new();
    let entity = scene.main_world_mut().spawn((
        Name::new("Emitter"),
        TransformComponent(Transform::IDENTITY),
        ParticleSystemComponent::new(90.0, ParticleBehaviorPreset::Physics),
        ParticleEmitterComponent {
            spawn_rate: 90.0,
            ..Default::default()
        },
    ));

    let updated_spawn_rate = 240.0;
    {
        let world = scene.main_world_mut();
        let mut system = world
            .get::<&mut ParticleSystemComponent>(entity)
            .expect("system component should exist");
        system.spawn_rate = updated_spawn_rate;
        let mut emitter = world
            .get::<&mut ParticleEmitterComponent>(entity)
            .expect("emitter component should exist");
        emitter.spawn_rate = updated_spawn_rate;
    }

    let asset = scene
        .export_main_asset("Particles")
        .expect("scene should export asset after spawn rate edit");
    let serialized = asset
        .entities
        .iter()
        .find(|entity| entity.particle_system.is_some())
        .expect("particle entity should exist");
    let serialized_system = serialized
        .particle_system
        .as_ref()
        .expect("serialized system should exist");
    let serialized_emitter = serialized
        .particle_emitter
        .as_ref()
        .expect("serialized emitter should exist");

    assert_eq!(serialized_system.spawn_rate, updated_spawn_rate);
    assert_eq!(serialized_emitter.spawn_rate, updated_spawn_rate);
}
