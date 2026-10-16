// src/shader/light_probes.wgsl
// The light probe grid (renderer/light_probes.rs): every probe keeps, in a tile of each
// of two atlases, the light arriving from every direction (irradiance, as the cosine-
// weighted mean radiance, so a diffuse surface sends back its colour times it) and the
// mean and mean square distance to what it sees, both over an octahedral map of the
// sphere with a one-texel border. A point takes the light of the eight probes around it,
// weighted by distance, by facing and by whether the probe can see it (Chebyshev's
// inequality on the distances), after Majercik et al., "Dynamic Diffuse Global
// Illumination with Ray-Traced Irradiance Fields" (JCGT 2019). Probes may sit off their
// grid positions, moved out of geometry (relocation): below its tiles the irradiance
// atlas keeps one texel per probe with that offset.
//
// Needs environment.wgsl (its sampler).

struct LightProbeGrid {
    // xyz: the first probe's position, w: the spacing (m).
    origin_spacing : vec4<f32>,
    // xyz: probes along each axis, w: 1 while the grid has light to give.
    counts : vec4<u32>,
    // x: probe tiles per atlas row, y: irradiance tile side, z: distance tile side (texels,
    // with the border), w: rays per probe.
    atlas : vec4<u32>,
    // x: intensity, y: normal bias, z: view bias (in spacings), w: 1 while probes may be
    // moved out of geometry.
    params : vec4<f32>,
};

@group(2) @binding(12) var<uniform> light_probe_grid : LightProbeGrid;
// rgb: irradiance; a: 1 for a probe in the open, 0 for one inside geometry. Below the
// tiles, one texel per probe: rgb its offset from the grid position (m).
@group(2) @binding(13) var light_probe_irradiance : texture_2d<f32>;
// r: mean distance, g: mean squared distance (m).
@group(2) @binding(14) var light_probe_distance : texture_2d<f32>;

fn light_probe_sign(v : vec2<f32>) -> vec2<f32> {
    return select(vec2<f32>(-1.0), vec2<f32>(1.0), v >= vec2<f32>(0.0));
}

// A direction onto the octahedral square [-1, 1]^2 (+z in the middle).
fn light_probe_oct_encode(d : vec3<f32>) -> vec2<f32> {
    let p = d.xy / max(abs(d.x) + abs(d.y) + abs(d.z), 1e-8);
    if d.z < 0.0 {
        return (vec2<f32>(1.0) - abs(p.yx)) * light_probe_sign(p);
    }
    return p;
}

fn light_probe_oct_decode(p : vec2<f32>) -> vec3<f32> {
    var d = vec3<f32>(p, 1.0 - abs(p.x) - abs(p.y));
    if d.z < 0.0 {
        let xy = (vec2<f32>(1.0) - abs(d.yx)) * light_probe_sign(d.xy);
        d = vec3<f32>(xy, d.z);
    }
    return normalize(d);
}

fn light_probe_index(coords : vec3<i32>) -> u32 {
    let counts = vec3<i32>(light_probe_grid.counts.xyz);
    return u32(coords.x + counts.x * (coords.y + counts.y * coords.z));
}

fn light_probe_count() -> u32 {
    let counts = light_probe_grid.counts.xyz;
    return counts.x * counts.y * counts.z;
}

// Rows of probe tiles in an atlas.
fn light_probe_tile_rows() -> u32 {
    let columns = max(light_probe_grid.atlas.x, 1u);
    return (light_probe_count() + columns - 1u) / columns;
}

// The irradiance atlas texel holding `probe`'s offset: rows below the tiles, as wide.
fn light_probe_offset_texel(probe : u32) -> vec2<i32> {
    let width = max(light_probe_grid.atlas.x, 1u) * light_probe_grid.atlas.y;
    let top = light_probe_tile_rows() * light_probe_grid.atlas.y;
    return vec2<i32>(i32(probe % width), i32(top + probe / width));
}

// Where probe `coords` sits in its grid (before relocation).
fn light_probe_grid_position(coords : vec3<i32>) -> vec3<f32> {
    return light_probe_grid.origin_spacing.xyz
        + vec3<f32>(coords) * light_probe_grid.origin_spacing.w;
}

// Where probe `coords` is: its grid position, moved out of geometry.
fn light_probe_position(coords : vec3<i32>) -> vec3<f32> {
    let offset = textureLoad(
        light_probe_irradiance, light_probe_offset_texel(light_probe_index(coords)), 0
    ).xyz;
    return light_probe_grid_position(coords) + offset;
}

// Where direction `d` of probe `probe` lies in an atlas of `tile`-texel tiles, as uv.
fn light_probe_atlas_uv(probe : u32, d : vec3<f32>, tile : u32, dims : vec2<f32>) -> vec2<f32> {
    let columns = max(light_probe_grid.atlas.x, 1u);
    let corner = vec2<f32>(f32(probe % columns), f32(probe / columns)) * f32(tile) + 1.0;
    let oct = light_probe_oct_encode(d) * 0.5 + 0.5;
    return (corner + oct * f32(tile - 2u)) / dims;
}

// What the probes give a point, from the eight around it.
struct LightProbeLight {
    // Irradiance for the surface's normal.
    diffuse : vec3<f32>,
    // Irradiance toward a second direction (the reflection, for rough specular light).
    other : vec3<f32>,
    // How much of it applies: 1 inside the grid, fading to 0 within half a spacing
    // outside it, 0 without a grid.
    coverage : f32,
};

// The probes' light at `position` for a surface facing `n`, seen from `view` (the unit
// direction from the surface to the eye), and toward `other`. Visibility is judged from
// the surface's point and normal for both.
fn light_probe_light_at(
    position : vec3<f32>,
    n : vec3<f32>,
    view : vec3<f32>,
    other : vec3<f32>,
) -> LightProbeLight {
    var out : LightProbeLight;
    out.diffuse = vec3<f32>(0.0);
    out.other = vec3<f32>(0.0);
    out.coverage = 0.0;
    if light_probe_grid.counts.w == 0u {
        return out;
    }
    let spacing = light_probe_grid.origin_spacing.w;
    let counts = vec3<i32>(light_probe_grid.counts.xyz);
    let last = vec3<f32>(counts - vec3<i32>(1));
    // Off the surface, so probes just behind it (and the surface's own self-shadowing in
    // the distances) do not darken it.
    let biased = position
        + (n * light_probe_grid.params.y + view * light_probe_grid.params.z) * spacing;
    let grid = (biased - light_probe_grid.origin_spacing.xyz) / spacing;
    let outside = max(max(-grid, grid - last), vec3<f32>(0.0));
    out.coverage = clamp(1.0 - 2.0 * max(outside.x, max(outside.y, outside.z)), 0.0, 1.0);
    if out.coverage <= 0.0 {
        return out;
    }
    let clamped = clamp(grid, vec3<f32>(0.0), last);
    let base = clamp(vec3<i32>(floor(clamped)), vec3<i32>(0), max(counts - vec3<i32>(2), vec3<i32>(0)));
    let alpha = clamp(clamped - vec3<f32>(base), vec3<f32>(0.0), vec3<f32>(1.0));
    let irradiance_dims = vec2<f32>(textureDimensions(light_probe_irradiance));
    let distance_dims = vec2<f32>(textureDimensions(light_probe_distance));

    var sum = vec3<f32>(0.0);
    var sum_other = vec3<f32>(0.0);
    var total = 0.0;
    for (var corner = 0u; corner < 8u; corner = corner + 1u) {
        let step = vec3<u32>(corner, corner >> 1u, corner >> 2u) & vec3<u32>(1u);
        let coords = min(base + vec3<i32>(step), counts - vec3<i32>(1));
        let probe = light_probe_index(coords);
        let probe_position = light_probe_position(coords);

        let light = textureSampleLevel(
            light_probe_irradiance, environment_sampler,
            light_probe_atlas_uv(probe, n, light_probe_grid.atlas.y, irradiance_dims), 0.0
        );
        // Probes inside walls see only backfaces: they have nothing to give.
        if light.a < 0.5 {
            continue;
        }

        let trilinear_axes = mix(vec3<f32>(1.0) - alpha, alpha, vec3<f32>(step));
        let trilinear = trilinear_axes.x * trilinear_axes.y * trilinear_axes.z;

        // Probes behind the surface count less (smoothly, so the light has no seam).
        let to_probe = probe_position - position;
        let to_probe_length = length(to_probe);
        let probe_dir = select(n, to_probe / to_probe_length, to_probe_length > 1e-4);
        let facing = (dot(probe_dir, n) + 1.0) * 0.5;
        var weight = facing * facing + 0.2;

        // Probes that cannot see the point (a wall between them) hardly count.
        let from_probe = biased - probe_position;
        let distance = length(from_probe);
        let from_dir = select(-n, from_probe / distance, distance > 1e-4);
        let moments = textureSampleLevel(
            light_probe_distance, environment_sampler,
            light_probe_atlas_uv(probe, from_dir, light_probe_grid.atlas.z, distance_dims), 0.0
        ).rg;
        if distance > moments.x {
            let variance = abs(moments.y - moments.x * moments.x);
            let excess = distance - moments.x;
            let chebyshev = variance / (variance + excess * excess);
            weight *= max(chebyshev * chebyshev * chebyshev, 0.0);
        }
        weight = max(weight, 1e-6);
        // Very small weights fade out faster, against light leaking at a low weight.
        let crush = 0.2;
        if weight < crush {
            weight *= weight * weight / (crush * crush);
        }
        weight *= trilinear;
        let toward_other = textureSampleLevel(
            light_probe_irradiance, environment_sampler,
            light_probe_atlas_uv(probe, other, light_probe_grid.atlas.y, irradiance_dims), 0.0
        ).rgb;
        sum += light.rgb * weight;
        sum_other += toward_other * weight;
        total += weight;
    }
    if total > 1e-5 {
        let scale = light_probe_grid.params.x / total;
        out.diffuse = sum * scale;
        out.other = sum_other * scale;
    }
    return out;
}

// The probes' indirect diffuse light at `position` for a surface facing `n`, seen from
// `view`: rgb (multiply by the diffuse colour), and in w how much of it applies.
fn light_probe_irradiance_at(position : vec3<f32>, n : vec3<f32>, view : vec3<f32>) -> vec4<f32> {
    let light = light_probe_light_at(position, n, view, n);
    return vec4<f32>(light.diffuse, light.coverage);
}

// The environment light calculate_environment_lighting gives, for a surface inside the
// grid, from the probes instead of the sky: diffuse from the irradiance at the normal,
// specular from the irradiance toward the reflection (a stand-in for a rough reflection
// of the surroundings: walls hide the sky; ray-traced or screen-space reflections bring
// sharp ones), with the same strength the sky's reflection gets. The flat ambient metals
// get stays (the reflection passes take it off where they find a reflection). w: how much
// of it applies.
fn light_probe_environment_lighting(
    position : vec3<f32>,
    N : vec3<f32>,
    V : vec3<f32>,
    base_color : vec3<f32>,
    metallic : f32,
    roughness : f32,
    occlusion : f32,
) -> vec4<f32> {
    let light = light_probe_light_at(position, N, V, reflect(-V, N));
    if light.coverage <= 0.0 {
        return vec4<f32>(0.0);
    }
    let hdr_enabled = environment_hdr_enabled();
    let ambient_tint = select(vec3<f32>(1.0), environment_settings.ambient_color.rgb, hdr_enabled);
    let metal_ambient = ambient_tint * environment_ambient_intensity() * base_color * metallic;
    let diffuse = light.diffuse * base_color * (1.0 - metallic);
    let specular_color = mix(vec3<f32>(0.04), base_color, vec3<f32>(metallic));
    let specular_strength = max(pow(clamp(1.0 - roughness, 0.0, 1.0), 4.0), 0.05);
    let specular = light.other * specular_color * specular_strength;
    return vec4<f32>((metal_ambient + diffuse + specular) * occlusion, light.coverage);
}
