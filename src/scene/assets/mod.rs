// Submodules
pub mod serialization;
mod core;
mod resources;
mod prefabs;
mod entity;
mod builder;
mod tree;

// Re-export main types from serialization (only public ones)
pub use serialization::{
    ImportedGltfMeta,
    SerializedAnimationClip,
    SerializedMaterial,
    SerializedMaterialKind,
    SerializedParticleBehavior,
    SerializedParticleEmitter,
    SerializedParticleSystem,
    SerializedRuneScript,
    SerializedRuneScriptSource,
    SerializedTransform,
};

// Re-export from core
pub use core::{SceneAsset, InstantiatedSceneAsset};

// Re-export from resources
pub use resources::{
    SceneAssetResources, SceneAssetResourcesBuilder, SceneAssetBundle,
};

// Re-export from prefabs
pub use prefabs::{ScenePrefabRef, ScenePrefabOverrides};

// Re-export from entity
pub use entity::{SceneAssetEntity, SceneMaterialHandle};

// Re-export from builder
pub use builder::{SceneAssetEntityBuilder, SceneAssetBuilder};

// Re-export from tree
pub use tree::{
    SceneTreeAsset, SceneTreeAssetNode, SceneTreeAssetNodeBuilder,
};

// Re-export pub(crate) functions for internal use
pub(crate) use tree::{serialize_world, build_tree_asset_node};


#[cfg(test)]
mod tests {
    use super::*;
    use super::serialization::{SerializedBillboard, SerializedTextureSlot};
    use crate::asset::Assets;
    use crate::project::{ProjectManifest, ProjectMetadata, CONTENT_DIR};
    use crate::renderer::material::MaterialFlags;
    use crate::scene::components::{Billboard, BillboardOrientation, BillboardSpace, TransformComponent, Visible};
    use crate::scene::transform::Transform;
    use crate::scene::{Scene, SceneLibrary};
    use glam::Vec3;
    use hecs::World;
    use serde_json::Value;
    use std::collections::BTreeMap;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn make_material(flags: MaterialFlags, base_texture: u32) -> SerializedMaterial {
        SerializedMaterial {
            base_color: [255, 255, 255, 255],
            flags: flags.bits(),
            base_color_texture: SerializedTextureSlot::from_index(base_texture),
            metallic_roughness_texture: SerializedTextureSlot::from_index(0),
            normal_texture: SerializedTextureSlot::from_index(0),
            emissive_texture: SerializedTextureSlot::from_index(0),
            occlusion_texture: SerializedTextureSlot::from_index(0),
            metallic_factor: 0,
            roughness_factor: 0,
            emissive_strength: 0,
            kind: SerializedMaterialKind::default(),
        }
    }

    #[test]
    fn serialized_transform_roundtrip() {
        let transform = Transform::from_trs(
            Vec3::new(1.0, 2.0, 3.0),
            glam::Quat::from_rotation_y(1.2),
            Vec3::new(0.5, 0.75, 1.25),
        );

        let serialized = SerializedTransform::from(transform);
        let restored: Transform = serialized.into();

        assert!(restored
            .translation
            .abs_diff_eq(transform.translation, 1e-5));
        assert!(restored.rotation.abs_diff_eq(transform.rotation, 1e-5));
        assert!(restored.scale.abs_diff_eq(transform.scale, 1e-5));
    }

    #[test]
    fn billboard_component_roundtrip() {
        let billboard_component = Billboard::new(BillboardOrientation::FaceCameraYAxis)
            .with_space(BillboardSpace::View {
                offset: Vec3::new(1.0, 2.0, -3.5),
            })
            .with_lighting(true);
        let serialized_billboard = SerializedBillboard::from(billboard_component);

        let entity = SceneAssetEntity::builder(SerializedTransform::identity())
            .with_name("Billboard")
            .with_billboard(serialized_billboard)
            .build();

        let asset = SceneAsset::builder("BillboardScene")
            .add_entity(entity)
            .build();

        let json = asset.to_json().expect("serialize scene asset");
        let parsed: Value = serde_json::from_str(&json).expect("parse asset json");
        assert_eq!(
            parsed["entities"][0]["billboard"]["orientation"],
            Value::String("FaceCameraYAxis".into())
        );
        let restored = SceneAsset::from_json(&json).expect("deserialize scene asset");

        assert_eq!(restored.entities.len(), 1);
        let restored_billboard = restored.entities[0]
            .billboard
            .expect("billboard should deserialize");
        let restored_component = Billboard::from(restored_billboard);

        assert_eq!(
            restored_component.orientation,
            billboard_component.orientation
        );
        assert_eq!(restored_component.lit, billboard_component.lit);

        match (restored_component.space, billboard_component.space) {
            (
                BillboardSpace::View {
                    offset: restored_offset,
                },
                BillboardSpace::View {
                    offset: original_offset,
                },
            ) => {
                assert!(restored_offset.abs_diff_eq(original_offset, 1e-5));
            }
            (BillboardSpace::World, BillboardSpace::World) => {}
            other => panic!("mismatched billboard spaces: {other:?}"),
        }

        let mut assets = Assets::new();
        let instance = restored.instantiate(None, &mut assets);
        let mut query = instance.world().query::<&Billboard>();
        let billboards: Vec<_> = query.iter().map(|(_, component)| *component).collect();
        assert_eq!(billboards.len(), 1);
        assert_eq!(billboards[0].orientation, billboard_component.orientation);
        assert_eq!(billboards[0].lit, billboard_component.lit);
        match (billboards[0].space, billboard_component.space) {
            (
                BillboardSpace::View {
                    offset: restored_offset,
                },
                BillboardSpace::View {
                    offset: original_offset,
                },
            ) => {
                assert!(restored_offset.abs_diff_eq(original_offset, 1e-5));
            }
            (BillboardSpace::World, BillboardSpace::World) => {}
            other => panic!("mismatched billboard spaces after instantiate: {other:?}"),
        }
    }

    #[test]
    fn serialize_world_preserves_billboard() {
        let mut world = World::new();
        let billboard = Billboard::new(BillboardOrientation::FaceCamera)
            .with_space(BillboardSpace::View {
                offset: Vec3::new(-4.0, 0.5, 2.25),
            })
            .with_lighting(false);

        world.spawn((
            TransformComponent(Transform::IDENTITY),
            Visible(true),
            billboard,
        ));

        let assets = Assets::new();
        let (entities, _) = serialize_world(&world, &assets);
        assert_eq!(entities.len(), 1);

        let serialized_billboard = entities[0]
            .billboard
            .expect("serialized world should contain billboard");
        let roundtrip = Billboard::from(serialized_billboard);
        assert_eq!(roundtrip.orientation, billboard.orientation);
        assert_eq!(roundtrip.lit, billboard.lit);
        match (roundtrip.space, billboard.space) {
            (
                BillboardSpace::View {
                    offset: restored_offset,
                },
                BillboardSpace::View {
                    offset: original_offset,
                },
            ) => assert!(restored_offset.abs_diff_eq(original_offset, 1e-5)),
            (BillboardSpace::World, BillboardSpace::World) => {}
            other => panic!("unexpected spaces from serialize_world: {other:?}"),
        }

        let mut asset = SceneAsset {
            name: "BillboardSerializeWorld".into(),
            root_transform: SerializedTransform::identity(),
            entities,
            animations: Vec::new(),
            animation_states: Vec::new(),
            mesh_data: Vec::new(),
            active_camera: None,
        };

        let project_root = tempdir().unwrap();
        asset
            .persist_material_assets(project_root.path())
            .expect("persist materials");

        let mut assets = Assets::new();
        let instance = asset.instantiate(None, &mut assets);
        let mut query = instance.world().query::<&Billboard>();
        let instances: Vec<_> = query.iter().map(|(_, component)| *component).collect();
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].orientation, billboard.orientation);
        assert_eq!(instances[0].lit, billboard.lit);
        match (instances[0].space, billboard.space) {
            (
                BillboardSpace::View {
                    offset: restored_offset,
                },
                BillboardSpace::View {
                    offset: original_offset,
                },
            ) => assert!(restored_offset.abs_diff_eq(original_offset, 1e-5)),
            (BillboardSpace::World, BillboardSpace::World) => {}
            other => panic!("unexpected spaces after instantiate: {other:?}"),
        }
    }

    #[test]
    fn project_manifest_roundtrip_preserves_billboard() {
        let billboard_component = Billboard::new(BillboardOrientation::FaceCameraYAxis)
            .with_space(BillboardSpace::View {
                offset: Vec3::new(0.0, 3.0, 1.0),
            })
            .with_lighting(true);
        let serialized_billboard = SerializedBillboard::from(billboard_component);

        let entity = SceneAssetEntity::builder(SerializedTransform::identity())
            .with_billboard(serialized_billboard)
            .build();

        let scene_asset = SceneAsset::builder("BillboardProject")
            .add_entity(entity)
            .build();

        let mut scene = Scene::new();
        let mut library = SceneLibrary::new();
        let node = scene.instantiate_asset(&mut library, &scene_asset, None, None);
        scene.set_main_scene(node);

        let manifest = ProjectManifest::capture(&scene, ProjectMetadata::default(), None)
            .expect("capture project manifest");

        let dir = tempdir().unwrap();
        manifest.save_to_dir(dir.path()).expect("save project");

        let reloaded = ProjectManifest::load_from_dir(dir.path()).expect("load project");
        let document = reloaded
            .scenes()
            .first()
            .expect("reloaded manifest should contain a scene");
        let asset = reloaded
            .scene_asset(&document.id)
            .expect("reloaded manifest should provide a scene asset");
        assert_eq!(asset.entities.len(), 1);
        let serialized = asset.entities[0]
            .billboard
            .expect("billboard should persist in project");
        let restored = Billboard::from(serialized);

        assert_eq!(restored.orientation, billboard_component.orientation);
        assert_eq!(restored.lit, billboard_component.lit);
        match (restored.space, billboard_component.space) {
            (
                BillboardSpace::View {
                    offset: restored_offset,
                },
                BillboardSpace::View {
                    offset: original_offset,
                },
            ) => assert!(restored_offset.abs_diff_eq(original_offset, 1e-5)),
            (BillboardSpace::World, BillboardSpace::World) => {}
            other => panic!("unexpected billboard space after project roundtrip: {other:?}"),
        }
    }

    #[test]
    fn scene_capture_roundtrip_preserves_billboard() {
        let billboard_component = Billboard::new(BillboardOrientation::FaceCamera)
            .with_space(BillboardSpace::View {
                offset: Vec3::new(2.0, -1.0, 0.5),
            })
            .with_lighting(false);
        let serialized_billboard = SerializedBillboard::from(billboard_component);

        let entity = SceneAssetEntity::builder(SerializedTransform::identity())
            .with_billboard(serialized_billboard)
            .build();

        let scene_asset = SceneAsset::builder("CapturedBillboard")
            .add_entity(entity)
            .build();

        let tree = SceneTreeAsset::new(
            "Root",
            SceneTreeAssetNode {
                name: "Root".into(),
                transform: SerializedTransform::identity(),
                asset: Some(scene_asset.clone()),
                scene_ref: None,
                children: Vec::new(),
            },
        );

        let mut scene = Scene::new();
        let mut library = SceneLibrary::new();
        let root_node = scene.instantiate_tree_asset(&mut library, &tree, None, None);
        scene.set_main_scene(root_node);

        let manifest = ProjectManifest::capture(&scene, ProjectMetadata::default(), None)
            .expect("capture project manifest");

        let dir = tempdir().unwrap();
        manifest
            .save_to_dir(dir.path())
            .expect("save captured project");
        let reloaded = ProjectManifest::load_from_dir(dir.path()).expect("load captured project");
        let document = reloaded
            .scenes()
            .first()
            .expect("reloaded manifest should contain a scene");
        let asset = reloaded
            .scene_asset(&document.id)
            .expect("reloaded manifest should provide a scene asset");

        let mut new_scene = Scene::new();
        let instantiated = new_scene.instantiate_asset(&mut library, asset, None, None);
        new_scene.set_main_scene(instantiated);
        new_scene.propagate_transforms();

        let mut query = new_scene.main_world().query::<&Billboard>();
        let billboards: Vec<_> = query.iter().map(|(_, component)| *component).collect();
        assert_eq!(billboards.len(), 1);
        assert_eq!(billboards[0].orientation, billboard_component.orientation);
        assert_eq!(billboards[0].lit, billboard_component.lit);
        match (billboards[0].space, billboard_component.space) {
            (
                BillboardSpace::View {
                    offset: restored_offset,
                },
                BillboardSpace::View {
                    offset: original_offset,
                },
            ) => assert!(restored_offset.abs_diff_eq(original_offset, 1e-5)),
            (BillboardSpace::World, BillboardSpace::World) => {}
            other => panic!("scene capture lost billboard space: {other:?}"),
        }
    }

    #[test]
    fn texture_indices_can_be_rebased_multiple_times() {
        let mut asset = SceneAsset {
            name: "Test".into(),
            root_transform: SerializedTransform::identity(),
            entities: vec![SceneAssetEntity {
                name: Some("Entity".into()),
                transform: SerializedTransform::identity(),
                visible: true,
                mesh_handle: None,
                primitive_mesh: None,
                mesh_bounds: None,
                material: None,
                material_data: Some(make_material(MaterialFlags::USE_BASE_COLOR_TEXTURE, 10)),
                parent: None,
                children: Vec::new(),
                gltf_node: None,
                gltf_material: None,
                gltf_source: None,
                gltf_primitive: None,
                script: None,
                directional_light: None,
                point_light: None,
                spot_light: None,
                casts_shadow: None,
                billboard: None,
                editor_id: None,
                particle_system: None,
                particle_emitter: None,
                particle_behavior: None,
                environment: None,
                camera: None,
                scene_ref: None,
            }],
            animations: Vec::new(),
            animation_states: Vec::new(),
            mesh_data: Vec::new(),
            active_camera: None,
        };

        let mut to_local = std::collections::HashMap::new();
        to_local.insert(10, 0);
        asset.apply_resource_mappings(0, &to_local);

        let material = asset.entities[0]
            .material_data
            .as_ref()
            .expect("material present");
        assert_eq!(material.base_color_texture.index, Some(0));

        let mut to_global = std::collections::HashMap::new();
        to_global.insert(0, 42);
        asset.apply_resource_mappings(0, &to_global);

        let material = asset.entities[0]
            .material_data
            .as_ref()
            .expect("material present");
        assert_eq!(material.base_color_texture.index, Some(42));
    }

    #[test]
    fn persist_material_assets_writes_meta_mapping() {
        let project_dir = tempdir().unwrap();
        let project_root = project_dir.path();
        let gltf_dir = project_root.join("content/models/sample");
        std::fs::create_dir_all(&gltf_dir).unwrap();

        let mut asset = SceneAsset {
            name: "Test".into(),
            root_transform: SerializedTransform::identity(),
            entities: vec![SceneAssetEntity {
                name: Some("Entity".into()),
                transform: SerializedTransform::identity(),
                visible: true,
                mesh_handle: None,
                primitive_mesh: None,
                mesh_bounds: None,
                material: None,
                material_data: Some(make_material(MaterialFlags::NONE, 0)),
                parent: None,
                children: Vec::new(),
                gltf_node: None,
                gltf_material: Some(0),
                gltf_source: Some(PathBuf::from("content/models/sample/scene.gltf")),
                gltf_primitive: None,
                script: None,
                directional_light: None,
                point_light: None,
                spot_light: None,
                casts_shadow: None,
                billboard: None,
                editor_id: None,
                particle_system: None,
                particle_emitter: None,
                particle_behavior: None,
                environment: None,
                camera: None,
                scene_ref: None,
            }],
            animations: Vec::new(),
            animation_states: Vec::new(),
            mesh_data: Vec::new(),
            active_camera: None,
        };

        asset.persist_material_assets(project_root).unwrap();

        let entity = &asset.entities[0];
        let material_handle = entity.material.as_ref().expect("material handle assigned");
        let meta_path = gltf_dir.join("meta.json");
        assert!(meta_path.exists(), "meta file should be created");

        let material_file = material_handle
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("material file name available");
        assert_eq!(material_file, "scene_000.mat.json");

        let contents = std::fs::read_to_string(meta_path).unwrap();
        let parsed: BTreeMap<String, ImportedGltfMeta> = serde_json::from_str(&contents).unwrap();
        let meta = parsed
            .get("scene.gltf")
            .expect("meta entry for glTF present");
        assert_eq!(
            meta.materials.get(&0),
            Some(&material_handle.path().to_path_buf()),
            "meta should point at generated material path"
        );
        assert_eq!(
            meta.materials_by_key.get("material:0"),
            Some(&material_handle.path().to_path_buf()),
            "keyed meta should point at generated material path"
        );
    }

    #[test]
    fn imported_meta_prefers_specific_binding() {
        let mut meta = ImportedGltfMeta::default();
        meta.record_material(1, PathBuf::from("by_index.mat.json"));

        let key = crate::scene::gltf_material_registry::GltfMaterialKey::new(
            PathBuf::from("content/models/sample/scene.gltf"),
            Some(4),
            Some(2),
            Some(1),
        );
        meta.record_material_key(&key, PathBuf::from("by_key.mat.json"));

        assert_eq!(
            meta.lookup_material_path(Some(4), Some(2), Some(1))
                .cloned(),
            Some(PathBuf::from("by_key.mat.json")),
        );

        assert_eq!(
            meta.lookup_material_path(None, None, Some(1)).cloned(),
            Some(PathBuf::from("by_index.mat.json")),
        );
    }

    #[test]
    fn persist_material_assets_serializes_relative_texture_paths() {
        let project_dir = tempdir().unwrap();
        let project_root = project_dir.path();

        let texture_dir = project_root.join(CONTENT_DIR).join("textures");
        fs::create_dir_all(&texture_dir).unwrap();
        let texture_path = texture_dir.join("albedo.png");
        fs::write(&texture_path, b"dummy").unwrap();

        let mut material = make_material(MaterialFlags::USE_BASE_COLOR_TEXTURE, 0);
        material.base_color_texture.path = Some(texture_path.clone());
        material
            .base_color_texture
            .name
            .get_or_insert_with(|| "albedo.png".to_string());

        let mut asset = SceneAsset {
            name: "Test".into(),
            root_transform: SerializedTransform::identity(),
            entities: vec![SceneAssetEntity {
                name: Some("Entity".into()),
                transform: SerializedTransform::identity(),
                visible: true,
                mesh_handle: None,
                primitive_mesh: None,
                mesh_bounds: None,
                material: None,
                material_data: Some(material),
                parent: None,
                children: Vec::new(),
                gltf_node: None,
                gltf_material: Some(0),
                gltf_source: Some(PathBuf::from("content/models/sample/scene.gltf")),
                gltf_primitive: None,
                script: None,
                directional_light: None,
                point_light: None,
                spot_light: None,
                casts_shadow: None,
                billboard: None,
                editor_id: None,
                particle_system: None,
                particle_emitter: None,
                particle_behavior: None,
                environment: None,
                camera: None,
                scene_ref: None,
            }],
            animations: Vec::new(),
            animation_states: Vec::new(),
            mesh_data: Vec::new(),
            active_camera: None,
        };

        asset.persist_material_assets(project_root).unwrap();

        let entity = &asset.entities[0];
        let expected_rel = PathBuf::from(CONTENT_DIR)
            .join("textures")
            .join("albedo.png");

        let stored_material = entity
            .material_data
            .as_ref()
            .expect("material data should be restored");
        assert_eq!(
            stored_material
                .base_color_texture
                .path
                .as_ref()
                .expect("path should be present"),
            &expected_rel
        );
        assert!(
            stored_material
                .base_color_texture
                .path
                .as_ref()
                .unwrap()
                .is_relative(),
            "stored material data should be relative"
        );

        let material_handle = entity.material.as_ref().expect("material handle assigned");
        let abs_material_path = project_root.join(material_handle.path());
        let contents = fs::read_to_string(abs_material_path).unwrap();
        let parsed: SerializedMaterial = serde_json::from_str(&contents).unwrap();
        assert_eq!(
            parsed
                .base_color_texture
                .path
                .as_ref()
                .expect("serialized material should contain texture path"),
            &expected_rel
        );
        assert!(
            parsed
                .base_color_texture
                .path
                .as_ref()
                .unwrap()
                .is_relative(),
            "material file should contain relative texture path"
        );
    }

    #[test]
    fn persist_material_assets_reuses_cached_paths_for_duplicate_materials() {
        let project_dir = tempdir().unwrap();
        let project_root = project_dir.path();
        let gltf_dir = project_root.join("content/models/sample");
        std::fs::create_dir_all(&gltf_dir).unwrap();

        let gltf_source = PathBuf::from("content/models/sample/My Scene 01.gltf");

        let mut asset = SceneAsset {
            name: "Test".into(),
            root_transform: SerializedTransform::identity(),
            entities: vec![
                SceneAssetEntity {
                    name: Some("Entity A".into()),
                    transform: SerializedTransform::identity(),
                    visible: true,
                    mesh_handle: None,
                    primitive_mesh: None,
                    mesh_bounds: None,
                    material: None,
                    material_data: Some(make_material(MaterialFlags::NONE, 0)),
                    parent: None,
                    children: Vec::new(),
                    gltf_node: None,
                    gltf_material: Some(0),
                    gltf_source: Some(gltf_source.clone()),
                    gltf_primitive: None,
                    script: None,
                    directional_light: None,
                    point_light: None,
                    spot_light: None,
                    casts_shadow: None,
                    billboard: None,
                    editor_id: None,
                    particle_system: None,
                    particle_emitter: None,
                    particle_behavior: None,
                    environment: None,
                    camera: None,
                    scene_ref: None,
                },
                SceneAssetEntity {
                    name: Some("Entity B".into()),
                    transform: SerializedTransform::identity(),
                    visible: true,
                    mesh_handle: None,
                    primitive_mesh: None,
                    mesh_bounds: None,
                    material: None,
                    material_data: Some(make_material(MaterialFlags::NONE, 0)),
                    parent: None,
                    children: Vec::new(),
                    gltf_node: None,
                    gltf_material: Some(0),
                    gltf_source: Some(gltf_source.clone()),
                    gltf_primitive: None,
                    script: None,
                    directional_light: None,
                    point_light: None,
                    spot_light: None,
                    casts_shadow: None,
                    billboard: None,
                    editor_id: None,
                    particle_system: None,
                    particle_emitter: None,
                    particle_behavior: None,
                    environment: None,
                    camera: None,
                    scene_ref: None,
                },
            ],
            animations: Vec::new(),
            animation_states: Vec::new(),
            mesh_data: Vec::new(),
            active_camera: None,
        };

        asset.persist_material_assets(project_root).unwrap();

        let first = asset.entities[0]
            .material
            .as_ref()
            .expect("first material assigned")
            .path()
            .to_path_buf();
        let second = asset.entities[1]
            .material
            .as_ref()
            .expect("second material assigned")
            .path()
            .to_path_buf();

        assert_eq!(first, second, "materials should reuse cached path");
        assert_eq!(
            first
                .file_name()
                .and_then(|name| name.to_str())
                .expect("file name present"),
            "my_scene_01_000.mat.json"
        );
    }

    #[test]
    fn persist_material_assets_adds_suffix_when_colliding() {
        let project_dir = tempdir().unwrap();
        let project_root = project_dir.path();
        let gltf_dir_a = project_root.join("content/models/a");
        let gltf_dir_b = project_root.join("content/models/b");
        std::fs::create_dir_all(&gltf_dir_a).unwrap();
        std::fs::create_dir_all(&gltf_dir_b).unwrap();

        let mut asset = SceneAsset {
            name: "Test".into(),
            root_transform: SerializedTransform::identity(),
            entities: vec![
                SceneAssetEntity {
                    name: Some("Entity A".into()),
                    transform: SerializedTransform::identity(),
                    visible: true,
                    mesh_handle: None,
                    primitive_mesh: None,
                    mesh_bounds: None,
                    material: None,
                    material_data: Some(make_material(MaterialFlags::NONE, 0)),
                    parent: None,
                    children: Vec::new(),
                    gltf_node: None,
                    gltf_material: Some(0),
                    gltf_source: Some(PathBuf::from("content/models/a/Scene.gltf")),
                    gltf_primitive: None,
                    script: None,
                    directional_light: None,
                    point_light: None,
                    spot_light: None,
                    casts_shadow: None,
                    billboard: None,
                    editor_id: None,
                    particle_system: None,
                    particle_emitter: None,
                    particle_behavior: None,
                    environment: None,
                    camera: None,
                    scene_ref: None,
                },
                SceneAssetEntity {
                    name: Some("Entity B".into()),
                    transform: SerializedTransform::identity(),
                    visible: true,
                    mesh_handle: None,
                    primitive_mesh: None,
                    mesh_bounds: None,
                    material: None,
                    material_data: Some(make_material(MaterialFlags::NONE, 0)),
                    parent: None,
                    children: Vec::new(),
                    gltf_node: None,
                    gltf_material: Some(0),
                    gltf_source: Some(PathBuf::from("content/models/b/Scene.gltf")),
                    gltf_primitive: None,
                    script: None,
                    directional_light: None,
                    point_light: None,
                    spot_light: None,
                    casts_shadow: None,
                    billboard: None,
                    editor_id: None,
                    particle_system: None,
                    particle_emitter: None,
                    particle_behavior: None,
                    environment: None,
                    camera: None,
                    scene_ref: None,
                },
            ],
            animations: Vec::new(),
            animation_states: Vec::new(),
            mesh_data: Vec::new(),
            active_camera: None,
        };

        asset.persist_material_assets(project_root).unwrap();

        let first_name = asset.entities[0]
            .material
            .as_ref()
            .expect("first material assigned")
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("file name present");
        let second_name = asset.entities[1]
            .material
            .as_ref()
            .expect("second material assigned")
            .path()
            .file_name()
            .and_then(|name| name.to_str())
            .expect("file name present");

        assert_eq!(first_name, "scene_000.mat.json");
        assert_eq!(second_name, "scene_000_1.mat.json");
    }
}
