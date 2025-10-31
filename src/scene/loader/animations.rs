use std::collections::HashMap;
use std::path::Path;

use glam::{Quat, Vec3, Vec4};
use gltf::json::validation::Checked;
use serde_json::Value;

use super::GltfImport;
use crate::scene::animation::{
    AnimationChannel, AnimationClip, AnimationInterpolation, AnimationOutput, AnimationSampler,
    AnimationTarget, MaterialProperty, TransformProperty,
};

#[derive(Debug, Clone, Copy)]
struct MaterialPointerTarget {
    material_index: usize,
    property: MaterialProperty,
}

pub(super) fn load_animation_clips(
    scale: f32,
    source_path: &Path,
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    node_entities: &[Option<hecs::Entity>],
) -> Result<Vec<AnimationClip>, String> {
    if document.animations().len() == 0 {
        log::info!("No animations in glTF document");
        return Ok(Vec::new());
    }

    let pointer_targets = extract_pointer_targets(document, Some(source_path));
    let mut clips = Vec::new();
    let mut loaded_clip_count = 0usize;

    for (animation_index, animation) in document.animations().enumerate() {
        let clip_name = animation
            .name()
            .map(|name| name.to_string())
            .unwrap_or_else(|| format!("Animation_{}", animation_index));
        let mut clip = AnimationClip::new(clip_name.clone());
        let mut supported_channels = 0usize;

        for (channel_index, channel) in animation.channels().enumerate() {
            let reader = channel.reader(|buffer| Some(&buffers[buffer.index()].0));

            let Some(inputs) = reader.read_inputs() else {
                log::warn!(
                    "Animation '{}' channel {} is missing input keyframes",
                    clip_name,
                    channel_index
                );
                continue;
            };

            let mut times: Vec<f32> = inputs.collect();
            if times.is_empty() {
                continue;
            }

            let interpolation = match channel.sampler().interpolation() {
                gltf::animation::Interpolation::Step => AnimationInterpolation::Step,
                gltf::animation::Interpolation::Linear => AnimationInterpolation::Linear,
                gltf::animation::Interpolation::CubicSpline => AnimationInterpolation::CubicSpline,
            };

            if is_pointer_channel(document, animation_index, channel_index) {
                let Some(pointer_target) = pointer_targets.get(&(animation_index, channel_index))
                else {
                    log::warn!(
                        "Animation '{}' channel {} uses unsupported pointer target",
                        clip_name,
                        channel_index
                    );
                    continue;
                };

                let output_accessor = channel.sampler().output();
                let mut values = match read_vec4_outputs(&output_accessor, buffers) {
                    Ok(values) => values,
                    Err(err) => {
                        log::warn!(
                            "Failed to read pointer animation data for '{}' channel {}: {}",
                            clip_name,
                            channel_index,
                            err
                        );
                        continue;
                    }
                };

                if values.is_empty() {
                    continue;
                }

                if values.len() != times.len() {
                    let min_len = times.len().min(values.len());
                    log::warn!(
                        "Pointer animation '{}' channel {} has {} inputs but {} outputs - truncating",
                        clip_name,
                        channel_index,
                        times.len(),
                        values.len()
                    );
                    times.truncate(min_len);
                    values.truncate(min_len);
                }

                if times.is_empty() || values.is_empty() {
                    continue;
                }

                let sampler = AnimationSampler {
                    times,
                    output: AnimationOutput::Vec4(values),
                    interpolation,
                };

                clip.add_channel(AnimationChannel {
                    sampler,
                    target: AnimationTarget::Material {
                        material_index: pointer_target.material_index,
                        property: pointer_target.property,
                    },
                });

                supported_channels += 1;
                continue;
            }

            let target_node = channel.target().node();
            let Some(entity) = node_entities
                .get(target_node.index())
                .and_then(|entry| *entry)
            else {
                log::warn!(
                    "Animation '{}' channel {} references node {} that was not instantiated",
                    clip_name,
                    channel_index,
                    target_node.index()
                );
                continue;
            };

            let property = channel.target().property();
            let output = match property {
                gltf::animation::Property::Translation => match reader.read_outputs() {
                    Some(gltf::animation::util::ReadOutputs::Translations(iter)) => {
                        let mut values: Vec<Vec3> = iter.map(Vec3::from).collect();

                        if !reconcile_keyframe_lengths(
                            &mut times,
                            &mut values,
                            interpolation,
                            &clip_name,
                            channel_index,
                            "Translation",
                        ) {
                            continue;
                        }

                        if scale != 1.0 {
                            for value in &mut values {
                                *value *= scale;
                            }
                        }

                        AnimationOutput::Vec3(values)
                    }
                    _ => {
                        log::warn!(
                            "Unexpected translation outputs for animation '{}' channel {}",
                            clip_name,
                            channel_index
                        );
                        continue;
                    }
                },
                gltf::animation::Property::Scale => match reader.read_outputs() {
                    Some(gltf::animation::util::ReadOutputs::Scales(iter)) => {
                        let mut values: Vec<Vec3> = iter.map(Vec3::from).collect();

                        if !reconcile_keyframe_lengths(
                            &mut times,
                            &mut values,
                            interpolation,
                            &clip_name,
                            channel_index,
                            "Scale",
                        ) {
                            continue;
                        }

                        AnimationOutput::Vec3(values)
                    }
                    _ => {
                        log::warn!(
                            "Unexpected scale outputs for animation '{}' channel {}",
                            clip_name,
                            channel_index
                        );
                        continue;
                    }
                },
                gltf::animation::Property::Rotation => match reader.read_outputs() {
                    Some(gltf::animation::util::ReadOutputs::Rotations(rotations)) => {
                        let mut values: Vec<Quat> = rotations
                            .into_f32()
                            .map(|r| Quat::from_xyzw(r[0], r[1], r[2], r[3]))
                            .collect();

                        if !reconcile_keyframe_lengths(
                            &mut times,
                            &mut values,
                            interpolation,
                            &clip_name,
                            channel_index,
                            "Rotation",
                        ) {
                            continue;
                        }

                        AnimationOutput::Quat(values)
                    }
                    _ => {
                        log::warn!(
                            "Unexpected rotation outputs for animation '{}' channel {}",
                            clip_name,
                            channel_index
                        );
                        continue;
                    }
                },
                gltf::animation::Property::MorphTargetWeights => {
                    log::warn!(
                        "Skipping morph target animation '{}' channel {} (not supported)",
                        clip_name,
                        channel_index
                    );
                    continue;
                }
            };

            if times.is_empty() {
                continue;
            }

            let sampler = AnimationSampler {
                times,
                output,
                interpolation,
            };

            let target = match property {
                gltf::animation::Property::Translation => AnimationTarget::Transform {
                    entity,
                    property: TransformProperty::Translation,
                },
                gltf::animation::Property::Rotation => AnimationTarget::Transform {
                    entity,
                    property: TransformProperty::Rotation,
                },
                gltf::animation::Property::Scale => AnimationTarget::Transform {
                    entity,
                    property: TransformProperty::Scale,
                },
                gltf::animation::Property::MorphTargetWeights => unreachable!(),
            };

            clip.add_channel(AnimationChannel { sampler, target });
            supported_channels += 1;
        }

        if supported_channels > 0 {
            loaded_clip_count += 1;
            clips.push(clip);
        } else {
            log::debug!(
                "Skipping animation '{}' because it has no supported channels",
                clip_name
            );
        }
    }

    if loaded_clip_count > 0 {
        log::info!("Loaded {} animation clips", loaded_clip_count);
    } else {
        log::info!("No supported animations were loaded");
    }

    Ok(clips)
}

#[cfg(not(target_arch = "wasm32"))]
pub(super) fn import_gltf_with_pointer_patch(
    path: &Path,
) -> Result<Option<GltfImport>, gltf::Error> {
    use gltf::{import_buffers, import_images};

    let json_text = std::fs::read_to_string(path).map_err(gltf::Error::Io)?;
    let mut root: Value = serde_json::from_str(&json_text).map_err(gltf::Error::Deserialize)?;

    let mut channels_to_patch: Vec<(usize, usize)> = Vec::new();

    if let Some(animations) = root.get("animations").and_then(|value| value.as_array()) {
        for (animation_index, animation) in animations.iter().enumerate() {
            if let Some(channels) = animation.get("channels").and_then(|value| value.as_array()) {
                for (channel_index, channel) in channels.iter().enumerate() {
                    let Some(target_value) = channel.get("target") else {
                        continue;
                    };

                    let Some(target_object) = target_value.as_object() else {
                        continue;
                    };

                    if target_object.contains_key("node") {
                        continue;
                    }

                    let pointer = target_object
                        .get("extensions")
                        .and_then(Value::as_object)
                        .and_then(|extensions| extensions.get("KHR_animation_pointer"))
                        .and_then(Value::as_object)
                        .and_then(|pointer| pointer.get("pointer"))
                        .and_then(Value::as_str);

                    if pointer.is_some() {
                        channels_to_patch.push((animation_index, channel_index));
                    }
                }
            }
        }
    }

    if channels_to_patch.is_empty() {
        return Ok(None);
    }

    let placeholder_index = insert_placeholder_node(&mut root).ok_or_else(|| {
        gltf::Error::Deserialize(serde_json::Error::io(std::io::Error::other(
            "Failed to create placeholder node for pointer animation",
        )))
    })?;

    for (animation_index, channel_index) in channels_to_patch {
        let Some(animation) = root
            .get_mut("animations")
            .and_then(|value| value.as_array_mut())
            .and_then(|animations| animations.get_mut(animation_index))
        else {
            continue;
        };

        let Some(channel) = animation
            .get_mut("channels")
            .and_then(|value| value.as_array_mut())
            .and_then(|channels| channels.get_mut(channel_index))
        else {
            continue;
        };

        let Some(target_value) = channel.get_mut("target") else {
            continue;
        };

        let Some(target_object) = target_value.as_object_mut() else {
            continue;
        };

        target_object.insert(
            "node".to_string(),
            Value::Number(serde_json::Number::from(placeholder_index as u64)),
        );
    }

    let patched_bytes = serde_json::to_vec(&root).map_err(gltf::Error::Deserialize)?;
    let gltf::Gltf { document, blob } = gltf::Gltf::from_slice(&patched_bytes)?;
    let base_dir = path.parent().unwrap_or_else(|| Path::new("./"));
    let buffers = import_buffers(&document, Some(base_dir), blob)?;
    let images = import_images(&document, Some(base_dir), &buffers)?;
    Ok(Some((document, buffers, images)))
}

fn reconcile_keyframe_lengths<T>(
    times: &mut Vec<f32>,
    values: &mut Vec<T>,
    interpolation: AnimationInterpolation,
    clip_name: &str,
    channel_index: usize,
    property_label: &str,
) -> bool {
    if times.is_empty() || values.is_empty() {
        return false;
    }

    let components_per_keyframe = match interpolation {
        AnimationInterpolation::CubicSpline => 3,
        AnimationInterpolation::Step | AnimationInterpolation::Linear => 1,
    };

    if values.len() < components_per_keyframe {
        log::warn!(
            "{} animation '{}' channel {} has insufficient output data ({} values)",
            property_label,
            clip_name,
            channel_index,
            values.len()
        );
        return false;
    }

    if !values.len().is_multiple_of(components_per_keyframe) {
        let valid_values = values.len() / components_per_keyframe * components_per_keyframe;
        log::warn!(
            "{} animation '{}' channel {} outputs ({}) are not a multiple of {} - truncating",
            property_label,
            clip_name,
            channel_index,
            values.len(),
            components_per_keyframe
        );
        values.truncate(valid_values);
    }

    let available_keyframes = values.len() / components_per_keyframe;
    if available_keyframes == 0 {
        return false;
    }

    if available_keyframes != times.len() {
        log::warn!(
            "{} animation '{}' channel {} has {} inputs but {} outputs - truncating",
            property_label,
            clip_name,
            channel_index,
            times.len(),
            available_keyframes
        );
        let min_keyframes = times.len().min(available_keyframes);
        times.truncate(min_keyframes);
        values.truncate(min_keyframes * components_per_keyframe);
    }

    !times.is_empty() && values.len() >= times.len() * components_per_keyframe
}

pub(super) fn is_pointer_channel(
    document: &gltf::Document,
    animation_index: usize,
    channel_index: usize,
) -> bool {
    document
        .as_json()
        .animations
        .get(animation_index)
        .and_then(|anim| anim.channels.get(channel_index))
        .map(|channel| matches!(channel.target.path.as_ref(), Checked::Invalid))
        .unwrap_or(false)
}

fn read_vec4_outputs(
    accessor: &gltf::Accessor,
    buffers: &[gltf::buffer::Data],
) -> Result<Vec<Vec4>, String> {
    let mut values = Vec::new();
    let iter = gltf::accessor::Iter::<[f32; 4]>::new(accessor.clone(), |buffer| {
        Some(&buffers[buffer.index()].0)
    })
    .ok_or_else(|| "Accessor output is not a VEC4 float".to_string())?;

    for value in iter {
        values.push(Vec4::from_array(value));
    }

    Ok(values)
}

fn extract_pointer_targets(
    document: &gltf::Document,
    path: Option<&Path>,
) -> HashMap<(usize, usize), MaterialPointerTarget> {
    let mut targets = HashMap::new();

    if let Ok(root) = gltf::json::serialize::to_value(document.as_json()) {
        collect_pointer_targets_from_json(&root, &mut targets);
    }

    if targets.is_empty() {
        if let Some(path) = path {
            if let Ok(bytes) = crate::io::load_binary(path) {
                if let Ok(root) = serde_json::from_slice::<Value>(&bytes) {
                    collect_pointer_targets_from_json(&root, &mut targets);
                }
            }
        }
    }

    targets
}

fn collect_pointer_targets_from_json(
    root: &Value,
    targets: &mut HashMap<(usize, usize), MaterialPointerTarget>,
) {
    let Some(animations) = root.get("animations").and_then(|value| value.as_array()) else {
        return;
    };

    for (animation_index, animation) in animations.iter().enumerate() {
        let Some(channels) = animation.get("channels").and_then(|value| value.as_array()) else {
            continue;
        };

        for (channel_index, channel) in channels.iter().enumerate() {
            let pointer_value = channel
                .get("target")
                .and_then(|target| target.get("extensions"))
                .and_then(|ext| ext.get("KHR_animation_pointer"))
                .and_then(|pointer| pointer.get("pointer"))
                .and_then(|pointer| pointer.as_str());

            let Some(pointer) = pointer_value else {
                continue;
            };

            if let Some(target) = parse_pointer_target(pointer) {
                targets.insert((animation_index, channel_index), target);
            } else {
                log::warn!(
                    "Unsupported animation pointer path '{}' in animation {} channel {}",
                    pointer,
                    animation_index,
                    channel_index
                );
            }
        }
    }
}

fn parse_pointer_target(pointer: &str) -> Option<MaterialPointerTarget> {
    let mut segments = pointer.split('/').filter(|segment| !segment.is_empty());
    let first = segments.next()?;
    if first != "materials" {
        return None;
    }

    let index_segment = segments.next()?;
    let material_index = index_segment.parse().ok()?;

    let property_group = segments.next()?;
    let property_name = segments.next()?;

    match (property_group, property_name) {
        ("pbrMetallicRoughness", "baseColorFactor") => Some(MaterialPointerTarget {
            material_index,
            property: MaterialProperty::BaseColorFactor,
        }),
        _ => None,
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn insert_placeholder_node(root: &mut Value) -> Option<usize> {
    let root_object = root.as_object_mut()?;
    let nodes_entry = root_object
        .entry("nodes".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));

    let nodes = nodes_entry.as_array_mut()?;
    nodes.push(Value::Object(serde_json::Map::new()));
    Some(nodes.len() - 1)
}
