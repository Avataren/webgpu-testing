//! The lights a frame is lit by, and their GPU layout.
//!
//! Directional lights are a small fixed array. Point and spot lights ("local" lights)
//! are one list of any length up to [`MAX_LOCAL_LIGHTS`]: a compute pass sorts them into
//! clusters of the camera's view (and a world-space grid for ray-traced passes), so a
//! surface only loops over the lights that can reach it (see `light_culling.rs`).

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};

pub const MAX_DIRECTIONAL_LIGHTS: usize = 4;
/// Shadow map cascades per directional light at most (see `internal/cascades.rs`).
pub const MAX_SHADOW_CASCADES: usize = 4;
/// Point and spot lights together. Beyond this, the lights nearest the camera are kept.
pub const MAX_LOCAL_LIGHTS: usize = 4096;
/// Point and spot lights with shadow maps at once (each a slot of the shadows uniform);
/// lights past the shadow budget are drawn unshadowed.
pub const MAX_SHADOWED_LOCAL_LIGHTS: usize = 64;
/// Local lights one cluster (or world grid cell) lists at most; a surface loops over no
/// more than this.
pub const MAX_LIGHTS_PER_CLUSTER: u32 = 256;
/// Bytes before the local lights in the lights buffer: the header and the directional
/// lights, padded so the light culling pass can bind the local lights on their own
/// (storage binding offsets are multiples of 256).
pub const LOCAL_LIGHTS_OFFSET: u64 = 512;
/// Point light cube faces.
pub const POINT_SHADOW_FACES: usize = 6;

/// A point or spot light as the scene hands it to the renderer.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LocalLight {
    /// Identifies the light across frames (its entity and scene), so it keeps its shadow
    /// map tiles and its shadow state can be looked up
    /// ([`crate::renderer::Renderer::light_shadow_state`]).
    pub key: u64,
    pub position: Vec3,
    pub color: Vec3,
    pub intensity: f32,
    /// Where the light fades out (0: never; it is then cut off where it gets too dim to
    /// matter, see [`LocalLight::reach`]).
    pub range: f32,
    /// A spot light's cone; `None` for a point light.
    pub cone: Option<SpotCone>,
    pub casts_shadow: bool,
    /// Weighs the light against others for the shadow budget (1: by its size on screen
    /// and brightness alone).
    pub shadow_priority: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotCone {
    /// Where the light shines (unit length).
    pub direction: Vec3,
    /// Half angles in radians: full light inside `inner_angle`, none past `outer_angle`.
    pub inner_angle: f32,
    pub outer_angle: f32,
}

impl Default for LocalLight {
    fn default() -> Self {
        Self {
            key: 0,
            position: Vec3::ZERO,
            color: Vec3::ONE,
            intensity: 1.0,
            range: 10.0,
            cone: None,
            casts_shadow: false,
            shadow_priority: 1.0,
        }
    }
}

/// Radiance below which a light without a range counts as out of reach.
const UNLIMITED_RANGE_CUTOFF: f32 = 0.005;

impl LocalLight {
    /// The distance the light reaches: its range, or for a light without one, where its
    /// inverse-square falloff drops below a small cutoff (and the shaders fade it out
    /// there as they fade out a ranged light).
    pub fn reach(&self) -> f32 {
        if self.range > 0.0 {
            return self.range;
        }
        let peak = self.color.max_element().max(0.0) * self.intensity.max(0.0);
        (peak / UNLIMITED_RANGE_CUTOFF).sqrt().clamp(0.1, 1.0e4)
    }

    /// Its colour times intensity, as one brightness.
    pub fn luminance(&self) -> f32 {
        self.color.dot(Vec3::new(0.2126, 0.7152, 0.0722)).max(0.0) * self.intensity.max(0.0)
    }

    /// Shadow map faces: 6 for a point light (a cube), 1 for a spot light.
    pub fn shadow_faces(&self) -> usize {
        if self.cone.is_some() {
            1
        } else {
            POINT_SHADOW_FACES
        }
    }

    /// The view projections of the light's shadow map faces for tiles `size` texels wide
    /// with `guard` texels of border: each face sees a little past its edges, so the
    /// filter taps near an edge stay inside its tile. Faces of a point light look along
    /// +X, -X, +Y, -Y, +Z, -Z (`select_point_face` in shadows.wgsl); a spot light has one.
    /// Also returns tan(fov / 2) of a face.
    pub fn shadow_view_projs(&self, size: u32, guard: u32) -> (Vec<Mat4>, f32) {
        let near = SHADOW_NEAR;
        let far = self.reach().max(near + 0.1);
        let widen = shadow_guard_scale(size, guard);
        match self.cone {
            None => {
                let tan_half = widen;
                let projection = Mat4::perspective_rh(2.0 * tan_half.atan(), 1.0, near, far);
                let matrices = POINT_FACE_AXES
                    .iter()
                    .map(|(dir, up)| {
                        projection * Mat4::look_at_rh(self.position, self.position + *dir, *up)
                    })
                    .collect();
                (matrices, tan_half)
            }
            Some(cone) => {
                let fov = (cone.outer_angle * 2.0).clamp(0.1, std::f32::consts::PI - 0.1);
                let tan_half = (fov * 0.5).tan() * widen;
                let projection = Mat4::perspective_rh(2.0 * tan_half.atan(), 1.0, near, far);
                let forward = safe_normalize(cone.direction, Vec3::NEG_Z);
                let helper = if forward.dot(Vec3::Y).abs() < 0.99 {
                    Vec3::Y
                } else {
                    Vec3::Z
                };
                let view = Mat4::look_at_rh(self.position, self.position + forward, helper);
                (vec![projection * view], tan_half)
            }
        }
    }
}

/// Where a local light's shadow map faces start.
pub const SHADOW_NEAR: f32 = 0.1;

/// Cube face directions and up vectors, in the order the shaders pick them.
const POINT_FACE_AXES: [(Vec3, Vec3); POINT_SHADOW_FACES] = [
    (Vec3::X, Vec3::Y),
    (Vec3::NEG_X, Vec3::Y),
    (Vec3::Y, Vec3::Z),
    (Vec3::NEG_Y, Vec3::NEG_Z),
    (Vec3::Z, Vec3::Y),
    (Vec3::NEG_Z, Vec3::Y),
];

/// How much wider than its nominal field of view a face of `size` texels with `guard`
/// texels of border sees (as a factor on tan(fov / 2)).
pub fn shadow_guard_scale(size: u32, guard: u32) -> f32 {
    let size = size.max(2 * guard + 2) as f32;
    size / (size - 2.0 * guard as f32)
}

fn safe_normalize(vec: Vec3, fallback: Vec3) -> Vec3 {
    vec.try_normalize().unwrap_or(fallback)
}

#[derive(Clone, Default)]
pub struct LightsData {
    directional: Vec<DirectionalLightRaw>,
    directional_shadows: Vec<Option<DirectionalShadowData>>,
    local: Vec<LocalLight>,
}

impl LightsData {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.directional.clear();
        self.directional_shadows.clear();
        self.local.clear();
    }

    pub fn add_directional(
        &mut self,
        direction: Vec3,
        color: Vec3,
        intensity: f32,
        shadow: Option<DirectionalShadowData>,
    ) {
        self.directional
            .push(DirectionalLightRaw::new(direction, color, intensity));
        self.directional_shadows.push(shadow);
    }

    /// Add a point or spot light.
    pub fn add_local(&mut self, light: LocalLight) {
        self.local.push(light);
    }

    /// Add `other`'s lights (directional lights up to [`MAX_DIRECTIONAL_LIGHTS`]; local
    /// lights all, the renderer keeps up to [`MAX_LOCAL_LIGHTS`]).
    pub fn extend_from(&mut self, other: &Self) {
        let remaining_dir = MAX_DIRECTIONAL_LIGHTS.saturating_sub(self.directional.len());
        let take_dir = remaining_dir.min(other.directional.len());
        self.directional
            .extend_from_slice(&other.directional[..take_dir]);
        self.directional_shadows
            .extend_from_slice(&other.directional_shadows[..take_dir]);
        self.local.extend_from_slice(&other.local);
    }

    pub fn directional_lights(&self) -> &[DirectionalLightRaw] {
        let count = self.directional.len().min(MAX_DIRECTIONAL_LIGHTS);
        &self.directional[..count]
    }

    /// Per directional light (as [`Self::directional_lights`]), how it casts shadows.
    pub fn directional_shadows(&self) -> &[Option<DirectionalShadowData>] {
        let count = self.directional_shadows.len().min(MAX_DIRECTIONAL_LIGHTS);
        &self.directional_shadows[..count]
    }

    pub fn local_lights(&self) -> &[LocalLight] {
        &self.local
    }

    /// Keep at most `max` local lights: those whose reach comes nearest `camera`, in
    /// their original order.
    pub fn limit_local_lights(&mut self, camera: Vec3, max: usize) {
        if self.local.len() <= max {
            return;
        }
        let mut order: Vec<(f32, usize)> = self
            .local
            .iter()
            .enumerate()
            .map(|(index, light)| (light.position.distance(camera) - light.reach(), index))
            .collect();
        order.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut keep = vec![false; self.local.len()];
        for (_, index) in order.into_iter().take(max) {
            keep[index] = true;
        }
        let mut index = 0;
        self.local.retain(|_| {
            let kept = keep[index];
            index += 1;
            kept
        });
    }
}

// All raw light/shadow structs are uploaded directly to GPU buffers.  WebGPU
// follows WGSL's std140/std430 layout rules which require 16 byte alignment for
// anything containing vectors/matrices.  The default C representation would only
// guarantee 4 byte alignment for arrays of `f32`, which in turn caused
// subsequent array elements to become misaligned once more than one entry was
// present.  Explicitly enforcing 16 byte alignment on these structs keeps the
// CPU layout in lock-step with the shader expectations.

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct DirectionalLightRaw {
    pub direction: [f32; 4],
    pub color_intensity: [f32; 4],
}

impl DirectionalLightRaw {
    pub fn new(direction: Vec3, color: Vec3, intensity: f32) -> Self {
        Self {
            direction: [direction.x, direction.y, direction.z, 0.0],
            color_intensity: [color.x, color.y, color.z, intensity],
        }
    }
}

/// How a directional light casts shadows: cascaded shadow maps that follow the camera
/// (the renderer fits them to the view each frame).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DirectionalShadowData {
    /// The farthest from the camera (in view depth, meters) its shadows reach; they fade
    /// out toward it. The cascades cover less when everything in view is nearer.
    pub distance: f32,
}

/// One cascade of a directional light's shadow maps (`ShadowCascade` in shadows.wgsl).
#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq, Debug)]
pub struct ShadowCascadeRaw {
    /// Light-space x and y to map coordinates: `uv = xy * [x, y] + [z, w]`.
    pub uv_transform: [f32; 4],
    /// x, y: light-space z to depth (`z * x + y`); z: a texel's width in meters; w: 0.
    pub depth_transform: [f32; 4],
}

/// A directional light's cascaded shadow maps (`DirectionalShadow` in shadows.wgsl).
#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq, Debug)]
pub struct DirectionalShadowRaw {
    /// World to light space (a rotation: the light shines along -z).
    pub light_view: [[f32; 4]; 4],
    pub cascades: [ShadowCascadeRaw; MAX_SHADOW_CASCADES],
    /// x: cascades (0: no shadow maps), y: the first one's layer of the cascade maps, z:
    /// texels across a map, w: tan of half the light's apparent size (soft shadows).
    pub params: [f32; 4],
    /// The camera's view depth of a point: `dot(xyz, point) + w`.
    pub view_depth: [f32; 4],
    /// x: view depth where the shadows start to fade out, y: 1 / the fade's length (0:
    /// no fade), z: the widest soft shadow edge in meters (how far the filter looks for
    /// casters), w: 0.
    pub fade: [f32; 4],
}

/// A point or spot light in the lights buffer (`LocalLight` in lighting_common.wgsl).
#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq, Debug)]
pub struct LocalLightRaw {
    /// xyz: position, w: range (always > 0; see [`LocalLight::reach`]).
    pub position_range: [f32; 4],
    pub color_intensity: [f32; 4],
    /// xyz: a spot light's axis, w: the cosine of its outer angle (-2 for a point light,
    /// which has no cone).
    pub direction: [f32; 4],
    /// x: the cosine of a spot light's inner angle, y: its slot in the shadows uniform,
    /// z: its bit in the ray-traced shadow mask (-1 for neither), w: 0.
    pub params: [f32; 4],
}

impl LocalLightRaw {
    pub fn new(light: &LocalLight, shadow_slot: Option<usize>, rt_bit: Option<u32>) -> Self {
        let (direction, cos_outer, cos_inner) = match light.cone {
            Some(cone) => {
                let (mut inner, mut outer) = (cone.inner_angle, cone.outer_angle);
                if inner > outer {
                    std::mem::swap(&mut inner, &mut outer);
                }
                (
                    safe_normalize(cone.direction, Vec3::NEG_Z),
                    outer.cos(),
                    inner.cos(),
                )
            }
            None => (Vec3::NEG_Z, -2.0, -2.0),
        };
        let p = light.position;
        let c = light.color;
        Self {
            position_range: [p.x, p.y, p.z, light.reach()],
            color_intensity: [c.x, c.y, c.z, light.intensity],
            direction: [direction.x, direction.y, direction.z, cos_outer],
            params: [
                cos_inner,
                shadow_slot.map_or(-1.0, |slot| slot as f32),
                rt_bit.map_or(-1.0, |bit| bit as f32),
                0.0,
            ],
        }
    }
}

/// The start of the lights buffer (`Lights` in lighting_common.wgsl): counts, how a
/// surface finds its cluster and world grid cell, and the directional lights. The local
/// lights follow at [`LOCAL_LIGHTS_OFFSET`].
#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct LightsHeader {
    /// x: directional lights, y: local lights, z: 1 when the world grid was built this
    /// frame, w: which bits of the ray-traced shadow mask the frame traced.
    pub counts: [u32; 4],
    /// The camera's view projection without TAA jitter: a surface's screen tile.
    pub cluster_view_proj: [[f32; 4]; 4],
    /// A point's view depth is dot(xyz, point) + w.
    pub cluster_depth_axis: [f32; 4],
    /// Clusters along x, y and depth, and their count.
    pub cluster_dims: [u32; 4],
    /// Depth slice: floor(log(depth) * x + y), or floor(depth * x + y) for an
    /// orthographic camera (w = 1); depths below z are in the first slice.
    pub cluster_slices: [f32; 4],
    /// The world grid's lowest corner (xyz) and its cells per meter (w).
    pub grid_origin: [f32; 4],
    /// Grid cells along x, y and z, and where their entries start in the light lists.
    pub grid_dims: [u32; 4],
    pub directionals: [DirectionalLightRaw; MAX_DIRECTIONAL_LIGHTS],
    pub _padding: [[f32; 4]; 14],
}

/// A local light's shadow maps in the atlas (`LocalShadow` in shadows.wgsl).
#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct LocalShadowRaw {
    /// Per face (one for a spot light).
    pub view_proj: [[[f32; 4]; 4]; POINT_SHADOW_FACES],
    /// Each face's tile in the atlas in texture coordinates: x, y, width, height.
    pub rects: [[f32; 4]; POINT_SHADOW_FACES],
    /// x: faces, y: texels across a tile, z: tan(fov / 2) of a face (a texel covers
    /// 2 * depth * z / y meters at view depth `depth`), w: 0.
    pub params: [f32; 4],
}

#[repr(C, align(16))]
#[derive(Clone, Copy, Pod, Zeroable, PartialEq)]
pub struct ShadowsUniform {
    /// x: directional lights, y: local shadow slots, z: the frame's noise index (soft
    /// shadow filter rotation), w: 0.
    pub counts: [u32; 4],
    pub directionals: [DirectionalShadowRaw; MAX_DIRECTIONAL_LIGHTS],
    pub locals: [LocalShadowRaw; MAX_SHADOWED_LOCAL_LIGHTS],
}

impl ShadowsUniform {
    /// The directional lights' cascades `directionals`, the local shadow slots `locals`,
    /// and the noise index `frame`.
    pub fn new(
        directionals: &[DirectionalShadowRaw],
        locals: &[LocalShadowRaw],
        frame: u32,
    ) -> Self {
        let mut uniform = Self::zeroed();
        let directional = directionals.len().min(MAX_DIRECTIONAL_LIGHTS);
        uniform.counts[0] = directional as u32;
        uniform.directionals[..directional].copy_from_slice(&directionals[..directional]);
        let count = locals.len().min(MAX_SHADOWED_LOCAL_LIGHTS);
        uniform.counts[1] = count as u32;
        uniform.counts[2] = frame;
        uniform.locals[..count].copy_from_slice(&locals[..count]);
        uniform
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_lights_pack_their_cone_shadow_slot_and_mask_bit() {
        let spot = LocalLight {
            position: Vec3::new(1.0, 2.0, 3.0),
            color: Vec3::new(0.8, 0.7, 0.6),
            intensity: 4.0,
            range: 10.0,
            cone: Some(SpotCone {
                direction: Vec3::new(0.0, -2.0, 0.0),
                inner_angle: 0.6,
                outer_angle: 0.3,
            }),
            ..LocalLight::default()
        };
        let raw = LocalLightRaw::new(&spot, Some(5), Some(9));
        assert_eq!(raw.position_range, [1.0, 2.0, 3.0, 10.0]);
        assert_eq!(raw.color_intensity, [0.8, 0.7, 0.6, 4.0]);
        // Normalized, and the angles swapped into inner <= outer.
        assert_eq!(&raw.direction[..3], &[0.0, -1.0, 0.0]);
        assert!((raw.direction[3] - 0.6f32.cos()).abs() < 1e-6);
        assert!((raw.params[0] - 0.3f32.cos()).abs() < 1e-6);
        assert_eq!(raw.params[1..], [5.0, 9.0, 0.0]);

        let point = LocalLightRaw::new(&LocalLight::default(), None, None);
        assert_eq!(point.direction[3], -2.0, "a point light has no cone");
        assert_eq!(point.params[1..3], [-1.0, -1.0]);
    }

    #[test]
    fn lights_without_a_range_reach_until_they_are_dim() {
        let bright = LocalLight {
            range: 0.0,
            intensity: 200.0,
            ..LocalLight::default()
        };
        let reach = bright.reach();
        // The falloff at the reach is the cutoff.
        assert!((bright.intensity / (reach * reach) - UNLIMITED_RANGE_CUTOFF).abs() < 1e-4);
        assert_eq!(
            LocalLight {
                range: 7.0,
                ..bright
            }
            .reach(),
            7.0
        );
    }

    #[test]
    fn too_many_local_lights_keep_the_nearest_in_order() {
        let mut data = LightsData::new();
        for x in [9.0, 1.0, 5.0, 2.0] {
            data.add_local(LocalLight {
                position: Vec3::new(x, 0.0, 0.0),
                range: 0.5,
                ..LocalLight::default()
            });
        }
        data.limit_local_lights(Vec3::ZERO, 2);
        let kept: Vec<f32> = data.local_lights().iter().map(|l| l.position.x).collect();
        assert_eq!(kept, vec![1.0, 2.0]);
    }

    #[test]
    fn point_shadow_faces_look_along_the_axes_and_see_past_their_edges() {
        let light = LocalLight {
            position: Vec3::new(-3.0, 4.5, 1.0),
            range: 12.0,
            ..LocalLight::default()
        };
        let (matrices, tan_half) = light.shadow_view_projs(256, 2);
        assert_eq!(matrices.len(), POINT_SHADOW_FACES);
        assert!(tan_half > 1.0);
        for (matrix, (dir, _)) in matrices.iter().zip(POINT_FACE_AXES) {
            // Straight ahead lands in the middle, at the far plane's depth at the range.
            let clip = *matrix * (light.position + dir * 12.0).extend(1.0);
            let ndc = clip.truncate() / clip.w;
            assert!(ndc.x.abs() < 1e-4 && ndc.y.abs() < 1e-4, "{dir:?}: {ndc:?}");
            assert!((ndc.z - 1.0).abs() < 1e-4);
            // The face's 45 degree edge lands guard texels inside the tile.
            let side = if dir.x.abs() > 0.5 { Vec3::Z } else { Vec3::X };
            let clip = *matrix * (light.position + dir + side).extend(1.0);
            let edge = (clip.truncate() / clip.w).abs().max_element();
            assert!((edge - 252.0 / 256.0).abs() < 1e-3, "{dir:?}: {edge}");
        }
    }

    #[test]
    fn spot_shadow_projects_along_the_cone() {
        let light = LocalLight {
            position: Vec3::new(2.0, 5.0, -1.0),
            range: 25.0,
            cone: Some(SpotCone {
                direction: Vec3::new(0.2, -1.0, 0.1).normalize(),
                inner_angle: 0.3,
                outer_angle: 0.6,
            }),
            ..LocalLight::default()
        };
        let (matrices, tan_half) = light.shadow_view_projs(512, 2);
        assert_eq!(matrices.len(), 1);
        assert!((tan_half - 0.6f32.tan() * 512.0 / 508.0).abs() < 1e-4);
        let axis = light.cone.unwrap().direction;
        let clip = matrices[0] * (light.position + axis * 10.0).extend(1.0);
        let ndc = clip.truncate() / clip.w;
        assert!(ndc.x.abs() < 1e-4 && ndc.y.abs() < 1e-4);
        assert!((0.0..=1.0).contains(&ndc.z));
    }

    #[test]
    fn gpu_structs_match_the_shader_layout() {
        use std::mem::{align_of, size_of};

        assert_eq!(align_of::<DirectionalLightRaw>(), 16);
        assert_eq!(align_of::<DirectionalShadowRaw>(), 16);
        assert_eq!(size_of::<ShadowCascadeRaw>(), 32);
        assert_eq!(
            size_of::<DirectionalShadowRaw>(),
            64 + 32 * MAX_SHADOW_CASCADES + 48
        );
        assert_eq!(size_of::<LocalLightRaw>(), 64);
        assert_eq!(size_of::<LightsHeader>() as u64, LOCAL_LIGHTS_OFFSET);
        assert_eq!(size_of::<LocalShadowRaw>(), 496);
        assert_eq!(size_of::<ShadowsUniform>() % 16, 0);
        // WebGPU's default uniform binding limit.
        assert!(size_of::<ShadowsUniform>() <= 65536);
    }
}
