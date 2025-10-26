use std::sync::Arc;

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
    MeshBounds, MeshComponent, Name, PrimitiveMeshComponent, Scene, SceneAssetBundle,
    SceneAssetResources, SceneImportDevice, Transform, TransformComponent, Visible,
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
    let manifest = ProjectManifest::capture(&scene, metadata)
        .expect("capturing manifest for primitive scene should succeed");

    let temp_dir = tempdir().expect("temporary directory should create");
    manifest
        .save_to_dir(temp_dir.path())
        .expect("manifest should save to disk");

    let loaded =
        ProjectManifest::load_from_dir(temp_dir.path()).expect("manifest should reload from disk");

    let mut bundle = SceneAssetBundle::new(loaded.scene.clone(), SceneAssetResources::default());
    let mut restored_scene = Scene::new();

    bundle.register_resources(&mut headless, &mut restored_scene.assets);

    let node = restored_scene.instantiate_asset_with_renderer(&bundle.asset, None, &mut headless);
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
