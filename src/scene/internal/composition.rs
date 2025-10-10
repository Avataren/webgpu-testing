use crate::scene::animation::AnimationTarget;
use crate::scene::components::{
    Children, GltfMaterial, GltfNode, MaterialComponent, MeshComponent, Name, OrbitAnimation,
    Parent, RotateAnimation, TransformComponent, Visible, WorldTransform,
};
use crate::scene::Scene;

pub(crate) fn merge_as_child(scene: &mut Scene, parent_entity: hecs::Entity, other: Scene) {
    let entity_count = other.world.len();
    log::info!("Merging scene with {} entities as child", entity_count);

    let (
        other_world,
        other_assets,
        _other_environment,
        mut other_animations,
        mut other_animation_states,
    ) = other.into_parts();

    let mut entity_map = std::collections::HashMap::new();

    let entities_to_copy: Vec<_> = other_world
        .iter()
        .map(|entity_ref| entity_ref.entity())
        .collect();

    for old_entity in entities_to_copy {
        let mut builder = hecs::EntityBuilder::new();

        if let Ok(name) = other_world.get::<&Name>(old_entity) {
            builder.add(Name(name.0.clone()));
        }
        if let Ok(transform) = other_world.get::<&TransformComponent>(old_entity) {
            builder.add(*transform);
        }
        if let Ok(mesh) = other_world.get::<&MeshComponent>(old_entity) {
            builder.add(*mesh);
        }
        if let Ok(material) = other_world.get::<&MaterialComponent>(old_entity) {
            builder.add(*material);
        }
        if let Ok(gltf_node) = other_world.get::<&GltfNode>(old_entity) {
            builder.add(*gltf_node);
        }
        if let Ok(gltf_material) = other_world.get::<&GltfMaterial>(old_entity) {
            builder.add(*gltf_material);
        }
        if let Ok(visible) = other_world.get::<&Visible>(old_entity) {
            builder.add(*visible);
        }
        if let Ok(rotate) = other_world.get::<&RotateAnimation>(old_entity) {
            builder.add(*rotate);
        }
        if let Ok(orbit) = other_world.get::<&OrbitAnimation>(old_entity) {
            builder.add(*orbit);
        }
        if let Ok(world_trans) = other_world.get::<&WorldTransform>(old_entity) {
            builder.add(*world_trans);
        }

        let new_entity = scene.world.spawn(builder.build());
        entity_map.insert(old_entity, new_entity);
    }

    let parent_children_to_fix: Vec<_> = entity_map
        .iter()
        .map(|(old, &new)| {
            let parent = other_world.get::<&Parent>(*old).ok().map(|p| p.0);
            let children = other_world.get::<&Children>(*old).ok().map(|c| c.0.clone());
            (new, parent, children)
        })
        .collect();

    let mut root_entities = Vec::new();

    for (new_entity, parent, children) in parent_children_to_fix {
        if let Some(old_parent) = parent {
            if let Some(&new_parent) = entity_map.get(&old_parent) {
                scene.world.insert_one(new_entity, Parent(new_parent)).ok();
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
                scene
                    .world
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
            scene.world.insert_one(root, Parent(parent_entity)).ok();
        }

        let has_children = scene.world.get::<&Children>(parent_entity).is_ok();

        if has_children {
            if let Ok(mut parent_children) = scene.world.get::<&mut Children>(parent_entity) {
                parent_children.0.extend(&root_entities);
            }
        } else {
            scene
                .world
                .insert_one(parent_entity, Children(root_entities))
                .ok();
        }
    }

    let animation_offset = scene.animations().len();
    for mut clip in other_animations.drain(..) {
        for channel in clip.channels.iter_mut() {
            if let AnimationTarget::Transform { entity, property } = channel.target {
                if let Some(&new_entity) = entity_map.get(&entity) {
                    channel.target = AnimationTarget::Transform {
                        entity: new_entity,
                        property,
                    };
                } else {
                    log::warn!(
                        "Skipping animation channel targeting entity {:?} missing from merge",
                        entity
                    );
                }
            }
        }
        scene.animations_mut().push(clip);
    }

    for mut state in other_animation_states.drain(..) {
        state.clip_index += animation_offset;
        scene.animation_states_mut().push(state);
    }

    log::info!(
        "Merged {} meshes, {} textures",
        other_assets.meshes.len(),
        other_assets.textures.len()
    );
}
