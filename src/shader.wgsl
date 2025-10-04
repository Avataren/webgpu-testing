struct Globals {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> globals: Globals;

struct Object {
    model: mat4x4<f32>,
    color: vec4<f32>,
    texture_index: u32,
    material_flags: u32,
    _padding: vec2<u32>,
};
@group(1) @binding(0) var<storage, read> objects: array<Object>;

// Bindless texture array - note: this requires the SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING feature
@group(2) @binding(0) var textures: binding_array<texture_2d<f32>, 256>;
@group(2) @binding(1) var tex_sampler: sampler;

// Material flags
const FLAG_USE_TEXTURE: u32 = 1u;
const FLAG_ALPHA_BLEND: u32 = 2u;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @builtin(instance_index) instance: u32,
};

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
    @location(3) @interpolate(flat) texture_index: u32,
    @location(4) @interpolate(flat) material_flags: u32,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    let obj = objects[in.instance];
    let M = obj.model;
    let world_pos = M * vec4(in.pos, 1.0);
    let n = normalize((M * vec4(in.normal, 0.0)).xyz);

    var out: VsOut;
    out.pos = globals.view_proj * world_pos;
    out.normal = n;
    out.uv = in.uv;
    out.color = obj.color;
    out.texture_index = obj.texture_index;
    out.material_flags = obj.material_flags;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // Lighting
    let L = normalize(vec3<f32>(0.5, 0.3, 1.0));
    let N = normalize(in.normal);
    let ndotl = max(dot(N, L), 0.0);
    let ambient = 0.1;
    let diffuse = ndotl * 0.8;
    let lighting = ambient + diffuse;
    
    // Rim light
    let V = vec3<f32>(0.0, 0.0, 1.0);
    let rim = pow(1.0 - max(dot(N, -V), 0.0), 3.0) * 0.15;
    
    var base_color: vec3<f32>;
    
    // Check if we should use texture
    if ((in.material_flags & FLAG_USE_TEXTURE) != 0u) {
        // Sample from bindless texture array
        let tex_idx = in.texture_index;
        let tex_color = textureSample(textures[tex_idx], tex_sampler, in.uv);
        base_color = tex_color.rgb * in.color.rgb;
    } else {
        // Solid color or checker pattern
        if (in.color.a < 0.5) {
            // Checker pattern
            let tiles = 6.0;
            let c = in.uv * tiles;
            let parity = (i32(floor(c.x) + floor(c.y)) & 1) == 0;
            let colorA = vec3<f32>(0.08, 0.09, 0.11);
            let colorB = vec3<f32>(0.93, 0.93, 0.96);
            base_color = select(colorA, colorB, parity);
        } else {
            base_color = in.color.rgb;
        }
    }
    
    return vec4(base_color * lighting + rim, 1.0);
}