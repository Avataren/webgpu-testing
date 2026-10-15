//! Skeletal animation: skinned meshes, name-bound clips and the [`Animator`].
//!
//! - [`SkinnedMesh`] deforms a mesh with joint entities (the renderer skins it on the
//!   GPU, or on the CPU on devices without compute).
//! - [`ClipLibrary`] reads the animations of a glTF/GLB file as [`SkeletalClip`]s that
//!   target bones by name.
//! - [`Animator`] plays, cross-fades and layers those clips on the bones below its
//!   entity, with retargeting and root motion.

pub mod animator;
pub mod clip;
pub mod skin;

pub use animator::{
    animator_entity, find_bone, Animator, AnimatorCommand, AnimatorLayer, AnimatorState,
    AnimatorStatus, ClipInfo, PlayOptions, PlayingClip, RootMotion, MAX_LAYERS,
};
pub use clip::{node_name, normalize_bone_name, BoneRest, BoneTrack, ClipLibrary, SkeletalClip};
pub use skin::{skeleton_segments, SkinnedMesh, MAX_JOINTS};

pub(crate) use animator::update_animators;
