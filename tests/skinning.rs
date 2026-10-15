//! Skinned glTF import, serialization and animation through the public scene API.

use std::path::PathBuf;

use pollster::block_on;
use wgpu_cube::asset::{Assets, Mesh};
use wgpu_cube::renderer::Vertex;
use wgpu_cube::scene::components::{Name, Parent, TransformComponent};
use wgpu_cube::scene::skeletal::{Animator, SkinnedMesh};
use wgpu_cube::scene::{
    Scene, SceneAsset, SceneAssetEntity, SceneImportDevice, SceneLibrary, SceneLoader,
    SerializedSkin, SerializedTransform,
};

struct HeadlessDevice {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl HeadlessDevice {
    fn new() -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) = block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("skinning-tests"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: wgpu::MemoryHints::Performance,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: wgpu::Trace::Off,
        }))
        .ok()?;
        Some(Self { device, queue })
    }
}

impl SceneImportDevice for HeadlessDevice {
    fn device(&self) -> &wgpu::Device {
        &self.device
    }

    fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    fn create_mesh(&mut self, vertices: &[Vertex], indices: &[u32]) -> Mesh {
        Mesh::from_vertices(&self.device, vertices, indices)
    }
}

const FOX: &str = "web/assets/skinned/Fox/Fox.glb";

#[test]
fn fox_imports_with_a_skin_and_an_animator_that_survive_json() {
    let Some(mut device) = HeadlessDevice::new() else {
        eprintln!("Skipping: no wgpu adapter");
        return;
    };
    let bundle = SceneLoader::load_gltf_asset(FOX, &mut device, 0.01).expect("load Fox");
    let asset = &bundle.asset;

    let skinned: Vec<&SceneAssetEntity> =
        asset.entities.iter().filter(|e| e.skin.is_some()).collect();
    assert_eq!(skinned.len(), 1);
    let skin = skinned[0].skin.as_ref().unwrap();
    assert_eq!(skin.joints.len(), 24);
    assert!(asset.mesh_data[skinned[0].mesh_handle.unwrap()].is_skinned());

    let animator = asset
        .entities
        .iter()
        .find_map(|e| e.animator.as_ref())
        .expect("skinned imports get an animator");
    assert_eq!(animator.libraries, [PathBuf::from(FOX)]);
    assert_eq!(animator.default_clip.as_deref(), Some("Survey"));
    // The skeleton plays by name through the animator, not as entity-bound clips.
    assert!(asset.animations.is_empty());

    let json = asset.to_json().unwrap();
    let back = SceneAsset::from_json(&json).unwrap();
    let back_skinned = back.entities.iter().find(|e| e.skin.is_some()).unwrap();
    assert_eq!(back_skinned.skin.as_ref(), Some(skin));
    assert_eq!(
        back.entities.iter().find_map(|e| e.animator.as_ref()),
        Some(animator)
    );
    assert!(back.mesh_data.iter().any(|mesh| mesh.is_skinned()));
}

#[test]
fn fox_animates_in_play_and_holds_its_rest_pose_while_editing() {
    let Some(mut device) = HeadlessDevice::new() else {
        eprintln!("Skipping: no wgpu adapter");
        return;
    };
    let mut bundle = SceneLoader::load_gltf_asset(FOX, &mut device, 0.01).expect("load Fox");
    let mut scene = Scene::new();
    let mut library = SceneLibrary::new();
    bundle.register_resources(&mut device, &mut scene.assets);
    let node =
        scene.instantiate_asset_with_renderer(&mut library, &bundle.asset, None, &mut device, None);
    scene.set_main_scene(node);
    scene.propagate_transforms();

    let world = scene.main_world();
    let (_, skin) = world
        .query::<&SkinnedMesh>()
        .iter()
        .next()
        .map(|(e, s)| (e, s.clone()))
        .unwrap();
    assert!(skin.joints.iter().all(|&joint| world.contains(joint)));
    let hip = world
        .query::<&Name>()
        .iter()
        .find(|(_, name)| name.0 == "b_Hip_01")
        .map(|(entity, _)| entity)
        .unwrap();
    assert!(skin.joints.contains(&hip));
    let rest = world.get::<&TransformComponent>(hip).unwrap().0;

    // Editing: animators do not run, so the rest pose is what gets saved.
    scene.set_animation_playback(false);
    scene.update(0.5);
    assert_eq!(
        scene
            .main_world()
            .get::<&TransformComponent>(hip)
            .unwrap()
            .0,
        rest
    );

    scene.set_animation_playback(true);
    for _ in 0..10 {
        scene.update(0.05);
    }
    let posed = scene
        .main_world()
        .get::<&TransformComponent>(hip)
        .unwrap()
        .0;
    assert_ne!(posed, rest, "Survey moves the hips");
    let animator_entity = scene
        .main_world()
        .query::<&Animator>()
        .iter()
        .next()
        .map(|(entity, _)| entity)
        .unwrap();
    let status = scene
        .main_world()
        .get::<&Animator>(animator_entity)
        .unwrap()
        .status(0);
    assert_eq!(status.clip.as_deref(), Some("Survey"));
    assert!((status.time - 0.5).abs() < 1e-3, "{}", status.time);

    // Stopping playback forgets it; the next play starts the default clip afresh.
    scene.set_animation_playback(false);
    assert!(scene
        .main_world()
        .get::<&Animator>(animator_entity)
        .unwrap()
        .state
        .layers
        .is_empty());
}

#[test]
fn instantiated_and_merged_skins_point_at_their_own_joints() {
    // Root -> Bone, plus a mesh entity skinned to Bone.
    let root = SceneAssetEntity::builder(SerializedTransform::identity())
        .with_name("Root")
        .with_children([1, 2])
        .build();
    let bone = SceneAssetEntity::builder(SerializedTransform::identity())
        .with_name("Bone")
        .with_parent(0)
        .build();
    let mut mesh = SceneAssetEntity::builder(SerializedTransform::identity())
        .with_name("Body")
        .with_parent(0)
        .build();
    mesh.skin = Some(SerializedSkin {
        joints: vec![1],
        inverse_bind: vec![glam::Mat4::IDENTITY.to_cols_array()],
    });
    let asset = SceneAsset::builder("Rig")
        .add_entity(root)
        .add_entity(bone)
        .add_entity(mesh)
        .build();

    let mut assets = Assets::new();
    let instantiated = asset.instantiate_into_world(None, &mut assets);
    let mut scene = Scene::new();
    let parent = scene.main_world_mut().spawn((Name::new("Holder"),));
    let first = scene.merge_world_as_child(parent, instantiated.world);
    let second = scene.merge_world_as_child(
        parent,
        asset.instantiate_into_world(None, &mut assets).world,
    );
    assert_eq!(first.len(), 3);
    assert_eq!(second.len(), 3);

    let world = scene.main_world();
    let skins: Vec<(hecs::Entity, SkinnedMesh)> = world
        .query::<&SkinnedMesh>()
        .iter()
        .map(|(entity, skin)| (entity, skin.clone()))
        .collect();
    assert_eq!(skins.len(), 2);
    for (mesh, skin) in &skins {
        let joint = skin.joints[0];
        assert_eq!(world.get::<&Name>(joint).unwrap().0, "Bone");
        // The joint is the sibling bone of this copy, not of the other one.
        let mesh_parent = world.get::<&Parent>(*mesh).unwrap().0;
        assert_eq!(world.get::<&Parent>(joint).unwrap().0, mesh_parent);
    }
    assert_ne!(skins[0].1.joints[0], skins[1].1.joints[0]);
}
