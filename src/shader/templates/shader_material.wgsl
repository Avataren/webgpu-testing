// src/shader/templates/shader_material.wgsl
//
// Default template for authoring custom shader materials.
//
// The renderer scans shader source code for the following opt-in include
// markers and inserts the corresponding shared modules automatically:
//   // @include_material_system   -> Shared material buffers and flags.
//   // @include_lighting          -> Scene lights (scene_lights_at, scene_light) and PBR helpers.
//   // @include_shadows           -> Shadow samplers and lighting integration.
//   // @include_environment       -> Environment lighting utilities.
//
// Remove the markers you do not need, or add additional ones to opt into more
// functionality. This template keeps the lighting marker enabled so the
// material starts with a basic Phong lighting model.
//
// Keep `fs_main` taking a struct with `world_pos` and `normal` (as VertexOutput below):
// the renderer then fills depth and the G-buffer (reflections, ambient occlusion, TAA)
// from this shader's own vertex stage, so vertices it moves are drawn where they end up.
// Other shapes work too, but use the undisplaced mesh for those.

// @include_material_system
// @include_lighting

// The camera (renderer/uniforms.rs, CameraUniform). With the last two matrices the
// renderer derives motion vectors for this material (TAA, upscaling).
struct Globals {
    view_proj: mat4x4<f32>,
    inverse_view_proj: mat4x4<f32>,
    camera_pos: vec3<f32>,
    mip_bias: f32,
    camera_forward: vec3<f32>,
    _padding1: f32,
    camera_up: vec3<f32>,
    _padding2: f32,
    unjittered_view_proj: mat4x4<f32>,
    prev_view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Object {
    model: mat4x4<f32>,
    material_index: u32,
    pick_id: array<u32, 2u>,
    _padding: u32,
    _padding2: array<u32, 4>,
};
@group(1) @binding(0) var<storage, read> objects: array<Object>;
@group(1) @binding(1) var<storage, read> materials: array<MaterialData>;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) tangent: vec4<f32>,
    @builtin(instance_index) instance: u32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) @interpolate(flat) instance_id: u32,
    @location(4) tangent: vec3<f32>,
    @location(5) bitangent: vec3<f32>,
};

fn compute_bitangent(normal: vec3<f32>, tangent: vec3<f32>, handedness: f32) -> vec3<f32> {
    return normalize(cross(normal, tangent) * handedness);
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    let object = objects[input.instance];
    let model = object.model;

    let world_pos = model * vec4<f32>(input.position, 1.0);
    let world_normal = normalize(normal_matrix(model) * input.normal);
    let world_tangent = normalize((model * vec4<f32>(input.tangent.xyz, 0.0)).xyz);
    let world_bitangent = compute_bitangent(world_normal, world_tangent, input.tangent.w);

    var output: VertexOutput;
    output.clip_position = globals.view_proj * world_pos;
    output.world_pos = world_pos.xyz;
    output.normal = world_normal;
    output.uv = input.uv;
    output.instance_id = input.instance;
    output.tangent = world_tangent;
    output.bitangent = world_bitangent;
    return output;
}

fn phong_light(
    N: vec3<f32>,
    V: vec3<f32>,
    L: vec3<f32>,
    light_color: vec3<f32>,
    light_intensity: f32,
    diffuse_color: vec3<f32>,
    specular_color: vec3<f32>,
    shininess: f32,
) -> vec3<f32> {
    let NdotL = max(dot(N, L), 0.0);
    if (NdotL <= 0.0) {
        return vec3<f32>(0.0);
    }

    let reflection = reflect(-L, N);
    let spec = pow(max(dot(reflection, V), 0.0), shininess);
    let diffuse = diffuse_color * NdotL;
    let specular = specular_color * spec;
    return (diffuse + specular) * light_color * light_intensity;
}

// Every light that reaches the surface: the directional lights, then the point and spot
// lights of its cluster (scene_lights_at and scene_light in lighting_common.wgsl; the
// renderer does not keep its lights in fixed arrays, so shaders read them through these).
fn phong_scene_lighting(
    world_pos: vec3<f32>,
    N: vec3<f32>,
    V: vec3<f32>,
    diffuse_color: vec3<f32>,
    specular_color: vec3<f32>,
    shininess: f32,
) -> vec3<f32> {
    var lighting = vec3<f32>(0.0);
    let here = scene_lights_at(world_pos);
    for (var i = 0u; i < scene_light_count(here); i = i + 1u) {
        let light = scene_light(here, i, world_pos);
        lighting += phong_light(
            N,
            V,
            light.direction,
            light.radiance,
            1.0,
            diffuse_color,
            specular_color,
            shininess,
        );
    }
    return lighting;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let object = objects[input.instance_id];
    let material = materials[object.material_index];
    let flags = material.material_flags;

    let base_color_sample = sample_base_color_texture(material.base_color_texture, input.uv);
    var base_color = material.color;
    if ((flags & FLAG_USE_BASE_COLOR_TEXTURE) != 0u) {
        base_color = base_color * base_color_sample;
    }

    let metallic_roughness_sample = sample_metallic_roughness_texture(
        material.metallic_roughness_texture,
        input.uv,
    );
    var metallic = material.metallic_factor;
    var roughness = material.roughness_factor;
    if ((flags & FLAG_USE_METALLIC_ROUGHNESS_TEXTURE) != 0u) {
        metallic = metallic_roughness_sample.b * material.metallic_factor;
        roughness = metallic_roughness_sample.g * material.roughness_factor;
    }
    roughness = clamp(roughness, 0.04, 1.0);

    var normal = normalize(input.normal);
    if ((flags & FLAG_USE_NORMAL_TEXTURE) != 0u) {
        let tangent_normal = sample_normal_texture(material.normal_texture, input.uv) * 2.0 - 1.0;
        let T = normalize(input.tangent);
        let B = normalize(input.bitangent);
        let N = normalize(input.normal);
        let tbn = mat3x3<f32>(T, B, N);
        normal = normalize(tbn * tangent_normal);
    }

    var emissive = vec3<f32>(0.0);
    if ((flags & FLAG_USE_EMISSIVE_TEXTURE) != 0u) {
        emissive = sample_emissive_texture(material.emissive_texture, input.uv)
            * material.emissive_strength;
    }

    var occlusion = 1.0;
    if ((flags & FLAG_USE_OCCLUSION_TEXTURE) != 0u) {
        occlusion = sample_occlusion_texture(material.occlusion_texture, input.uv);
    }

    let view_dir = normalize(globals.camera_pos - input.world_pos);
    var shading_normal = normal;
    if ((flags & FLAG_DOUBLE_SIDED) != 0u && dot(shading_normal, view_dir) < 0.0) {
        shading_normal = -shading_normal;
    }

    if ((flags & FLAG_UNLIT) != 0u) {
        return vec4<f32>(base_color.rgb + emissive, base_color.a);
    }

    let diffuse_color = base_color.rgb;
    let specular_strength = mix(0.04, 1.0, metallic);
    let specular_color = mix(vec3<f32>(specular_strength), diffuse_color, metallic);
    let shininess = mix(8.0, 128.0, clamp(1.0 - roughness, 0.0, 1.0));

    let lighting = phong_scene_lighting(
        input.world_pos,
        shading_normal,
        view_dir,
        diffuse_color,
        specular_color,
        shininess,
    );

    let ambient = diffuse_color * 0.03 * occlusion;
    var color = ambient + lighting + emissive;
    // Linear HDR: post-processing applies exposure and tonemapping.

    return vec4<f32>(color, base_color.a);
}
