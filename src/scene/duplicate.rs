//! Copying an entity and everything below it: the remote `entity.duplicate` and Lua's
//! `clone_entity` both go through [`duplicate_subtree`], so they copy the same
//! components.

use std::collections::{HashMap, HashSet};

use hecs::{Entity, World};

use crate::asset::{Handle, MaterialAsset};
use crate::scene::components::{Billboard, DepthState};
use crate::scene::skeletal::{AnimationTree, Animator, SkinnedMesh};
use crate::scene::{
    CameraComponent, CanCastShadow, Children, DirectionalLight, EnvironmentComponent, GltfSource,
    LightProbeVolume, MaterialComponent, MeshBounds, MeshComponent, Name, OrbitAnimation, Parent,
    ParticleEmitterComponent, ParticleSystemComponent, PointLight, PostProcessVolume,
    PrimitiveMeshComponent, RotateAnimation, SpotLight, TransformComponent, Visible,
};
use crate::scripting::LuaScriptComponent;

/// `root` and all its descendants, parents before children.
pub fn subtree(world: &World, root: Entity) -> Vec<Entity> {
    let mut order = Vec::new();
    let mut seen = HashSet::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if !world.contains(entity) || !seen.insert(entity) {
            continue;
        }
        order.push(entity);
        if let Ok(children) = world.get::<&Children>(entity) {
            stack.extend(children.0.iter().rev().copied());
        }
    }
    order
}

/// Copy `source` and its descendants. Each copy gets the components the engine knows
/// about; `material` maps the source's material to the copy's (share it, or make a copy).
/// Skinned meshes in the copy deform with the copied skeleton, and copied animators and
/// animation trees start their playback afresh (a playing animator's bone binding would
/// otherwise keep posing the original's bones). The copied root keeps the source's parent.
///
/// Returns `(original, copy)` pairs, the root first; empty when `source` does not exist.
pub fn duplicate_subtree(
    world: &mut World,
    source: Entity,
    mut material: impl FnMut(Handle<MaterialAsset>) -> Handle<MaterialAsset>,
) -> Vec<(Entity, Entity)> {
    let originals = subtree(world, source);
    let mut pairs = Vec::with_capacity(originals.len());
    for original in originals {
        let mut builder = copy_components(world, original);
        if let Ok(handle) = world.get::<&MaterialComponent>(original).map(|m| m.0) {
            builder.add(MaterialComponent(material(handle)));
        }
        pairs.push((original, world.spawn(builder.build())));
    }

    let copies: HashMap<Entity, Entity> = pairs.iter().copied().collect();
    let mapped = |entity: Entity| copies.get(&entity).copied();
    for &(original, copy) in &pairs {
        let parent = world.get::<&Parent>(original).ok().map(|p| p.0);
        let parent = if original == source {
            parent
        } else {
            parent.and_then(mapped)
        };
        if let Some(parent) = parent {
            attach(world, copy, parent);
        }
        if let Ok(mut skin) = world.get::<&mut SkinnedMesh>(copy) {
            skin.remap(mapped);
        }
    }
    pairs
}

/// Parent a fresh copy (no parent yet) under `parent`.
fn attach(world: &mut World, child: Entity, parent: Entity) {
    let _ = world.insert_one(child, Parent(parent));
    let listed = world
        .get::<&mut Children>(parent)
        .map(|mut children| children.0.push(child))
        .is_ok();
    if !listed {
        let _ = world.insert_one(parent, Children(vec![child]));
    }
}

/// The components the engine knows about, except the material and the hierarchy.
fn copy_components(world: &World, source: Entity) -> hecs::EntityBuilder {
    fn copy<T: hecs::Component + Clone>(
        world: &World,
        entity: Entity,
        builder: &mut hecs::EntityBuilder,
    ) {
        if let Ok(component) = world.get::<&T>(entity) {
            builder.add((*component).clone());
        }
    }

    let mut builder = hecs::EntityBuilder::new();
    copy::<Name>(world, source, &mut builder);
    copy::<TransformComponent>(world, source, &mut builder);
    copy::<Visible>(world, source, &mut builder);
    copy::<MeshComponent>(world, source, &mut builder);
    copy::<PrimitiveMeshComponent>(world, source, &mut builder);
    copy::<MeshBounds>(world, source, &mut builder);
    copy::<PointLight>(world, source, &mut builder);
    copy::<DirectionalLight>(world, source, &mut builder);
    copy::<SpotLight>(world, source, &mut builder);
    copy::<CanCastShadow>(world, source, &mut builder);
    copy::<CameraComponent>(world, source, &mut builder);
    copy::<LuaScriptComponent>(world, source, &mut builder);
    copy::<ParticleSystemComponent>(world, source, &mut builder);
    copy::<ParticleEmitterComponent>(world, source, &mut builder);
    copy::<EnvironmentComponent>(world, source, &mut builder);
    copy::<PostProcessVolume>(world, source, &mut builder);
    copy::<LightProbeVolume>(world, source, &mut builder);
    crate::physics::edit::clone_components(world, source, &mut builder);
    #[cfg(feature = "navigation")]
    crate::navigation::edit::clone_components(world, source, &mut builder);
    crate::ai::edit::clone_components(world, source, &mut builder);
    copy::<Billboard>(world, source, &mut builder);
    copy::<DepthState>(world, source, &mut builder);
    copy::<RotateAnimation>(world, source, &mut builder);
    copy::<OrbitAnimation>(world, source, &mut builder);
    copy::<GltfSource>(world, source, &mut builder);
    copy::<SkinnedMesh>(world, source, &mut builder);
    if let Ok(animator) = world.get::<&Animator>(source) {
        builder.add(Animator {
            state: Default::default(),
            ..(*animator).clone()
        });
    }
    if let Ok(tree) = world.get::<&AnimationTree>(source) {
        builder.add(AnimationTree {
            runtime: Default::default(),
            ..(*tree).clone()
        });
    }
    builder
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::skeletal::PlayOptions;
    use crate::scene::Transform;
    use glam::{Mat4, Vec3};
    use std::sync::Arc;

    #[test]
    fn copies_keep_their_own_skeleton_and_the_source_parent() {
        let mut world = World::new();
        let stage = world.spawn((Name::new("Stage"),));
        let root = world.spawn((
            Name::new("Dancer"),
            TransformComponent(Transform::from_translation(Vec3::X)),
            Animator::with_library("dance.glb"),
        ));
        attach(&mut world, root, stage);
        let hips = world.spawn((Name::new("Hips"), TransformComponent(Transform::IDENTITY)));
        attach(&mut world, hips, root);
        let body = world.spawn((
            Name::new("Body"),
            SkinnedMesh {
                joints: vec![hips],
                inverse_bind: Arc::from(vec![Mat4::IDENTITY]),
            },
        ));
        attach(&mut world, body, root);
        world
            .get::<&mut Animator>(root)
            .unwrap()
            .play("Dance", PlayOptions::default());

        let pairs = duplicate_subtree(&mut world, root, |handle| handle);
        assert_eq!(pairs.len(), 3);
        let copy = |original| pairs.iter().find(|(o, _)| *o == original).unwrap().1;
        let (copy_root, copy_hips, copy_body) = (copy(root), copy(hips), copy(body));

        assert_eq!(world.get::<&Parent>(copy_root).unwrap().0, stage);
        assert!(world
            .get::<&Children>(stage)
            .unwrap()
            .0
            .contains(&copy_root));
        assert_eq!(world.get::<&Parent>(copy_hips).unwrap().0, copy_root);
        assert_eq!(world.get::<&Parent>(copy_body).unwrap().0, copy_root);
        assert_eq!(
            world.get::<&SkinnedMesh>(copy_body).unwrap().joints,
            [copy_hips]
        );
        assert_eq!(world.get::<&SkinnedMesh>(body).unwrap().joints, [hips]);
        assert_eq!(
            world.get::<&Animator>(copy_root).unwrap().libraries,
            [std::path::PathBuf::from("dance.glb")]
        );
        assert_eq!(world.get::<&Name>(copy_hips).unwrap().0, "Hips");
        assert!(
            world
                .get::<&Animator>(copy_root)
                .unwrap()
                .state
                .layers
                .is_empty(),
            "the copy starts its own playback"
        );
        assert!(!world
            .get::<&Animator>(root)
            .unwrap()
            .state
            .layers
            .is_empty());
    }
}
