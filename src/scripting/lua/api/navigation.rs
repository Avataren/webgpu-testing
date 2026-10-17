//! # Navigation API
//!
//! Navmeshes, paths and agents (see the `NavMeshSurface`, `NavAgent`, `NavObstacle` and
//! `NavModifier` components; their settings are read and changed with `get_component`
//! / `set_component`, e.g. `set_component(guard, "NavAgent", { speed = 3 })`).
//!
//! Agents walk in play mode after every script's `update` (before the physics), so a
//! destination set here is planned and walked this frame.
//!
//! ## Agents
//! - `nav_set_destination(entity, x, y, z)`: walk there (the nearest point on the
//!   navmesh). Returns false when the entity is not an agent.
//! - `nav_stop(entity)`: stop where it is.
//! - `nav_warp(entity, x, y, z)`: put it somewhere else at once.
//! - `nav_face(entity, yaw)`: turn it to face `yaw` (radians about +Y; 0 faces +Z,
//!   `math.atan(x, z)` faces a direction) while it stands; walking hands the facing back
//!   to its movement.
//! - `nav_agent_state(entity)` returns `{state, position, velocity, speed,
//!   desired_velocity, target, remaining_distance, partial, corners}` or nil; `state` is
//!   "idle", "planning", "moving", "arrived", "failed", "off_mesh" or "lost";
//!   `partial` means the way is blocked for now and it walks as near as it can.
//! - Scripts on an agent get `on_nav_arrived(self)`, `on_nav_failed(self, reason)`,
//!   `on_nav_link_start(self, {start, finish, link})` and `on_nav_link_end(self)`.
//!
//! ## Queries
//! - `nav_find_path(x0, y0, z0, x1, y1, z1[, options])` returns `{points = {{x, y, z},
//!   ...}, complete, length}` or nil; `options.agent_type` picks the navmesh.
//! - `nav_closest_point(x, y, z[, radius])` returns `{x, y, z}` or nil.
//! - `nav_random_point([x, y, z, radius])`: a random point on the navmesh (reachable
//!   from x, y, z within `radius` when given).
//! - `nav_raycast(x0, y0, z0, x1, y1, z1)` returns `{hit, point, normal}`: walking a
//!   straight line over the floor, where a wall stops it.
//! - `nav_pick(ox, oy, oz, dx, dy, dz)` returns `{x, y, z}` where a ray (from
//!   `screen_to_world_ray`) first meets a navmesh, on any floor, or nil.
//!
//! ## Navmesh
//! - `nav_debug_draw(enabled | {mesh, agents, paths, obstacles, x_ray})`.
//! - `nav_rebuild([surface])`: build the navmeshes again from the scene as it is now.
//! - `nav_status()` returns `{surfaces = {{name, tiles, polys, source, agents}}, agents,
//!   obstacles, carving}`.

use glam::Vec3;
use hecs::Entity;
use mlua::{Lua, Result as LuaResult, Table, Value};

use crate::navigation::{NavCommand, NavDebugDraw, QueryFilter};
use crate::scripting::lua::guards::{resolve_entity, with_active_commands, with_active_navigation};

fn error(message: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}

fn vec3_table(lua: &Lua, v: Vec3) -> LuaResult<Table> {
    let table = lua.create_table()?;
    table.set("x", v.x)?;
    table.set("y", v.y)?;
    table.set("z", v.z)?;
    Ok(table)
}

fn vector(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3::new(x as f32, y as f32, z as f32)
}

/// Queue a navigation command for an entity handle; false when it names nothing.
fn queue(handle: i64, make: impl FnOnce(Entity) -> NavCommand) -> LuaResult<bool> {
    with_active_commands(|commands| {
        let Ok(bits) = commands.resolve_entity_bits(handle) else {
            return Ok(false);
        };
        let Some(entity) = Entity::from_bits(bits) else {
            return Ok(false);
        };
        commands.navigation.push(make(entity));
        Ok(true)
    })
}

fn debug_from(value: Value) -> LuaResult<NavDebugDraw> {
    Ok(match value {
        Value::Boolean(true) => NavDebugDraw::all(),
        Value::Boolean(false) | Value::Nil => NavDebugDraw::default(),
        Value::Table(table) => NavDebugDraw {
            mesh: table.get::<Option<bool>>("mesh")?.unwrap_or(false),
            agents: table.get::<Option<bool>>("agents")?.unwrap_or(false),
            paths: table.get::<Option<bool>>("paths")?.unwrap_or(false),
            obstacles: table.get::<Option<bool>>("obstacles")?.unwrap_or(false),
            x_ray: table.get::<Option<bool>>("x_ray")?.unwrap_or(false),
        },
        _ => return Err(error("nav_debug_draw takes a boolean or a table")),
    })
}

pub(crate) fn register_navigation_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    globals.set(
        "nav_set_destination",
        lua.create_function(|_, (entity, x, y, z): (i64, f64, f64, f64)| {
            queue(entity, |entity| NavCommand::SetDestination {
                entity,
                target: vector(x, y, z),
            })
        })?,
    )?;

    globals.set(
        "nav_stop",
        lua.create_function(|_, entity: i64| queue(entity, |entity| NavCommand::Stop { entity }))?,
    )?;

    globals.set(
        "nav_warp",
        lua.create_function(|_, (entity, x, y, z): (i64, f64, f64, f64)| {
            queue(entity, |entity| NavCommand::Warp {
                entity,
                position: vector(x, y, z),
            })
        })?,
    )?;

    globals.set(
        "nav_steer",
        lua.create_function(|_, (entity, vx, vz, yaw): (i64, f64, f64, Option<f64>)| {
            queue(entity, |entity| NavCommand::Steer {
                entity,
                velocity: vector(vx, 0.0, vz),
                yaw: yaw.map(|yaw| yaw as f32),
            })
        })?,
    )?;

    globals.set(
        "nav_face",
        lua.create_function(|_, (entity, yaw): (i64, f64)| {
            queue(entity, |entity| NavCommand::Face {
                entity,
                yaw: yaw as f32,
            })
        })?,
    )?;

    globals.set(
        "nav_pick",
        lua.create_function(
            |lua, (ox, oy, oz, dx, dy, dz): (f64, f64, f64, f64, f64, f64)| {
                let direction = vector(dx, dy, dz).normalize_or_zero();
                let hit = with_active_navigation(|navigation| {
                    Ok(navigation.pick(vector(ox, oy, oz), direction))
                })?;
                hit.map(|p| vec3_table(lua, p)).transpose()
            },
        )?,
    )?;

    globals.set(
        "nav_agent_state",
        lua.create_function(|lua, entity: i64| {
            let Some(entity) = resolve_entity(entity) else {
                return Ok(None);
            };
            let status = with_active_navigation(|navigation| Ok(navigation.agent_status(entity)))?;
            let Some(status) = status else {
                return Ok(None);
            };
            let table = lua.create_table()?;
            table.set("state", status.state.name())?;
            table.set("position", vec3_table(lua, status.position)?)?;
            table.set("velocity", vec3_table(lua, status.velocity)?)?;
            table.set(
                "speed",
                Vec3::new(status.velocity.x, 0.0, status.velocity.z).length(),
            )?;
            table.set(
                "desired_velocity",
                vec3_table(lua, status.desired_velocity)?,
            )?;
            if let Some(target) = status.target {
                table.set("target", vec3_table(lua, target)?)?;
            }
            if let Some(remaining) = status.remaining_distance {
                table.set("remaining_distance", remaining)?;
            }
            table.set("partial", status.partial)?;
            table.set("steering", status.steering)?;
            let corners = lua.create_table()?;
            for (i, corner) in status.corners.iter().enumerate() {
                corners.set(i + 1, vec3_table(lua, *corner)?)?;
            }
            table.set("corners", corners)?;
            Ok(Some(table))
        })?,
    )?;

    globals.set(
        "nav_find_path",
        lua.create_function(
            |lua, (x0, y0, z0, x1, y1, z1, options): (f64, f64, f64, f64, f64, f64, Option<Table>)| {
                let agent_type = match &options {
                    Some(options) => options.get::<Option<String>>("agent_type")?.unwrap_or_default(),
                    None => String::new(),
                };
                let found = with_active_navigation(|navigation| {
                    Ok(navigation.find_path(vector(x0, y0, z0), vector(x1, y1, z1), &agent_type, &QueryFilter::default()))
                })?;
                let Some((points, complete, length)) = found else {
                    return Ok(None);
                };
                let table = lua.create_table()?;
                let list = lua.create_table()?;
                for (i, point) in points.iter().enumerate() {
                    list.set(i + 1, vec3_table(lua, *point)?)?;
                }
                table.set("points", list)?;
                table.set("complete", complete)?;
                table.set("length", length)?;
                Ok(Some(table))
            },
        )?,
    )?;

    globals.set(
        "nav_closest_point",
        lua.create_function(|lua, (x, y, z, radius): (f64, f64, f64, Option<f64>)| {
            let r = radius.unwrap_or(2.0).max(0.01) as f32;
            let point = with_active_navigation(|navigation| {
                Ok(navigation.closest_point(vector(x, y, z), Vec3::new(r, r.max(2.0), r), ""))
            })?;
            point.map(|p| vec3_table(lua, p)).transpose()
        })?,
    )?;

    globals.set(
        "nav_random_point",
        lua.create_function(
            |lua, (x, y, z, radius): (Option<f64>, Option<f64>, Option<f64>, Option<f64>)| {
                let around = match (x, y, z) {
                    (Some(x), Some(y), Some(z)) => {
                        Some((vector(x, y, z), radius.unwrap_or(10.0) as f32))
                    }
                    _ => None,
                };
                let point =
                    with_active_navigation(|navigation| Ok(navigation.random_point(around, "")))?;
                point.map(|p| vec3_table(lua, p)).transpose()
            },
        )?,
    )?;

    globals.set(
        "nav_raycast",
        lua.create_function(
            |lua, (x0, y0, z0, x1, y1, z1): (f64, f64, f64, f64, f64, f64)| {
                let hit = with_active_navigation(|navigation| {
                    Ok(navigation.raycast(vector(x0, y0, z0), vector(x1, y1, z1), ""))
                })?;
                let Some((blocked, point, normal)) = hit else {
                    return Ok(None);
                };
                let table = lua.create_table()?;
                table.set("hit", blocked)?;
                table.set("point", vec3_table(lua, point)?)?;
                table.set("normal", vec3_table(lua, normal)?)?;
                Ok(Some(table))
            },
        )?,
    )?;

    globals.set(
        "nav_debug_draw",
        lua.create_function(|_, value: Value| {
            let debug = debug_from(value)?;
            with_active_commands(|commands| {
                commands.navigation.push(NavCommand::SetDebugDraw(debug));
                Ok(())
            })
        })?,
    )?;

    globals.set(
        "nav_rebuild",
        lua.create_function(|_, surface: Option<i64>| {
            let surface = surface.and_then(resolve_entity);
            with_active_commands(|commands| {
                commands.navigation.push(NavCommand::Rebuild { surface });
                Ok(())
            })
        })?,
    )?;

    globals.set(
        "nav_status",
        lua.create_function(|lua, ()| {
            let table = lua.create_table()?;
            let world_surfaces = crate::scripting::lua::guards::with_active_world(|world| {
                Ok(with_active_navigation(|navigation| {
                    Ok((
                        navigation.surface_statuses(world),
                        navigation.agent_count(),
                        navigation.obstacle_count(),
                    ))
                }))
            })??;
            let (surfaces, agents, (obstacles, carving)) = world_surfaces;
            let list = lua.create_table()?;
            for (i, surface) in surfaces.iter().enumerate() {
                let entry = lua.create_table()?;
                entry.set(
                    "entity",
                    crate::scripting::lua::commands::entity_bits(surface.entity),
                )?;
                entry.set("name", surface.name.clone())?;
                entry.set("tiles", surface.tiles)?;
                entry.set("polys", surface.polys)?;
                entry.set("source", surface.source)?;
                entry.set("agents", surface.agents)?;
                if let Some(error) = &surface.error {
                    entry.set("error", error.clone())?;
                }
                list.set(i + 1, entry)?;
            }
            table.set("surfaces", list)?;
            table.set("agents", agents)?;
            table.set("obstacles", obstacles)?;
            table.set("carving", carving)?;
            Ok(table)
        })?,
    )?;

    Ok(())
}
