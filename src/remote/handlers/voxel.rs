//! `voxel.spawn`: build a voxel model and spawn it as an entity with one child per colour.

use std::collections::HashMap;

use glam::Vec3;
use mlua::{Function, HookTriggers, Lua, LuaOptions, StdLib, Value as LuaValue, VmState};
use serde_json::{json, Map, Value};

use super::super::json::{color_from_json, entity_from_json, entity_id, opt_bool, opt_u64};
use super::entity::{apply_patch, set_generated_mesh, spawn};
use super::{RemoteContext, RemoteResult};
use crate::procgen::noise::{fbm3, perlin3};
use crate::procgen::voxel::{VoxelKey, VoxelModel, VoxelSpec};
use crate::scene::{Name, Transform, TransformComponent, Visible};
use crate::scripting::lua::api::voxel::key_from_lua;

/// How long a `fill` expression may run before it is stopped.
const FILL_SECONDS: f64 = 10.0;

/// Transform and hierarchy fields that go to the root entity.
const ROOT_FIELDS: &[&str] = &[
    "name",
    "translation",
    "position",
    "rotation",
    "rotation_quat",
    "scale",
    "look_at",
    "parent",
    "visible",
];

fn key_from_json(value: &Value) -> Result<Option<VoxelKey>, String> {
    Ok(match value {
        Value::Null | Value::Bool(false) => None,
        Value::Bool(true) => Some(VoxelKey::Int(1)),
        Value::Number(n) => match n.as_i64() {
            Some(0) => None,
            Some(n) => Some(VoxelKey::Int(n)),
            None => return Err(format!("voxel keys must be integers, not {n}")),
        },
        Value::String(s) if s.is_empty() => None,
        Value::String(s) => Some(VoxelKey::Str(s.clone())),
        other => {
            return Err(format!(
                "a voxel must be a string, integer, true or null, not {other}"
            ))
        }
    })
}

fn key_json(key: &VoxelKey) -> Value {
    match key {
        VoxelKey::Int(n) => json!(n),
        VoxelKey::Str(s) => json!(s),
    }
}

fn rows_from_json(value: &Value, what: &str) -> Result<Vec<String>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{what} must be a list of strings"))?
        .iter()
        .map(|row| {
            row.as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("{what} must hold strings, not {row}"))
        })
        .collect()
}

fn vec3_param(params: &Value, key: &str) -> Result<Option<Vec3>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Number(n)) => Ok(Some(Vec3::splat(n.as_f64().unwrap_or(0.0) as f32))),
        Some(Value::Array(items)) if items.len() == 3 => {
            let get = |i: usize| {
                items[i]
                    .as_f64()
                    .map(|v| v as f32)
                    .ok_or_else(|| format!("{key} must hold numbers"))
            };
            Ok(Some(Vec3::new(get(0)?, get(1)?, get(2)?)))
        }
        Some(other) => Err(format!("{key} must be a number or [x, y, z], not {other}")),
    }
}

fn spec_from_params(params: &Value) -> Result<VoxelSpec, String> {
    let layers = match (params.get("rows"), params.get("layers")) {
        (Some(_), Some(_)) => return Err("give rows or layers, not both".into()),
        (Some(rows), None) => Some(vec![rows_from_json(rows, "rows")?]),
        (None, Some(layers)) => Some(
            layers
                .as_array()
                .ok_or("layers must be a list of row lists")?
                .iter()
                .enumerate()
                .map(|(i, layer)| rows_from_json(layer, &format!("layer {}", i + 1)))
                .collect::<Result<_, _>>()?,
        ),
        (None, None) => None,
    };
    let mut spec = VoxelSpec {
        layers,
        ..VoxelSpec::default()
    };
    if let Some(legend) = params.get("legend") {
        let legend = legend.as_object().ok_or("legend must be an object")?;
        let mut map = HashMap::new();
        for (ch, value) in legend {
            let mut chars = ch.chars();
            let (Some(c), None) = (chars.next(), chars.next()) else {
                return Err(format!("legend keys must be single characters, not {ch:?}"));
            };
            map.insert(c, key_from_json(value)?);
        }
        spec.legend = Some(map);
    }
    if let Some(depth) = opt_u64(params, "depth") {
        spec.depth = depth.clamp(1, 1024) as u32;
    }
    if let Some(size) = params.get("size") {
        let size: Vec<u32> = serde_json::from_value(size.clone())
            .map_err(|_| "size must be [width, height, depth]")?;
        let [w, h, d] = size[..] else {
            return Err("size must be [width, height, depth]".into());
        };
        spec.size = Some([w, h, d]);
    }
    if let Some(voxels) = params.get("voxels") {
        spec.voxels = Some(
            voxels
                .as_array()
                .ok_or("voxels must be a flat list, x fastest, then y, then z")?
                .iter()
                .map(key_from_json)
                .collect::<Result<_, _>>()?,
        );
    }
    if let Some(voxel) = vec3_param(params, "voxel")? {
        if voxel.min_element() <= 0.0 {
            return Err("voxel size must be positive".into());
        }
        spec.voxel_size = voxel;
    }
    if let Some(pivot) = vec3_param(params, "pivot")? {
        spec.pivot = pivot;
    }
    if let Some(ao) = params.get("ao") {
        let ao = ao.as_f64().ok_or("ao must be a number from 0 to 1")?;
        spec.look.ao = (ao as f32).clamp(0.0, 1.0);
    }
    if let Some(colors) = params.get("colors") {
        let colors = colors
            .as_object()
            .ok_or("colors must be an object of key -> colour")?;
        for (key, color) in colors {
            let color = color_from_json(color).map_err(|err| format!("colors.{key}: {err}"))?;
            spec.look.colors.insert(object_key(key), color);
        }
    }
    if let Some(parts) = params.get("parts") {
        let parts = parts
            .as_object()
            .ok_or("parts must be an object of key -> part key")?;
        for (key, part) in parts {
            if let Some(part) = key_from_json(part)? {
                spec.look.parts.insert(object_key(key), part);
            }
        }
    }
    if let Some(part) = params.get("part") {
        spec.look.part = key_from_json(part)?;
    }
    Ok(spec)
}

/// A key written as a JSON object key: digits are an integer key.
fn object_key(key: &str) -> VoxelKey {
    key.parse::<i64>()
        .map(VoxelKey::Int)
        .unwrap_or_else(|_| VoxelKey::Str(key.to_owned()))
}

/// A sandboxed Lua function `(x, y, z) -> key` from a `fill` expression or chunk. It sees
/// `math`, `string`, `table`, `noise`, `fbm` and the grid size `W`, `H`, `D`, nothing else.
fn fill_function(source: &str, size: [u32; 3], seconds: f64) -> Result<(Lua, Function), String> {
    let lua = Lua::new_with(
        StdLib::MATH | StdLib::STRING | StdLib::TABLE,
        LuaOptions::default(),
    )
    .map_err(|err| err.to_string())?;
    let globals = lua.globals();
    let setup = || -> mlua::Result<()> {
        globals.set("W", size[0])?;
        globals.set("H", size[1])?;
        globals.set("D", size[2])?;
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
    };
    setup().map_err(|err| err.to_string())?;

    // An expression (`y < 4 and "grass"`) or statements that return a key.
    let function = lua
        .load(format!("return function(x, y, z) return ({source}) end"))
        .eval::<Function>()
        .or_else(|_| {
            lua.load(format!("return function(x, y, z) {source}\nend"))
                .eval::<Function>()
        })
        .map_err(|err| format!("fill does not compile: {err}"))?;

    let deadline = instant::Instant::now() + std::time::Duration::from_secs_f64(seconds);
    lua.set_hook(
        HookTriggers::new().every_nth_instruction(10_000),
        move |_, _| {
            if instant::Instant::now() > deadline {
                Err(mlua::Error::RuntimeError(format!(
                    "fill ran longer than {seconds} s"
                )))
            } else {
                Ok(VmState::Continue)
            }
        },
    );
    Ok((lua, function))
}

/// Build the model `params` describe.
fn build_model(params: &Value) -> Result<VoxelModel, String> {
    build_model_within(params, FILL_SECONDS)
}

fn build_model_within(params: &Value, fill_seconds: f64) -> Result<VoxelModel, String> {
    let spec = spec_from_params(params)?;
    let (grid, palette) = match params.get("fill").and_then(Value::as_str) {
        Some(source) => {
            let size = spec
                .size
                .ok_or("fill needs size = [width, height, depth]")?;
            let (_lua, fill) = fill_function(source, size, fill_seconds)?;
            let mut call = |x: u32, y: u32, z: u32| -> Result<Option<VoxelKey>, String> {
                let value = fill
                    .call::<LuaValue>((x, y, z))
                    .map_err(|err| format!("fill({x}, {y}, {z}) failed: {err}"))?;
                key_from_lua(&value).map_err(|err| format!("fill({x}, {y}, {z}): {err}"))
            };
            spec.grid(Some(&mut call))?
        }
        None => spec.grid(None)?,
    };
    Ok(VoxelModel::build(&grid, &palette, &spec))
}

/// The material patch for one part: a colour string, `[r, g, b(, a)]` or material fields.
fn part_material(palette: Option<&Value>, key: &VoxelKey) -> Result<Option<Value>, String> {
    let entry = palette.and_then(|palette| palette.get(key.to_string()));
    Ok(match entry {
        Some(Value::String(_)) | Some(Value::Array(_)) => Some(json!({ "color": entry })),
        Some(Value::Object(fields)) => Some(Value::Object(fields.clone())),
        Some(Value::Null) | None => match key {
            // A key that is a colour colours itself.
            VoxelKey::Str(s) if s.starts_with('#') => Some(json!({ "color": s })),
            _ => None,
        },
        Some(other) => {
            return Err(format!(
                "palette entry for {key} must be a colour or material fields, not {other}"
            ))
        }
    })
}

pub(super) fn spawn_voxels(params: &Value, ctx: &mut RemoteContext<'_, '_>) -> RemoteResult {
    let model = build_model(params)?;
    let palette = params.get("palette");
    // Check every palette entry before anything is spawned.
    let materials = model
        .parts
        .iter()
        .map(|part| part_material(palette, &part.key))
        .collect::<Result<Vec<_>, _>>()?;

    let mut root_params = Map::new();
    root_params.insert("kind".into(), json!("empty"));
    root_params.insert("quiet".into(), json!(true));
    for &field in ROOT_FIELDS {
        if let Some(value) = params.get(field) {
            root_params.insert(field.into(), value.clone());
        }
    }
    if !root_params.contains_key("name") {
        root_params.insert("name".into(), json!("Voxels"));
    }
    let name = root_params["name"].as_str().unwrap_or("Voxels").to_owned();
    let root_value = spawn(&Value::Object(root_params), ctx)?;
    let root = entity_from_json(ctx.gpu.scene.main_world(), &root_value["id"])?;

    let cast_shadow = opt_bool(params, "cast_shadow").unwrap_or(true);
    let mut parts = Vec::new();
    for (part, material) in model.parts.iter().zip(materials) {
        let child = ctx.gpu.scene.main_world_mut().spawn((
            Name::new(format!("{name}.{}", part.key)),
            TransformComponent(Transform::default()),
            Visible(true),
        ));
        set_generated_mesh(
            &mut ctx.gpu.scene,
            &mut *ctx.gpu.renderer,
            child,
            &part.mesh,
        )?;
        let mut patch = json!({ "parent": entity_id(root), "cast_shadow": cast_shadow });
        if let Some(material) = material {
            patch["material"] = material;
        }
        apply_patch(ctx, child, &patch)?;
        parts.push(json!({
            "key": key_json(&part.key),
            "keys": part.keys.iter().map(key_json).collect::<Vec<_>>(),
            "entity": entity_id(child),
            "quads": part.quads,
            "voxels": part.voxels,
        }));
    }
    ctx.gpu.scene.propagate_transforms();
    ctx.mark_modified();

    let triangles: usize = model.parts.iter().map(|part| part.mesh.triangles()).sum();
    Ok(json!({
        "id": entity_id(root),
        "name": name,
        "size": model.size,
        "voxels": model.voxels,
        "quads": model.quads(),
        "triangles": triangles,
        "cube_quads": model.voxels * 6,
        "parts": parts,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_text_art_with_a_legend_builds() {
        let model = build_model(&json!({
            "rows": ["RR", "GR"],
            "legend": { "R": "red", "G": 2 },
            "depth": 3,
            "voxel": 0.25,
        }))
        .unwrap();
        assert_eq!(model.size, [2, 2, 3]);
        assert_eq!(model.parts.len(), 2);
        assert_eq!(model.parts[0].key, VoxelKey::Str("red".into()));
        assert_eq!(model.parts[1].key, VoxelKey::Int(2));
        assert_eq!(model.voxel_size, Vec3::splat(0.25));
    }

    #[test]
    fn fill_expressions_and_chunks_run_sandboxed() {
        let model = build_model(&json!({
            "size": [8, 8, 8],
            "fill": "y < 2 and 'ground' or (y < 4 + noise(x * 0.3, z * 0.3) * 2 and 'hill')",
        }))
        .unwrap();
        assert!(model.part(&VoxelKey::Str("ground".into())).is_some());
        assert_eq!(
            model.part(&VoxelKey::Str("ground".into())).unwrap().voxels,
            128
        );

        let model = build_model(&json!({
            "size": [4, 4, 4],
            "fill": "local r = W / 2\nif (x - 1.5)^2 + (y - 1.5)^2 + (z - 1.5)^2 < r * r then return 1 end",
        }))
        .unwrap();
        assert_eq!(model.voxels, 32);

        let err = build_model(&json!({ "size": [2, 2, 2], "fill": "os.exit(1)" })).unwrap_err();
        assert!(err.contains("os"), "{err}");
        let endless = json!({ "size": [2, 2, 2], "fill": "while true do end" });
        let err = build_model_within(&endless, 0.2).map(|_| ()).unwrap_err();
        assert!(err.contains("longer than"), "{err}");
    }

    #[test]
    fn look_params_group_keys_and_colour_vertices() {
        let model = build_model(&json!({
            "rows": ["AB", "C."],
            "ao": 0,
            "colors": { "A": "#ff0000", "2": [0, 0, 1] },
            "parts": { "C": "trim" },
            "part": "body",
            "legend": { "A": "A", "B": 2, "C": "C" },
        }))
        .unwrap();
        let keys: Vec<_> = model.parts.iter().map(|p| p.key.clone()).collect();
        assert_eq!(
            keys,
            vec![VoxelKey::Str("body".into()), VoxelKey::Str("trim".into())]
        );
        let body = &model.parts[0];
        assert_eq!(body.keys, vec![VoxelKey::Str("A".into()), VoxelKey::Int(2)]);
        let colors: Vec<[f32; 4]> = body.mesh.data.vertices.iter().map(|v| v.color).collect();
        assert!(colors.contains(&[1.0, 0.0, 0.0, 1.0]));
        assert!(colors.contains(&[0.0, 0.0, 1.0, 1.0]));
        assert!(build_model(&json!({ "rows": ["A"], "colors": { "A": "blue" } })).is_err());
    }

    #[test]
    fn palette_entries_become_material_patches() {
        let palette = json!({ "red": "#ff0000", "2": [0, 1, 0], "glow": { "emissive": 3 } });
        let red = part_material(Some(&palette), &VoxelKey::Str("red".into())).unwrap();
        assert_eq!(red, Some(json!({ "color": "#ff0000" })));
        let two = part_material(Some(&palette), &VoxelKey::Int(2)).unwrap();
        assert_eq!(two, Some(json!({ "color": [0, 1, 0] })));
        let glow = part_material(Some(&palette), &VoxelKey::Str("glow".into())).unwrap();
        assert_eq!(glow, Some(json!({ "emissive": 3 })));
        let hex = part_material(None, &VoxelKey::Str("#00ff00".into())).unwrap();
        assert_eq!(hex, Some(json!({ "color": "#00ff00" })));
        assert_eq!(part_material(None, &VoxelKey::Int(7)).unwrap(), None);
    }
}
