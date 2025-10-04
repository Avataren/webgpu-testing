// scene/scene.rs - Fixed version with improved transform propagation
use super::components::*;
use crate::asset::Assets;
use crate::renderer::{RenderBatcher, RenderObject, Renderer};
use crate::scene::Transform;
use glam::{Quat, Vec3};
use hecs::World;
use instant::Instant;

pub struct Scene {
    pub world: World,
    pub assets: Assets,
    time: f64,
    last_frame: Instant,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            world: World::new(),
            assets: Assets::default(),
            time: 0.0,
            last_frame: Instant::now(),
        }
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn last_frame(&self) -> Instant {
        self.last_frame
    }

    pub fn set_last_frame(&mut self, instant: Instant) {
        self.last_frame = instant;
    }

    pub fn update(&mut self, dt: f64) {
        self.time += dt;

        // Run animation systems BEFORE propagating transforms
        self.system_rotate_animation(dt);
        self.system_orbit_animation(dt);

        // CRITICAL: Always propagate transforms after animations
        self.system_propagate_transforms();
    }

    pub fn render(&mut self, renderer: &mut Renderer, batcher: &mut RenderBatcher) {
        batcher.clear();

        let mut world_transform_count = 0;
        let mut local_transform_count = 0;

        // Query for all visible renderable entities
        for (_entity, (mesh, material, visible)) in self
            .world
            .query::<(&MeshComponent, &MaterialComponent, &Visible)>()
            .iter()
        {
            if !visible.0 {
                continue;
            }

            //CRITICAL FIX: Always prefer WorldTransform if it exists
            //WorldTransform is the authoritative transform after propagation
            let transform = if let Ok(world_trans) = self.world.get::<&WorldTransform>(_entity) {
                // Entity is part of a hierarchy or has been processed
                world_transform_count += 1;
                world_trans.0
            } else if let Ok(local_trans) = self.world.get::<&TransformComponent>(_entity) {
                // Root entity without children - use local transform directly
                local_transform_count += 1;
                if let Ok(name) = self.world.get::<&Name>(_entity) {
                    log::warn!("Entity '{}' using LOCAL transform (no WorldTransform)", name.0);
                } else {
                    log::warn!("Entity {:?} using LOCAL transform (no WorldTransform)", _entity);
                }
                local_trans.0
            } else {
                // Fallback - should rarely happen
                log::warn!("Entity {:?} without transform", _entity);
                Transform::IDENTITY
            };


            batcher.add(RenderObject {
                mesh: mesh.0,
                material: material.0,
                transform,
            });
        }

        if local_transform_count > 0 {
            log::warn!(
                "Rendering: {} entities with WorldTransform, {} with LOCAL transform (BAD!)",
                world_transform_count,
                local_transform_count
            );
        }

        if let Err(e) = renderer.render(&self.assets, batcher) {
            log::error!("Render error: {:?}", e);
        }
    }

    // ========================================================================
    // Animation Systems
    // ========================================================================

    fn system_rotate_animation(&mut self, dt: f64) {
        for (_entity, (transform, anim)) in self
            .world
            .query::<(&mut TransformComponent, &RotateAnimation)>()
            .iter()
        {
            let rotation = Quat::from_axis_angle(anim.axis, anim.speed * dt as f32);
            transform.0.rotation = rotation * transform.0.rotation;
        }
    }

    fn system_orbit_animation(&mut self, dt: f64) {
        let time = self.time as f32;

        for (_entity, (transform, orbit)) in self
            .world
            .query::<(&mut TransformComponent, &OrbitAnimation)>()
            .iter()
        {
            let angle = time * orbit.speed + orbit.offset;
            transform.0.translation = orbit.center
                + Vec3::new(
                    angle.cos() * orbit.radius,
                    (time + orbit.offset).sin() * 0.5,
                    angle.sin() * orbit.radius,
                );
        }
    }

    // ========================================================================
    // Transform Propagation System (CRITICAL)
    // ========================================================================

    /// System: Propagate transforms through parent-child hierarchy
    /// This must run AFTER all animation systems and BEFORE rendering
    fn system_propagate_transforms(&mut self) {
        // Find all root entities (those without a Parent component)
        let roots: Vec<hecs::Entity> = self
            .world
            .query::<&TransformComponent>()
            .without::<&Parent>()
            .iter()
            .map(|(entity, _)| entity)
            .collect();

        log::trace!("Propagating transforms from {} root entities", roots.len());

        // Recursively compute world transforms from the roots
        for root in roots {
            // For root entities, world transform = local transform
            self.propagate_recursive(root, Transform::IDENTITY);
        }
    }

    fn propagate_recursive(&mut self, entity: hecs::Entity, parent_world: Transform) {
        // Get the local transform (clone it to drop the borrow immediately)
        let local = if let Ok(t) = self.world.get::<&TransformComponent>(entity) {
            t.0
        } else {
            // Entity has no transform - skip it and its children
            log::trace!("Entity {:?} has no TransformComponent, skipping", entity);
            return;
        };

        // Compute world transform by combining parent with local
        // parent_world is Transform::IDENTITY for root entities
        let world = if parent_world.translation == Vec3::ZERO 
            && parent_world.rotation == Quat::IDENTITY 
            && parent_world.scale == Vec3::ONE {
            // Parent is identity, just use local transform
            local
        } else {
            // Combine parent and local transforms
            parent_world.mul_transform(&local)
        };

        log::trace!(
            "Entity {:?}: local T:{:?}, world T:{:?}",
            entity,
            local.translation,
            world.translation
        );

        // Update or insert the WorldTransform component
        // We need to check if it exists first, then handle update vs insert
        let has_world_transform = self.world.get::<&WorldTransform>(entity).is_ok();
        
        if has_world_transform {
            // Update existing WorldTransform
            if let Ok(mut wt) = self.world.get::<&mut WorldTransform>(entity) {
                wt.0 = world;
            } else {
                log::error!("Failed to get mutable WorldTransform for entity {:?}", entity);
            }
        } else {
            // Insert new WorldTransform (borrow is dropped)
            if let Err(e) = self.world.insert_one(entity, WorldTransform(world)) {
                log::error!("Failed to insert WorldTransform for entity {:?}: {:?}", entity, e);
            } else {
                log::trace!("Inserted WorldTransform for entity {:?}", entity);
            }
        }

        // Clone children list before recursing to avoid borrow issues
        let children = if let Ok(children_comp) = self.world.get::<&Children>(entity) {
            children_comp.0.clone()
        } else {
            // No children, we're done
            return;
        };
        // Borrow is dropped here
        
        // Now we can recursively process children
        for child in children {
            self.propagate_recursive(child, world);
        }
    }

    // ========================================================================
    // Debug Utilities
    // ========================================================================

    pub fn debug_print_transforms(&self) {
        log::info!("=== Transform Debug ===");
        for (entity, (name, local, world)) in self
            .world
            .query::<(&Name, &TransformComponent, Option<&WorldTransform>)>()
            .iter()
        {
            log::info!(
                "{}: Local T:{:?} R:{:?} S:{:?}",
                name.0,
                local.0.translation,
                local.0.rotation,
                local.0.scale
            );
            if let Some(w) = world {
                log::info!(
                    "    World T:{:?} R:{:?} S:{:?}",
                    w.0.translation,
                    w.0.rotation,
                    w.0.scale
                );
            } else {
                log::info!("    World: NONE (root entity)");
            }
        }
        log::info!("=====================");
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::Transform;

    #[test]
    fn test_transform_propagation_simple() {
        let mut scene = Scene::new();

        // Create parent at (5, 0, 0)
        let parent = scene.world.spawn((
            Name::new("Parent"),
            TransformComponent(Transform::from_trs(
                Vec3::new(5.0, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::ONE,
            )),
        ));

        // Create child at local (2, 0, 0) - should be at world (7, 0, 0)
        let child = scene.world.spawn((
            Name::new("Child"),
            TransformComponent(Transform::from_trs(
                Vec3::new(2.0, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::ONE,
            )),
            Parent(parent),
        ));

        // Add children list to parent
        scene.world.insert_one(parent, Children(vec![child])).ok();

        // Run propagation
        scene.system_propagate_transforms();

        // Check world transforms
        let parent_world = scene.world.get::<&WorldTransform>(parent).unwrap();
        assert_eq!(parent_world.0.translation, Vec3::new(5.0, 0.0, 0.0));

        let child_world = scene.world.get::<&WorldTransform>(child).unwrap();
        assert_eq!(child_world.0.translation, Vec3::new(7.0, 0.0, 0.0));
    }

    #[test]
    fn test_transform_propagation_scale() {
        let mut scene = Scene::new();

        // Parent with 2x scale
        let parent = scene.world.spawn((
            Name::new("Parent"),
            TransformComponent(Transform::from_trs(
                Vec3::ZERO,
                Quat::IDENTITY,
                Vec3::splat(2.0),
            )),
        ));

        // Child at local (1, 0, 0) with 0.5x scale
        // Should be at world (2, 0, 0) with 1.0x scale
        let child = scene.world.spawn((
            Name::new("Child"),
            TransformComponent(Transform::from_trs(
                Vec3::new(1.0, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::splat(0.5),
            )),
            Parent(parent),
        ));

        scene.world.insert_one(parent, Children(vec![child])).ok();

        scene.system_propagate_transforms();

        let child_world = scene.world.get::<&WorldTransform>(child).unwrap();
        assert_eq!(child_world.0.translation, Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(child_world.0.scale, Vec3::splat(1.0));
    }
}