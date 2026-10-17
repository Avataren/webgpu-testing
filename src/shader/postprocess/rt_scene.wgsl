// The ray tracing scene (src/renderer/raytracing/scene.rs): its acceleration structure,
// geometry pool, instances and materials, bound as group 3. Passes that bind the scene
// as another group get this text with the group number replaced
// (raytracing::scene_shader_modules).

// The engine's `Vertex` (src/renderer/vertex.rs), as the ray tracing pool stores it;
// scalars keep the vec3s at their packed offsets. `RtInstance` mirrors
// src/renderer/raytracing/scene.rs.
struct RtVertex {
    px : f32,
    py : f32,
    pz : f32,
    nx : f32,
    ny : f32,
    nz : f32,
    u : f32,
    v : f32,
    tangent : vec4<f32>,
    color : vec4<f32>,
};

fn rt_vertex_position(vertex : RtVertex) -> vec3<f32> {
    return vec3<f32>(vertex.px, vertex.py, vertex.pz);
}

fn rt_vertex_normal(vertex : RtVertex) -> vec3<f32> {
    return vec3<f32>(vertex.nx, vertex.ny, vertex.nz);
}

// `material_index` is into `rt_particle_materials` for a particle system
// (RT_INSTANCE_PARTICLES), else into `rt_materials`.
struct RtInstance {
    vertex_base : u32,
    index_base : u32,
    material_index : u32,
    flags : u32,
};

// RtInstance flags (INSTANCE_* in raytracing/scene.rs).
const RT_INSTANCE_PARTICLES : u32 = 1u;
const RT_INSTANCE_ADDITIVE : u32 = 2u;

// TLAS instance masks (MASK_* in raytracing/scene.rs): what stops reflection and
// refraction rays (opaque surfaces with or without shadows, transmissive surfaces, opaque
// particles), the opaque surfaces alone, and the blended particles reflection rays gather
// on the way. Shadow and ambient occlusion rays use 0x01 (MASK_OPAQUE): surfaces that
// cast no shadow (0x10) and glass let light through.
const RT_MASK_SURFACES : u32 = 0x17u;
const RT_MASK_OPAQUE_SURFACES : u32 = 0x11u;
const RT_MASK_BLENDED_PARTICLES : u32 = 0x08u;

@group(3) @binding(0)
var rt_tlas : acceleration_structure;
@group(3) @binding(1)
var<storage, read> rt_vertices : array<RtVertex>;
@group(3) @binding(2)
var<storage, read> rt_indices : array<u32>;
@group(3) @binding(3)
var<storage, read> rt_instances : array<RtInstance>;
@group(3) @binding(4)
var<storage, read> rt_materials : array<MaterialData>;
@group(3) @binding(5)
var<storage, read> rt_particle_materials : array<MaterialData>;
