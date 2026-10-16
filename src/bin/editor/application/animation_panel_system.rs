use std::path::PathBuf;

use hecs::{Entity, World};
use wgpu_cube::app::RuntimeMode;
use wgpu_cube::physics::CharacterController;
use wgpu_cube::scene::skeletal::{tree_animator, AnimationTree, Animator, ClipInfo};
use wgpu_cube::scene::Parent;

use super::system::{EditorContext, EditorSystem};
use crate::inspector::{AnimationPanelStatus, TreeLive};

/// Works out what the inspector's animation sections show for the selected entity: the
/// clips its animator can play (bound once per animator and library list) and, in play
/// mode, the animation tree's state and the clips playing.
#[derive(Default)]
pub(crate) struct AnimationPanelSystem {
    /// What `clips` was worked out for: selection, animator, its libraries, play mode.
    key: Option<(Entity, Entity, Vec<PathBuf>, bool)>,
    clips: Vec<ClipInfo>,
    /// Frames since the clips were worked out, to try again now and then while there
    /// are none (they bind a frame after play starts, or once a library loads).
    age: u32,
}

impl EditorSystem for AnimationPanelSystem {
    fn gpu_update<'app, 'ctx, 'scene>(&mut self, ctx: &mut EditorContext<'app, 'ctx, 'scene>) {
        ctx.with_gpu_app(|app, gpu| {
            let selected = app.selection_system().selected();
            let is_playing = app.runtime_state().active_mode() == RuntimeMode::Playing;
            let Some(entity) = selected.filter(|&e| gpu.scene.main_world().contains(e)) else {
                *app.animation_status_mut() = AnimationPanelStatus::default();
                self.key = None;
                return;
            };
            let world = gpu.scene.main_world();
            let animator = tree_animator(world, entity);
            let libraries = animator
                .and_then(|owner| world.get::<&Animator>(owner).ok())
                .map(|animator| animator.libraries.clone())
                .unwrap_or_default();
            let key = animator.map(|owner| (entity, owner, libraries, is_playing));
            self.age = self.age.saturating_add(1);
            if key != self.key || (self.clips.is_empty() && self.age > 30) {
                self.clips = if key.is_some() {
                    gpu.scene.animator_clips(entity).unwrap_or_default()
                } else {
                    Vec::new()
                };
                self.key = key;
                self.age = 0;
            }

            let world = gpu.scene.main_world();
            let live = is_playing
                .then(|| world.get::<&AnimationTree>(entity).ok())
                .flatten()
                .map(|tree| {
                    let mut parameters: Vec<(String, f32)> = tree
                        .runtime
                        .values
                        .iter()
                        .map(|(name, value)| (name.clone(), *value))
                        .collect();
                    parameters.sort_by(|a, b| a.0.cmp(&b.0));
                    TreeLive {
                        state: tree.runtime.state.clone(),
                        previous: tree.runtime.previous.clone(),
                        time: tree.runtime.time_in_state,
                        progress: tree.runtime.progress,
                        parameters,
                    }
                });
            let playing = if is_playing {
                animator
                    .and_then(|owner| world.get::<&Animator>(owner).ok())
                    .and_then(|animator| {
                        animator.state.layers.first().map(|layer| {
                            layer
                                .clips
                                .iter()
                                .map(|clip| (clip.name.clone(), clip.effective_weight()))
                                .filter(|(_, weight)| *weight > 0.01)
                                .collect()
                        })
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            let can_make_character = animator
                .is_some_and(|owner| is_at_or_below(world, owner, entity))
                && !has_controller_at_or_above(world, entity)
                && !has_controller_below(world, entity);

            *app.animation_status_mut() = AnimationPanelStatus {
                entity: Some(entity),
                animator,
                clips: self.clips.clone(),
                live,
                playing,
                can_make_character,
                is_playing,
            };
        });
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
}

fn parent(world: &World, entity: Entity) -> Option<Entity> {
    world.get::<&Parent>(entity).ok().map(|parent| parent.0)
}

fn is_at_or_below(world: &World, entity: Entity, root: Entity) -> bool {
    let mut current = Some(entity);
    for _ in 0..256 {
        match current {
            Some(e) if e == root => return true,
            Some(e) => current = parent(world, e),
            None => return false,
        }
    }
    false
}

fn has_controller_at_or_above(world: &World, entity: Entity) -> bool {
    let mut current = Some(entity);
    for _ in 0..256 {
        let Some(e) = current else {
            return false;
        };
        if world.satisfies::<&CharacterController>(e).unwrap_or(false) {
            return true;
        }
        current = parent(world, e);
    }
    false
}

fn has_controller_below(world: &World, root: Entity) -> bool {
    world
        .query::<&CharacterController>()
        .iter()
        .any(|(e, _)| e != root && is_at_or_below(world, e, root))
}
