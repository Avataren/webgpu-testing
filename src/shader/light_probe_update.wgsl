// src/shader/light_probe_update.wgsl
// Updating the light probe grid (light_probes.wgsl) by ray tracing, in three fullscreen
// passes per update:
// 1. fs_probe_trace: one texel per ray, one row per probe updated: the light arriving
//    along the ray (direct light at the hit, plus the probes' own light there for the
//    further bounces) and the distance to the hit (negative for the back of a surface);
// 2. fs_probe_irradiance: every irradiance atlas texel, the cosine-weighted mean of its
//    probe's rays, blended into its previous value;
// 3. fs_probe_distance: the same for the distances (and their squares), with a sharper
//    lobe.
// Border texels repeat the interior texel across the octahedral seam, so bilinear
// filtering wraps around the sphere. Trace needs the ray tracing scene modules.

struct LightProbeUpdate {
    // A random rotation for this update's ray directions.
    rotation : mat4x4<f32>,
    // x: the first probe updated, y: probes updated, z: flags (PROBE_FLAG_*), w: rays per
    // probe.
    frame : vec4<u32>,
    // x: hysteresis, y: the longest distance kept (m), z: the sky's blur level (0-1 of the
    // environment's mip levels), w: the share of backface hits marking a probe as inside
    // geometry.
    params : vec4<f32>,
};

// No previous value: take this update's.
const PROBE_FLAG_RESET : u32 = 1u;
const PROBE_MAX_RAY_DISTANCE : f32 = 1.0e3;
// Through at most this many transmissive surfaces before a ray counts as blocked.
const PROBE_MAX_TRANSMISSIONS : u32 = 3u;

@group(0) @binding(0) var<uniform> probe_update : LightProbeUpdate;
// rgb: radiance, a: distance (negative at a backface); one row per probe updated.
@group(0) @binding(1) var probe_rays : texture_2d<f32>;

// Ray `index` of `count`, spread evenly over the sphere (a spherical Fibonacci lattice)
// and turned by this update's rotation.
fn probe_ray_direction(index : u32, count : u32) -> vec3<f32> {
    let golden_angle = 2.399963229728653;
    let z = 1.0 - (2.0 * f32(index) + 1.0) / f32(count);
    let r = sqrt(max(1.0 - z * z, 0.0));
    let phi = f32(index) * golden_angle;
    let d = vec3<f32>(cos(phi) * r, sin(phi) * r, z);
    return normalize((probe_update.rotation * vec4<f32>(d, 0.0)).xyz);
}

fn probe_count_total() -> u32 {
    let counts = light_probe_grid.counts.xyz;
    return counts.x * counts.y * counts.z;
}

fn probe_coords(probe : u32) -> vec3<i32> {
    let counts = light_probe_grid.counts.xyz;
    let x = probe % counts.x;
    let y = (probe / counts.x) % counts.y;
    let z = probe / (counts.x * counts.y);
    return vec3<i32>(vec3<u32>(x, y, z));
}

// Which row of the ray texture holds `probe` this update, if any (-1).
fn probe_update_row(probe : u32) -> i32 {
    let total = probe_count_total();
    let row = (probe + total - probe_update.frame.x % total) % total;
    return select(-1, i32(row), row < probe_update.frame.y);
}

// What the environment gives a ray leaving the scene: the HDR sky (somewhat blurred, as
// a few hundred rays sample it), plus the flat ambient light as a uniform sky.
fn probe_sky(direction : vec3<f32>) -> vec3<f32> {
    let hdr = environment_hdr_enabled();
    let tint = select(vec3<f32>(1.0), environment_settings.ambient_color.rgb, hdr);
    var sky = tint * environment_ambient_intensity();
    if hdr {
        let lod = environment_settings.flags_intensity.w * probe_update.params.z;
        sky += sample_environment_hdr(direction, lod) * environment_hdr_intensity();
    }
    return sky;
}

// The diffuse light the environment gives a surface outside the grid.
fn probe_environment_diffuse(n : vec3<f32>) -> vec3<f32> {
    let hdr = environment_hdr_enabled();
    let tint = select(vec3<f32>(1.0), environment_settings.ambient_color.rgb, hdr);
    var light = tint * environment_ambient_intensity();
    if hdr {
        let lod = environment_settings.flags_intensity.w;
        light += sample_environment_hdr(n, lod) * environment_hdr_intensity();
    }
    return light;
}

@fragment
fn fs_probe_trace(in : VertexOutput) -> @location(0) vec4<f32> {
    let texel = vec2<u32>(in.position.xy);
    let rays = probe_update.frame.w;
    if texel.x >= rays || texel.y >= probe_update.frame.y {
        return vec4<f32>(0.0);
    }
    let probe = (probe_update.frame.x + texel.y) % probe_count_total();
    let start = light_probe_position(probe_coords(probe));
    let direction = probe_ray_direction(texel.x, rays);
    let cone_spread = sqrt(4.0 * PI / f32(rays));

    var origin = start;
    var travelled = 0.0;
    var throughput = vec3<f32>(1.0);
    for (var event = 0u; event <= PROBE_MAX_TRANSMISSIONS; event = event + 1u) {
        let hit = rt_closest_hit(origin, direction, PROBE_MAX_RAY_DISTANCE);
        if hit.kind == RAY_QUERY_INTERSECTION_NONE {
            return vec4<f32>(throughput * probe_sky(direction), PROBE_MAX_RAY_DISTANCE);
        }
        travelled += hit.t;
        let position = origin + direction * hit.t;
        let material = rt_material(hit, direction, cone_spread * travelled);
        if material.back_facing {
            // Inside a closed mesh or behind a wall: no light, and a mark for the
            // probe's classification (shortened, as DDGI does, so it barely blocks).
            return vec4<f32>(0.0, 0.0, 0.0, -0.2 * travelled);
        }
        if material.transmission > 0.5 && event < PROBE_MAX_TRANSMISSIONS {
            // Glass passes the light on, tinted.
            throughput *= material.base_color * material.transmission;
            origin = position + direction * RT_SHADOW_BIAS;
            continue;
        }
        let n = material.normal;
        let v = -direction;
        var radiance = material.emissive;
        if material.unlit {
            radiance += material.base_color;
        } else {
            let diffuse_color = material.base_color * (1.0 - material.metallic);
            // Direct light as a diffuse surface (the probe stores no view direction).
            radiance += rt_direct_lighting(position, n, v, diffuse_color, 0.0, 1.0);
            // Further bounces: the grid's own light there, last update's; past the
            // grid's edge, the environment's.
            let probes = light_probe_irradiance_at(position, n, v);
            let indirect = mix(probe_environment_diffuse(n), probes.rgb, probes.w);
            radiance += diffuse_color * indirect;
        }
        return vec4<f32>(throughput * radiance, travelled);
    }
    return vec4<f32>(0.0, 0.0, 0.0, travelled);
}

// The interior texel of a tile that texel `local` of a `tile`-texel tile shows: itself,
// or for a border texel the one across the octahedral seam.
fn probe_interior_texel(local : vec2<i32>, tile : i32) -> vec2<i32> {
    let last = tile - 1;
    var x = local.x;
    var y = local.y;
    let border_x = x == 0 || x == last;
    let border_y = y == 0 || y == last;
    if border_x && border_y {
        return vec2<i32>(select(1, tile - 2, x == 0), select(1, tile - 2, y == 0));
    }
    if border_y {
        return vec2<i32>(last - x, select(tile - 2, 1, y == 0));
    }
    if border_x {
        return vec2<i32>(select(tile - 2, 1, x == 0), last - y);
    }
    return local;
}

struct ProbeTexel {
    probe : u32,
    // The direction the texel stands for.
    direction : vec3<f32>,
    // The ray texture row with this update's rays (-1: not updated now).
    row : i32,
    valid : bool,
};

fn probe_texel(pixel : vec2<i32>, tile_u : u32) -> ProbeTexel {
    var out : ProbeTexel;
    let tile = i32(tile_u);
    let columns = i32(max(light_probe_grid.atlas.x, 1u));
    let tile_coords = pixel / vec2<i32>(tile);
    let probe = tile_coords.y * columns + tile_coords.x;
    out.valid = tile_coords.x < columns && probe < i32(probe_count_total());
    if !out.valid {
        return out;
    }
    out.probe = u32(probe);
    let interior = probe_interior_texel(pixel - tile_coords * tile, tile) - vec2<i32>(1);
    let oct = (vec2<f32>(interior) + 0.5) / f32(tile - 2) * 2.0 - 1.0;
    out.direction = light_probe_oct_decode(oct);
    out.row = probe_update_row(out.probe);
    return out;
}

@fragment
fn fs_probe_irradiance(in : VertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let texel = probe_texel(pixel, light_probe_grid.atlas.y);
    if !texel.valid {
        return vec4<f32>(0.0);
    }
    let previous = textureLoad(light_probe_irradiance, pixel, 0);
    if texel.row < 0 {
        return previous;
    }
    let rays = probe_update.frame.w;
    var sum = vec3<f32>(0.0);
    var total = 0.0;
    var backfaces = 0u;
    for (var ray = 0u; ray < rays; ray = ray + 1u) {
        let sample = textureLoad(probe_rays, vec2<i32>(i32(ray), texel.row), 0);
        if sample.a < 0.0 {
            backfaces += 1u;
        }
        let weight = max(dot(texel.direction, probe_ray_direction(ray, rays)), 0.0);
        sum += sample.rgb * weight;
        total += weight;
    }
    let light = sum / max(total, 1e-4);
    // A probe seeing mostly backfaces is inside geometry: it gives nothing.
    let open = select(1.0, 0.0, f32(backfaces) > probe_update.params.w * f32(rays));
    if (probe_update.frame.z & PROBE_FLAG_RESET) != 0u || previous.a != open {
        return vec4<f32>(light, open);
    }
    return vec4<f32>(mix(light, previous.rgb, probe_update.params.x), open);
}

@fragment
fn fs_probe_distance(in : VertexOutput) -> @location(0) vec4<f32> {
    let pixel = vec2<i32>(in.position.xy);
    let texel = probe_texel(pixel, light_probe_grid.atlas.z);
    if !texel.valid {
        return vec4<f32>(0.0);
    }
    let previous = textureLoad(light_probe_distance, pixel, 0);
    if texel.row < 0 {
        return previous;
    }
    let rays = probe_update.frame.w;
    let max_distance = probe_update.params.y;
    var sum = vec2<f32>(0.0);
    var total = 0.0;
    for (var ray = 0u; ray < rays; ray = ray + 1u) {
        let sample = textureLoad(probe_rays, vec2<i32>(i32(ray), texel.row), 0);
        let cosine = max(dot(texel.direction, probe_ray_direction(ray, rays)), 0.0);
        // A sharp lobe: distances change quickly with direction near walls.
        let weight = pow(cosine, 50.0);
        let distance = min(abs(sample.a), max_distance);
        sum += vec2<f32>(distance, distance * distance) * weight;
        total += weight;
    }
    let moments = sum / max(total, 1e-4);
    if (probe_update.frame.z & PROBE_FLAG_RESET) != 0u {
        return vec4<f32>(moments, 0.0, 0.0);
    }
    return vec4<f32>(mix(moments, previous.rg, probe_update.params.x), 0.0, 0.0);
}
