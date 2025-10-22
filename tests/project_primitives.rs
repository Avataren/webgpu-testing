use std::sync::Arc;

use pollster::block_on;
use tempfile::tempdir;
use wgpu::{
    Backends, DeviceDescriptor, Features, Instance, InstanceDescriptor, Limits, MemoryHints,
    PowerPreference, RequestAdapterOptions, Trace,
};

use wgpu_cube::asset::Mesh;
use wgpu_cube::project::{ProjectManifest, ProjectMetadata};
use wgpu_cube::renderer::primitives::cube_mesh;
use wgpu_cube::renderer::Vertex;
use wgpu_cube::scene::{
    MeshComponent, Name, Scene, SceneAssetBundle, SceneAssetResources, SceneImportDevice,
    Transform, TransformComponent, Visible,
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
    let Some(headless) = HeadlessDevice::new() else {
        eprintln!("Skipping primitive mesh roundtrip test: no suitable headless adapter available");
        return;
    };

    let (cube_vertices, cube_indices) = cube_mesh();
    let expected_vertices = cube_vertices.clone();
    let expected_indices = cube_indices.clone();

    let cube_mesh = Mesh::from_vertices(headless.device.as_ref(), &cube_vertices, &cube_indices);

    let mut scene = Scene::new();
    let cube_handle = scene.assets.meshes.insert(cube_mesh);
    scene.main_world_mut().spawn((
        Name::new("Cube"),
        TransformComponent(Transform::default()),
        MeshComponent(cube_handle),
        Visible(true),
    ));

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

    bundle.register_resources(&headless, &mut restored_scene.assets);

    let node = restored_scene.instantiate_asset(&bundle.asset, None);
    restored_scene.set_main_scene(node);

    let mut cube_found = false;
    let mut query = restored_scene.world().query::<(&MeshComponent, &Name)>();

    for (_, (mesh_component, name)) in query.iter() {
        if name.0 == "Cube" {
            cube_found = true;
            let mesh = restored_scene
                .assets
                .meshes
                .get(mesh_component.0)
                .expect("cube mesh handle should resolve after reload");
            let data = mesh.data();

            assert_eq!(data.indices, expected_indices);
            assert_eq!(data.vertices.len(), expected_vertices.len());
            for (actual, expected) in data.vertices.iter().zip(&expected_vertices) {
                assert_eq!(actual.pos, expected.pos);
                assert_eq!(actual.normal, expected.normal);
                assert_eq!(actual.uv, expected.uv);
                assert_eq!(actual.tangent, expected.tangent);
            }
            break;
        }
    }

    assert!(cube_found, "cube entity should exist after manifest reload");
}
