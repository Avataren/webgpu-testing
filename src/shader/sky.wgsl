// src/shader/sky.wgsl
//
// The procedural sky (src/sky/mod.rs). The air scatters the sun's and the moon's light:
// Rayleigh and Mie single scattering, with ozone absorbing, marched along the view ray
// (Nishita et al. 1993; O'Neil, "Accurate Atmospheric Scattering", GPU Gems 2 ch. 16,
// which fits the same integrals). The light's way down to each sample is read from a
// transmittance table (Bruneton and Neyret 2008's layout), and rays under the horizon end
// on the ground. sky_scattering is rendered into the environment map; the background
// adds what is too sharp for it per pixel: the sun's disc, the moon in its phase, the
// stars and the Milky Way. Light scattered more than once comes from a second table
// (Hillaire 2020, "A Scalable and Production Ready Sky and Atmosphere Rendering
// Technique"): without it twilight is grey and dim.
//
// Distances are in kilometres (the planet's radius squared stays exact enough in f32).

struct SkyParams {
    sun: vec4<f32>,          // xyz: toward the sun; w: 1 while the procedural sky is on
    sun_light: vec4<f32>,    // rgb: sunlight above the atmosphere; w: the sun's angular radius (rad)
    sun_seen: vec4<f32>,     // rgb: sunlight through the air to the observer
    moon: vec4<f32>,         // xyz: toward the moon; w: its angular radius (rad)
    moon_light: vec4<f32>,   // rgb: moonlight above the atmosphere at this phase; w: share of the disc lit
    moon_seen: vec4<f32>,    // rgb: a full moon's light through the air (the disc's phase is shaded)
    moon_north: vec4<f32>,   // xyz: the celestial north at the moon (the disc's up)
    scattering: vec4<f32>,   // x: air density, y: haze, z: ozone, w: Mie anisotropy g
    look: vec4<f32>,         // x: sky intensity, y: stars, z: Milky Way, w: night glow
    ground: vec4<f32>,       // rgb: ground albedo; w: seconds, for the stars' twinkling
    night_tint: vec4<f32>,   // rgb: the night's colour
    zenith_depth: vec4<f32>, // rgb: the air's optical depth straight up (dims what lies beyond it)
    celestial_x: vec4<f32>,  // rows of the turn from the world into the equatorial frame
    celestial_y: vec4<f32>,
    celestial_z: vec4<f32>,
};

const SKY_GROUND_RADIUS: f32 = 6360.0;
const SKY_TOP_RADIUS: f32 = 6460.0;
const SKY_OBSERVER_HEIGHT: f32 = 0.2;
const SKY_RAYLEIGH_SCATTERING: vec3<f32> = vec3<f32>(5.802e-3, 13.558e-3, 33.1e-3);
const SKY_RAYLEIGH_SCALE_HEIGHT: f32 = 8.0;
const SKY_MIE_SCATTERING: f32 = 3.996e-3;
const SKY_MIE_EXTINCTION: f32 = 4.40e-3;
const SKY_MIE_SCALE_HEIGHT: f32 = 1.2;
const SKY_OZONE_ABSORPTION: vec3<f32> = vec3<f32>(0.650e-3, 1.881e-3, 0.085e-3);
const SKY_OZONE_CENTER: f32 = 25.0;
const SKY_OZONE_HALF_WIDTH: f32 = 15.0;
const SKY_PI: f32 = 3.14159265359;
// The brightest the sun's disc gets (half floats end at 65504).
const SKY_SUN_DISC_MAX: f32 = 20000.0;
// Starlight of a magnitude 0 star (irradiance at stars = 1), the Milky Way's glow and the
// night's glow at their factors of 1.
const SKY_STAR_FLUX: f32 = 6e-6;
const SKY_MILKY_WAY: f32 = 5e-3;
const SKY_NIGHT_GLOW: f32 = 3e-4;

fn sky_enabled(sky: SkyParams) -> bool {
    return sky.sun.w > 0.5;
}

// ---------------------------------------------------------------------------- the air

// Rayleigh, Mie and ozone densities (as shares of their densest) at `height` km.
fn sky_densities(sky: SkyParams, height: f32) -> vec3<f32> {
    let rayleigh = exp(-max(height, 0.0) / SKY_RAYLEIGH_SCALE_HEIGHT) * sky.scattering.x;
    let mie = exp(-max(height, 0.0) / SKY_MIE_SCALE_HEIGHT) * sky.scattering.y;
    let ozone = max(1.0 - abs(height - SKY_OZONE_CENTER) / SKY_OZONE_HALF_WIDTH, 0.0) * sky.scattering.z;
    return vec3<f32>(rayleigh, mie, ozone);
}

fn sky_extinction(densities: vec3<f32>) -> vec3<f32> {
    return SKY_RAYLEIGH_SCATTERING * densities.x + vec3<f32>(SKY_MIE_EXTINCTION * densities.y)
        + SKY_OZONE_ABSORPTION * densities.z;
}

// Distances along a ray from `origin` (unit `direction`) to a sphere of `radius` round the
// planet's centre: (near, far), both negative when it misses.
fn sky_ray_sphere(origin: vec3<f32>, direction: vec3<f32>, radius: f32) -> vec2<f32> {
    let b = dot(origin, direction);
    let c = dot(origin, origin) - radius * radius;
    let discriminant = b * b - c;
    if discriminant < 0.0 {
        return vec2<f32>(-1.0);
    }
    let root = sqrt(discriminant);
    return vec2<f32>(-b - root, -b + root);
}

fn sky_rayleigh_phase(cos_theta: f32) -> f32 {
    return 3.0 / (16.0 * SKY_PI) * (1.0 + cos_theta * cos_theta);
}

// Cornette-Shanks: Henyey-Greenstein with a Rayleigh-like back lobe.
fn sky_mie_phase(cos_theta: f32, g: f32) -> f32 {
    let g2 = g * g;
    let denominator = max(1.0 + g2 - 2.0 * g * cos_theta, 1e-4);
    return 3.0 / (8.0 * SKY_PI) * (1.0 - g2) * (1.0 + cos_theta * cos_theta)
        / ((2.0 + g2) * denominator * sqrt(denominator));
}

// ---------------------------------------------------------------------------- the transmittance table

// Where (radius `r`, cosine of the zenith angle `mu`) lies in the transmittance table.
fn sky_transmittance_uv(r: f32, mu: f32) -> vec2<f32> {
    let h = sqrt(SKY_TOP_RADIUS * SKY_TOP_RADIUS - SKY_GROUND_RADIUS * SKY_GROUND_RADIUS);
    let rho = sqrt(max(r * r - SKY_GROUND_RADIUS * SKY_GROUND_RADIUS, 0.0));
    let discriminant = r * r * (mu * mu - 1.0) + SKY_TOP_RADIUS * SKY_TOP_RADIUS;
    let d = max(-r * mu + sqrt(max(discriminant, 0.0)), 0.0);
    let d_min = SKY_TOP_RADIUS - r;
    let d_max = rho + h;
    return vec2<f32>((d - d_min) / max(d_max - d_min, 1e-6), rho / h);
}

// The radius and zenith cosine a table texel stands for.
fn sky_transmittance_params(uv: vec2<f32>) -> vec2<f32> {
    let h = sqrt(SKY_TOP_RADIUS * SKY_TOP_RADIUS - SKY_GROUND_RADIUS * SKY_GROUND_RADIUS);
    let rho = h * uv.y;
    let r = sqrt(rho * rho + SKY_GROUND_RADIUS * SKY_GROUND_RADIUS);
    let d_min = SKY_TOP_RADIUS - r;
    let d_max = rho + h;
    let d = d_min + uv.x * (d_max - d_min);
    var mu = 1.0;
    if d > 0.0 {
        mu = clamp((h * h - rho * rho - d * d) / (2.0 * r * d), -1.0, 1.0);
    }
    return vec2<f32>(r, mu);
}

// The table's texel: what the air lets through from radius `r` along `mu` to space.
fn sky_transmittance_texel(sky: SkyParams, r: f32, mu: f32) -> vec3<f32> {
    let origin = vec3<f32>(0.0, r, 0.0);
    let direction = vec3<f32>(sqrt(max(1.0 - mu * mu, 0.0)), mu, 0.0);
    let span = max(sky_ray_sphere(origin, direction, SKY_TOP_RADIUS).y, 0.0);
    let steps = 40;
    let step = span / f32(steps);
    var depth = vec3<f32>(0.0);
    for (var i = 0; i < steps; i++) {
        let height = length(origin + direction * ((f32(i) + 0.5) * step)) - SKY_GROUND_RADIUS;
        depth += sky_extinction(sky_densities(sky, height)) * step;
    }
    return exp(-depth);
}

// ---------------------------------------------------------------------------- multiple scattering

// The multiple scattering table's texel: the light scattered twice and more (isotropic,
// for a sun of irradiance 1) at height and sun zenith cosine `uv` (x: cosine * 0.5 + 0.5,
// y: height from the ground to the top), integrated over 64 directions with the ground's
// bounce, the higher orders as a geometric series (Hillaire 2020).
fn sky_multiple_scattering_texel(
    sky: SkyParams,
    lut: texture_2d<f32>,
    lut_sampler: sampler,
    uv: vec2<f32>,
) -> vec3<f32> {
    let cos_sun = uv.x * 2.0 - 1.0;
    let r = mix(SKY_GROUND_RADIUS + 0.01, SKY_TOP_RADIUS - 0.01, uv.y);
    let origin = vec3<f32>(0.0, r, 0.0);
    let sun = vec3<f32>(sqrt(max(1.0 - cos_sun * cos_sun, 0.0)), cos_sun, 0.0);
    let isotropic = 1.0 / (4.0 * SKY_PI);
    let directions = 64;
    let steps = 20;
    var second = vec3<f32>(0.0);
    var as_one = vec3<f32>(0.0);
    for (var d = 0; d < directions; d++) {
        // A Fibonacci sphere.
        let z = 1.0 - (2.0 * f32(d) + 1.0) / f32(directions);
        let ring = sqrt(max(1.0 - z * z, 0.0));
        let angle = f32(d) * 2.39996323;
        let direction = vec3<f32>(ring * cos(angle), z, ring * sin(angle));
        let top = sky_ray_sphere(origin, direction, SKY_TOP_RADIUS).y;
        let ground = sky_ray_sphere(origin, direction, SKY_GROUND_RADIUS).x;
        let hits_ground = ground > 0.0;
        let end = select(top, ground, hits_ground);
        let dt = end / f32(steps);
        var depth = vec3<f32>(0.0);
        for (var i = 0; i < steps; i++) {
            let p = origin + direction * ((f32(i) + 0.5) * dt);
            let height = length(p);
            let densities = sky_densities(sky, height - SKY_GROUND_RADIUS);
            let extinction = sky_extinction(densities);
            let scattering = SKY_RAYLEIGH_SCATTERING * densities.x + vec3<f32>(SKY_MIE_SCATTERING * densities.y);
            let view = exp(-(depth + extinction * (0.5 * dt)));
            depth += extinction * dt;
            let sun_way = sky_light_transmittance(lut, lut_sampler, height, dot(p / height, sun));
            second += view * scattering * sun_way * isotropic * dt;
            as_one += view * scattering * dt;
        }
        if hits_ground {
            let p = origin + direction * end;
            let n = normalize(p);
            let sun_cos = dot(n, sun);
            second += exp(-depth) * sky_light_transmittance(lut, lut_sampler, SKY_GROUND_RADIUS, sun_cos)
                * max(sun_cos, 0.0) * sky.ground.rgb / SKY_PI;
        }
    }
    // Averages over the sphere: the second order's integrand carries the isotropic
    // phase, the share scattered on (f_ms) the sphere's 4 pi times it (Hillaire's eq. 7).
    let count = f32(directions);
    second = second / count;
    as_one = as_one / count;
    // Each further bounce scatters as much of the last as the air sends back: 1 / (1 - r).
    return second / max(vec3<f32>(1.0) - as_one, vec3<f32>(1e-3));
}

// The multiply scattered light at radius `r` with the light at zenith cosine `mu`.
fn sky_multiple_scattering(ms: texture_2d<f32>, ms_sampler: sampler, r: f32, mu: f32) -> vec3<f32> {
    let uv = vec2<f32>(
        clamp(mu * 0.5 + 0.5, 0.0, 1.0),
        clamp((r - SKY_GROUND_RADIUS) / (SKY_TOP_RADIUS - SKY_GROUND_RADIUS), 0.0, 1.0),
    );
    return textureSampleLevel(ms, ms_sampler, uv, 0.0).rgb;
}

// ---------------------------------------------------------------------------- scattering

// The night sky's own faint glow (airglow and unresolved starlight), brightest low down.
fn sky_night_glow(sky: SkyParams, direction: vec3<f32>) -> vec3<f32> {
    let low = 1.0 - clamp(direction.y, 0.0, 1.0);
    return sky.night_tint.rgb * (SKY_NIGHT_GLOW * sky.look.w * (1.0 + 2.0 * low * low * low * low));
}

// The light the air (and below the horizon the ground) sends toward the observer from
// `direction`: sunlight and moonlight scattered along the view, read with the
// transmittance table `lut` and the multiple scattering table `ms`.
fn sky_scattering(
    sky: SkyParams,
    lut: texture_2d<f32>,
    lut_sampler: sampler,
    ms: texture_2d<f32>,
    direction: vec3<f32>,
) -> vec3<f32> {
    let origin = vec3<f32>(0.0, SKY_GROUND_RADIUS + SKY_OBSERVER_HEIGHT, 0.0);
    let top = sky_ray_sphere(origin, direction, SKY_TOP_RADIUS).y;
    let ground = sky_ray_sphere(origin, direction, SKY_GROUND_RADIUS).x;
    let hits_ground = ground > 0.0;
    let end = select(top, ground, hits_ground);

    let sun = sky.sun.xyz;
    let moon = sky.moon.xyz;
    let g = sky.scattering.w;
    let cos_sun = dot(direction, sun);
    let cos_moon = dot(direction, moon);
    let rayleigh_sun = sky_rayleigh_phase(cos_sun);
    let mie_sun = sky_mie_phase(cos_sun, g);
    let rayleigh_moon = sky_rayleigh_phase(cos_moon);
    let mie_moon = sky_mie_phase(cos_moon, g);

    // Samples crowd toward the observer, where the air is densest along low rays.
    let steps = 32;
    var light = vec3<f32>(0.0);
    var depth = vec3<f32>(0.0);
    var previous = 0.0;
    for (var i = 0; i < steps; i++) {
        let s = (f32(i) + 1.0) / f32(steps);
        let t_next = end * s * s;
        let dt = t_next - previous;
        let t = previous + 0.5 * dt;
        previous = t_next;

        let p = origin + direction * t;
        let r = length(p);
        let up = p / r;
        let densities = sky_densities(sky, r - SKY_GROUND_RADIUS);
        let extinction = sky_extinction(densities);
        let view = exp(-(depth + extinction * (0.5 * dt)));
        depth += extinction * dt;

        let rayleigh = SKY_RAYLEIGH_SCATTERING * densities.x;
        let mie = SKY_MIE_SCATTERING * densities.y;
        let sun_up = dot(up, sun);
        let moon_up = dot(up, moon);
        let sun_way = sky_light_transmittance(lut, lut_sampler, r, sun_up);
        let moon_way = sky_light_transmittance(lut, lut_sampler, r, moon_up);
        let scatter = rayleigh + vec3<f32>(mie);
        let scattered = sky.sun_light.rgb * (sun_way * (rayleigh * rayleigh_sun + mie * mie_sun)
                + scatter * sky_multiple_scattering(ms, lut_sampler, r, sun_up))
            + sky.moon_light.rgb * (moon_way * (rayleigh * rayleigh_moon + mie * mie_moon)
                + scatter * sky_multiple_scattering(ms, lut_sampler, r, moon_up));
        light += view * scattered * dt;
    }

    if hits_ground {
        // The lit ground, seen through the air in front of it. Its sky light is a rough
        // share of the sun's.
        let p = origin + direction * end;
        let n = normalize(p);
        let sun_cos = dot(n, sun);
        let moon_cos = dot(n, moon);
        let irradiance = sky.sun_light.rgb * sky_light_transmittance(lut, lut_sampler, SKY_GROUND_RADIUS, sun_cos)
                * max(sun_cos, 0.0)
            + sky.sun_light.rgb * (0.12 * smoothstep(-0.1, 0.4, sun_cos))
            + sky.moon_light.rgb * sky_light_transmittance(lut, lut_sampler, SKY_GROUND_RADIUS, moon_cos)
                * max(moon_cos, 0.0);
        light += exp(-depth) * sky.ground.rgb / SKY_PI * irradiance;
    }
    return light * sky.look.x + sky_night_glow(sky, direction);
}

// What the air lets through from radius `r` toward a light at zenith cosine `mu`: none
// where the planet is in the way (its shadow's edge softened a little for twilight).
fn sky_light_transmittance(lut: texture_2d<f32>, lut_sampler: sampler, r: f32, mu: f32) -> vec3<f32> {
    let horizon = -sqrt(max(1.0 - (SKY_GROUND_RADIUS / r) * (SKY_GROUND_RADIUS / r), 0.0));
    let lit = smoothstep(horizon - 0.004, horizon + 0.004, mu);
    if lit <= 0.0 {
        return vec3<f32>(0.0);
    }
    let uv = sky_transmittance_uv(r, max(mu, horizon));
    return textureSampleLevel(lut, lut_sampler, uv, 0.0).rgb * lit;
}

// ---------------------------------------------------------------------------- what the background adds

// What the air lets through from beyond it toward the observer along `direction`
// (relative airmass, Kasten and Young 1989): none below the horizon.
fn sky_view_transmittance(sky: SkyParams, direction: vec3<f32>) -> vec3<f32> {
    let altitude = degrees(asin(clamp(direction.y, -1.0, 1.0)));
    if altitude < -0.5 {
        return vec3<f32>(0.0);
    }
    let a = max(altitude, 0.0);
    let airmass = 1.0 / (sin(radians(a)) + 0.50572 * pow(a + 6.07995, -1.6364));
    return exp(-sky.zenith_depth.rgb * airmass) * smoothstep(-0.5, 0.2, altitude);
}

fn sky_hash(v: vec3<u32>) -> vec3<u32> {
    // pcg3d (Jarzynski and Olano 2020).
    var w = v * 1664525u + 1013904223u;
    w.x += w.y * w.z;
    w.y += w.z * w.x;
    w.z += w.x * w.y;
    w = w ^ (w >> vec3<u32>(16u));
    w.x += w.y * w.z;
    w.y += w.z * w.x;
    w.z += w.x * w.y;
    return w;
}

fn sky_random(v: vec3<u32>) -> vec3<f32> {
    return vec3<f32>(sky_hash(v) >> vec3<u32>(8u)) / 16777216.0;
}

// The face of a cube `c` points through, and where on it ([-1, 1] each way).
fn sky_cube_face(c: vec3<f32>) -> vec3<f32> {
    let a = abs(c);
    if a.x >= a.y && a.x >= a.z {
        return vec3<f32>(c.yz / a.x, select(0.0, 1.0, c.x < 0.0));
    }
    if a.y >= a.z {
        return vec3<f32>(c.xz / a.y, select(2.0, 3.0, c.y < 0.0));
    }
    return vec3<f32>(c.xy / a.z, select(4.0, 5.0, c.z < 0.0));
}

fn sky_cube_direction(uv: vec2<f32>, face: f32) -> vec3<f32> {
    let f = u32(face);
    let s = select(1.0, -1.0, (f & 1u) == 1u);
    if f < 2u {
        return normalize(vec3<f32>(s, uv.x, uv.y));
    }
    if f < 4u {
        return normalize(vec3<f32>(uv.x, s, uv.y));
    }
    return normalize(vec3<f32>(uv.x, uv.y, s));
}

// One layer of stars over a cube of `cells` x `cells` cells a face, each holding a star
// with the chance `chance`, magnitudes from `brightest` to `faintest` (more of the faint
// ones, as in the real sky). `c` is the direction in the equatorial frame, `pixel` the
// pixel's angular size.
fn sky_star_layer(
    sky: SkyParams,
    c: vec3<f32>,
    cells: f32,
    chance: f32,
    brightest: f32,
    faintest: f32,
    pixel: f32,
    salt: u32,
) -> vec3<f32> {
    let face = sky_cube_face(c);
    let cell = floor((face.xy * 0.5 + 0.5) * cells);
    let key = vec3<u32>(vec2<u32>(cell), u32(face.z) * 16u + salt);
    let r = sky_random(key);
    if r.x > chance {
        return vec3<f32>(0.0);
    }
    let r2 = sky_random(key ^ vec3<u32>(0x9e3779b9u));
    // Away from the cell's edges, so its glow stays in the cell.
    let spot = (cell + 0.3 + 0.4 * r.yz) / cells * 2.0 - 1.0;
    let star = sky_cube_direction(spot, face.z);
    let away = length(star - c);
    let sigma = max(pixel * 0.55, 1e-5);
    if away > 4.0 * sigma {
        return vec3<f32>(0.0);
    }
    let magnitude = clamp(faintest + 2.0 * log2(max(r2.x, 1e-4)) * 0.30103, brightest, faintest);
    let flux = SKY_STAR_FLUX * pow(10.0, -0.4 * magnitude);
    // Hot stars blue-white, cool ones orange.
    let temperature = r2.y;
    let color = mix(vec3<f32>(1.0, 0.72, 0.45), vec3<f32>(0.78, 0.86, 1.0), smoothstep(0.1, 0.9, temperature));
    let twinkle = 1.0 + 0.35 * sin(sky.ground.w * (3.0 + 5.0 * r2.z) + 40.0 * r2.z);
    let profile = exp(-0.5 * (away / sigma) * (away / sigma)) / (2.0 * SKY_PI * sigma * sigma);
    return color * (flux * profile * twinkle);
}

fn sky_value_noise(p: vec3<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let base = vec3<u32>(vec3<i32>(i) + vec3<i32>(4096));
    var corners: array<f32, 8>;
    for (var k = 0u; k < 8u; k++) {
        let offset = vec3<u32>(k & 1u, (k >> 1u) & 1u, (k >> 2u) & 1u);
        corners[k] = sky_random(base + offset).x;
    }
    let x00 = mix(corners[0], corners[1], u.x);
    let x10 = mix(corners[2], corners[3], u.x);
    let x01 = mix(corners[4], corners[5], u.x);
    let x11 = mix(corners[6], corners[7], u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
}

fn sky_fbm(p: vec3<f32>) -> f32 {
    var sum = 0.0;
    var amplitude = 0.5;
    var q = p;
    for (var i = 0; i < 4; i++) {
        sum += amplitude * sky_value_noise(q);
        q = q * 2.03 + vec3<f32>(17.1, 3.7, 9.2);
        amplitude *= 0.5;
    }
    return sum;
}

// How much of the Milky Way lies along equatorial direction `c` (0-1): a band along the
// galactic plane, brightest toward the centre in Sagittarius, cut by dust lanes.
fn sky_milky_way_density(c: vec3<f32>) -> f32 {
    // The galactic north pole and centre (J2000).
    let pole = vec3<f32>(-0.8676661, -0.1980764, 0.4559838);
    let centre = vec3<f32>(-0.0548755, -0.8734371, -0.4838350);
    let latitude = asin(clamp(dot(c, pole), -1.0, 1.0));
    let toward_centre = max(dot(c, centre), 0.0);
    let width = 0.16 + 0.12 * toward_centre;
    let band = exp(-(latitude / width) * (latitude / width));
    let clouds = sky_fbm(c * 9.0);
    let dust = smoothstep(0.42, 0.62, sky_fbm(c * 17.0 + vec3<f32>(3.1)))
        * exp(-(latitude / 0.07) * (latitude / 0.07));
    return band * (0.35 + 0.65 * clouds) * (1.0 + 2.0 * toward_centre * toward_centre) * (1.0 - 0.7 * dust);
}

// The stars and the Milky Way along world `direction` (above the air; dim them with
// sky_view_transmittance), for a pixel `pixel` radians across.
fn sky_starlight(sky: SkyParams, direction: vec3<f32>, pixel: f32) -> vec3<f32> {
    if sky.look.y <= 0.0 && sky.look.z <= 0.0 {
        return vec3<f32>(0.0);
    }
    let c = normalize(vec3<f32>(
        dot(sky.celestial_x.xyz, direction),
        dot(sky.celestial_y.xyz, direction),
        dot(sky.celestial_z.xyz, direction),
    ));
    let galaxy = sky_milky_way_density(c);
    var stars = sky_star_layer(sky, c, 10.0, 0.5, -1.5, 3.0, pixel, 1u)
        + sky_star_layer(sky, c, 40.0, 0.3, 3.0, 5.0, pixel, 2u)
        + sky_star_layer(sky, c, 160.0, 0.2 + 0.3 * galaxy, 5.0, 7.0, pixel, 3u)
        + sky_star_layer(sky, c, 512.0, 0.08 + 0.5 * galaxy, 7.0, 9.0, pixel, 4u);
    let milky_way = vec3<f32>(1.0, 0.93, 0.82) * (galaxy * SKY_MILKY_WAY * sky.look.z);
    return stars * sky.look.y + milky_way;
}

// The sun's disc, darker toward its limb, through the air; `pixel` softens its edge.
fn sky_sun_disc(sky: SkyParams, direction: vec3<f32>, pixel: f32) -> vec3<f32> {
    let radius = sky.sun_light.w;
    let angle = acos(clamp(dot(direction, sky.sun.xyz), -1.0, 1.0));
    let coverage = clamp((radius - angle) / max(pixel, 1e-6) + 0.5, 0.0, 1.0);
    if coverage <= 0.0 {
        return vec3<f32>(0.0);
    }
    let mu = sqrt(max(1.0 - (angle / radius) * (angle / radius), 0.0));
    // Limb darkening u = 0.6, its mean over the disc 1 - u / 3.
    let limb = (1.0 - 0.6 * (1.0 - mu)) / 0.8;
    let radiance = sky.sun_seen.rgb / (SKY_PI * radius * radius) * limb;
    return min(radiance, vec3<f32>(SKY_SUN_DISC_MAX)) * coverage;
}

// The moon's disc lit in its phase by the sun (rgb), and how much of the pixel it covers
// (a), through the air.
fn sky_moon_disc(sky: SkyParams, direction: vec3<f32>, pixel: f32) -> vec4<f32> {
    let m = sky.moon.xyz;
    let radius = sky.moon.w;
    let angle = acos(clamp(dot(direction, m), -1.0, 1.0));
    let coverage = clamp((radius - angle) / max(pixel, 1e-6) + 0.5, 0.0, 1.0);
    if coverage <= 0.0 {
        return vec4<f32>(0.0);
    }
    // The disc's frame: up toward the celestial north, so its face turns over the night.
    let right = normalize(cross(sky.moon_north.xyz, m));
    let up = cross(m, right);
    let xy = vec2<f32>(dot(direction - m, right), dot(direction - m, up)) / sin(radius);
    let z = sqrt(max(1.0 - dot(xy, xy), 0.0));
    let normal = right * xy.x + up * xy.y - m * z;
    let lit = max(dot(normal, sky.sun.xyz), 0.0);
    // Maria and highlands.
    let face = vec3<f32>(xy, z);
    let maria = smoothstep(0.48, 0.62, sky_fbm(face * 2.2 + vec3<f32>(5.3, 1.7, 0.0)));
    let albedo = (0.92 - 0.38 * maria) * (0.9 + 0.2 * sky_fbm(face * 14.0));
    // A full moon's lit sphere averages 2/3 of its facing light; earthshine the rest.
    let radiance = sky.moon_seen.rgb / (SKY_PI * radius * radius) * 1.5 * albedo * (lit + 0.004);
    return vec4<f32>(radiance, coverage);
}
