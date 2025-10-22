use std::fs;

use tempfile::tempdir;
use wgpu_cube::project::{ProjectManifest, ProjectMetadata, SCENE_FILE_NAME};
use wgpu_cube::scene::{
    Name, ParticleBehaviorPreset, ParticleSystemComponent, Scene, SceneAsset, Transform,
    TransformComponent,
};

#[test]
fn particle_system_asset_roundtrip() {
    let mut scene = Scene::new();
    scene.main_world_mut().spawn((
        Name::new("Particle Emitter"),
        TransformComponent(Transform::IDENTITY),
        ParticleSystemComponent::new(120.0, ParticleBehaviorPreset::Starfield),
    ));

    let asset = scene
        .export_main_asset("Particles")
        .expect("scene should export particle asset");
    assert!(asset
        .entities
        .iter()
        .any(|entity| entity.particle_system.is_some()));

    let json = asset.to_json().expect("asset should serialize");
    let restored = SceneAsset::from_json(&json).expect("asset should deserialize");

    let mut restored_scene = Scene::new();
    let node = restored_scene.instantiate_asset(&restored, None);
    restored_scene.set_main_scene(node);

    let mut query = restored_scene
        .main_world()
        .query::<&ParticleSystemComponent>();
    let restored_component = query
        .iter()
        .map(|(_, component)| *component)
        .next()
        .expect("particle system component should restore");

    assert_eq!(
        restored_component,
        ParticleSystemComponent::new(120.0, ParticleBehaviorPreset::Starfield)
    );
}

#[test]
fn project_manifest_scene_json_roundtrip_includes_particles() {
    let mut scene = Scene::new();
    scene.main_world_mut().spawn((
        Name::new("Burst"),
        TransformComponent(Transform::IDENTITY),
        ParticleSystemComponent::new(45.0, ParticleBehaviorPreset::Physics),
    ));

    let metadata = ProjectMetadata::default();
    let manifest = ProjectManifest::capture(&scene, metadata)
        .expect("manifest capture should succeed for particle scene");

    assert!(manifest
        .scene
        .entities
        .iter()
        .any(|entity| entity.particle_system.is_some()));

    let temp_dir = tempdir().expect("temp dir should create");
    manifest
        .save_to_dir(temp_dir.path())
        .expect("manifest should save");

    let scene_path = temp_dir.path().join(SCENE_FILE_NAME);
    assert!(scene_path.exists(), "scene.json should be written");

    let scene_json = fs::read_to_string(&scene_path).expect("scene.json should read");
    let saved_scene = SceneAsset::from_json(&scene_json).expect("scene asset should load");
    assert!(saved_scene
        .entities
        .iter()
        .any(|entity| entity.particle_system.is_some()));

    let loaded_manifest =
        ProjectManifest::load_from_dir(temp_dir.path()).expect("manifest should load from disk");
    assert!(loaded_manifest
        .scene
        .entities
        .iter()
        .any(|entity| entity.particle_system.is_some()));
}
