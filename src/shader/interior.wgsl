// src/shader/interior.wgsl
//
// Interior mapping: rooms behind a window, traced in the shader instead of modelled (van
// Dongen 2008, "Interior Mapping"). Materials with FLAG_INTERIOR use it. The main pass
// (common.wgsl) and the ray-traced passes (rt_material in rt_common.wgsl) trace the same
// rooms, so a reflection of a window shows what the window shows.
//
// Each unit of the surface's uvs is one room: the integer part of the uv picks the room
// (its image, mirroring, lights and curtains, hashed with the object's position) and the
// fraction is where on the window the view ray enters. The room is a box behind the
// window, as wide and high as a uv unit is on the surface and `depth` room widths deep.
//
// A room image is the room seen square-on through its window from outside, in one-point
// perspective, with the back wall filling the middle half of the picture (a camera a
// room depth in front of the window, its view just spanning the window). Rooms deeper or
// shallower than drawn stretch along their depth. The images sit in a grid of `columns`
// x `rows` in the material's emissive texture, read row by row from the top.
//
// Room space: x across the window along u and y down it along v (-1 to 1 each), z into
// the room from the window (0) to the back wall, in half room widths.

// The share of the daylight on a window that lights the room behind it.
const INTERIOR_DAYLIGHT: f32 = 0.35;
// How far behind the glass curtains and blinds hang, in half room widths.
const INTERIOR_CURTAIN_PLANE: f32 = 0.1;

fn interior_columns(params: u32) -> u32 {
    return ((params >> 4u) & 15u) + 1u;
}

fn interior_rows(params: u32) -> u32 {
    return (params & 15u) + 1u;
}

// The back wall's z: the stored depth is in room widths times 64.
fn interior_back(params: u32) -> f32 {
    return f32(max((params >> 8u) & 255u, 1u)) / 32.0;
}

fn interior_lit_share(params: u32) -> f32 {
    return f32((params >> 16u) & 255u) / 255.0;
}

fn interior_curtain_share(params: u32) -> f32 {
    return f32(params >> 24u) / 255.0;
}

// PCG hash (Jarzynski and Olano 2020, "Hash Functions for GPU Rendering").
fn interior_hash(value: u32) -> u32 {
    let state = value * 747796405u + 2891336453u;
    let word = ((state >> ((state >> 28u) + 4u)) ^ state) * 277803737u;
    return (word >> 22u) ^ word;
}

// A number in [0, 1) for the room `seed` and the choice `salt`.
fn interior_random(seed: u32, salt: u32) -> f32 {
    return f32(interior_hash(seed ^ (salt * 0x9e3779b9u)) >> 8u) / 16777216.0;
}

// A seed for an object's rooms from where the object stands, so copies of a building
// differ. The main pass and the ray-traced passes see the same model matrix.
fn interior_object_seed(position: vec3<f32>) -> u32 {
    let cell = vec3<i32>(floor(position * 4.0));
    return interior_hash(
        bitcast<u32>(cell.x) ^ interior_hash(bitcast<u32>(cell.y) ^ interior_hash(bitcast<u32>(cell.z)))
    );
}

// How far the surface runs in the world per unit of u and of v.
struct InteriorFrame {
    dpdu: vec3<f32>,
    dpdv: vec3<f32>,
};

// The uv frame from two position differences and their uv differences: two screen
// derivatives in the main pass, two triangle edges for a ray hit.
fn interior_uv_frame(dp1: vec3<f32>, dp2: vec3<f32>, duv1: vec2<f32>, duv2: vec2<f32>) -> InteriorFrame {
    let det = duv1.x * duv2.y - duv2.x * duv1.y;
    let inv = select(0.0, 1.0 / det, abs(det) > 1e-30);
    return InteriorFrame((dp1 * duv2.y - dp2 * duv1.y) * inv, (dp2 * duv1.x - dp1 * duv2.x) * inv);
}

struct InteriorHit {
    // Where the view ray ends, in room space.
    position: vec3<f32>,
    // The back wall's z.
    back: f32,
    // What it ends on: 0 the back wall, 1 a side wall, 2 the floor, 3 the ceiling.
    face: u32,
    // Where that is in the atlas, and how much smaller a room image is in the atlas than
    // the window is in uv (to scale the window's uv footprint for the texture level).
    atlas_uv: vec2<f32>,
    atlas_scale: vec2<f32>,
    // The room's lamplight: colour times brightness, around 1; 0 when it is dark.
    lamp: vec3<f32>,
    // A curtain or blind in front of the room: its colour, and in w how much it hides.
    curtain: vec4<f32>,
    seed: u32,
};

// The room behind `uv` on an interior-mapped surface, seen along `ray` (the view ray's
// direction in the world, any length). `n` is the surface normal on the viewer's side;
// `params` is MaterialData.interior.
fn interior_trace(
    uv: vec2<f32>,
    ray: vec3<f32>,
    frame: InteriorFrame,
    n: vec3<f32>,
    params: u32,
    object_seed: u32,
) -> InteriorHit {
    let cell = floor(uv);
    var local = uv - cell;
    let id = vec2<i32>(cell);
    let seed = interior_hash(
        object_seed ^ interior_hash(bitcast<u32>(id.x) ^ interior_hash(bitcast<u32>(id.y) + 0x632be5abu))
    );

    // The ray in room space: solve ray = x * axis_x + y * axis_y + z * axis_z. Only its
    // direction matters, so the inverse's determinant contributes just its sign.
    let axis_x = frame.dpdu * 0.5;
    let axis_y = frame.dpdv * 0.5;
    let axis_z = -n * (length(frame.dpdu) * 0.5);
    let row_x = cross(axis_y, axis_z);
    let row_y = cross(axis_z, axis_x);
    let row_z = cross(axis_x, axis_y);
    var dir = vec3<f32>(dot(ray, row_x), dot(ray, row_y), dot(ray, row_z)) * sign(dot(axis_x, row_x));
    // Seen edge-on (or from behind) the ray still goes in.
    dir.z = max(dir.z, 1e-4 * max(abs(dir.x), abs(dir.y)) + 1e-20);

    // Half the rooms are drawn mirrored.
    if interior_random(seed, 1u) < 0.5 {
        local.x = 1.0 - local.x;
        dir.x = -dir.x;
    }

    // Where the ray leaves the box: the nearest of the walls it heads for.
    let back = interior_back(params);
    let start = vec3<f32>(local * 2.0 - 1.0, 0.0);
    let to_x = select(1e30, (select(-1.0, 1.0, dir.x > 0.0) - start.x) / dir.x, dir.x != 0.0);
    let to_y = select(1e30, (select(-1.0, 1.0, dir.y > 0.0) - start.y) / dir.y, dir.y != 0.0);
    let to_z = back / dir.z;
    let t = min(to_z, min(to_x, to_y));
    var face = 0u;
    if t == to_x && t < to_z {
        face = 1u;
    } else if t == to_y && t < to_z {
        // y runs down the window: the floor at +1.
        face = select(3u, 2u, dir.y > 0.0);
    }
    let position = clamp(start + dir * t, vec3<f32>(-1.0, -1.0, 0.0), vec3<f32>(1.0, 1.0, back));

    // The room image's point: the back wall at half size (see the top).
    let image_uv = clamp(
        position.xy / (1.0 + position.z / back) * 0.5 + 0.5,
        vec2<f32>(0.002),
        vec2<f32>(0.998),
    );
    let columns = interior_columns(params);
    let grid = vec2<f32>(f32(columns), f32(interior_rows(params)));
    let room = interior_hash(seed ^ 0x51ed27u) % (columns * interior_rows(params));
    let atlas_cell = vec2<f32>(f32(room % columns), f32(room / columns));

    var hit: InteriorHit;
    hit.position = position;
    hit.back = back;
    hit.face = face;
    hit.atlas_uv = (atlas_cell + image_uv) / grid;
    // A room image spans its window from half size (the back wall) to full size (the
    // walls by the glass).
    hit.atlas_scale = 0.6 / grid;
    hit.seed = seed;

    // Lamps from candlelight to a whiter flame, some brighter than others.
    let lit = interior_random(seed, 2u) < interior_lit_share(params);
    let tint = mix(vec3<f32>(1.0, 0.74, 0.5), vec3<f32>(1.0, 0.95, 0.86), interior_random(seed, 3u));
    let brightness = 0.6 + 0.8 * interior_random(seed, 4u);
    hit.lamp = select(
        vec3<f32>(0.0),
        tint * (brightness / dot(tint, vec3<f32>(0.2126, 0.7152, 0.0722))),
        lit,
    );

    hit.curtain = vec4<f32>(0.0);
    if interior_random(seed, 5u) < interior_curtain_share(params) {
        let to_curtain = INTERIOR_CURTAIN_PLANE / dir.z;
        if to_curtain < t {
            hit.curtain = interior_curtain((start + dir * to_curtain).xy, seed);
        }
    }
    return hit;
}

// A room's curtain or blind at `p` (room space x and y): its colour, and in w how much
// of the room it hides there.
fn interior_curtain(p: vec2<f32>, seed: u32) -> vec4<f32> {
    var fabrics = array<vec3<f32>, 6>(
        vec3<f32>(0.78, 0.72, 0.58), // linen
        vec3<f32>(0.45, 0.08, 0.07), // madder red
        vec3<f32>(0.62, 0.42, 0.12), // ochre
        vec3<f32>(0.22, 0.32, 0.20), // sage
        vec3<f32>(0.20, 0.26, 0.42), // woad blue
        vec3<f32>(0.66, 0.58, 0.46), // undyed wool
    );
    let fabric = fabrics[interior_hash(seed ^ 0xc0ffeeu) % 6u];
    if interior_random(seed, 6u) < 0.65 {
        // Two drapes drawn to the sides, each over `cover` of its half of the window,
        // gathered a little toward the sill; the most closed meet in the middle.
        let cover = mix(0.15, 1.05, interior_random(seed, 7u));
        let width = cover * mix(1.0, 0.75, p.y * 0.5 + 0.5);
        let covered = smoothstep(-0.03, 0.03, abs(p.x) - (1.0 - width));
        let phase = interior_random(seed, 9u) * 6.2832;
        let folds = 0.68 + 0.32 * cos(abs(p.x) * 15.0 / max(cover, 0.3) + phase);
        return vec4<f32>(fabric * folds, covered);
    }
    // A blind let down from the top to `drop`, with a darker hem.
    let drop = mix(-0.6, 0.95, interior_random(seed, 7u));
    let covered = 1.0 - smoothstep(drop - 0.02, drop + 0.02, p.y);
    let weave = 0.92 + 0.08 * sin(p.y * 40.0);
    let hem = select(1.0, 0.6, p.y > drop - 0.07);
    return vec4<f32>(fabric * weave * hem, covered);
}

// A plain room, for materials without room images: plaster walls in a colour of the
// room's own, a plank floor and a beamed ceiling, lit by a lamp hanging in the middle.
fn interior_plain_room(hit: InteriorHit) -> vec3<f32> {
    let p = hit.position;
    let wall = mix(vec3<f32>(0.74, 0.66, 0.52), vec3<f32>(0.60, 0.50, 0.44), interior_random(hit.seed, 8u));
    let wood = vec3<f32>(0.30, 0.19, 0.10);
    var colour: vec3<f32>;
    switch hit.face {
        case 0u: {
            colour = wall;
        }
        case 1u: {
            colour = wall * 0.85;
        }
        case 2u: {
            colour = wood * select(1.0, 0.7, fract(p.x * 2.5) < 0.06);
        }
        default: {
            colour = select(wall, wood * 0.8, fract(p.z * 1.2) < 0.18);
        }
    }
    let to_lamp = p - vec3<f32>(0.0, -0.5, hit.back * 0.5);
    return colour * (1.6 / (1.0 + 0.5 * dot(to_lamp, to_lamp)));
}

// The light leaving the window from its room. `room` is the room's colour where the ray
// ends (from its image, or interior_plain_room), `daylight` the light the window lets
// in (scaled by INTERIOR_DAYLIGHT), `strength` the lamps' brightness (the material's
// emissive strength).
fn interior_radiance(hit: InteriorHit, room: vec3<f32>, daylight: vec3<f32>, strength: f32) -> vec3<f32> {
    let lamp = hit.lamp * strength;
    let inside = room * (lamp + daylight);
    // Cloth lets some of the lamplight through and catches the daylight.
    let cloth = hit.curtain.rgb * (lamp * 0.3 + daylight);
    return mix(inside, cloth, hit.curtain.w);
}
