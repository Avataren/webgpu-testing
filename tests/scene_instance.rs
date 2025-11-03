use std::path::PathBuf;

use wgpu_cube::project::{SceneDocument, SceneDocumentDependencies};
use wgpu_cube::scene::{
    Scene, SceneAsset, SceneAssetEntity, SceneLibrary, ScenePrefabOverrides, ScenePrefabRef,
    SceneTreeAsset, SceneTreeAssetNode, SerializedTransform,
};

fn make_scene_document(id: &str, name: &str) -> SceneDocument {
    SceneDocument {
        id: id.into(),
        name: name.into(),
        relative_path: PathBuf::from(format!("{}.json", id)),
        dependencies: SceneDocumentDependencies::default(),
    }
}

#[test]
fn instantiating_scene_prefab_creates_entities_and_tracks_dependency() {
    let prefab_entity = SceneAssetEntity::builder(SerializedTransform::identity())
        .with_name("PrefabRoot")
        .build();
    let prefab_asset = SceneAsset::builder("PrefabAsset")
        .add_entity(prefab_entity)
        .build();

    let prefab_doc = make_scene_document("prefab.scene", "PrefabScene");
    let host_doc = make_scene_document("host.scene", "HostScene");

    let mut library = SceneLibrary::new();
    library.register_document(&prefab_doc);
    library.register_document(&host_doc);
    library.insert(prefab_doc.id.clone(), prefab_asset.clone());

    let mut scene = Scene::new();
    let prefab_ref = ScenePrefabRef {
        document: prefab_doc.id.clone(),
        node_path: vec!["PrefabRoot".into()],
        overrides: ScenePrefabOverrides::default(),
    };
    let tree_node = SceneTreeAssetNode::builder("InstanceRoot")
        .with_scene_ref(prefab_ref)
        .build();
    let tree_asset = SceneTreeAsset::new("PrefabInstance", tree_node);

    let node_id = scene.instantiate_tree_asset(
        &mut library,
        &tree_asset,
        Some(scene.main_scene()),
        Some(host_doc.id.as_str()),
    );

    let spawned_entity = scene
        .node_asset_entity(node_id, 0)
        .expect("prefab instantiation should produce an entity");
    assert_eq!(
        scene.node_for_entity(spawned_entity),
        Some(node_id),
        "entity should map back to its owning node"
    );

    let dependencies = library
        .prefab_dependencies(&host_doc.id)
        .expect("host document should have dependency tracking");
    assert!(
        dependencies.contains(&prefab_doc.id),
        "prefab instantiation should record dependency"
    );
}
