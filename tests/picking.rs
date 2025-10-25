use glam::{Vec2, Vec3};
use wgpu_cube::renderer::RenderRegion;
use wgpu_cube::scene::components::{EditorEntityId, MeshBounds, TransformComponent, Visible};
use wgpu_cube::scene::{encode_pick_value, entity_for_pick_value, Scene, Transform};

#[test]
fn pick_request_resolves_expected_entity() {
    let mut scene = Scene::new();
    let editor_id = EditorEntityId(0xABCD_EF01_2345_6789_ABCD_EF01_2345_6789);
    let pick_id = editor_id.pick_identifier();
    let pick_value = encode_pick_value(pick_id);

    let entity = scene.main_world_mut().spawn((
        editor_id,
        MeshBounds::new(Vec3::splat(-0.5), Vec3::splat(0.5)),
        TransformComponent(Transform::IDENTITY),
        Visible(true),
    ));

    assert_eq!(entity_for_pick_value(&scene, pick_value), Some(entity));

    let origin = Vec3::new(0.0, 0.0, 2.0);
    let direction = Vec3::new(0.0, 0.0, -1.0);
    let region = RenderRegion::new(0, 0, 1280, 720).expect("valid render region");
    let uv = Vec2::new(0.5, 0.5);

    let fallback = cpu_pick_entity(&scene, origin, direction, uv, region);
    assert_eq!(fallback, Some(entity));
}

fn cpu_pick_entity(
    scene: &Scene,
    ray_origin: Vec3,
    ray_dir: Vec3,
    _uv: Vec2,
    _region: RenderRegion,
) -> Option<hecs::Entity> {
    let world = scene.main_world();
    let mut best: Option<(hecs::Entity, f32)> = None;

    for (entity, (bounds, transform)) in world
        .query::<(&MeshBounds, Option<&TransformComponent>)>()
        .iter()
    {
        let transform = transform.map(|t| t.0).unwrap_or(Transform::IDENTITY);
        let Some(distance) = ray_aabb_intersection(transform, *bounds, ray_origin, ray_dir) else {
            continue;
        };

        match best {
            Some((_, best_distance)) if distance >= best_distance => {}
            _ => best = Some((entity, distance)),
        }
    }

    best.map(|(entity, _)| entity)
}

fn ray_aabb_intersection(
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

    let local_t = ray_bounds_intersection(origin_local, direction_local, bounds.min, bounds.max)?;

    let hit_local = origin_local + direction_local * local_t;
    let hit_world = (world_matrix * hit_local.extend(1.0)).truncate();
    let distance = (hit_world - origin).length();

    distance.is_finite().then_some(distance)
}

fn ray_bounds_intersection(origin: Vec3, direction: Vec3, min: Vec3, max: Vec3) -> Option<f32> {
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
