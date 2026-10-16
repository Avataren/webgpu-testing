//! Skeletal animation clips that target bones by name.
//!
//! The entity-bound [`AnimationClip`](crate::scene::animation::AnimationClip) drives the
//! exact entities it was imported with. A [`SkeletalClip`] instead names the bones it
//! moves, so one walk cycle can drive any character whose skeleton uses the same bone
//! names: the character it came with, a second copy of it, or another character with a
//! compatible rig (Mixamo-style animation packs are separate files applied to many
//! characters).
//!
//! Clips are read from glTF/GLB files, which act as clip libraries: every animation in
//! the file becomes one clip. Only node translation, rotation and scale channels are
//! used; images and meshes are never decoded, so a full character file is a fine
//! library.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{Mat4, Quat, Vec3};

use crate::scene::animation::{AnimationInterpolation, AnimationOutput, AnimationSampler};
use crate::scene::transform::Transform;

/// The keyframes for one bone.
#[derive(Debug, Clone)]
pub struct BoneTrack {
    /// The bone's name in the source file.
    pub bone: String,
    /// [`normalize_bone_name`] of `bone`, used for matching.
    pub key: String,
    pub translation: Option<AnimationSampler>,
    pub rotation: Option<AnimationSampler>,
    pub scale: Option<AnimationSampler>,
}

impl BoneTrack {
    pub fn new(bone: impl Into<String>) -> Self {
        let bone = bone.into();
        Self {
            key: normalize_bone_name(&bone),
            bone,
            translation: None,
            rotation: None,
            scale: None,
        }
    }

    fn end_time(&self) -> f32 {
        [&self.translation, &self.rotation, &self.scale]
            .into_iter()
            .flatten()
            .map(AnimationSampler::end_time)
            .fold(0.0, f32::max)
    }

    /// Sample the track; channels the track does not have come back as `None`.
    pub fn sample(&self, time: f32) -> (Option<Vec3>, Option<Quat>, Option<Vec3>) {
        (
            self.translation.as_ref().and_then(|s| s.sample_vec3(time)),
            self.rotation.as_ref().and_then(|s| s.sample_quat(time)),
            self.scale.as_ref().and_then(|s| s.sample_vec3(time)),
        )
    }
}

/// One animation: keyframe tracks for named bones.
#[derive(Debug, Clone)]
pub struct SkeletalClip {
    pub name: String,
    /// Seconds; the end of the longest track.
    pub duration: f32,
    pub tracks: Vec<BoneTrack>,
}

impl SkeletalClip {
    pub fn new(name: impl Into<String>, tracks: Vec<BoneTrack>) -> Self {
        let duration = tracks.iter().map(BoneTrack::end_time).fold(0.0, f32::max);
        Self {
            name: name.into(),
            duration,
            tracks,
        }
    }
}

/// A bone's rest (bind-time) local transform in a library's source file, and how deep
/// it sits in that file's hierarchy (0 = a root node).
#[derive(Debug, Clone, Copy)]
pub struct BoneRest {
    pub transform: Transform,
    pub depth: u32,
    /// Rotation of the node's parents in the file (scene root to parent), the frame its
    /// keys are in.
    pub parent_rotation: Quat,
    /// Where the node rests in the file's space.
    pub position: Vec3,
}

/// Which way a skeleton faces at rest (a horizontal unit vector, Y up), from where its
/// left and right legs (or shoulders, arms, feet) sit; `position` gives a bone's rest
/// position by normalized name. `None` without such a pair.
pub fn skeleton_facing(position: impl Fn(&str) -> Option<Vec3>) -> Option<Vec3> {
    const PAIRS: [(&str, &str); 8] = [
        ("leftupleg", "rightupleg"),
        ("leftthigh", "rightthigh"),
        ("thigh.l", "thigh.r"),
        ("thigh_l", "thigh_r"),
        ("leftshoulder", "rightshoulder"),
        ("leftarm", "rightarm"),
        ("upperarm.l", "upperarm.r"),
        ("leftfoot", "rightfoot"),
    ];
    PAIRS.iter().find_map(|(left, right)| {
        let lateral = position(left)? - position(right)?;
        // Facing +Z with Y up, the left side is +X.
        let forward = lateral.cross(Vec3::Y);
        let forward = Vec3::new(forward.x, 0.0, forward.z);
        (forward.length() > 1e-5).then(|| forward.normalize())
    })
}

/// The clips of one glTF/GLB file, and the rest pose of that file's nodes (used to
/// retarget translations to skeletons of other proportions).
#[derive(Debug, Clone, Default)]
pub struct ClipLibrary {
    pub source: PathBuf,
    pub clips: Vec<Arc<SkeletalClip>>,
    /// Rest transforms by normalized bone name.
    pub rest: HashMap<String, BoneRest>,
}

impl ClipLibrary {
    /// Read the clips of a glTF or GLB file (relative paths resolve against the open
    /// project). Works natively and on the web.
    pub fn load(path: &Path) -> Result<Self, String> {
        let resolved = crate::project::resolve_project_path(path);
        let bytes = crate::io::load_binary(&resolved)
            .map_err(|err| format!("cannot read {}: {err}", resolved.display()))?;
        let gltf = gltf::Gltf::from_slice_without_validation(&bytes)
            .map_err(|err| format!("{} is not valid glTF: {err}", resolved.display()))?;
        let base = resolved.parent().map(Path::to_path_buf);
        let buffers = load_buffers(&gltf.document, base.as_deref(), gltf.blob.clone())?;
        Ok(Self::from_document(path, &gltf.document, &buffers))
    }

    /// Build a library from an already parsed document.
    pub fn from_document(
        source: &Path,
        document: &gltf::Document,
        buffers: &[gltf::buffer::Data],
    ) -> Self {
        let mut rest = HashMap::new();
        let mut depths = vec![0u32; document.nodes().len()];
        // Each node's parents' accumulated transform (scene root to parent).
        let mut parents = vec![Mat4::IDENTITY; document.nodes().len()];
        for scene in document.scenes() {
            let mut stack: Vec<(gltf::Node, u32, Mat4)> =
                scene.nodes().map(|n| (n, 0, Mat4::IDENTITY)).collect();
            while let Some((node, depth, parent)) = stack.pop() {
                depths[node.index()] = depth;
                parents[node.index()] = parent;
                let below = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
                stack.extend(node.children().map(|child| (child, depth + 1, below)));
            }
        }
        for node in document.nodes() {
            let (t, r, s) = node.transform().decomposed();
            rest.entry(normalize_bone_name(&node_name(&node)))
                .or_insert(BoneRest {
                    transform: Transform::from_trs(
                        Vec3::from(t),
                        Quat::from_array(r),
                        Vec3::from(s),
                    ),
                    depth: depths[node.index()],
                    parent_rotation: parents[node.index()]
                        .to_scale_rotation_translation()
                        .1
                        .normalize(),
                    position: parents[node.index()].transform_point3(Vec3::from(t)),
                });
        }

        let clips = document
            .animations()
            .enumerate()
            .filter_map(|(index, animation)| {
                let clip = read_animation(&animation, index, buffers);
                (!clip.tracks.is_empty()).then(|| Arc::new(clip))
            })
            .collect();

        Self {
            source: source.to_path_buf(),
            clips,
            rest,
        }
    }

    pub fn clip(&self, name: &str) -> Option<&Arc<SkeletalClip>> {
        self.clips.iter().find(|clip| clip.name == name)
    }
}

/// The name a glTF node gets as an entity and as a bone: its own name, or
/// `node_<index>` when it has none.
pub fn node_name(node: &gltf::Node) -> String {
    node.name()
        .map(str::to_string)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("node_{}", node.index()))
}

/// The form bone names are compared in: case-insensitive, without a namespace prefix
/// (`mixamorig:Hips`, `Character1:Hips`) and without Mixamo's `mixamorig` prefix in any
/// of the spellings converters produce (`mixamorig1:`, `mixamorig_`, `mixamorig`).
///
/// ```
/// use wgpu_cube::scene::skeletal::normalize_bone_name;
/// for name in ["mixamorig:Hips", "mixamorig1:Hips", "mixamorig_Hips", "mixamorigHips", "Hips"] {
///     assert_eq!(normalize_bone_name(name), "hips");
/// }
/// ```
pub fn normalize_bone_name(name: &str) -> String {
    let name = name.trim();
    let name = name.rsplit(':').next().unwrap_or(name);
    let lower = name.to_ascii_lowercase();
    let stripped = lower
        .strip_prefix("mixamorig")
        .map(|rest| rest.trim_start_matches(|c: char| c.is_ascii_digit()))
        .map(|rest| rest.trim_start_matches(['_', '.']))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(&lower);
    stripped.to_string()
}

fn read_animation(
    animation: &gltf::Animation,
    index: usize,
    buffers: &[gltf::buffer::Data],
) -> SkeletalClip {
    let name = animation
        .name()
        .map(str::to_string)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("Animation_{index}"));
    let mut tracks: Vec<BoneTrack> = Vec::new();
    let mut by_node: HashMap<usize, usize> = HashMap::new();

    for channel in animation.channels() {
        let target = channel.target();
        let node = target.node();
        let reader = channel.reader(|buffer| buffers.get(buffer.index()).map(|d| &d.0[..]));
        let Some(times) = reader
            .read_inputs()
            .map(|inputs| inputs.collect::<Vec<f32>>())
        else {
            continue;
        };
        if times.is_empty() || times.iter().any(|t| !t.is_finite()) {
            continue;
        }
        let interpolation = match channel.sampler().interpolation() {
            gltf::animation::Interpolation::Step => AnimationInterpolation::Step,
            gltf::animation::Interpolation::Linear => AnimationInterpolation::Linear,
            gltf::animation::Interpolation::CubicSpline => AnimationInterpolation::CubicSpline,
        };
        use gltf::animation::util::ReadOutputs;
        let output = match reader.read_outputs() {
            Some(ReadOutputs::Translations(values)) => {
                AnimationOutput::Vec3(values.map(Vec3::from).collect())
            }
            Some(ReadOutputs::Scales(values)) => {
                AnimationOutput::Vec3(values.map(Vec3::from).collect())
            }
            Some(ReadOutputs::Rotations(values)) => {
                AnimationOutput::Quat(values.into_f32().map(Quat::from_array).collect())
            }
            // Morph weights are not supported by the engine.
            Some(ReadOutputs::MorphTargetWeights(_)) | None => continue,
        };
        let Some(sampler) = make_sampler(times, output, interpolation) else {
            log::warn!(
                "Clip '{name}': keyframe count mismatch on node '{}', channel skipped",
                node_name(&node)
            );
            continue;
        };

        let slot = *by_node.entry(node.index()).or_insert_with(|| {
            tracks.push(BoneTrack::new(node_name(&node)));
            tracks.len() - 1
        });
        let track = &mut tracks[slot];
        match target.property() {
            gltf::animation::Property::Translation => track.translation = Some(sampler),
            gltf::animation::Property::Rotation => track.rotation = Some(sampler),
            gltf::animation::Property::Scale => track.scale = Some(sampler),
            gltf::animation::Property::MorphTargetWeights => {}
        }
    }

    SkeletalClip::new(name, tracks)
}

/// A sampler whose value count matches its times (three values per key for cubic
/// splines); `None` if the file is inconsistent.
fn make_sampler(
    times: Vec<f32>,
    output: AnimationOutput,
    interpolation: AnimationInterpolation,
) -> Option<AnimationSampler> {
    let per_key = if interpolation == AnimationInterpolation::CubicSpline {
        3
    } else {
        1
    };
    let values = match &output {
        AnimationOutput::Vec3(v) => v.len(),
        AnimationOutput::Quat(v) => v.len(),
        AnimationOutput::Vec4(v) => v.len(),
    };
    (values == times.len() * per_key).then_some(AnimationSampler {
        times,
        output,
        interpolation,
    })
}

/// Load the buffers of a parsed glTF: the GLB binary chunk, data URIs and files next
/// to the source (through [`crate::io`], so this also works on the web).
pub(crate) fn load_buffers(
    document: &gltf::Document,
    base: Option<&Path>,
    mut blob: Option<Vec<u8>>,
) -> Result<Vec<gltf::buffer::Data>, String> {
    document
        .buffers()
        .map(|buffer| {
            let mut data = match buffer.source() {
                gltf::buffer::Source::Bin => blob
                    .take()
                    .ok_or_else(|| format!("buffer {} has no GLB chunk", buffer.index()))?,
                gltf::buffer::Source::Uri(uri) => {
                    if let Some(rest) = uri.strip_prefix("data:") {
                        let (_, encoded) = rest.split_once(',').ok_or_else(|| {
                            format!("malformed data URI in buffer {}", buffer.index())
                        })?;
                        base64::decode(encoded).map_err(|err| {
                            format!("bad data URI in buffer {}: {err}", buffer.index())
                        })?
                    } else {
                        let decoded =
                            crate::io::percent_decode_uri(uri).unwrap_or_else(|_| uri.to_string());
                        let path =
                            base.map_or_else(|| PathBuf::from(&decoded), |b| b.join(&decoded));
                        crate::io::load_binary(&path).map_err(|err| {
                            format!("cannot read buffer {}: {err}", path.display())
                        })?
                    }
                }
            };
            if data.len() < buffer.length() {
                return Err(format!(
                    "buffer {} has {} bytes, expected {}",
                    buffer.index(),
                    data.len(),
                    buffer.length()
                ));
            }
            while data.len() % 4 != 0 {
                data.push(0);
            }
            Ok(gltf::buffer::Data(data))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(path: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
    }

    #[test]
    fn bone_names_normalize_across_conventions() {
        assert_eq!(normalize_bone_name("mixamorig:LeftHand"), "lefthand");
        assert_eq!(normalize_bone_name("mixamorig12:LeftHand"), "lefthand");
        assert_eq!(normalize_bone_name("mixamorig.LeftHand"), "lefthand");
        assert_eq!(normalize_bone_name("Armature:Spine"), "spine");
        assert_eq!(normalize_bone_name("b_Hip_01"), "b_hip_01");
        // A bone actually called "mixamorig" keeps its name.
        assert_eq!(normalize_bone_name("mixamorig"), "mixamorig");
    }

    #[test]
    fn fox_library_has_three_clips_with_named_tracks() {
        let library = ClipLibrary::load(&asset("web/assets/skinned/Fox/Fox.glb")).unwrap();
        let names: Vec<&str> = library.clips.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Survey", "Walk", "Run"]);
        let walk = library.clip("Walk").unwrap();
        assert!(walk.duration > 0.5, "walk lasts {}", walk.duration);
        assert!(walk.tracks.iter().any(|t| t.key == "b_hip_01"));
        assert!(walk
            .tracks
            .iter()
            .all(|t| t.rotation.is_some() || t.translation.is_some()));
        assert_eq!(library.rest["b_hip_01"].depth, 3);
    }

    #[test]
    fn embedded_data_uri_buffers_load() {
        let library =
            ClipLibrary::load(&asset("web/assets/skinned/SimpleSkin/SimpleSkin.gltf")).unwrap();
        assert_eq!(library.clips.len(), 1);
        // The animated joint is unnamed, so it gets its node index as a name.
        assert_eq!(library.clips[0].tracks[0].bone, "node_2");
    }
}
