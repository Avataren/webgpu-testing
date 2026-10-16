//! "Make Character": turn an imported model into a character you can play, in one step.
//!
//! Like a Godot scene with a CharacterBody3D at its root, the model goes under a new
//! entity that carries the [`CharacterController`] (a capsule sized from the model), an
//! [`AnimationTree`](crate::scene::skeletal::AnimationTree) generated from the model's
//! clips, and optionally a script with the engine's third-person controls
//! (`require("engine.third_person")`). The scene's camera gets a [`SpringCamera`] that
//! follows it.

use glam::{Mat4, Vec3};
use hecs::{Entity, World};

use super::{CharacterController, Collider, SpringCamera};
use crate::scene::{
    CameraComponent, Children, MeshBounds, Name, Parent, Scene, Transform, TransformComponent,
    Visible, WorldTransform,
};
use crate::scripting::{LuaScriptComponent, LuaScriptSource};

/// What [`make_character`] does besides adding the controller.
#[derive(Debug, Clone)]
pub struct MakeCharacterOptions {
    /// Name of the new character entity. Default: "Player", or "<model> Character" when
    /// that name is taken.
    pub name: Option<String>,
    /// Have the scene's camera follow the character with a spring camera (it gets one,
    /// or a camera is made when the scene has none).
    pub camera: bool,
    /// Attach a script with the engine's third-person controls: `Some(path)` writes it
    /// there (project-relative, e.g. `content/scripts/player.lua`; an existing file is
    /// attached as it is), an empty path to `content/scripts/<name>.lua` (a new file),
    /// `None` adds no script.
    pub script: Option<std::path::PathBuf>,
}

impl Default for MakeCharacterOptions {
    fn default() -> Self {
        Self {
            name: None,
            camera: true,
            script: None,
        }
    }
}

/// What [`make_character`] made.
#[derive(Debug, Clone)]
pub struct MadeCharacter {
    /// The new entity with the controller; the model is its child.
    pub character: Entity,
    pub name: String,
    pub model: Entity,
    /// The camera that follows it.
    pub camera: Option<Entity>,
    pub height: f32,
    pub radius: f32,
    /// The generated tree's states; empty when the model's clips gave none.
    pub tree_states: Vec<String>,
    /// The script attached (project-relative), if any.
    pub script: Option<std::path::PathBuf>,
    /// Anything the user should know (missing clips, camera choices).
    pub notes: Vec<String>,
}

/// The script [`make_character`] writes: the engine's third-person controls.
pub fn character_script(camera: &str) -> String {
    format!(
        r#"-- Third-person controls from the engine (engine/third_person.lua; lua_api_reference
-- "Third-person controller" lists the options). The CharacterController moves the body,
-- the AnimationTree picks the animation from how it moves, and this script steers.
--
-- scheme "mmo": W/S move, A/D turn, Q/E strafe, left drag looks around, right drag
-- steers, both buttons run. scheme "action": WASD relative to the camera, click to
-- capture the mouse. Space jumps, Shift walks, the wheel zooms.

local ThirdPerson = require("engine.third_person")
local controls

function on_created(self)
    controls = ThirdPerson.new(self, {{ camera = {camera:?}, scheme = "mmo" }})
end

function update(self, dt)
    controls:update(dt)
end

function on_jumped(self) controls:jumped() end
function on_landed(self, speed) controls:landed(speed) end

function on_destroyed(self, reason)
    if controls then controls:release() end
end
"#
    )
}

/// Put `model` (an imported character: its root, or any entity with meshes below it)
/// under a new character entity. See the [module docs](self).
pub fn make_character(
    scene: &mut Scene,
    model: Entity,
    options: &MakeCharacterOptions,
) -> Result<MadeCharacter, String> {
    scene.propagate_transforms();
    let world = scene.main_world();
    if !world.contains(model) {
        return Err("no such entity".into());
    }
    let mut cursor = Some(model);
    while let Some(current) = cursor {
        if world
            .satisfies::<&CharacterController>(current)
            .unwrap_or(false)
        {
            return Err(format!(
                "{} is already a character",
                name_of(world, current)
            ));
        }
        cursor = world.get::<&Parent>(current).ok().map(|p| p.0);
    }
    let (min, max) = subtree_bounds(world, model)
        .ok_or("no meshes at or below the entity to size the character from")?;
    let height = max.y - min.y;
    if height < 0.05 {
        return Err(format!(
            "the model is too flat to stand ({height:.3} m tall)"
        ));
    }
    let feet = Vec3::new((min.x + max.x) * 0.5, min.y, (min.z + max.z) * 0.5);
    let model_name = name_of(world, model);
    let name = match &options.name {
        Some(name) if !name.trim().is_empty() => unique_name(world, name.trim()),
        _ if !name_taken(world, "Player") => "Player".to_string(),
        _ => unique_name(world, &format!("{model_name} Character")),
    };
    let parent = world.get::<&Parent>(model).ok().map(|p| p.0);
    let parent_matrix = parent
        .and_then(|p| world.get::<&WorldTransform>(p).ok().map(|t| t.0.matrix()))
        .unwrap_or(Mat4::IDENTITY);
    let model_matrix = world
        .get::<&WorldTransform>(model)
        .map(|t| t.0.matrix())
        .unwrap_or(Mat4::IDENTITY);

    // The character stands where the model's feet are, facing +Z (as the controller
    // expects); a model facing elsewhere (its toes say where) is turned to face +Z.
    let character_matrix = Mat4::from_translation(feet);
    let character_local = transform_from(parent_matrix.inverse() * character_matrix);
    let turn = model_facing(world, model).map_or(0.0, |yaw| {
        // Models face along an axis; the feet only roughly say which.
        let quarter = std::f32::consts::FRAC_PI_2;
        -(yaw / quarter).round() * quarter
    });
    let model_local =
        transform_from(Mat4::from_rotation_y(turn) * character_matrix.inverse() * model_matrix);
    let turned_degrees = turn.to_degrees().round().rem_euclid(360.0);

    // Sized for a person 1.8 m tall and scaled with the model (a fox runs slower).
    let size = (height / 1.8).clamp(0.25, 4.0);
    let defaults = CharacterController::default();
    let controller = CharacterController {
        height,
        radius: (height * 0.17).clamp(0.05, height * 0.45),
        step_height: defaults.step_height * size,
        snap_distance: defaults.snap_distance * size,
        walk_speed: defaults.walk_speed * size,
        run_speed: defaults.run_speed * size,
        jump_height: defaults.jump_height * size,
        ..defaults
    };
    let radius = controller.radius;

    let character = scene.main_world_mut().spawn((
        Name::new(name.clone()),
        TransformComponent(character_local),
        WorldTransform(Transform::from_trs(feet, glam::Quat::IDENTITY, Vec3::ONE)),
        Visible(true),
        controller,
    ));
    let world = scene.main_world_mut();
    set_parent(world, character, parent)?;
    set_parent(world, model, Some(character))?;
    if let Ok(mut transform) = world.get::<&mut TransformComponent>(model) {
        transform.0 = model_local;
    }
    scene.propagate_transforms();

    let mut notes = Vec::new();
    if turned_degrees != 0.0 {
        notes.push(format!(
            "turned the model {turned_degrees:.0}° to face +Z, the way characters walk"
        ));
    }
    if scene
        .main_world()
        .query::<&Collider>()
        .iter()
        .next()
        .is_none()
    {
        notes.push(
            "nothing in the scene has a Collider, so the character falls through the floor: \
             give the floor one (Add Component > Physics > Collider)"
                .into(),
        );
    }
    let tree_states = match scene.locomotion_tree(character) {
        Ok(tree) => {
            let states = tree.states.iter().map(|s| s.name.clone()).collect();
            scene
                .main_world_mut()
                .insert_one(character, tree)
                .map_err(|err| err.to_string())?;
            states
        }
        Err(err) => {
            notes.push(format!(
                "no AnimationTree: {err}. Add clips to the model's Animator, then use \
                 \"Generate from clips\" on the character"
            ));
            Vec::new()
        }
    };

    // The controller moves the character; clips that travel (Mixamo's without "In
    // Place") would drag the hips away from it. Their speeds were measured already.
    if scene.play_travelling_clips_in_place(character) {
        notes.push(
            "the clips travel, so the Animator plays them in place (the controller moves the \
             character)"
                .into(),
        );
    }

    let camera = if options.camera {
        Some(follow_with_camera(scene, &name, height, &mut notes)?)
    } else {
        None
    };

    let script = match &options.script {
        Some(path) => {
            let path = if path.as_os_str().is_empty() {
                new_script_path(&name)
            } else {
                path.clone()
            };
            let camera_name = camera
                .map(|c| name_of(scene.main_world(), c))
                .unwrap_or_default();
            write_script(&path, &camera_name, &mut notes)?;
            scene
                .main_world_mut()
                .insert_one(
                    character,
                    // In memory scripts point at their file; saving makes the path
                    // project-relative again.
                    LuaScriptComponent::new(LuaScriptSource::file(
                        crate::project::resolve_project_path(&path),
                    )),
                )
                .map_err(|err| err.to_string())?;
            Some(path)
        }
        None => None,
    };

    Ok(MadeCharacter {
        character,
        name,
        model,
        camera,
        height,
        radius,
        tree_states,
        script,
        notes,
    })
}

/// A spring camera on the scene's camera (or a new one) following `target`.
fn follow_with_camera(
    scene: &mut Scene,
    target: &str,
    height: f32,
    notes: &mut Vec<String>,
) -> Result<Entity, String> {
    let size = (height / 1.8).clamp(0.25, 4.0);
    let spring = SpringCamera {
        target: target.to_string(),
        pivot_offset: [0.0, height * 0.83, 0.0],
        distance: 4.5 * size,
        min_distance: 1.5 * size,
        max_distance: 10.0 * size,
        // Behind a character facing +Z, as imported models do.
        yaw: 180.0,
        zoom_step: 0.6 * size,
        probe_radius: 0.2 * size.min(1.0),
        ..SpringCamera::default()
    };
    let world = scene.main_world();
    // A camera already following someone keeps doing so.
    let existing = world
        .query::<&SpringCamera>()
        .iter()
        .find(|(_, camera)| camera.target == target)
        .map(|(entity, _)| entity);
    if let Some(entity) = existing {
        return Ok(entity);
    }
    let active = scene
        .active_camera_entity()
        .filter(|&e| world.satisfies::<&CameraComponent>(e).unwrap_or(false));
    let camera = match active {
        Some(entity) if world.get::<&SpringCamera>(entity).is_err() => {
            notes.push(format!(
                "{} follows {target} now (Spring Camera)",
                name_of(world, entity)
            ));
            entity
        }
        other => {
            if let Some(entity) = other {
                notes.push(format!(
                    "{} already follows {}; a new camera follows {target}",
                    name_of(world, entity),
                    world
                        .get::<&SpringCamera>(entity)
                        .map(|c| c.target.clone())
                        .unwrap_or_default()
                ));
            }
            let name = unique_name(world, &format!("{target} Camera"));
            let entity = scene.main_world_mut().spawn((
                Name::new(name),
                TransformComponent(Transform::from_trs(
                    Vec3::new(0.0, height * 1.5, -4.5 * size),
                    glam::Quat::IDENTITY,
                    Vec3::ONE,
                )),
                CameraComponent::perspective(60f32.to_radians(), 0.05, 500.0),
                Visible(true),
            ));
            scene.set_active_camera_entity(Some(entity));
            entity
        }
    };
    scene
        .main_world_mut()
        .insert_one(camera, spring)
        .map_err(|err| err.to_string())?;
    Ok(camera)
}

/// `content/scripts/<name>.lua`, numbered when that file exists.
fn new_script_path(name: &str) -> std::path::PathBuf {
    let stem: String = name
        .trim()
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let stem = if stem.is_empty() {
        "character".into()
    } else {
        stem
    };
    let root = crate::project::active_project_root().unwrap_or_default();
    (1..)
        .map(|index| {
            std::path::PathBuf::from(if index == 1 {
                format!("content/scripts/{stem}.lua")
            } else {
                format!("content/scripts/{stem}_{index}.lua")
            })
        })
        .find(|path| !root.join(path).exists())
        .unwrap_or_default()
}

fn write_script(
    path: &std::path::Path,
    camera: &str,
    notes: &mut Vec<String>,
) -> Result<(), String> {
    let Some(root) = crate::project::active_project_root() else {
        return Err("open a project to write the character's script into".into());
    };
    let file = root.join(path);
    if file.exists() {
        notes.push(format!(
            "{} exists and was attached as it is",
            path.display()
        ));
        return Ok(());
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
    }
    std::fs::write(&file, character_script(camera))
        .map_err(|err| format!("cannot write {}: {err}", file.display()))
}

fn name_of(world: &World, entity: Entity) -> String {
    world
        .get::<&Name>(entity)
        .map(|name| name.0.clone())
        .unwrap_or_else(|_| format!("Entity {}", entity.id()))
}

fn name_taken(world: &World, name: &str) -> bool {
    world.query::<&Name>().iter().any(|(_, n)| n.0 == name)
}

fn unique_name(world: &World, base: &str) -> String {
    if !name_taken(world, base) {
        return base.to_string();
    }
    (2..)
        .map(|index| format!("{base} {index}"))
        .find(|name| !name_taken(world, name))
        .unwrap_or_else(|| base.to_string())
}

fn transform_from(matrix: Mat4) -> Transform {
    let (scale, rotation, translation) = matrix.to_scale_rotation_translation();
    Transform::from_trs(translation, rotation, scale)
}

/// The direction a humanoid model faces (yaw, radians; 0 is +Z), from where its toe bones
/// sit in front of its foot bones.
fn model_facing(world: &World, root: Entity) -> Option<f32> {
    let mut feet = Vec::new();
    let mut toes = Vec::new();
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let Ok(name) = world.get::<&Name>(entity) {
            let name = crate::scene::skeletal::normalize_bone_name(&name.0);
            let side = if name.starts_with("left") || name.ends_with(".l") || name.ends_with("_l") {
                Some(0)
            } else if name.starts_with("right") || name.ends_with(".r") || name.ends_with("_r") {
                Some(1)
            } else {
                None
            };
            if let (Some(side), Ok(transform)) = (side, world.get::<&WorldTransform>(entity)) {
                let at = transform.0.translation;
                if name.contains("toe") {
                    toes.push((side, at));
                } else if name.contains("foot") {
                    feet.push((side, at));
                }
            }
        }
        if let Ok(children) = world.get::<&Children>(entity) {
            stack.extend(children.0.iter().copied());
        }
    }
    let mut forward = Vec3::ZERO;
    for (side, foot) in &feet {
        if let Some((_, toe)) = toes.iter().find(|(toe_side, _)| toe_side == side) {
            forward += *toe - *foot;
        }
    }
    forward.y = 0.0;
    (forward.length() > 1e-4).then(|| forward.x.atan2(forward.z))
}

/// Where a mesh's vertices go in the world. A skinned mesh is placed by its joints (its
/// node's transform does not count): at rest, joint × inverse bind matrix, the same for
/// every joint.
fn mesh_matrix(world: &World, entity: Entity) -> Option<Mat4> {
    if let Ok(skin) = world.get::<&crate::scene::skeletal::SkinnedMesh>(entity) {
        let joint = skin
            .joints
            .first()
            .and_then(|&joint| world.get::<&WorldTransform>(joint).ok());
        if let (Some(joint), Some(inverse_bind)) = (joint, skin.inverse_bind.first()) {
            return Some(joint.0.matrix() * *inverse_bind);
        }
    }
    world
        .get::<&WorldTransform>(entity)
        .ok()
        .map(|transform| transform.0.matrix())
}

/// World-space box around the meshes at and below `root`.
fn subtree_bounds(world: &World, root: Entity) -> Option<(Vec3, Vec3)> {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    let mut stack = vec![root];
    while let Some(entity) = stack.pop() {
        if let (Ok(bounds), Some(matrix)) =
            (world.get::<&MeshBounds>(entity), mesh_matrix(world, entity))
        {
            for i in 0..8 {
                let corner = Vec3::new(
                    if i & 1 == 0 {
                        bounds.min.x
                    } else {
                        bounds.max.x
                    },
                    if i & 2 == 0 {
                        bounds.min.y
                    } else {
                        bounds.max.y
                    },
                    if i & 4 == 0 {
                        bounds.min.z
                    } else {
                        bounds.max.z
                    },
                );
                let point = matrix.transform_point3(corner);
                min = min.min(point);
                max = max.max(point);
            }
        }
        if let Ok(children) = world.get::<&Children>(entity) {
            stack.extend(children.0.iter().copied());
        }
    }
    (min.is_finite() && max.is_finite()).then_some((min, max))
}

/// Re-parent `entity` (keeping its local transform), updating both `Children` lists.
fn set_parent(world: &mut World, entity: Entity, parent: Option<Entity>) -> Result<(), String> {
    if let Ok(old) = world.get::<&Parent>(entity).map(|p| p.0) {
        let mut now_empty = false;
        if let Ok(mut children) = world.get::<&mut Children>(old) {
            children.0.retain(|child| *child != entity);
            now_empty = children.0.is_empty();
        }
        if now_empty {
            let _ = world.remove_one::<Children>(old);
        }
    }
    match parent {
        Some(parent) => {
            world
                .insert_one(entity, Parent(parent))
                .map_err(|err| err.to_string())?;
            let appended = world
                .get::<&mut Children>(parent)
                .map(|mut children| {
                    if !children.0.contains(&entity) {
                        children.0.push(entity);
                    }
                })
                .is_ok();
            if !appended {
                world
                    .insert_one(parent, Children(vec![entity]))
                    .map_err(|err| err.to_string())?;
            }
        }
        None => {
            let _ = world.remove_one::<Parent>(entity);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_model_faces_where_its_toes_point() {
        let mut world = World::new();
        let at = |x: f32, y: f32, z: f32| {
            WorldTransform(Transform::from_trs(
                Vec3::new(x, y, z),
                glam::Quat::IDENTITY,
                Vec3::ONE,
            ))
        };
        let bones = [
            ("mixamorig:LeftFoot", at(0.1, 0.1, 0.0)),
            ("mixamorig:LeftToeBase", at(0.12, 0.02, -0.15)),
            ("foot.R", at(-0.1, 0.1, 0.0)),
            ("toe.R", at(-0.12, 0.02, -0.15)),
            ("Hips", at(0.0, 1.0, 0.0)),
        ];
        let children: Vec<Entity> = bones
            .into_iter()
            .map(|(name, transform)| world.spawn((Name::new(name), transform)))
            .collect();
        let root = world.spawn((Name::new("Soldier"), at(0.0, 0.0, 0.0), Children(children)));
        let yaw = model_facing(&world, root).unwrap();
        assert!(
            (yaw.abs() - std::f32::consts::PI).abs() < 1e-3,
            "faces -Z: {yaw}"
        );

        let bare = world.spawn((Name::new("Crate"), at(0.0, 0.0, 0.0)));
        assert!(model_facing(&world, bare).is_none());
    }

    #[test]
    fn the_script_requires_the_built_in_controls() {
        let script = character_script("Main Camera");
        assert!(script.contains("require(\"engine.third_person\")"));
        assert!(script.contains("camera = \"Main Camera\""));
    }
}
