//! Which point and spot lights get shadow maps, how big, and where in the shadow atlas.
//!
//! Every frame each shadow-casting local light gets a score: its shadow priority, times
//! how large its sphere of reach looks on screen, times its brightness (0 when its reach
//! is outside the view, so it lights nothing visible). From that size each light asks for
//! square tiles (six for a point light's cube faces, one for a spot light) of a power of
//! two texels; when the atlas cannot hold them all, the lowest-scoring lights get smaller
//! tiles, down to [`MIN_TILE`], and then none: those are drawn unshadowed. Lights keep
//! their tiles while their size class stays (a light that held tiles last frame scores a
//! little higher, so two lights of about equal score do not swap every frame), so the
//! cached shadow maps in them stay valid, and a light that leaves the view keeps its
//! tiles for [`KEEP_FRAMES`] unless another needs the room. Tiles come from a buddy
//! allocator over the atlas; the atlas grows (by powers of two, up to its configured
//! size) when the lights need more room than it has, and shrinks again when they have
//! needed at most half of a smaller one for [`SHRINK_FRAMES`].

use std::collections::HashMap;

use glam::Vec3;

use crate::renderer::internal::culling::{Frustum, Sphere};
use crate::renderer::lights::{LocalLight, MAX_SHADOWED_LOCAL_LIGHTS};

/// The smallest and the starting atlas size, and the smallest tile, in texels.
pub(crate) const MIN_ATLAS: u32 = 1024;
pub(crate) const MIN_TILE: u32 = 128;
/// Texels of border around each face (see `LocalLight::shadow_view_projs`).
pub(crate) const TILE_GUARD: u32 = 2;
/// Tile texels per pixel of the light's reach on screen.
const RESOLUTION_SCALE: f32 = 0.75;
/// Share of the largest atlas the planner fills before it shrinks tiles (the rest absorbs
/// the buddy allocator's fragmentation).
const FILL: f32 = 0.8;
/// A light that held tiles last frame scores this much higher.
const HOLD_BONUS: f32 = 1.25;
/// Frames a light keeps its tiles after it last had a shadow map (out of view or over
/// budget), so its cached maps survive a glance away. A light that is gone loses them at
/// once.
pub(crate) const KEEP_FRAMES: u64 = 600;
/// Frames the lights must need at most half of a smaller atlas (or none) before the atlas
/// shrinks to it.
pub(crate) const SHRINK_FRAMES: u32 = 600;

/// A square region of the atlas.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Tile {
    pub x: u32,
    pub y: u32,
    pub size: u32,
}

impl Tile {
    /// The tile in texture coordinates of an atlas `atlas` texels wide.
    pub(crate) fn rect(&self, atlas: u32) -> [f32; 4] {
        let scale = 1.0 / atlas as f32;
        [
            self.x as f32 * scale,
            self.y as f32 * scale,
            self.size as f32 * scale,
            self.size as f32 * scale,
        ]
    }
}

/// A buddy allocator of power-of-two square tiles in a `size` x `size` atlas.
#[derive(Debug, Default)]
pub(crate) struct TileAllocator {
    size: u32,
    /// Free tiles by size.
    free: HashMap<u32, Vec<(u32, u32)>>,
}

impl TileAllocator {
    pub(crate) fn new(size: u32) -> Self {
        let mut allocator = Self {
            size,
            free: HashMap::new(),
        };
        if size > 0 {
            allocator.free.insert(size, vec![(0, 0)]);
        }
        allocator
    }

    /// A free tile of `size` texels (a power of two), splitting a bigger one if needed.
    pub(crate) fn allocate(&mut self, size: u32) -> Option<Tile> {
        if size == 0 || size > self.size {
            return None;
        }
        if let Some((x, y)) = self.free.get_mut(&size).and_then(Vec::pop) {
            return Some(Tile { x, y, size });
        }
        let parent = self.allocate(size * 2)?;
        let half = size;
        self.free.entry(size).or_default().extend([
            (parent.x + half, parent.y),
            (parent.x, parent.y + half),
            (parent.x + half, parent.y + half),
        ]);
        Some(Tile {
            x: parent.x,
            y: parent.y,
            size,
        })
    }

    /// Give `tile` back, merging it with its free siblings into bigger tiles.
    pub(crate) fn release(&mut self, tile: Tile) {
        let mut tile = tile;
        while tile.size < self.size {
            let parent_size = tile.size * 2;
            let px = tile.x - tile.x % parent_size;
            let py = tile.y - tile.y % parent_size;
            let siblings = [
                (px, py),
                (px + tile.size, py),
                (px, py + tile.size),
                (px + tile.size, py + tile.size),
            ];
            let free = self.free.entry(tile.size).or_default();
            let all_free = siblings
                .iter()
                .all(|&s| s == (tile.x, tile.y) || free.contains(&s));
            if !all_free {
                break;
            }
            free.retain(|s| !siblings.contains(s));
            tile = Tile {
                x: px,
                y: py,
                size: parent_size,
            };
        }
        self.free
            .entry(tile.size)
            .or_default()
            .push((tile.x, tile.y));
    }
}

/// How the camera sees the lights, for their scores.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ShadowCamera {
    pub position: Vec3,
    pub frustum: Frustum,
    /// Pixels per unit of tan(angle) at the view's centre (viewport height over twice the
    /// tangent of half the field of view); for an orthographic camera, pixels per meter.
    pub pixels_per_unit: f32,
    pub orthographic: bool,
    pub viewport_height: f32,
}

/// One local light's place in the shadow atlas.
#[derive(Clone, Debug)]
pub(crate) struct LightTiles {
    pub size: u32,
    pub tiles: Vec<Tile>,
    /// The frame it last had a slot.
    pub last_used: u64,
}

/// Why a shadow-casting local light has no shadow map this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShadowState {
    /// Tiles of this many texels.
    Atlas { size: u32, rank: u32 },
    /// Its shadow comes from the ray-traced mask.
    RayTraced,
    /// Its reach is outside the view.
    OutOfView,
    /// The atlas (or the slots) ran out before its turn.
    OverBudget { rank: u32 },
}

/// What the planner decided for a frame.
#[derive(Default)]
pub(crate) struct AtlasPlan {
    /// Per local light: its tiles' key in [`ShadowAtlasPlanner::lights`] when it has
    /// a shadow map this frame, in score order (its slot in the shadows uniform is its
    /// position in `shadowed`).
    pub shadowed: Vec<usize>,
    /// The scores of the lights in `shadowed`.
    pub scores: Vec<f32>,
    /// Per local light, its state (`None` for lights that cast no shadow).
    pub states: Vec<Option<ShadowState>>,
    /// The atlas was recreated (it grew, shrank or was repacked): every cached map in
    /// it is gone.
    pub atlas_changed: bool,
    /// Lights with a shadow map but no room, and texels of the atlas in use.
    pub over_budget: u32,
    pub texels_used: u64,
}

/// Places the local lights' shadow maps in the atlas from frame to frame.
pub(crate) struct ShadowAtlasPlanner {
    /// The atlas's current size (0: none yet) and the largest it may grow to.
    pub size: u32,
    max_size: u32,
    max_tile: u32,
    allocator: TileAllocator,
    /// Tiles by light key.
    pub lights: HashMap<u64, LightTiles>,
    frame: u64,
    /// Consecutive frames the lights have fit in a smaller atlas.
    roomy_frames: u32,
}

impl ShadowAtlasPlanner {
    pub(crate) fn new(max_size: u32, max_tile: u32) -> Self {
        let max_size = max_size.max(MIN_ATLAS).next_power_of_two();
        Self {
            size: 0,
            max_size,
            max_tile: max_tile.clamp(MIN_TILE, max_size / 4).next_power_of_two(),
            allocator: TileAllocator::new(0),
            lights: HashMap::new(),
            frame: 0,
            roomy_frames: 0,
        }
    }

    /// The largest the atlas grows to.
    pub(crate) fn max_size(&self) -> u32 {
        self.max_size
    }

    /// Decide this frame's shadow maps for `lights`. Lights in `skip` (by index; their
    /// shadows come from the ray-traced mask) get none.
    pub(crate) fn plan(
        &mut self,
        lights: &[LocalLight],
        camera: &ShadowCamera,
        skip: &[bool],
    ) -> AtlasPlan {
        self.frame += 1;
        let mut plan = AtlasPlan {
            states: vec![None; lights.len()],
            ..AtlasPlan::default()
        };

        // Score the shadow casters the view can see.
        let brightest = brightest_caster(lights);
        let mut candidates: Vec<(f32, usize, u32)> = Vec::new();
        for (index, light) in lights.iter().enumerate() {
            if !light.casts_shadow {
                continue;
            }
            if skip.get(index).copied().unwrap_or(false) {
                plan.states[index] = Some(ShadowState::RayTraced);
                continue;
            }
            let Some((mut score, screen)) = shadow_score(light, camera, brightest) else {
                plan.states[index] = Some(ShadowState::OutOfView);
                continue;
            };
            let held = self.lights.get(&light.key);
            if held.is_some_and(|tiles| tiles.last_used + 1 >= self.frame) {
                score *= HOLD_BONUS;
            }
            let size = desired_tile(screen, held.map(|tiles| tiles.size), self.max_tile);
            candidates.push((score, index, size));
        }
        candidates.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
        candidates.truncate(MAX_SHADOWED_LOCAL_LIGHTS.max(1));

        // Fit them into the largest atlas allowed: shrink the lowest scores first.
        let area =
            |size: u32, index: usize| u64::from(size * size) * lights[index].shadow_faces() as u64;
        let capacity = (f64::from(self.max_size).powi(2) * f64::from(FILL)) as u64;
        let mut total: u64 = candidates.iter().map(|&(_, i, s)| area(s, i)).sum();
        while total > capacity {
            let Some(shrink) = candidates.iter().rposition(|&(_, _, size)| size > MIN_TILE) else {
                break;
            };
            let (_, index, size) = candidates[shrink];
            total -= area(size, index) - area(size / 2, index);
            candidates[shrink].2 = size / 2;
        }
        while total > capacity {
            let Some((_, index, size)) = candidates.pop() else {
                break;
            };
            total -= area(size, index);
            plan.states[index] = Some(ShadowState::OverBudget {
                rank: candidates.len() as u32 + 1,
            });
            plan.over_budget += 1;
        }

        // Tiles of lights that are gone, or have not had a shadow map for a while, go.
        let present: std::collections::HashSet<u64> = lights.iter().map(|l| l.key).collect();
        let expired: Vec<u64> = self
            .lights
            .iter()
            .filter(|(key, tiles)| {
                !present.contains(key) || tiles.last_used + KEEP_FRAMES < self.frame
            })
            .map(|(key, _)| *key)
            .collect();
        for key in expired {
            self.evict(key);
        }

        // An atlas that can hold them: at least their area, and bigger when they do not
        // fit (below). It shrinks (or goes, when no light needs one) once they have needed
        // at most half of a smaller one for SHRINK_FRAMES.
        let atlas_for = |texels: u64| {
            ((texels as f64).sqrt().ceil() as u32)
                .next_power_of_two()
                .clamp(MIN_ATLAS, self.max_size)
        };
        if candidates.is_empty() && self.size == 0 {
            return plan;
        }
        let least = atlas_for(total);
        if least > self.size {
            self.recreate(least);
            plan.atlas_changed = true;
            self.roomy_frames = 0;
        } else {
            let idle = candidates.is_empty() && self.lights.is_empty();
            let roomy = if idle { 0 } else { atlas_for(total * 2) };
            if roomy < self.size {
                self.roomy_frames += 1;
            } else {
                self.roomy_frames = 0;
            }
            if self.roomy_frames >= SHRINK_FRAMES {
                self.recreate(roomy);
                plan.atlas_changed = true;
                self.roomy_frames = 0;
                if roomy == 0 {
                    return plan;
                }
            }
        }

        // Free what changed size, then place the rest; double the atlas while they do not
        // fit.
        let keys: HashMap<u64, u32> = candidates
            .iter()
            .map(|&(_, index, size)| (lights[index].key, size))
            .collect();
        let stale: Vec<u64> = self
            .lights
            .iter()
            .filter(|(key, tiles)| keys.get(key).is_some_and(|size| *size != tiles.size))
            .map(|(key, _)| *key)
            .collect();
        for key in stale {
            self.evict(key);
        }
        let placed = loop {
            let (placed, repacked) = self.place(&candidates, lights, &keys);
            plan.atlas_changed |= repacked;
            if placed.iter().all(|p| *p) || self.size >= self.max_size {
                break placed;
            }
            self.recreate(self.size * 2);
            plan.atlas_changed = true;
            self.roomy_frames = 0;
        };

        for (rank, (slot, &(score, index, size))) in candidates.iter().enumerate().enumerate() {
            if !placed[slot] {
                plan.states[index] = Some(ShadowState::OverBudget {
                    rank: rank as u32 + 1,
                });
                plan.over_budget += 1;
                continue;
            }
            if let Some(tiles) = self.lights.get_mut(&lights[index].key) {
                tiles.last_used = self.frame;
            }
            plan.states[index] = Some(ShadowState::Atlas {
                size,
                rank: rank as u32 + 1,
            });
            plan.shadowed.push(index);
            plan.scores.push(score);
        }
        plan.texels_used = self
            .lights
            .values()
            .map(|tiles| u64::from(tiles.size * tiles.size) * tiles.tiles.len() as u64)
            .sum();
        plan
    }

    /// Give each candidate without tiles its tiles, the largest first (which keeps the
    /// buddy allocator from fragmenting); evict lights not wanted this frame (those not
    /// seen for the longest first) when room runs out, and start over in an empty atlas
    /// if it is too fragmented. Returns which candidates have tiles and whether the
    /// atlas started over.
    fn place(
        &mut self,
        candidates: &[(f32, usize, u32)],
        lights: &[LocalLight],
        keys: &HashMap<u64, u32>,
    ) -> (Vec<bool>, bool) {
        let mut order: Vec<usize> = (0..candidates.len()).collect();
        order.sort_by_key(|&slot| std::cmp::Reverse(candidates[slot].2));
        let mut repacked = false;
        let mut placed = vec![false; candidates.len()];
        let mut attempt = 0;
        while attempt < order.len() {
            let slot = order[attempt];
            let (_, index, size) = candidates[slot];
            let light = &lights[index];
            if self.lights.contains_key(&light.key) {
                placed[slot] = true;
                attempt += 1;
                continue;
            }
            match self.allocate_faces(size, light.shadow_faces(), keys) {
                Some(tiles) => {
                    self.lights.insert(
                        light.key,
                        LightTiles {
                            size,
                            tiles,
                            last_used: self.frame,
                        },
                    );
                    placed[slot] = true;
                    attempt += 1;
                }
                None if !repacked => {
                    // Fragmented: free everything and place them all again.
                    repacked = true;
                    self.recreate(self.size);
                    placed.iter_mut().for_each(|p| *p = false);
                    attempt = 0;
                }
                None => {
                    attempt += 1;
                }
            }
        }
        (placed, repacked)
    }

    /// `count` tiles of `size`, evicting lights not in `wanted` (oldest first) for room.
    fn allocate_faces(
        &mut self,
        size: u32,
        count: usize,
        wanted: &HashMap<u64, u32>,
    ) -> Option<Vec<Tile>> {
        let mut tiles = Vec::with_capacity(count);
        while tiles.len() < count {
            match self.allocator.allocate(size) {
                Some(tile) => tiles.push(tile),
                None => {
                    let victim = self
                        .lights
                        .iter()
                        .filter(|(key, _)| !wanted.contains_key(key))
                        .min_by_key(|(_, tiles)| tiles.last_used)
                        .map(|(key, _)| *key);
                    match victim {
                        Some(key) => self.evict(key),
                        None => {
                            for tile in tiles {
                                self.allocator.release(tile);
                            }
                            return None;
                        }
                    }
                }
            }
        }
        Some(tiles)
    }

    fn evict(&mut self, key: u64) {
        if let Some(tiles) = self.lights.remove(&key) {
            for tile in tiles.tiles {
                self.allocator.release(tile);
            }
        }
    }

    /// An empty atlas of `size` texels.
    fn recreate(&mut self, size: u32) {
        self.size = size;
        self.allocator = TileAllocator::new(size);
        self.lights.clear();
    }
}

/// The luminance of the brightest shadow caster, against which the others' brightness is
/// weighed.
fn brightest_caster(lights: &[LocalLight]) -> f32 {
    lights
        .iter()
        .filter(|light| light.casts_shadow)
        .map(LocalLight::luminance)
        .fold(0.0f32, f32::max)
        .max(1e-6)
}

/// A shadow caster's score (its priority, times its size on screen in pixels, times its
/// brightness against the brightest's) and that size; `None` when its reach is outside the
/// view.
fn shadow_score(light: &LocalLight, camera: &ShadowCamera, brightest: f32) -> Option<(f32, f32)> {
    let reach = light.reach();
    let sphere = Sphere {
        center: light.position,
        radius: reach,
    };
    if !camera.frustum.intersects(sphere) {
        return None;
    }
    let screen = screen_size(light.position, reach, camera);
    let score =
        light.shadow_priority.max(0.0) * screen * (0.25 + 0.75 * light.luminance() / brightest);
    Some((score, screen))
}

/// The shadow-casting lights the view can see, highest score first (as the shadow budget
/// ranks them), by index.
pub(crate) fn rank_shadow_casters(lights: &[LocalLight], camera: &ShadowCamera) -> Vec<usize> {
    let brightest = brightest_caster(lights);
    let mut ranked: Vec<(f32, usize)> = lights
        .iter()
        .enumerate()
        .filter(|(_, light)| light.casts_shadow)
        .filter_map(|(index, light)| Some((shadow_score(light, camera, brightest)?.0, index)))
        .collect();
    ranked.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    ranked.into_iter().map(|(_, index)| index).collect()
}

/// How many pixels across the sphere of `radius` around `center` looks from `camera`
/// (twice the view's height when the camera is inside it).
fn screen_size(center: Vec3, radius: f32, camera: &ShadowCamera) -> f32 {
    let full = camera.viewport_height * 2.0;
    if camera.orthographic {
        return (2.0 * radius * camera.pixels_per_unit).min(full);
    }
    let distance = center.distance(camera.position);
    if distance <= radius {
        return full;
    }
    let tangent = radius / (distance * distance - radius * radius).sqrt();
    (2.0 * tangent * camera.pixels_per_unit).min(full)
}

/// The tile size for a light `screen` pixels across: the power of two near
/// [`RESOLUTION_SCALE`] times that, kept at `current` while it stays within a band
/// around it (so tiles do not flip between sizes as the camera moves).
fn desired_tile(screen: f32, current: Option<u32>, max_tile: u32) -> u32 {
    let wanted = (screen * RESOLUTION_SCALE).max(1.0);
    if let Some(current) = current {
        let current_f = current as f32;
        if wanted >= current_f * 0.6 && wanted <= current_f * 1.5 {
            return current.clamp(MIN_TILE, max_tile);
        }
    }
    (wanted.round() as u32)
        .next_power_of_two()
        .clamp(MIN_TILE, max_tile)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Mat4;

    #[test]
    fn the_allocator_splits_and_merges_buddies() {
        let mut allocator = TileAllocator::new(1024);
        let a = allocator.allocate(256).unwrap();
        let b = allocator.allocate(256).unwrap();
        assert_ne!(a, b);
        let tiles: Vec<Tile> = (0..14).map(|_| allocator.allocate(256).unwrap()).collect();
        assert!(
            allocator.allocate(256).is_none(),
            "16 tiles of 256 fill 1024"
        );
        for tile in tiles.into_iter().chain([a, b]) {
            allocator.release(tile);
        }
        // Everything merged back: the whole atlas is one tile again.
        assert_eq!(
            allocator.allocate(1024),
            Some(Tile {
                x: 0,
                y: 0,
                size: 1024
            })
        );
    }

    fn camera(position: Vec3) -> ShadowCamera {
        let view = Mat4::look_at_rh(position, position + Vec3::NEG_Z, Vec3::Y);
        let projection = Mat4::perspective_rh(60f32.to_radians(), 16.0 / 9.0, 0.1, 400.0);
        ShadowCamera {
            position,
            frustum: Frustum::from_view_proj(projection * view),
            pixels_per_unit: 900.0 / (2.0 * 30f32.to_radians().tan()),
            orthographic: false,
            viewport_height: 900.0,
        }
    }

    fn lantern(key: u64, z: f32) -> LocalLight {
        LocalLight {
            key,
            position: Vec3::new(0.0, 2.0, z),
            range: 9.0,
            intensity: 30.0,
            casts_shadow: true,
            ..LocalLight::default()
        }
    }

    #[test]
    fn nearer_lights_get_bigger_tiles_and_lights_out_of_view_none() {
        let mut planner = ShadowAtlasPlanner::new(8192, 1024);
        let lights = [lantern(1, -5.0), lantern(2, -60.0), lantern(3, 50.0)];
        let plan = planner.plan(&lights, &camera(Vec3::ZERO), &[]);
        assert_eq!(plan.shadowed, vec![0, 1]);
        let size = |state: Option<ShadowState>| match state {
            Some(ShadowState::Atlas { size, .. }) => size,
            other => panic!("{other:?}"),
        };
        assert_eq!(size(plan.states[0]), 1024, "the camera is inside its reach");
        assert!(size(plan.states[1]) < 1024);
        assert_eq!(plan.states[2], Some(ShadowState::OutOfView));
        // Every face has a tile, and no two overlap.
        let mut all: Vec<Tile> = Vec::new();
        for tiles in planner.lights.values() {
            assert_eq!(tiles.tiles.len(), 6);
            all.extend(&tiles.tiles);
        }
        for (i, a) in all.iter().enumerate() {
            for b in &all[i + 1..] {
                let apart = a.x + a.size <= b.x
                    || b.x + b.size <= a.x
                    || a.y + a.size <= b.y
                    || b.y + b.size <= a.y;
                assert!(apart, "{a:?} overlaps {b:?}");
            }
        }
    }

    #[test]
    fn tiles_stay_put_while_nothing_changes() {
        let mut planner = ShadowAtlasPlanner::new(8192, 1024);
        let lights = [lantern(1, -5.0), lantern(2, -20.0)];
        let camera = camera(Vec3::ZERO);
        planner.plan(&lights, &camera, &[]);
        let before = planner.lights.get(&2).unwrap().tiles.clone();
        let plan = planner.plan(&lights, &camera, &[]);
        assert!(!plan.atlas_changed);
        assert_eq!(planner.lights.get(&2).unwrap().tiles, before);
    }

    #[test]
    fn a_full_atlas_shrinks_the_lowest_scores_then_drops_them() {
        // Room for about one light at the biggest tiles.
        let mut planner = ShadowAtlasPlanner::new(1024, 256);
        let lights: Vec<LocalLight> = (0..40).map(|i| lantern(i, -3.0 - i as f32 * 4.0)).collect();
        let plan = planner.plan(&lights, &camera(Vec3::ZERO), &[]);
        assert!(plan.over_budget > 0);
        assert!(plan.texels_used <= 1024 * 1024);
        assert!(matches!(plan.states[0], Some(ShadowState::Atlas { .. })));
        assert!(matches!(
            plan.states[39],
            Some(ShadowState::OverBudget { .. }) | Some(ShadowState::OutOfView)
        ));
        // The higher scores are first and never smaller than lower ones.
        let sizes: Vec<u32> = plan
            .shadowed
            .iter()
            .map(|&i| match plan.states[i] {
                Some(ShadowState::Atlas { size, .. }) => size,
                _ => 0,
            })
            .collect();
        assert!(sizes.windows(2).all(|w| w[0] >= w[1]), "{sizes:?}");
    }

    #[test]
    fn ray_traced_lights_and_lights_without_shadows_take_no_tiles() {
        let mut planner = ShadowAtlasPlanner::new(4096, 512);
        let mut unshadowed = lantern(2, -6.0);
        unshadowed.casts_shadow = false;
        let lights = [lantern(1, -5.0), unshadowed];
        let plan = planner.plan(&lights, &camera(Vec3::ZERO), &[true, false]);
        assert!(plan.shadowed.is_empty());
        assert_eq!(plan.states[0], Some(ShadowState::RayTraced));
        assert_eq!(plan.states[1], None);
        assert_eq!(planner.size, 0, "no atlas until a light needs one");
    }

    #[test]
    fn the_atlas_grows_with_demand() {
        let mut planner = ShadowAtlasPlanner::new(8192, 1024);
        let camera = camera(Vec3::ZERO);
        planner.plan(&[lantern(1, -40.0)], &camera, &[]);
        let small = planner.size;
        assert!(small >= MIN_ATLAS);
        let lights: Vec<LocalLight> = (0..12).map(|i| lantern(i, -4.0 - i as f32)).collect();
        let plan = planner.plan(&lights, &camera, &[]);
        assert!(planner.size > small);
        assert!(plan.atlas_changed);
        assert_eq!(plan.shadowed.len(), 12);
    }

    #[test]
    fn the_atlas_is_as_small_as_the_tiles_allow() {
        let mut planner = ShadowAtlasPlanner::new(8192, 1024);
        let spot = |key, z| LocalLight {
            cone: Some(crate::renderer::lights::SpotCone {
                direction: Vec3::NEG_Y,
                inner_angle: 0.4,
                outer_angle: 0.6,
            }),
            ..lantern(key, z)
        };
        // Two cube maps and two spot maps of 1024: 14 of the 16 tiles of a 4096 atlas.
        let lights = [
            lantern(1, -3.0),
            lantern(2, -4.0),
            spot(3, -3.0),
            spot(4, -4.0),
        ];
        let plan = planner.plan(&lights, &camera(Vec3::ZERO), &[]);
        assert_eq!(plan.shadowed.len(), 4);
        assert!(plan
            .states
            .iter()
            .all(|state| matches!(state, Some(ShadowState::Atlas { size: 1024, .. }))));
        assert_eq!(planner.size, 4096);
    }

    #[test]
    fn lights_that_go_give_their_tiles_back_and_the_atlas_shrinks() {
        let mut planner = ShadowAtlasPlanner::new(8192, 1024);
        let camera = camera(Vec3::ZERO);
        let many: Vec<LocalLight> = (0..12).map(|i| lantern(i, -4.0 - i as f32)).collect();
        planner.plan(&many, &camera, &[]);
        let big = planner.size;

        // One light stays: the others' tiles go at once, the atlas after a while.
        let one = [lantern(100, -40.0)];
        planner.plan(&one, &camera, &[]);
        assert_eq!(planner.lights.len(), 1);
        assert_eq!(planner.size, big, "the atlas does not shrink at once");
        let mut shrank = false;
        for _ in 0..SHRINK_FRAMES {
            shrank |= planner.plan(&one, &camera, &[]).atlas_changed;
        }
        assert!(shrank && planner.size < big, "{} of {big}", planner.size);
        let plan = planner.plan(&one, &camera, &[]);
        assert_eq!(plan.shadowed, vec![0], "the light is placed again");

        // No light needs a shadow map: the atlas goes.
        for _ in 0..SHRINK_FRAMES + 1 {
            planner.plan(&[], &camera, &[]);
        }
        assert_eq!(planner.size, 0);
        assert!(planner.lights.is_empty());
    }

    #[test]
    fn a_light_out_of_view_keeps_its_tiles_for_a_while() {
        let mut planner = ShadowAtlasPlanner::new(8192, 1024);
        let ahead = camera(Vec3::ZERO);
        let lights = [lantern(1, -5.0), lantern(2, -30.0)];
        planner.plan(&lights, &ahead, &[]);
        let tiles = planner.lights.get(&2).unwrap().tiles.clone();
        // Look away from light 2 (it sits 30 m behind a camera at z = -60 facing -z).
        let away = camera(Vec3::new(0.0, 0.0, -60.0));
        let plan = planner.plan(&lights, &away, &[]);
        assert_eq!(plan.states[1], Some(ShadowState::OutOfView));
        planner.plan(&lights, &ahead, &[]);
        assert_eq!(planner.lights.get(&2).unwrap().tiles, tiles);
        for _ in 0..KEEP_FRAMES + 1 {
            planner.plan(&lights, &away, &[]);
        }
        assert!(!planner.lights.contains_key(&2));
    }
}
