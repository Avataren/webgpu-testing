use crate::scene::components::{
    Children, GltfMaterial, GltfNode, MaterialComponent, MeshComponent, Name, OrbitAnimation,
    Parent, RotateAnimation, TransformComponent, Visible, WorldTransform,
};
use hecs::World;
use std::collections::HashMap;

pub(crate) fn merge_world_as_child(
    target_world: &mut World,
    parent_entity: hecs::Entity,
    source_world: World,
) -> HashMap<hecs::Entity, hecs::Entity> {
    let entity_count = source_world.iter().count();
    log::info!("Merging scene with {entity_count} entities as child");

    let mut entity_map = HashMap::new();
    let entities_to_copy: Vec<_> = source_world
        .iter()
        .map(|entity_ref| entity_ref.entity())
        .collect();

    for old_entity in entities_to_copy {
        let mut builder = hecs::EntityBuilder::new();

        if let Ok(name) = source_world.get::<&Name>(old_entity) {
            builder.add(Name(name.0.clone()));
        }
        if let Ok(transform) = source_world.get::<&TransformComponent>(old_entity) {
            builder.add(*transform);
        }
        if let Ok(mesh) = source_world.get::<&MeshComponent>(old_entity) {
            builder.add(*mesh);
        }
        if let Ok(material) = source_world.get::<&MaterialComponent>(old_entity) {
            builder.add(*material);
        }
        if let Ok(gltf_node) = source_world.get::<&GltfNode>(old_entity) {
            builder.add(*gltf_node);
        }
        if let Ok(gltf_material) = source_world.get::<&GltfMaterial>(old_entity) {
            builder.add(*gltf_material);
        }
        if let Ok(visible) = source_world.get::<&Visible>(old_entity) {
            builder.add(*visible);
        }
        if let Ok(rotate) = source_world.get::<&RotateAnimation>(old_entity) {
            builder.add(*rotate);
        }
        if let Ok(orbit) = source_world.get::<&OrbitAnimation>(old_entity) {
            builder.add(*orbit);
        }
        if let Ok(world_trans) = source_world.get::<&WorldTransform>(old_entity) {
            builder.add(*world_trans);
        }

        let new_entity = target_world.spawn(builder.build());
        entity_map.insert(old_entity, new_entity);
    }

    let parent_children_to_fix: Vec<_> = entity_map
        .iter()
        .map(|(old, &new)| {
            let parent = source_world.get::<&Parent>(*old).ok().map(|p| p.0);
            let children = source_world
                .get::<&Children>(*old)
                .ok()
                .map(|c| c.0.clone());
            (new, parent, children)
        })
        .collect();

    let mut root_entities = Vec::new();

    for (new_entity, parent, children) in parent_children_to_fix {
        if let Some(old_parent) = parent {
            if let Some(&new_parent) = entity_map.get(&old_parent) {
                target_world.insert_one(new_entity, Parent(new_parent)).ok();
            } else {
                root_entities.push(new_entity);
            }
        } else {
            root_entities.push(new_entity);
        }

        if let Some(old_children) = children {
            let new_children: Vec<_> = old_children
                .iter()
                .filter_map(|old_child| entity_map.get(old_child).copied())
                .collect();

            if !new_children.is_empty() {
                target_world
                    .insert_one(new_entity, Children(new_children))
                    .ok();
            }
        }
    }

    if !root_entities.is_empty() {
        log::info!(
            "Setting {} root entities as children of parent",
            root_entities.len()
        );

        for &root in &root_entities {
            target_world.insert_one(root, Parent(parent_entity)).ok();
        }

        let has_children = target_world.get::<&Children>(parent_entity).is_ok();

        if has_children {
            if let Ok(mut parent_children) = target_world.get::<&mut Children>(parent_entity) {
                parent_children.0.extend(root_entities.iter().copied());
            }
        } else {
            target_world
                .insert_one(parent_entity, Children(root_entities))
                .ok();
        }
    }

    drop(source_world);
    entity_map
}
