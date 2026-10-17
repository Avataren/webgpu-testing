//! Animator and AnimationTree edits from the inspector, and "Make Character".

use std::path::PathBuf;

use hecs::Entity;
use wgpu_cube::app::RuntimeMode;
use wgpu_cube::scene::skeletal::{tree_animator, AnimationTree, Animator, AnimatorCommand};

use super::{ActionContext, ActionResult};
use crate::application::core::ReloadNotification;
use crate::application::EditorCommand;

fn notify(ctx: &mut ActionContext, notification: ReloadNotification) {
    ctx.app.shared.reload_notifications.push(notification);
}

/// Handle SetAnimator: the saved fields change, playback carries on (a changed library
/// list rebinds the clips).
pub fn handle_set_animator(
    ctx: &mut ActionContext,
    entity: Entity,
    animator: Option<Box<Animator>>,
) -> ActionResult {
    let world = ctx.scene.main_world_mut();
    let Some(edited) = animator else {
        return match world.remove_one::<Animator>(entity) {
            Ok(_) => ActionResult::scene_changed(),
            Err(_) => ActionResult::no_change(),
        };
    };
    if let Ok(mut current) = world.get::<&mut Animator>(entity) {
        if *current == *edited {
            return ActionResult::no_change();
        }
        current.libraries = edited.libraries;
        current.default_clip = edited.default_clip;
        current.speed = edited.speed;
        current.root_motion = edited.root_motion;
        return ActionResult::scene_changed();
    }
    match world.insert_one(entity, *edited) {
        Ok(()) => ActionResult::scene_changed(),
        Err(err) => {
            log::warn!("Cannot add an animator to {entity:?}: {err}");
            ActionResult::no_change()
        }
    }
}

/// Handle AddAnimationLibrary: on the animator `entity`'s tree drives (or its own),
/// making one on `entity` when there is none.
pub fn handle_add_animation_library(
    ctx: &mut ActionContext,
    entity: Entity,
    path: PathBuf,
) -> ActionResult {
    let world = ctx.scene.main_world_mut();
    if !world.contains(entity) {
        return ActionResult::no_change();
    }
    let target = tree_animator(world, entity).unwrap_or(entity);
    if let Ok(mut animator) = world.get::<&mut Animator>(target) {
        if animator.libraries.contains(&path) {
            return ActionResult::no_change();
        }
        animator.apply(AnimatorCommand::AddLibrary(path));
        return ActionResult::scene_changed();
    }
    let animator = Animator {
        libraries: vec![path],
        ..Animator::default()
    };
    match world.insert_one(target, animator) {
        Ok(()) => ActionResult::scene_changed(),
        Err(err) => {
            log::warn!("Cannot add an animator to {target:?}: {err}");
            ActionResult::no_change()
        }
    }
}

/// Handle ImportAnimations: import the file (through Blender for FBX), then add it.
pub fn handle_import_animations(
    ctx: &mut ActionContext,
    entity: Entity,
    path: PathBuf,
) -> ActionResult {
    ctx.app.enqueue_command(EditorCommand::ImportClips {
        path,
        attach: Some(entity),
    });
    ActionResult::no_change()
}

/// Handle SetAnimationTree: the saved fields change and the tree carries on playing
/// (from its start state if its current state was removed).
pub fn handle_set_animation_tree(
    ctx: &mut ActionContext,
    entity: Entity,
    tree: Option<Box<AnimationTree>>,
) -> ActionResult {
    let world = ctx.scene.main_world_mut();
    let Some(edited) = tree else {
        return match world.remove_one::<AnimationTree>(entity) {
            Ok(_) => ActionResult::scene_changed(),
            Err(_) => ActionResult::no_change(),
        };
    };
    if let Ok(mut current) = world.get::<&mut AnimationTree>(entity) {
        if *current == *edited {
            return ActionResult::no_change();
        }
        let runtime = std::mem::take(&mut current.runtime);
        *current = *edited;
        current.runtime = runtime;
        return ActionResult::scene_changed();
    }
    match world.insert_one(entity, *edited) {
        Ok(()) => ActionResult::scene_changed(),
        Err(err) => {
            log::warn!("Cannot add an animation tree to {entity:?}: {err}");
            ActionResult::no_change()
        }
    }
}

/// Handle GenerateAnimationTree: a locomotion tree from the animator's clips by name.
pub fn handle_generate_animation_tree(ctx: &mut ActionContext, entity: Entity) -> ActionResult {
    let generated = match ctx.scene.locomotion_tree(entity) {
        Ok(tree) => tree,
        Err(err) => {
            notify(
                ctx,
                ReloadNotification::warning(format!("Cannot generate an animation tree: {err}")),
            );
            return ActionResult::no_change();
        }
    };
    let world = ctx.scene.main_world_mut();
    let mut tree = world
        .get::<&AnimationTree>(entity)
        .map(|tree| (*tree).clone())
        .unwrap_or_default();
    tree.start = generated.start;
    tree.states = generated.states;
    tree.transitions = generated.transitions;
    tree.measured_scale = generated.measured_scale;
    tree.runtime = Default::default();
    let states: Vec<&str> = tree.states.iter().map(|s| s.name.as_str()).collect();
    let mut message = format!("Animation tree: {}", states.join(", "));
    if let Err(err) = world.insert_one(entity, tree) {
        log::warn!("Cannot add an animation tree to {entity:?}: {err}");
        return ActionResult::no_change();
    }
    if ctx.scene.play_travelling_clips_in_place(entity) {
        message.push_str("; the clips travel, so the Animator now plays them in place");
    }
    notify(ctx, ReloadNotification::success(message));
    ActionResult::scene_changed()
}

/// Handle MakeCharacter (see `wgpu_cube::physics::make_character`), then select the
/// new character.
pub fn handle_make_character(
    ctx: &mut ActionContext,
    entity: Entity,
    script: bool,
) -> ActionResult {
    if ctx.app.shared.runtime_state.active_mode() == RuntimeMode::Playing {
        notify(
            ctx,
            ReloadNotification::warning("Stop play mode to make a character"),
        );
        return ActionResult::no_change();
    }
    let options = wgpu_cube::physics::MakeCharacterOptions {
        script: script.then(PathBuf::new),
        ..Default::default()
    };
    let made = match wgpu_cube::physics::make_character(ctx.scene, entity, &options) {
        Ok(made) => made,
        Err(err) => {
            notify(
                ctx,
                ReloadNotification::error(format!("Make Character: {err}")),
            );
            return ActionResult::no_change();
        }
    };
    let mut message = format!(
        "{} is a character ({:.2} m tall, radius {:.2} m)",
        made.name, made.height, made.radius
    );
    if !made.tree_states.is_empty() {
        message.push_str(&format!(
            "; animation tree: {}",
            made.tree_states.join(", ")
        ));
    }
    if let Some(path) = &made.script {
        message.push_str(&format!("; controls in {}", path.display()));
    }
    log::info!("{message}");
    notify(ctx, ReloadNotification::success(message));
    for note in made.notes {
        log::info!("Make Character: {note}");
        notify(ctx, ReloadNotification::warning(note));
    }
    let selection = ctx.app.selection_system_mut();
    selection.set_selected(Some(made.character));
    selection.set_highlighted(Some(made.character));
    selection.request_override(Some(made.character));
    ActionResult::scene_changed()
}
