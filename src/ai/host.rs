//! [`AiHost`] over a scene: entities' transforms, the navigation's agents and queries,
//! physics rays, animators and animation trees, and Lua (run in the defining script's
//! context through [`ScriptingState::run_in_script`]).

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;

use glam::{Quat, Vec3};
use hecs::{Entity, World};

#[cfg(feature = "navigation")]
use super::context::NavPhase;
use super::context::{AiHost, LineOfSight, LuaCall, NavInfo, PlayRequest};
use super::def::{Callable, Library, LuaFn};
use super::value::{LuaRef, Value};
use crate::scene::components::TransformComponent;
use crate::scene::skeletal::{
    animator_entity, tree_entity, AnimationTree, Animator, AnimatorCommand, PlayOptions,
    TreeCommand,
};
use crate::scripting::lua::api::ai as lua_ai;
use crate::scripting::ScriptingState;

pub(crate) struct SceneHost<'a> {
    pub world: &'a mut World,
    pub scripting: &'a mut ScriptingState,
    #[cfg(feature = "navigation")]
    pub navigation: &'a crate::navigation::SceneNavigation,
    #[cfg(feature = "navigation")]
    pub nav_commands: &'a mut Vec<crate::navigation::NavCommand>,
    pub physics: &'a crate::physics::ScenePhysics,
    pub library: &'a RefCell<Library>,
    /// Problems already reported (each is logged once).
    pub warned: &'a mut HashSet<String>,
    /// The last problem of each agent this step.
    pub errors: &'a mut super::value::EntityMap<String>,
}

fn name_of(world: &World, entity: Entity) -> String {
    world
        .get::<&crate::scene::Name>(entity)
        .map(|name| name.0.clone())
        .unwrap_or_else(|_| format!("entity {}", entity.to_bits().get()))
}

/// Keep the first lines of a Lua error (its traceback goes to the log once).
fn short_error(message: &str) -> String {
    let mut lines = message.lines().filter(|l| !l.trim().is_empty());
    let first = lines.next().unwrap_or_default().trim().to_owned();
    match lines.find(|l| !l.contains("stack traceback")) {
        Some(second) if first.ends_with(':') => format!("{first} {}", second.trim()),
        _ => first,
    }
}

enum Target {
    Key(Arc<LuaFn>),
    Function(mlua::Function, Option<Entity>),
}

impl SceneHost<'_> {
    fn animator<R>(&mut self, entity: Entity, f: impl FnOnce(&mut Animator) -> R) -> Option<R> {
        let owner = animator_entity(self.world, entity)?;
        let mut animator = self.world.get::<&mut Animator>(owner).ok()?;
        Some(f(&mut animator))
    }

    fn tree<R>(&mut self, entity: Entity, f: impl FnOnce(&mut AnimationTree) -> R) -> Option<R> {
        let owner = tree_entity(self.world, entity)?;
        let mut tree = self.world.get::<&mut AnimationTree>(owner).ok()?;
        Some(f(&mut tree))
    }

    fn resolve(&self, function: &Callable, agent: Entity) -> Result<Target, String> {
        match function {
            Callable::Lua(f) => Ok(Target::Key(f.clone())),
            Callable::Named(name) => {
                if let Some(f) = self.scripting.script_function(agent, name) {
                    return Ok(Target::Function(f, Some(agent)));
                }
                if let Some(f) = self.library.borrow().function(name) {
                    return Ok(Target::Key(f));
                }
                if let Ok(Some(f)) = self
                    .scripting
                    .lua()
                    .globals()
                    .get::<Option<mlua::Function>>(&**name)
                {
                    return Ok(Target::Function(f, None));
                }
                Err(format!(
                    "no function called {name:?} (define it in the agent's script or register it with ai_register)"
                ))
            }
        }
    }
}

impl AiHost for SceneHost<'_> {
    fn position(&self, entity: Entity) -> Option<Vec3> {
        self.world
            .contains(entity)
            .then(|| crate::physics::world_transform(self.world, entity).translation)
    }

    fn yaw(&self, entity: Entity) -> Option<f32> {
        if !self.world.contains(entity) {
            return None;
        }
        let forward = crate::physics::world_transform(self.world, entity).rotation * Vec3::Z;
        Some(forward.x.atan2(forward.z))
    }

    fn exists(&self, entity: Entity) -> bool {
        self.world.contains(entity)
    }

    #[cfg(feature = "navigation")]
    fn nav_info(&self, entity: Entity) -> Option<NavInfo> {
        use crate::navigation::AgentState;
        let Some(status) = self.navigation.agent_status(entity) else {
            // An agent the navigation has not taken in yet (spawned this frame).
            return self
                .world
                .satisfies::<&crate::navigation::NavAgent>(entity)
                .unwrap_or(false)
                .then_some(NavInfo {
                    phase: NavPhase::Pending,
                    remaining: None,
                    partial: false,
                    speed: 0.0,
                });
        };
        let phase = match status.state {
            AgentState::Idle => NavPhase::Idle,
            AgentState::Planning | AgentState::Moving | AgentState::OffMesh => NavPhase::Moving,
            AgentState::Arrived => NavPhase::Arrived,
            AgentState::Failed | AgentState::Lost => NavPhase::Failed,
        };
        Some(NavInfo {
            phase,
            remaining: status.remaining_distance,
            partial: status.partial,
            speed: Vec3::new(status.velocity.x, 0.0, status.velocity.z).length(),
        })
    }

    #[cfg(not(feature = "navigation"))]
    fn nav_info(&self, _entity: Entity) -> Option<NavInfo> {
        None
    }

    #[cfg(feature = "navigation")]
    fn nav_move(&mut self, entity: Entity, target: Vec3) -> bool {
        self.nav_commands
            .push(crate::navigation::NavCommand::SetDestination { entity, target });
        true
    }

    #[cfg(not(feature = "navigation"))]
    fn nav_move(&mut self, _entity: Entity, _target: Vec3) -> bool {
        false
    }

    fn nav_stop(&mut self, entity: Entity) {
        #[cfg(feature = "navigation")]
        self.nav_commands
            .push(crate::navigation::NavCommand::Stop { entity });
        #[cfg(not(feature = "navigation"))]
        let _ = entity;
    }

    fn face(&mut self, entity: Entity, yaw: f32) {
        #[cfg(feature = "navigation")]
        if self.navigation.is_agent(entity) {
            self.nav_commands
                .push(crate::navigation::NavCommand::Face { entity, yaw });
            return;
        }
        if let Ok(mut transform) = self.world.get::<&mut TransformComponent>(entity) {
            transform.0.rotation = Quat::from_rotation_y(yaw);
        }
    }

    #[cfg(feature = "navigation")]
    fn nav_random_point(&self, around: Vec3, radius: f32) -> Option<Vec3> {
        self.navigation.random_point(Some((around, radius)), "")
    }

    #[cfg(not(feature = "navigation"))]
    fn nav_random_point(&self, around: Vec3, radius: f32) -> Option<Vec3> {
        let _ = radius;
        Some(around)
    }

    #[cfg(feature = "navigation")]
    fn nav_closest(&self, point: Vec3, radius: f32) -> Option<Vec3> {
        let r = radius.max(0.1);
        self.navigation
            .closest_point(point, Vec3::new(r, r.max(2.0), r), "")
    }

    #[cfg(not(feature = "navigation"))]
    fn nav_closest(&self, point: Vec3, _radius: f32) -> Option<Vec3> {
        Some(point)
    }

    fn line_of_sight(
        &self,
        from: Vec3,
        to: Vec3,
        eye: f32,
        mode: LineOfSight,
        ignore: &[Entity],
    ) -> bool {
        match mode {
            LineOfSight::None => true,
            LineOfSight::Physics => {
                let origin = from + Vec3::Y * eye;
                let target = to + Vec3::Y * eye;
                let span = target - origin;
                let distance = span.length();
                if distance < 1e-3 {
                    return true;
                }
                let options = crate::physics::QueryOptions {
                    exclude: ignore.to_vec(),
                    sensors: false,
                    ignore_dynamic: true,
                };
                self.physics
                    .raycast(origin, span / distance, distance, &options)
                    .is_none_or(|hit| hit.distance >= distance - 0.2)
            }
            #[cfg(feature = "navigation")]
            LineOfSight::Navmesh => match self.navigation.raycast(from, to, "") {
                Some((blocked, end, _)) => !blocked && (end.y - to.y).abs() < 1.0,
                // No navmesh to ask: nothing is known to be in the way.
                None => true,
            },
            #[cfg(not(feature = "navigation"))]
            LineOfSight::Navmesh => true,
        }
    }

    fn agent_speed(&self, entity: Entity) -> Option<f32> {
        #[cfg(feature = "navigation")]
        {
            self.world
                .get::<&crate::navigation::NavAgent>(entity)
                .ok()
                .map(|agent| agent.speed)
        }
        #[cfg(not(feature = "navigation"))]
        {
            let _ = entity;
            None
        }
    }

    fn set_agent_speed(&mut self, entity: Entity, speed: f32) {
        #[cfg(feature = "navigation")]
        if let Ok(mut agent) = self.world.get::<&mut crate::navigation::NavAgent>(entity) {
            agent.speed = speed;
        }
        #[cfg(not(feature = "navigation"))]
        let _ = (entity, speed);
    }

    fn play(&mut self, entity: Entity, request: &PlayRequest) -> bool {
        let command = AnimatorCommand::Play {
            clip: request.clip.clone(),
            options: PlayOptions {
                fade: request.fade,
                looping: request.looping,
                speed: request.speed,
                layer: request.layer,
                start: Some(0.0),
            },
        };
        let played = self.animator(entity, |animator| animator.apply(command));
        if played.is_none() {
            self.warn(
                entity,
                &format!("play {}: no Animator on the agent", request.clip),
            );
        }
        played.is_some()
    }

    fn stop_animation(&mut self, entity: Entity, layer: Option<usize>, fade: f32) {
        self.animator(entity, |animator| {
            animator.apply(AnimatorCommand::Stop { layer, fade })
        });
    }

    fn animation_done(&self, entity: Entity, layer: usize, clip: &str) -> bool {
        let Some(owner) = animator_entity(self.world, entity) else {
            return true;
        };
        let Ok(animator) = self.world.get::<&Animator>(owner) else {
            return true;
        };
        let status = animator.status(layer);
        status.clip.as_deref() != Some(clip) || status.finished
    }

    fn set_anim_param(&mut self, entity: Entity, name: &str, value: &Value) {
        let number = match value {
            Value::Bool(b) => f32::from(u8::from(*b)),
            other => other.as_f64().unwrap_or(0.0) as f32,
        };
        self.tree(entity, |tree| {
            tree.apply(TreeCommand::Set(name.to_owned(), number))
        });
    }

    fn anim_trigger(&mut self, entity: Entity, name: &str) {
        self.tree(entity, |tree| {
            tree.apply(TreeCommand::Trigger(name.to_owned()))
        });
    }

    fn call(&mut self, function: &Callable, call: &LuaCall) -> Result<Value, String> {
        let target = self.resolve(function, call.agent)?;
        let owner = match &target {
            Target::Key(f) => f.owner,
            Target::Function(_, owner) => *owner,
        };
        let result = self.scripting.run_in_script(self.world, owner, |lua| {
            let f = match target {
                Target::Key(f) => lua.registry_value::<mlua::Function>(&f.key)?,
                Target::Function(f, _) => f,
            };
            let ctx = lua_ai::make_ctx(lua, call)?;
            let value: mlua::Value = f.call(ctx)?;
            Ok::<Value, mlua::Error>(lua_ai::value_from_lua(lua, value))
        });
        // Walks the function asked for keep their place among the brain's own.
        #[cfg(feature = "navigation")]
        self.nav_commands
            .extend(self.scripting.take_navigation_commands());
        result.map_err(|err| {
            let message = err.to_string();
            if self.warned.len() < 4096 && self.warned.insert(format!("lua:{message}")) {
                log::error!(target: "ai", "{}: {message}", function.label());
            }
            short_error(&message)
        })
    }

    fn field(&mut self, value: &Value, name: &str) -> Value {
        match value {
            Value::Lua(lua_ref) => lua_ai::lua_field(self.scripting.lua(), lua_ref, name),
            other => other.json_field(name),
        }
    }

    fn positions(&mut self, value: &Value) -> Vec<Vec3> {
        let items: Vec<Value> = match value {
            Value::Json(json) => match &**json {
                serde_json::Value::Array(items) => items.iter().map(Value::from_json).collect(),
                _ => Vec::new(),
            },
            Value::Lua(lua_ref) => lua_ai::lua_list(self.scripting.lua(), lua_ref),
            _ => Vec::new(),
        };
        items
            .iter()
            .filter_map(|item| {
                item.as_vec3()
                    .or_else(|| item.as_entity().and_then(|e| self.position(e)))
                    .or_else(|| match item {
                        Value::Lua(r) => lua_ai::lua_vec3(self.scripting.lua(), r),
                        _ => None,
                    })
            })
            .collect()
    }

    fn len(&mut self, value: &Value) -> Option<usize> {
        match value {
            Value::Str(s) => Some(s.chars().count()),
            Value::Json(json) => json.as_array().map(Vec::len),
            Value::Lua(lua_ref) => lua_ai::lua_len(self.scripting.lua(), lua_ref),
            _ => None,
        }
    }

    fn new_memory(&mut self) -> Value {
        let lua = self.scripting.lua();
        lua.create_table()
            .and_then(|table| lua.create_registry_value(table))
            .map(|key| Value::Lua(Arc::new(LuaRef(key))))
            .unwrap_or_default()
    }

    fn emit(&mut self, event: &str, data: &Value) {
        self.scripting.queue_event(event, data.to_json());
    }

    fn log(&mut self, agent: Entity, message: &str) {
        log::info!(target: "ai", "[{}] {message}", name_of(self.world, agent));
    }

    fn warn(&mut self, agent: Entity, message: &str) {
        let name = name_of(self.world, agent);
        // Many agents share a brain, and so its mistakes: each is logged once.
        if self.warned.len() < 4096 && self.warned.insert(message.to_owned()) {
            log::warn!(target: "ai", "[{name}] {message}");
        }
        self.errors.insert(agent, message.to_owned());
    }
}
