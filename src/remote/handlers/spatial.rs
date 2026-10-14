//! World-space bounding-box queries, surface placement and repeatable scattering.

use super::super::json::{
    entity_from_json, entity_id, opt_bool, opt_f64, opt_str, opt_u64, vec3_from_json,
};
use super::{entity, info, RemoteContext, RemoteResult};
use crate::scene::{MeshBounds, Parent, Scene, TransformComponent, Visible, WorldTransform};
use glam::{Quat, Vec3};
use hecs::{Entity, World};
use rand::{rngs::SmallRng, Rng, SeedableRng};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug)]
pub(super) struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    fn json(self) -> Value {
        json!({"min": self.min.to_array(), "max": self.max.to_array()})
    }
    fn corners(self) -> impl Iterator<Item = Vec3> {
        (0..8).map(move |i| {
            Vec3::new(
                if i & 1 == 0 { self.min.x } else { self.max.x },
                if i & 2 == 0 { self.min.y } else { self.max.y },
                if i & 4 == 0 { self.min.z } else { self.max.z },
            )
        })
    }
    fn overlaps(self, other: Self) -> bool {
        self.max.cmpgt(other.min + Vec3::splat(1e-5)).all()
            && other.max.cmpgt(self.min + Vec3::splat(1e-5)).all()
    }
    fn ray(self, origin: Vec3, direction: Vec3, limit: f32) -> Option<(f32, Vec3)> {
        let mut enter = f32::NEG_INFINITY;
        let mut exit = f32::INFINITY;
        let mut near_normal = Vec3::ZERO;
        let mut far_normal = Vec3::ZERO;
        for axis in 0..3 {
            if direction[axis].abs() < 1e-8 {
                if origin[axis] < self.min[axis] || origin[axis] > self.max[axis] {
                    return None;
                }
                continue;
            }
            let a = (self.min[axis] - origin[axis]) / direction[axis];
            let b = (self.max[axis] - origin[axis]) / direction[axis];
            let mut normal = Vec3::ZERO;
            normal[axis] = -direction[axis].signum();
            if a.min(b) > enter {
                enter = a.min(b);
                near_normal = normal;
            }
            if a.max(b) < exit {
                exit = a.max(b);
                far_normal = -normal;
            }
            if exit < enter {
                return None;
            }
        }
        let (distance, normal) = if enter >= 0.0 {
            (enter, near_normal)
        } else {
            (exit, far_normal)
        };
        (distance >= 0.0 && distance <= limit && distance.is_finite()).then_some((distance, normal))
    }
}

pub(super) fn bounds(world: &World, entity: Entity) -> Option<Bounds> {
    let local = world.get::<&MeshBounds>(entity).ok()?;
    let transform = world.get::<&WorldTransform>(entity).ok()?;
    let mut result = Bounds {
        min: Vec3::splat(f32::INFINITY),
        max: Vec3::splat(f32::NEG_INFINITY),
    };
    for corner in (Bounds {
        min: local.min,
        max: local.max,
    })
    .corners()
    {
        let point = transform.0.matrix().transform_point3(corner);
        result.min = result.min.min(point);
        result.max = result.max.max(point);
    }
    (result.min.is_finite() && result.max.is_finite()).then_some(result)
}

fn visible(world: &World, entity: Entity) -> bool {
    let mut current = entity;
    for _ in 0..world.len() + 1 {
        if world.get::<&Visible>(current).is_ok_and(|v| !v.0) {
            return false;
        }
        match world.get::<&Parent>(current) {
            Ok(parent) => current = parent.0,
            Err(_) => return true,
        }
    }
    false
}

fn under(world: &World, entity: Entity, root: Entity) -> bool {
    let mut current = entity;
    for _ in 0..world.len() + 1 {
        if current == root {
            return true;
        }
        match world.get::<&Parent>(current) {
            Ok(parent) => current = parent.0,
            Err(_) => return false,
        }
    }
    true
}

fn selected(world: &World, params: &Value) -> Result<Vec<Entity>, String> {
    let excludes = params
        .get("exclude")
        .map(|v| {
            v.as_array()
                .ok_or("exclude must be an array")?
                .iter()
                .map(|value| entity_from_json(world, value))
                .collect::<Result<Vec<_>, _>>()
        })
        .transpose()?
        .unwrap_or_default();
    let name = opt_str(params, "name_contains").map(str::to_lowercase);
    let mut entities = world
        .iter()
        .map(|e| e.entity())
        .filter(|e| {
            !excludes.iter().any(|root| under(world, *e, *root))
                && (!opt_bool(params, "visible_only").unwrap_or(true) || visible(world, *e))
                && name.as_ref().is_none_or(|name| {
                    world
                        .get::<&crate::scene::Name>(*e)
                        .is_ok_and(|n| n.0.to_lowercase().contains(name))
                })
        })
        .collect::<Vec<_>>();
    entities.sort_unstable_by_key(|e| e.to_bits());
    Ok(entities)
}

fn vector(params: &Value, key: &str) -> Result<Vec3, String> {
    let value = vec3_from_json(
        params
            .get(key)
            .ok_or_else(|| format!("{key} is required"))?,
    )?;
    if !value.is_finite() {
        return Err(format!("{key} must be finite"));
    }
    Ok(value)
}

fn ray_hits(scene: &Scene, params: &Value) -> Result<Vec<Value>, String> {
    let origin = vector(params, "origin")?;
    let direction = vector(params, "direction")?
        .try_normalize()
        .ok_or("direction must be nonzero")?;
    let limit = opt_f64(params, "max_distance").unwrap_or(10000.0) as f32;
    if !limit.is_finite() || limit <= 0.0 {
        return Err("max_distance must be finite and positive".into());
    }
    let world = scene.main_world();
    let mut hits = selected(world,params)?.into_iter().filter_map(|e| {
        let b = bounds(world,e)?;
        let (distance,normal) = b.ray(origin,direction,limit)?;
        Some(json!({"id":entity_id(e),"name":super::super::json::entity_name(world,e),
            "distance":distance,"position":(origin+direction*distance).to_array(),"normal":normal.to_array(),"bounds":b.json()}))
    }).collect::<Vec<_>>();
    hits.sort_by(|a, b| {
        a["distance"]
            .as_f64()
            .unwrap()
            .total_cmp(&b["distance"].as_f64().unwrap())
            .then_with(|| a["id"].as_u64().cmp(&b["id"].as_u64()))
    });
    Ok(hits)
}

pub(super) fn raycast(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    ctx.gpu.scene.propagate_transforms();
    let mut hits = ray_hits(&ctx.gpu.scene, params)?;
    let total = hits.len();
    hits.truncate(opt_u64(params, "limit").unwrap_or(1).min(1000) as usize);
    Ok(json!({"total":total,"hits":hits,"precision":"world_aabb"}))
}

pub(super) fn query(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    ctx.gpu.scene.propagate_transforms();
    let region = params
        .get("bounds")
        .map(|b| -> Result<Bounds, String> {
            let region = Bounds {
                min: vector(b, "min")?,
                max: vector(b, "max")?,
            };
            if !region.max.cmpge(region.min).all() {
                return Err("bounds.max must be >= bounds.min".into());
            }
            Ok(region)
        })
        .transpose()?;
    let center = params
        .get("center")
        .map(|_| vector(params, "center"))
        .transpose()?;
    let radius = opt_f64(params, "radius").unwrap_or(f64::INFINITY) as f32;
    if radius.is_nan() || radius < 0.0 || params.get("radius").is_some() && !radius.is_finite() {
        return Err("radius must be nonnegative".into());
    }
    let world = ctx.gpu.scene.main_world();
    let mut rows = selected(world, params)?
        .into_iter()
        .filter_map(|e| {
            let b = bounds(world, e)?;
            if region.is_some_and(|region| {
                !b.max.cmpge(region.min).all() || !region.max.cmpge(b.min).all()
            }) {
                return None;
            }
            let distance = center.map(|c| c.distance(c.clamp(b.min, b.max)));
            if distance.is_some_and(|distance| distance > radius) {
                return None;
            }
            let mut row = info::entity_summary(world, e);
            row["bounds"] = b.json();
            if let Some(distance) = distance {
                row["distance"] = json!(distance);
            }
            Some(row)
        })
        .collect::<Vec<_>>();
    if center.is_some() {
        rows.sort_by(|a, b| {
            a["distance"]
                .as_f64()
                .unwrap()
                .total_cmp(&b["distance"].as_f64().unwrap())
        });
    }
    let total = rows.len();
    let offset = opt_u64(params, "offset").unwrap_or(0) as usize;
    let rows = rows
        .into_iter()
        .skip(offset)
        .take(opt_u64(params, "limit").unwrap_or(100).min(1000) as usize)
        .collect::<Vec<_>>();
    Ok(json!({"total":total,"offset":offset,"entities":rows,"precision":"world_aabb"}))
}

fn hierarchy_bounds(scene: &Scene, root: Entity) -> Option<Bounds> {
    let world = scene.main_world();
    world
        .iter()
        .map(|e| e.entity())
        .filter(|e| under(world, *e, root))
        .filter_map(|e| bounds(world, e))
        .reduce(|a, b| Bounds {
            min: a.min.min(b.min),
            max: a.max.max(b.max),
        })
}

fn set_world_position(scene: &mut Scene, e: Entity, position: Vec3) -> Result<(), String> {
    let world = scene.main_world_mut();
    let local = match world.get::<&Parent>(e) {
        Ok(parent) => {
            let transform = world
                .get::<&WorldTransform>(parent.0)
                .map_err(|_| "parent has no world transform")?;
            let matrix = transform.0.matrix();
            if matrix.determinant().abs() < 1e-8 {
                return Err("parent transform is singular".into());
            }
            matrix.inverse().transform_point3(position)
        }
        Err(_) => position,
    };
    world
        .get::<&mut TransformComponent>(e)
        .map_err(|_| "entity has no transform")?
        .0
        .translation = local;
    scene.propagate_transforms();
    Ok(())
}

pub(super) fn place(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    ctx.gpu.scene.propagate_transforms();
    let e = entity_from_json(
        ctx.gpu.scene.main_world(),
        params.get("entity").ok_or("entity required")?,
    )?;
    let mut ray = params.clone();
    let excludes = ray
        .as_object_mut()
        .unwrap()
        .entry("exclude")
        .or_insert(json!([]));
    excludes
        .as_array_mut()
        .ok_or("exclude must be an array")?
        .push(json!(entity_id(e)));
    let hit = ray_hits(&ctx.gpu.scene, &ray)?
        .into_iter()
        .next()
        .ok_or("ray did not hit a surface")?;
    let position = vec3_from_json(&hit["position"])?;
    let normal = vec3_from_json(&hit["normal"])?;
    let offset = opt_f64(params, "offset").unwrap_or(0.0) as f32;
    if !offset.is_finite() {
        return Err("offset must be finite".into());
    }
    let original_transform = *ctx
        .gpu
        .scene
        .main_world()
        .get::<&TransformComponent>(e)
        .map_err(|_| "entity has no transform")?;
    let result = (|| {
        if opt_bool(params, "align_normal").unwrap_or(false) {
            let world = ctx.gpu.scene.main_world_mut();
            let parent_rotation = world
                .get::<&Parent>(e)
                .ok()
                .and_then(|p| world.get::<&WorldTransform>(p.0).ok().map(|t| t.0.rotation))
                .unwrap_or(Quat::IDENTITY);
            world
                .get::<&mut TransformComponent>(e)
                .map_err(|_| "entity has no transform")?
                .0
                .rotation = parent_rotation.inverse() * Quat::from_rotation_arc(Vec3::Y, normal);
            ctx.gpu.scene.propagate_transforms();
        }
        let b = hierarchy_bounds(&ctx.gpu.scene, e).ok_or("entity hierarchy has no mesh bounds")?;
        let origin = ctx
            .gpu
            .scene
            .main_world()
            .get::<&WorldTransform>(e)
            .map_err(|_| "entity has no world transform")?
            .0
            .translation;
        let support = b
            .corners()
            .map(|c| (c - origin).dot(normal))
            .fold(f32::INFINITY, f32::min);
        set_world_position(
            &mut ctx.gpu.scene,
            e,
            position + normal * (offset - support),
        )?;
        let placed = hierarchy_bounds(&ctx.gpu.scene, e).unwrap();
        if opt_bool(params, "avoid_overlap").unwrap_or(false) {
            let world = ctx.gpu.scene.main_world();
            let hit_id = entity_from_json(world, &hit["id"])?;
            if world.iter().any(|row| {
                let other = row.entity();
                other != hit_id
                    && !under(world, other, e)
                    && visible(world, other)
                    && bounds(world, other).is_some_and(|b| placed.overlaps(b))
            }) {
                return Err("placement overlaps another visible mesh".into());
            }
        }
        Ok(json!({"id":entity_id(e),"surface":hit,"bounds":placed.json(),"precision":"world_aabb"}))
    })();
    match result {
        Ok(result) => {
            ctx.mark_modified();
            Ok(result)
        }
        Err(error) => {
            *ctx.gpu
                .scene
                .main_world_mut()
                .get::<&mut TransformComponent>(e)
                .map_err(|_| "entity lost its transform")? = original_transform;
            ctx.gpu.scene.propagate_transforms();
            Err(error)
        }
    }
}

pub(super) fn scatter(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    ctx.gpu.scene.propagate_transforms();
    let source = entity_from_json(
        ctx.gpu.scene.main_world(),
        params.get("entity").ok_or("entity required")?,
    )?;
    let region = params.get("bounds").ok_or("bounds {min,max} required")?;
    let min = vector(region, "min")?;
    let max = vector(region, "max")?;
    if !max.cmpge(min).all() {
        return Err("bounds.max must be >= bounds.min".into());
    }
    let count = opt_u64(params, "count").unwrap_or(10);
    let spacing = opt_f64(params, "spacing").unwrap_or(1.0) as f32;
    if !(1..=1000).contains(&count) || !spacing.is_finite() || spacing < 0.0 {
        return Err("count must be 1..=1000 and spacing finite/nonnegative".into());
    }
    let seed = opt_u64(params, "seed").unwrap_or(0);
    let mut rng = SmallRng::seed_from_u64(seed);
    let source_bounds =
        hierarchy_bounds(&ctx.gpu.scene, source).ok_or("source has no mesh bounds")?;
    let source_position = ctx
        .gpu
        .scene
        .main_world()
        .get::<&WorldTransform>(source)
        .map_err(|_| "source has no transform")?
        .0
        .translation;
    let max_slope = opt_f64(params, "max_slope").unwrap_or(45.0) as f32;
    if !(0.0..=90.0).contains(&max_slope) {
        return Err("max_slope must be 0..=90 degrees".into());
    }
    let mut positions: Vec<Vec3> = Vec::new();
    let mut boxes: Vec<Bounds> = Vec::new();
    for _ in 0..count * 200 {
        if positions.len() == count as usize {
            break;
        }
        let mut p = Vec3::new(
            rng.gen_range(min.x..=max.x),
            min.y,
            rng.gen_range(min.z..=max.z),
        );
        let mut surface = None;
        if opt_bool(params, "on_surface").unwrap_or(true) {
            let ray = json!({"origin":[p.x,max.y,p.z],"direction":[0,-1,0],"max_distance":(max.y-min.y).max(0.001),"exclude":[entity_id(source)]});
            let Some(hit) = ray_hits(&ctx.gpu.scene, &ray)?.into_iter().next() else {
                continue;
            };
            let normal = vec3_from_json(&hit["normal"])?;
            if normal.dot(Vec3::Y) < max_slope.to_radians().cos() {
                continue;
            }
            p.y = hit["position"][1].as_f64().unwrap() as f32;
            surface = hit["id"].as_u64();
        }
        p.y += source_position.y - source_bounds.min.y;
        if positions.iter().any(|other| {
            Vec3::new(p.x, 0.0, p.z).distance(Vec3::new(other.x, 0.0, other.z)) < spacing
        }) {
            continue;
        }
        let shift = p - source_position;
        let b = Bounds {
            min: source_bounds.min + shift,
            max: source_bounds.max + shift,
        };
        if boxes.iter().any(|other| b.overlaps(*other)) {
            continue;
        }
        let world = ctx.gpu.scene.main_world();
        if opt_bool(params, "avoid_overlap").unwrap_or(true)
            && world.iter().any(|row| {
                let e = row.entity();
                !under(world, e, source)
                    && Some(entity_id(e)) != surface
                    && visible(world, e)
                    && bounds(world, e).is_some_and(|other| b.overlaps(other))
            })
        {
            continue;
        }
        positions.push(p);
        boxes.push(b);
    }
    if positions.len() != count as usize {
        return Err(format!(
            "could place only {} of {count} objects within the attempt budget; nothing changed",
            positions.len()
        ));
    }
    let snapshot = crate::scene::SceneStateSnapshot::capture(&ctx.gpu.scene);
    let mut ids = Vec::new();
    for position in positions {
        let result = (|| {
            let copy = entity::duplicate(&json!({"entity":entity_id(source),"quiet":true}), ctx)?;
            let e = entity_from_json(ctx.gpu.scene.main_world(), &copy["id"])?;
            set_world_position(&mut ctx.gpu.scene, e, position)?;
            Ok::<_, String>(json!({"id":entity_id(e),"position":position.to_array()}))
        })();
        match result {
            Ok(row) => ids.push(row),
            Err(error) => {
                snapshot.restore(&mut ctx.gpu.scene);
                return Err(error);
            }
        }
    }
    ctx.mark_modified();
    Ok(json!({"count":ids.len(),"seed":seed,"entities":ids,"precision":"world_aabb"}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rays_handle_parallel_axes_inside_origins_and_flat_planes() {
        let b = Bounds {
            min: Vec3::splat(-1.0),
            max: Vec3::splat(1.0),
        };
        assert_eq!(
            b.ray(Vec3::new(0.0, 3.0, 0.0), -Vec3::Y, 10.0),
            Some((2.0, Vec3::Y))
        );
        assert_eq!(b.ray(Vec3::ZERO, Vec3::X, 10.0), Some((1.0, Vec3::X)));
        assert!(b.ray(Vec3::new(2.0, 3.0, 0.0), -Vec3::Y, 10.0).is_none());
        let plane = Bounds {
            min: Vec3::new(-2.0, 0.0, -2.0),
            max: Vec3::new(2.0, 0.0, 2.0),
        };
        assert_eq!(plane.ray(Vec3::Y, -Vec3::Y, 2.0), Some((1.0, Vec3::Y)));
    }
    #[test]
    fn world_bounds_include_rotation_and_negative_scale() {
        let mut world = World::new();
        let e = world.spawn((
            MeshBounds {
                min: Vec3::new(-1.0, -0.5, -0.5),
                max: Vec3::new(1.0, 0.5, 0.5),
            },
            WorldTransform(crate::scene::transform::Transform::from_trs(
                Vec3::X * 5.0,
                Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
                Vec3::new(-2.0, 1.0, 1.0),
            )),
        ));
        let b = bounds(&world, e).unwrap();
        assert!(b.min.abs_diff_eq(Vec3::new(4.5, -0.5, -2.0), 1e-5));
        assert!(b.max.abs_diff_eq(Vec3::new(5.5, 0.5, 2.0), 1e-5));
    }
}
