use glam::Vec3;
use wgpu_cube::scene::{
    Scene, SceneAsset, SceneAssetEntity, SceneLibrary, ScenePrefabOverrides, ScenePrefabRef,
    SceneTreeAsset, SceneTreeAssetNode, SerializedTransform, Visible,
};

fn prefab_overrides_sample() -> ScenePrefabOverrides {
    let transform = SerializedTransform {
        translation: [1.0, 2.0, 3.0],
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [2.0, 2.0, 2.0],
    };

    ScenePrefabOverrides {
        transform: Some(transform),
        visible: Some(false),
        casts_shadow: Some(true),
        ..ScenePrefabOverrides::default()
    }
}

#[test]
fn scene_asset_roundtrip_without_prefab_reference() {
    let entity = SceneAssetEntity::builder(SerializedTransform::identity())
        .with_name("Standalone")
        .build();

    let asset = SceneAsset::builder("NoPrefab").add_entity(entity).build();

    let json = asset.to_json().expect("asset should serialize");
    assert!(
        !json.contains("\"scene_ref\""),
        "legacy assets should not emit prefab references"
    );

    let restored = SceneAsset::from_json(&json).expect("asset should deserialize");
    assert!(restored.entities[0].scene_ref.is_none());

    let node = SceneTreeAssetNode::builder("Root")
        .with_asset(restored.clone())
        .build();
    let tree = SceneTreeAsset::new("Tree", node);
    let tree_json = tree.to_json().expect("tree should serialize");
    assert!(
        !tree_json.contains("\"scene_ref\""),
        "tree nodes without prefab references should not emit the field"
    );

    let restored_tree = SceneTreeAsset::from_json(&tree_json).expect("tree should deserialize");
    assert!(restored_tree.root.scene_ref.is_none());
}

#[test]
fn scene_asset_roundtrip_with_prefab_reference() {
    let overrides = prefab_overrides_sample();
    let prefab_ref = ScenePrefabRef {
        document: "prefabs".into(),
        node_path: vec!["Root".into(), "Light".into()],
        overrides: overrides.clone(),
    };

    let entity = SceneAssetEntity::builder(SerializedTransform::identity())
        .with_name("PrefabUser")
        .with_scene_ref(prefab_ref.clone())
        .build();

    let asset = SceneAsset::builder("WithPrefab").add_entity(entity).build();

    let json = asset.to_json().expect("asset should serialize");
    assert!(json.contains("\"scene_ref\""));

    let restored = SceneAsset::from_json(&json).expect("asset should deserialize");
    let restored_ref = restored.entities[0]
        .scene_ref
        .as_ref()
        .expect("prefab reference should survive roundtrip");
    assert_eq!(restored_ref.document, prefab_ref.document);
    assert_eq!(restored_ref.node_path, prefab_ref.node_path);
    assert_eq!(
        restored_ref
            .overrides
            .transform
            .as_ref()
            .expect("transform override should survive")
            .translation,
        [1.0, 2.0, 3.0]
    );
    assert_eq!(restored_ref.overrides.visible, Some(false));
    assert_eq!(restored_ref.overrides.casts_shadow, Some(true));

    let node = SceneTreeAssetNode::builder("PrefabNode")
        .with_scene_ref(prefab_ref)
        .build();
    let tree = SceneTreeAsset::new("PrefabTree", node);
    let tree_json = tree.to_json().expect("tree should serialize");
    assert!(tree_json.contains("\"scene_ref\""));

    let restored_tree = SceneTreeAsset::from_json(&tree_json).expect("tree should deserialize");
    let tree_ref = restored_tree
        .root
        .scene_ref
        .as_ref()
        .expect("tree node prefab ref should survive");
    assert_eq!(tree_ref.document, "prefabs");
    assert_eq!(tree_ref.node_path, vec!["Root", "Light"]);
    assert_eq!(tree_ref.overrides.visible, Some(false));
}

fn make_transform(translation: [f32; 3]) -> SerializedTransform {
    SerializedTransform {
        translation,
        rotation: [0.0, 0.0, 0.0, 1.0],
        scale: [1.0, 1.0, 1.0],
    }
}

#[test]
fn nested_prefab_instantiation_applies_overrides() {
    let grandchild_entity = SceneAssetEntity::builder(SerializedTransform::identity())
        .with_name("GrandRoot")
        .with_visibility(true)
        .build();
    let grandchild_asset = SceneAsset::builder("Grandchild")
        .add_entity(grandchild_entity)
        .build();

    let child_prefab = ScenePrefabRef {
        document: "grandchild".into(),
        node_path: vec!["GrandRoot".into()],
        overrides: ScenePrefabOverrides {
            visible: Some(false),
            ..ScenePrefabOverrides::default()
        },
    };
    let child_entity = SceneAssetEntity::builder(SerializedTransform::identity())
        .with_name("ChildRoot")
        .with_scene_ref(child_prefab)
        .build();
    let child_asset = SceneAsset::builder("Child")
        .add_entity(child_entity)
        .build();

    let root_prefab = ScenePrefabRef {
        document: "child".into(),
        node_path: vec!["ChildRoot".into()],
        overrides: ScenePrefabOverrides {
            transform: Some(make_transform([1.0, 2.0, 3.0])),
            ..ScenePrefabOverrides::default()
        },
    };
    let root_entity = SceneAssetEntity::builder(SerializedTransform::identity())
        .with_name("RootPrefab")
        .with_scene_ref(root_prefab)
        .build();
    let root_asset = SceneAsset::builder("Root").add_entity(root_entity).build();

    let mut library = SceneLibrary::new();
    library.insert("grandchild", grandchild_asset);
    library.insert("child", child_asset);
    library.insert("root", root_asset.clone());

    let mut scene = Scene::new();
    let root_node = scene.instantiate_asset(&mut library, &root_asset, None, Some("root"));
    scene.set_main_scene(root_node);

    let child_nodes = scene.node_children(root_node);
    assert_eq!(
        child_nodes.len(),
        1,
        "root prefab should spawn child prefab"
    );
    let child_node = child_nodes[0];
    let grandchild_nodes = scene.node_children(child_node);
    assert_eq!(grandchild_nodes.len(), 1, "nested prefab should spawn");

    let child_transform = scene.node_local_transform(child_node);
    assert!(child_transform
        .translation
        .abs_diff_eq(Vec3::new(1.0, 2.0, 3.0), 1e-5));

    let grandchild_node = grandchild_nodes[0];
    let child_entity = scene
        .node_asset_entity(grandchild_node, 0)
        .expect("grandchild entity should exist");
    let visible = scene
        .node_instance_world(grandchild_node)
        .expect("grandchild node should expose instance world")
        .get::<&Visible>(child_entity)
        .expect("visible component should be present")
        .0;
    assert!(!visible, "visible override should apply to prefab root");

    let root_deps = library
        .prefab_dependencies("root")
        .expect("dependencies should exist for root document");
    assert!(root_deps.contains("child"));
    let child_deps = library
        .prefab_dependencies("child")
        .expect("dependencies should exist for child document");
    assert!(child_deps.contains("grandchild"));
}

#[test]
fn prefab_instantiation_respects_parent_chain_transform() {
    let prefab_root = SceneAssetEntity::builder(SerializedTransform::identity())
        .with_name("PrefabRoot")
        .build();
    let prefab_asset = SceneAsset::builder("Prefab")
        .add_entity(prefab_root)
        .build();

    let prefab_ref = ScenePrefabRef {
        document: "prefab".into(),
        node_path: vec!["PrefabRoot".into()],
        overrides: ScenePrefabOverrides::default(),
    };

    let root_entity = SceneAssetEntity::builder(SerializedTransform::identity())
        .with_name("HostRoot")
        .with_children(vec![1])
        .build();
    let intermediate_entity = SceneAssetEntity::builder(make_transform([2.0, 0.0, 0.0]))
        .with_name("Intermediate")
        .with_parent(0)
        .with_children(vec![2])
        .build();
    let placeholder_entity = SceneAssetEntity::builder(make_transform([0.0, 4.0, 0.0]))
        .with_name("PrefabHolder")
        .with_parent(1)
        .with_scene_ref(prefab_ref)
        .build();

    let host_asset = SceneAsset::builder("Host")
        .add_entity(root_entity)
        .add_entity(intermediate_entity)
        .add_entity(placeholder_entity)
        .build();

    let mut library = SceneLibrary::new();
    library.insert("prefab", prefab_asset);
    library.insert("host", host_asset.clone());

    let mut scene = Scene::new();
    let host_node = scene.instantiate_asset(&mut library, &host_asset, None, Some("host"));
    scene.set_main_scene(host_node);

    let children = scene.node_children(host_node);
    assert_eq!(
        children.len(),
        1,
        "prefab should instantiate under host root"
    );

    let prefab_node = children[0];
    let prefab_transform = scene.node_local_transform(prefab_node);
    assert!(prefab_transform
        .translation
        .abs_diff_eq(Vec3::new(2.0, 4.0, 0.0), 1e-5));
}

#[test]
fn prefab_cycle_detection_prevents_recursion() {
    let child_ref = ScenePrefabRef {
        document: "b".into(),
        node_path: vec!["B".into()],
        overrides: ScenePrefabOverrides::default(),
    };
    let asset_a = SceneAsset::builder("A")
        .add_entity(
            SceneAssetEntity::builder(SerializedTransform::identity())
                .with_name("A")
                .with_scene_ref(child_ref)
                .build(),
        )
        .build();

    let root_ref = ScenePrefabRef {
        document: "a".into(),
        node_path: vec!["A".into()],
        overrides: ScenePrefabOverrides::default(),
    };
    let asset_b = SceneAsset::builder("B")
        .add_entity(
            SceneAssetEntity::builder(SerializedTransform::identity())
                .with_name("B")
                .with_scene_ref(root_ref)
                .build(),
        )
        .build();

    let mut library = SceneLibrary::new();
    library.insert("a", asset_a.clone());
    library.insert("b", asset_b.clone());

    let mut scene = Scene::new();
    let root = scene.instantiate_asset(&mut library, &asset_a, None, Some("a"));
    let children = scene.node_children(root);
    assert_eq!(children.len(), 1, "first-level prefab should instantiate");
    let cycle_child = children[0];
    assert!(
        scene.node_children(cycle_child).is_empty(),
        "cycle should prevent nested prefab instantiation"
    );
}
