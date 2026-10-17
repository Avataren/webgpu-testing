//! Brain definitions: behaviour trees and state machines as read from Lua tables or
//! JSON, checked and flattened into immutable, shared definitions. Every agent running
//! one keeps its own runtime state ([`super::tree`], [`super::machine`]).
//!
//! The written form (docs/ai.md has the full reference): a node is a table whose first
//! item is its type, then its children (composites and decorators) or its main
//! arguments (leaves), and named parameters:
//!
//! ```lua
//! { "selector",
//!     { "sequence", when = "group.alarm", abort = "lower",
//!         { "move_to", "group.alarm_at", run = true },
//!         { "play", "look-around", time = 3 } },
//!     { "wander", 8 } }
//! ```
//!
//! JSON uses objects: `{"type": "sequence", "children": [...], "when": "..."}`.

use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use hecs::Entity;

use super::expr::Expr;
use super::value::Value;

/// A Lua function a brain calls, and the script that defined it (whose context the
/// call runs in: its entity handles, state and `get_self()`).
pub struct LuaFn {
    pub key: mlua::RegistryKey,
    pub owner: Option<Entity>,
    /// Where it was defined (`file:line`), for messages.
    pub label: String,
}

impl fmt::Debug for LuaFn {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LuaFn({})", self.label)
    }
}

/// Something a brain calls: a Lua function, or one named (looked up in the agent's
/// script, the brain's script, then functions registered with `ai_register`).
#[derive(Debug, Clone)]
pub enum Callable {
    Lua(Arc<LuaFn>),
    Named(Arc<str>),
}

impl Callable {
    pub fn label(&self) -> String {
        match self {
            Callable::Lua(f) => f.label.clone(),
            Callable::Named(name) => name.to_string(),
        }
    }
}

/// A definition as written, before it is checked: from a Lua table or JSON.
#[derive(Debug, Clone)]
pub enum Def {
    Nil,
    Bool(bool),
    Int(i64),
    Num(f64),
    Str(String),
    /// A table (Lua) or object/array (JSON): named fields and positional items.
    Table {
        named: Vec<(String, Def)>,
        items: Vec<Def>,
    },
    Func(Arc<LuaFn>),
}

impl Def {
    pub fn from_json(json: &serde_json::Value) -> Self {
        match json {
            serde_json::Value::Null => Def::Nil,
            serde_json::Value::Bool(b) => Def::Bool(*b),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) => Def::Int(i),
                None => Def::Num(n.as_f64().unwrap_or(0.0)),
            },
            serde_json::Value::String(s) => Def::Str(s.clone()),
            serde_json::Value::Array(items) => Def::Table {
                named: Vec::new(),
                items: items.iter().map(Def::from_json).collect(),
            },
            serde_json::Value::Object(map) => Def::Table {
                named: map
                    .iter()
                    .map(|(k, v)| (k.clone(), Def::from_json(v)))
                    .collect(),
                items: Vec::new(),
            },
        }
    }

    fn kind(&self) -> &'static str {
        match self {
            Def::Nil => "nil",
            Def::Bool(_) => "a boolean",
            Def::Int(_) | Def::Num(_) => "a number",
            Def::Str(_) => "text",
            Def::Table { .. } => "a table",
            Def::Func(_) => "a function",
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            Def::Int(i) => Some(*i as f64),
            Def::Num(n) => Some(*n),
            _ => None,
        }
    }

    fn field(&self, name: &str) -> Option<&Def> {
        match self {
            Def::Table { named, .. } => named.iter().find(|(k, _)| k == name).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The items of a list (a table's positional items, or a JSON array).
    fn list(&self) -> Option<&[Def]> {
        match self {
            Def::Table { named, items } if named.is_empty() || !items.is_empty() => Some(items),
            _ => None,
        }
    }

    /// As a literal blackboard value (tables as data).
    pub fn to_value(&self) -> Value {
        match self {
            Def::Nil => Value::Nil,
            Def::Bool(b) => Value::Bool(*b),
            Def::Int(i) => Value::Int(*i),
            Def::Num(n) => Value::Num(*n),
            Def::Str(s) => Value::str(s),
            Def::Func(_) => Value::Nil,
            Def::Table { .. } => Value::from_json(&self.to_json()),
        }
    }

    fn to_json(&self) -> serde_json::Value {
        match self {
            Def::Nil | Def::Func(_) => serde_json::Value::Null,
            Def::Bool(b) => serde_json::Value::Bool(*b),
            Def::Int(i) => serde_json::json!(i),
            Def::Num(n) => serde_json::json!(n),
            Def::Str(s) => serde_json::Value::String(s.clone()),
            Def::Table { named, items } => {
                if named.is_empty() {
                    serde_json::Value::Array(items.iter().map(Def::to_json).collect())
                } else {
                    let mut map = serde_json::Map::new();
                    for (k, v) in named {
                        map.insert(k.clone(), v.to_json());
                    }
                    for (i, v) in items.iter().enumerate() {
                        map.insert((i + 1).to_string(), v.to_json());
                    }
                    serde_json::Value::Object(map)
                }
            }
        }
    }
}

/// A number parameter: a constant, a random range `{min, max}`, an expression or a
/// Lua function returning one.
#[derive(Debug, Clone)]
pub enum Num {
    Const(f64),
    Range(f64, f64),
    Expr(Expr),
    Call(Callable),
}

/// A yes/no parameter: an expression or a Lua function (its result's truth).
#[derive(Debug, Clone)]
pub enum Cond {
    Expr(Expr),
    Call(Callable),
}

impl Cond {
    pub fn label(&self) -> String {
        match self {
            Cond::Expr(e) => e.source().to_owned(),
            Cond::Call(c) => format!("{}()", c.label()),
        }
    }
}

/// A value parameter: a constant, an expression or a Lua function.
#[derive(Debug, Clone)]
pub enum Operand {
    Const(Value),
    Expr(Expr),
    Call(Callable),
}

/// When a guard re-checks its condition while the tree runs (like Unreal's observer
/// aborts): `self` stops its own branch once the condition fails, `lower` interrupts a
/// later sibling (of the selector it is in) once the condition holds, `both` does both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Abort {
    #[default]
    None,
    SelfOnly,
    Lower,
    Both,
}

impl Abort {
    pub fn aborts_self(self) -> bool {
        matches!(self, Abort::SelfOnly | Abort::Both)
    }

    pub fn aborts_lower(self) -> bool {
        matches!(self, Abort::Lower | Abort::Both)
    }
}

/// How many children of a parallel must succeed (or fail) for it to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Threshold {
    All,
    Count(usize),
}

/// A node's result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Success,
    Failure,
    Running,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Success => "success",
            Status::Failure => "failure",
            Status::Running => "running",
        }
    }
}

/// Who an event goes to.
#[derive(Debug, Clone)]
pub enum Recipient {
    /// The agent itself.
    Me,
    /// The entity a value names.
    To(Operand),
    /// Every agent of the sender's group.
    Group,
    /// Agents within `radius` (all, with a tag).
    Near { radius: Num, tag: Option<String> },
    /// Every agent.
    All,
}

/// Built-in leaves.
#[derive(Debug, Clone)]
pub enum Action {
    Succeed,
    Fail,
    Running,
    Wait {
        time: Num,
    },
    Log {
        message: String,
    },
    Set {
        key: String,
        value: Operand,
    },
    Clear {
        key: String,
    },
    Add {
        key: String,
        amount: Num,
        min: Option<f64>,
        max: Option<f64>,
    },
    Check {
        cond: Cond,
    },
    WaitUntil {
        cond: Cond,
    },
    Lua {
        run: Option<Callable>,
        enter: Option<Callable>,
        exit: Option<Callable>,
    },
    Send {
        event: String,
        to: Recipient,
        data: Option<Operand>,
    },
    Noise {
        tag: String,
        radius: Num,
        data: Option<Operand>,
        at: Option<Operand>,
    },
    Emit {
        event: String,
        data: Option<Operand>,
    },
    WaitEvent {
        event: String,
        store: Option<String>,
        store_from: Option<String>,
        store_position: Option<String>,
    },
    MoveTo {
        target: Operand,
        radius: f32,
        speed: Option<Num>,
        run: bool,
        /// Re-plan this often while the target (an entity) moves (seconds).
        repath: f32,
        /// Give up (fail) after this long without arriving.
        give_up: Option<Num>,
        /// Succeed when the way is blocked (a partial path) once this close.
        accept_partial: Option<f32>,
    },
    Wander {
        center: Option<Operand>,
        radius: Num,
        min_distance: f32,
        speed: Option<Num>,
        store: Option<String>,
    },
    Flee {
        from: Operand,
        distance: Num,
        speed: Option<Num>,
        run: bool,
    },
    Follow {
        target: Operand,
        distance: f32,
        speed: Option<Num>,
        run: bool,
    },
    Face {
        target: Option<Operand>,
        yaw: Option<Num>,
        wait: f32,
    },
    Stop,
    Patrol {
        points: Operand,
        index_key: String,
        radius: f32,
        speed: Option<Num>,
    },
    PickPoint {
        store: String,
        around: Option<Operand>,
        radius: Num,
        min_distance: f32,
    },
    Play {
        clip: Operand,
        layer: usize,
        looping: Option<bool>,
        fade: f32,
        speed: f32,
        time: Option<Num>,
        stop_on_exit: bool,
    },
    StopAnimation {
        layer: Option<usize>,
        fade: f32,
    },
    AnimParam {
        name: String,
        value: Operand,
    },
    AnimTrigger {
        name: String,
    },
    Find {
        tag: Option<String>,
        store: String,
        range: Option<f32>,
        /// Only what it sees now (not what it remembers).
        visible: bool,
        filter: Option<Cond>,
    },
}

impl Action {
    pub fn type_name(&self) -> &'static str {
        match self {
            Action::Succeed => "succeed",
            Action::Fail => "fail",
            Action::Running => "running",
            Action::Wait { .. } => "wait",
            Action::Log { .. } => "log",
            Action::Set { .. } => "set",
            Action::Clear { .. } => "clear",
            Action::Add { .. } => "add",
            Action::Check { .. } => "check",
            Action::WaitUntil { .. } => "wait_until",
            Action::Lua { .. } => "lua",
            Action::Send { .. } => "send",
            Action::Noise { .. } => "noise",
            Action::Emit { .. } => "emit",
            Action::WaitEvent { .. } => "wait_event",
            Action::MoveTo { .. } => "move_to",
            Action::Wander { .. } => "wander",
            Action::Flee { .. } => "flee",
            Action::Follow { .. } => "follow",
            Action::Face { .. } => "face",
            Action::Stop => "stop",
            Action::Patrol { .. } => "patrol",
            Action::PickPoint { .. } => "pick_point",
            Action::Play { .. } => "play",
            Action::StopAnimation { .. } => "stop_animation",
            Action::AnimParam { .. } => "anim_param",
            Action::AnimTrigger { .. } => "anim_trigger",
            Action::Find { .. } => "find",
        }
    }
}

/// What a tree node is.
#[derive(Debug, Clone)]
pub enum NodeKind {
    /// Children in order until one fails.
    Sequence,
    /// Children in order until one succeeds.
    Selector,
    /// A sequence that starts from its first child every tick (re-checking conditions).
    ReactiveSequence,
    /// A selector that starts from its first child every tick (higher priorities win).
    ReactiveSelector,
    /// Every child each tick, until enough succeed or fail.
    Parallel {
        success: Threshold,
        failure: Threshold,
    },
    /// A sequence in a random order (shuffled on entry).
    ShuffleSequence,
    /// A selector in a random order (shuffled on entry, children's `weight` biasing it).
    ShuffleSelector,
    /// One child picked at random by `weight`.
    Random,
    /// The child with the best `score` first, then the next best (a selector by score).
    /// With `interval`, scores again while running and switches when another child
    /// beats the running one by `hysteresis`.
    Utility {
        interval: Option<f32>,
        hysteresis: f64,
    },
    Invert,
    ForceSuccess,
    ForceFailure,
    /// Run the child again (`count` times, or for ever), stopping early when it ends
    /// with `until`.
    Repeat {
        count: Option<u32>,
        until: Option<Status>,
    },
    /// Run the child again after it fails, up to `count` more times.
    Retry {
        count: u32,
    },
    /// Fail when the child runs longer than `time`.
    Timeout {
        time: Num,
    },
    /// Wait `time`, then run the child.
    Delay {
        time: Num,
    },
    /// Fail without running the child for `time` after it last ended.
    Cooldown {
        time: Num,
    },
    /// The child runs once; afterwards the node repeats that result.
    Once,
    /// Run the child with probability `p` (rolled on entry), else fail.
    Chance {
        p: Num,
    },
    /// Run the child while `cond` holds (see [`Abort`]).
    Guard {
        cond: Cond,
        abort: Abort,
    },
    /// Another tree by name, run as part of this one.
    Subtree {
        name: String,
    },
    Leaf(Action),
}

impl NodeKind {
    pub fn type_name(&self) -> &'static str {
        match self {
            NodeKind::Sequence => "sequence",
            NodeKind::Selector => "selector",
            NodeKind::ReactiveSequence => "reactive_sequence",
            NodeKind::ReactiveSelector => "reactive_selector",
            NodeKind::Parallel { .. } => "parallel",
            NodeKind::ShuffleSequence => "shuffle_sequence",
            NodeKind::ShuffleSelector => "shuffle_selector",
            NodeKind::Random => "random",
            NodeKind::Utility { .. } => "utility",
            NodeKind::Invert => "invert",
            NodeKind::ForceSuccess => "force_success",
            NodeKind::ForceFailure => "force_failure",
            NodeKind::Repeat { .. } => "repeat",
            NodeKind::Retry { .. } => "retry",
            NodeKind::Timeout { .. } => "timeout",
            NodeKind::Delay { .. } => "delay",
            NodeKind::Cooldown { .. } => "cooldown",
            NodeKind::Once => "once",
            NodeKind::Chance { .. } => "chance",
            NodeKind::Guard { .. } => "guard",
            NodeKind::Subtree { .. } => "subtree",
            NodeKind::Leaf(action) => action.type_name(),
        }
    }

    fn is_composite(&self) -> bool {
        matches!(
            self,
            NodeKind::Sequence
                | NodeKind::Selector
                | NodeKind::ReactiveSequence
                | NodeKind::ReactiveSelector
                | NodeKind::Parallel { .. }
                | NodeKind::ShuffleSequence
                | NodeKind::ShuffleSelector
                | NodeKind::Random
                | NodeKind::Utility { .. }
        )
    }

    fn is_decorator(&self) -> bool {
        matches!(
            self,
            NodeKind::Invert
                | NodeKind::ForceSuccess
                | NodeKind::ForceFailure
                | NodeKind::Repeat { .. }
                | NodeKind::Retry { .. }
                | NodeKind::Timeout { .. }
                | NodeKind::Delay { .. }
                | NodeKind::Cooldown { .. }
                | NodeKind::Once
                | NodeKind::Chance { .. }
                | NodeKind::Guard { .. }
        )
    }
}

/// A node of a [`TreeDef`].
#[derive(Debug, Clone)]
pub struct NodeDef {
    pub kind: NodeKind,
    pub children: Vec<usize>,
    pub parent: Option<usize>,
    /// Its `name`, or a description made from its type and main argument.
    pub label: String,
    /// How a `utility` parent rates it.
    pub score: Option<Num>,
    /// How a `random` or `shuffle_selector` parent favours it.
    pub weight: f64,
    pub depth: usize,
}

/// A behaviour tree: nodes in depth-first order (the root first).
#[derive(Debug)]
pub struct TreeDef {
    pub name: String,
    pub nodes: Vec<NodeDef>,
}

impl TreeDef {
    pub const ROOT: usize = 0;
}

/// A state machine, or a state's nested machine.
#[derive(Debug)]
pub struct MachineDef {
    pub name: String,
    pub states: Vec<StateDef>,
    pub initial: usize,
    /// Transitions checked in every state (before the state's own).
    pub transitions: Vec<TransitionDef>,
}

impl MachineDef {
    pub fn state_index(&self, name: &str) -> Option<usize> {
        self.states.iter().position(|s| s.name == name)
    }
}

/// A tree or machine given inline, or by the name it was defined under.
#[derive(Debug, Clone)]
pub enum BrainRef<T> {
    Inline(Arc<T>),
    Named(String),
}

#[derive(Debug)]
pub struct StateDef {
    pub name: String,
    pub enter: Option<Callable>,
    pub update: Option<Callable>,
    pub exit: Option<Callable>,
    /// Run while in the state; restarted when it ends (unless a transition leaves).
    pub tree: Option<BrainRef<TreeDef>>,
    pub machine: Option<BrainRef<MachineDef>>,
    /// Entering it ends the machine with this result (a parent state's `on` sees it).
    pub finish: Option<Status>,
    /// Blackboard values set on entry.
    pub set: Vec<(String, Operand)>,
    pub transitions: Vec<TransitionDef>,
}

/// Where a transition goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionTarget {
    State(usize),
    /// The state before this one.
    Previous,
    /// This state again, from the start.
    Restart,
}

/// What finished, for a transition's `on`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoneOn {
    Success,
    Failure,
    Done,
}

#[derive(Debug, Clone)]
pub struct TransitionDef {
    pub to: TransitionTarget,
    /// Taken when this event arrives.
    pub event: Option<String>,
    /// Taken when this holds (with `event`: when both do).
    pub when: Option<Cond>,
    /// Taken after this long in the state (rolled on entry when a range).
    pub after: Option<Num>,
    /// Taken when the state's tree or machine finishes so.
    pub on: Option<DoneOn>,
    /// Taken only this often when the rest says yes (rolled each time it would be).
    pub chance: Option<f64>,
    /// Put the event's data, sender or position on the blackboard.
    pub store: Option<String>,
    pub store_from: Option<String>,
    pub store_position: Option<String>,
    /// For machine-wide transitions: states it is not taken from.
    pub except: Vec<usize>,
}

impl TransitionDef {
    pub fn describe(&self, machine: &MachineDef) -> String {
        let to = match self.to {
            TransitionTarget::State(i) => machine.states[i].name.clone(),
            TransitionTarget::Previous => "@previous".into(),
            TransitionTarget::Restart => "@restart".into(),
        };
        let mut why = Vec::new();
        if let Some(event) = &self.event {
            why.push(format!("event {event}"));
        }
        if let Some(when) = &self.when {
            why.push(format!("when {}", when.label()));
        }
        if let Some(after) = &self.after {
            why.push(format!("after {}", describe_num(after)));
        }
        if let Some(on) = self.on {
            why.push(format!("on {on:?}").to_lowercase());
        }
        format!("-> {to} ({})", why.join(", "))
    }
}

fn describe_num(num: &Num) -> String {
    match num {
        Num::Const(v) => format!("{v}"),
        Num::Range(a, b) => format!("{a}..{b}"),
        Num::Expr(e) => e.source().to_owned(),
        Num::Call(c) => format!("{}()", c.label()),
    }
}

/// A brain: what an agent runs.
#[derive(Debug, Clone)]
pub enum Brain {
    Tree(Arc<TreeDef>),
    Machine(Arc<MachineDef>),
}

impl Brain {
    pub fn kind(&self) -> &'static str {
        match self {
            Brain::Tree(_) => "tree",
            Brain::Machine(_) => "machine",
        }
    }
}

/// Named brains and functions.
#[derive(Debug, Default)]
pub struct Library {
    brains: HashMap<String, Brain>,
    /// When each name was last (re)defined, so agents running it restart.
    revisions: HashMap<String, u64>,
    revision: u64,
    functions: HashMap<String, Arc<LuaFn>>,
    /// Brains loaded from files: path → file stamp.
    files: HashMap<String, Option<std::time::SystemTime>>,
}

impl Library {
    pub fn define(&mut self, name: impl Into<String>, brain: Brain) {
        let name = name.into();
        self.revision += 1;
        self.revisions.insert(name.clone(), self.revision);
        self.brains.insert(name, brain);
    }

    pub fn get(&self, name: &str) -> Option<&Brain> {
        self.brains.get(name)
    }

    pub fn tree(&self, name: &str) -> Option<Arc<TreeDef>> {
        match self.brains.get(name)? {
            Brain::Tree(tree) => Some(tree.clone()),
            Brain::Machine(_) => None,
        }
    }

    pub fn machine(&self, name: &str) -> Option<Arc<MachineDef>> {
        match self.brains.get(name)? {
            Brain::Machine(machine) => Some(machine.clone()),
            Brain::Tree(_) => None,
        }
    }

    /// When `name` was last defined (0: never).
    pub fn revision_of(&self, name: &str) -> u64 {
        self.revisions.get(name).copied().unwrap_or(0)
    }

    pub fn names(&self) -> Vec<(String, &'static str)> {
        let mut names: Vec<_> = self
            .brains
            .iter()
            .map(|(name, brain)| (name.clone(), brain.kind()))
            .collect();
        names.sort();
        names
    }

    pub fn register_function(&mut self, name: impl Into<String>, function: Arc<LuaFn>) {
        self.functions.insert(name.into(), function);
    }

    pub fn function(&self, name: &str) -> Option<Arc<LuaFn>> {
        self.functions.get(name).cloned()
    }

    pub fn file_stamp(&self, path: &str) -> Option<Option<std::time::SystemTime>> {
        self.files.get(path).copied()
    }

    pub fn set_file_stamp(&mut self, path: &str, stamp: Option<std::time::SystemTime>) {
        self.files.insert(path.to_owned(), stamp);
    }
}

// ---------------------------------------------------------------------------------
// Parsing

/// Read a brain: a machine when it has `states`, else a tree.
pub fn parse_brain(name: &str, def: &Def) -> Result<Brain, String> {
    if def.field("states").is_some() {
        Ok(Brain::Machine(Arc::new(parse_machine(name, def)?)))
    } else {
        Ok(Brain::Tree(Arc::new(parse_tree(name, def)?)))
    }
}

/// Read a behaviour tree from its root node.
///
/// ```
/// use wgpu_cube::ai::{parse_tree, Def};
///
/// let json = serde_json::json!({
///     "type": "sequence",
///     "children": [
///         { "type": "check", "expr": "hunger > 0.5" },
///         { "type": "wait", "time": [1, 2] }
///     ]
/// });
/// let tree = parse_tree("eat", &Def::from_json(&json)).unwrap();
/// assert_eq!(tree.nodes.len(), 3);
/// assert!(parse_tree("bad", &Def::from_json(&serde_json::json!({"type": "wiat"}))).is_err());
/// ```
pub fn parse_tree(name: &str, def: &Def) -> Result<TreeDef, String> {
    let mut nodes = Vec::new();
    parse_node(def, &mut nodes, None, 0, name)?;
    Ok(TreeDef {
        name: name.to_owned(),
        nodes,
    })
}

/// The table form of a node, whichever way it was written.
struct NodeForm<'a> {
    kind: String,
    named: Vec<(&'a str, &'a Def)>,
    /// Positional items after the type.
    items: Vec<&'a Def>,
}

fn node_form<'a>(def: &'a Def, path: &str) -> Result<NodeForm<'a>, String> {
    match def {
        Def::Func(_) => Ok(NodeForm {
            kind: "lua".into(),
            named: Vec::new(),
            items: vec![def],
        }),
        Def::Table { named, items } => {
            let mut form = NodeForm {
                kind: String::new(),
                named: Vec::new(),
                items: Vec::new(),
            };
            let mut rest = items.iter();
            if let Some(Def::Str(kind)) = items.first() {
                form.kind = kind.clone();
                rest.next();
            }
            form.items.extend(rest);
            for (key, value) in named {
                match key.as_str() {
                    "type" => match value {
                        Def::Str(kind) if form.kind.is_empty() => form.kind = kind.clone(),
                        Def::Str(_) => {
                            return Err(format!("{path}: the type is given twice"));
                        }
                        other => {
                            return Err(format!("{path}: type must be text, not {}", other.kind()))
                        }
                    },
                    "children" => match value.list() {
                        Some(children) => form.items.extend(children.iter()),
                        None => return Err(format!("{path}: children must be a list")),
                    },
                    "child" => form.items.push(value),
                    _ => form.named.push((key.as_str(), value)),
                }
            }
            if form.kind.is_empty() {
                return Err(format!(
                    "{path}: a node needs a type (its first item, or `type`)"
                ));
            }
            Ok(form)
        }
        other => Err(format!(
            "{path}: a node is a table or a function, not {}",
            other.kind()
        )),
    }
}

/// Named parameters of one node, checked off as they are read.
struct Params<'a> {
    path: String,
    named: Vec<(&'a str, &'a Def)>,
    used: Vec<bool>,
}

impl<'a> Params<'a> {
    fn new(path: String, named: Vec<(&'a str, &'a Def)>) -> Self {
        let used = vec![false; named.len()];
        Self { path, named, used }
    }

    fn take(&mut self, name: &str) -> Option<&'a Def> {
        let index = self.named.iter().position(|(k, _)| *k == name)?;
        self.used[index] = true;
        match self.named[index].1 {
            Def::Nil => None,
            def => Some(def),
        }
    }

    fn err(&self, message: impl fmt::Display) -> String {
        format!("{}: {message}", self.path)
    }

    fn finish(self) -> Result<(), String> {
        let unknown: Vec<&str> = self
            .named
            .iter()
            .zip(&self.used)
            .filter(|(_, used)| !**used)
            .map(|((k, _), _)| *k)
            .collect();
        if unknown.is_empty() {
            Ok(())
        } else {
            Err(self.err(format!("unknown parameter(s) {}", unknown.join(", "))))
        }
    }

    fn text(&mut self, name: &str) -> Result<Option<String>, String> {
        match self.take(name) {
            None => Ok(None),
            Some(Def::Str(s)) => Ok(Some(s.clone())),
            Some(other) => Err(self.err(format!("{name} must be text, not {}", other.kind()))),
        }
    }

    fn required_text(&mut self, name: &str) -> Result<String, String> {
        self.text(name)?
            .ok_or_else(|| self.err(format!("needs {name}")))
    }

    fn number(&mut self, name: &str) -> Result<Option<f64>, String> {
        match self.take(name) {
            None => Ok(None),
            Some(def) => def
                .as_f64()
                .map(Some)
                .ok_or_else(|| self.err(format!("{name} must be a number, not {}", def.kind()))),
        }
    }

    fn flag(&mut self, name: &str) -> Result<Option<bool>, String> {
        match self.take(name) {
            None => Ok(None),
            Some(Def::Bool(b)) => Ok(Some(*b)),
            Some(other) => Err(self.err(format!(
                "{name} must be true or false, not {}",
                other.kind()
            ))),
        }
    }

    fn num(&mut self, name: &str) -> Result<Option<Num>, String> {
        match self.take(name) {
            None => Ok(None),
            Some(def) => num_from(def)
                .map(Some)
                .map_err(|e| self.err(format!("{name}: {e}"))),
        }
    }

    fn cond(&mut self, name: &str) -> Result<Option<Cond>, String> {
        match self.take(name) {
            None => Ok(None),
            Some(def) => cond_from(def)
                .map(Some)
                .map_err(|e| self.err(format!("{name}: {e}"))),
        }
    }

    /// A value that names something: text is an expression (usually a key).
    fn operand(&mut self, name: &str) -> Result<Option<Operand>, String> {
        match self.take(name) {
            None => Ok(None),
            Some(def) => operand_from(def, true)
                .map(Some)
                .map_err(|e| self.err(format!("{name}: {e}"))),
        }
    }

    /// A literal value: text is text.
    fn literal(&mut self, name: &str) -> Result<Option<Operand>, String> {
        match self.take(name) {
            None => Ok(None),
            Some(def) => operand_from(def, false)
                .map(Some)
                .map_err(|e| self.err(format!("{name}: {e}"))),
        }
    }

    fn callable(&mut self, name: &str) -> Result<Option<Callable>, String> {
        match self.take(name) {
            None => Ok(None),
            Some(def) => callable_from(def)
                .map(Some)
                .map_err(|e| self.err(format!("{name}: {e}"))),
        }
    }
}

fn num_from(def: &Def) -> Result<Num, String> {
    match def {
        Def::Int(_) | Def::Num(_) => Ok(Num::Const(def.as_f64().unwrap_or(0.0))),
        Def::Str(s) => Expr::parse(s).map(Num::Expr),
        Def::Func(f) => Ok(Num::Call(Callable::Lua(f.clone()))),
        Def::Table { .. } => match def.list() {
            Some([a, b]) => match (a.as_f64(), b.as_f64()) {
                (Some(a), Some(b)) => Ok(Num::Range(a.min(b), a.max(b))),
                _ => Err("a range is two numbers, {min, max}".into()),
            },
            _ => Err("expected a number, a range {min, max}, an expression or a function".into()),
        },
        other => Err(format!("expected a number, not {}", other.kind())),
    }
}

fn cond_from(def: &Def) -> Result<Cond, String> {
    match def {
        Def::Str(s) => Expr::parse(s).map(Cond::Expr),
        Def::Bool(b) => Ok(Cond::Expr(Expr::constant(Value::Bool(*b)))),
        Def::Func(f) => Ok(Cond::Call(Callable::Lua(f.clone()))),
        other => Err(format!(
            "expected an expression or a function, not {}",
            other.kind()
        )),
    }
}

fn callable_from(def: &Def) -> Result<Callable, String> {
    match def {
        Def::Func(f) => Ok(Callable::Lua(f.clone())),
        Def::Str(name) if !name.is_empty() => Ok(Callable::Named(Arc::from(name.as_str()))),
        other => Err(format!(
            "expected a function or a function's name, not {}",
            other.kind()
        )),
    }
}

fn operand_from(def: &Def, text_is_expr: bool) -> Result<Operand, String> {
    match def {
        Def::Str(s) if text_is_expr => Expr::parse(s).map(Operand::Expr),
        Def::Func(f) => Ok(Operand::Call(Callable::Lua(f.clone()))),
        // `{expr = "..."}` is an expression even where text is literal.
        Def::Table { .. } if def.field("expr").is_some() => match def.field("expr") {
            Some(Def::Str(s)) => Expr::parse(s).map(Operand::Expr),
            _ => Err("expr must be text".into()),
        },
        other => Ok(Operand::Const(other.to_value())),
    }
}

fn abort_from(text: &str) -> Result<Abort, String> {
    Ok(match text {
        "none" => Abort::None,
        "self" => Abort::SelfOnly,
        "lower" | "lower_priority" => Abort::Lower,
        "both" => Abort::Both,
        other => return Err(format!("abort is none, self, lower or both, not {other:?}")),
    })
}

fn threshold_from(def: &Def) -> Result<Threshold, String> {
    match def {
        Def::Str(s) if s == "all" => Ok(Threshold::All),
        Def::Str(s) if s == "one" || s == "any" => Ok(Threshold::Count(1)),
        _ => match def.as_f64() {
            Some(n) if n >= 1.0 => Ok(Threshold::Count(n as usize)),
            _ => Err("expected \"all\", \"one\" or a count".into()),
        },
    }
}

fn status_from(text: &str) -> Result<Status, String> {
    match text {
        "success" => Ok(Status::Success),
        "failure" => Ok(Status::Failure),
        other => Err(format!(
            "expected \"success\" or \"failure\", not {other:?}"
        )),
    }
}

/// The positional parameters each leaf takes, in order.
fn leaf_positional(kind: &str) -> &'static [&'static str] {
    match kind {
        "wait" => &["time"],
        "log" => &["message"],
        "set" => &["key", "value"],
        "clear" => &["key"],
        "add" => &["key", "amount"],
        "check" | "condition" | "wait_until" => &["expr"],
        "lua" | "action" => &["run"],
        "send" => &["event", "to"],
        "broadcast" => &["event", "radius"],
        "noise" => &["tag", "radius"],
        "emit" => &["event"],
        "wait_event" => &["event"],
        "move_to" => &["target"],
        "wander" => &["radius"],
        "flee" => &["from", "distance"],
        "follow" => &["target", "distance"],
        "face" => &["target"],
        "patrol" => &["points"],
        "pick_point" => &["store", "radius"],
        "play" => &["clip"],
        "stop_animation" => &["layer"],
        "anim_param" => &["name", "value"],
        "anim_trigger" => &["name"],
        "find" => &["tag", "store"],
        "subtree" => &["tree"],
        _ => &[],
    }
}

/// Parse one node (and its children) into `nodes`; returns its index.
fn parse_node(
    def: &Def,
    nodes: &mut Vec<NodeDef>,
    parent: Option<usize>,
    depth: usize,
    path: &str,
) -> Result<usize, String> {
    if depth > 64 {
        return Err(format!("{path}: the tree is nested too deeply"));
    }
    let form = node_form(def, path)?;
    let path = format!("{path} > {}", form.kind);
    let kind_name = form.kind.as_str();

    // Leaves take their main arguments positionally.
    let mut named = form.named;
    let mut children: Vec<&Def> = Vec::new();
    let positional = leaf_positional(kind_name);
    if is_known_leaf(kind_name) {
        if form.items.len() > positional.len() {
            return Err(format!(
                "{path}: takes {} positional argument(s) ({}), got {}",
                positional.len(),
                positional.join(", "),
                form.items.len()
            ));
        }
        for (name, item) in positional.iter().zip(form.items) {
            if named.iter().any(|(k, _)| k == name) {
                return Err(format!("{path}: {name} is given twice"));
            }
            named.push((name, item));
        }
    } else {
        children = form.items;
    }

    let mut params = Params::new(path.clone(), named);
    // Shared by every node: wrappers and how parents rate it. (A `guard` node reads its
    // own `when` and `abort`.)
    let label = params.text("name")?;
    let _comment = params.take("comment");
    let (when, abort) = if kind_name == "guard" {
        (None, Abort::None)
    } else {
        let when = params.cond("when")?;
        let abort = match params.text("abort")? {
            Some(text) => abort_from(&text).map_err(|e| params.err(e))?,
            None => Abort::None,
        };
        (when, abort)
    };
    if abort != Abort::None && when.is_none() {
        return Err(params.err("abort needs a `when` condition"));
    }
    let cooldown = params.num("cooldown")?;
    let timeout = params.num("timeout")?;
    let chance = params.num("chance")?;
    let score = params.num("score")?;
    let weight = params.number("weight")?.unwrap_or(1.0).max(0.0);

    let kind = parse_kind(kind_name, &mut params)?;
    params.finish()?;

    let child_count = children.len();
    if kind.is_composite() && child_count == 0 {
        return Err(format!("{path}: has no children"));
    }
    if kind.is_decorator() && child_count != 1 {
        return Err(format!(
            "{path}: takes exactly one child, got {child_count}"
        ));
    }
    if !kind.is_composite() && !kind.is_decorator() && child_count > 0 {
        return Err(format!("{path}: is a leaf and takes no children"));
    }

    // Wrappers from the shared parameters, outermost first: when, chance, cooldown,
    // timeout. The outermost carries the score and weight its parent sees.
    let mut wrappers: Vec<(NodeKind, String)> = Vec::new();
    if let Some(cond) = when {
        let text = format!("when {}", cond.label());
        wrappers.push((NodeKind::Guard { cond, abort }, text));
    }
    if let Some(p) = chance {
        let text = format!("chance {}", describe_num(&p));
        wrappers.push((NodeKind::Chance { p }, text));
    }
    if let Some(time) = cooldown {
        let text = format!("cooldown {}", describe_num(&time));
        wrappers.push((NodeKind::Cooldown { time }, text));
    }
    if let Some(time) = timeout {
        let text = format!("timeout {}", describe_num(&time));
        wrappers.push((NodeKind::Timeout { time }, text));
    }

    let mut parent = parent;
    let mut first = None;
    let mut depth = depth;
    let wrapper_count = wrappers.len();
    for (i, (wrapper, text)) in wrappers.into_iter().enumerate() {
        let index = nodes.len();
        nodes.push(NodeDef {
            kind: wrapper,
            children: Vec::new(),
            parent,
            label: text,
            score: if i == 0 { score.clone() } else { None },
            weight: if i == 0 { weight } else { 1.0 },
            depth,
        });
        if let Some(p) = parent {
            nodes[p].children.push(index);
        }
        first.get_or_insert(index);
        parent = Some(index);
        depth += 1;
    }

    let index = nodes.len();
    let auto_label = describe_kind(&kind);
    nodes.push(NodeDef {
        kind,
        children: Vec::new(),
        parent,
        label: label.unwrap_or(auto_label),
        score: if wrapper_count == 0 { score } else { None },
        weight: if wrapper_count == 0 { weight } else { 1.0 },
        depth,
    });
    if let Some(p) = parent {
        nodes[p].children.push(index);
    }
    for (i, child) in children.iter().enumerate() {
        parse_node(
            child,
            nodes,
            Some(index),
            depth + 1,
            &format!("{path}[{}]", i + 1),
        )?;
    }
    Ok(first.unwrap_or(index))
}

fn is_known_leaf(kind: &str) -> bool {
    !leaf_positional(kind).is_empty()
        || matches!(
            kind,
            "succeed" | "success" | "fail" | "failure" | "running" | "stop"
        )
}

/// A short description for a node without a name: its type and main argument.
fn describe_kind(kind: &NodeKind) -> String {
    let operand = |op: &Operand| match op {
        Operand::Const(v) => v.to_string(),
        Operand::Expr(e) => e.source().to_owned(),
        Operand::Call(c) => format!("{}()", c.label()),
    };
    match kind {
        NodeKind::Leaf(action) => match action {
            Action::Wait { time } => format!("wait {}", describe_num(time)),
            Action::Log { message } => format!("log {message:?}"),
            Action::Set { key, value } => format!("set {key} = {}", operand(value)),
            Action::Clear { key } => format!("clear {key}"),
            Action::Add { key, amount, .. } => format!("add {key} += {}", describe_num(amount)),
            Action::Check { cond } => format!("check {}", cond.label()),
            Action::WaitUntil { cond } => format!("wait until {}", cond.label()),
            Action::Lua { run, enter, .. } => format!(
                "lua {}",
                run.as_ref()
                    .or(enter.as_ref())
                    .map(Callable::label)
                    .unwrap_or_default()
            ),
            Action::Send { event, .. } => format!("send {event}"),
            Action::Noise { tag, .. } => format!("noise {tag}"),
            Action::Emit { event, .. } => format!("emit {event}"),
            Action::WaitEvent { event, .. } => format!("wait for {event}"),
            Action::MoveTo { target, .. } => format!("move to {}", operand(target)),
            Action::Wander { radius, .. } => format!("wander {}", describe_num(radius)),
            Action::Flee { from, .. } => format!("flee {}", operand(from)),
            Action::Follow { target, .. } => format!("follow {}", operand(target)),
            Action::Face { target, yaw, .. } => match (target, yaw) {
                (Some(t), _) => format!("face {}", operand(t)),
                (None, Some(y)) => format!("face {}", describe_num(y)),
                _ => "face".into(),
            },
            Action::Patrol { points, .. } => format!("patrol {}", operand(points)),
            Action::PickPoint { store, .. } => format!("pick point -> {store}"),
            Action::Play { clip, .. } => format!("play {}", operand(clip)),
            Action::AnimParam { name, .. } => format!("anim param {name}"),
            Action::AnimTrigger { name } => format!("anim trigger {name}"),
            Action::Find { tag, store, .. } => {
                format!("find {} -> {store}", tag.as_deref().unwrap_or("anything"))
            }
            other => other.type_name().to_owned(),
        },
        NodeKind::Subtree { name } => format!("subtree {name}"),
        NodeKind::Repeat { count: Some(n), .. } => format!("repeat {n}"),
        NodeKind::Retry { count } => format!("retry {count}"),
        other => other.type_name().to_owned(),
    }
}

fn parse_kind(kind: &str, p: &mut Params) -> Result<NodeKind, String> {
    let speed = |p: &mut Params| p.num("speed");
    Ok(match kind {
        "sequence" => NodeKind::Sequence,
        "selector" | "fallback" => NodeKind::Selector,
        "reactive_sequence" => NodeKind::ReactiveSequence,
        "reactive_selector" | "reactive_fallback" => NodeKind::ReactiveSelector,
        "parallel" => {
            let success = match p.take("success") {
                Some(def) => threshold_from(def).map_err(|e| p.err(format!("success: {e}")))?,
                None => Threshold::All,
            };
            let failure = match p.take("failure") {
                Some(def) => threshold_from(def).map_err(|e| p.err(format!("failure: {e}")))?,
                None => Threshold::Count(1),
            };
            NodeKind::Parallel { success, failure }
        }
        "shuffle_sequence" | "random_sequence" => NodeKind::ShuffleSequence,
        "shuffle_selector" | "random_selector" => NodeKind::ShuffleSelector,
        "random" => NodeKind::Random,
        "utility" => NodeKind::Utility {
            interval: p.number("interval")?.map(|v| v.max(0.05) as f32),
            hysteresis: p.number("hysteresis")?.unwrap_or(0.1).max(0.0),
        },
        "invert" | "inverter" | "not" => NodeKind::Invert,
        "force_success" | "succeeder" => NodeKind::ForceSuccess,
        "force_failure" | "failer" => NodeKind::ForceFailure,
        "repeat" => NodeKind::Repeat {
            count: p.number("count")?.map(|c| c.max(1.0) as u32),
            until: match p.text("until")? {
                Some(t) => Some(status_from(&t).map_err(|e| p.err(e))?),
                None => None,
            },
        },
        "retry" => NodeKind::Retry {
            count: p.number("count")?.unwrap_or(1.0).max(1.0) as u32,
        },
        "timeout" => NodeKind::Timeout {
            time: p.num("time")?.ok_or_else(|| p.err("needs time"))?,
        },
        "delay" => NodeKind::Delay {
            time: p.num("time")?.ok_or_else(|| p.err("needs time"))?,
        },
        "cooldown" => NodeKind::Cooldown {
            time: p.num("time")?.ok_or_else(|| p.err("needs time"))?,
        },
        "once" => NodeKind::Once,
        "guard" => NodeKind::Guard {
            cond: p.cond("when")?.ok_or_else(|| p.err("needs `when`"))?,
            abort: match p.text("abort")? {
                Some(t) => abort_from(&t).map_err(|e| p.err(e))?,
                None => Abort::None,
            },
        },
        "subtree" => NodeKind::Subtree {
            name: p.required_text("tree")?,
        },
        // Leaves.
        "succeed" | "success" => NodeKind::Leaf(Action::Succeed),
        "fail" | "failure" => NodeKind::Leaf(Action::Fail),
        "running" => NodeKind::Leaf(Action::Running),
        "wait" => NodeKind::Leaf(Action::Wait {
            time: p.num("time")?.ok_or_else(|| p.err("needs time"))?,
        }),
        "log" => NodeKind::Leaf(Action::Log {
            message: p.required_text("message")?,
        }),
        "set" => {
            let key = p.required_text("key")?;
            let value = match (p.literal("value")?, p.take("expr")) {
                (Some(_), Some(_)) => return Err(p.err("give value or expr, not both")),
                (Some(value), None) => value,
                (None, Some(Def::Str(s))) => Operand::Expr(Expr::parse(s).map_err(|e| p.err(e))?),
                (None, Some(_)) => return Err(p.err("expr must be text")),
                (None, None) => Operand::Const(Value::Nil),
            };
            NodeKind::Leaf(Action::Set { key, value })
        }
        "clear" => NodeKind::Leaf(Action::Clear {
            key: p.required_text("key")?,
        }),
        "add" => NodeKind::Leaf(Action::Add {
            key: p.required_text("key")?,
            amount: p.num("amount")?.unwrap_or(Num::Const(1.0)),
            min: p.number("min")?,
            max: p.number("max")?,
        }),
        "check" | "condition" => NodeKind::Leaf(Action::Check {
            cond: p.cond("expr")?.ok_or_else(|| p.err("needs a condition"))?,
        }),
        "wait_until" => NodeKind::Leaf(Action::WaitUntil {
            cond: p.cond("expr")?.ok_or_else(|| p.err("needs a condition"))?,
        }),
        "lua" | "action" => {
            let run = p.callable("run")?;
            let enter = p.callable("enter")?;
            let exit = p.callable("exit")?;
            if run.is_none() && enter.is_none() {
                return Err(p.err("needs run (or enter)"));
            }
            NodeKind::Leaf(Action::Lua { run, enter, exit })
        }
        "send" => {
            let event = p.required_text("event")?;
            let to = match p.take("to") {
                None => Recipient::Me,
                Some(Def::Str(s)) if s == "self" || s == "me" => Recipient::Me,
                Some(Def::Str(s)) if s == "group" => Recipient::Group,
                Some(Def::Str(s)) if s == "all" => Recipient::All,
                Some(def) => Recipient::To(operand_from(def, true).map_err(|e| p.err(e))?),
            };
            NodeKind::Leaf(Action::Send {
                event,
                to,
                data: p.literal("data")?,
            })
        }
        "broadcast" => {
            let event = p.required_text("event")?;
            let radius = p.num("radius")?;
            let tag = p.text("tag")?;
            let data = p.literal("data")?;
            NodeKind::Leaf(Action::Send {
                event,
                to: match radius {
                    Some(radius) => Recipient::Near { radius, tag },
                    None => Recipient::All,
                },
                data,
            })
        }
        "noise" => NodeKind::Leaf(Action::Noise {
            tag: p.required_text("tag")?,
            radius: p.num("radius")?.unwrap_or(Num::Const(10.0)),
            data: p.literal("data")?,
            at: p.operand("at")?,
        }),
        "emit" => NodeKind::Leaf(Action::Emit {
            event: p.required_text("event")?,
            data: p.literal("data")?,
        }),
        "wait_event" => NodeKind::Leaf(Action::WaitEvent {
            event: p.required_text("event")?,
            store: p.text("store")?,
            store_from: p.text("store_from")?,
            store_position: p.text("store_position")?,
        }),
        "move_to" => NodeKind::Leaf(Action::MoveTo {
            target: p
                .operand("target")?
                .ok_or_else(|| p.err("needs a target"))?,
            radius: p.number("radius")?.unwrap_or(0.0).max(0.0) as f32,
            speed: speed(p)?,
            run: p.flag("run")?.unwrap_or(false),
            repath: p.number("repath")?.unwrap_or(0.5).max(0.05) as f32,
            give_up: p.num("give_up")?,
            accept_partial: p.number("accept_partial")?.map(|v| v as f32),
        }),
        "wander" => NodeKind::Leaf(Action::Wander {
            center: p.operand("center")?,
            radius: p.num("radius")?.unwrap_or(Num::Const(8.0)),
            min_distance: p.number("min_distance")?.unwrap_or(1.5) as f32,
            speed: speed(p)?,
            store: p.text("store")?,
        }),
        "flee" => NodeKind::Leaf(Action::Flee {
            from: p.operand("from")?.ok_or_else(|| p.err("needs from"))?,
            distance: p.num("distance")?.unwrap_or(Num::Const(8.0)),
            speed: speed(p)?,
            run: p.flag("run")?.unwrap_or(true),
        }),
        "follow" => NodeKind::Leaf(Action::Follow {
            target: p
                .operand("target")?
                .ok_or_else(|| p.err("needs a target"))?,
            distance: p.number("distance")?.unwrap_or(1.5) as f32,
            speed: speed(p)?,
            run: p.flag("run")?.unwrap_or(false),
        }),
        "face" => {
            let target = p.operand("target")?;
            let yaw = p.num("yaw")?;
            if target.is_none() && yaw.is_none() {
                return Err(p.err("needs a target or a yaw"));
            }
            NodeKind::Leaf(Action::Face {
                target,
                yaw,
                wait: p.number("wait")?.unwrap_or(0.0).max(0.0) as f32,
            })
        }
        "stop" => NodeKind::Leaf(Action::Stop),
        "patrol" => NodeKind::Leaf(Action::Patrol {
            points: p.operand("points")?.ok_or_else(|| p.err("needs points"))?,
            index_key: p.text("index")?.unwrap_or_else(|| "patrol_index".into()),
            radius: p.number("radius")?.unwrap_or(0.0).max(0.0) as f32,
            speed: speed(p)?,
        }),
        "pick_point" => NodeKind::Leaf(Action::PickPoint {
            store: p.required_text("store")?,
            around: p.operand("around")?,
            radius: p.num("radius")?.unwrap_or(Num::Const(8.0)),
            min_distance: p.number("min_distance")?.unwrap_or(0.0) as f32,
        }),
        "play" => {
            let clip = p.literal("clip")?.ok_or_else(|| p.err("needs a clip"))?;
            let layer = p.number("layer")?.unwrap_or(1.0);
            if !(0.0..crate::scene::skeletal::MAX_LAYERS as f64).contains(&layer) {
                return Err(p.err(format!(
                    "layer must be 0..{}",
                    crate::scene::skeletal::MAX_LAYERS - 1
                )));
            }
            NodeKind::Leaf(Action::Play {
                clip,
                layer: layer as usize,
                looping: p.flag("loop")?,
                fade: p.number("fade")?.unwrap_or(0.4).max(0.0) as f32,
                speed: p.number("speed")?.unwrap_or(1.0) as f32,
                time: p.num("time")?,
                stop_on_exit: p.flag("stop")?.unwrap_or(true),
            })
        }
        "stop_animation" => NodeKind::Leaf(Action::StopAnimation {
            layer: p.number("layer")?.map(|l| l.max(0.0) as usize),
            fade: p.number("fade")?.unwrap_or(0.4).max(0.0) as f32,
        }),
        "anim_param" => NodeKind::Leaf(Action::AnimParam {
            name: p.required_text("name")?,
            value: p.operand("value")?.ok_or_else(|| p.err("needs a value"))?,
        }),
        "anim_trigger" => NodeKind::Leaf(Action::AnimTrigger {
            name: p.required_text("name")?,
        }),
        "find" => NodeKind::Leaf(Action::Find {
            tag: p.text("tag")?,
            store: p.required_text("store")?,
            range: p.number("range")?.map(|r| r as f32),
            visible: p.flag("visible")?.unwrap_or(false),
            filter: p.cond("filter")?,
        }),
        other => return Err(p.err(format!("unknown node type {other:?}"))),
    })
}

/// Read a state machine: `{ initial = "idle", states = {...}, transitions = {...} }`.
///
/// ```
/// use wgpu_cube::ai::{parse_machine, Def};
///
/// let json = serde_json::json!({
///     "initial": "idle",
///     "states": {
///         "idle": { "transitions": [{ "to": "alert", "event": "noise" }] },
///         "alert": { "transitions": [{ "to": "idle", "after": 3 }] }
///     }
/// });
/// let machine = parse_machine("guard", &Def::from_json(&json)).unwrap();
/// assert_eq!(machine.states[machine.initial].name, "idle");
/// ```
pub fn parse_machine(name: &str, def: &Def) -> Result<MachineDef, String> {
    parse_machine_at(name, def, name, 0)
}

fn parse_machine_at(name: &str, def: &Def, path: &str, depth: usize) -> Result<MachineDef, String> {
    if depth > 16 {
        return Err(format!("{path}: machines are nested too deeply"));
    }
    let Def::Table { named, .. } = def else {
        return Err(format!("{path}: a machine is a table, not {}", def.kind()));
    };
    let mut params = Params::new(
        path.to_owned(),
        named.iter().map(|(k, v)| (k.as_str(), v)).collect(),
    );
    let _comment = params.take("comment");
    let states_def = params
        .take("states")
        .ok_or_else(|| format!("{path}: a machine needs states"))?;
    let initial = params.text("initial")?;
    let transitions_def = params.take("transitions");
    params.finish()?;

    // States as a map (sorted by name) or a list of tables with names.
    let mut entries: Vec<(String, &Def)> = Vec::new();
    match states_def {
        Def::Table { named, items } => {
            let mut sorted: Vec<&(String, Def)> = named.iter().collect();
            sorted.sort_by(|a, b| a.0.cmp(&b.0));
            for (state_name, state) in sorted {
                entries.push((state_name.clone(), state));
            }
            for (i, state) in items.iter().enumerate() {
                match state.field("name") {
                    Some(Def::Str(n)) => entries.push((n.clone(), state)),
                    _ => return Err(format!("{path}: state {} in the list needs a name", i + 1)),
                }
            }
        }
        other => {
            return Err(format!(
                "{path}: states must be a table, not {}",
                other.kind()
            ))
        }
    }
    if entries.is_empty() {
        return Err(format!("{path}: a machine needs at least one state"));
    }
    for (i, (state_name, _)) in entries.iter().enumerate() {
        if entries[..i].iter().any(|(n, _)| n == state_name) {
            return Err(format!("{path}: two states are called {state_name:?}"));
        }
        if state_name.starts_with('@') {
            return Err(format!("{path}: state names cannot start with '@'"));
        }
    }
    let names: Vec<String> = entries.iter().map(|(n, _)| n.clone()).collect();
    let initial = match initial {
        Some(initial) => names
            .iter()
            .position(|n| *n == initial)
            .ok_or_else(|| format!("{path}: initial state {initial:?} is not one of its states"))?,
        None if names.len() == 1 => 0,
        None => return Err(format!("{path}: say which state is `initial`")),
    };

    let mut states = Vec::with_capacity(entries.len());
    for (state_name, state_def) in &entries {
        states.push(parse_state(
            state_name,
            state_def,
            &names,
            &format!("{path}.{state_name}"),
            depth,
        )?);
    }
    let transitions = match transitions_def {
        Some(def) => parse_transitions(def, &names, &format!("{path}.transitions"), true)?,
        None => Vec::new(),
    };
    Ok(MachineDef {
        name: name.to_owned(),
        states,
        initial,
        transitions,
    })
}

fn parse_state(
    name: &str,
    def: &Def,
    names: &[String],
    path: &str,
    depth: usize,
) -> Result<StateDef, String> {
    let Def::Table { named, items } = def else {
        return Err(format!("{path}: a state is a table, not {}", def.kind()));
    };
    if !items.is_empty() {
        return Err(format!("{path}: a state takes named fields only"));
    }
    let mut p = Params::new(
        path.to_owned(),
        named.iter().map(|(k, v)| (k.as_str(), v)).collect(),
    );
    let _ = p.take("name");
    let _comment = p.take("comment");
    let enter = p.callable("enter")?;
    let update = p.callable("update")?;
    let exit = p.callable("exit")?;
    let tree = match p.take("tree") {
        None => None,
        Some(Def::Str(tree_name)) => Some(BrainRef::Named(tree_name.clone())),
        Some(tree_def) => Some(BrainRef::Inline(Arc::new(parse_tree(
            &format!("{path}.tree"),
            tree_def,
        )?))),
    };
    let machine = match p.take("machine") {
        None => None,
        Some(Def::Str(machine_name)) => Some(BrainRef::Named(machine_name.clone())),
        Some(machine_def) => Some(BrainRef::Inline(Arc::new(parse_machine_at(
            &format!("{path}.machine"),
            machine_def,
            &format!("{path}.machine"),
            depth + 1,
        )?))),
    };
    let finish = match p.take("final") {
        None | Some(Def::Bool(false)) => None,
        Some(Def::Bool(true)) => Some(Status::Success),
        Some(Def::Str(s)) => Some(status_from(s).map_err(|e| p.err(format!("final: {e}")))?),
        Some(other) => {
            return Err(p.err(format!(
                "final must be true or a result, not {}",
                other.kind()
            )))
        }
    };
    let set = match p.take("set") {
        None => Vec::new(),
        Some(Def::Table { named, items }) if items.is_empty() => {
            let mut set = Vec::new();
            for (key, value) in named {
                set.push((
                    key.clone(),
                    operand_from(value, false).map_err(|e| p.err(format!("set.{key}: {e}")))?,
                ));
            }
            set.sort_by(|a, b| a.0.cmp(&b.0));
            set
        }
        Some(_) => return Err(p.err("set must be a table of key = value")),
    };
    let transitions = match p.take("transitions") {
        Some(def) => parse_transitions(def, names, &format!("{path}.transitions"), false)?,
        None => Vec::new(),
    };
    p.finish()?;
    Ok(StateDef {
        name: name.to_owned(),
        enter,
        update,
        exit,
        tree,
        machine,
        finish,
        set,
        transitions,
    })
}

fn parse_transitions(
    def: &Def,
    names: &[String],
    path: &str,
    machine_wide: bool,
) -> Result<Vec<TransitionDef>, String> {
    let Some(list) = def.list() else {
        return Err(format!("{path}: transitions must be a list"));
    };
    list.iter()
        .enumerate()
        .map(|(i, t)| parse_transition(t, names, &format!("{path}[{}]", i + 1), machine_wide))
        .collect()
}

fn parse_transition(
    def: &Def,
    names: &[String],
    path: &str,
    machine_wide: bool,
) -> Result<TransitionDef, String> {
    let Def::Table { named, items } = def else {
        return Err(format!(
            "{path}: a transition is a table, not {}",
            def.kind()
        ));
    };
    let mut fields: Vec<(&str, &Def)> = named.iter().map(|(k, v)| (k.as_str(), v)).collect();
    match items.as_slice() {
        [] => {}
        [to] => fields.push(("to", to)),
        _ => return Err(format!("{path}: only the target state goes first")),
    }
    let mut p = Params::new(path.to_owned(), fields);
    let _comment = p.take("comment");
    let to_name = p.required_text("to")?;
    let to = match to_name.as_str() {
        "@previous" => TransitionTarget::Previous,
        "@restart" | "@self" => TransitionTarget::Restart,
        name => TransitionTarget::State(
            names
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| p.err(format!("no state called {name:?}")))?,
        ),
    };
    let event = p.text("event")?;
    let when = p.cond("when")?;
    let after = p.num("after")?;
    let on = match p.text("on")? {
        None => None,
        Some(t) => Some(match t.as_str() {
            "success" => DoneOn::Success,
            "failure" => DoneOn::Failure,
            "done" => DoneOn::Done,
            other => return Err(p.err(format!("on is success, failure or done, not {other:?}"))),
        }),
    };
    let chance = p.number("chance")?.map(|c| c.clamp(0.0, 1.0));
    let store = p.text("store")?;
    let store_from = p.text("store_from")?;
    let store_position = p.text("store_position")?;
    let except = match p.take("except") {
        None => Vec::new(),
        Some(_) if !machine_wide => {
            return Err(p.err("except is for machine-wide transitions"));
        }
        Some(Def::Str(s)) => vec![s.clone()],
        Some(def) => match def.list() {
            Some(items) => items
                .iter()
                .map(|d| match d {
                    Def::Str(s) => Ok(s.clone()),
                    other => Err(p.err(format!("except lists state names, not {}", other.kind()))),
                })
                .collect::<Result<Vec<_>, _>>()?,
            None => return Err(p.err("except is a state name or a list of them")),
        },
    };
    let except = except
        .iter()
        .map(|name| {
            names
                .iter()
                .position(|n| n == name)
                .ok_or_else(|| p.err(format!("except: no state called {name:?}")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    p.finish()?;
    if event.is_none() && when.is_none() && after.is_none() && on.is_none() {
        return Err(format!(
            "{path}: a transition needs an event, when, after or on"
        ));
    }
    Ok(TransitionDef {
        to,
        event,
        when,
        after,
        on,
        chance,
        store,
        store_from,
        store_position,
        except,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tree(json: serde_json::Value) -> Result<TreeDef, String> {
        parse_tree("test", &Def::from_json(&json))
    }

    #[test]
    fn shared_parameters_wrap_the_node() {
        let t = tree(json!({
            "type": "selector",
            "children": [
                {"type": "wait", "time": 1, "when": "alarm", "abort": "lower", "cooldown": 5, "score": 2},
                {"type": "succeed"}
            ]
        }))
        .unwrap();
        // selector, guard, cooldown, wait, succeed
        assert_eq!(t.nodes.len(), 5);
        assert!(matches!(
            t.nodes[1].kind,
            NodeKind::Guard {
                abort: Abort::Lower,
                ..
            }
        ));
        assert!(t.nodes[1].score.is_some());
        assert!(matches!(t.nodes[2].kind, NodeKind::Cooldown { .. }));
        assert!(matches!(
            t.nodes[3].kind,
            NodeKind::Leaf(Action::Wait { .. })
        ));
        assert_eq!(t.nodes[0].children, vec![1, 4]);
        assert_eq!(t.nodes[3].parent, Some(2));
    }

    #[test]
    fn mistakes_are_reported_with_their_place() {
        let err =
            tree(json!({"type": "sequence", "children": [{"type": "move_to", "targt": "x"}]}))
                .unwrap_err();
        assert!(err.contains("sequence[1] > move_to"), "{err}");
        let err =
            tree(json!({"type": "invert", "children": [{"type": "succeed"}, {"type": "fail"}]}))
                .unwrap_err();
        assert!(err.contains("exactly one child"), "{err}");
        assert!(tree(json!({"type": "sequence"}))
            .unwrap_err()
            .contains("no children"));
        assert!(tree(json!({"type": "wait", "time": "1 +"})).is_err());
        assert!(tree(json!({"type": "check", "expr": "a", "abort": "self"}))
            .unwrap_err()
            .contains("needs a `when`"));
    }

    #[test]
    fn positional_arguments_in_table_form() {
        // As Lua writes it: { "move_to", "spot", run = true }.
        let def = Def::Table {
            named: vec![("run".into(), Def::Bool(true))],
            items: vec![Def::Str("move_to".into()), Def::Str("spot".into())],
        };
        let t = parse_tree("x", &def).unwrap();
        match &t.nodes[0].kind {
            NodeKind::Leaf(Action::MoveTo {
                target: Operand::Expr(e),
                run: true,
                ..
            }) => {
                assert_eq!(e.source(), "spot")
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(t.nodes[0].label, "move to spot");
    }

    #[test]
    fn machines_check_their_states() {
        let ok = parse_machine(
            "m",
            &Def::from_json(&json!({
                "initial": "a",
                "states": {
                    "a": {"tree": {"type": "wait", "time": 1}, "transitions": [{"to": "b", "on": "success"}]},
                    "b": {"final": true, "set": {"done": true}}
                },
                "transitions": [{"to": "a", "event": "reset", "except": ["a"]}]
            })),
        )
        .unwrap();
        assert_eq!(ok.states.len(), 2);
        assert_eq!(ok.transitions[0].except, vec![0]);
        assert_eq!(ok.states[1].finish, Some(Status::Success));

        let bad = parse_machine(
            "m",
            &Def::from_json(&json!({"initial": "a", "states": {"a": {"transitions": [{"to": "nowhere", "after": 1}]}}})),
        )
        .unwrap_err();
        assert!(bad.contains("no state called \"nowhere\""), "{bad}");
        let bad = parse_machine("m", &Def::from_json(&json!({"states": {"a": {}, "b": {}}})))
            .unwrap_err();
        assert!(bad.contains("initial"), "{bad}");
        let bad = parse_machine(
            "m",
            &Def::from_json(
                &json!({"initial": "a", "states": {"a": {"transitions": [{"to": "a"}]}}}),
            ),
        )
        .unwrap_err();
        assert!(bad.contains("needs an event"), "{bad}");
    }
}
