// scene/scene.rs - Fixed version with improved transform propagation
use super::components::*;
use crate::asset::Assets;
use crate::renderer::{
    DirectionalShadowData, LightsData, PointShadowData, RenderBatcher, RenderObject, Renderer,
    SpotShadowData,
};
use crate::scene::Transform;
use crate::time::Instant;
use glam::{Mat4, Quat, Vec3};
use hecs::World;

pub struct Scene {
    pub world: World,
    pub assets: Assets,
    time: f64,
    last_frame: Option<Instant>,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            world: World::new(),
            assets: Assets::default(),
            time: 0.0,
            last_frame: None, // Start as None - will be initialized later
        }
    }

    /// Initialize the timer (must be called before first update)
    pub fn init_timer(&mut self) {
        self.last_frame = Some(Instant::now());
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn last_frame(&self) -> Instant {
        self.last_frame
            .expect("Scene timer not initialized - call init_timer() first")
    }

    pub fn set_last_frame(&mut self, instant: Instant) {
        self.last_frame = Some(instant);
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
        for (entity, (mesh, material, visible, world_transform, local_transform, name)) in self
            .world
            .query::<(
                &MeshComponent,
                &MaterialComponent,
                &Visible,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
                Option<&Name>,
            )>()
            .iter()
        {
            if !visible.0 {
                continue;
            }

            // CRITICAL FIX: Always prefer WorldTransform if it exists
            // WorldTransform is the authoritative transform after propagation
            let transform = if let Some(world_trans) = world_transform {
                // Entity is part of a hierarchy or has been processed
                world_transform_count += 1;
                world_trans.0
            } else if let Some(local_trans) = local_transform {
                // Root entity without children - use local transform directly
                local_transform_count += 1;
                if let Some(name) = name {
                    log::warn!(
                        "Entity '{}' using LOCAL transform (no WorldTransform)",
                        name.0.as_str()
                    );
                } else {
                    log::warn!(
                        "Entity {:?} using LOCAL transform (no WorldTransform)",
                        entity
                    );
                }
                local_trans.0
            } else {
                // Fallback - should rarely happen
                log::warn!("Entity {:?} without transform", entity);
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

        let mut lights = LightsData::default();

        for (_entity, (light, world_transform, local_transform, shadow_flag)) in self
            .world
            .query::<(
                &DirectionalLight,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
                Option<&CanCastShadow>,
            )>()
            .iter()
        {
            let transform = world_transform
                .map(|t| t.0)
                .or_else(|| local_transform.map(|t| t.0))
                .unwrap_or(Transform::IDENTITY);
            let raw_dir = transform.rotation * Vec3::NEG_Z;
            let direction = if raw_dir.length_squared() > 0.0 {
                raw_dir.normalize()
            } else {
                Vec3::new(0.0, -1.0, 0.0)
            };

            let shadow = shadow_flag
                .filter(|flag| flag.0)
                .map(|_| Self::build_directional_shadow(transform.translation, direction));

            lights.add_directional(direction, light.color, light.intensity, shadow);
        }

        for (_entity, (light, world_transform, local_transform, shadow_flag)) in self
            .world
            .query::<(
                &PointLight,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
                Option<&CanCastShadow>,
            )>()
            .iter()
        {
            let transform = world_transform
                .map(|t| t.0)
                .or_else(|| local_transform.map(|t| t.0))
                .unwrap_or(Transform::IDENTITY);

            let shadow = shadow_flag
                .filter(|flag| flag.0)
                .map(|_| Self::build_point_shadow(transform.translation, light.range));

            lights.add_point(
                transform.translation,
                light.color,
                light.intensity,
                light.range,
                shadow,
            );
        }

        for (_entity, (light, world_transform, local_transform, shadow_flag)) in self
            .world
            .query::<(
                &SpotLight,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
                Option<&CanCastShadow>,
            )>()
            .iter()
        {
            let transform = world_transform
                .map(|t| t.0)
                .or_else(|| local_transform.map(|t| t.0))
                .unwrap_or(Transform::IDENTITY);
            let raw_dir = transform.rotation * Vec3::NEG_Z;
            let direction = if raw_dir.length_squared() > 0.0 {
                raw_dir.normalize()
            } else {
                Vec3::new(0.0, -1.0, 0.0)
            };

            let shadow = shadow_flag
                .filter(|flag| flag.0)
                .map(|_| Self::build_spot_shadow(transform.translation, direction, light));

            lights.add_spot(
                transform.translation,
                direction,
                light.color,
                light.intensity,
                light.range,
                light.inner_angle,
                light.outer_angle,
                shadow,
            );
        }

        renderer.set_lights(&lights);

        if let Err(e) = renderer.render(&self.assets, batcher, &lights) {
            log::error!("Render error: {:?}", e);
        }
    }

    fn build_directional_shadow(position: Vec3, direction: Vec3) -> DirectionalShadowData {
        const SHADOW_DISTANCE: f32 = 40.0;
        const NEAR_PLANE: f32 = 0.1;
        let focus = position;
        let light_position = focus - direction * SHADOW_DISTANCE;
        let up = Self::shadow_up(direction);
        let view = Mat4::look_at_rh(light_position, focus, up);
        let projection = Mat4::orthographic_rh(
            -SHADOW_DISTANCE,
            SHADOW_DISTANCE,
            -SHADOW_DISTANCE,
            SHADOW_DISTANCE,
            NEAR_PLANE,
            SHADOW_DISTANCE * 2.0,
        );

        DirectionalShadowData {
            view_proj: projection * view,
            bias: 0.0005,
        }
    }

    fn build_point_shadow(position: Vec3, range: f32) -> PointShadowData {
        use std::f32::consts::FRAC_PI_2;

        let near = 0.1f32;
        let far = range.max(near + 0.1);
        let projection = Mat4::perspective_rh(FRAC_PI_2, 1.0, near, far);

        let dirs = [
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ];
        let ups = [Vec3::Y, Vec3::Y, Vec3::Z, Vec3::NEG_Z, Vec3::Y, Vec3::Y];

        let mut matrices = [Mat4::IDENTITY; 6];
        for ((matrix, dir), up) in matrices.iter_mut().zip(dirs.iter()).zip(ups.iter()) {
            let view = Mat4::look_at_rh(position, position + *dir, *up);
            *matrix = projection * view;
        }

        PointShadowData {
            view_proj: matrices,
            bias: 0.001,
            near,
            far,
        }
    }

    fn build_spot_shadow(position: Vec3, direction: Vec3, light: &SpotLight) -> SpotShadowData {
        let near = 0.1f32;
        let far = light.range.max(near + 0.1);
        let fov = (light.outer_angle * 2.0).clamp(0.1, std::f32::consts::PI - 0.1);
        let up = Self::shadow_up(direction);
        let view = Mat4::look_at_rh(position, position + direction, up);
        let projection = Mat4::perspective_rh(fov, 1.0, near, far);

        SpotShadowData {
            view_proj: projection * view,
            bias: 0.0007,
        }
    }

    fn shadow_up(direction: Vec3) -> Vec3 {
        let up = Vec3::Y;
        if direction.abs().dot(up) > 0.95 {
            Vec3::Z
        } else {
            up
        }
    }

    /// Ensure the scene has a reasonable default lighting setup.
    ///
    /// Returns the number of lights that were created. If the scene already
    /// contains any light components, no additional lights will be spawned.
    pub fn add_default_lighting(&mut self) -> usize {
        if self.has_any_lights() {
            return 0;
        }

        log::info!("No lights found in scene - adding default lighting setup");

        let mut created = 0usize;

        // Key directional light coming from above-right.
        let key_direction = Vec3::new(0.5, 0.8, 0.3);
        let key_rotation = Self::rotation_from_light_direction(key_direction);
        self.world.spawn((
            Name::new("Default Key Light"),
            TransformComponent(Transform::from_trs(Vec3::ZERO, key_rotation, Vec3::ONE)),
            DirectionalLight {
                color: Vec3::splat(1.0),
                intensity: 2.5,
            },
            CanCastShadow(true),
        ));
        created += 1;

        // Soft fill point light near the camera position.
        self.world.spawn((
            Name::new("Default Fill Light"),
            TransformComponent(Transform::from_trs(
                Vec3::new(0.0, 2.5, 6.0),
                Quat::IDENTITY,
                Vec3::ONE,
            )),
            PointLight {
                color: Vec3::new(0.9, 0.95, 1.0),
                intensity: 1.5,
                range: 25.0,
            },
            CanCastShadow(true),
        ));
        created += 1;

        // Rim directional light from behind for edge definition.
        let rim_direction = Vec3::new(-0.3, 0.2, -0.5);
        let rim_rotation = Self::rotation_from_light_direction(rim_direction);
        self.world.spawn((
            Name::new("Default Rim Light"),
            TransformComponent(Transform::from_trs(Vec3::ZERO, rim_rotation, Vec3::ONE)),
            DirectionalLight {
                color: Vec3::new(1.0, 0.95, 0.9),
                intensity: 1.0,
            },
            CanCastShadow(true),
        ));
        created += 1;

        created
    }

    fn has_any_lights(&self) -> bool {
        if self
            .world
            .query::<&DirectionalLight>()
            .iter()
            .next()
            .is_some()
        {
            return true;
        }

        if self.world.query::<&PointLight>().iter().next().is_some() {
            return true;
        }

        if self.world.query::<&SpotLight>().iter().next().is_some() {
            return true;
        }

        false
    }

    fn rotation_from_light_direction(direction: Vec3) -> Quat {
        let dir = if direction.length_squared() > 0.0 {
            direction.normalize()
        } else {
            Vec3::new(0.0, -1.0, 0.0)
        };

        let target = (-dir).normalize();
        Quat::from_rotation_arc(Vec3::NEG_Z, target)
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

    fn system_orbit_animation(&mut self, _dt: f64) {
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

        let mut stack: Vec<(hecs::Entity, Transform)> = Vec::new();

        for root in roots {
            stack.push((root, Transform::IDENTITY));

            while let Some((entity, parent_world)) = stack.pop() {
                // Get the local transform (copy it to drop the borrow immediately)
                let local = match self.world.get::<&TransformComponent>(entity) {
                    Ok(t) => t.0,
                    Err(_) => {
                        // Entity has no transform - skip it and its children
                        log::trace!("Entity {:?} has no TransformComponent, skipping", entity);
                        continue;
                    }
                };

                // Combine parent and local transforms
                let world = parent_world.mul_transform(&local);

                log::trace!(
                    "Entity {:?}: local T:{:?}, world T:{:?}",
                    entity,
                    local.translation,
                    world.translation
                );

                let mut has_world_transform = false;
                {
                    if let Ok(mut wt) = self.world.get::<&mut WorldTransform>(entity) {
                        wt.0 = world;
                        has_world_transform = true;
                    }
                }

                if !has_world_transform {
                    if let Err(e) = self.world.insert_one(entity, WorldTransform(world)) {
                        log::error!(
                            "Failed to insert WorldTransform for entity {:?}: {:?}",
                            entity,
                            e
                        );
                        continue;
                    } else {
                        log::trace!("Inserted WorldTransform for entity {:?}", entity);
                    }
                }

                if let Ok(children) = self.world.get::<&Children>(entity) {
                    for &child in children.0.iter().rev() {
                        stack.push((child, world));
                    }
                }
            }
        }
    }

    // ========================================================================
    // Scene Composition
    // ========================================================================

    /// Add all entities from another scene as children of a parent entity
    /// This allows composing complex scenes from smaller scene hierarchies
    pub fn merge_as_child(&mut self, parent_entity: hecs::Entity, mut other: Scene) {
        log::info!("Merging scene with {} entities as child", other.world.len());

        // Map old entity IDs to new entity IDs
        let mut entity_map = std::collections::HashMap::new();

        // First pass: spawn all entities and build the mapping
        // Collect all entity IDs first (query with no filter gets all entities)
        let entities_to_copy: Vec<_> = other
            .world
            .iter()
            .map(|entity_ref| entity_ref.entity())
            .collect();

        for old_entity in entities_to_copy {
            // Build a new entity with the same components (except Parent/Children for now)
            let mut builder = hecs::EntityBuilder::new();

            // Copy all components except Parent and Children
            // Clone components to avoid holding borrows
            if let Ok(name) = other.world.get::<&Name>(old_entity) {
                builder.add(Name(name.0.clone()));
            }
            if let Ok(transform) = other.world.get::<&TransformComponent>(old_entity) {
                builder.add(*transform);
            }
            if let Ok(mesh) = other.world.get::<&MeshComponent>(old_entity) {
                builder.add(*mesh);
            }
            if let Ok(material) = other.world.get::<&MaterialComponent>(old_entity) {
                builder.add(*material);
            }
            if let Ok(visible) = other.world.get::<&Visible>(old_entity) {
                builder.add(*visible);
            }
            if let Ok(rotate) = other.world.get::<&RotateAnimation>(old_entity) {
                builder.add(*rotate);
            }
            if let Ok(orbit) = other.world.get::<&OrbitAnimation>(old_entity) {
                builder.add(*orbit);
            }
            if let Ok(world_trans) = other.world.get::<&WorldTransform>(old_entity) {
                builder.add(*world_trans);
            }

            // Spawn the new entity
            let new_entity = self.world.spawn(builder.build());
            entity_map.insert(old_entity, new_entity);
        }

        // Second pass: fix up Parent and Children references
        let parent_children_to_fix: Vec<_> = entity_map
            .iter()
            .map(|(old, new)| {
                let parent = other.world.get::<&Parent>(*old).ok().map(|p| p.0);
                let children = other.world.get::<&Children>(*old).ok().map(|c| c.0.clone());
                (*old, *new, parent, children)
            })
            .collect();

        // Find root entities (those without Parent in the original scene)
        let mut root_entities = Vec::new();

        for (old_entity, new_entity, parent, children) in parent_children_to_fix {
            // Update Parent component
            if let Some(old_parent) = parent {
                // This entity had a parent in the original scene
                if let Some(&new_parent) = entity_map.get(&old_parent) {
                    self.world.insert_one(new_entity, Parent(new_parent)).ok();
                } else {
                    // Parent wasn't in the scene (shouldn't happen), make it a root
                    root_entities.push(new_entity);
                }
            } else {
                // This was a root entity in the original scene
                root_entities.push(new_entity);
            }

            // Update Children component
            if let Some(old_children) = children {
                let new_children: Vec<_> = old_children
                    .iter()
                    .filter_map(|old_child| entity_map.get(old_child).copied())
                    .collect();

                if !new_children.is_empty() {
                    self.world
                        .insert_one(new_entity, Children(new_children))
                        .ok();
                }
            }
        }

        // Make all root entities children of the specified parent
        if !root_entities.is_empty() {
            log::info!(
                "Setting {} root entities as children of parent",
                root_entities.len()
            );

            // Set parent reference on all roots
            for &root in &root_entities {
                self.world.insert_one(root, Parent(parent_entity)).ok();
            }

            // Add roots to parent's children list
            // Check if parent already has children
            let has_children = self.world.get::<&Children>(parent_entity).is_ok();

            if has_children {
                // Parent has children, extend the list
                if let Ok(mut parent_children) = self.world.get::<&mut Children>(parent_entity) {
                    parent_children.0.extend(&root_entities);
                }
            } else {
                // Parent didn't have children, create new list
                self.world
                    .insert_one(parent_entity, Children(root_entities))
                    .ok();
            }
        }

        // Merge assets
        // Note: This just adds new assets, doesn't remap handles
        // If you need handle remapping, you'd need to track asset handles during copy
        log::info!(
            "Merged {} meshes, {} textures",
            other.assets.meshes.len(),
            other.assets.textures.len()
        );
    }

    // ========================================================================
    // Debug Utilities
    // ========================================================================

    pub fn debug_print_transforms(&self) {
        log::info!("=== Transform Debug ===");
        for (_entity, (name, local, world)) in self
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
    use std::f32::consts::FRAC_PI_2;

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

        {
            let child_world = scene.world.get::<&WorldTransform>(child).unwrap();
            assert_eq!(child_world.0.translation, Vec3::new(2.0, 0.0, 0.0));
            assert_eq!(child_world.0.scale, Vec3::splat(1.0));
        }
    }

    #[test]
    fn test_transform_propagation_rotation() {
        let mut scene = Scene::new();

        let parent = scene.world.spawn((
            Name::new("Parent"),
            TransformComponent(Transform::from_trs(
                Vec3::ZERO,
                Quat::from_rotation_y(FRAC_PI_2),
                Vec3::ONE,
            )),
        ));

        let child = scene.world.spawn((
            Name::new("Child"),
            TransformComponent(Transform::from_trs(
                Vec3::new(1.0, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::ONE,
            )),
            Parent(parent),
        ));

        scene.world.insert_one(parent, Children(vec![child])).ok();

        scene.system_propagate_transforms();

        let parent_world = scene.world.get::<&WorldTransform>(parent).unwrap();
        assert!(parent_world.0.translation.abs_diff_eq(Vec3::ZERO, 1e-5));

        let child_world = scene.world.get::<&WorldTransform>(child).unwrap();
        assert!(child_world
            .0
            .translation
            .abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), 1e-5));
    }

    #[test]
    fn test_transform_propagation_updates_existing_world_transform() {
        let mut scene = Scene::new();

        let parent = scene.world.spawn((
            Name::new("Parent"),
            TransformComponent(Transform::from_trs(Vec3::ZERO, Quat::IDENTITY, Vec3::ONE)),
        ));

        let child = scene.world.spawn((
            Name::new("Child"),
            TransformComponent(Transform::from_trs(
                Vec3::new(2.0, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::ONE,
            )),
            Parent(parent),
        ));

        scene.world.insert_one(parent, Children(vec![child])).ok();

        scene.system_propagate_transforms();

        {
            let child_world = scene.world.get::<&WorldTransform>(child).unwrap();
            assert_eq!(child_world.0.translation, Vec3::new(2.0, 0.0, 0.0));
        }

        {
            let mut parent_transform = scene.world.get::<&mut TransformComponent>(parent).unwrap();
            parent_transform.0.translation = Vec3::new(1.0, 0.0, 0.0);
        }

        scene.system_propagate_transforms();

        {
            let child_world = scene.world.get::<&WorldTransform>(child).unwrap();
            assert_eq!(child_world.0.translation, Vec3::new(3.0, 0.0, 0.0));
        }
    }
}
