//! The [`Animator`] component: plays [`SkeletalClip`]s on the bones below an entity.
//!
//! An animator sits on a character's root entity and lists clip libraries (glTF/GLB
//! files). Clips bind to bones by [normalized name](super::normalize_bone_name) among
//! the animator entity and its descendants, so the same clip drives any character with
//! matching bone names. Playback is organised in layers:
//!
//! - Layer 0 is the base pose. Playing a clip cross-fades from whatever was playing
//!   (or from the rest pose) over `fade` seconds.
//! - Higher layers override the layers below, optionally only for the bones under a
//!   mask bone (e.g. an upper-body "wave" over a walk with mask `Spine`), weighted by
//!   the layer weight.
//!
//! Translations are retargeted per bone by the ratio of the target's rest bone length
//! to the source's, so clips authored in centimetres or on a taller character keep
//! their feet on the ground. Rotations are copied (rigs must share rest orientations,
//! as Mixamo rigs do).
//!
//! Root motion (the horizontal travel of the top-most translated bone, usually the
//! hips) can be kept, removed (`in_place`) or transferred to the animator entity
//! (`apply`), so a walk cycle moves the character without foot sliding.
//!
//! Several clips can also play as one *blend* ([`Animator::play_blend`]): a group of
//! clips that fades in and out together, each with its share of the pose, optionally
//! kept in step (a walk and a run blended by speed put their feet down together). An
//! [`AnimationTree`](super::AnimationTree) drives an animator through blends.
//!
//! Animators run with the scene's animations: in play mode and under explicit
//! simulation steps, not while editing.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use glam::{Mat4, Quat, Vec3};
use hecs::{Entity, World};
use serde::{Deserialize, Serialize};

use super::clip::{normalize_bone_name, ClipLibrary, SkeletalClip};
use crate::asset::Assets;
use crate::scene::components::{
    Children, GltfSource, Name, Parent, TransformComponent, WorldTransform,
};
use crate::scene::transform::Transform;

/// Most clips mixed at once on one layer; older, weaker ones are dropped first.
const MAX_CLIPS_PER_LAYER: usize = 16;
/// Most layers per animator.
pub const MAX_LAYERS: usize = 8;

/// What happens to the horizontal travel of the root bone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootMotion {
    /// Play the clip as authored: a walk moves away from the entity and snaps back
    /// when it loops.
    #[default]
    Keep,
    /// Remove the horizontal travel; the character walks on the spot.
    InPlace,
    /// Remove it from the bone and move the animator entity by it instead.
    Apply,
}

impl RootMotion {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Keep => "keep",
            Self::InPlace => "in_place",
            Self::Apply => "apply",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "keep" | "none" => Some(Self::Keep),
            "in_place" => Some(Self::InPlace),
            "apply" => Some(Self::Apply),
            _ => None,
        }
    }
}

/// Options for [`Animator::play`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlayOptions {
    /// Cross-fade duration in seconds (0 snaps).
    pub fade: f32,
    pub looping: bool,
    /// Playback rate of this clip (negative plays backwards).
    pub speed: f32,
    /// Layer to play on (0 = base).
    pub layer: usize,
    /// Start time in seconds; `None` keeps the time of a clip that is already playing
    /// and starts others at 0.
    pub start: Option<f32>,
}

impl Default for PlayOptions {
    fn default() -> Self {
        Self {
            fade: 0.2,
            looping: true,
            speed: 1.0,
            layer: 0,
            start: None,
        }
    }
}

/// Plays skeletal clips on the bones below its entity. See the [module docs](self).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Animator {
    /// glTF/GLB files whose animations this animator can play (project-relative).
    #[serde(default)]
    pub libraries: Vec<PathBuf>,
    /// Clip started (looping) when play begins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_clip: Option<String>,
    /// Multiplies the speed of every clip.
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub speed: f32,
    #[serde(default, skip_serializing_if = "is_default")]
    pub root_motion: RootMotion,
    /// Playback state; not saved.
    #[serde(skip)]
    pub state: AnimatorState,
}

fn one() -> f32 {
    1.0
}

fn is_one(value: &f32) -> bool {
    *value == 1.0
}

fn is_default<T: Default + PartialEq>(value: &T) -> bool {
    *value == T::default()
}

impl Default for Animator {
    fn default() -> Self {
        Self {
            libraries: Vec::new(),
            default_clip: None,
            speed: 1.0,
            root_motion: RootMotion::Keep,
            state: AnimatorState::default(),
        }
    }
}

/// Runtime playback state of an [`Animator`].
#[derive(Debug, Clone, Default)]
pub struct AnimatorState {
    pub layers: Vec<AnimatorLayer>,
    /// Whether the default clip has been started.
    started: bool,
    binding: Option<Arc<Binding>>,
    /// Problems already reported, so a script calling `animator_play` with a bad name
    /// every frame logs once.
    warned: HashSet<String>,
}

impl PartialEq for AnimatorState {
    /// Playback state never makes two animators different (scene diffs and undo compare
    /// the saved fields).
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// One playback layer.
#[derive(Debug, Clone)]
pub struct AnimatorLayer {
    pub weight: f32,
    target_weight: f32,
    weight_rate: f32,
    /// Only bones at or below this bone (normalized name) are affected; `None` = all.
    pub mask: Option<String>,
    pub clips: Vec<PlayingClip>,
}

impl Default for AnimatorLayer {
    fn default() -> Self {
        Self {
            weight: 1.0,
            target_weight: 1.0,
            weight_rate: 0.0,
            mask: None,
            clips: Vec::new(),
        }
    }
}

/// A clip playing on a layer.
#[derive(Debug, Clone)]
pub struct PlayingClip {
    /// The name it was played with.
    pub name: String,
    pub time: f32,
    pub speed: f32,
    pub looping: bool,
    pub weight: f32,
    target_weight: f32,
    /// Weight change per second while fading.
    fade_rate: f32,
    /// A non-looping clip reached its end.
    pub finished: bool,
    /// Horizontal root bone offset from the clip start sampled last frame (for root
    /// motion), in the bone's parent space.
    last_root: Option<Vec3>,
    /// Times the clip wrapped around during the last advance (negative backwards).
    wraps: i32,
    /// Its share of the blend it belongs to (1 for a clip played on its own).
    pub blend: f32,
    /// The blend it belongs to ([`Animator::play_blend`]); `None` for a clip played on
    /// its own.
    pub group: Option<String>,
    /// Kept in step with the other synced clips of its blend: they share one phase.
    sync: bool,
    /// Position in the cycle, 0..1, for synced clips.
    phase: f32,
}

impl PlayingClip {
    pub fn fading_out(&self) -> bool {
        self.target_weight == 0.0
    }

    /// How much it contributes to the pose: its fade times its share of its blend.
    pub fn effective_weight(&self) -> f32 {
        self.weight * self.blend
    }

    /// A clip fading in over `fade` seconds.
    fn new(name: &str, fade: f32) -> Self {
        Self {
            name: name.to_string(),
            time: 0.0,
            speed: 1.0,
            looping: true,
            weight: if fade > 0.0 { 0.0 } else { 1.0 },
            target_weight: 1.0,
            fade_rate: if fade > 0.0 { 1.0 / fade } else { 0.0 },
            finished: false,
            last_root: None,
            wraps: 0,
            blend: 1.0,
            group: None,
            sync: false,
            phase: 0.0,
        }
    }
}

/// One clip of a blend ([`Animator::play_blend`]).
#[derive(Debug, Clone, PartialEq)]
pub struct BlendClip {
    pub clip: String,
    /// Its share of the pose; the shares of a blend add up to 1.
    pub weight: f32,
    /// Its playback rate; negative plays it backwards (a walk as a backpedal).
    pub speed: f32,
}

/// Options for [`Animator::play_blend`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlendOptions {
    /// Cross-fade from whatever else plays on the layer (seconds).
    pub fade: f32,
    pub looping: bool,
    /// Keep the clips in step: one phase for all, advancing at their weighted duration.
    pub sync: bool,
    pub layer: usize,
    /// Start from the beginning even if the blend is still playing (entering a state
    /// again).
    pub restart: bool,
}

impl Default for BlendOptions {
    fn default() -> Self {
        Self {
            fade: 0.2,
            looping: true,
            sync: true,
            layer: 0,
            restart: false,
        }
    }
}

/// What a script or tool sees of an animator's playback.
#[derive(Debug, Clone, PartialEq)]
pub struct AnimatorStatus {
    pub clip: Option<String>,
    pub time: f32,
    pub duration: f32,
    pub playing: bool,
    pub looping: bool,
    pub finished: bool,
    pub fading: bool,
    pub speed: f32,
}

/// A clip as an animator can play it, and how well it fits the skeleton.
#[derive(Debug, Clone, PartialEq)]
pub struct ClipInfo {
    /// The name to play it by.
    pub name: String,
    /// Its name in the file (differs when two libraries share a clip name).
    pub source_name: String,
    pub library: PathBuf,
    pub duration: f32,
    pub tracks: usize,
    /// Track bones found in the skeleton.
    pub bound: usize,
    /// Track bones the skeleton does not have.
    pub missing: Vec<String>,
    /// How fast the root bone travels over the ground in the clip (m/s; about 0 for
    /// clips animated in place). A locomotion clip played at this ground speed does not
    /// slide its feet.
    pub root_speed: f32,
}

impl Animator {
    pub fn with_library(library: impl Into<PathBuf>) -> Self {
        Self {
            libraries: vec![library.into()],
            ..Self::default()
        }
    }

    /// Cross-fade to `clip` on `options.layer`. The name is checked when the animator
    /// next updates; an unknown name is logged once and ignored.
    pub fn play(&mut self, clip: &str, options: PlayOptions) {
        let layer_index = options.layer.min(MAX_LAYERS - 1);
        while self.state.layers.len() <= layer_index {
            self.state.layers.push(AnimatorLayer::default());
        }
        // An explicit play replaces the default clip.
        self.state.started = true;
        let layer = &mut self.state.layers[layer_index];
        let fade = options.fade.max(0.0);

        let existing = layer.clips.iter().position(|playing| playing.name == clip);
        for (index, playing) in layer.clips.iter_mut().enumerate() {
            if Some(index) == existing {
                continue;
            }
            fade_to(playing, 0.0, fade);
        }

        let playing = match existing {
            Some(index) => &mut layer.clips[index],
            None => {
                layer.clips.push(PlayingClip::new(clip, fade));
                layer.clips.last_mut().expect("just pushed")
            }
        };
        playing.group = None;
        playing.blend = 1.0;
        playing.sync = false;
        playing.speed = options.speed;
        playing.looping = options.looping;
        if let Some(start) = options.start {
            playing.time = start.max(0.0);
            playing.last_root = None;
        } else if playing.finished {
            // Playing a clip that ran to its end plays it again.
            playing.time = 0.0;
            playing.last_root = None;
        }
        playing.finished = false;
        fade_to(playing, 1.0, fade);

        drop_weakest(layer);
    }

    /// Play several clips as one blend called `group` on `options.layer`: they fade in
    /// together over `options.fade` while everything else on the layer fades out, each
    /// posing the character by its `weight`. Calling it again with the same group (every
    /// frame, as a blend space's parameter moves) only updates the weights and rates;
    /// clips of the group left out stay at weight 0, so they keep their place.
    pub fn play_blend(&mut self, group: &str, clips: &[BlendClip], options: BlendOptions) {
        let layer_index = options.layer.min(MAX_LAYERS - 1);
        while self.state.layers.len() <= layer_index {
            self.state.layers.push(AnimatorLayer::default());
        }
        self.state.started = true;
        let layer = &mut self.state.layers[layer_index];
        let fade = options.fade.max(0.0);

        // Entering again: a group that has faded out (or finished) starts over.
        let members: Vec<&PlayingClip> = layer
            .clips
            .iter()
            .filter(|playing| playing.group.as_deref() == Some(group))
            .collect();
        let restart = options.restart
            && (members.is_empty()
                || members
                    .iter()
                    .all(|playing| playing.fading_out() || playing.finished));
        let phase = members
            .iter()
            .find(|playing| playing.sync)
            .map_or(0.0, |playing| playing.phase);
        let phase = if restart { 0.0 } else { phase };

        for playing in layer.clips.iter_mut() {
            if playing.group.as_deref() == Some(group) {
                playing.blend = 0.0;
            } else if !playing.fading_out() {
                // Only start fades: renewing one every frame would never finish it.
                fade_to(playing, 0.0, fade);
            }
        }
        for blend in clips {
            let index = layer.clips.iter().position(|playing| {
                playing.group.as_deref() == Some(group) && playing.name == blend.clip
            });
            let playing = match index {
                Some(index) => &mut layer.clips[index],
                None => {
                    let mut playing = PlayingClip::new(&blend.clip, fade);
                    playing.group = Some(group.to_string());
                    playing.phase = phase;
                    layer.clips.push(playing);
                    layer.clips.last_mut().expect("just pushed")
                }
            };
            if restart {
                playing.time = 0.0;
                playing.phase = 0.0;
                playing.last_root = None;
            }
            playing.blend = blend.weight.max(0.0);
            playing.speed = blend.speed;
            playing.looping = options.looping;
            playing.sync = options.sync;
            if restart || playing.fading_out() {
                playing.finished = false;
            }
            if playing.target_weight != 1.0 {
                fade_to(playing, 1.0, fade);
            }
        }
        drop_weakest(layer);
    }

    /// Fade out everything on `layer` (`None` = every layer) over `fade` seconds; the
    /// bones return to the rest pose (or to the layers below).
    pub fn stop(&mut self, layer: Option<usize>, fade: f32) {
        self.state.started = true;
        for (index, playing_layer) in self.state.layers.iter_mut().enumerate() {
            if layer.is_some_and(|layer| layer != index) {
                continue;
            }
            for playing in &mut playing_layer.clips {
                fade_to(playing, 0.0, fade.max(0.0));
            }
        }
    }

    /// Set a layer's weight (fading over `fade` seconds) and optionally its bone mask.
    pub fn set_layer(
        &mut self,
        layer: usize,
        weight: f32,
        fade: f32,
        mask: Option<Option<String>>,
    ) {
        let layer = layer.min(MAX_LAYERS - 1);
        while self.state.layers.len() <= layer {
            self.state.layers.push(AnimatorLayer::default());
        }
        let playing_layer = &mut self.state.layers[layer];
        let weight = weight.clamp(0.0, 1.0);
        playing_layer.target_weight = weight;
        if fade <= 0.0 {
            playing_layer.weight = weight;
            playing_layer.weight_rate = 0.0;
        } else {
            playing_layer.weight_rate = (weight - playing_layer.weight).abs() / fade;
        }
        if let Some(mask) = mask {
            playing_layer.mask = mask.map(|name| normalize_bone_name(&name));
        }
    }

    /// The clip that dominates `layer` (the strongest one not fading out).
    pub fn status(&self, layer: usize) -> AnimatorStatus {
        let current = self.state.layers.get(layer).and_then(|layer| {
            layer
                .clips
                .iter()
                .filter(|clip| !clip.fading_out())
                .max_by(|a, b| a.effective_weight().total_cmp(&b.effective_weight()))
        });
        let duration = current
            .and_then(|clip| self.clip_duration(&clip.name))
            .unwrap_or(0.0);
        let fading = self
            .state
            .layers
            .get(layer)
            .is_some_and(|layer| layer.clips.iter().any(|clip| clip.fade_rate > 0.0));
        AnimatorStatus {
            clip: current.map(|clip| clip.name.clone()),
            time: current.map_or(0.0, |clip| clip.time),
            duration,
            playing: current.is_some_and(|clip| !clip.finished),
            looping: current.is_some_and(|clip| clip.looping),
            finished: current.is_some_and(|clip| clip.finished),
            fading,
            speed: current.map_or(0.0, |clip| clip.speed),
        }
    }

    /// How far the blend `group` on `layer` has played through its cycle (0..1), and
    /// whether it reached its end (a blend that does not loop), once it has started.
    pub fn blend_progress(&self, group: &str, layer: usize) -> Option<(f32, bool)> {
        let layer = self.state.layers.get(layer)?;
        let strongest = layer
            .clips
            .iter()
            .filter(|playing| playing.group.as_deref() == Some(group))
            .max_by(|a, b| a.blend.total_cmp(&b.blend))?;
        let progress = if strongest.sync {
            strongest.phase
        } else {
            let duration = self.clip_duration(&strongest.name).unwrap_or(0.0);
            if duration > 0.0 {
                let t = (strongest.time / duration).clamp(0.0, 1.0);
                if strongest.speed < 0.0 {
                    1.0 - t
                } else {
                    t
                }
            } else {
                1.0
            }
        };
        Some((progress, strongest.finished))
    }

    /// Duration of a clip by the name it plays under, once the animator has bound its
    /// libraries (after its first update, or [`Animator::bind`]).
    pub fn clip_duration(&self, name: &str) -> Option<f32> {
        let binding = self.state.binding.as_ref()?;
        binding
            .names
            .get(name)
            .map(|&index| binding.clips[index].clip.duration)
    }

    /// The playable clips, once bound. See [`Animator::bind`].
    pub fn clips(&self) -> Vec<ClipInfo> {
        self.state
            .binding
            .as_ref()
            .map(|binding| binding.clip_infos())
            .unwrap_or_default()
    }

    /// Errors loading libraries, once bound.
    pub fn library_errors(&self) -> Vec<String> {
        self.state
            .binding
            .as_ref()
            .map(|binding| binding.errors.clone())
            .unwrap_or_default()
    }

    /// Load the libraries and bind clips to the skeleton below `entity` now, without
    /// playing anything (tools use this to list clips in edit mode).
    pub fn bind(&mut self, world: &World, entity: Entity, assets: &mut Assets) {
        self.bind_with(world, entity, &mut |path| assets.clip_library(path));
    }

    /// [`Animator::bind`] with libraries from `load` (e.g. ones loaded beforehand).
    pub fn bind_with(
        &mut self,
        world: &World,
        entity: Entity,
        load: &mut dyn FnMut(&std::path::Path) -> Result<Arc<ClipLibrary>, String>,
    ) {
        if self
            .state
            .binding
            .as_ref()
            .is_none_or(|binding| !binding.is_valid(self, world))
        {
            let previous = self.state.binding.take();
            self.state.binding = Some(Arc::new(Binding::build(
                self,
                world,
                entity,
                load,
                previous.as_deref(),
            )));
        }
    }
}

/// A change to an animator, as scripts and tools request it.
#[derive(Debug, Clone, PartialEq)]
pub enum AnimatorCommand {
    Play {
        clip: String,
        options: PlayOptions,
    },
    Stop {
        layer: Option<usize>,
        fade: f32,
    },
    SetSpeed(f32),
    SetLayer {
        layer: usize,
        weight: f32,
        fade: f32,
        /// `Some(None)` clears the mask; `None` leaves it.
        mask: Option<Option<String>>,
    },
    AddLibrary(PathBuf),
    SetRootMotion(RootMotion),
}

impl Animator {
    pub fn apply(&mut self, command: AnimatorCommand) {
        match command {
            AnimatorCommand::Play { clip, options } => self.play(&clip, options),
            AnimatorCommand::Stop { layer, fade } => self.stop(layer, fade),
            AnimatorCommand::SetSpeed(speed) => self.speed = speed,
            AnimatorCommand::SetLayer {
                layer,
                weight,
                fade,
                mask,
            } => self.set_layer(layer, weight, fade, mask),
            AnimatorCommand::AddLibrary(path) => {
                if !self.libraries.contains(&path) {
                    self.libraries.push(path);
                }
            }
            AnimatorCommand::SetRootMotion(mode) => self.root_motion = mode,
        }
    }
}

/// The entity whose [`Animator`] drives `entity`: itself or its nearest ancestor with
/// one, so scripts may address a character by any part of it; failing that, the nearest
/// one below it (a character's root, whose model is its child).
pub fn animator_entity(world: &World, entity: Entity) -> Option<Entity> {
    let mut current = Some(entity);
    for _ in 0..256 {
        let Some(candidate) = current else {
            break;
        };
        if world.satisfies::<&Animator>(candidate).unwrap_or(false) {
            return Some(candidate);
        }
        current = parent_of(world, candidate);
    }
    let mut queue = std::collections::VecDeque::from([entity]);
    while let Some(current) = queue.pop_front() {
        if current != entity && world.satisfies::<&Animator>(current).unwrap_or(false) {
            return Some(current);
        }
        if let Ok(children) = world.get::<&Children>(current) {
            queue.extend(children.0.iter().copied());
        }
    }
    None
}

/// The entity named `bone` (compared as [`normalize_bone_name`] does) at or below
/// `root`, shallowest first. Use it to attach props to a hand or read a head position.
pub fn find_bone(world: &World, root: Entity, bone: &str) -> Option<Entity> {
    let key = normalize_bone_name(bone);
    let mut queue = std::collections::VecDeque::from([root]);
    while let Some(current) = queue.pop_front() {
        if name_key(world, current).is_some_and(|name| name == key) {
            return Some(current);
        }
        if let Ok(children) = world.get::<&Children>(current) {
            queue.extend(children.0.iter().copied());
        }
    }
    None
}

/// Keep a layer within [`MAX_CLIPS_PER_LAYER`] by dropping the weakest clip on its way
/// out.
fn drop_weakest(layer: &mut AnimatorLayer) {
    while layer.clips.len() > MAX_CLIPS_PER_LAYER {
        let Some(weakest) = layer
            .clips
            .iter()
            .enumerate()
            .filter(|(_, c)| c.fading_out())
            .min_by(|a, b| a.1.weight.total_cmp(&b.1.weight))
            .map(|(i, _)| i)
        else {
            break;
        };
        layer.clips.remove(weakest);
    }
}

fn fade_to(playing: &mut PlayingClip, target: f32, fade: f32) {
    playing.target_weight = target;
    if fade <= 0.0 {
        playing.weight = target;
        playing.fade_rate = 0.0;
    } else {
        playing.fade_rate = (target - playing.weight).abs() / fade;
    }
}

/// Clips and bones resolved for one animator.
#[derive(Debug)]
struct Binding {
    libraries: Vec<PathBuf>,
    errors: Vec<String>,
    clips: Vec<BoundClip>,
    /// Every name a clip can be played by -> index into `clips`.
    names: HashMap<String, usize>,
    /// Bone slots: entity, rest local transform, depth below the animator.
    bones: Vec<Entity>,
    rest: Vec<Transform>,
    depth: Vec<u32>,
    /// For each slot, the normalized names of the bone and its ancestors up to the
    /// animator, for layer masks.
    lineage: Vec<Vec<String>>,
    /// Rest transform of each bone's parent chain up to the animator (exclusive), by
    /// slot: maps the bone's parent space into the animator's local space.
    parent_to_animator: Vec<Mat4>,
    animator_entity: Entity,
}

#[derive(Debug)]
struct BoundClip {
    name: String,
    library: PathBuf,
    clip: Arc<SkeletalClip>,
    tracks: Vec<BoundTrack>,
    missing: Vec<String>,
    /// Index into `tracks` of the root-motion track: the shallowest bone with a
    /// translation channel.
    root: Option<usize>,
}

/// A clip track bound to a bone of the skeleton.
#[derive(Debug, Clone, Copy)]
struct BoundTrack {
    /// Index into the clip's tracks.
    track: usize,
    /// The bone's slot.
    slot: usize,
    /// Multiplies translation keys (target rest length / source rest length).
    scale: f32,
    /// The bone's rest in the clip's file: its parent frame (file root to parent) and
    /// its own rotation. `None` when the file does not have the node.
    source_rest: Option<(Quat, Quat)>,
    /// Retargeting, when the skeleton's rest differs from the clip file's: keys become
    /// `pre * key * post` (translations `pre * key`), so each bone turns away from its
    /// rest as it does in the clip, in the world. That covers the same rig exported with
    /// other axes (a Mixamo FBX through Blender, whose bones point along Y, against a
    /// GLB that kept Mixamo's joint orientations).
    retarget: Option<(Quat, Quat)>,
}

impl BoundTrack {
    fn translation(&self, key: Vec3) -> Vec3 {
        let key = key * self.scale;
        self.retarget.map_or(key, |(pre, _)| pre * key)
    }

    fn rotation(&self, key: Quat) -> Quat {
        self.retarget.map_or(key, |(pre, post)| pre * key * post)
    }
}

impl Binding {
    fn is_valid(&self, animator: &Animator, world: &World) -> bool {
        self.libraries == animator.libraries && self.bones.iter().all(|&bone| world.contains(bone))
    }

    fn build(
        animator: &Animator,
        world: &World,
        entity: Entity,
        load: &mut dyn FnMut(&std::path::Path) -> Result<Arc<ClipLibrary>, String>,
        previous: Option<&Binding>,
    ) -> Self {
        // Every named entity at or below the animator, shallowest first.
        let mut skeleton: HashMap<String, (Entity, u32)> = HashMap::new();
        let mut queue = std::collections::VecDeque::from([(entity, 0u32)]);
        while let Some((current, depth)) = queue.pop_front() {
            if let Ok(name) = world.get::<&Name>(current) {
                skeleton
                    .entry(normalize_bone_name(&name.0))
                    .or_insert((current, depth));
            }
            if let Ok(children) = world.get::<&Children>(current) {
                queue.extend(children.0.iter().map(|&child| (child, depth + 1)));
            }
        }

        let mut binding = Binding {
            libraries: animator.libraries.clone(),
            errors: Vec::new(),
            clips: Vec::new(),
            names: HashMap::new(),
            bones: Vec::new(),
            rest: Vec::new(),
            depth: Vec::new(),
            lineage: Vec::new(),
            parent_to_animator: Vec::new(),
            animator_entity: entity,
        };
        let mut slots: HashMap<Entity, usize> = HashMap::new();
        let previous_rest: HashMap<Entity, Transform> = previous
            .map(|previous| {
                previous
                    .bones
                    .iter()
                    .copied()
                    .zip(previous.rest.iter().copied())
                    .collect()
            })
            .unwrap_or_default();

        let libraries: Vec<(PathBuf, Arc<ClipLibrary>)> = animator
            .libraries
            .iter()
            .filter_map(|path| match load(path) {
                Ok(library) => Some((path.clone(), library)),
                Err(err) => {
                    binding.errors.push(err);
                    None
                }
            })
            .collect();

        for (path, library) in &libraries {
            for clip in &library.clips {
                let mut tracks = Vec::new();
                let mut missing = Vec::new();
                for (track_index, track) in clip.tracks.iter().enumerate() {
                    let Some(&(bone, depth)) = skeleton.get(&track.key) else {
                        missing.push(track.bone.clone());
                        continue;
                    };
                    let slot = *slots.entry(bone).or_insert_with(|| {
                        let rest = previous_rest.get(&bone).copied().unwrap_or_else(|| {
                            world
                                .get::<&TransformComponent>(bone)
                                .map(|t| t.0)
                                .unwrap_or_default()
                        });
                        binding.bones.push(bone);
                        binding.rest.push(rest);
                        binding.depth.push(depth);
                        binding.bones.len() - 1
                    });
                    let source = library.rest.get(&track.key);
                    let scale = translation_scale(
                        source.map(|r| r.transform.translation),
                        binding.rest[slot].translation,
                    );
                    let source_rest = source.map(|r| (r.parent_rotation, r.transform.rotation));
                    tracks.push((track_index, slot, scale, source_rest));
                }
                // Bones whose rest translation is zero take the scale of the
                // shallowest bone that has one, so root motion on a bone at the origin
                // is still converted.
                let global = tracks
                    .iter()
                    .filter(|(_, _, scale, _)| scale.is_some())
                    .min_by_key(|(_, slot, _, _)| binding.depth[*slot])
                    .and_then(|(_, _, scale, _)| *scale)
                    .unwrap_or(1.0);
                let tracks: Vec<BoundTrack> = tracks
                    .into_iter()
                    .map(|(track, slot, scale, source_rest)| BoundTrack {
                        track,
                        slot,
                        scale: scale.unwrap_or(global),
                        source_rest,
                        retarget: None,
                    })
                    .collect();
                let root = tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, bound)| clip.tracks[bound.track].translation.is_some())
                    .min_by_key(|(_, bound)| binding.depth[bound.slot])
                    .map(|(index, _)| index);
                binding.clips.push(BoundClip {
                    name: clip.name.clone(),
                    library: path.clone(),
                    clip: Arc::clone(clip),
                    tracks,
                    missing,
                    root,
                });
            }
        }

        // The model's own file says how its skeleton's frames turn, whatever the user
        // has done to its nodes since.
        let model = world
            .get::<&GltfSource>(entity)
            .ok()
            .map(|source| {
                let path = source.0.to_string_lossy();
                PathBuf::from(path.strip_suffix(".import").unwrap_or(&path))
            })
            .and_then(|path| {
                libraries
                    .iter()
                    .find(|(library, _)| *library == path)
                    .map(|(_, library)| Arc::clone(library))
                    .or_else(|| load(&path).ok())
            });

        binding.finish_names();
        binding.finish_hierarchy(world);
        binding.finish_frames(world, model.as_deref());
        binding
    }

    /// Clip names: the file's clip name when unique across libraries, else
    /// `<library stem>/<clip name>`; a library with one clip also answers to its stem.
    fn finish_names(&mut self) {
        let mut counts: HashMap<&str, usize> = HashMap::new();
        for clip in &self.clips {
            *counts.entry(clip.clip.name.as_str()).or_default() += 1;
        }
        let mut per_library: HashMap<PathBuf, usize> = HashMap::new();
        for clip in &self.clips {
            *per_library.entry(clip.library.clone()).or_default() += 1;
        }
        let mut names = HashMap::new();
        let mut display = Vec::with_capacity(self.clips.len());
        for (index, clip) in self.clips.iter().enumerate() {
            let stem = library_stem(&clip.library);
            let qualified = format!("{stem}/{}", clip.clip.name);
            let unique = counts[clip.clip.name.as_str()] == 1;
            let name = if per_library[&clip.library] == 1 && !names.contains_key(&stem) {
                // A one-clip file ("walk.glb" holding "mixamo.com") plays as "walk".
                names.entry(stem.clone()).or_insert(index);
                stem
            } else if unique {
                clip.clip.name.clone()
            } else {
                qualified.clone()
            };
            if unique {
                names.entry(clip.clip.name.clone()).or_insert(index);
            }
            names.entry(qualified).or_insert(index);
            names.entry(name.clone()).or_insert(index);
            display.push(name);
        }
        for (clip, name) in self.clips.iter_mut().zip(display) {
            clip.name = name;
        }
        self.names = names;
    }

    fn finish_hierarchy(&mut self, world: &World) {
        let slots: HashMap<Entity, usize> = self
            .bones
            .iter()
            .enumerate()
            .map(|(slot, &bone)| (bone, slot))
            .collect();
        for &bone in &self.bones {
            let parent = parent_of(world, bone);
            let mut lineage: Vec<String> = name_key(world, bone).into_iter().collect();
            // Rest chain from the animator down to the bone's parent.
            let mut chain = Mat4::IDENTITY;
            let mut current = parent;
            let mut guard = 0;
            while let Some(entity) = current {
                lineage.extend(name_key(world, entity));
                if entity == self.animator_entity || guard > 256 {
                    break;
                }
                let local = slots
                    .get(&entity)
                    .map(|&slot| self.rest[slot])
                    .or_else(|| world.get::<&TransformComponent>(entity).ok().map(|t| t.0))
                    .unwrap_or_default();
                chain = local.matrix() * chain;
                current = parent_of(world, entity);
                guard += 1;
            }
            self.parent_to_animator.push(chain);
            self.lineage.push(lineage);
        }
    }

    /// Retarget each track whose bone rests differently here than in the clip's file
    /// (see [`BoundTrack::retarget`]): with world rest rotations `S` (clip file) and
    /// `T` (skeleton), a bone keyed `k` under a parent `p` gets
    /// `T(p)⁻¹ S(p) · k · S(b)⁻¹ T(b)`, which turns it away from its rest as the clip
    /// does. Identical rigs need nothing.
    ///
    /// `model` is the skeleton's own file; without it the skeleton's rest is read from
    /// the entities.
    fn finish_frames(&mut self, world: &World, model: Option<&ClipLibrary>) {
        // From the entities, frames start at the root of the imported file: the
        // animator, and the nodes of the same file above it, count; the entity the user
        // places the model with does not.
        let mut above = Quat::IDENTITY;
        let mut current = Some(self.animator_entity);
        let mut guard = 0;
        while let Some(entity) = current {
            if guard > 256 || !world.satisfies::<&GltfSource>(entity).unwrap_or(false) {
                break;
            }
            if let Ok(transform) = world.get::<&TransformComponent>(entity) {
                above = transform.0.rotation * above;
            }
            current = parent_of(world, entity);
            guard += 1;
        }
        for clip in &mut self.clips {
            for bound in &mut clip.tracks {
                let Some((source_parent, source_rest)) = bound.source_rest else {
                    continue;
                };
                let key = &clip.clip.tracks[bound.track].key;
                let (target_parent, target_rest) = match model.and_then(|model| model.rest.get(key))
                {
                    Some(rest) => (rest.parent_rotation, rest.transform.rotation),
                    None => {
                        let (_, chain, _) =
                            self.parent_to_animator[bound.slot].to_scale_rotation_translation();
                        ((above * chain).normalize(), self.rest[bound.slot].rotation)
                    }
                };
                let pre = (target_parent.inverse() * source_parent).normalize();
                let post = ((source_parent * source_rest).inverse()
                    * (target_parent * target_rest))
                    .normalize();
                let turned = |q: Quat| q.angle_between(Quat::IDENTITY) > 0.01;
                if turned(pre) || turned(post) {
                    bound.retarget = Some((pre, post));
                }
            }
        }
    }

    fn clip_infos(&self) -> Vec<ClipInfo> {
        self.clips
            .iter()
            .map(|clip| ClipInfo {
                name: clip.name.clone(),
                source_name: clip.clip.name.clone(),
                library: clip.library.clone(),
                duration: clip.clip.duration,
                tracks: clip.clip.tracks.len(),
                bound: clip.tracks.len(),
                missing: clip.missing.clone(),
                root_speed: self.root_speed(clip),
            })
            .collect()
    }

    /// How fast the clip's root bone travels over the ground, start to end, in the
    /// animator's space (m/s; 0 for clips animated in place).
    fn root_speed(&self, clip: &BoundClip) -> f32 {
        let (Some(root), duration) = (clip.root, clip.clip.duration) else {
            return 0.0;
        };
        let bound = clip.tracks[root];
        let Some(translation) = clip.clip.tracks[bound.track].translation.as_ref() else {
            return 0.0;
        };
        let start_time = translation.times.first().copied().unwrap_or(0.0);
        let span = duration - start_time;
        let (Some(start), Some(end)) = (
            translation.sample_vec3(start_time),
            translation.sample_vec3(duration),
        ) else {
            return 0.0;
        };
        if span <= 1.0e-4 {
            return 0.0;
        }
        let travel = self.parent_to_animator[bound.slot]
            .transform_vector3(bound.translation(end) - bound.translation(start));
        Vec3::new(travel.x, 0.0, travel.z).length() / span
    }

    /// Whether `slot` lies at or below the bone named `mask` (normalized).
    fn in_mask(&self, slot: usize, mask: &str) -> bool {
        self.lineage[slot].iter().any(|name| name == mask)
    }
}

fn parent_of(world: &World, entity: Entity) -> Option<Entity> {
    world.get::<&Parent>(entity).ok().map(|parent| parent.0)
}

fn name_key(world: &World, entity: Entity) -> Option<String> {
    world
        .get::<&Name>(entity)
        .ok()
        .map(|name| normalize_bone_name(&name.0))
}

fn library_stem(path: &std::path::Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// How much to scale a bone's translation keys: target rest length / source rest
/// length. `None` when either rest translation is (near) zero; 1 when they agree.
fn translation_scale(source_rest: Option<Vec3>, target_rest: Vec3) -> Option<f32> {
    let source = source_rest?.length();
    let target = target_rest.length();
    if source < 1e-6 || target < 1e-6 {
        return None;
    }
    let ratio = target / source;
    Some(if (ratio - 1.0).abs() < 1e-4 {
        1.0
    } else {
        ratio
    })
}

/// Accumulated pose of one bone on one layer.
#[derive(Clone, Copy)]
struct Accum {
    weight: f32,
    translation: Vec3,
    rotation: Quat,
    scale: Vec3,
}

impl Accum {
    const ZERO: Self = Self {
        weight: 0.0,
        translation: Vec3::ZERO,
        rotation: Quat::from_xyzw(0.0, 0.0, 0.0, 0.0),
        scale: Vec3::ZERO,
    };

    fn add(&mut self, weight: f32, pose: Transform) {
        // Keep quaternions in one hemisphere so the weighted sum does not cancel out.
        let rotation = if self.weight > 0.0 && self.rotation.dot(pose.rotation) < 0.0 {
            -pose.rotation
        } else {
            pose.rotation
        };
        self.weight += weight;
        self.translation += pose.translation * weight;
        self.rotation =
            Quat::from_vec4(glam::Vec4::from(self.rotation) + glam::Vec4::from(rotation) * weight);
        self.scale += pose.scale * weight;
    }

    /// The weighted average, topped up with `fill` when the weights sum below 1.
    fn resolve(self, fill: Transform) -> Transform {
        let mut acc = self;
        if acc.weight < 1.0 {
            acc.add(1.0 - acc.weight, fill);
        }
        let inv = 1.0 / acc.weight;
        Transform {
            translation: acc.translation * inv,
            rotation: acc.rotation.normalize(),
            scale: acc.scale * inv,
        }
    }
}

fn blend(a: Transform, b: Transform, t: f32) -> Transform {
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    Transform {
        translation: a.translation.lerp(b.translation, t),
        rotation: a.rotation.slerp(b.rotation, t),
        scale: a.scale.lerp(b.scale, t),
    }
}

/// Advance every animator in `world` by `dt` seconds and pose its bones.
pub(crate) fn update_animators(world: &mut World, assets: &mut Assets, dt: f32) {
    let entities: Vec<Entity> = world
        .query::<&Animator>()
        .iter()
        .map(|(entity, _)| entity)
        .collect();
    for entity in entities {
        // Take the component out so the world can be read while it is updated.
        let Ok(mut animator) = world
            .get::<&mut Animator>(entity)
            .map(|mut a| std::mem::take(&mut *a))
        else {
            continue;
        };
        animator.bind(world, entity, assets);
        let root_delta = advance_and_pose(&mut animator, world, entity, dt);
        if let Some(delta) = root_delta {
            if let Ok(mut transform) = world.get::<&mut TransformComponent>(entity) {
                let t = &mut transform.0;
                t.translation += t.rotation * (t.scale * delta);
            }
        }
        if let Ok(mut slot) = world.get::<&mut Animator>(entity) {
            *slot = animator;
        }
    }
}

/// Advance clip times and fades, then write the blended pose to the bones. Returns the
/// root motion to apply to the animator entity (in its local space) for
/// [`RootMotion::Apply`].
fn advance_and_pose(
    animator: &mut Animator,
    world: &mut World,
    entity: Entity,
    dt: f32,
) -> Option<Vec3> {
    let binding = Arc::clone(animator.state.binding.as_ref()?);

    if !animator.state.started {
        animator.state.started = true;
        if let Some(clip) = animator.default_clip.clone() {
            animator.play(
                &clip,
                PlayOptions {
                    fade: 0.0,
                    ..PlayOptions::default()
                },
            );
        }
    }

    // Report unknown clip names once and drop them.
    for layer in &mut animator.state.layers {
        layer.clips.retain(|playing| {
            let known = binding.names.contains_key(&playing.name);
            if !known && animator.state.warned.insert(playing.name.clone()) {
                let mut available: Vec<&String> = binding.names.keys().collect();
                available.sort();
                log::warn!(
                    "Animator: no clip named '{}' (available: {:?})",
                    playing.name,
                    available
                );
            }
            known
        });
    }

    let speed = animator.speed;
    for layer in &mut animator.state.layers {
        if layer.weight_rate > 0.0 {
            let step = layer.weight_rate * dt;
            if (layer.target_weight - layer.weight).abs() <= step {
                layer.weight = layer.target_weight;
                layer.weight_rate = 0.0;
            } else {
                layer.weight += step * (layer.target_weight - layer.weight).signum();
            }
        }
        for playing in &mut layer.clips {
            let duration = binding.clips[binding.names[&playing.name]].clip.duration;
            if playing.sync && playing.group.is_some() {
                advance_fade(playing, dt);
            } else {
                advance_clip(playing, dt, speed, duration);
            }
        }
        advance_synced(layer, &binding, dt, speed);
        layer
            .clips
            .retain(|playing| !(playing.fading_out() && playing.weight <= 0.0));
    }

    if binding.bones.is_empty() {
        return None;
    }

    // Animator-space up vector, for separating vertical from horizontal root motion.
    let up_local = world
        .get::<&WorldTransform>(entity)
        .map(|w| w.0.rotation.inverse() * Vec3::Y)
        .unwrap_or(Vec3::Y);

    let bone_count = binding.bones.len();
    let mut pose: Vec<Transform> = binding.rest.clone();
    let mut root_delta = Vec3::ZERO;
    let mut has_root_delta = false;
    let mut accum = vec![Accum::ZERO; bone_count];

    for (layer_index, layer) in animator.state.layers.iter_mut().enumerate() {
        if layer.clips.is_empty() || (layer_index > 0 && layer.weight <= 0.0) {
            continue;
        }
        accum.iter_mut().for_each(|a| *a = Accum::ZERO);
        for playing in &mut layer.clips {
            let weight = playing.effective_weight();
            if weight <= 0.0 {
                continue;
            }
            let bound = &binding.clips[binding.names[&playing.name]];
            for (track_position, bound_track) in bound.tracks.iter().enumerate() {
                let slot = bound_track.slot;
                let track = &bound.clip.tracks[bound_track.track];
                let rest = binding.rest[slot];
                let (t, r, s) = track.sample(playing.time);
                let mut bone = Transform {
                    translation: t.map_or(rest.translation, |t| bound_track.translation(t)),
                    rotation: r.map_or(rest.rotation, |r| bound_track.rotation(r)),
                    scale: s.unwrap_or(rest.scale),
                };
                if Some(track_position) == bound.root && animator.root_motion != RootMotion::Keep {
                    // Keep only the vertical part of the travel since the first key.
                    let start = track
                        .translation
                        .as_ref()
                        .and_then(|s| s.sample_vec3(s.times[0]))
                        .map_or(rest.translation, |t| bound_track.translation(t));
                    let to_animator = binding.parent_to_animator[slot];
                    let up = to_animator
                        .inverse()
                        .transform_vector3(up_local)
                        .normalize_or(Vec3::Y);
                    let offset = bone.translation - start;
                    let vertical = up * offset.dot(up);
                    bone.translation = start + vertical;
                    if animator.root_motion == RootMotion::Apply && layer_index == 0 {
                        let horizontal = offset - vertical;
                        if let Some(last) = playing.last_root {
                            // Each wrap adds one full cycle of travel.
                            let travel = if playing.wraps != 0 {
                                let end = track
                                    .translation
                                    .as_ref()
                                    .and_then(|s| s.sample_vec3(bound.clip.duration))
                                    .map_or(start, |t| bound_track.translation(t))
                                    - start;
                                let cycle = end - up * end.dot(up);
                                horizontal - last + cycle * playing.wraps as f32
                            } else {
                                horizontal - last
                            };
                            root_delta += to_animator.transform_vector3(travel) * weight;
                            has_root_delta = true;
                        }
                        playing.last_root = Some(horizontal);
                    }
                }
                accum[slot].add(weight, bone);
            }
        }

        for slot in 0..bone_count {
            let acc = accum[slot];
            if acc.weight <= 0.0 {
                continue;
            }
            if layer_index == 0 {
                pose[slot] = acc.resolve(binding.rest[slot]);
            } else {
                let in_mask = layer
                    .mask
                    .as_deref()
                    .is_none_or(|mask| binding.in_mask(slot, mask));
                if in_mask {
                    let layer_pose = acc.resolve(pose[slot]);
                    pose[slot] = blend(pose[slot], layer_pose, layer.weight * acc.weight.min(1.0));
                }
            }
        }
    }

    for (slot, &bone) in binding.bones.iter().enumerate() {
        if let Ok(mut transform) = world.get::<&mut TransformComponent>(bone) {
            transform.0 = pose[slot];
        }
    }

    (has_root_delta && animator.root_motion == RootMotion::Apply).then_some(root_delta)
}

/// Advance a clip by `dt` seconds of real time (fades) scaled by the animator's and the
/// clip's speed (playback).
fn advance_clip(playing: &mut PlayingClip, dt: f32, animator_speed: f32, duration: f32) {
    advance_fade(playing, dt);
    playing.wraps = 0;
    if playing.finished || duration <= 0.0 {
        return;
    }
    let time = playing.time + dt * animator_speed * playing.speed;
    if playing.looping {
        playing.wraps = (time / duration).floor() as i32;
        playing.time = time.rem_euclid(duration);
    } else if time >= duration {
        playing.time = duration;
        playing.finished = true;
    } else if time <= 0.0 && playing.speed < 0.0 {
        playing.time = 0.0;
        playing.finished = true;
    } else {
        playing.time = time.max(0.0);
    }
}

/// Move a clip's weight toward its fade target.
fn advance_fade(playing: &mut PlayingClip, dt: f32) {
    if playing.fade_rate > 0.0 {
        let step = playing.fade_rate * dt.max(0.0);
        if (playing.target_weight - playing.weight).abs() <= step {
            playing.weight = playing.target_weight;
            playing.fade_rate = 0.0;
        } else {
            playing.weight += step * (playing.target_weight - playing.weight).signum();
        }
    }
}

/// Advance the synced clips of each blend together: one phase, moving at the rate of
/// their weighted duration, so a walk and a run blended by speed keep step. A clip with
/// a negative speed runs through the cycle backwards.
fn advance_synced(layer: &mut AnimatorLayer, binding: &Binding, dt: f32, animator_speed: f32) {
    let mut groups: Vec<String> = layer
        .clips
        .iter()
        .filter(|playing| playing.sync)
        .filter_map(|playing| playing.group.clone())
        .collect();
    groups.sort();
    groups.dedup();
    for group in groups {
        let duration_of =
            |playing: &PlayingClip| binding.clips[binding.names[&playing.name]].clip.duration;
        let members = || {
            layer
                .clips
                .iter()
                .filter(|playing| playing.sync && playing.group.as_deref() == Some(&group))
        };
        let (mut weighted, mut total, mut longest, mut rate) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        for playing in members() {
            let duration = duration_of(playing);
            weighted += playing.blend * duration;
            total += playing.blend;
            longest = longest.max(duration);
            rate += playing.blend * playing.speed.abs();
        }
        let duration = if total > 1.0e-4 {
            weighted / total
        } else {
            longest
        };
        let rate = if total > 1.0e-4 { rate / total } else { 1.0 };
        let Some(phase) = members().next().map(|playing| playing.phase) else {
            continue;
        };
        let looping = members().all(|playing| playing.looping);
        let finished = members().all(|playing| playing.finished);
        let mut next = if duration > 0.0 && !finished {
            phase + dt * animator_speed * rate / duration
        } else {
            phase
        };
        let wraps = if looping { next.floor() as i32 } else { 0 };
        let ended = !looping && next >= 1.0;
        next = if looping {
            next.rem_euclid(1.0)
        } else {
            next.min(1.0)
        };
        for playing in layer
            .clips
            .iter_mut()
            .filter(|playing| playing.sync && playing.group.as_deref() == Some(&group))
        {
            let duration = binding.clips[binding.names[&playing.name]].clip.duration;
            let backwards = playing.speed < 0.0;
            playing.phase = next;
            playing.time = if backwards { 1.0 - next } else { next } * duration;
            playing.wraps = if backwards { -wraps } else { wraps };
            if ended {
                playing.finished = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::animation::{AnimationInterpolation, AnimationOutput, AnimationSampler};
    use crate::scene::skeletal::clip::{BoneRest, BoneTrack};

    fn linear_vec3(times: &[f32], values: &[Vec3]) -> AnimationSampler {
        AnimationSampler {
            times: times.to_vec(),
            output: AnimationOutput::Vec3(values.to_vec()),
            interpolation: AnimationInterpolation::Linear,
        }
    }

    fn linear_quat(times: &[f32], values: &[Quat]) -> AnimationSampler {
        AnimationSampler {
            times: times.to_vec(),
            output: AnimationOutput::Quat(values.to_vec()),
            interpolation: AnimationInterpolation::Linear,
        }
    }

    /// Root "Character" (animator) -> "mixamorig:Hips" (rest y = 1) -> "Spine" -> "Arm".
    struct Rig {
        world: World,
        root: Entity,
        hips: Entity,
        spine: Entity,
        arm: Entity,
    }

    fn rig(hip_height: f32) -> Rig {
        let mut world = World::new();
        let root = world.spawn((
            Name::new("Character"),
            TransformComponent(Transform::IDENTITY),
            WorldTransform(Transform::IDENTITY),
        ));
        let hips = world.spawn((
            Name::new("mixamorig:Hips"),
            TransformComponent(Transform::from_translation(Vec3::Y * hip_height)),
            Parent(root),
        ));
        let spine = world.spawn((
            Name::new("mixamorig:Spine"),
            TransformComponent(Transform::from_translation(Vec3::Y * 0.2)),
            Parent(hips),
        ));
        let arm = world.spawn((
            Name::new("mixamorig:Arm"),
            TransformComponent(Transform::from_translation(Vec3::X * 0.3)),
            Parent(spine),
        ));
        world.insert_one(root, Children(vec![hips])).unwrap();
        world.insert_one(hips, Children(vec![spine])).unwrap();
        world.insert_one(spine, Children(vec![arm])).unwrap();
        Rig {
            world,
            root,
            hips,
            spine,
            arm,
        }
    }

    /// A library authored in centimetres: hips rest at y = 100, walking 200 cm forward
    /// (+Z) over 1 s and bobbing 10 cm; the spine turns 90 degrees about Y.
    fn walk_library() -> ClipLibrary {
        let mut hips = BoneTrack::new("Hips");
        hips.translation = Some(linear_vec3(
            &[0.0, 0.5, 1.0],
            &[
                Vec3::new(0.0, 100.0, 0.0),
                Vec3::new(0.0, 110.0, 100.0),
                Vec3::new(0.0, 100.0, 200.0),
            ],
        ));
        let mut spine = BoneTrack::new("mixamorig1:Spine");
        spine.rotation = Some(linear_quat(
            &[0.0, 1.0],
            &[
                Quat::IDENTITY,
                Quat::from_rotation_y(std::f32::consts::FRAC_PI_2),
            ],
        ));
        let mut wave = BoneTrack::new("Arm");
        wave.rotation = Some(linear_quat(
            &[0.0, 1.0],
            &[Quat::from_rotation_z(1.0), Quat::from_rotation_z(1.0)],
        ));
        let rest = |t: Vec3, depth| BoneRest {
            transform: Transform::from_translation(t),
            depth,
            parent_rotation: Quat::IDENTITY,
        };
        ClipLibrary {
            source: PathBuf::from("walk.glb"),
            clips: vec![
                Arc::new(SkeletalClip::new("mixamo.com", vec![hips, spine])),
                Arc::new(SkeletalClip::new("wave", vec![wave])),
            ],
            rest: HashMap::from([
                ("hips".to_string(), rest(Vec3::Y * 100.0, 1)),
                ("spine".to_string(), rest(Vec3::Y * 20.0, 2)),
                ("arm".to_string(), rest(Vec3::X * 30.0, 3)),
            ]),
        }
    }

    fn setup(animator: Animator) -> (Rig, Assets) {
        let mut assets = Assets::new();
        assets.insert_clip_library("walk.glb", walk_library());
        let mut rig = rig(1.0);
        rig.world.insert_one(rig.root, animator).unwrap();
        (rig, assets)
    }

    fn local(world: &World, entity: Entity) -> Transform {
        world.get::<&TransformComponent>(entity).unwrap().0
    }

    #[test]
    fn clips_are_named_and_bound_by_normalized_bone_names() {
        let (mut rig, mut assets) = setup(Animator::with_library("walk.glb"));
        let mut animator = rig.world.remove_one::<Animator>(rig.root).unwrap();
        animator.bind(&rig.world, rig.root, &mut assets);
        let clips = animator.clips();
        let names: Vec<&str> = clips.iter().map(|c| c.name.as_str()).collect();
        // Two clips in one library: their own names (unique) are used.
        assert_eq!(names, ["mixamo.com", "wave"]);
        assert!(clips.iter().all(|c| c.missing.is_empty()), "{clips:?}");
        assert_eq!(animator.clip_duration("walk/wave"), Some(1.0));
        assert!(animator.library_errors().is_empty());
    }

    #[test]
    fn clips_report_how_fast_they_travel() {
        let (mut rig, mut assets) = setup(Animator::with_library("walk.glb"));
        let mut animator = rig.world.remove_one::<Animator>(rig.root).unwrap();
        animator.bind(&rig.world, rig.root, &mut assets);
        let clips = animator.clips();
        // 200 cm in 1 s on a 100 cm-tall source is 2 m/s on the 1 m-tall target.
        let walk = clips.iter().find(|c| c.name == "mixamo.com").unwrap();
        assert!((walk.root_speed - 2.0).abs() < 1e-3, "{}", walk.root_speed);
        let wave = clips.iter().find(|c| c.name == "wave").unwrap();
        assert_eq!(wave.root_speed, 0.0);
    }

    #[test]
    fn blends_share_a_phase_and_run_backwards_clips_backwards() {
        let (mut rig, mut assets) = setup(Animator::with_library("walk.glb"));
        let mut animator = rig.world.remove_one::<Animator>(rig.root).unwrap();
        animator.play_blend(
            "Locomotion",
            &[
                BlendClip {
                    clip: "mixamo.com".into(),
                    weight: 0.75,
                    speed: 1.0,
                },
                BlendClip {
                    clip: "wave".into(),
                    weight: 0.25,
                    speed: -1.0,
                },
            ],
            BlendOptions {
                fade: 0.0,
                ..BlendOptions::default()
            },
        );
        rig.world.insert_one(rig.root, animator).unwrap();
        update_animators(&mut rig.world, &mut assets, 0.25);
        let animator = rig.world.get::<&Animator>(rig.root).unwrap();
        let clips = &animator.state.layers[0].clips;
        let walk = clips.iter().find(|c| c.name == "mixamo.com").unwrap();
        let wave = clips.iter().find(|c| c.name == "wave").unwrap();
        assert!((walk.time - 0.25).abs() < 1e-4, "{}", walk.time);
        assert!((wave.time - 0.75).abs() < 1e-4, "{}", wave.time);
        assert_eq!(walk.effective_weight(), 0.75);
        let (progress, finished) = animator.blend_progress("Locomotion", 0).unwrap();
        assert!((progress - 0.25).abs() < 1e-4 && !finished);
    }

    #[test]
    fn clips_retarget_to_skeletons_resting_with_other_axes() {
        // One rig exported twice. The clip's file has no turn above the hips and joints
        // at rest without rotation. The skeleton's armature turns -90 degrees about X
        // (its hips rest +90 to stand up again), and its spine joint rests turned about
        // Z (other bone axes). Posed the same in the world, its keys come out as
        // `pre * key * post`.
        let key = Quat::from_rotation_y(0.5);
        let mut hips = BoneTrack::new("Hips");
        hips.rotation = Some(linear_quat(&[0.0, 1.0], &[key, key]));
        hips.translation = Some(linear_vec3(
            &[0.0, 1.0],
            &[Vec3::Y * 100.0, Vec3::Y * 100.0],
        ));
        let mut spine = BoneTrack::new("Spine");
        spine.rotation = Some(linear_quat(&[0.0, 1.0], &[key, key]));
        let rest = |t: Vec3, depth| BoneRest {
            transform: Transform::from_translation(t),
            depth,
            parent_rotation: Quat::IDENTITY,
        };
        let library = ClipLibrary {
            source: PathBuf::from("dance.glb"),
            clips: vec![Arc::new(SkeletalClip::new("dance", vec![hips, spine]))],
            rest: HashMap::from([
                ("hips".to_string(), rest(Vec3::Y * 100.0, 1)),
                ("spine".to_string(), rest(Vec3::Y * 20.0, 2)),
            ]),
        };
        let quarter = std::f32::consts::FRAC_PI_2;
        let spine_rest = Quat::from_rotation_z(0.3);
        let pose_with = |other_axes: bool| {
            let mut assets = Assets::new();
            assets.insert_clip_library("dance.glb", library.clone());
            let mut rig = rig(1.0);
            if other_axes {
                let set = |world: &mut World, entity, rotation| {
                    world
                        .get::<&mut TransformComponent>(entity)
                        .unwrap()
                        .0
                        .rotation = rotation;
                };
                set(&mut rig.world, rig.root, Quat::from_rotation_x(-quarter));
                set(&mut rig.world, rig.hips, Quat::from_rotation_x(quarter));
                set(&mut rig.world, rig.spine, spine_rest);
                // The armature node came from the model's file.
                rig.world
                    .insert_one(rig.root, GltfSource(PathBuf::from("soldier.glb")))
                    .unwrap();
            }
            let animator = Animator {
                default_clip: Some("dance".into()),
                ..Animator::with_library("dance.glb")
            };
            rig.world.insert_one(rig.root, animator).unwrap();
            update_animators(&mut rig.world, &mut assets, 0.25);
            (local(&rig.world, rig.hips), local(&rig.world, rig.spine))
        };

        let (hips, spine) = pose_with(false);
        assert!(hips.rotation.abs_diff_eq(key, 1e-4), "same rig: as keyed");
        assert!(spine.rotation.abs_diff_eq(key, 1e-4));

        let (hips, spine) = pose_with(true);
        let up = Quat::from_rotation_x(quarter);
        assert!(
            hips.rotation.abs_diff_eq(up * key, 1e-4),
            "{:?}",
            hips.rotation
        );
        assert!(
            hips.translation.abs_diff_eq(up * Vec3::Y, 1e-4),
            "{}",
            hips.translation
        );
        assert!(
            spine.rotation.abs_diff_eq(key * spine_rest, 1e-4),
            "{:?}",
            spine.rotation
        );
    }

    #[test]
    fn a_blend_entered_with_a_fade_fades_in() {
        let (mut rig, mut assets) = setup(Animator::with_library("walk.glb"));
        let mut animator = rig.world.remove_one::<Animator>(rig.root).unwrap();
        let options = BlendOptions {
            fade: 0.2,
            ..BlendOptions::default()
        };
        let wave = [BlendClip {
            clip: "wave".into(),
            weight: 1.0,
            speed: 1.0,
        }];
        animator.play_blend("Wave", &wave, options);
        rig.world.insert_one(rig.root, animator).unwrap();
        for _ in 0..3 {
            // A tree plays its state's blend every frame.
            rig.world
                .get::<&mut Animator>(rig.root)
                .unwrap()
                .play_blend("Wave", &wave, options);
            update_animators(&mut rig.world, &mut assets, 0.1);
        }
        let animator = rig.world.get::<&Animator>(rig.root).unwrap();
        let playing = &animator.state.layers[0].clips[0];
        assert_eq!(playing.name, "wave");
        assert_eq!(playing.effective_weight(), 1.0);
    }

    #[test]
    fn translations_are_retargeted_to_the_skeleton_scale() {
        let (mut rig, mut assets) = setup(Animator {
            default_clip: Some("mixamo.com".into()),
            ..Animator::with_library("walk.glb")
        });
        update_animators(&mut rig.world, &mut assets, 0.5);
        // 110 cm on a 100 cm-tall source becomes 1.1 m on a 1 m-tall target.
        let hips = local(&rig.world, rig.hips);
        assert!(
            hips.translation.abs_diff_eq(Vec3::new(0.0, 1.1, 1.0), 1e-4),
            "{}",
            hips.translation
        );
        // Bones the clip does not animate keep their rest pose.
        assert_eq!(
            local(&rig.world, rig.arm),
            Transform::from_translation(Vec3::X * 0.3)
        );
        let spine = local(&rig.world, rig.spine);
        assert!(spine
            .rotation
            .abs_diff_eq(Quat::from_rotation_y(std::f32::consts::FRAC_PI_4), 1e-4));
    }

    #[test]
    fn cross_fade_blends_from_the_rest_pose_and_between_clips() {
        let (mut rig, mut assets) = setup(Animator::with_library("walk.glb"));
        rig.world.get::<&mut Animator>(rig.root).unwrap().play(
            "mixamo.com",
            PlayOptions {
                fade: 1.0,
                ..PlayOptions::default()
            },
        );
        update_animators(&mut rig.world, &mut assets, 0.5);
        // Halfway through the fade: half rest (1.0), half clip at t = 0.5 (1.1).
        let hips = local(&rig.world, rig.hips);
        assert!(
            (hips.translation.y - 1.05).abs() < 1e-4,
            "{}",
            hips.translation
        );

        update_animators(&mut rig.world, &mut assets, 0.6);
        let status = rig.world.get::<&Animator>(rig.root).unwrap().status(0);
        assert_eq!(status.clip.as_deref(), Some("mixamo.com"));
        assert!(!status.fading);
        assert!((status.time - 0.1).abs() < 1e-4, "looped: {}", status.time);

        // Stopping fades back to the rest pose.
        rig.world
            .get::<&mut Animator>(rig.root)
            .unwrap()
            .stop(None, 0.0);
        update_animators(&mut rig.world, &mut assets, 0.1);
        assert_eq!(local(&rig.world, rig.hips).translation, Vec3::Y);
    }

    #[test]
    fn non_looping_clips_finish_and_replay_from_the_start() {
        let (mut rig, mut assets) = setup(Animator::with_library("walk.glb"));
        let once = PlayOptions {
            fade: 0.0,
            looping: false,
            ..PlayOptions::default()
        };
        rig.world
            .get::<&mut Animator>(rig.root)
            .unwrap()
            .play("wave", once);
        update_animators(&mut rig.world, &mut assets, 2.0);
        let status = rig.world.get::<&Animator>(rig.root).unwrap().status(0);
        assert!(status.finished && !status.playing);
        assert_eq!(status.time, 1.0);

        rig.world
            .get::<&mut Animator>(rig.root)
            .unwrap()
            .play("wave", once);
        update_animators(&mut rig.world, &mut assets, 0.25);
        let status = rig.world.get::<&Animator>(rig.root).unwrap().status(0);
        assert!(status.playing);
        assert!((status.time - 0.25).abs() < 1e-5);
    }

    #[test]
    fn in_place_keeps_vertical_motion_and_apply_moves_the_entity() {
        let (mut rig, mut assets) = setup(Animator {
            default_clip: Some("mixamo.com".into()),
            root_motion: RootMotion::InPlace,
            ..Animator::with_library("walk.glb")
        });
        update_animators(&mut rig.world, &mut assets, 0.5);
        let hips = local(&rig.world, rig.hips);
        assert!(hips.translation.abs_diff_eq(Vec3::new(0.0, 1.1, 0.0), 1e-4));
        assert_eq!(local(&rig.world, rig.root).translation, Vec3::ZERO);

        let (mut rig, mut assets) = setup(Animator {
            default_clip: Some("mixamo.com".into()),
            root_motion: RootMotion::Apply,
            ..Animator::with_library("walk.glb")
        });
        // First update establishes the start; then 0.5 s of travel, then a wrap.
        update_animators(&mut rig.world, &mut assets, 0.0);
        update_animators(&mut rig.world, &mut assets, 0.5);
        let root = local(&rig.world, rig.root).translation;
        assert!(root.abs_diff_eq(Vec3::new(0.0, 0.0, 1.0), 1e-4), "{root}");
        update_animators(&mut rig.world, &mut assets, 0.75);
        let root = local(&rig.world, rig.root).translation;
        // 1.25 s of a 2 m/s walk, across the loop point.
        assert!(root.abs_diff_eq(Vec3::new(0.0, 0.0, 2.5), 1e-3), "{root}");
        let hips = local(&rig.world, rig.hips).translation;
        assert!(
            (hips.z).abs() < 1e-4,
            "the bone itself stays in place: {hips}"
        );
    }

    #[test]
    fn upper_layers_override_masked_bones_only() {
        let (mut rig, mut assets) = setup(Animator {
            default_clip: Some("mixamo.com".into()),
            ..Animator::with_library("walk.glb")
        });
        {
            let mut animator = rig.world.get::<&mut Animator>(rig.root).unwrap();
            animator.set_layer(1, 0.5, 0.0, Some(Some("mixamorig:Spine".into())));
            animator.play(
                "wave",
                PlayOptions {
                    fade: 0.0,
                    layer: 1,
                    ..PlayOptions::default()
                },
            );
        }
        update_animators(&mut rig.world, &mut assets, 0.5);
        // The arm is under the spine: half of the wave layer's rotation.
        let arm = local(&rig.world, rig.arm);
        assert!(arm.rotation.abs_diff_eq(Quat::from_rotation_z(0.5), 1e-4));

        // Masked to a bone the arm is not under: no effect.
        rig.world.get::<&mut Animator>(rig.root).unwrap().set_layer(
            1,
            1.0,
            0.0,
            Some(Some("Hips_Other".into())),
        );
        update_animators(&mut rig.world, &mut assets, 0.1);
        assert_eq!(local(&rig.world, rig.arm).rotation, Quat::IDENTITY);
    }

    #[test]
    fn unknown_clips_are_dropped_and_missing_libraries_reported() {
        let (mut rig, mut assets) = setup(Animator {
            libraries: vec![PathBuf::from("walk.glb"), PathBuf::from("missing.glb")],
            ..Animator::default()
        });
        rig.world
            .get::<&mut Animator>(rig.root)
            .unwrap()
            .play("nope", PlayOptions::default());
        update_animators(&mut rig.world, &mut assets, 0.1);
        let animator = rig.world.get::<&Animator>(rig.root).unwrap();
        assert_eq!(animator.status(0).clip, None);
        assert_eq!(animator.library_errors().len(), 1);
    }

    #[test]
    fn animator_serializes_only_its_settings() {
        let mut animator = Animator {
            default_clip: Some("Walk".into()),
            root_motion: RootMotion::Apply,
            ..Animator::with_library("content/animations/walk.glb")
        };
        animator.play("Walk", PlayOptions::default());
        let json = serde_json::to_value(&animator).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "libraries": ["content/animations/walk.glb"],
                "default_clip": "Walk",
                "root_motion": "apply",
            })
        );
        let back: Animator = serde_json::from_value(json).unwrap();
        assert_eq!(back.speed, 1.0);
        assert!(back.state.layers.is_empty());
    }
}
