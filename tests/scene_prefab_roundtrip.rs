use wgpu_cube::scene::{
    SceneAsset, SceneAssetEntity, ScenePrefabOverrides, ScenePrefabRef, SceneTreeAsset,
    SceneTreeAssetNode, SerializedTransform,
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
