// src/shader/light_probe_debug.wgsl
// The light probe debug view (renderer/light_probe_debug.rs): a sphere at every probe of
// every volume that holds light (light_probes.wgsl), where it is after relocation, showing the light its
// irradiance tile holds for each direction. Probes inside geometry (which give nothing)
// are dark red; probes moved out of geometry have a yellow rim. Drawn over the finished
// image, depth tested against the scene. Needs camera.wgsl (group 0).

// As in light_probes.wgsl.
const DEBUG_MAX_VOLUMES : u32 = 8u;
const DEBUG_IRRADIANCE_TILE : u32 = 8u;
const DEBUG_VOLUME_LIT : u32 = 1u;

struct LightProbeVolume {
    origin_spacing : vec4<f32>,
    counts : vec4<u32>,
    atlas : vec4<u32>,
    params : vec4<f32>,
    axis_x : vec4<f32>,
    axis_y : vec4<f32>,
    axis_z : vec4<f32>,
};

struct LightProbeVolumes {
    header : vec4<u32>,
    volumes : array<LightProbeVolume, DEBUG_MAX_VOLUMES>,
};

struct LightProbeDebug {
    // x: sphere radius (m), y: exposure (a factor on the light), zw: unused.
    params : vec4<f32>,
};

@group(1) @binding(0) var<uniform> probe_volumes : LightProbeVolumes;
@group(1) @binding(1) var probe_irradiance : texture_2d<f32>;
@group(1) @binding(2) var<uniform> probe_debug : LightProbeDebug;
@group(1) @binding(3) var probe_sampler : sampler;

struct ProbeVertex {
    @builtin(position) position : vec4<f32>,
    @location(0) world : vec3<f32>,
    @location(1) @interpolate(flat) center : vec3<f32>,
    @location(2) @interpolate(flat) probe : u32,
    @location(4) @interpolate(flat) volume : u32,
    @location(3) @interpolate(flat) relocated : f32,
};

struct ProbeFragment {
    @location(0) color : vec4<f32>,
    @builtin(frag_depth) depth : f32,
};

fn debug_probe_count(v : u32) -> u32 {
    let counts = probe_volumes.volumes[v].counts.xyz;
    return counts.x * counts.y * counts.z;
}

fn debug_probe_offset(v : u32, probe : u32) -> vec3<f32> {
    let volume = probe_volumes.volumes[v];
    let columns = max(volume.atlas.x, 1u);
    let width = columns * DEBUG_IRRADIANCE_TILE;
    let top = volume.atlas.y + (debug_probe_count(v) + columns - 1u) / columns * DEBUG_IRRADIANCE_TILE;
    return textureLoad(
        probe_irradiance, vec2<i32>(i32(probe % width), i32(top + probe / width)), 0
    ).xyz;
}

fn debug_sign(v : vec2<f32>) -> vec2<f32> {
    return select(vec2<f32>(-1.0), vec2<f32>(1.0), v >= vec2<f32>(0.0));
}

fn debug_oct_encode(d : vec3<f32>) -> vec2<f32> {
    let p = d.xy / max(abs(d.x) + abs(d.y) + abs(d.z), 1e-8);
    if d.z < 0.0 {
        return (vec2<f32>(1.0) - abs(p.yx)) * debug_sign(p);
    }
    return p;
}

// The irradiance of `probe` of volume `v` toward direction `d` (filtered, as surfaces
// read it).
fn debug_probe_light(v : u32, probe : u32, d : vec3<f32>) -> vec4<f32> {
    let volume = probe_volumes.volumes[v];
    let columns = max(volume.atlas.x, 1u);
    let tile = DEBUG_IRRADIANCE_TILE;
    let corner = vec2<f32>(f32(probe % columns), f32(probe / columns)) * f32(tile)
        + vec2<f32>(1.0, 1.0 + f32(volume.atlas.y));
    let oct = debug_oct_encode(d) * 0.5 + 0.5;
    let dims = vec2<f32>(textureDimensions(probe_irradiance));
    let uv = (corner + oct * f32(tile - 2u)) / dims;
    return textureSampleLevel(probe_irradiance, probe_sampler, uv, 0.0);
}

@vertex
fn vs_probe(
    @builtin(vertex_index) vertex : u32,
    @builtin(instance_index) instance : u32,
) -> ProbeVertex {
    var out : ProbeVertex;
    // Instances run through the volumes' probes, one volume after another.
    var v = 0u;
    var probe = instance;
    let volume_count = min(probe_volumes.header.x, DEBUG_MAX_VOLUMES);
    loop {
        if v + 1u >= volume_count || probe < debug_probe_count(v) {
            break;
        }
        probe -= debug_probe_count(v);
        v += 1u;
    }
    let volume = probe_volumes.volumes[v];
    if v >= volume_count || probe >= debug_probe_count(v) || (volume.counts.w & DEBUG_VOLUME_LIT) == 0u {
        // Nothing to show: off screen.
        out.position = vec4<f32>(0.0, 0.0, -2.0, 1.0);
        return out;
    }
    let counts = volume.counts.xyz;
    let coords = vec3<f32>(vec3<u32>(probe % counts.x, (probe / counts.x) % counts.y, probe / (counts.x * counts.y)))
        * volume.origin_spacing.w;
    let offset = debug_probe_offset(v, probe);
    let center = volume.origin_spacing.xyz + volume.axis_x.xyz * coords.x
        + volume.axis_y.xyz * coords.y + volume.axis_z.xyz * coords.z + offset;
    let radius = probe_debug.params.x;

    // A camera-facing square wide enough for the sphere's silhouette in perspective.
    let to_camera = globals.camera_pos - center;
    let distance = max(length(to_camera), radius * 1.01);
    let facing = to_camera / distance;
    var up = globals.camera_up;
    if abs(dot(up, facing)) > 0.99 {
        up = vec3<f32>(0.0, 0.0, 1.0);
    }
    let right = normalize(cross(up, facing));
    let quad_up = cross(facing, right);
    let half_size = radius * distance / sqrt(distance * distance - radius * radius) * 1.05;
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let c = corners[vertex];
    // On the near side of the sphere, so the square is not clipped by what it covers.
    let world = center + facing * radius + (right * c.x + quad_up * c.y) * half_size;

    out.position = globals.view_proj * vec4<f32>(world, 1.0);
    out.world = world;
    out.center = center;
    out.probe = probe;
    out.volume = v;
    out.relocated = select(0.0, 1.0, length(offset) > 0.01 * volume.origin_spacing.w);
    return out;
}

@fragment
fn fs_probe(in : ProbeVertex) -> ProbeFragment {
    let radius = probe_debug.params.x;
    let origin = globals.camera_pos;
    let direction = normalize(in.world - origin);
    let oc = origin - in.center;
    let b = dot(oc, direction);
    let h = b * b - (dot(oc, oc) - radius * radius);
    if h < 0.0 {
        discard;
    }
    let t = -b - sqrt(h);
    if t <= 0.0 {
        discard;
    }
    let hit = origin + direction * t;
    let n = normalize(hit - in.center);

    let light = debug_probe_light(in.volume, in.probe, n);
    var color : vec3<f32>;
    if light.a < 0.5 {
        // Inside geometry: no light to give.
        color = vec3<f32>(0.35, 0.03, 0.03) * (0.5 + 0.5 * max(dot(n, -direction), 0.0));
    } else {
        let exposed = light.rgb * probe_debug.params.y;
        color = exposed / (vec3<f32>(1.0) + exposed);
    }
    if in.relocated > 0.5 {
        let rim = smoothstep(0.35, 0.15, max(dot(n, -direction), 0.0));
        color = mix(color, vec3<f32>(1.0, 0.75, 0.1), rim);
    }

    var out : ProbeFragment;
    out.color = vec4<f32>(color, 1.0);
    let clip = globals.view_proj * vec4<f32>(hit, 1.0);
    out.depth = clamp(clip.z / clip.w, 0.0, 1.0);
    return out;
}
