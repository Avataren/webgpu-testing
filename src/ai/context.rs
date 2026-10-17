//! What a brain sees while it ticks: its agent, blackboards, events, senses and the
//! world (through [`AiHost`], which the scene implements over its entities, navigation,
//! animators and Lua, and tests over a toy world).

use std::sync::Arc;

use glam::Vec3;
use hecs::Entity;
use rand::rngs::SmallRng;
use rand::Rng;

use super::blackboard::{split_key, BlackboardRef, BoardScope};
use super::def::{Callable, Cond, Library, Num, Operand};
use super::expr::{ExprEnv, Function};
use super::perception::Senses;
use super::value::Value;

/// An event delivered to agents: sent by a brain or a script, or from their senses
/// (`see`, `see:<tag>`, `lose`, `lose:<tag>`, `hear`, `hear:<tag>`).
#[derive(Debug, Clone)]
pub struct AiEvent {
    pub name: Arc<str>,
    pub data: Value,
    /// Who sent it (or what was seen or heard).
    pub from: Option<Entity>,
    /// Where it happened (a noise, a sighting, the sender).
    pub position: Option<Vec3>,
}

impl AiEvent {
    pub fn new(name: impl AsRef<str>, data: Value) -> Self {
        Self {
            name: Arc::from(name.as_ref()),
            data,
            from: None,
            position: None,
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": &*self.name,
            "data": self.data.to_json(),
            "from": self.from.map(|e| e.to_bits().get()),
            "position": self.position.map(|p| [p.x, p.y, p.z]),
        })
    }
}

/// Where an agent's navigation stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavPhase {
    /// An agent the navigation has not taken in yet (spawned this frame).
    Pending,
    Idle,
    Moving,
    Arrived,
    Failed,
}

#[derive(Debug, Clone, Copy)]
pub struct NavInfo {
    pub phase: NavPhase,
    pub remaining: Option<f32>,
    /// The way is blocked for now (it walks as near as it can).
    pub partial: bool,
    pub speed: f32,
}

/// How an agent checks it can see something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineOfSight {
    /// Walk a straight line over the navmesh (walls stop it); other floors are hidden.
    #[default]
    Navmesh,
    /// A ray against physics colliders.
    Physics,
    /// Nothing blocks the view.
    None,
}

/// An animation to play.
#[derive(Debug, Clone)]
pub struct PlayRequest {
    pub clip: String,
    pub layer: usize,
    pub looping: bool,
    pub fade: f32,
    pub speed: f32,
}

/// Which hook a Lua function is called as (its `ctx.phase`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallPhase {
    Run,
    Enter,
    Exit,
    Condition,
    Value,
    StateEnter,
    StateUpdate,
    StateExit,
}

impl CallPhase {
    pub fn name(self) -> &'static str {
        match self {
            CallPhase::Run => "run",
            CallPhase::Enter => "enter",
            CallPhase::Exit => "exit",
            CallPhase::Condition => "condition",
            CallPhase::Value => "value",
            CallPhase::StateEnter => "state_enter",
            CallPhase::StateUpdate => "state_update",
            CallPhase::StateExit => "state_exit",
        }
    }
}

/// What a Lua function gets as `ctx`.
pub struct LuaCall<'a> {
    pub agent: Entity,
    pub board: &'a BlackboardRef,
    pub group: &'a BlackboardRef,
    pub global: &'a BlackboardRef,
    pub dt: f32,
    pub time: f64,
    /// The node or state calling.
    pub node: &'a str,
    pub phase: CallPhase,
    /// The Lua table a `lua` leaf keeps while it runs (`ctx.memory`).
    pub memory: Option<&'a Value>,
    /// The event that caused this (a state entered by an event transition).
    pub event: Option<&'a AiEvent>,
    /// How the node ended (`exit`): "success", "failure" or "aborted".
    pub result: Option<&'a str>,
    pub state: Option<&'a str>,
    pub state_time: f64,
}

/// What brains need from the world.
pub trait AiHost {
    fn position(&self, entity: Entity) -> Option<Vec3>;
    /// Facing about +Y (0 faces +Z).
    fn yaw(&self, entity: Entity) -> Option<f32>;
    fn exists(&self, entity: Entity) -> bool;

    fn nav_info(&self, entity: Entity) -> Option<NavInfo>;
    fn nav_move(&mut self, entity: Entity, target: Vec3) -> bool;
    fn nav_stop(&mut self, entity: Entity);
    /// Turn to face `yaw` (standing agents turn; anything else snaps).
    fn face(&mut self, entity: Entity, yaw: f32);
    fn nav_random_point(&self, around: Vec3, radius: f32) -> Option<Vec3>;
    fn nav_closest(&self, point: Vec3, radius: f32) -> Option<Vec3>;
    /// Whether `from` sees `to` (both feet positions; `eye` is the eye height over them).
    fn line_of_sight(
        &self,
        from: Vec3,
        to: Vec3,
        eye: f32,
        mode: LineOfSight,
        ignore: &[Entity],
    ) -> bool;
    /// The agent's top speed, and setting it (`None` restores the component's own).
    fn agent_speed(&self, entity: Entity) -> Option<f32>;
    fn set_agent_speed(&mut self, entity: Entity, speed: f32);

    fn play(&mut self, entity: Entity, request: &PlayRequest) -> bool;
    fn stop_animation(&mut self, entity: Entity, layer: Option<usize>, fade: f32);
    /// Whether `clip` is no longer playing on `layer` (a one-shot that ended).
    fn animation_done(&self, entity: Entity, layer: usize, clip: &str) -> bool;
    fn set_anim_param(&mut self, entity: Entity, name: &str, value: &Value);
    fn anim_trigger(&mut self, entity: Entity, name: &str);

    /// Call a Lua function; its first result, converted.
    fn call(&mut self, function: &Callable, call: &LuaCall) -> Result<Value, String>;
    /// A field of a value only the host can look into (a Lua table).
    fn field(&mut self, value: &Value, name: &str) -> Value {
        value.json_field(name)
    }
    /// The positions a list value holds (`{{x, y, z}, ...}` or entities).
    fn positions(&mut self, value: &Value) -> Vec<Vec3>;
    /// The length of a list or text value.
    fn len(&mut self, value: &Value) -> Option<usize> {
        match value {
            Value::Str(s) => Some(s.chars().count()),
            Value::Json(json) => json.as_array().map(Vec::len),
            _ => None,
        }
    }
    /// A fresh Lua table (a `lua` leaf's memory), if Lua is there.
    fn new_memory(&mut self) -> Value;
    /// Emit a script event (`subscribe_event` handlers get it).
    fn emit(&mut self, event: &str, data: &Value);
    fn log(&mut self, agent: Entity, message: &str);
    /// Report a problem once per distinct message.
    fn warn(&mut self, agent: Entity, message: &str);
}

/// Something a brain sends this tick, delivered after it (for the next tick).
#[derive(Debug, Clone)]
pub enum Outgoing {
    Event {
        to: Recipients,
        event: AiEvent,
    },
    Noise {
        tag: Arc<str>,
        position: Vec3,
        radius: f32,
        data: Value,
    },
}

#[derive(Debug, Clone)]
pub enum Recipients {
    One(Entity),
    Group,
    Near {
        center: Vec3,
        radius: f32,
        tag: Option<String>,
    },
    All,
}

/// The agent's blackboards.
#[derive(Clone)]
pub struct Boards {
    pub own: BlackboardRef,
    pub group: BlackboardRef,
    pub global: BlackboardRef,
}

impl Boards {
    pub fn board(&self, scope: BoardScope) -> &BlackboardRef {
        match scope {
            BoardScope::Agent => &self.own,
            BoardScope::Group => &self.group,
            BoardScope::Global => &self.global,
        }
    }

    pub fn get(&self, key: &str) -> Value {
        let (scope, name) = split_key(key);
        self.board(scope).get(name)
    }

    pub fn set(&self, key: &str, value: Value) {
        let (scope, name) = split_key(key);
        self.board(scope).set(name, value);
    }
}

/// One agent's tick.
pub struct Cx<'a> {
    pub host: &'a mut dyn AiHost,
    /// Named brains (borrowed only briefly: Lua called meanwhile may define more).
    pub library: &'a std::cell::RefCell<Library>,
    pub agent: Entity,
    pub boards: &'a Boards,
    /// The AI clock (seconds since play started).
    pub time: f64,
    /// Seconds since this agent last ticked.
    pub dt: f32,
    pub frame: u64,
    pub events: &'a [AiEvent],
    pub senses: &'a Senses,
    pub rng: &'a mut SmallRng,
    pub outbox: &'a mut Vec<Outgoing>,
    /// The active states, outermost first.
    pub states: Vec<Arc<str>>,
    /// When the state whose rules or tree run now was entered.
    pub state_entered: f64,
    /// The event that caused the current transition (for `enter`).
    pub event: Option<AiEvent>,
    /// Running speed for `run = true` (the agent's setting).
    pub run_speed: f32,
    /// Candidate a `find` filter is checking (`it` in its expression).
    pub it: Option<Entity>,
}

impl Cx<'_> {
    pub fn tree_named(&self, name: &str) -> Option<Arc<super::def::TreeDef>> {
        self.library.borrow().tree(name)
    }

    pub fn machine_named(&self, name: &str) -> Option<Arc<super::def::MachineDef>> {
        self.library.borrow().machine(name)
    }

    pub fn position(&self) -> Vec3 {
        self.host.position(self.agent).unwrap_or(Vec3::ZERO)
    }

    pub fn state_name(&self) -> Option<&str> {
        self.states.last().map(|s| &**s)
    }

    pub fn state_time(&self) -> f64 {
        self.time - self.state_entered
    }

    /// Call Lua with this tick's context.
    pub fn call(
        &mut self,
        function: &Callable,
        node: &str,
        phase: CallPhase,
        memory: Option<&Value>,
        result: Option<&str>,
    ) -> Result<Value, String> {
        let state = self.states.last().cloned();
        let call = LuaCall {
            agent: self.agent,
            board: &self.boards.own,
            group: &self.boards.group,
            global: &self.boards.global,
            dt: self.dt,
            time: self.time,
            node,
            phase,
            memory,
            event: self.event.as_ref(),
            result,
            state: state.as_deref(),
            state_time: self.state_time(),
        };
        self.host.call(function, &call)
    }

    /// Call Lua; failures are warned about and give nil.
    pub fn call_or_warn(
        &mut self,
        function: &Callable,
        node: &str,
        phase: CallPhase,
        memory: Option<&Value>,
        result: Option<&str>,
    ) -> Option<Value> {
        match self.call(function, node, phase, memory, result) {
            Ok(value) => Some(value),
            Err(err) => {
                let agent = self.agent;
                self.host.warn(agent, &format!("{node}: {err}"));
                None
            }
        }
    }

    pub fn eval_num(&mut self, num: &Num, node: &str) -> Option<f64> {
        match num {
            Num::Const(v) => Some(*v),
            Num::Range(a, b) => Some(if b > a {
                self.rng.gen_range(*a..*b)
            } else {
                *a
            }),
            Num::Expr(expr) => expr.eval(self).as_f64(),
            Num::Call(function) => self
                .call_or_warn(function, node, CallPhase::Value, None, None)?
                .as_f64(),
        }
    }

    pub fn eval_cond(&mut self, cond: &Cond, node: &str) -> bool {
        match cond {
            Cond::Expr(expr) => expr.eval(self).truthy(),
            Cond::Call(function) => self
                .call_or_warn(function, node, CallPhase::Condition, None, None)
                .is_some_and(|v| v.truthy()),
        }
    }

    pub fn eval_operand(&mut self, operand: &Operand, node: &str) -> Value {
        match operand {
            Operand::Const(value) => value.clone(),
            Operand::Expr(expr) => expr.eval(self),
            Operand::Call(function) => self
                .call_or_warn(function, node, CallPhase::Value, None, None)
                .unwrap_or_default(),
        }
    }

    /// Where a value points: a position, an entity's position, or a table with
    /// `position`, `entity` or `x, y, z`.
    pub fn resolve_position(&mut self, value: &Value) -> Option<Vec3> {
        self.resolve_target(value).map(|(p, _)| p)
    }

    /// A value's position and, when it names an entity, the entity.
    pub fn resolve_target(&mut self, value: &Value) -> Option<(Vec3, Option<Entity>)> {
        if let Some(v) = value.as_vec3() {
            return Some((v, None));
        }
        if let Some(entity) = value.as_entity() {
            return self.host.position(entity).map(|p| (p, Some(entity)));
        }
        match value {
            Value::Json(_) | Value::Lua(_) => {
                for key in ["entity", "position", "pos"] {
                    let inner = self.host.field(value, key);
                    if !inner.is_nil() {
                        if let Some(found) = self.resolve_target(&inner) {
                            return Some(found);
                        }
                    }
                }
                let x = self.host.field(value, "x").as_f64()?;
                let y = self.host.field(value, "y").as_f64()?;
                let z = self.host.field(value, "z").as_f64()?;
                Some((Vec3::new(x as f32, y as f32, z as f32), None))
            }
            _ => None,
        }
    }
}

impl ExprEnv for Cx<'_> {
    fn lookup(&mut self, key: &str) -> Value {
        match key {
            "self" => Value::entity(self.agent),
            "it" => self.it.map(Value::entity).unwrap_or_default(),
            _ => self.boards.get(key),
        }
    }

    fn field(&mut self, value: &Value, name: &str) -> Value {
        self.host.field(value, name)
    }

    fn call(&mut self, function: Function, args: &[Value]) -> Value {
        let tag = |i: usize| args.get(i).and_then(Value::as_str);
        match function {
            Function::Dist => {
                let (a, b) = match args {
                    [a] => (Some(self.position()), self.resolve_position(a)),
                    [a, b] => (self.resolve_position(a), self.resolve_position(b)),
                    _ => (None, None),
                };
                match (a, b) {
                    (Some(a), Some(b)) => Value::Num(a.distance(b) as f64),
                    _ => Value::Nil,
                }
            }
            Function::Sees => Value::Bool(self.senses.sees(tag(0))),
            Function::Heard => Value::Bool(self.senses.heard(tag(0), self.time)),
            Function::Count => {
                let radius = args.get(1).and_then(Value::as_f64).map(|r| r as f32);
                let here = self.position();
                Value::Int(self.senses.count(tag(0), radius, here) as i64)
            }
            Function::Nearest => {
                let here = self.position();
                self.senses
                    .nearest(tag(0), here, false)
                    .map(Value::entity)
                    .unwrap_or_default()
            }
            Function::InState => {
                let name = tag(0).unwrap_or_default();
                Value::Bool(self.states.iter().any(|s| &**s == name))
            }
            Function::Time => Value::Num(self.time),
            Function::StateTime => Value::Num(self.state_time()),
            Function::Random => match (
                args.first().and_then(Value::as_f64),
                args.get(1).and_then(Value::as_f64),
            ) {
                (Some(a), Some(b)) if b > a => Value::Num(self.rng.gen_range(a..b)),
                (Some(a), Some(_)) => Value::Num(a),
                _ => Value::Num(self.rng.gen::<f64>()),
            },
            Function::Chance => {
                let p = args.first().and_then(Value::as_f64).unwrap_or(0.0);
                Value::Bool(self.rng.gen::<f64>() < p)
            }
            Function::Len => match args.first() {
                Some(value) => self
                    .host
                    .len(value)
                    .map(|n| Value::Int(n as i64))
                    .unwrap_or_default(),
                None => Value::Nil,
            },
            _ => Value::Nil,
        }
    }
}
