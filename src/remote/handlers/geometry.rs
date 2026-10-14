//! Profile geometry tools create or replace one entity, using the usual editing fields.

use serde_json::{json, Map, Value};

use crate::procgen::geometry::{self, ExtrudeOptions, LatheOptions, MeshPart};
use crate::remote::json::{entity_from_json, entity_id};

use super::{entity, RemoteContext, RemoteResult};

fn parse<T: serde::de::DeserializeOwned>(value: &Value) -> Result<T, String> {
    serde_json::from_value(value.clone()).map_err(|e| format!("invalid geometry options: {e}"))
}

pub(super) fn generate(
    method: &str,
    params: &Value,
    ctx: &mut RemoteContext<'_, '_>,
) -> RemoteResult {
    // Finish validation and meshing before touching the scene.
    let data = match method {
        "mesh.lathe" => geometry::lathe(&parse::<LatheOptions>(params)?),
        "mesh.extrude" => geometry::extrude(&parse::<ExtrudeOptions>(params)?),
        "mesh.compose" => geometry::compose(&parse::<Vec<MeshPart>>(
            params.get("parts").ok_or("parts is required")?,
        )?),
        _ => unreachable!(),
    }?;
    let mesh = geometry::generated(data);
    let target = params
        .get("entity")
        .map(|value| entity_from_json(ctx.gpu.scene.main_world(), value))
        .transpose()?;
    let mut fields = Map::new();
    for key in [
        "name",
        "translation",
        "position",
        "rotation",
        "rotation_quat",
        "scale",
        "look_at",
        "parent",
        "visible",
        "material",
        "cast_shadow",
    ] {
        if let Some(value) = params.get(key) {
            fields.insert(key.into(), value.clone());
        }
    }
    let target = match target {
        Some(target) => {
            entity::apply_patch(ctx, target, &Value::Object(fields))?;
            target
        }
        None => {
            fields.entry("cast_shadow").or_insert(json!(true));
            fields.entry("name").or_insert(json!(match method {
                "mesh.lathe" => "Lathe",
                "mesh.extrude" => "Extrusion",
                _ => "Composite",
            }));
            fields.insert("kind".into(), json!("empty"));
            fields.insert("quiet".into(), json!(true));
            let report = entity::spawn(&Value::Object(fields), ctx)?;
            entity_from_json(ctx.gpu.scene.main_world(), &report["id"])?
        }
    };
    entity::set_generated_mesh(&mut ctx.gpu.scene, ctx.gpu.renderer, target, &mesh)?;
    ctx.gpu.scene.propagate_transforms();
    ctx.mark_modified();
    let mut report = entity::get(&json!({"entity": entity_id(target)}), ctx)?;
    report["geometry"] = json!({
        "vertices": mesh.data.vertices.len(), "triangles": mesh.triangles(),
        "local_bounds": {"min": mesh.min.to_array(), "max": mesh.max.to_array()},
        "key": format!("{:016x}", mesh.key),
        "recipe": {"generator": method, "version": 1, "parameters": params},
        "notes": "UVs, normals and tangents included. project_save persists geometry and materials. Composition joins parts without boolean union."
    });
    Ok(report)
}
