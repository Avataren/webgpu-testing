//! Gradient noise (Ken Perlin's improved noise) for procedural shapes.
//!
//! Deterministic: the same coordinates always give the same value, so offset the
//! coordinates to get a different pattern.

/// Perlin's reference permutation, repeated so lookups need no wrapping.
const PERM: [u8; 256] = [
    151, 160, 137, 91, 90, 15, 131, 13, 201, 95, 96, 53, 194, 233, 7, 225, 140, 36, 103, 30, 69,
    142, 8, 99, 37, 240, 21, 10, 23, 190, 6, 148, 247, 120, 234, 75, 0, 26, 197, 62, 94, 252, 219,
    203, 117, 35, 11, 32, 57, 177, 33, 88, 237, 149, 56, 87, 174, 20, 125, 136, 171, 168, 68, 175,
    74, 165, 71, 134, 139, 48, 27, 166, 77, 146, 158, 231, 83, 111, 229, 122, 60, 211, 133, 230,
    220, 105, 92, 41, 55, 46, 245, 40, 244, 102, 143, 54, 65, 25, 63, 161, 1, 216, 80, 73, 209, 76,
    132, 187, 208, 89, 18, 169, 200, 196, 135, 130, 116, 188, 159, 86, 164, 100, 109, 198, 173,
    186, 3, 64, 52, 217, 226, 250, 124, 123, 5, 202, 38, 147, 118, 126, 255, 82, 85, 212, 207, 206,
    59, 227, 47, 16, 58, 17, 182, 189, 28, 42, 223, 183, 170, 213, 119, 248, 152, 2, 44, 154, 163,
    70, 221, 153, 101, 155, 167, 43, 172, 9, 129, 22, 39, 253, 19, 98, 108, 110, 79, 113, 224, 232,
    178, 185, 112, 104, 218, 246, 97, 228, 251, 34, 242, 193, 238, 210, 144, 12, 191, 179, 162,
    241, 81, 51, 145, 235, 249, 14, 239, 107, 49, 192, 214, 31, 181, 199, 106, 157, 184, 84, 204,
    176, 115, 121, 50, 45, 127, 4, 150, 254, 138, 236, 205, 93, 222, 114, 67, 29, 24, 72, 243, 141,
    128, 195, 78, 66, 215, 61, 156, 180,
];

fn perm(i: usize) -> usize {
    PERM[i & 255] as usize
}

fn fade(t: f64) -> f64 {
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

fn grad(hash: usize, x: f64, y: f64, z: f64) -> f64 {
    let h = hash & 15;
    let u = if h < 8 { x } else { y };
    let v = if h < 4 {
        y
    } else if h == 12 || h == 14 {
        x
    } else {
        z
    };
    (if h & 1 == 0 { u } else { -u }) + (if h & 2 == 0 { v } else { -v })
}

/// 3D gradient noise, roughly in -1..1, smooth, with features about one unit apart. It is
/// 0 at integer coordinates, so sample between them (e.g. `x * 0.13`).
pub fn perlin3(x: f64, y: f64, z: f64) -> f64 {
    let (xf, yf, zf) = (x.floor(), y.floor(), z.floor());
    let (xi, yi, zi) = (
        (xf as i64 & 255) as usize,
        (yf as i64 & 255) as usize,
        (zf as i64 & 255) as usize,
    );
    let (x, y, z) = (x - xf, y - yf, z - zf);
    let (u, v, w) = (fade(x), fade(y), fade(z));

    let a = perm(xi) + yi;
    let aa = perm(a) + zi;
    let ab = perm(a + 1) + zi;
    let b = perm(xi + 1) + yi;
    let ba = perm(b) + zi;
    let bb = perm(b + 1) + zi;

    lerp(
        w,
        lerp(
            v,
            lerp(u, grad(perm(aa), x, y, z), grad(perm(ba), x - 1.0, y, z)),
            lerp(
                u,
                grad(perm(ab), x, y - 1.0, z),
                grad(perm(bb), x - 1.0, y - 1.0, z),
            ),
        ),
        lerp(
            v,
            lerp(
                u,
                grad(perm(aa + 1), x, y, z - 1.0),
                grad(perm(ba + 1), x - 1.0, y, z - 1.0),
            ),
            lerp(
                u,
                grad(perm(ab + 1), x, y - 1.0, z - 1.0),
                grad(perm(bb + 1), x - 1.0, y - 1.0, z - 1.0),
            ),
        ),
    )
}

/// Fractal noise: `octaves` layers of [`perlin3`], each at twice the frequency and half the
/// amplitude of the last, scaled back to roughly -1..1.
pub fn fbm3(x: f64, y: f64, z: f64, octaves: u32) -> f64 {
    let (mut sum, mut amplitude, mut frequency, mut total) = (0.0, 1.0, 1.0, 0.0);
    for _ in 0..octaves.clamp(1, 16) {
        sum += amplitude * perlin3(x * frequency, y * frequency, z * frequency);
        total += amplitude;
        amplitude *= 0.5;
        frequency *= 2.0;
    }
    sum / total
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn noise_is_zero_at_lattice_points_and_bounded() {
        assert_eq!(perlin3(3.0, -7.0, 12.0), 0.0);
        let mut seen_positive = false;
        let mut seen_negative = false;
        for i in 0..2000 {
            let t = i as f64 * 0.137;
            let n = perlin3(t, t * 0.71 + 3.3, -t * 0.39);
            assert!((-1.1..=1.1).contains(&n), "{n} out of range");
            seen_positive |= n > 0.2;
            seen_negative |= n < -0.2;
        }
        assert!(seen_positive && seen_negative);
    }

    #[test]
    fn noise_is_deterministic_and_continuous() {
        let a = perlin3(1.25, 2.5, 3.75);
        assert_eq!(a, perlin3(1.25, 2.5, 3.75));
        let b = perlin3(1.2501, 2.5, 3.75);
        assert!((a - b).abs() < 0.01);
    }

    #[test]
    fn fbm_stays_in_range() {
        for i in 0..500 {
            let t = i as f64 * 0.31;
            let n = fbm3(t, -t * 0.5, t * 0.25, 5);
            assert!((-1.1..=1.1).contains(&n));
        }
    }
}
