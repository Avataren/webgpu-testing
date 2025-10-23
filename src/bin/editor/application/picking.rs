use glam::{Vec2, Vec3, Vec4};
use hecs::Entity;
use wgpu_cube::app::UpdateContext;
use wgpu_cube::renderer::RenderRegion;
use wgpu_cube::scene::components::{DirectionalLight, PointLight, SpotLight};
use wgpu_cube::scene::{MeshBounds, Transform, TransformComponent, Visible, WorldTransform};

use super::core::{CameraView, EditorApplication, SceneRay};
use super::history_system::HistorySystem;

const LIGHT_ICON_SCREEN_FRACTION: f32 = 0.055;
const LIGHT_ICON_MIN_DISTANCE: f32 = 0.1;
const LIGHT_ICON_ORTHO_WORLD_SIZE: f32 = 0.65;
const LIGHT_ICON_PICK_PADDING: f32 = 0.15;

impl EditorApplication {
    pub(super) fn pick_entity(
        &self,
        ctx: &UpdateContext,
        uv: Vec2,
        region: RenderRegion,
    ) -> Option<Entity> {
        let width = region.width().max(1) as f32;
        let height = region.height().max(1) as f32;
        let aspect = width / height;
        let camera = ctx.scene.camera();
        let (origin, direction) = Self::ray_from_uv(camera, uv, aspect);

        let world = ctx.scene.main_world();
        let mut best: Option<(Entity, f32)> = None;

        for (entity, (bounds, world_transform, local_transform, visible)) in world
            .query::<(
                &MeshBounds,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
                Option<&Visible>,
            )>()
            .iter()
        {
            if visible.is_some_and(|v| !v.0) {
                continue;
            }

            let transform = world_transform
                .map(|wt| wt.0)
                .or_else(|| local_transform.map(|lt| lt.0))
                .unwrap_or(Transform::IDENTITY);

            let Some(distance) = Self::entity_hit_distance(transform, *bounds, origin, direction)
            else {
                continue;
            };

            match best {
                Some((_, best_distance)) if distance >= best_distance => {}
                _ => best = Some((entity, distance)),
            }
        }

        let camera_view = CameraView::new(camera.eye, camera.up, camera.fov_y_radians());
        let ray = SceneRay::new(origin, direction);
        self.consider_light_picks(world, camera_view, ray, &mut best);

        best.map(|(entity, _)| entity)
    }

    fn consider_light_picks(
        &self,
        world: &hecs::World,
        camera: CameraView,
        ray: SceneRay,
        best: &mut Option<(Entity, f32)>,
    ) {
        let mut consider = |entity: Entity, distance: f32| {
            if let Some((_, best_distance)) = best.as_ref() {
                if distance >= *best_distance {
                    return;
                }
            }
            *best = Some((entity, distance));
        };

        for (entity, (_light, world_transform, local_transform)) in world
            .query::<(
                &PointLight,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
            )>()
            .iter()
        {
            let transform = world_transform
                .map(|wt| wt.0)
                .or_else(|| local_transform.map(|lt| lt.0))
                .unwrap_or(Transform::IDENTITY);
            if let Some(distance) = Self::light_icon_hit_distance(
                camera.eye,
                camera.up,
                camera.fov_y,
                transform.translation,
                ray.origin,
                ray.direction,
            ) {
                consider(entity, distance);
            }
        }

        for (entity, (_light, world_transform, local_transform)) in world
            .query::<(
                &SpotLight,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
            )>()
            .iter()
        {
            let transform = world_transform
                .map(|wt| wt.0)
                .or_else(|| local_transform.map(|lt| lt.0))
                .unwrap_or(Transform::IDENTITY);
            if let Some(distance) = Self::light_icon_hit_distance(
                camera.eye,
                camera.up,
                camera.fov_y,
                transform.translation,
                ray.origin,
                ray.direction,
            ) {
                consider(entity, distance);
            }
        }

        for (entity, (_light, world_transform, local_transform)) in world
            .query::<(
                &DirectionalLight,
                Option<&WorldTransform>,
                Option<&TransformComponent>,
            )>()
            .iter()
        {
            let transform = world_transform
                .map(|wt| wt.0)
                .or_else(|| local_transform.map(|lt| lt.0))
                .unwrap_or(Transform::IDENTITY);
            if let Some(distance) = Self::light_icon_hit_distance(
                camera.eye,
                camera.up,
                camera.fov_y,
                transform.translation,
                ray.origin,
                ray.direction,
            ) {
                consider(entity, distance);
            }
        }
    }

    pub(super) fn viewport_uv(rect: egui::Rect, pos: egui::Pos2) -> Vec2 {
        let width = rect.width();
        let height = rect.height();
        if width <= 0.0 || height <= 0.0 {
            return Vec2::ZERO;
        }

        let local_x = (pos.x - rect.min.x) / width;
        let local_y = (pos.y - rect.min.y) / height;

        if !local_x.is_finite() || !local_y.is_finite() {
            Vec2::ZERO
        } else {
            Vec2::new(local_x.clamp(0.0, 1.0), local_y.clamp(0.0, 1.0))
        }
    }

    pub(super) fn ray_from_uv(
        camera: &wgpu_cube::scene::Camera,
        uv: Vec2,
        aspect: f32,
    ) -> (Vec3, Vec3) {
        let ndc_x = uv.x * 2.0 - 1.0;
        let ndc_y = 1.0 - uv.y * 2.0;

        let view = camera.view();
        let proj = camera.proj(aspect);
        let inv = (proj * view).inverse();

        let near = inv * Vec4::new(ndc_x, ndc_y, -1.0, 1.0);
        let far = inv * Vec4::new(ndc_x, ndc_y, 1.0, 1.0);

        let near_point = if near.w.abs() > f32::EPSILON {
            near.truncate() / near.w
        } else {
            near.truncate()
        };
        let far_point = if far.w.abs() > f32::EPSILON {
            far.truncate() / far.w
        } else {
            far.truncate()
        };

        let origin = camera.eye;
        let mut direction = far_point - origin;
        if direction.length_squared() < 1e-12 {
            direction = (far_point - near_point).normalize_or_zero();
        } else {
            direction = direction.normalize();
        }

        (origin, direction)
    }

    fn entity_hit_distance(
        transform: Transform,
        bounds: MeshBounds,
        origin: Vec3,
        direction: Vec3,
    ) -> Option<f32> {
        let world_matrix = transform.matrix();
        let inverse = world_matrix.inverse();

        let origin_local = (inverse * origin.extend(1.0)).truncate();
        let direction_local = (inverse * direction.extend(0.0)).truncate();

        if !direction_local.is_finite() || direction_local.length_squared() < 1e-12 {
            return None;
        }

        let local_t =
            Self::ray_aabb_intersection(origin_local, direction_local, bounds.min, bounds.max)?;

        let hit_local = origin_local + direction_local * local_t;
        let hit_world = (world_matrix * hit_local.extend(1.0)).truncate();
        let distance = (hit_world - origin).length();

        distance.is_finite().then_some(distance)
    }

    fn ray_aabb_intersection(origin: Vec3, direction: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
        let mut t_min = f32::NEG_INFINITY;
        let mut t_max = f32::INFINITY;

        for axis in 0..3 {
            let o = origin[axis];
            let d = direction[axis];
            let min_bound = min[axis];
            let max_bound = max[axis];

            if d.abs() < 1e-6 {
                if o < min_bound || o > max_bound {
                    return None;
                }
                continue;
            }

            let inv = 1.0 / d;
            let mut t1 = (min_bound - o) * inv;
            let mut t2 = (max_bound - o) * inv;

            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }

            t_min = t_min.max(t1);
            t_max = t_max.min(t2);

            if t_min > t_max {
                return None;
            }
        }

        if t_max < 0.0 {
            return None;
        }

        let hit = if t_min >= 0.0 { t_min } else { t_max };
        (hit.is_finite() && hit >= 0.0).then_some(hit)
    }

    fn light_icon_hit_distance(
        camera_eye: Vec3,
        camera_up: Vec3,
        camera_fov_y: f32,
        position: Vec3,
        ray_origin: Vec3,
        ray_dir: Vec3,
    ) -> Option<f32> {
        let icon_scale = Self::light_icon_world_scale(camera_eye, camera_fov_y, position);
        let forward = HistorySystem::safe_normalize(camera_eye - position, Vec3::Z);
        let up_hint = HistorySystem::safe_normalize(camera_up, Vec3::Y);
        let (right, up) = HistorySystem::basis_from_up_forward(up_hint, forward);
        let normal = forward;
        let denom = ray_dir.dot(normal);
        if denom.abs() < 1e-6 {
            return None;
        }

        let to_center = position - ray_origin;
        let t = to_center.dot(normal) / denom;
        if !t.is_finite() || t < 0.0 {
            return None;
        }

        let hit_point = ray_origin + ray_dir * t;
        let offset = hit_point - position;
        let half_extent = 0.5 * icon_scale;
        let padding = half_extent * LIGHT_ICON_PICK_PADDING;
        let limit = half_extent + padding;

        let proj_right = offset.dot(right);
        let proj_up = offset.dot(up);
        if proj_right.abs() <= limit && proj_up.abs() <= limit {
            Some(t)
        } else {
            None
        }
    }

    fn light_icon_world_scale(camera_eye: Vec3, camera_fov_y: f32, position: Vec3) -> f32 {
        let distance = (camera_eye - position)
            .length()
            .max(LIGHT_ICON_MIN_DISTANCE);
        let half_fov = (camera_fov_y * 0.5).max(1e-4);
        if half_fov <= 1e-3 {
            LIGHT_ICON_ORTHO_WORLD_SIZE
        } else {
            let vertical_extent = half_fov.tan();
            2.0 * distance * vertical_extent * LIGHT_ICON_SCREEN_FRACTION
        }
    }
}
