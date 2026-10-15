//! G-buffer output for shader materials.
//!
//! A shader material is a user-written module with its own `vs_main` and `fs_main`. The
//! main pass writes the G-buffer (normal and motion, world position) that reflections,
//! ambient occlusion, TAA and path tracing read. For that pass the material's `fs_main`
//! is wrapped in a generated `fs_main_gbuffer`, so the G-buffer and depth come from the
//! material's own vertex stage, displacement included.
//!
//! The wrapper needs `fs_main` to take one struct with `world_pos: vec3<f32>` and
//! `normal: vec3<f32>` members and return `@location(0) vec4<f32>` (the template's shape).
//! Motion vectors come from the camera when the module's `globals` has
//! `unjittered_view_proj` and `prev_view_proj` (zero otherwise). A module of another
//! shape gets no wrapper, and the PBR shader stands in for its G-buffer.

use naga::{ScalarKind, TypeInner, VectorSize};

/// Name `fs_main` is renamed to, so the generated entry points can call it (WGSL entry
/// points cannot be called).
const SURFACE_FN: &str = "shader_material_surface";

/// `source` (a composed shader material) with `fs_main` turned into a function that a
/// generated `fs_main` and `fs_main_gbuffer` call, or `None` when its shape does not
/// allow it (see the module docs) or the result does not validate.
pub(crate) fn with_gbuffer_entry(source: &str) -> Option<String> {
    let module = naga::front::wgsl::parse_str(source).ok()?;
    let input_type = surface_input_type(&module)?;
    let motion = has_camera_history(&module);
    if module
        .functions
        .iter()
        .any(|(_, function)| function.name.as_deref() == Some(SURFACE_FN))
        || module
            .entry_points
            .iter()
            .any(|entry| entry.name == "fs_main_gbuffer")
    {
        return None;
    }

    let mut rewritten = rename_fragment_entry(source)?;
    rewritten.push_str(&generated_entries(&input_type, motion));
    crate::renderer::shader_source::validate(&rewritten).ok()?;
    Some(rewritten)
}

/// The name of `fs_main`'s input struct, when it has the members the wrapper reads.
fn surface_input_type(module: &naga::Module) -> Option<String> {
    let entry = module
        .entry_points
        .iter()
        .find(|entry| entry.name == "fs_main" && entry.stage == naga::ShaderStage::Fragment)?;
    let [argument] = entry.function.arguments.as_slice() else {
        return None;
    };
    if argument.binding.is_some() {
        return None;
    }
    let input = &module.types[argument.ty];
    let TypeInner::Struct { members, .. } = &input.inner else {
        return None;
    };
    let is_vec3 = |name: &str| {
        members.iter().any(|member| {
            member.name.as_deref() == Some(name)
                && matches!(
                    module.types[member.ty].inner,
                    TypeInner::Vector {
                        size: VectorSize::Tri,
                        scalar: naga::Scalar {
                            kind: ScalarKind::Float,
                            width: 4
                        }
                    }
                )
        })
    };
    if !is_vec3("world_pos") || !is_vec3("normal") {
        return None;
    }
    let result = entry.function.result.as_ref()?;
    let returns_color = matches!(
        module.types[result.ty].inner,
        TypeInner::Vector {
            size: VectorSize::Quad,
            ..
        }
    ) && matches!(
        result.binding,
        Some(naga::Binding::Location { location: 0, .. })
    );
    returns_color.then(|| input.name.clone()).flatten()
}

/// Whether `globals` holds this frame's unjittered and last frame's view-projection.
fn has_camera_history(module: &naga::Module) -> bool {
    module.global_variables.iter().any(|(_, global)| {
        global.name.as_deref() == Some("globals")
            && matches!(&module.types[global.ty].inner, TypeInner::Struct { members, .. }
            if ["unjittered_view_proj", "prev_view_proj"].iter().all(|name| {
                members.iter().any(|member| member.name.as_deref() == Some(*name))
            }))
    })
}

/// `source` with `@fragment fn fs_main(...) -> @location(0) T` turned into the plain
/// function `fn shader_material_surface(...) -> T`.
fn rename_fragment_entry(source: &str) -> Option<String> {
    let mut matches = source.match_indices("fn fs_main").filter(|(index, _)| {
        let rest = &source[index + "fn fs_main".len()..];
        rest.trim_start().starts_with('(')
    });
    let (fn_index, _) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    // The `@fragment` attribute right before it (other attributes would be unexpected).
    let before = source[..fn_index].trim_end();
    let attribute = before.strip_suffix("@fragment")?;
    let header_end = fn_index + source[fn_index..].find('{')?;
    let header = &source[fn_index..header_end];
    let arrow = header.find("->")?;
    let return_type = header[arrow + 2..].trim_start();
    let return_type = return_type.strip_prefix("@location")?.trim_start();
    let return_type = return_type.strip_prefix('(')?.trim_start();
    let return_type = return_type.strip_prefix('0')?.trim_start();
    let return_type = return_type.strip_prefix(')')?.trim();
    let parameters = &header["fn fs_main".len()..arrow];

    let mut out = String::with_capacity(source.len() + 1024);
    out.push_str(attribute);
    out.push_str("\nfn ");
    out.push_str(SURFACE_FN);
    out.push_str(parameters.trim_end());
    out.push_str(" -> ");
    out.push_str(return_type);
    out.push(' ');
    out.push_str(&source[header_end..]);
    Some(out)
}

fn generated_entries(input_type: &str, motion: bool) -> String {
    let velocity = if motion {
        "    let world = vec4<f32>(input.world_pos, 1.0);
    let curr = globals.unjittered_view_proj * world;
    let prev = globals.prev_view_proj * world;
    if curr.w > 0.0 && prev.w > 0.0 {
        velocity = (curr.xy / curr.w - prev.xy / prev.w) * vec2<f32>(0.5, -0.5);
    }
"
    } else {
        ""
    };
    format!(
        "
// ---- Generated by the renderer (see renderer/internal/shader_material.rs) ----

@fragment
fn fs_main(input: {input_type}) -> @location(0) vec4<f32> {{
    return {SURFACE_FN}(input);
}}

struct ShaderMaterialGBuffer {{
    @location(0) color: vec4<f32>,
    @location(1) normal: vec4<f32>,
    @location(2) world_pos: vec4<f32>,
}};

fn shader_material_encode_normal(n: vec3<f32>) -> vec2<f32> {{
    let p = n.xy / max(abs(n.x) + abs(n.y) + abs(n.z), 1e-6);
    if n.z >= 0.0 {{
        return p;
    }}
    let signs = select(vec2<f32>(-1.0), vec2<f32>(1.0), p >= vec2<f32>(0.0));
    return (1.0 - abs(p.yx)) * signs;
}}

// The G-buffer as common.wgsl's fs_main_gbuffer writes it. Position w 1.0 is a rough
// dielectric: screen-space reflections leave the surface as the shader drew it.
@fragment
fn fs_main_gbuffer(input: {input_type}) -> ShaderMaterialGBuffer {{
    let color = {SURFACE_FN}(input);
    var velocity = vec2<f32>(0.0);
{velocity}    return ShaderMaterialGBuffer(
        vec4<f32>(color.rgb, max(color.a, 0.0)),
        vec4<f32>(shader_material_encode_normal(normalize(input.normal)), velocity),
        vec4<f32>(input.world_pos, 1.0),
    );
}}
"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::material::ShaderMaterialMetadata;
    use crate::renderer::{SamplerFilterMode, ShaderBuilder};

    fn compose(source: &str) -> String {
        ShaderBuilder::new()
            .with_material_system()
            .with_constants()
            .with_bindings_for_filter(true, SamplerFilterMode::Linear)
            .build(source)
    }

    #[test]
    fn the_template_gets_a_gbuffer_entry_with_motion() {
        let metadata = ShaderMaterialMetadata::default_template();
        let source = with_gbuffer_entry(&compose(metadata.wgsl_source()))
            .expect("the template has the shape the wrapper needs");
        assert!(source.contains("fn fs_main_gbuffer("));
        assert!(source.contains("globals.prev_view_proj * world"));
        assert_eq!(source.matches("@fragment").count(), 2);
    }

    #[test]
    fn other_shapes_keep_the_pbr_stand_in() {
        let source = compose(
            "@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    return vec4<f32>(f32(index), 0.0, 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return position;
}
",
        );
        assert!(with_gbuffer_entry(&source).is_none());
    }
}
