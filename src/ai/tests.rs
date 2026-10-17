//! Behaviour tree and state machine tests over a toy world: agents walk straight to
//! where they are sent, clips end after a few ticks, and named "Lua" functions are Rust
//! closures.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};

use glam::Vec3;
use hecs::Entity;
use rand::rngs::SmallRng;
use rand::SeedableRng;
use serde_json::json;

use super::blackboard::BlackboardRef;
use super::brain::BrainRuntime;
use super::context::{
    AiEvent, AiHost, Boards, Cx, LineOfSight, LuaCall, NavInfo, NavPhase, Outgoing, PlayRequest,
};
use super::def::{parse_brain, Callable, Def, Library, Status};
use super::perception::{look, PerceptionSettings, Senses, Visible, VisibleGrid};
use super::value::Value;

type Function = Box<dyn FnMut(&LuaCall) -> Value>;

#[derive(Default)]
struct Mock {
    positions: HashMap<Entity, Vec3>,
    yaws: HashMap<Entity, f32>,
    /// Where each agent walks, and whether it got there.
    walks: HashMap<Entity, (Vec3, NavPhase)>,
    speeds: HashMap<Entity, f32>,
    /// Clips playing: (entity, layer) -> (clip, ticks left).
    clips: HashMap<(Entity, usize), (String, u32)>,
    log: Vec<String>,
    functions: HashMap<String, Function>,
    warnings: Vec<String>,
    /// Pairs that cannot see each other.
    walls: HashSet<(Entity, Entity)>,
}

impl Mock {
    /// The world moves on: walkers advance, clips play.
    fn advance(&mut self, dt: f64) {
        for (entity, (goal, phase)) in self.walks.iter_mut() {
            if *phase != NavPhase::Moving {
                continue;
            }
            let speed = self.speeds.get(entity).copied().unwrap_or(2.0);
            let pos = self.positions.entry(*entity).or_default();
            let to = *goal - *pos;
            let step = speed * dt as f32;
            if to.length() <= step + 0.01 {
                *pos = *goal;
                *phase = NavPhase::Arrived;
            } else {
                *pos += to.normalize() * step;
            }
        }
        for (_, ticks) in self.clips.values_mut() {
            *ticks = ticks.saturating_sub(1);
        }
    }

    fn function(&mut self, name: &str, f: impl FnMut(&LuaCall) -> Value + 'static) {
        self.functions.insert(name.to_owned(), Box::new(f));
    }
}

impl AiHost for Mock {
    fn position(&self, entity: Entity) -> Option<Vec3> {
        self.positions.get(&entity).copied()
    }
    fn yaw(&self, entity: Entity) -> Option<f32> {
        Some(self.yaws.get(&entity).copied().unwrap_or(0.0))
    }
    fn exists(&self, entity: Entity) -> bool {
        self.positions.contains_key(&entity)
    }
    fn nav_info(&self, entity: Entity) -> Option<NavInfo> {
        let phase = self.walks.get(&entity).map_or(NavPhase::Idle, |w| w.1);
        Some(NavInfo {
            phase,
            remaining: None,
            partial: false,
            speed: 0.0,
        })
    }
    fn nav_move(&mut self, entity: Entity, target: Vec3) -> bool {
        self.log
            .push(format!("move {:.0} {:.0}", target.x, target.z));
        self.walks.insert(entity, (target, NavPhase::Moving));
        true
    }
    fn nav_stop(&mut self, entity: Entity) {
        self.log.push("stop".into());
        self.walks.remove(&entity);
    }
    fn face(&mut self, entity: Entity, yaw: f32) {
        self.yaws.insert(entity, yaw);
    }
    fn nav_random_point(&self, around: Vec3, radius: f32) -> Option<Vec3> {
        Some(around + Vec3::new(radius * 0.5, 0.0, 0.0))
    }
    fn nav_closest(&self, point: Vec3, _radius: f32) -> Option<Vec3> {
        Some(point)
    }
    fn line_of_sight(&self, _: Vec3, _: Vec3, _: f32, _: LineOfSight, ignore: &[Entity]) -> bool {
        !(ignore.len() == 2 && self.walls.contains(&(ignore[0], ignore[1])))
    }
    fn agent_speed(&self, entity: Entity) -> Option<f32> {
        Some(self.speeds.get(&entity).copied().unwrap_or(2.0))
    }
    fn set_agent_speed(&mut self, entity: Entity, speed: f32) {
        self.speeds.insert(entity, speed);
    }
    fn play(&mut self, entity: Entity, request: &PlayRequest) -> bool {
        self.log.push(format!("play {}", request.clip));
        self.clips
            .insert((entity, request.layer), (request.clip.clone(), 3));
        true
    }
    fn stop_animation(&mut self, entity: Entity, layer: Option<usize>, _fade: f32) {
        self.log.push("stop clip".into());
        if let Some(layer) = layer {
            self.clips.remove(&(entity, layer));
        }
    }
    fn animation_done(&self, entity: Entity, layer: usize, clip: &str) -> bool {
        self.clips
            .get(&(entity, layer))
            .is_none_or(|(c, ticks)| c != clip || *ticks == 0)
    }
    fn set_anim_param(&mut self, _: Entity, name: &str, value: &Value) {
        self.log.push(format!("param {name} {value}"));
    }
    fn anim_trigger(&mut self, _: Entity, name: &str) {
        self.log.push(format!("trigger {name}"));
    }
    fn call(&mut self, function: &Callable, call: &LuaCall) -> Result<Value, String> {
        let Callable::Named(name) = function else {
            return Err("no Lua here".into());
        };
        let f = self
            .functions
            .get_mut(&**name)
            .ok_or_else(|| format!("no function {name}"))?;
        Ok(f(call))
    }
    fn positions(&mut self, value: &Value) -> Vec<Vec3> {
        match value {
            Value::Json(json) => json
                .as_array()
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|i| Value::from_json(i).as_vec3())
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }
    fn new_memory(&mut self) -> Value {
        Value::Nil
    }
    fn emit(&mut self, event: &str, _data: &Value) {
        self.log.push(format!("emit {event}"));
    }
    fn log(&mut self, _: Entity, message: &str) {
        self.log.push(format!("log {message}"));
    }
    fn warn(&mut self, _: Entity, message: &str) {
        self.warnings.push(message.to_owned());
    }
}

struct Harness {
    host: Mock,
    library: RefCell<Library>,
    boards: Boards,
    senses: Senses,
    rng: SmallRng,
    time: f64,
    frame: u64,
    agent: Entity,
    events: Vec<AiEvent>,
    outbox: Vec<Outgoing>,
    brain: BrainRuntime,
    _world: hecs::World,
}

impl Harness {
    fn new(def: serde_json::Value) -> Self {
        let mut world = hecs::World::new();
        let agent = world.spawn(());
        let brain = parse_brain("test", &Def::from_json(&def)).unwrap_or_else(|e| panic!("{e}"));
        let mut host = Mock::default();
        host.positions.insert(agent, Vec3::ZERO);
        Self {
            host,
            library: RefCell::new(Library::default()),
            boards: Boards {
                own: BlackboardRef::default(),
                group: BlackboardRef::default(),
                global: BlackboardRef::default(),
            },
            senses: Senses::default(),
            rng: SmallRng::seed_from_u64(7),
            time: 0.0,
            frame: 0,
            agent,
            events: Vec::new(),
            outbox: Vec::new(),
            brain: BrainRuntime::new(&brain),
            _world: world,
        }
    }

    fn tick(&mut self, dt: f64) -> Status {
        self.time += dt;
        self.frame += 1;
        self.host.advance(dt);
        let events = std::mem::take(&mut self.events);
        let mut cx = Cx {
            host: &mut self.host,
            library: &self.library,
            agent: self.agent,
            boards: &self.boards,
            time: self.time,
            dt: dt as f32,
            frame: self.frame,
            events: &events,
            senses: &self.senses,
            rng: &mut self.rng,
            outbox: &mut self.outbox,
            states: Vec::new(),
            state_entered: self.time,
            event: None,
            run_speed: 4.0,
            it: None,
        };
        self.brain.tick(&mut cx)
    }

    fn get(&self, key: &str) -> Value {
        self.boards.get(key)
    }

    fn set(&self, key: &str, value: impl Into<Value>) {
        self.boards.set(key, value.into());
    }

    fn state(&self) -> String {
        self.brain
            .state_path()
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
            .join("/")
    }

    fn send(&mut self, name: &str) {
        self.events.push(AiEvent {
            name: name.into(),
            data: Value::Int(7),
            from: None,
            position: Some(Vec3::new(1.0, 0.0, 2.0)),
        });
    }
}

#[test]
fn selector_and_sequence_follow_the_blackboard() {
    let mut h = Harness::new(json!({
        "type": "selector", "children": [
            {"type": "sequence", "children": [
                {"type": "check", "expr": "hungry"},
                {"type": "set", "key": "ate", "value": true}
            ]},
            {"type": "set", "key": "idle", "value": true}
        ]
    }));
    assert_eq!(h.tick(0.1), Status::Success);
    assert_eq!(h.get("idle"), Value::Bool(true));
    assert!(h.get("ate").is_nil());
    h.set("hungry", true);
    assert_eq!(h.tick(0.1), Status::Success);
    assert_eq!(h.get("ate"), Value::Bool(true));
}

#[test]
fn wait_runs_for_its_time() {
    let mut h = Harness::new(json!({
        "type": "sequence", "children": [
            {"type": "wait", "time": 1},
            {"type": "set", "key": "done", "value": true}
        ]
    }));
    for _ in 0..3 {
        assert_eq!(h.tick(0.4), Status::Running);
    }
    assert_eq!(h.tick(0.4), Status::Success);
    assert_eq!(h.get("done"), Value::Bool(true));
}

#[test]
fn guard_with_self_abort_stops_its_branch() {
    let mut h = Harness::new(json!({
        "type": "sequence", "when": "alarm", "abort": "self",
        "children": [{"type": "move_to", "target": [10, 0, 0]}]
    }));
    h.set("alarm", true);
    assert_eq!(h.tick(0.1), Status::Running);
    assert_eq!(h.tick(0.1), Status::Running);
    h.set("alarm", false);
    assert_eq!(h.tick(0.1), Status::Failure);
    assert_eq!(h.host.log, vec!["move 10 0", "stop"]);
}

#[test]
fn guard_with_lower_abort_interrupts_later_branches() {
    let mut h = Harness::new(json!({
        "type": "selector", "children": [
            {"type": "sequence", "when": "alarm", "abort": "lower", "children": [
                {"type": "set", "key": "doing", "value": "flee"},
                {"type": "wait", "time": 5}
            ]},
            {"type": "sequence", "children": [
                {"type": "set", "key": "doing", "value": "idle"},
                {"type": "play", "clip": "bored", "time": 10}
            ]}
        ]
    }));
    assert_eq!(h.tick(0.1), Status::Running);
    assert_eq!(h.get("doing"), Value::str("idle"));
    assert_eq!(h.tick(0.1), Status::Running);
    h.set("alarm", true);
    assert_eq!(h.tick(0.1), Status::Running);
    assert_eq!(h.get("doing"), Value::str("flee"));
    // The idle branch was left: its clip stopped.
    assert_eq!(h.host.log, vec!["play bored", "stop clip"]);
}

#[test]
fn utility_picks_the_best_score_and_switches_with_hysteresis() {
    let mut h = Harness::new(json!({
        "type": "utility", "interval": 0.5, "hysteresis": 0.1, "children": [
            {"type": "sequence", "score": "hunger", "children": [
                {"type": "set", "key": "doing", "value": "eat"},
                {"type": "wait", "time": 100}
            ]},
            {"type": "sequence", "score": "tired", "children": [
                {"type": "set", "key": "doing", "value": "sleep"},
                {"type": "wait", "time": 100}
            ]},
            {"type": "sequence", "score": -1, "children": [
                {"type": "set", "key": "doing", "value": "never"}
            ]}
        ]
    }));
    h.set("hunger", 0.8);
    h.set("tired", 0.2);
    h.tick(0.1);
    assert_eq!(h.get("doing"), Value::str("eat"));
    // Close, but not by the hysteresis: it keeps eating.
    h.set("tired", 0.85);
    h.tick(0.6);
    assert_eq!(h.get("doing"), Value::str("eat"));
    h.set("tired", 0.95);
    h.tick(0.6);
    assert_eq!(h.get("doing"), Value::str("sleep"));
}

#[test]
fn parallel_counts_successes_and_failures() {
    let mut h = Harness::new(json!({
        "type": "parallel", "success": 1, "children": [
            {"type": "wait", "time": 1},
            {"type": "play", "clip": "dance", "time": 5}
        ]
    }));
    assert_eq!(h.tick(0.5), Status::Running);
    assert_eq!(h.tick(0.6), Status::Running);
    assert_eq!(h.tick(0.6), Status::Success);
    assert_eq!(h.host.log, vec!["play dance", "stop clip"]);

    let mut h = Harness::new(json!({
        "type": "parallel", "children": [
            {"type": "fail"},
            {"type": "wait", "time": 5}
        ]
    }));
    assert_eq!(h.tick(0.1), Status::Failure);
}

#[test]
fn decorators() {
    // repeat
    let mut h = Harness::new(json!({"type": "repeat", "count": 3, "children": [
        {"type": "add", "key": "n", "amount": 1}
    ]}));
    assert_eq!(h.tick(0.1), Status::Running);
    assert_eq!(h.tick(0.1), Status::Running);
    assert_eq!(h.tick(0.1), Status::Success);
    assert_eq!(h.get("n"), Value::Num(3.0));

    // retry until the child succeeds
    let mut h = Harness::new(json!({"type": "retry", "count": 5, "children": [
        {"type": "sequence", "children": [
            {"type": "add", "key": "tries", "amount": 1},
            {"type": "check", "expr": "tries >= 3"}
        ]}
    ]}));
    assert_eq!(h.tick(0.1), Status::Running);
    assert_eq!(h.tick(0.1), Status::Running);
    assert_eq!(h.tick(0.1), Status::Success);

    // timeout
    let mut h = Harness::new(json!({"type": "wait", "time": 5, "timeout": 1}));
    assert_eq!(h.tick(0.6), Status::Running);
    assert_eq!(h.tick(0.6), Status::Running);
    assert_eq!(h.tick(0.6), Status::Failure);

    // cooldown: the branch runs, then is skipped for 2 s
    let mut h = Harness::new(json!({"type": "selector", "children": [
        {"type": "add", "key": "n", "amount": 1, "cooldown": 2},
        {"type": "succeed"}
    ]}));
    for _ in 0..4 {
        h.tick(0.5);
    }
    assert_eq!(h.get("n"), Value::Num(1.0));
    h.tick(0.5);
    assert_eq!(h.get("n"), Value::Num(2.0));

    // once
    let mut h = Harness::new(json!({"type": "once", "children": [
        {"type": "add", "key": "n", "amount": 1}
    ]}));
    for _ in 0..5 {
        assert_eq!(h.tick(0.1), Status::Success);
    }
    assert_eq!(h.get("n"), Value::Num(1.0));

    // chance and invert
    let mut never = Harness::new(json!({"type": "succeed", "chance": 0}));
    assert_eq!(never.tick(0.1), Status::Failure);
    let mut h = Harness::new(json!({"type": "invert", "children": [{"type": "fail"}]}));
    assert_eq!(h.tick(0.1), Status::Success);
}

#[test]
fn reactive_sequence_halts_the_running_action() {
    let mut h = Harness::new(json!({"type": "reactive_sequence", "children": [
        {"type": "check", "expr": "go"},
        {"type": "move_to", "target": "spot", "run": true}
    ]}));
    h.set("go", true);
    h.set("spot", Vec3::new(20.0, 0.0, 0.0));
    assert_eq!(h.tick(0.1), Status::Running);
    // Running uses the agent's run speed, and puts its own speed back afterwards.
    assert_eq!(h.host.speeds.get(&h.agent), Some(&4.0));
    h.set("go", false);
    assert_eq!(h.tick(0.1), Status::Failure);
    assert_eq!(h.host.log, vec!["move 20 0", "stop"]);
    assert_eq!(h.host.speeds.get(&h.agent), Some(&2.0));
}

#[test]
fn move_to_arrives_and_follows_entities() {
    let mut h = Harness::new(json!({"type": "move_to", "target": [3, 0, 0]}));
    let mut ticks = 0;
    while h.tick(0.5) == Status::Running {
        ticks += 1;
        assert!(ticks < 10, "never arrived");
    }
    assert!(h.host.positions[&h.agent].distance(Vec3::new(3.0, 0.0, 0.0)) < 0.05);

    // An entity target: re-planned as it moves; `radius` succeeds when close.
    let mut h =
        Harness::new(json!({"type": "move_to", "target": "friend", "radius": 1.0, "repath": 0.1}));
    let friend = h._world.spawn(());
    h.host.positions.insert(friend, Vec3::new(4.0, 0.0, 0.0));
    h.set("friend", Value::entity(friend));
    assert_eq!(h.tick(0.1), Status::Running);
    h.host.positions.insert(friend, Vec3::new(4.0, 0.0, 3.0));
    let mut status = Status::Running;
    for _ in 0..20 {
        status = h.tick(0.25);
        if status != Status::Running {
            break;
        }
    }
    assert_eq!(status, Status::Success);
    assert!(
        h.host.log.contains(&"move 4 3".to_string()),
        "{:?}",
        h.host.log
    );
    assert_eq!(h.host.log.last().map(String::as_str), Some("stop"));
}

#[test]
fn play_waits_for_one_shots_and_lua_leaves_run() {
    let mut h = Harness::new(json!({"type": "sequence", "children": [
        {"type": "play", "clip": "wave"},
        {"type": "lua", "run": "count"},
        {"type": "log", "message": "count is {n}"}
    ]}));
    h.host.function("count", |call| {
        let n = call.board.get("n").as_f64().unwrap_or(0.0) + 1.0;
        call.board.set("n", Value::Num(n));
        if n < 2.0 {
            Value::str("running")
        } else {
            Value::Bool(true)
        }
    });
    let mut status = Status::Running;
    for _ in 0..10 {
        status = h.tick(0.1);
        if status != Status::Running {
            break;
        }
    }
    assert_eq!(status, Status::Success);
    assert_eq!(h.get("n"), Value::Num(2.0));
    assert_eq!(
        h.host.log.last().map(String::as_str),
        Some("log count is 2")
    );
    assert!(h.host.warnings.is_empty(), "{:?}", h.host.warnings);
}

#[test]
fn machine_transitions_on_events_time_trees_and_previous() {
    let mut h = Harness::new(json!({
        "initial": "idle",
        "states": {
            "idle": {
                "tree": {"type": "wait", "time": 1},
                "transitions": [
                    {"to": "alert", "event": "noise", "store_position": "noise_at", "store": "loudness"},
                    {"to": "walk", "on": "success"}
                ]
            },
            "walk": {"set": {"walked": true}, "transitions": [{"to": "idle", "after": 2}]},
            "alert": {"enter": "on_alert", "transitions": [{"to": "@previous", "after": 1}]}
        },
        "transitions": [{"to": "idle", "event": "reset", "except": ["idle"]}]
    }));
    h.host.function("on_alert", |call| {
        call.board.set("alerted", Value::Bool(true));
        assert_eq!(call.event.map(|e| &*e.name), Some("noise"));
        Value::Nil
    });
    h.tick(0.1);
    assert_eq!(h.state(), "idle");
    h.tick(0.6);
    h.tick(0.6);
    assert_eq!(h.state(), "walk");
    assert_eq!(h.get("walked"), Value::Bool(true));
    h.tick(1.0);
    assert_eq!(h.state(), "walk");
    h.tick(1.1);
    assert_eq!(h.state(), "idle");

    h.send("noise");
    h.tick(0.1);
    assert_eq!(h.state(), "alert");
    assert_eq!(h.get("alerted"), Value::Bool(true));
    assert_eq!(h.get("noise_at"), Value::Vec3(Vec3::new(1.0, 0.0, 2.0)));
    assert_eq!(h.get("loudness"), Value::Int(7));
    h.tick(1.1);
    assert_eq!(h.state(), "idle");

    // Machine-wide, except from idle.
    h.send("reset");
    h.tick(0.1);
    assert_eq!(h.state(), "idle");
    h.tick(1.0);
    assert_eq!(h.state(), "walk");
    h.send("reset");
    h.tick(0.1);
    assert_eq!(h.state(), "idle");
}

#[test]
fn nested_machines_finish_into_their_parent_and_update_can_switch() {
    let mut h = Harness::new(json!({
        "initial": "work",
        "states": {
            "work": {
                "machine": {
                    "initial": "a",
                    "states": {
                        "a": {"transitions": [{"to": "b", "after": 1}]},
                        "b": {"final": true}
                    }
                },
                "transitions": [{"to": "rest", "on": "success"}]
            },
            "rest": {"update": "decide"},
            "play": {}
        }
    }));
    h.host.function("decide", |call| {
        if call.state_time > 0.5 {
            Value::str("play")
        } else {
            Value::Nil
        }
    });
    h.tick(0.1);
    assert_eq!(h.state(), "work/a");
    h.tick(1.0);
    // b is final: work ends and the parent leaves it.
    assert_eq!(h.state(), "rest");
    h.tick(0.3);
    assert_eq!(h.state(), "rest");
    h.tick(0.3);
    assert_eq!(h.state(), "play");
}

#[test]
fn sends_noises_and_finds() {
    let mut h = Harness::new(json!({"type": "sequence", "children": [
        {"type": "send", "event": "hello", "to": "friend", "data": {"n": 1}},
        {"type": "noise", "tag": "alarm", "radius": 12},
        {"type": "broadcast", "event": "hey", "radius": 5, "tag": "guard"},
        {"type": "find", "tag": "thief", "store": "suspect"}
    ]}));
    let friend = h._world.spawn(());
    let thief = h._world.spawn(());
    h.set("friend", Value::entity(friend));
    assert_eq!(h.tick(0.1), Status::Failure);
    assert_eq!(h.outbox.len(), 3);
    assert!(
        matches!(&h.outbox[0], Outgoing::Event { to: super::context::Recipients::One(e), .. } if *e == friend)
    );
    assert!(matches!(&h.outbox[1], Outgoing::Noise { radius, .. } if *radius == 12.0));

    h.senses.percepts.push(super::perception::Percept {
        entity: thief,
        tags: vec![std::sync::Arc::from("thief")].into(),
        position: Vec3::new(2.0, 0.0, 0.0),
        visible: true,
        last_seen: 0.0,
        last_heard: f64::NEG_INFINITY,
    });
    assert_eq!(h.tick(0.1), Status::Success);
    assert_eq!(h.get("suspect"), Value::entity(thief));
}

#[test]
fn senses_see_within_the_cone_and_report_changes() {
    let mut world = hecs::World::new();
    let me = world.spawn(());
    let ahead = world.spawn(());
    let behind = world.spawn(());
    let near_behind = world.spawn(());
    let mut host = Mock::default();
    let settings = PerceptionSettings {
        sight_range: 10.0,
        sight_angle: 90.0,
        near_range: 1.5,
        ..Default::default()
    };
    let tag =
        |t: &str| -> std::sync::Arc<[std::sync::Arc<str>]> { vec![std::sync::Arc::from(t)].into() };
    let mut candidates = vec![
        Visible {
            entity: ahead,
            position: Vec3::new(0.0, 0.0, 5.0),
            tags: tag("thief"),
            height: 1.8,
        },
        Visible {
            entity: behind,
            position: Vec3::new(0.0, 0.0, -5.0),
            tags: tag("guard"),
            height: 1.8,
        },
        Visible {
            entity: near_behind,
            position: Vec3::new(0.0, 0.0, -1.0),
            tags: tag("child"),
            height: 1.2,
        },
    ];
    let mut senses = Senses::default();
    let mut events = Vec::new();
    let look_at = |senses: &mut Senses,
                   candidates: &[Visible],
                   host: &Mock,
                   now: f64,
                   events: &mut Vec<AiEvent>| {
        let grid = VisibleGrid::new(candidates.to_vec(), 8.0);
        let mut nearby = Vec::new();
        grid.near(Vec3::ZERO, 10.0, &mut nearby);
        look(
            senses,
            &settings,
            me,
            Vec3::ZERO,
            0.0,
            &nearby,
            &|e| grid.contains(e),
            host,
            now,
            events,
        );
    };
    look_at(&mut senses, &candidates, &host, 1.0, &mut events);
    assert!(senses.sees(Some("thief")));
    assert!(!senses.sees(Some("guard")));
    assert!(senses.sees(Some("child")));
    let names: Vec<&str> = events.iter().map(|e| &*e.name).collect();
    assert_eq!(names, vec!["see", "see:thief", "see", "see:child"]);
    assert_eq!(senses.count(None, Some(2.0), Vec3::ZERO), 1);
    assert_eq!(senses.nearest(None, Vec3::ZERO, true), Some(near_behind));

    // A wall between: lost from sight, but remembered for a while.
    host.walls.insert((me, ahead));
    events.clear();
    look_at(&mut senses, &candidates, &host, 2.0, &mut events);
    assert!(!senses.sees(Some("thief")));
    assert!(senses.percept(ahead).is_some());
    assert_eq!(events[0].name.as_ref(), "lose");
    candidates.remove(0);
    look_at(&mut senses, &candidates, &host, 10.0, &mut events);
    assert!(senses.percept(ahead).is_none());
}

/// Brains defined by Lua scripts, run by a real scene.
mod in_scene {
    use glam::Vec3;
    use hecs::Entity;

    use crate::ai::{AiAgent, PerceptionSettings, Value};
    use crate::scene::components::TransformComponent;
    use crate::scene::transform::Transform;
    use crate::scene::{Name, Scene};
    use crate::scripting::LuaScriptComponent;

    fn spawn_agent(scene: &mut Scene, name: &str, at: Vec3, agent: AiAgent) -> Entity {
        scene.main_world_mut().spawn((
            Name::new(name),
            TransformComponent(Transform::from_translation(at)),
            agent,
        ))
    }

    fn director(scene: &mut Scene, source: &str) -> Entity {
        scene.main_world_mut().spawn((
            Name::new("Director"),
            TransformComponent(Transform::default()),
            LuaScriptComponent::new_inline("director", source),
        ))
    }

    fn run(scene: &mut Scene, seconds: f64) {
        let frames = (seconds * 30.0).round() as usize;
        for _ in 0..frames {
            scene.update_with_mode(1.0 / 30.0, false);
        }
    }

    fn get(scene: &Scene, entity: Entity, key: &str) -> Value {
        scene
            .ai()
            .shared()
            .board(entity)
            .map(|b| b.get(key))
            .unwrap_or_default()
    }

    fn state(scene: &Scene, entity: Entity) -> String {
        scene
            .ai()
            .shared()
            .agents
            .borrow()
            .get(&entity)
            .map(|a| a.state_path())
            .unwrap_or_default()
    }

    #[test]
    fn lua_trees_run_with_their_scripts_context() {
        let mut scene = Scene::new();
        director(
            &mut scene,
            r#"
            local ticks = 0
            function on_created(self)
                ai_tree("counter", { "sequence",
                    function(ctx)
                        ticks = ticks + 1
                        ctx.bb.count = (ctx.bb.count or 0) + 1
                        ctx.bb.home = { x = 1, y = 0, z = 2 }
                        ctx.bb.notes = { said = "hi" }
                    end,
                    { "check", "count >= 3 and home.x == 1 and notes.said == 'hi'" },
                    { "set", "done", true },
                })
            end
            function update(self, dt) seen = ticks end
            "#,
        );
        let a = spawn_agent(
            &mut scene,
            "A",
            Vec3::ZERO,
            AiAgent {
                brain: "counter".into(),
                tick_rate: 0.0,
                ..Default::default()
            },
        );
        run(&mut scene, 0.5);
        assert!(get(&scene, a, "count").as_f64().unwrap() >= 3.0);
        assert_eq!(get(&scene, a, "done"), Value::Bool(true));
        assert_eq!(
            get(&scene, a, "home"),
            Value::Vec3(Vec3::new(1.0, 0.0, 2.0))
        );
        let seen = scene
            .eval_lua("seen", Some(scene_director(&scene)))
            .unwrap();
        assert!(seen.values[0].as_i64().unwrap() >= 3);
    }

    fn scene_director(scene: &Scene) -> Entity {
        scene
            .main_world()
            .query::<&Name>()
            .iter()
            .find(|(_, n)| n.0 == "Director")
            .map(|(e, _)| e)
            .unwrap()
    }

    #[test]
    fn agents_talk_with_events_and_hear_noises() {
        let mut scene = Scene::new();
        director(
            &mut scene,
            r#"
            function on_created(self)
                ai_tree("caller", { "sequence",
                    { "send", "hello", to = "friend", data = "hi" },
                    { "noise", "alarm", radius = 8 },
                    { "wait", 100 },
                })
                ai_machine("listener", {
                    initial = "idle",
                    states = {
                        idle = { transitions = {
                            { "greeted", event = "hello", store = "said", store_from = "caller" },
                            { "alarmed", event = "hear:alarm", store_position = "alarm_at" },
                        } },
                        greeted = { transitions = { { "alarmed", event = "hear:alarm" } } },
                        alarmed = {},
                    },
                })
            end
            "#,
        );
        let hearing = |range: f32| AiAgent {
            brain: "listener".into(),
            tick_rate: 0.0,
            perception: PerceptionSettings {
                hearing_range: range,
                ..Default::default()
            },
            ..Default::default()
        };
        let near = spawn_agent(&mut scene, "Near", Vec3::new(3.0, 0.0, 0.0), hearing(20.0));
        let far = spawn_agent(&mut scene, "Far", Vec3::new(30.0, 0.0, 0.0), hearing(50.0));
        let deaf = spawn_agent(&mut scene, "Deaf", Vec3::new(2.0, 0.0, 0.0), hearing(0.0));
        let caller = spawn_agent(
            &mut scene,
            "Caller",
            Vec3::ZERO,
            AiAgent {
                brain: "caller".into(),
                tick_rate: 0.0,
                blackboard: serde_json::json!({ "friend": near.to_bits().get() }),
                ..Default::default()
            },
        );
        run(&mut scene, 0.5);
        assert_eq!(get(&scene, near, "said"), Value::str("hi"));
        assert_eq!(get(&scene, near, "caller"), Value::entity(caller));
        assert_eq!(state(&scene, near), "alarmed");
        // Out of the noise's radius, and deaf.
        assert_eq!(state(&scene, far), "idle");
        assert_eq!(state(&scene, deaf), "idle");
    }

    #[test]
    fn agent_scripts_hear_state_changes_and_errors_are_kept() {
        let mut scene = Scene::new();
        director(
            &mut scene,
            r#"
            function on_created(self)
                ai_machine("stepper", {
                    initial = "a",
                    states = {
                        a = { transitions = { { "b", after = 0.2 } } },
                        b = { tree = { "lua", run = function(ctx) error("boom") end } },
                    },
                })
            end
            "#,
        );
        let agent = scene.main_world_mut().spawn((
            Name::new("Stepper"),
            TransformComponent(Transform::default()),
            AiAgent {
                brain: "stepper".into(),
                tick_rate: 0.0,
                ..Default::default()
            },
            LuaScriptComponent::new_inline(
                "stepper",
                "function on_ai_state(self, change) last_change = change.from .. '>' .. change.to end",
            ),
        ));
        run(&mut scene, 0.6);
        let change = scene.eval_lua("last_change", Some(agent)).unwrap();
        assert_eq!(change.values[0], serde_json::json!("a>b"));
        let error = scene
            .ai()
            .shared()
            .agents
            .borrow()
            .get(&agent)
            .and_then(|a| a.error.clone())
            .unwrap_or_default();
        assert!(error.contains("boom"), "{error}");
    }

    /// Lantern Market's brains (games/lantern-market/content/scripts/market/brains) all
    /// parse, and a few of each run without errors (no navmesh here: walks fail, which
    /// the brains must cope with).
    #[test]
    fn lantern_market_brains_define_and_run() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("games/lantern-market/content/scripts");
        let mut scene = Scene::new();
        scene.scripting_mut().runtime_mut().set_script_root(&root);
        director(
            &mut scene,
            r#"
            function on_created(self)
                for _, name in ipairs({ "townsfolk", "vendor", "guard", "thief", "child", "performer" }) do
                    require("market/brains/" .. name).define()
                end
                require("market/props").define()
                local names = {}
                for _, brain in ipairs(ai_brains()) do
                    table.insert(names, brain.name)
                end
                table.sort(names)
                defined = table.concat(names, ",")
            end
            "#,
        );
        scene.update_with_mode(1.0 / 30.0, false);
        let dir = scene_director(&scene);
        let defined = scene.eval_lua("defined", Some(dir)).unwrap();
        assert_eq!(
            defined.values[0],
            serde_json::json!("cart,child,gate,guard,performer,thief,townsfolk,vendor"),
            "{:?}",
            scene.scripting().last_error(dir)
        );
        let blackboard = scene
            .eval_lua(
                "return require('market/brains/townsfolk').blackboard(function(a, b) return (a + b) / 2 end, 'Ada')",
                Some(dir),
            )
            .unwrap();
        let mut agents = Vec::new();
        for (i, (brain, tags)) in [
            ("townsfolk", "townsfolk"),
            ("townsfolk", "townsfolk"),
            ("vendor", "vendor"),
            ("guard", "guard"),
            ("thief", "thief"),
            ("child", "child"),
            ("performer", "performer"),
        ]
        .into_iter()
        .enumerate()
        {
            let mut agent = AiAgent {
                brain: brain.into(),
                tags: vec![tags.into()],
                tick_rate: 0.0,
                perception: PerceptionSettings {
                    sight_range: 10.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            if brain == "townsfolk" {
                agent.blackboard = blackboard.values[0].clone();
            }
            agents.push(spawn_agent(
                &mut scene,
                &format!("{brain} {i}"),
                Vec3::new(i as f32, 0.0, 0.0),
                agent,
            ));
        }
        run(&mut scene, 3.0);
        let shared = scene.ai().shared().clone();
        let infos = shared.agents.borrow();
        for entity in &agents {
            let info = &infos[entity];
            // Walking fails here (no navmesh); nothing else may.
            if let Some(error) = &info.error {
                assert!(
                    error.contains("NavAgent") || error.contains("Animator"),
                    "{}: {error}",
                    info.brain
                );
            }
            assert!(!info.states.is_empty(), "{} has no state", info.brain);
        }
    }

    /// Lantern Market simulated without rendering anything (no GPU, no meshes): the real
    /// navmesh bake, carts, gate, `market/people.lua` and every brain, with stand-in
    /// characters (entities with an empty animator, so clips "play" without bones).
    /// Reports what happened. CPU only:
    /// `cargo test --release --lib lantern_market_simulation -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn lantern_market_simulation() {
        use crate::scene::skeletal::Animator;
        use std::collections::{BTreeMap, BTreeSet, HashMap};

        let project = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("games/lantern-market");
        crate::project::set_active_project_root(Some(project.clone()));
        let scene_json: serde_json::Value = serde_json::from_slice(
            &std::fs::read(project.join("content/scenes/9FKKrEIPvZz8.json")).unwrap(),
        )
        .unwrap();

        let mut scene = Scene::new();
        scene
            .scripting_mut()
            .runtime_mut()
            .set_script_root(project.join("content/scripts"));
        // The level's navigation: the surface (with its bake), the stage jumps, carts, gate.
        // Obstacles do not carve here: a carved tile is rebuilt from the level's meshes,
        // which this headless scene does not load (they would come back empty).
        for entity in scene_json["entities"].as_array().unwrap() {
            let name = entity["name"].as_str().unwrap_or_default();
            let transform: Transform = serde_json::from_value::<
                crate::scene::assets::SerializedTransform,
            >(entity["transform"].clone())
            .map(Into::into)
            .unwrap_or_default();
            let world = scene.main_world_mut();
            let e = world.spawn((Name::new(name), TransformComponent(transform)));
            let mut any = false;
            for key in ["nav_mesh_surface", "nav_link", "nav_obstacle"] {
                if !entity[key].is_null() {
                    let mut value = entity[key].clone();
                    if key == "nav_obstacle" {
                        value["carve"] = serde_json::json!(false);
                    }
                    crate::navigation::edit::patch_component(world, e, key, &value).unwrap();
                    any = true;
                }
            }
            if !any {
                world.despawn(e).unwrap();
            }
        }
        let director = director(
            &mut scene,
            r#"
            local C = require("market/core")
            local People = require("market/people")
            local Props = require("market/props")
            MARKET = { frame = 0 }
            local CLIPS = { "idle", "happy-idle", "breathing-idle", "bored", "look-around", "thinking",
                "talking", "laughing", "waving", "clapping", "cheering", "pointing", "shrugging", "drinking",
                "sitting-idle", "salute", "samba-dancing", "hip-hop-dancing", "quick-formal-bow", "walking",
                "jogging", "running", "jump" }
            function setup(templates, size)
                if size then
                    People.cast_size = function() return size end
                end
                C.characters = {
                    { id = "folk", name = "Folk", height = 1.8, groups = { "townsfolk", "vendor", "puller" } },
                    { id = "guard", name = "Guard", height = 1.8, groups = { "guard" } },
                    { id = "thief", name = "Thief", height = 1.8, groups = { "thief" } },
                    { id = "kid", name = "Kid", height = 1.3, groups = { "child" } },
                    { id = "star", name = "Star", height = 1.8, groups = { "performer" } },
                }
                C.by_group = {}
                for _, char in ipairs(C.characters) do
                    for _, group in ipairs(char.groups) do
                        C.by_group[group] = C.by_group[group] or {}
                        table.insert(C.by_group[group], char)
                    end
                    C.templates[char.id] = templates[char.id]
                end
                for _, clip in ipairs(CLIPS) do
                    C.clips[clip] = { speed = 0 }
                end
                for _, name in ipairs({ "townsfolk", "vendor", "guard", "thief", "child", "performer" }) do
                    require("market/brains/" .. name).define()
                end
                Props.define()
                People.spawn_all()
                Props.setup(nil)
                started = true
            end
            function update(self, dt)
                MARKET.frame = MARKET.frame + 1
                if started then People.resolve() end
            end
            function start_tag() return require("market/brains/child").start_tag(People.ids("child")) end
            function strike() return People.thief and ai_send(People.thief.e, "strike") end
            function roles()
                local out = {}
                for _, person in ipairs(People.list) do
                    if person.id then out[tostring(person.id)] = person.role end
                end
                return out
            end
            "#,
        );
        // Stand-in characters: templates with an animator (cloned with it).
        let mut templates = serde_json::Map::new();
        for id in ["folk", "guard", "thief", "kid", "star"] {
            let e = scene.main_world_mut().spawn((
                Name::new(format!("Template {id}")),
                TransformComponent(Transform::from_translation(Vec3::new(0.0, -60.0, 0.0))),
                Animator::default(),
            ));
            templates.insert(id.into(), serde_json::json!(e.to_bits().get()));
        }
        let dt = 1.0 / 30.0;
        run(&mut scene, 0.2); // the navmesh loads
                              // MARKET_SIM_SIZE="townsfolk=0,vendors=0,guards=1,children=0,thief=0,performer=0"
                              // runs a smaller cast.
        let size = std::env::var("MARKET_SIM_SIZE")
            .map(|s| format!("{{ {s} }}"))
            .unwrap_or_else(|_| "nil".into());
        scene
            .eval_lua(
                &format!(
                    "setup({}, {size})",
                    lua_table(&serde_json::Value::Object(templates))
                ),
                Some(director),
            )
            .unwrap();
        run(&mut scene, 0.2);
        // Paths across the tile seams where agents were seen stuck, before any carving.
        let probes = [
            (Vec3::new(0.0, 3.8, -13.5), Vec3::new(26.0, 3.8, -13.5)),
            (Vec3::new(14.6, 0.0, -6.0), Vec3::new(3.5, 0.0, 7.1)),
            (Vec3::new(15.4, 0.0, -10.6), Vec3::new(-9.6, 0.0, 2.0)),
        ];
        let probe = |scene: &Scene, when: &str| {
            for (from, to) in probes {
                let found = scene.navigation().find_path(
                    from,
                    to,
                    "",
                    &crate::navigation::QueryFilter::default(),
                );
                println!(
                    "path {when}: {from:.1?} -> {to:.1?}: {:?}",
                    found.map(|(points, complete, length)| (complete, length, points.len()))
                );
            }
        };
        probe(&scene, "at start");
        let roles: HashMap<u64, String> = scene.eval_lua("roles()", Some(director)).unwrap().values
            [0]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.parse().unwrap(), v.as_str().unwrap().to_owned()))
        .collect();
        let full_cast = std::env::var("MARKET_SIM_SIZE").is_err();
        assert!(!full_cast || roles.len() > 80, "spawned {}", roles.len());

        let seconds = 180.0;
        let frames = (seconds / dt) as usize;
        let mut visited: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut entries: BTreeMap<String, usize> = BTreeMap::new();
        let mut last_state: HashMap<u64, String> = HashMap::new();
        let mut errors: BTreeMap<String, usize> = BTreeMap::new();
        let mut still: HashMap<u64, (Vec3, f64)> = HashMap::new();
        let mut stuck: BTreeMap<String, usize> = BTreeMap::new();
        let mut ai_ms = 0.0;
        let mut worst_ms: f64 = 0.0;
        for frame in 0..frames {
            let t = frame as f64 * dt;
            if frame == (10.0 / dt) as usize {
                scene.eval_lua("start_tag()", Some(director)).unwrap();
            }
            if frame % (45.0 / dt) as usize == (20.0 / dt) as usize {
                scene.eval_lua("strike()", Some(director)).unwrap();
            }
            scene.update_with_mode(dt, false);
            let ms = scene.ai().stats().step_ms;
            ai_ms += ms;
            worst_ms = worst_ms.max(ms);
            if frame % 5 != 0 {
                continue;
            }
            let shared = scene.ai().shared().clone();
            let agents = shared.agents.borrow();
            for (entity, info) in agents.iter() {
                let id = entity.to_bits().get();
                let Some(role) = roles.get(&id) else { continue };
                let state = info.state_path();
                visited
                    .entry(role.clone())
                    .or_default()
                    .insert(state.clone());
                if last_state.get(&id) != Some(&state) {
                    *entries.entry(format!("{role}: {state}")).or_default() += 1;
                    last_state.insert(id, state.clone());
                }
                if let Some(error) = &info.error {
                    *errors.entry(format!("{role}: {error}")).or_default() += 1;
                }
                // Walking (a move leaf) but not getting anywhere for 30 s.
                let here = crate::physics::world_transform(scene.main_world(), *entity).translation;
                let walking = info.action.as_deref().is_some_and(|a| {
                    a.starts_with("move to")
                        || a.starts_with("walk")
                        || a.contains("run")
                        || a.starts_with("wander")
                        || a.starts_with("go to")
                });
                let entry = still.entry(id).or_insert((here, t));
                if !walking || here.distance(entry.0) > 0.5 {
                    *entry = (here, t);
                } else if t - entry.1 > 30.0 {
                    *stuck
                        .entry(format!(
                            "{role}: {state} / {}",
                            info.action.clone().unwrap_or_default()
                        ))
                        .or_default() += 1;
                    if let Some(nav) = scene.navigation().agent_status(*entity) {
                        println!(
                            "stuck at {t:.0} s: {role} {state} at {:.1?} -> {:.1?} {:?} partial {} remaining {:?} speed {:.2} desired {:.2} max speed {:?} corners {:.1?}",
                            here, nav.target, nav.state, nav.partial, nav.remaining_distance, nav.velocity.length(),
                            nav.desired_velocity.length(),
                            scene.main_world().get::<&crate::navigation::NavAgent>(*entity).map(|a| a.speed).ok(),
                            nav.corners
                        );
                        if let Some(target) = nav.target {
                            let path = scene.navigation().find_path(
                                here,
                                target,
                                "",
                                &crate::navigation::QueryFilter::default(),
                            );
                            let toward = here + (target - here).normalize_or_zero() * 1.5;
                            let ray = scene.navigation().raycast(here, toward, "");
                            let nearest = scene.navigation().closest_point(
                                here,
                                Vec3::new(0.5, 2.0, 0.5),
                                "",
                            );
                            println!(
                                "    from here: path {:?}, ray {:.2?}, nearest navmesh point {:.2?}",
                                path.map(|p| (p.1, p.0.len())),
                                ray,
                                nearest
                            );
                        }
                        for (other, other_info) in agents.iter() {
                            let there = crate::physics::world_transform(scene.main_world(), *other)
                                .translation;
                            if *other != *entity && there.distance(here) < 2.0 {
                                let other_nav = scene.navigation().agent_status(*other);
                                println!(
                                    "    near: {} {} at {:.1?} dist {:.2} {:?} speed {:.2} doing {:?}",
                                    roles.get(&other.to_bits().get()).cloned().unwrap_or_default(),
                                    other_info.state_path(),
                                    there,
                                    there.distance(here),
                                    other_nav.as_ref().map(|n| n.state),
                                    other_nav.as_ref().map_or(0.0, |n| n.velocity.length()),
                                    other_info.action
                                );
                            }
                        }
                    }
                    *entry = (here, t);
                }
            }
        }
        probe(&scene, "at the end");
        let global = scene.ai().shared().global.lock().to_json();
        let kids = scene.ai().shared().group_board("kids").lock().to_json();
        // The thief's and a guard's minds, for diagnosing.
        for role in ["thief", "guard"] {
            for (&id, _) in roles.iter().filter(|(_, r)| r.as_str() == role) {
                let entity = hecs::Entity::from_bits(id).unwrap();
                if let Some(mind) = scene.ai().inspect(scene.main_world(), entity) {
                    let machine = &mind["runtime"]["machine"];
                    println!(
                        "
{role}: state {} action {:?} pos {:?}",
                        mind["state"],
                        mind["action"],
                        crate::physics::world_transform(scene.main_world(), entity).translation
                    );
                    println!("  history: {}", machine["history"]);
                    println!("  blackboard: {}", mind["blackboard"]);
                    println!("  nav: {:?}", scene.navigation().agent_status(entity));
                    println!(
                        "  senses: {}",
                        mind["senses"]["percepts"].as_array().map_or(0, Vec::len)
                    );
                    if let Some(tree) = machine.get("tree") {
                        for node in tree["nodes"].as_array().unwrap() {
                            println!(
                                "    {}{} {} {}",
                                "  ".repeat(node["depth"].as_u64().unwrap() as usize),
                                node["label"],
                                node["last"],
                                node.get("detail")
                                    .map(|d| d.to_string())
                                    .unwrap_or_default()
                            );
                        }
                    }
                }
            }
        }
        crate::project::set_active_project_root(None);

        println!(
            "\n{} agents over {seconds} s; AI step {:.3} ms average, {:.3} ms worst",
            roles.len(),
            ai_ms / frames as f64,
            worst_ms
        );
        println!("global: {global}\nkids: {kids}");
        println!("\nstates entered:");
        for (state, n) in &entries {
            println!("  {n:>5}  {state}");
        }
        println!("\nerrors:");
        for (error, n) in &errors {
            println!("  {n:>5}  {error}");
        }
        println!("\nstuck walking > 30 s:");
        for (what, n) in &stuck {
            println!("  {n:>5}  {what}");
        }
        let entered = |key: &str| entries.get(key).copied().unwrap_or(0);
        if !full_cast {
            return;
        }
        assert!(entered("townsfolk: chat") > 0, "nobody chatted");
        assert!(global["thefts"].as_i64().unwrap_or(0) > 0, "no theft");
        assert!(global["shows"].as_i64().unwrap_or(0) > 0, "no show");
        assert!(
            kids["tags"].as_i64().unwrap_or(0) > 0 || entered("child: it") > 1,
            "no game of tag"
        );
        assert!(entered("vendor: serve") > 0, "no vendor served anyone");
    }

    /// A JSON object as a Lua table literal (string keys, number values).
    fn lua_table(value: &serde_json::Value) -> String {
        let fields: Vec<String> = value
            .as_object()
            .unwrap()
            .iter()
            .map(|(k, v)| format!("[{k:?}] = {v}"))
            .collect();
        format!("{{ {} }}", fields.join(", "))
    }

    /// The AI's cost for crowds, measured without rendering anything (no GPU, no skinned
    /// meshes): a Lua-defined machine with a utility tree, timers, Lua leaves, senses and
    /// events, at 10 thinks and 2 looks a second. Run it in release:
    /// `cargo test --release --lib ai_crowd_benchmark -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn ai_crowd_benchmark() {
        for count in [500usize, 2000, 10000] {
            let mut scene = Scene::new();
            director(
                &mut scene,
                r#"
                function on_created(self)
                    ai_machine("busy", {
                        initial = "work",
                        states = {
                            work = {
                                update = function(ctx) ctx.bb.energy = (ctx.bb.energy or 1) - ctx.dt * 0.05 end,
                                tree = { "utility", interval = 2,
                                    { "sequence", score = "energy", { "add", "done", 1 }, { "wait", { 0.5, 1.5 } } },
                                    { "sequence", score = "1 - energy", { "set", "energy", 1 }, { "wait", 1 } },
                                    { "sequence", score = "count('worker', 6) * 0.1",
                                        function(ctx) ctx.bb.met = (ctx.bb.met or 0) + 1 end,
                                        { "send", "hello", to = "nearest('worker')", when = "exists(nearest('worker'))" },
                                        { "wait", 2 } },
                                },
                                transitions = { { "greet", event = "hello", chance = 0.5 } },
                            },
                            greet = { tree = { "wait", 1 }, transitions = { { "work", on = "done" } } },
                        },
                    })
                end
                "#,
            );
            let side = (count as f32).sqrt().ceil() as usize;
            for i in 0..count {
                let at = Vec3::new((i % side) as f32 * 2.5, 0.0, (i / side) as f32 * 2.5);
                spawn_agent(
                    &mut scene,
                    &format!("Worker {i}"),
                    at,
                    AiAgent {
                        brain: "busy".into(),
                        tags: vec!["worker".into()],
                        perception: PerceptionSettings {
                            sight_range: 10.0,
                            rate: 2.0,
                            line_of_sight: crate::ai::LineOfSight::None,
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                );
            }
            let dt = 1.0 / 60.0;
            for _ in 0..60 {
                scene.update_with_mode(dt, false);
            }
            let mut total = 0.0;
            let mut worst: f64 = 0.0;
            let frames = 240;
            for _ in 0..frames {
                scene.update_with_mode(dt, false);
                let ms = scene.ai().stats().step_ms;
                total += ms;
                worst = worst.max(ms);
            }
            let stats = scene.ai().stats();
            println!(
                "{count:>6} agents: AI step {:.3} ms average, {:.3} ms worst ({} thought, {} looked, {} events in the last frame)",
                total / frames as f64,
                worst,
                stats.ticked,
                stats.looked,
                stats.events
            );
            assert_eq!(stats.agents, count);
        }
    }

    #[test]
    fn scripts_give_brains_and_read_state() {
        let mut scene = Scene::new();
        let dir = director(
            &mut scene,
            r#"
            function on_created(self)
                ai_tree("idler", { "wait", 100, name = "idling" })
                guy = spawn_entity("Guy")
                ai_set_brain(guy, "idler", { tags = { "townsfolk" }, tick_rate = 0, blackboard = { mood = "calm" } })
            end
            function update(self, dt)
                if entity_exists(guy) then
                    action = ai_action(guy)
                    mood = ai_get(guy, "mood")
                    ai_set(guy, "global.night", true)
                    agents = #ai_agents({ tag = "townsfolk" })
                end
            end
            "#,
        );
        run(&mut scene, 0.3);
        let out = scene
            .eval_lua("return action, mood, agents, ai_global().night", Some(dir))
            .unwrap();
        assert_eq!(
            out.values,
            vec![
                serde_json::json!("idling"),
                serde_json::json!("calm"),
                serde_json::json!(1),
                serde_json::json!(true)
            ]
        );
    }
}
