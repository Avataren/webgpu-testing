//! Which shadow casters stay put and which move, from frame to frame, for the point and
//! spot lights' shadow cache (`shadows.rs`).
//!
//! A local light's face keeps its static casters in a second atlas and is rebuilt each
//! frame from it: a copy of the static depth, then the dynamic casters drawn over it. A
//! caster is dynamic when it is skinned, placed by the GPU, has no bounds (a shader
//! material may move its vertices), or changed within the last [`SETTLE_FRAMES`]
//! frames (a cart rolling, a door swinging); everything else is static. A face's static
//! depth stays valid until a static caster near it changes: one appears, goes, starts
//! moving or settles. [`CasterTracker::update`] lists the spheres where that happened,
//! so faces only re-render their static casters when one of those spheres reaches into
//! their view, instead of hashing their casters every frame.

use std::collections::HashMap;

use crate::renderer::batch::InstanceSource;
use crate::renderer::internal::culling::Sphere;
use crate::renderer::internal::{OrderedBatch, PreparedBatches};
use crate::renderer::RenderPass;

/// Frames a caster must keep still to count as static again.
pub(crate) const SETTLE_FRAMES: u64 = 60;

/// Whether a pass's batches cast shadows.
pub(crate) fn casts_shadows(pass: RenderPass) -> bool {
    !matches!(
        pass,
        RenderPass::Transparent | RenderPass::Overlay | RenderPass::Gizmo | RenderPass::GizmoSolid
    )
}

/// A static caster as last seen.
struct Tracked {
    /// A hash of what decides its shadow: transform, cut-out material, sidedness.
    content: u64,
    sphere: Sphere,
    /// The frame its content last changed (`None`: not since it appeared).
    changed: Option<u64>,
    /// The frame it was last seen.
    seen: u64,
    /// Whether it counted as static last frame.
    settled: bool,
}

/// What changed among the casters this frame.
#[derive(Default)]
pub(crate) struct CasterFrame {
    /// Per batch (as `PreparedBatches::all`), per instance: whether it is a dynamic caster
    /// this frame. Empty for batches that cast no shadow.
    pub dynamic: Vec<Vec<bool>>,
    /// Where the static casters changed: faces whose view reaches one of these spheres
    /// must render their static casters again.
    pub dirty: Vec<Sphere>,
    /// Every face's static casters must render again (cut-out textures changed).
    pub all_dirty: bool,
    /// The dynamic casters' spheres (unbounded for those without bounds).
    pub dynamic_spheres: Vec<Sphere>,
}

impl CasterFrame {
    /// Whether instance `instance` of batch `batch` is dynamic.
    pub(crate) fn is_dynamic(&self, batch: usize, instance: usize) -> bool {
        self.dynamic
            .get(batch)
            .and_then(|flags| flags.get(instance))
            .copied()
            .unwrap_or(false)
    }
}

/// Tracks the static casters across frames.
#[derive(Default)]
pub(crate) struct CasterTracker {
    frame: u64,
    /// By mesh identity, pick id, and which occurrence of that pair the instance is.
    casters: HashMap<(u64, u64, u32), Tracked>,
    texture_generation: Option<u64>,
}

impl CasterTracker {
    /// Classify this frame's casters and list where the static ones changed.
    /// `mesh_identity` names a batch's geometry (None: nothing to draw).
    pub(crate) fn update(
        &mut self,
        batches: &PreparedBatches,
        mesh_identity: impl Fn(&OrderedBatch) -> Option<u64>,
        texture_generation: u64,
    ) -> CasterFrame {
        use std::hash::{Hash, Hasher};
        self.frame += 1;
        let frame = self.frame;
        let mut out = CasterFrame {
            all_dirty: self.texture_generation != Some(texture_generation),
            ..CasterFrame::default()
        };
        self.texture_generation = Some(texture_generation);
        let materials = batches.materials();
        let mut occurrences: HashMap<(u64, u64), u32> = HashMap::new();

        for batch in batches.all() {
            if !casts_shadows(batch.pass) {
                out.dynamic.push(Vec::new());
                continue;
            }
            let mesh_id = mesh_identity(batch);
            let always_dynamic = batch.skin.is_some()
                || batch.bounds.len() != batch.instances.len()
                || batch
                    .instances
                    .iter()
                    .any(|instance| instance.source == InstanceSource::Gpu);
            let mut flags = vec![false; batch.instances.len()];
            for (index, instance) in batch.instances.iter().enumerate() {
                let Some(material) = materials.get(instance.material_index as usize) else {
                    continue;
                };
                if !material.casts_shadow() {
                    continue;
                }
                let sphere = batch
                    .bounds
                    .get(index)
                    .copied()
                    .unwrap_or(Sphere::UNBOUNDED);
                let Some(mesh_id) = mesh_id.filter(|_| !always_dynamic) else {
                    flags[index] = true;
                    out.dynamic_spheres.push(sphere);
                    continue;
                };
                let occurrence = occurrences.entry((mesh_id, instance.pick_id)).or_insert(0);
                let key = (mesh_id, instance.pick_id, *occurrence);
                *occurrence += 1;

                let transform = &instance.transform;
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                for value in transform
                    .translation
                    .to_array()
                    .into_iter()
                    .chain(transform.rotation.to_array())
                    .chain(transform.scale.to_array())
                {
                    value.to_bits().hash(&mut hasher);
                }
                (batch.alpha_masked, batch.cull_mode as u8).hash(&mut hasher);
                if batch.alpha_masked {
                    material.hash(&mut hasher);
                }
                let content = hasher.finish();

                let tracked = self.casters.entry(key).or_insert_with(|| Tracked {
                    content,
                    sphere,
                    // A caster that appears counts as static at once (most do not move).
                    changed: None,
                    seen: frame,
                    settled: false,
                });
                tracked.seen = frame;
                if tracked.content != content {
                    if tracked.settled {
                        // It leaves the static maps where it was.
                        out.dirty.push(tracked.sphere);
                    }
                    tracked.content = content;
                    tracked.sphere = sphere;
                    tracked.changed = Some(frame);
                }
                let settled = tracked
                    .changed
                    .is_none_or(|changed| frame - changed >= SETTLE_FRAMES);
                if settled && !tracked.settled {
                    // It joins the static maps where it now is.
                    out.dirty.push(tracked.sphere);
                }
                tracked.settled = settled;
                if !settled {
                    flags[index] = true;
                    out.dynamic_spheres.push(sphere);
                }
            }
            out.dynamic.push(flags);
        }

        // Casters gone since last frame leave the static maps.
        self.casters.retain(|_, tracked| {
            let present = tracked.seen == frame;
            if !present && tracked.settled {
                out.dirty.push(tracked.sphere);
            }
            present
        });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{Assets, Handle, MaterialAsset};
    use crate::renderer::batch::{CullMode, RenderBatcher, RenderObject};
    use crate::renderer::material::Material;
    use crate::scene::components::DepthState;
    use crate::scene::transform::Transform;
    use glam::Vec3;
    use std::path::PathBuf;

    fn prepared(positions: &[(u64, Vec3)]) -> PreparedBatches {
        let mut batcher = RenderBatcher::new();
        let mut assets = Assets::new();
        let material = Material::white();
        let handle = assets
            .materials
            .insert(MaterialAsset::from_material(material, PathBuf::new()));
        for (pick_id, position) in positions {
            batcher
                .add(
                    RenderObject {
                        mesh: Handle::new(1),
                        material: handle,
                        resolved_material: Some(material),
                        transform: Transform::from_translation(*position),
                        depth_state: DepthState::default(),
                        force_overlay: false,
                        render_pass: None,
                        instance_source: InstanceSource::Cpu,
                        gpu_index: None,
                        cull_mode: CullMode::Back,
                        pick_id: *pick_id,
                        skin: None,
                    },
                    &assets,
                )
                .unwrap();
        }
        let bounds = crate::asset::LocalBounds {
            min: Vec3::splat(-0.5),
            max: Vec3::splat(0.5),
        };
        PreparedBatches::from_batcher(&batcher, Vec3::ZERO, &|_| Some(bounds), &|_, _| None)
    }

    fn update(tracker: &mut CasterTracker, positions: &[(u64, Vec3)]) -> CasterFrame {
        tracker.update(
            &prepared(positions),
            |batch| Some(batch.mesh.index() as u64),
            0,
        )
    }

    #[test]
    fn still_casters_change_nothing_and_moving_ones_turn_dynamic() {
        let mut tracker = CasterTracker::default();
        let a = (1, Vec3::ZERO);
        let b = (2, Vec3::new(5.0, 0.0, 0.0));
        let first = update(&mut tracker, &[a, b]);
        assert_eq!(first.dirty.len(), 2, "new casters join the static maps");
        assert!(first.all_dirty, "the first frame has no maps yet");
        let still = update(&mut tracker, &[a, b]);
        assert!(still.dirty.is_empty() && !still.all_dirty);
        assert!(still.dynamic_spheres.is_empty());

        // b moves: it leaves the static maps (where it was) and draws as dynamic.
        let moved = (2, Vec3::new(6.0, 0.0, 0.0));
        let frame = update(&mut tracker, &[a, moved]);
        assert_eq!(frame.dirty.len(), 1);
        assert!((frame.dirty[0].center - b.1).length() < 1e-4);
        assert_eq!(frame.dynamic_spheres.len(), 1);
        let dynamic: Vec<bool> = (0..2).map(|i| frame.is_dynamic(0, i)).collect();
        assert_eq!(dynamic.iter().filter(|d| **d).count(), 1);

        // Once it has kept still long enough it joins the static maps again.
        let mut joined = 0;
        for _ in 0..SETTLE_FRAMES + 1 {
            joined += update(&mut tracker, &[a, moved]).dirty.len();
        }
        assert_eq!(joined, 1);
        assert!(update(&mut tracker, &[a, moved]).dynamic_spheres.is_empty());

        // A caster that goes leaves them.
        let gone = update(&mut tracker, &[a]);
        assert_eq!(gone.dirty.len(), 1);
    }
}
