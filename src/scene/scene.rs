// scene/scene.rs - Optimized version with Rayon parallelization
use super::animation::{
    AnimationClip, AnimationState, AnimationTarget, MaterialUpdate, TransformUpdate,
};
use super::components::*;
use crate::asset::Assets;
use crate::renderer::{
    DirectionalShadowData, LightsData, PointShadowData, RenderBatcher, RenderObject, Renderer,
    SpotShadowData,
};
use crate::scene::{Camera, Transform};
use crate::time::Instant;
use glam::{Mat3, Mat4, Quat, Vec3};
use hecs::World;
use rayon::prelude::*;
use std::collections::HashMap;

pub struct Scene {
    pub world: World,
    pub assets: Assets,
    time: f64,
    last_frame: Option<Instant>,
    animations: Vec<AnimationClip>,
    animation_states: Vec<AnimationState>,
    camera: Camera,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            world: World::new(),
            assets: Assets::default(),
            time: 0.0,
            last_frame: None,
            animations: Vec::new(),
            animation_states: Vec::new(),
            camera: Camera::default(),
        }
    }

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

    pub fn animations(&self) -> &[AnimationClip] {
        &self.animations
    }

    pub fn animation_states(&self) -> &[AnimationState] {
        &self.animation_states
    }

    pub fn animation_states_mut(&mut self) -> &mut Vec<AnimationState> {
        &mut self.animation_states
    }

    pub fn camera(&self) -> &Camera {
        &self.camera
    }

    pub fn camera_mut(&mut self) -> &mut Camera {
        &mut self.camera
    }

    pub fn set_camera(&mut self, camera: Camera) {
        self.camera = camera;
    }

    pub fn add_animation_clip(&mut self, clip: AnimationClip) -> usize {
        let index = self.animations.len();
        self.animations.push(clip);
        index
    }

    pub fn play_animation(&mut self, clip_index: usize, looping: bool) -> Option<usize> {
        if clip_index >= self.animations.len() {
            return None;
        }

        let mut state = AnimationState::new(clip_index);
        state.looping = looping;
        let index = self.animation_states.len();
        self.animation_states.push(state);
        Some(index)
    }

    pub fn update(&mut self, dt: f64) {
        self.time += dt;

        // Run animation systems BEFORE propagating transforms
        self.system_animations(dt);
        self.system_rotate_animation(dt);
        self.system_orbit_animation(dt);

        // CRITICAL: Always propagate transforms after animations
        // This remains sequential due to parent-child dependencies
        self.system_propagate_transforms();
    }

    pub fn render(&mut self, renderer: &mut Renderer, batcher: &mut RenderBatcher) {
        batcher.clear();
        let camera_pos = renderer.camera_position();
        let camera_target = renderer.camera_target();
        let camera_up = renderer.camera_up();

        // OPTIMIZED: Parallel render object collection
        // Collect all data first to avoid borrow checker issues
        use crate::asset::Handle;
        use crate::asset::Mesh;

        let render_data: Vec<(
            Handle<Mesh>,              // mesh handle - FIXED: was u32, should be Handle<Mesh>
            crate::renderer::Material, // material
            bool,                      // visible
            Option<Transform>,         // world transform
            Option<Transform>,         // local transform
            Option<String>,            // name (for debug)
            Option<Billboard>,
            Option<DepthState>,
        )> = self
            .world
            .query::<(
                &MeshComponent,
                &MaterialComponent,
                &Visible,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
                Option<&Name>,
                Option<&Billboard>,
                Option<&DepthState>,
            )>()
            .iter()
            .map(
                |(
                    _entity,
                    (
                        mesh,
                        material,
                        visible,
                        world_transform,
                        local_transform,
                        name,
                        billboard,
                        depth_state,
                    ),
                )| {
                    (
                        mesh.0, // This is Handle<Mesh>, not u32
                        material.0,
                        visible.0,
                        world_transform.map(|wt| wt.0),
                        local_transform.map(|lt| lt.0),
                        name.map(|n| n.0.clone()),
                        billboard.copied(),
                        depth_state.copied(),
                    )
                },
            )
            .collect();

        let render_objects: Vec<RenderObject> = render_data
            .par_iter()
            .filter_map(
                |(
                    mesh,
                    material,
                    visible,
                    world_transform,
                    local_transform,
                    name,
                    billboard,
                    depth_state,
                )| {
                    if !*visible {
                        return None;
                    }

                    // Prefer WorldTransform if it exists
                    let mut transform = if let Some(world_trans) = world_transform {
                        *world_trans
                    } else if let Some(local_trans) = local_transform {
                        if cfg!(debug_assertions) {
                            if let Some(name_str) = name {
                                log::warn!(
                                    "Entity '{}' using LOCAL transform (no WorldTransform)",
                                    name_str
                                );
                            }
                        }
                        *local_trans
                    } else {
                        log::warn!("Entity without transform");
                        Transform::IDENTITY
                    };

                    let mut material_value = *material;

                    if let Some(billboard_val) = billboard {
                        transform = Self::apply_billboard_transform(
                            transform,
                            *billboard_val,
                            camera_pos,
                            camera_target,
                            camera_up,
                        );

                        if billboard_val.lit {
                            material_value = material_value.with_lit();
                        } else {
                            material_value = material_value.with_unlit();
                        }
                    }

                    let depth_state_val = depth_state.unwrap_or_default();
                    let force_overlay = billboard.is_some()
                        && !depth_state_val.depth_test
                        && !depth_state_val.depth_write;

                    Some(RenderObject {
                        mesh: *mesh, // FIXED: just dereference, it's already Handle<Mesh>
                        material: material_value,
                        transform,
                        depth_state: depth_state_val,
                        force_overlay,
                    })
                },
            )
            .collect();

        // Add all collected objects to the batcher
        for obj in render_objects {
            batcher.add(obj);
        }

        // Light collection - remains sequential as it's typically small
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
                .map(|_| Self::build_directional_shadow(camera_pos, camera_target, transform));

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
                .map(|_| Self::build_spot_shadow(transform, light));

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

    fn apply_billboard_transform(
        transform: Transform,
        billboard: Billboard,
        camera_position: Vec3,
        camera_target: Vec3,
        camera_up: Vec3,
    ) -> Transform {
        let mut result = transform;

        let view_forward = Self::safe_normalize(camera_target - camera_position, Vec3::NEG_Z);
        let mut view_up = Self::safe_normalize(camera_up, Vec3::Y);
        let mut view_right = view_forward.cross(view_up);
        if view_right.length_squared() < 1e-6 {
            view_right = Vec3::X;
        } else {
            view_right = view_right.normalize();
        }
        view_up = view_right.cross(view_forward);
        if view_up.length_squared() < 1e-6 {
            view_up = Vec3::Y;
        } else {
            view_up = view_up.normalize();
        }

        let translation = match billboard.space {
            BillboardSpace::World => transform.translation,
            BillboardSpace::View { offset } => {
                camera_position
                    + view_right * offset.x
                    + view_up * offset.y
                    + view_forward * offset.z
            }
        };

        let rotation_matrix = if matches!(billboard.space, BillboardSpace::View { .. }) {
            Mat3::from_cols(view_right, view_up, -view_forward)
        } else {
            match billboard.orientation {
                BillboardOrientation::FaceCamera => {
                    let forward = Self::safe_normalize(camera_position - translation, Vec3::Z);
                    let mut up_dir = Self::safe_normalize(camera_up, Vec3::Y);
                    let mut right = up_dir.cross(forward);
                    if right.length_squared() < 1e-6 {
                        right = Vec3::X;
                    } else {
                        right = right.normalize();
                    }
                    up_dir = forward.cross(right);
                    if up_dir.length_squared() < 1e-6 {
                        up_dir = Vec3::Y;
                    } else {
                        up_dir = up_dir.normalize();
                    }
                    Mat3::from_cols(right, up_dir, forward)
                }
                BillboardOrientation::FaceCameraYAxis => {
                    let mut forward = Vec3::new(
                        camera_position.x - translation.x,
                        0.0,
                        camera_position.z - translation.z,
                    );
                    if forward.length_squared() < 1e-6 {
                        forward = Vec3::Z;
                    } else {
                        forward = forward.normalize();
                    }
                    let mut right = Vec3::Y.cross(forward);
                    if right.length_squared() < 1e-6 {
                        right = Vec3::X;
                    } else {
                        right = right.normalize();
                    }
                    let mut up_dir = forward.cross(right);
                    if up_dir.length_squared() < 1e-6 {
                        up_dir = Vec3::Y;
                    } else {
                        up_dir = up_dir.normalize();
                    }
                    Mat3::from_cols(right, up_dir, forward)
                }
            }
        };

        let billboard_rotation = Quat::from_mat3(&rotation_matrix);
        result.translation = translation;
        result.rotation = billboard_rotation;
        result
    }

    fn safe_normalize(vec: Vec3, fallback: Vec3) -> Vec3 {
        if vec.length_squared() > 1e-6 {
            vec.normalize()
        } else {
            fallback
        }
    }

    fn build_directional_shadow(
        camera_pos: Vec3,
        camera_target: Vec3,
        light_transform: Transform,
    ) -> DirectionalShadowData {
        const SHADOW_SIZE: f32 = 5.0;
        const SHADOW_DISTANCE: f32 = 30.0;

        let raw_dir = light_transform.rotation * Vec3::NEG_Z;
        let direction = if raw_dir.length_squared() > 0.0 {
            raw_dir.normalize()
        } else {
            Vec3::new(0.0, -1.0, 0.0)
        };

        let focus = if (camera_target - camera_pos).length_squared() > 1e-4 {
            camera_target
        } else {
            camera_pos
        };
        let light_pos = focus - direction * SHADOW_DISTANCE;

        let mut up = light_transform.rotation * Vec3::Y;
        if up.length_squared() > 0.0 {
            up = up.normalize();
        }
        if up.length_squared() <= 0.0 || up.abs().dot(direction).abs() > 0.999 {
            up = Self::shadow_up(direction);
        }

        let view = Mat4::look_at_rh(light_pos, focus, up);

        let left = -SHADOW_SIZE;
        let right = SHADOW_SIZE;
        let bottom = -SHADOW_SIZE;
        let top = SHADOW_SIZE;
        let near = 0.1;
        let far = SHADOW_DISTANCE * 2.0;

        let projection = Mat4::from_cols(
            glam::Vec4::new(2.0 / (right - left), 0.0, 0.0, 0.0),
            glam::Vec4::new(0.0, 2.0 / (top - bottom), 0.0, 0.0),
            glam::Vec4::new(0.0, 0.0, -1.0 / (far - near), 0.0),
            glam::Vec4::new(
                -(right + left) / (right - left),
                -(top + bottom) / (top - bottom),
                -near / (far - near),
                1.0,
            ),
        );

        DirectionalShadowData {
            view_proj: projection * view,
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
            near,
            far,
        }
    }

    fn build_spot_shadow(transform: Transform, light: &SpotLight) -> SpotShadowData {
        let near = 0.1f32;
        let far = light.range.max(near + 0.1);
        let fov = (light.outer_angle * 2.0).clamp(0.1, std::f32::consts::PI - 0.1);

        let position = transform.translation;
        let mut forward = transform.rotation * Vec3::NEG_Z;
        if forward.length_squared() < 1e-8 {
            forward = Vec3::NEG_Z;
        }
        forward = forward.normalize();

        let mut up = transform.rotation * Vec3::Y;
        if up.length_squared() < 1e-8 {
            up = Vec3::Y;
        }

        let mut right = forward.cross(up);
        if right.length_squared() < 1e-8 {
            let fallback = if forward.dot(Vec3::X).abs() < 0.9 {
                Vec3::X
            } else {
                Vec3::Y
            };
            right = forward.cross(fallback);
        }
        right = right.normalize();
        let up = right.cross(forward).normalize();

        let view = Mat4::look_at_rh(position, position + forward, up);
        let projection = Mat4::perspective_rh(fov, 1.0, near, far);

        SpotShadowData {
            view_proj: projection * view,
            far,
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

    pub fn add_default_lighting(&mut self) -> usize {
        if self.has_any_lights() {
            return 0;
        }

        log::info!("No lights found in scene - adding default lighting setup");

        let mut created = 0usize;

        let sun1_direction = Vec3::new(0.3, -1.0, -1.1).normalize();
        let sun1_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, sun1_direction);

        self.world.spawn((
            Name::new("Default Sky Light"),
            TransformComponent(Transform::from_trs(Vec3::ZERO, sun1_rotation, Vec3::ONE)),
            DirectionalLight {
                color: Vec3::new(0.49, 0.95, 0.85),
                intensity: 2.5,
            },
            CanCastShadow(true),
        ));
        created += 1;

        let sun2_direction = Vec3::new(-1.4, -1.0, 1.25).normalize();
        let sun2_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, sun2_direction);

        self.world.spawn((
            Name::new("Default Sky Light"),
            TransformComponent(Transform::from_trs(Vec3::ZERO, sun2_rotation, Vec3::ONE)),
            DirectionalLight {
                color: Vec3::new(0.9, 0.95, 0.5),
                intensity: 2.5,
            },
            CanCastShadow(true),
        ));
        created += 1;

        // self.world.spawn((
        //     Name::new("Default Fill Light"),
        //     TransformComponent(Transform::from_trs(
        //         Vec3::new(3.0, 4.0, 2.0),
        //         Quat::IDENTITY,
        //         Vec3::ONE,
        //     )),
        //     PointLight {
        //         color: Vec3::new(1.0, 0.47, 0.22),
        //         intensity: 200.0,
        //         range: 20.0,
        //     },
        //     CanCastShadow(true),
        // ));
        // created += 1;

        // let rim_position = Vec3::new(-2.0, 6.0, -5.0);
        // let rim_direction = (Vec3::ZERO - rim_position).normalize();
        // let rim_rotation = Quat::from_rotation_arc(Vec3::NEG_Z, rim_direction);

        // self.world.spawn((
        //     Name::new("Default Rim Spot Light"),
        //     TransformComponent(Transform::from_trs(rim_position, rim_rotation, Vec3::ONE)),
        //     SpotLight {
        //         color: Vec3::new(0.3, 0.55, 0.9),
        //         intensity: 200.0,
        //         range: 20.0,
        //         inner_angle: 20f32.to_radians(),
        //         outer_angle: 30f32.to_radians(),
        //     },
        //     CanCastShadow(true),
        // ));
        // created += 1;

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

    // ========================================================================
    // Animation Systems (OPTIMIZED with Rayon)
    // ========================================================================

    fn system_animations(&mut self, dt: f64) {
        if self.animation_states.is_empty() || self.animations.is_empty() {
            return;
        }

        let dt = dt as f32;

        let mut transform_updates: HashMap<hecs::Entity, TransformUpdate> = HashMap::new();
        let mut material_updates: HashMap<usize, MaterialUpdate> = HashMap::new();

        for state in &mut self.animation_states {
            if state.clip_index >= self.animations.len() {
                continue;
            }

            let clip = &self.animations[state.clip_index];
            let sample_time = state.advance(dt, clip.duration);
            clip.sample(sample_time, &mut transform_updates, &mut material_updates);
        }

        for (entity, update) in transform_updates {
            if let Ok(mut transform) = self.world.get::<&mut TransformComponent>(entity) {
                if let Some(translation) = update.translation {
                    transform.0.translation = translation;
                }

                if let Some(rotation) = update.rotation {
                    transform.0.rotation = rotation;
                }

                if let Some(scale) = update.scale {
                    transform.0.scale = scale;
                }
            }
        }

        if material_updates.is_empty() {
            return;
        }

        let mut material_entities: Vec<hecs::Entity> = Vec::new();

        for (material_index, update) in material_updates {
            let Some(color) = update.base_color else {
                continue;
            };

            material_entities.clear();
            {
                let mut query = self.world.query::<&GltfMaterial>();
                for (entity, gltf_material) in query.iter() {
                    if gltf_material.0 == material_index {
                        material_entities.push(entity);
                    }
                }
            }

            if material_entities.is_empty() {
                continue;
            }

            let to_u8 = |value: f32| -> u8 { (value.clamp(0.0, 1.0) * 255.0).round() as u8 };

            for entity in &material_entities {
                if let Ok(mut material) = self.world.get::<&mut MaterialComponent>(*entity) {
                    material.0.base_color = [
                        to_u8(color.x),
                        to_u8(color.y),
                        to_u8(color.z),
                        to_u8(color.w),
                    ];
                }
            }
        }
    }

    fn system_rotate_animation(&mut self, dt: f64) {
        // OPTIMIZED: Collect entities first, then process in parallel
        let entities: Vec<_> = self
            .world
            .query::<(&TransformComponent, &RotateAnimation)>()
            .iter()
            .map(|(entity, (transform, anim))| (entity, transform.0, *anim))
            .collect();

        // Compute rotations in parallel
        let updates: Vec<_> = entities
            .par_iter()
            .map(|(entity, transform, anim)| {
                let rotation = Quat::from_axis_angle(anim.axis, anim.speed * dt as f32);
                let new_rotation = rotation * transform.rotation;
                (*entity, new_rotation)
            })
            .collect();

        // Apply updates sequentially (ECS requirement)
        for (entity, new_rotation) in updates {
            if let Ok(mut transform) = self.world.get::<&mut TransformComponent>(entity) {
                transform.0.rotation = new_rotation;
            }
        }
    }

    fn system_orbit_animation(&mut self, _dt: f64) {
        let time = self.time as f32;

        // OPTIMIZED: Collect entities first, then process in parallel
        let entities: Vec<_> = self
            .world
            .query::<(&TransformComponent, &OrbitAnimation)>()
            .iter()
            .map(|(entity, (_, orbit))| (entity, *orbit))
            .collect();

        // Compute positions in parallel
        let updates: Vec<_> = entities
            .par_iter()
            .map(|(entity, orbit)| {
                let angle = time * orbit.speed + orbit.offset;
                let new_translation = orbit.center
                    + Vec3::new(
                        angle.cos() * orbit.radius,
                        (time + orbit.offset).sin() * 0.5,
                        angle.sin() * orbit.radius,
                    );
                (*entity, new_translation)
            })
            .collect();

        // Apply updates sequentially (ECS requirement)
        for (entity, new_translation) in updates {
            if let Ok(mut transform) = self.world.get::<&mut TransformComponent>(entity) {
                transform.0.translation = new_translation;
            }
        }
    }

    // ========================================================================
    // Transform Propagation System (CRITICAL)
    // Remains SEQUENTIAL due to parent-child dependencies
    // ========================================================================

    fn system_propagate_transforms(&mut self) {
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
                let local = match self.world.get::<&TransformComponent>(entity) {
                    Ok(t) => t.0,
                    Err(_) => {
                        log::trace!("Entity {:?} has no TransformComponent, skipping", entity);
                        continue;
                    }
                };

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

    pub fn merge_as_child(&mut self, parent_entity: hecs::Entity, other: Scene) {
        log::info!("Merging scene with {} entities as child", other.world.len());

        let mut entity_map = std::collections::HashMap::new();

        let entities_to_copy: Vec<_> = other
            .world
            .iter()
            .map(|entity_ref| entity_ref.entity())
            .collect();

        for old_entity in entities_to_copy {
            let mut builder = hecs::EntityBuilder::new();

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
            if let Ok(gltf_node) = other.world.get::<&GltfNode>(old_entity) {
                builder.add(*gltf_node);
            }
            if let Ok(gltf_material) = other.world.get::<&GltfMaterial>(old_entity) {
                builder.add(*gltf_material);
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

            let new_entity = self.world.spawn(builder.build());
            entity_map.insert(old_entity, new_entity);
        }

        let parent_children_to_fix: Vec<_> = entity_map
            .iter()
            .map(|(old, &new)| {
                let parent = other.world.get::<&Parent>(*old).ok().map(|p| p.0);
                let children = other.world.get::<&Children>(*old).ok().map(|c| c.0.clone());
                (new, parent, children)
            })
            .collect();

        let mut root_entities = Vec::new();

        for (new_entity, parent, children) in parent_children_to_fix {
            if let Some(old_parent) = parent {
                if let Some(&new_parent) = entity_map.get(&old_parent) {
                    self.world.insert_one(new_entity, Parent(new_parent)).ok();
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
                    self.world
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
                self.world.insert_one(root, Parent(parent_entity)).ok();
            }

            let has_children = self.world.get::<&Children>(parent_entity).is_ok();

            if has_children {
                if let Ok(mut parent_children) = self.world.get::<&mut Children>(parent_entity) {
                    parent_children.0.extend(&root_entities);
                }
            } else {
                self.world
                    .insert_one(parent_entity, Children(root_entities))
                    .ok();
            }
        }

        let animation_offset = self.animations.len();
        for mut clip in other.animations {
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
            self.animations.push(clip);
        }

        for mut state in other.animation_states {
            state.clip_index += animation_offset;
            self.animation_states.push(state);
        }

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
    use crate::scene::components::SpotLight;
    use crate::scene::Transform;
    use glam::{EulerRot, Vec2, Vec4};
    use std::f32::consts::FRAC_PI_2;

    #[test]
    fn view_space_billboard_aligns_with_camera_basis() {
        let transform = Transform::IDENTITY;
        let offset = Vec3::new(2.0, -1.0, 6.0);
        let billboard = Billboard::new(BillboardOrientation::FaceCamera)
            .with_space(BillboardSpace::View { offset });
        let camera_pos = Vec3::new(1.5, -3.2, 4.0);
        let camera_target = Vec3::new(1.5, -2.2, -1.0);
        let camera_up = Vec3::new(0.0, 1.0, 0.1);

        let result = Scene::apply_billboard_transform(
            transform,
            billboard,
            camera_pos,
            camera_target,
            camera_up,
        );

        let view_forward = Scene::safe_normalize(camera_target - camera_pos, Vec3::NEG_Z);
        let mut view_up = Scene::safe_normalize(camera_up, Vec3::Y);
        let mut view_right = view_forward.cross(view_up);
        if view_right.length_squared() < 1e-6 {
            view_right = Vec3::X;
        } else {
            view_right = view_right.normalize();
        }
        view_up = view_right.cross(view_forward);
        if view_up.length_squared() < 1e-6 {
            view_up = Vec3::Y;
        } else {
            view_up = view_up.normalize();
        }

        let expected_translation =
            camera_pos + view_right * offset.x + view_up * offset.y + view_forward * offset.z;

        assert!(result.translation.abs_diff_eq(expected_translation, 1e-5));
        assert!((result.rotation * Vec3::X).abs_diff_eq(view_right, 1e-5));
        assert!((result.rotation * Vec3::Y).abs_diff_eq(view_up, 1e-5));
        assert!((result.rotation * Vec3::Z).abs_diff_eq(-view_forward, 1e-5));
    }

    #[test]
    fn world_space_billboard_faces_camera_position() {
        let transform =
            Transform::from_trs(Vec3::new(-2.0, 1.0, -5.0), Quat::IDENTITY, Vec3::splat(1.5));
        let billboard = Billboard::new(BillboardOrientation::FaceCamera);
        let camera_pos = Vec3::new(4.0, 3.0, 2.0);
        let camera_target = Vec3::new(0.0, 0.0, 0.0);
        let camera_up = Vec3::Y;

        let result = Scene::apply_billboard_transform(
            transform,
            billboard,
            camera_pos,
            camera_target,
            camera_up,
        );

        let expected_forward = Scene::safe_normalize(camera_pos - transform.translation, Vec3::Z);

        assert!(result.translation.abs_diff_eq(transform.translation, 1e-5));
        assert!((result.rotation * Vec3::Z).abs_diff_eq(expected_forward, 1e-5));
    }

    #[test]
    fn test_transform_propagation_simple() {
        let mut scene = Scene::new();

        let parent = scene.world.spawn((
            Name::new("Parent"),
            TransformComponent(Transform::from_trs(
                Vec3::new(5.0, 0.0, 0.0),
                Quat::IDENTITY,
                Vec3::ONE,
            )),
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

        let parent_world = scene.world.get::<&WorldTransform>(parent).unwrap();
        assert_eq!(parent_world.0.translation, Vec3::new(5.0, 0.0, 0.0));

        let child_world = scene.world.get::<&WorldTransform>(child).unwrap();
        assert_eq!(child_world.0.translation, Vec3::new(7.0, 0.0, 0.0));
    }

    #[test]
    fn test_transform_propagation_scale() {
        let mut scene = Scene::new();

        let parent = scene.world.spawn((
            Name::new("Parent"),
            TransformComponent(Transform::from_trs(
                Vec3::ZERO,
                Quat::IDENTITY,
                Vec3::splat(2.0),
            )),
        ));

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

    const EPS: f32 = 1e-5;

    fn build_directional_projection() -> Mat4 {
        let left = -15.0;
        let right = 15.0;
        let bottom = -15.0;
        let top = 15.0;
        let near = 0.1;
        let far = 60.0;

        Mat4::from_cols(
            Vec4::new(2.0 / (right - left), 0.0, 0.0, 0.0),
            Vec4::new(0.0, 2.0 / (top - bottom), 0.0, 0.0),
            Vec4::new(0.0, 0.0, -1.0 / (far - near), 0.0),
            Vec4::new(
                -(right + left) / (right - left),
                -(top + bottom) / (top - bottom),
                -near / (far - near),
                1.0,
            ),
        )
    }

    #[test]
    fn directional_shadow_view_matrix_matches_expected_orientation() {
        let camera_pos = Vec3::new(8.0, 6.0, -4.0);
        let camera_target = Vec3::new(2.5, 1.0, -3.0);
        let rotation = Quat::from_euler(EulerRot::YXZ, 0.35, -0.6, 0.5)
            * Quat::from_euler(EulerRot::ZXY, 0.2, 0.0, 0.1);
        let transform = Transform::from_trs(Vec3::new(1.5, 3.0, -2.0), rotation, Vec3::ONE);

        let shadow = Scene::build_directional_shadow(camera_pos, camera_target, transform);

        let direction = (rotation * Vec3::NEG_Z).normalize();
        let up = (rotation * Vec3::Y).normalize();
        let focus = camera_target;
        let light_pos = focus - direction * 30.0;
        let expected_view = Mat4::look_at_rh(light_pos, focus, up);
        let projection = build_directional_projection();
        let expected_view_proj = projection * expected_view;

        assert!(
            shadow.view_proj.abs_diff_eq(expected_view_proj, EPS),
            "view projection mismatch: {:?} vs {:?}",
            shadow.view_proj,
            expected_view_proj
        );

        let actual_view = projection.inverse() * shadow.view_proj;
        assert!(actual_view.abs_diff_eq(expected_view, EPS));

        let dir_in_view = (actual_view * direction.extend(0.0)).truncate().normalize();
        assert!(dir_in_view.abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), EPS));
    }

    #[test]
    fn directional_shadow_centers_camera_target() {
        let camera_pos = Vec3::new(4.0, 6.0, 12.0);
        let camera_target = Vec3::new(1.0, 0.5, -2.0);
        let rotation = Quat::from_euler(EulerRot::YXZ, -0.2, -0.9, 0.3);
        let transform = Transform::from_trs(Vec3::ZERO, rotation, Vec3::ONE);

        let shadow = Scene::build_directional_shadow(camera_pos, camera_target, transform);

        let clip = shadow.view_proj * camera_target.extend(1.0);
        assert!(clip.w > 0.0);
        let ndc = clip.truncate() / clip.w;
        let uv = Vec2::new(ndc.x * 0.5 + 0.5, -ndc.y * 0.5 + 0.5);

        assert!(
            uv.abs_diff_eq(Vec2::splat(0.5), 1e-4),
            "camera target projected to {:?}",
            uv
        );
    }

    #[test]
    fn directional_shadow_respects_light_roll() {
        let camera_pos = Vec3::new(-6.0, 5.0, 2.0);
        let camera_target = Vec3::new(0.5, 1.5, -3.0);
        let rotation = Quat::from_euler(EulerRot::ZXY, 0.3, -0.5, 0.9);
        let transform = Transform::from_trs(Vec3::new(-1.0, 2.0, 0.5), rotation, Vec3::splat(1.0));

        let shadow = Scene::build_directional_shadow(camera_pos, camera_target, transform);

        let projection = build_directional_projection();
        let actual_view = projection.inverse() * shadow.view_proj;

        let forward = (rotation * Vec3::NEG_Z).normalize();
        let up = (rotation * Vec3::Y).normalize();
        let right = (rotation * Vec3::X).normalize();

        let forward_in_view = (actual_view * forward.extend(0.0)).truncate().normalize();
        let up_in_view = (actual_view * up.extend(0.0)).truncate().normalize();
        let right_in_view = (actual_view * right.extend(0.0)).truncate().normalize();

        assert!(forward_in_view.abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), EPS));
        assert!(up_in_view.abs_diff_eq(Vec3::Y, EPS));
        assert!(right_in_view.abs_diff_eq(Vec3::X, EPS));
    }

    #[test]
    fn spot_shadow_view_matrix_uses_transform_basis() {
        let rotation = Quat::from_euler(EulerRot::YXZ, 0.45, -0.35, 0.2);
        let transform = Transform::from_trs(Vec3::new(2.0, 5.0, -1.0), rotation, Vec3::ONE);
        let light = SpotLight {
            color: Vec3::splat(1.0),
            intensity: 10.0,
            inner_angle: 0.3,
            outer_angle: 0.6,
            range: 25.0,
        };

        let shadow = Scene::build_spot_shadow(transform, &light);

        let near = 0.1;
        let far = light.range.max(near + 0.1);
        let fov = (light.outer_angle * 2.0).clamp(0.1, std::f32::consts::PI - 0.1);
        let expected_view = Mat4::look_at_rh(
            transform.translation,
            transform.translation + transform.rotation * Vec3::NEG_Z,
            transform.rotation * Vec3::Y,
        );
        let projection = Mat4::perspective_rh(fov, 1.0, near, far);
        let expected_view_proj = projection * expected_view;

        assert!(shadow.view_proj.abs_diff_eq(expected_view_proj, EPS));

        let actual_view = projection.inverse() * shadow.view_proj;
        assert!(actual_view.abs_diff_eq(expected_view, EPS));

        let forward = transform.rotation * Vec3::NEG_Z;
        let up = transform.rotation * Vec3::Y;
        let forward_in_view = (actual_view * forward.extend(0.0)).truncate().normalize();
        let up_in_view = (actual_view * up.extend(0.0)).truncate().normalize();
        assert!(forward_in_view.abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), EPS));
        assert!(up_in_view.abs_diff_eq(Vec3::Y, EPS));
    }

    #[test]
    fn point_shadow_view_matrices_cover_all_cubemap_faces() {
        let position = Vec3::new(-3.0, 4.5, 1.0);
        let range = 12.0;
        let shadow = Scene::build_point_shadow(position, range);

        let near = 0.1;
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

        for (((matrix, dir), up), face) in shadow
            .view_proj
            .iter()
            .zip(dirs.iter())
            .zip(ups.iter())
            .zip(0usize..)
        {
            let expected_view = Mat4::look_at_rh(position, position + *dir, *up);
            let expected_view_proj = projection * expected_view;
            assert!(
                matrix.abs_diff_eq(expected_view_proj, EPS),
                "face {} mismatch",
                face
            );

            let actual_view = projection.inverse() * *matrix;
            assert!(actual_view.abs_diff_eq(expected_view, EPS));

            let dir_in_view = (actual_view * dir.extend(0.0)).truncate().normalize();
            assert!(dir_in_view.abs_diff_eq(Vec3::new(0.0, 0.0, -1.0), EPS));
        }
    }

    #[test]
    fn spot_shadow_depth_maps_into_wgpu_range() {
        let rotation = Quat::from_euler(EulerRot::YXZ, -0.35, 0.5, 0.1);
        let transform = Transform::from_trs(Vec3::new(-4.0, 3.0, 6.0), rotation, Vec3::ONE);
        let light = SpotLight {
            color: Vec3::splat(1.0),
            intensity: 5.0,
            inner_angle: 0.4,
            outer_angle: 0.7,
            range: 30.0,
        };

        let shadow = Scene::build_spot_shadow(transform, &light);

        let near = 0.1;
        let far = light.range.max(near + 0.1);
        let forward = (transform.rotation * Vec3::NEG_Z).normalize();
        let position = transform.translation;

        let near_world = position + forward * near;
        let far_world = position + forward * far;

        let clip_near = shadow.view_proj * near_world.extend(1.0);
        let clip_far = shadow.view_proj * far_world.extend(1.0);
        assert!(clip_near.w > 0.0 && clip_far.w > 0.0);

        let ndc_near = clip_near.truncate() / clip_near.w;
        let ndc_far = clip_far.truncate() / clip_far.w;

        assert!(ndc_near.z >= -EPS && ndc_near.z <= 1.0 + EPS);
        assert!(ndc_far.z >= -EPS && ndc_far.z <= 1.0 + EPS);

        assert!((ndc_near.z - 0.0).abs() < 1e-4, "near depth {}", ndc_near.z);
        assert!((ndc_far.z - 1.0).abs() < 1e-4, "far depth {}", ndc_far.z);
    }

    #[test]
    fn point_shadow_depth_maps_into_wgpu_range() {
        let position = Vec3::new(2.5, -1.5, 7.0);
        let range = 18.0;
        let shadow = Scene::build_point_shadow(position, range);

        for (matrix, dir) in shadow.view_proj.iter().zip([
            Vec3::X,
            Vec3::NEG_X,
            Vec3::Y,
            Vec3::NEG_Y,
            Vec3::Z,
            Vec3::NEG_Z,
        ]) {
            let forward = dir.normalize();
            let near_world = position + forward * shadow.near;
            let far_world = position + forward * shadow.far;

            let clip_near = *matrix * near_world.extend(1.0);
            let clip_far = *matrix * far_world.extend(1.0);
            assert!(clip_near.w > 0.0 && clip_far.w > 0.0);

            let ndc_near = clip_near.truncate() / clip_near.w;
            let ndc_far = clip_far.truncate() / clip_far.w;

            assert!(ndc_near.z >= -EPS && ndc_near.z <= 1.0 + EPS);
            assert!(ndc_far.z >= -EPS && ndc_far.z <= 1.0 + EPS);

            assert!(
                (ndc_near.z - 0.0).abs() < 1e-4,
                "face dir {:?} near {}",
                dir,
                ndc_near.z
            );
            assert!(
                (ndc_far.z - 1.0).abs() < 1e-4,
                "face dir {:?} far {}",
                dir,
                ndc_far.z
            );
        }
    }
}
