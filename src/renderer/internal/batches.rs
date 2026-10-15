use std::{cmp::Ordering, ops::Range};

use crate::asset::{Handle, MaterialAsset, Mesh};
use crate::renderer::batch::{
    CullMode, InstanceData, InstanceSource, RenderBatcher, RenderPass, SkinKey,
};
use crate::renderer::internal::MaterialPipelineKey;
use crate::renderer::material::Material;
use crate::renderer::shader_builder::SamplerFilterMode;
use crate::scene::components::DepthState;
use glam::Vec3;

#[derive(Debug, Clone)]
pub(crate) struct OrderedBatch {
    pub mesh: Handle<Mesh>,
    pub skin: Option<SkinKey>,
    pub pass: RenderPass,
    pub depth_state: DepthState,
    pub instances: Vec<InstanceData>,
    pub alpha_blend: bool,
    pub first_instance: u32,
    pub sampler_filtering: SamplerFilterMode,
    pub cull_mode: CullMode,
    /// The depth prepass drew it this frame, so the main pass keeps that depth.
    pub prepassed: bool,
    /// Its materials cut the surface away by alpha (`Material::is_alpha_masked`).
    pub alpha_masked: bool,
}

pub(crate) struct PreparedBatches {
    pub batches: Vec<OrderedBatch>,
    pub opaque_range: Range<usize>,
    pub transparent_range: Range<usize>,
    pub overlay_range: Range<usize>,
    pub gizmo_range: Range<usize>,
    pub gizmo_solid_range: Range<usize>,
    pub materials: Vec<Material>,
    pub material_handles: Vec<Handle<MaterialAsset>>,
    pub material_pipeline_keys: Vec<MaterialPipelineKey>,
}

impl PreparedBatches {
    pub(crate) fn from_batcher(batcher: &RenderBatcher, camera_pos: Vec3) -> Self {
        let mut opaque = Vec::new();
        let mut transparent = Vec::new();
        let mut overlay = Vec::new();
        let mut gizmos = Vec::new();
        let mut gizmo_solids = Vec::new();
        // Transparent instances from every batch, with their batch (now without
        // instances) as `transparent_templates[template]`.
        let mut transparent_instances: Vec<(f32, usize, InstanceData)> = Vec::new();
        let mut transparent_templates: Vec<OrderedBatch> = Vec::new();
        let materials = batcher.materials();
        let material_handles = batcher.material_handles();
        let material_pipeline_keys = batcher.material_pipeline_keys();

        for batch in batcher.iter() {
            if batch.instances.is_empty() {
                continue;
            }

            let mut instances = batch.instances.to_vec();

            if batch.pass.requires_back_to_front_sort() {
                sort_instances_back_to_front(&mut instances, camera_pos);
            }
            optimize_instance_order(batch.pass, &mut instances);

            let alpha_blend = batch.pass.uses_alpha_blending()
                || instances.iter().any(|inst| {
                    materials
                        .get(inst.material_index as usize)
                        .map(|mat| mat.requires_separate_pass())
                        .unwrap_or(false)
                });

            let mut depth_state = batch.depth_state;
            if alpha_blend {
                // Keep depth testing but avoid writing so blended geometry layers correctly.
                depth_state.depth_write = false;
            }

            let mut ordered = OrderedBatch {
                mesh: batch.mesh,
                skin: batch.skin,
                pass: batch.pass,
                depth_state,
                instances,
                alpha_blend,
                first_instance: 0,
                sampler_filtering: if batch.use_nearest_filtering {
                    SamplerFilterMode::Nearest
                } else {
                    SamplerFilterMode::Linear
                },
                cull_mode: batch.cull_mode,
                prepassed: false,
                alpha_masked: batch.alpha_masked,
            };

            if ordered
                .instances
                .iter()
                .all(|inst| inst.source == InstanceSource::Gpu)
            {
                if let Some(first_gpu) = ordered.instances.first().and_then(|inst| inst.gpu_index) {
                    ordered.first_instance = first_gpu;
                }
            }

            match ordered.pass {
                RenderPass::Opaque => opaque.push(ordered),
                // Sorted with every other transparent instance below; GPU-written
                // instances keep their batch (their slots are fixed and contiguous).
                RenderPass::Transparent
                    if ordered
                        .instances
                        .iter()
                        .all(|inst| inst.source == InstanceSource::Cpu) =>
                {
                    let template = transparent_templates.len();
                    for instance in ordered.instances.drain(..) {
                        let distance =
                            (instance.transform.translation - camera_pos).length_squared();
                        transparent_instances.push((distance, template, instance));
                    }
                    transparent_templates.push(ordered);
                }
                RenderPass::Transparent => transparent.push(ordered),
                RenderPass::Overlay => overlay.push(ordered),
                RenderPass::Gizmo => gizmos.push(ordered),
                RenderPass::GizmoSolid => gizmo_solids.push(ordered),
            }
        }

        transparent.extend(sort_transparent_back_to_front(
            transparent_instances,
            &transparent_templates,
        ));
        sort_batches_back_to_front(&mut transparent, camera_pos);
        sort_batches_back_to_front(&mut overlay, camera_pos);
        sort_batches_back_to_front(&mut gizmos, camera_pos);
        sort_batches_back_to_front(&mut gizmo_solids, camera_pos);

        let mut batches = Vec::with_capacity(
            opaque.len() + transparent.len() + overlay.len() + gizmos.len() + gizmo_solids.len(),
        );
        let opaque_range = append_batches(&mut batches, opaque);
        let transparent_range = append_batches(&mut batches, transparent);
        let overlay_range = append_batches(&mut batches, overlay);
        let gizmo_range = append_batches(&mut batches, gizmos);
        let gizmo_solid_range = append_batches(&mut batches, gizmo_solids);

        let gpu_ranges = collect_gpu_ranges(&batches);
        let mut cpu_cursor = 0u32;
        let mut next_gpu_range = 0usize;

        for batch in &mut batches {
            if batch
                .instances
                .iter()
                .all(|inst| inst.source == InstanceSource::Gpu)
            {
                if let Some(first_gpu) = batch.instances.first().and_then(|inst| inst.gpu_index) {
                    batch.first_instance = first_gpu;
                    continue;
                }
            }

            let instance_count = batch.instances.len() as u32;
            let start = allocate_cpu_range(
                &mut cpu_cursor,
                instance_count,
                &gpu_ranges,
                &mut next_gpu_range,
            );
            batch.first_instance = start;
        }

        Self {
            batches,
            opaque_range,
            transparent_range,
            overlay_range,
            gizmo_range,
            gizmo_solid_range,
            materials: materials.to_vec(),
            material_handles: material_handles.to_vec(),
            material_pipeline_keys: material_pipeline_keys.to_vec(),
        }
    }

    pub(crate) fn all(&self) -> &[OrderedBatch] {
        &self.batches
    }

    pub(crate) fn opaque(&self) -> &[OrderedBatch] {
        &self.batches[self.opaque_range.clone()]
    }

    pub(crate) fn opaque_mut(&mut self) -> &mut [OrderedBatch] {
        let range = self.opaque_range.clone();
        &mut self.batches[range]
    }

    pub(crate) fn transparent(&self) -> &[OrderedBatch] {
        &self.batches[self.transparent_range.clone()]
    }

    pub(crate) fn overlay(&self) -> &[OrderedBatch] {
        &self.batches[self.overlay_range.clone()]
    }

    pub(crate) fn gizmos(&self) -> &[OrderedBatch] {
        &self.batches[self.gizmo_range.clone()]
    }

    pub(crate) fn gizmo_solids(&self) -> &[OrderedBatch] {
        &self.batches[self.gizmo_solid_range.clone()]
    }

    pub(crate) fn materials(&self) -> &[Material] {
        &self.materials
    }

    pub(crate) fn material_handles(&self) -> &[Handle<MaterialAsset>] {
        &self.material_handles
    }

    pub(crate) fn material_pipeline_keys(&self) -> &[MaterialPipelineKey] {
        &self.material_pipeline_keys
    }
}

fn sort_instances_back_to_front(instances: &mut [InstanceData], camera_pos: Vec3) {
    instances.sort_unstable_by(|a, b| {
        let da = (a.transform.translation - camera_pos).length_squared();
        let db = (b.transform.translation - camera_pos).length_squared();
        db.partial_cmp(&da).unwrap_or(Ordering::Equal)
    });
}

/// Blending needs the farthest surface drawn first across all transparent objects, not
/// only within a mesh's batch: a batch whose instances straddle another object would
/// otherwise draw its near instances behind it. Instances are ordered by distance, and
/// consecutive ones from the same batch share a draw (`templates[i]` with its instances).
fn sort_transparent_back_to_front(
    mut instances: Vec<(f32, usize, InstanceData)>,
    templates: &[OrderedBatch],
) -> Vec<OrderedBatch> {
    instances.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(Ordering::Equal));
    let mut runs: Vec<(usize, OrderedBatch)> = Vec::new();
    for (_, template, instance) in instances {
        match runs.last_mut() {
            Some((last, batch)) if *last == template => batch.instances.push(instance),
            _ => {
                let mut batch = templates[template].clone();
                batch.instances.push(instance);
                runs.push((template, batch));
            }
        }
    }
    runs.into_iter().map(|(_, batch)| batch).collect()
}

fn sort_batches_back_to_front(batches: &mut [OrderedBatch], camera_pos: Vec3) {
    // Stable: runs from `sort_transparent_back_to_front` at equal distances keep order.
    batches.sort_by(|a, b| {
        farthest_distance_sq(b, camera_pos)
            .partial_cmp(&farthest_distance_sq(a, camera_pos))
            .unwrap_or(Ordering::Equal)
    });
}

fn farthest_distance_sq(batch: &OrderedBatch, camera_pos: Vec3) -> f32 {
    batch
        .instances
        .iter()
        .map(|inst| (inst.transform.translation - camera_pos).length_squared())
        .fold(0.0, f32::max)
}

fn append_batches(dest: &mut Vec<OrderedBatch>, src: Vec<OrderedBatch>) -> Range<usize> {
    let start = dest.len();
    dest.extend(src);
    start..dest.len()
}

fn optimize_instance_order(pass: RenderPass, instances: &mut [InstanceData]) {
    if instances.len() <= 1 {
        return;
    }

    if instances
        .iter()
        .all(|inst| inst.source == InstanceSource::Gpu)
    {
        instances.sort_unstable_by_key(|inst| inst.gpu_index.unwrap_or(u32::MAX));
        return;
    }

    if matches!(pass, RenderPass::Opaque) {
        instances.sort_unstable_by_key(|inst| inst.material_index);
    }
}

fn collect_gpu_ranges(batches: &[OrderedBatch]) -> Vec<Range<u32>> {
    let mut ranges: Vec<Range<u32>> = Vec::new();

    for batch in batches {
        if !batch
            .instances
            .iter()
            .any(|inst| inst.source == InstanceSource::Gpu)
        {
            continue;
        }

        let mut current: Option<Range<u32>> = None;
        for inst in batch
            .instances
            .iter()
            .filter(|inst| inst.source == InstanceSource::Gpu)
            .filter_map(|inst| inst.gpu_index)
        {
            match current.as_mut() {
                Some(range) if inst == range.end => {
                    range.end += 1;
                }
                Some(range) if inst < range.end => {
                    // Overlapping or duplicate index, extend to cover it.
                    range.end = range.end.max(inst + 1);
                }
                Some(range) => {
                    ranges.push(range.clone());
                    *range = inst..inst + 1;
                }
                None => {
                    current = Some(inst..inst + 1);
                }
            }
        }

        if let Some(range) = current.take() {
            ranges.push(range);
        }
    }

    ranges.sort_unstable_by_key(|range| range.start);

    let mut merged: Vec<Range<u32>> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(last) = merged.last_mut() {
            if range.start <= last.end {
                last.end = last.end.max(range.end);
                continue;
            }
        }
        merged.push(range);
    }

    merged
}

fn allocate_cpu_range(
    cursor: &mut u32,
    length: u32,
    gpu_ranges: &[Range<u32>],
    next_gpu_range: &mut usize,
) -> u32 {
    loop {
        if let Some(range) = gpu_ranges.get(*next_gpu_range) {
            if cursor.saturating_add(length) <= range.start {
                let start = *cursor;
                *cursor = cursor.saturating_add(length);
                return start;
            }

            if *cursor >= range.end {
                *next_gpu_range += 1;
                continue;
            }

            let attempted_start = *cursor;
            let attempted_end = attempted_start.saturating_add(length);
            log::trace!(
                "CPU instance range [{}..{}) overlaps GPU reservation [{}..{}); skipping to {}",
                attempted_start,
                attempted_end,
                range.start,
                range.end,
                range.end
            );
            *cursor = range.end;
            *next_gpu_range += 1;
            continue;
        }

        let start = *cursor;
        *cursor = cursor.saturating_add(length);
        return start;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset::{Assets, Handle, MaterialAsset};
    use crate::renderer::batch::{CullMode, InstanceSource, RenderObject};
    use crate::renderer::material::Material;
    use crate::scene::components::DepthState;
    use crate::scene::transform::Transform;
    use glam::Vec3;
    use std::collections::BTreeSet;
    use std::path::PathBuf;

    #[test]
    fn empty_batches_are_skipped() {
        let mut batcher = RenderBatcher::new();
        let mut assets = Assets::new();
        let material = Material::white();
        let handle = assets
            .materials
            .insert(MaterialAsset::from_material(material, PathBuf::new()));

        batcher
            .add(
                RenderObject {
                    mesh: Handle::new(0),
                    material: handle,
                    resolved_material: Some(material),
                    transform: Transform::IDENTITY,
                    depth_state: DepthState::default(),
                    force_overlay: false,
                    render_pass: None,
                    instance_source: InstanceSource::Cpu,
                    gpu_index: None,
                    cull_mode: CullMode::Back,
                    pick_id: 0,
                    skin: None,
                },
                &assets,
            )
            .unwrap();

        batcher.clear();

        let prepared = PreparedBatches::from_batcher(&batcher, Vec3::ZERO);

        assert!(
            prepared.all().is_empty(),
            "empty batch entries should not produce draw calls"
        );
    }

    #[test]
    fn transparent_instances_interleave_back_to_front_across_batches() {
        let mut batcher = RenderBatcher::new();
        let mut assets = Assets::new();
        let material = Material::white().with_alpha();
        let handle = assets
            .materials
            .insert(MaterialAsset::from_material(material, PathBuf::new()));
        // Mesh 1 has instances behind and in front of mesh 2's.
        for (mesh, z, pick_id) in [(1, -10.0, 1), (1, -1.0, 2), (2, -5.0, 3)] {
            batcher
                .add(
                    RenderObject {
                        mesh: Handle::new(mesh),
                        material: handle,
                        resolved_material: Some(material),
                        transform: Transform::from_translation(Vec3::new(0.0, 0.0, z)),
                        depth_state: DepthState::default(),
                        force_overlay: false,
                        render_pass: None,
                        instance_source: InstanceSource::Cpu,
                        gpu_index: None,
                        cull_mode: CullMode::Back,
                        pick_id,
                        skin: None,
                    },
                    &assets,
                )
                .unwrap();
        }

        let prepared = PreparedBatches::from_batcher(&batcher, Vec3::ZERO);
        let order: Vec<(usize, Vec<u64>)> = prepared
            .transparent()
            .iter()
            .map(|batch| {
                (
                    batch.mesh.index(),
                    batch.instances.iter().map(|inst| inst.pick_id).collect(),
                )
            })
            .collect();
        assert_eq!(order, vec![(1, vec![1]), (2, vec![3]), (1, vec![2])]);
        // Each draw has its own slots in the objects buffer.
        let starts: BTreeSet<u32> = prepared
            .transparent()
            .iter()
            .map(|batch| batch.first_instance)
            .collect();
        assert_eq!(starts.len(), 3);
    }

    #[test]
    fn cpu_and_gpu_batches_do_not_overlap_reserved_ranges() {
        let mut batcher = RenderBatcher::new();
        let mut assets = Assets::new();
        let mesh = Handle::new(42);
        let material = Material::white();
        let handle = assets
            .materials
            .insert(MaterialAsset::from_material(material, PathBuf::new()));

        // CPU opaque batch that should be placed after GPU indices [0, 2).
        for pick_id in 0..3 {
            batcher
                .add(
                    RenderObject {
                        mesh,
                        material: handle,
                        resolved_material: Some(material),
                        transform: Transform::IDENTITY,
                        depth_state: DepthState::default(),
                        force_overlay: false,
                        render_pass: Some(RenderPass::Opaque),
                        instance_source: InstanceSource::Cpu,
                        gpu_index: None,
                        cull_mode: CullMode::Back,
                        pick_id: pick_id as u64,
                        skin: None,
                    },
                    &assets,
                )
                .unwrap();
        }

        // GPU transparent batch reserving indices [0, 2).
        for (i, gpu_index) in (0..2).enumerate() {
            batcher
                .add(
                    RenderObject {
                        mesh,
                        material: handle,
                        resolved_material: Some(material),
                        transform: Transform::IDENTITY,
                        depth_state: DepthState::default(),
                        force_overlay: false,
                        render_pass: Some(RenderPass::Transparent),
                        instance_source: InstanceSource::Gpu,
                        gpu_index: Some(gpu_index),
                        cull_mode: CullMode::Back,
                        pick_id: 100 + i as u64,
                        skin: None,
                    },
                    &assets,
                )
                .unwrap();
        }

        // GPU gizmo batch reserving indices [5, 8).
        for (i, gpu_index) in (5..8).enumerate() {
            batcher
                .add(
                    RenderObject {
                        mesh,
                        material: handle,
                        resolved_material: Some(material),
                        transform: Transform::IDENTITY,
                        depth_state: DepthState::default(),
                        force_overlay: false,
                        render_pass: Some(RenderPass::Gizmo),
                        instance_source: InstanceSource::Gpu,
                        gpu_index: Some(gpu_index),
                        cull_mode: CullMode::Back,
                        pick_id: 200 + i as u64,
                        skin: None,
                    },
                    &assets,
                )
                .unwrap();
        }

        // CPU overlay batch that should skip over the [5, 8) GPU reservation.
        for pick_id in 300..302 {
            batcher
                .add(
                    RenderObject {
                        mesh,
                        material: handle,
                        resolved_material: Some(material),
                        transform: Transform::IDENTITY,
                        depth_state: DepthState::default(),
                        force_overlay: false,
                        render_pass: Some(RenderPass::Overlay),
                        instance_source: InstanceSource::Cpu,
                        gpu_index: None,
                        cull_mode: CullMode::Back,
                        pick_id: pick_id as u64,
                        skin: None,
                    },
                    &assets,
                )
                .unwrap();
        }

        // CPU gizmo solid batch to verify ranges after the final GPU segment.
        batcher
            .add(
                RenderObject {
                    mesh,
                    material: handle,
                    resolved_material: Some(material),
                    transform: Transform::IDENTITY,
                    depth_state: DepthState::default(),
                    force_overlay: false,
                    render_pass: Some(RenderPass::GizmoSolid),
                    instance_source: InstanceSource::Cpu,
                    gpu_index: None,
                    cull_mode: CullMode::Back,
                    pick_id: 999u64,
                    skin: None,
                },
                &assets,
            )
            .unwrap();

        let prepared = PreparedBatches::from_batcher(&batcher, Vec3::ZERO);

        let mut gpu_indices = BTreeSet::new();
        for batch in prepared.all() {
            for inst in &batch.instances {
                if let Some(idx) = inst.gpu_index {
                    gpu_indices.insert(idx);
                }
            }
        }

        let expected: BTreeSet<_> = [0, 1, 5, 6, 7].into_iter().collect();
        assert_eq!(gpu_indices, expected);

        let cpu_batches: Vec<_> = prepared
            .all()
            .iter()
            .filter(|batch| {
                batch
                    .instances
                    .iter()
                    .all(|inst| inst.source == InstanceSource::Cpu)
            })
            .collect();
        assert_eq!(cpu_batches.len(), 3);

        let mut cpu_starts: Vec<u32> = cpu_batches
            .iter()
            .map(|batch| batch.first_instance)
            .collect();
        cpu_starts.sort_unstable();
        assert_eq!(cpu_starts, vec![2, 8, 10]);

        for batch in cpu_batches {
            let start = batch.first_instance;
            let end = start + batch.instances.len() as u32;
            for idx in start..end {
                assert!(
                    !gpu_indices.contains(&idx),
                    "CPU draw range [{}..{}) overlaps GPU index {}",
                    start,
                    end,
                    idx
                );
            }
        }
    }
}
