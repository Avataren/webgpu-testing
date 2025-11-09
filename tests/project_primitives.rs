use std::sync::Arc;

use glam::Vec3;
use pollster::block_on;
use tempfile::tempdir;
use wgpu::{
    Backends, DeviceDescriptor, Features, Instance, InstanceDescriptor, Limits, MemoryHints,
    PowerPreference, RequestAdapterOptions, Trace,
};

use wgpu_cube::asset::Mesh;
use wgpu_cube::project::{ProjectManifest, ProjectMetadata};
use wgpu_cube::renderer::primitives::PrimitiveMeshDescriptor;
use wgpu_cube::renderer::Vertex;
use wgpu_cube::scene::{
    CameraProjection, MeshBounds, MeshComponent, Name, PrimitiveMeshComponent, Scene,
    SceneAssetBundle, SceneAssetResources, SceneImportDevice, SceneLibrary, Transform,
    TransformComponent, Visible,
};

struct HeadlessDevice {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
}

impl HeadlessDevice {
    fn new() -> Option<Self> {
        let instance = Instance::new(&InstanceDescriptor {
            backends: Backends::all(),
            ..Default::default()
        });
        let adapter = match block_on(instance.request_adapter(&RequestAdapterOptions {
            power_preference: PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        })) {
            Ok(adapter) => adapter,
            Err(err) => {
                eprintln!(
                    "Skipping primitive mesh roundtrip test: failed to request adapter ({err})"
                );
                return None;
            }
        };

        let (device, queue) = block_on(adapter.request_device(&DeviceDescriptor {
            label: Some("headless-project-primitive-test"),
            required_features: Features::empty(),
            required_limits: Limits::default(),
            memory_hints: MemoryHints::Performance,
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            trace: Trace::Off,
        }))
        .ok()?;

        Some(Self {
            device: Arc::new(device),
            queue: Arc::new(queue),
        })
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

#[test]
fn project_manifest_roundtrip_preserves_primitive_meshes() {
    let Some(mut headless) = HeadlessDevice::new() else {
        eprintln!("Skipping primitive mesh roundtrip test: no suitable headless adapter available");
        return;
    };

    let descriptor = PrimitiveMeshDescriptor::Cube;

    let mut scene = Scene::new();
    let cube_handle = scene
        .assets
        .ensure_primitive_mesh(&mut headless, descriptor);
    let bounds = scene
        .assets
        .meshes
        .get(cube_handle)
        .and_then(|mesh| MeshBounds::from_vertices(&mesh.data().vertices));

    let spawn_cube = |scene: &mut Scene, name: &str| {
        if let Some(bounds) = bounds {
            scene.main_world_mut().spawn((
                Name::new(name.to_string()),
                TransformComponent(Transform::default()),
                MeshComponent(cube_handle),
                PrimitiveMeshComponent { descriptor },
                bounds,
                Visible(true),
            ));
        } else {
            scene.main_world_mut().spawn((
                Name::new(name.to_string()),
                TransformComponent(Transform::default()),
                MeshComponent(cube_handle),
                PrimitiveMeshComponent { descriptor },
                Visible(true),
            ));
        }
    };

    spawn_cube(&mut scene, "Cube A");
    spawn_cube(&mut scene, "Cube B");

    let handles: Vec<_> = scene
        .main_world()
        .query::<&MeshComponent>()
        .iter()
        .map(|(_, component)| component.0)
        .collect();
    assert_eq!(
        handles.len(),
        2,
        "expected two primitive meshes before capture"
    );
    assert!(
        handles.iter().all(|&handle| handle == cube_handle),
        "primitive meshes should share the same handle before capture"
    );

    let metadata = ProjectMetadata::default();
    let manifest = ProjectManifest::capture(&scene, metadata, None)
        .expect("capturing manifest for primitive scene should succeed");

    let temp_dir = tempdir().expect("temporary directory should create");
    manifest
        .save_to_dir(temp_dir.path())
        .expect("manifest should save to disk");

    let loaded =
        ProjectManifest::load_from_dir(temp_dir.path()).expect("manifest should reload from disk");
    let document = loaded
        .scenes()
        .first()
        .expect("loaded manifest should include a scene");
    let asset = loaded
        .scene_asset(&document.id)
        .expect("loaded manifest should provide scene asset");

    let mut bundle = SceneAssetBundle::new(asset.clone(), SceneAssetResources::default());
    let mut restored_scene = Scene::new();
    let mut library = SceneLibrary::new();

    bundle.register_resources(&mut headless, &mut restored_scene.assets);

    let node = restored_scene.instantiate_asset_with_renderer(
        &mut library,
        &bundle.asset,
        None,
        &mut headless,
        None,
    );
    restored_scene.set_main_scene(node);

    let mut query = restored_scene
        .world()
        .query::<(&MeshComponent, &PrimitiveMeshComponent)>();
    let restored: Vec<_> = query
        .iter()
        .map(|(_, (mesh, primitive))| (mesh.0, primitive.descriptor))
        .collect();

    assert_eq!(
        restored.len(),
        2,
        "restored scene should contain two primitives"
    );
    assert!(
        restored.iter().all(
            |&(handle, descriptor)| descriptor == PrimitiveMeshDescriptor::Cube
                && handle == restored[0].0
        ),
        "restored primitives should share the same handle and descriptor"
    );
}

#[test]
fn project_manifest_roundtrip_restores_engine_camera() {
    let Some(mut headless) = HeadlessDevice::new() else {
        eprintln!("Skipping engine camera roundtrip test: no suitable headless adapter available");
        return;
    };

    let mut scene = Scene::new();
    scene.main_world_mut().spawn((
        Name::new("Marker".to_string()),
        TransformComponent(Transform::default()),
        Visible(true),
    ));

    let desired_eye = Vec3::new(5.0, 10.0, -15.0);
    let desired_target = Vec3::new(-2.0, 1.5, 7.0);
    let desired_up = Vec3::new(0.0, 1.0, 0.0);
    let desired_projection = CameraProjection::perspective(1.25, 0.05, 350.0);

    {
        let camera = scene.camera_mut();
        camera.eye = desired_eye;
        camera.target = desired_target;
        camera.up = desired_up;
        camera.set_projection(desired_projection);
    }

    let metadata = ProjectMetadata::default();
    let manifest = ProjectManifest::capture(&scene, metadata, None)
        .expect("capturing manifest for engine camera roundtrip should succeed");

    let temp_dir = tempdir().expect("temporary directory should create");
    manifest
        .save_to_dir(temp_dir.path())
        .expect("manifest should save to disk");

    let loaded =
        ProjectManifest::load_from_dir(temp_dir.path()).expect("manifest should reload from disk");

    let mut library = SceneLibrary::new();
    let (workspace, textures_changed) = loaded
        .instantiate_workspace(&mut headless, temp_dir.path(), &mut library)
        .expect("manifest should instantiate into a scene");
    let restored_scene = workspace
        .into_active_scene()
        .expect("workspace should provide an active scene");
    if textures_changed {
        headless
            .queue()
            .submit(std::iter::empty::<wgpu::CommandBuffer>());
    }

    assert!(
        restored_scene.active_camera_entity().is_none(),
        "roundtrip should not create an active camera entity"
    );

    let restored_camera = restored_scene.camera();
    assert!(
        restored_camera.eye.abs_diff_eq(desired_eye, 1e-5),
        "engine camera eye should roundtrip"
    );
    assert!(
        restored_camera.target.abs_diff_eq(desired_target, 1e-5),
        "engine camera target should roundtrip"
    );
    assert!(
        restored_camera.up.abs_diff_eq(desired_up, 1e-5),
        "engine camera up vector should roundtrip"
    );

    match restored_camera.projection() {
        CameraProjection::Perspective {
            fov_y_radians,
            near,
            far,
        } => {
            if let CameraProjection::Perspective {
                fov_y_radians: expected_fov,
                near: expected_near,
                far: expected_far,
            } = desired_projection
            {
                assert!(
                    (fov_y_radians - expected_fov).abs() < 1e-6,
                    "engine camera FOV should roundtrip"
                );
                assert!(
                    (near - expected_near).abs() < 1e-6,
                    "engine camera near plane should roundtrip"
                );
                assert!(
                    (far - expected_far).abs() < 1e-4,
                    "engine camera far plane should roundtrip"
                );
            } else {
                panic!("expected perspective projection for desired camera");
            }
        }
        other => panic!("expected perspective camera, got {:?}", other),
    }
}

#[test]
fn project_manifest_capture_succeeds_for_empty_scene() {
    let scene = Scene::new();
    let metadata = ProjectMetadata {
        name: "Empty Scene".to_string(),
        ..Default::default()
    };

    let manifest = ProjectManifest::capture(&scene, metadata, None)
        .expect("capturing an empty scene should still succeed");

    assert_eq!(
        manifest.scenes().len(),
        1,
        "capture should produce a single scene document"
    );

    let document = &manifest.scenes()[0];
    let asset = manifest
        .scene_asset(&document.id)
        .expect("captured manifest should retain a scene asset");
    assert!(
        asset.entities.is_empty(),
        "empty scenes should serialize without entities"
    );
}
