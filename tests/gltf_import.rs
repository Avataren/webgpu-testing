use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::Arc;

use pollster::block_on;
use wgpu::{
    Backends, DeviceDescriptor, Features, Instance, InstanceDescriptor, Limits, MemoryHints,
    PowerPreference, RequestAdapterOptions, Trace,
};

use wgpu_cube::asset::Mesh;
use wgpu_cube::renderer::Vertex;
use wgpu_cube::scene::{GltfMaterial, MeshComponent, Scene, SceneImportDevice, SceneLoader};

struct HeadlessImporter {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
}

impl HeadlessImporter {
    fn new() -> Option<Self> {
        let instance = Instance::new(&InstanceDescriptor {
            backends: Backends::all(),
            ..Default::default()
        });
        let adapter = block_on(instance.request_adapter(&RequestAdapterOptions {
            power_preference: PowerPreference::LowPower,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok()?;

        let (device, queue) = block_on(adapter.request_device(&DeviceDescriptor {
            label: Some("headless-importer"),
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

impl SceneImportDevice for HeadlessImporter {
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
fn multi_material_import_preserves_mesh_material_association() {
    let Some(mut importer) = HeadlessImporter::new() else {
        eprintln!(
            "Skipping test: no suitable headless wgpu adapter available for regression import test"
        );
        return;
    };
    let mut bundle =
        SceneLoader::load_gltf_asset("web/assets/sponza/Sponza.gltf", &mut importer, 1.0)
            .expect("failed to load asset");

    let original_mesh_indices: Vec<Option<usize>> = bundle
        .asset
        .entities
        .iter()
        .map(|entity| entity.mesh_handle)
        .collect();

    let original_material_indices: Vec<Option<usize>> = bundle
        .asset
        .entities
        .iter()
        .map(|entity| entity.gltf_material)
        .collect();

    let mut scene = Scene::new();
    bundle.register_resources(&importer, &mut scene.assets);

    let node = scene.instantiate_asset(&bundle.asset, None);
    scene.set_main_scene(node);

    let mut local_to_global = HashMap::new();
    let mut local_to_material = HashMap::new();

    for (idx, local_opt) in original_mesh_indices.iter().enumerate() {
        if let Some(local) = *local_opt {
            if let Some(global) = bundle.asset.entities[idx].mesh_handle {
                match local_to_global.entry(local) {
                    Entry::Vacant(entry) => {
                        entry.insert(global);
                    }
                    Entry::Occupied(entry) => {
                        assert_eq!(*entry.get(), global);
                    }
                }
            }

            if let Some(material) = original_material_indices[idx] {
                match local_to_material.entry(local) {
                    Entry::Vacant(entry) => {
                        entry.insert(material);
                    }
                    Entry::Occupied(entry) => {
                        assert_eq!(*entry.get(), material);
                    }
                }
            }
        }
    }

    let global_to_local: HashMap<usize, usize> = local_to_global
        .iter()
        .map(|(&local, &global)| (global, local))
        .collect();

    let world = scene.world();
    for (_, (mesh_component, gltf_material)) in
        world.query::<(&MeshComponent, &GltfMaterial)>().iter()
    {
        let global_index = mesh_component.0.index();
        let local_index = *global_to_local
            .get(&global_index)
            .expect("global mesh missing local mapping");
        let expected_material = *local_to_material
            .get(&local_index)
            .expect("local mesh missing expected material");

        assert_eq!(expected_material, gltf_material.0);
    }
}
