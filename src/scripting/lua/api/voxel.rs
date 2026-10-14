//! # Procedural geometry API
//!
//! Voxel models built from text art, a flat array or a fill function, meshed in Rust
//! into one optimized mesh per colour (see [`crate::procgen::voxel`]), plus gradient
//! noise for procedural shapes.
//!
//! - `voxel_model(desc)` -> model table: `size`, `voxel`, `voxels`, `quads`, `triangles`,
//!   `cube_quads` (what one cube per voxel would draw), `min`, `max`, `palette`, and
//!   `parts` = `{ {key, keys, mesh, quads, voxels}, ... }` in order of first use.
//!   `part.mesh` goes to `set_mesh`. Look fields of `desc`: `ao` (baked ambient occlusion
//!   strength, 0..1, default 1), `colors` (key -> vertex colour, which multiplies the
//!   material colour), `parts` (key -> part key, so several keys share one mesh) and
//!   `part` (the part of every key `parts` does not list).
//! - `spawn_voxels(model_or_desc[, name[, opts]])` -> `root, parts`: an entity with one
//!   child per part; `parts[key]` is the child. `opts.materials[key]` is an entity to
//!   share the material of, a colour, or material fields; `opts.parent`, `opts.cast_shadow`.
//! - `noise(x[, y[, z]])`, `fbm(x, y, z[, octaves])` - gradient noise in about -1..1.

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use mlua::{
    Function, Lua, LuaSerdeExt, Result as LuaResult, Table, UserData, UserDataFields,
    Value as LuaValue,
};
use serde_json::json;

use super::material::material_patch;
use super::particles::color as color_from_lua;
use crate::procgen::noise::{fbm3, perlin3};
use crate::procgen::voxel::{VoxelKey, VoxelLook, VoxelModel, VoxelSpec};
use crate::procgen::GeneratedMesh;
use crate::scripting::lua::commands::AssetCommand;
use crate::scripting::lua::guards::with_active_commands;

/// A generated mesh handed to Lua; `set_mesh(entity, mesh)` draws it.
#[derive(Clone)]
pub(crate) struct LuaGeneratedMesh(pub Arc<GeneratedMesh>);

impl UserData for LuaGeneratedMesh {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("triangles", |_, this| Ok(this.0.triangles()));
        fields.add_field_method_get("vertices", |_, this| Ok(this.0.data.vertices.len()));
        fields.add_field_method_get("min", |_, this| Ok(this.0.min.to_array()));
        fields.add_field_method_get("max", |_, this| Ok(this.0.max.to_array()));
    }

    fn add_methods<M: mlua::UserDataMethods<Self>>(methods: &mut M) {
        methods.add_meta_method(mlua::MetaMethod::ToString, |_, this, ()| {
            Ok(format!("GeneratedMesh({} triangles)", this.0.triangles()))
        });
    }
}

fn runtime_error(message: impl Into<String>) -> mlua::Error {
    mlua::Error::RuntimeError(message.into())
}

/// A cell value from Lua: nil, false, 0 and "" are empty; true is key 1.
pub(crate) fn key_from_lua(value: &LuaValue) -> LuaResult<Option<VoxelKey>> {
    Ok(match value {
        LuaValue::Nil | LuaValue::Boolean(false) => None,
        LuaValue::Boolean(true) => Some(VoxelKey::Int(1)),
        LuaValue::Integer(0) => None,
        LuaValue::Integer(n) => Some(VoxelKey::Int(*n)),
        LuaValue::Number(n) if *n == 0.0 => None,
        LuaValue::Number(n) if n.fract() == 0.0 && n.abs() < 9.0e15 => {
            Some(VoxelKey::Int(*n as i64))
        }
        LuaValue::String(s) => {
            let s = s.to_str()?;
            (!s.is_empty()).then(|| VoxelKey::Str(s.to_string()))
        }
        other => {
            return Err(runtime_error(format!(
                "a voxel must be a key (string or integer), true, or nil/false/0 for empty, \
                 not {}",
                match other {
                    LuaValue::Number(n) => n.to_string(),
                    other => other.type_name().to_string(),
                }
            )))
        }
    })
}

fn key_to_lua(lua: &Lua, key: &VoxelKey) -> LuaResult<LuaValue> {
    Ok(match key {
        VoxelKey::Int(n) => LuaValue::Integer(*n),
        VoxelKey::Str(s) => LuaValue::String(lua.create_string(s)?),
    })
}

fn vec3_field(table: &Table, field: &str) -> LuaResult<Option<Vec3>> {
    match table.get::<LuaValue>(field)? {
        LuaValue::Nil => Ok(None),
        LuaValue::Integer(n) => Ok(Some(Vec3::splat(n as f32))),
        LuaValue::Number(n) => Ok(Some(Vec3::splat(n as f32))),
        LuaValue::Table(t) => {
            let get = |i: usize| t.get::<f32>(i);
            Ok(Some(Vec3::new(get(1)?, get(2)?, get(3)?)))
        }
        other => Err(runtime_error(format!(
            "{field} must be a number or {{x, y, z}}, not {}",
            other.type_name()
        ))),
    }
}

fn rows_of(table: Table, what: &str) -> LuaResult<Vec<String>> {
    table
        .sequence_values::<LuaValue>()
        .enumerate()
        .map(|(i, row)| match row? {
            LuaValue::String(s) => Ok(s.to_str()?.to_string()),
            other => Err(runtime_error(format!(
                "{what} entry {} must be a string, not {}",
                i + 1,
                other.type_name()
            ))),
        })
        .collect()
}

/// Read a model description and build the model.
fn build_model(desc: &Table) -> LuaResult<VoxelModel> {
    let mut spec = VoxelSpec::default();

    let rows: Option<Table> = desc.get("rows")?;
    let layers: Option<Table> = desc.get("layers")?;
    spec.layers = match (rows, layers) {
        (Some(_), Some(_)) => return Err(runtime_error("give rows or layers, not both")),
        (Some(rows), None) => Some(vec![rows_of(rows, "rows")?]),
        (None, Some(layers)) => Some(
            layers
                .sequence_values::<Table>()
                .enumerate()
                .map(|(i, layer)| rows_of(layer?, &format!("layer {}", i + 1)))
                .collect::<LuaResult<_>>()?,
        ),
        (None, None) => None,
    };

    if let Some(legend) = desc.get::<Option<Table>>("legend")? {
        let mut map = HashMap::new();
        for pair in legend.pairs::<String, LuaValue>() {
            let (ch, value) = pair?;
            let mut chars = ch.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return Err(runtime_error(format!(
                    "legend keys must be single characters, not {ch:?}"
                )));
            };
            map.insert(c, key_from_lua(&value)?);
        }
        spec.legend = Some(map);
    }

    if let Some(depth) = desc.get::<Option<u32>>("depth")? {
        spec.depth = depth.max(1);
    }
    if let Some(size) = desc.get::<Option<Table>>("size")? {
        let get = |i: usize| size.get::<u32>(i);
        spec.size = Some([get(1)?, get(2)?, get(3)?]);
    }
    if let Some(voxels) = desc.get::<Option<Table>>("voxels")? {
        let [w, h, d] = spec
            .size
            .ok_or_else(|| runtime_error("voxels needs size = {width, height, depth}"))?;
        let count = w as usize * h as usize * d as usize;
        if count > crate::procgen::voxel::MAX_CELLS {
            return Err(runtime_error(format!("size {w}x{h}x{d} is too large")));
        }
        // Read by index so holes (nil) are empty cells.
        spec.voxels = Some(
            (1..=count)
                .map(|i| key_from_lua(&voxels.raw_get::<LuaValue>(i)?))
                .collect::<LuaResult<_>>()?,
        );
    }
    if let Some(voxel) = vec3_field(desc, "voxel")? {
        if voxel.min_element() <= 0.0 {
            return Err(runtime_error("voxel size must be positive"));
        }
        spec.voxel_size = voxel;
    }
    if let Some(pivot) = vec3_field(desc, "pivot")? {
        spec.pivot = pivot;
    }
    spec.look = look_from_lua(desc)?;

    let fill: Option<Function> = desc.get("fill")?;
    let (grid, palette) = match fill {
        Some(fill) => {
            let mut call = |x: u32, y: u32, z: u32| -> Result<Option<VoxelKey>, String> {
                let value = fill
                    .call::<LuaValue>((x, y, z))
                    .map_err(|err| format!("fill({x}, {y}, {z}) failed: {err}"))?;
                key_from_lua(&value).map_err(|err| format!("fill({x}, {y}, {z}): {err}"))
            };
            spec.grid(Some(&mut call))
        }
        None => spec.grid(None),
    }
    .map_err(runtime_error)?;

    Ok(VoxelModel::build(&grid, &palette, &spec))
}

/// `ao`, `colors` and `parts` of a model description.
fn look_from_lua(desc: &Table) -> LuaResult<VoxelLook> {
    let mut look = VoxelLook::default();
    if let Some(ao) = desc.get::<Option<f32>>("ao")? {
        look.ao = ao.clamp(0.0, 1.0);
    }
    if let Some(colors) = desc.get::<Option<Table>>("colors")? {
        for pair in colors.pairs::<LuaValue, LuaValue>() {
            let (key, color) = pair?;
            let Some(key) = key_from_lua(&key)? else {
                continue;
            };
            let color = color_from_lua(&color, &format!("colors[{key}]"))?;
            look.colors.insert(key, color);
        }
    }
    if let Some(parts) = desc.get::<Option<Table>>("parts")? {
        for pair in parts.pairs::<LuaValue, LuaValue>() {
            let (key, part) = pair?;
            if let (Some(key), Some(part)) = (key_from_lua(&key)?, key_from_lua(&part)?) {
                look.parts.insert(key, part);
            }
        }
    }
    look.part = key_from_lua(&desc.get::<LuaValue>("part")?)?;
    Ok(look)
}

/// The Lua table `voxel_model` returns.
fn model_table(lua: &Lua, model: &VoxelModel, palette: Option<LuaValue>) -> LuaResult<Table> {
    let table = lua.create_table()?;
    table.set("size", model.size.to_vec())?;
    table.set("voxel", model.voxel_size.to_array())?;
    table.set("voxels", model.voxels)?;
    table.set("quads", model.quads())?;
    table.set(
        "triangles",
        model
            .parts
            .iter()
            .map(|p| p.mesh.triangles())
            .sum::<usize>(),
    )?;
    table.set("cube_quads", model.voxels * 6)?;
    let max = model.origin
        + Vec3::new(
            model.size[0] as f32,
            model.size[1] as f32,
            model.size[2] as f32,
        ) * model.voxel_size;
    table.set("min", model.origin.to_array())?;
    table.set("max", max.to_array())?;
    if let Some(palette) = palette {
        table.set("palette", palette)?;
    }
    let parts = lua.create_table()?;
    for (i, part) in model.parts.iter().enumerate() {
        let entry = lua.create_table()?;
        entry.set("key", key_to_lua(lua, &part.key)?)?;
        let keys = lua.create_table()?;
        for (i, key) in part.keys.iter().enumerate() {
            keys.set(i + 1, key_to_lua(lua, key)?)?;
        }
        entry.set("keys", keys)?;
        entry.set("mesh", LuaGeneratedMesh(part.mesh.clone()))?;
        entry.set("quads", part.quads)?;
        entry.set("voxels", part.voxels)?;
        parts.set(i + 1, entry)?;
    }
    table.set("parts", parts)?;
    Ok(table)
}

/// How one part of `spawn_voxels` gets its material.
enum PartMaterial {
    Share(i64),
    Patch(serde_json::Value),
    Default,
}

fn part_material(lua: &Lua, spec: LuaValue) -> LuaResult<PartMaterial> {
    Ok(match spec {
        LuaValue::Nil => PartMaterial::Default,
        LuaValue::Integer(entity) => PartMaterial::Share(entity),
        LuaValue::String(color) => {
            PartMaterial::Patch(json!({ "color": color.to_str()?.to_string() }))
        }
        LuaValue::Table(table) => {
            // {r, g, b[, a]} is a colour; anything else is material fields.
            if table.raw_len() > 0 {
                let color: Vec<f64> = lua.from_value(LuaValue::Table(table))?;
                PartMaterial::Patch(json!({ "color": color }))
            } else {
                PartMaterial::Patch(material_patch(lua, LuaValue::Table(table), "spawn_voxels")?)
            }
        }
        other => {
            return Err(runtime_error(format!(
                "a voxel material must be an entity, a colour or a table of material fields, \
                 not {}",
                other.type_name()
            )))
        }
    })
}

pub(crate) fn register_voxel_api(lua: &Lua) -> LuaResult<()> {
    let globals = lua.globals();

    globals.set(
        "voxel_model",
        lua.create_function(|lua, desc: Table| {
            let model = build_model(&desc)?;
            model_table(lua, &model, desc.get("palette")?)
        })?,
    )?;

    globals.set(
        "spawn_voxels",
        lua.create_function(
            |lua, (source, name, opts): (Table, Option<String>, Option<Table>)| {
                // A model from voxel_model, or a description to build one from.
                let model = match source.get::<Option<Table>>("parts")? {
                    Some(_) => source.clone(),
                    None => {
                        let model = build_model(&source)?;
                        model_table(lua, &model, source.get("palette")?)?
                    }
                };
                let palette: Option<Table> = model.get("palette")?;
                let materials: Option<Table> = match &opts {
                    Some(opts) => opts.get("materials")?,
                    None => None,
                };
                let parent: Option<i64> = match &opts {
                    Some(opts) => opts.get("parent")?,
                    None => None,
                };
                let cast_shadow = match &opts {
                    Some(opts) => opts.get::<Option<bool>>("cast_shadow")?.unwrap_or(true),
                    None => true,
                };

                // Everything that reads Lua happens before the command queue is borrowed.
                let mut parts = Vec::new();
                for part in model.get::<Table>("parts")?.sequence_values::<Table>() {
                    let part = part?;
                    let key: LuaValue = part.get("key")?;
                    let mesh = part.get::<mlua::AnyUserData>("mesh")?;
                    let mesh = mesh.borrow::<LuaGeneratedMesh>()?.0.clone();
                    let mut spec = match &materials {
                        Some(materials) => materials.get::<LuaValue>(key.clone())?,
                        None => LuaValue::Nil,
                    };
                    if spec.is_nil() {
                        if let Some(palette) = &palette {
                            spec = palette.get::<LuaValue>(key.clone())?;
                        }
                    }
                    // A key that is a colour colours itself.
                    if spec.is_nil() {
                        if let LuaValue::String(s) = &key {
                            if s.to_str()?.starts_with('#') {
                                spec = key.clone();
                            }
                        }
                    }
                    let label = match &key {
                        LuaValue::String(s) => s.to_str()?.to_string(),
                        other => other.to_string()?,
                    };
                    parts.push((key, label, mesh, part_material(lua, spec)?));
                }

                let name = name.unwrap_or_else(|| "Voxels".into());
                let (root, children) = with_active_commands(|commands| {
                    let root = commands.spawn_entity(Some(name.clone()));
                    if let Some(parent) = parent {
                        commands.set_parent(root, Some(parent))?;
                    }
                    let mut children = Vec::new();
                    for (key, label, mesh, material) in parts {
                        let child = commands.spawn_entity(Some(format!("{name}.{label}")));
                        commands.set_parent(child, Some(root))?;
                        // Shared first, so the mesh finds a material and makes none.
                        if let PartMaterial::Share(source) = material {
                            commands.resolve_or_pending(source)?;
                            commands.asset_command(child, AssetCommand::ShareMaterial(source))?;
                        }
                        commands.asset_command(child, AssetCommand::GeneratedMesh(mesh))?;
                        if let PartMaterial::Patch(patch) = material {
                            commands.asset_command(child, AssetCommand::Material(patch))?;
                        }
                        if !cast_shadow {
                            commands.set_component(child, "CanCastShadow".into(), json!(false))?;
                        }
                        children.push((key, child));
                    }
                    Ok((root, children))
                })?;

                let table = lua.create_table()?;
                for (key, child) in children {
                    table.set(key, child)?;
                }
                Ok((root, table))
            },
        )?,
    )?;

    globals.set(
        "noise",
        lua.create_function(|_, (x, y, z): (f64, Option<f64>, Option<f64>)| {
            Ok(perlin3(x, y.unwrap_or(0.0), z.unwrap_or(0.0)))
        })?,
    )?;
    globals.set(
        "fbm",
        lua.create_function(|_, (x, y, z, octaves): (f64, f64, f64, Option<u32>)| {
            Ok(fbm3(x, y, z, octaves.unwrap_or(4)))
        })?,
    )?;

    Ok(())
}
