use super::animations;
use super::SceneLoader;
use crate::scene::animation::{
    AnimationInterpolation, AnimationOutput, AnimationTarget, TransformProperty,
};
use crate::scene::components::{Name, TransformComponent, Visible};
use crate::scene::{Scene, Transform};
use glam::Vec3;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

#[test]
fn pointer_animation_gltf_is_patched_and_loaded() {
    let path = Path::new("web/assets/animated/AnimatedColorsCube.gltf");

    let standard_import = gltf::import(path);
    assert!(matches!(standard_import, Err(gltf::Error::Deserialize(_))));

    let (document, _, _) = SceneLoader::import_gltf_native(path).expect("patched import");
    assert_eq!(document.animations().len(), 1);

    let original_nodes: Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let original_node_count = original_nodes
        .get("nodes")
        .and_then(|value| value.as_array())
        .map(|nodes| nodes.len())
        .unwrap_or(0);

    let animation = document.animations().next().unwrap();
    let pointer_channel = animation.channels().nth(2).unwrap();
    assert!(animations::is_pointer_channel(&document, 0, 2));
    assert_eq!(pointer_channel.target().node().index(), original_node_count);
}

#[test]
fn translation_animation_channels_match_document() {
    let path = Path::new("web/assets/animated/InterpolationTest.gltf");

    let (document, buffers, _) =
        SceneLoader::import_gltf_native(path).expect("InterpolationTest import");

    let mut scene = Scene::new();
    let mut node_entities = vec![None; document.nodes().len()];
    for node in document.nodes() {
        let entity = scene.world_mut().spawn((
            Name::new(node.name().unwrap_or("")),
            TransformComponent(Transform::IDENTITY),
            Visible(true),
        ));
        node_entities[node.index()] = Some(entity);
    }

    let clips = animations::load_animation_clips(1.0, path, &document, &buffers, &node_entities)
        .expect("load animation clips");
    assert!(!clips.is_empty());

    let document_animations: Vec<_> = document.animations().collect();

    for clip_name in [
        "Step Translation",
        "CubicSpline Translation",
        "Linear Translation",
    ] {
        let clip = clips
            .iter()
            .find(|clip| clip.name == clip_name)
            .unwrap_or_else(|| panic!("Clip '{}' not loaded", clip_name));

        let channel = clip
            .channels
            .iter()
            .find(|channel| {
                matches!(
                    channel.target,
                    AnimationTarget::Transform {
                        property: TransformProperty::Translation,
                        ..
                    }
                )
            })
            .unwrap_or_else(|| panic!("Clip '{}' missing translation channel", clip_name));

        let animation = document_animations
            .iter()
            .find(|animation| animation.name() == Some(clip_name))
            .unwrap_or_else(|| panic!("Missing animation '{}'", clip_name));

        let doc_channel = animation
            .channels()
            .next()
            .expect("Animation should have a channel");
        let reader = doc_channel.reader(|buffer| Some(&buffers[buffer.index()].0));

        let doc_times: Vec<f32> = reader.read_inputs().expect("animation inputs").collect();
        let doc_values: Vec<Vec3> = match reader.read_outputs().expect("animation outputs") {
            gltf::animation::util::ReadOutputs::Translations(iter) => {
                iter.map(Vec3::from).collect()
            }
            _ => panic!("Unexpected outputs for {}", clip_name),
        };

        assert_eq!(channel.sampler.times, doc_times);

        match &channel.sampler.output {
            AnimationOutput::Vec3(values) => {
                assert_eq!(values.len(), doc_values.len());

                for (actual, expected) in values.iter().zip(doc_values.iter()) {
                    assert!(actual.abs_diff_eq(*expected, 1e-6));
                }
            }
            _ => panic!("Clip '{}' should produce Vec3 outputs", clip_name),
        }

        let final_time = *channel
            .sampler
            .times
            .last()
            .expect("Keyframe times should not be empty");

        let mut transform_updates = HashMap::new();
        let mut material_updates = HashMap::new();
        clip.sample(final_time, &mut transform_updates, &mut material_updates);

        let (entity, _) = match channel.target {
            AnimationTarget::Transform { entity, property } => (entity, property),
            _ => unreachable!(),
        };

        let update = transform_updates
            .get(&entity)
            .unwrap_or_else(|| panic!("No transform update for '{}'", clip_name));

        let expected_final = match channel.sampler.interpolation {
            AnimationInterpolation::CubicSpline => {
                let keyframe_index = channel.sampler.times.len() - 1;
                doc_values[keyframe_index * 3 + 1]
            }
            AnimationInterpolation::Linear | AnimationInterpolation::Step => doc_values
                .last()
                .copied()
                .expect("Translation outputs should not be empty"),
        };

        assert!(update
            .translation
            .expect("Translation update missing")
            .abs_diff_eq(expected_final, 1e-5));
    }
}
