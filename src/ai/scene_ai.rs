//! The AI of one scene: every `AiAgent`'s brain, blackboards, senses and mail.
//!
//! Like the navigation and physics it is derived state, rebuilt from components when
//! play starts. Each play-mode frame, after the scripts' `update` (so their blackboard
//! writes and events count this frame) and before the navigation (so walks start this
//! frame), [`SceneAi::step`]:
//!
//! 1. takes in new, changed and removed agents and finds their brains (defined by
//!    scripts, or JSON files loaded and reloaded when they change);
//! 2. applies what scripts asked (`ai_noise`, `ai_broadcast`, `ai_restart`);
//! 3. lets agents look around (at their perception rate) and turns what changed into
//!    events;
//! 4. ticks the agents that are due (at their tick rate, spread over frames);
//! 5. delivers what they sent, for their next tick.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

use glam::Vec3;
use hecs::{Entity, World};
use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};
use serde::{Deserialize, Serialize};

use super::agent::AiAgent;
use super::blackboard::{Blackboard, BlackboardRef};
use super::brain::BrainRuntime;
use super::context::{AiEvent, AiHost, Boards, Cx, Outgoing, Recipients};
use super::def::{parse_brain, Brain, Def, Library, Status};
use super::host::SceneHost;
use super::perception::{look, HeardNoise, Senses, Visible, VisibleGrid};
use super::value::{EntityMap, EntitySet, Value};
use crate::scripting::ScriptingState;

/// Events kept for an agent between its ticks at most (older ones are dropped).
const INBOX_LIMIT: usize = 64;

/// What the AI draws over the scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AiDebugDraw {
    /// View cones and hearing ranges.
    pub senses: bool,
    /// Lines to where agents are walking.
    pub goals: bool,
    /// Lines to what they see (green) and remember (grey).
    pub percepts: bool,
    /// A ring at each agent's feet, coloured by its state.
    pub states: bool,
}

impl AiDebugDraw {
    pub fn all() -> Self {
        Self {
            senses: true,
            goals: true,
            percepts: true,
            states: true,
        }
    }

    pub fn any(&self) -> bool {
        self.senses || self.goals || self.percepts || self.states
    }
}

/// The debug view is shared by every scene (the editor replaces the scene on play).
pub(crate) fn view_state() -> &'static Mutex<AiDebugDraw> {
    static VIEW: OnceLock<Mutex<AiDebugDraw>> = OnceLock::new();
    VIEW.get_or_init(|| Mutex::new(AiDebugDraw::default()))
}

/// What scripts and tools see of an agent between ticks.
#[derive(Debug, Clone)]
pub struct AgentShared {
    pub board: BlackboardRef,
    pub group: String,
    pub tags: Arc<[Arc<str>]>,
    pub brain: String,
    /// Events waiting for its next tick.
    pub inbox: Vec<AiEvent>,
    /// The active states (outermost first) after its last tick.
    pub states: Vec<Arc<str>>,
    /// The leaf its tree is running.
    pub action: Option<String>,
    pub status: Option<Status>,
    /// The last problem: a missing brain, a Lua error.
    pub error: Option<String>,
    pub paused: bool,
    pub senses: Arc<Senses>,
}

impl AgentShared {
    pub fn state_path(&self) -> String {
        self.states
            .iter()
            .map(|s| &**s)
            .collect::<Vec<_>>()
            .join("/")
    }
}

/// A request from a script, applied at the next step.
#[derive(Debug, Clone)]
pub enum AiCommand {
    Restart(Entity),
    Send {
        to: Recipients,
        event: AiEvent,
        group: Option<String>,
    },
    Noise {
        tag: Arc<str>,
        position: Vec3,
        radius: f32,
        data: Value,
        from: Option<Entity>,
    },
}

/// The parts of the AI scripts reach (through `ai_*` functions) while they run.
#[derive(Debug, Default)]
pub struct AiShared {
    /// The AI clock (seconds since play started).
    pub time: std::cell::Cell<f64>,
    pub library: RefCell<Library>,
    pub agents: RefCell<EntityMap<AgentShared>>,
    pub groups: RefCell<HashMap<String, BlackboardRef>>,
    pub global: BlackboardRef,
    pub commands: RefCell<Vec<AiCommand>>,
}

impl AiShared {
    pub fn group_board(&self, name: &str) -> BlackboardRef {
        self.groups
            .borrow_mut()
            .entry(name.to_owned())
            .or_default()
            .clone()
    }

    pub fn board(&self, entity: Entity) -> Option<BlackboardRef> {
        self.agents.borrow().get(&entity).map(|a| a.board.clone())
    }

    /// Put an event in an agent's inbox; false when it is not an agent.
    pub fn send(&self, entity: Entity, event: AiEvent) -> bool {
        let mut agents = self.agents.borrow_mut();
        let Some(agent) = agents.get_mut(&entity) else {
            return false;
        };
        push_event(&mut agent.inbox, event);
        true
    }

    pub fn queue(&self, command: AiCommand) {
        self.commands.borrow_mut().push(command);
    }
}

fn push_event(inbox: &mut Vec<AiEvent>, event: AiEvent) {
    if inbox.len() >= INBOX_LIMIT {
        inbox.remove(0);
    }
    inbox.push(event);
}

struct AgentRuntime {
    component: AiAgent,
    brain: Option<BrainRuntime>,
    /// The library revision of the brain it runs.
    revision: u64,
    rng: SmallRng,
    senses: Arc<Senses>,
    next_tick: f64,
    last_tick: f64,
    next_look: f64,
    /// When it joined (brains missing for a moment after that are not reported).
    joined: f64,
}

/// What the scene's frame needs from the AI step.
#[derive(Default)]
pub struct AiStepOutput {
    /// Walks, stops and turns for the navigation.
    #[cfg(feature = "navigation")]
    pub nav_commands: Vec<crate::navigation::NavCommand>,
    /// `on_ai_state(self, {from, to})` for agents with scripts.
    pub callbacks: Vec<(Entity, &'static str, serde_json::Value)>,
}

/// What a step works with.
pub struct AiEnv<'a> {
    pub world: &'a mut World,
    pub scripting: &'a mut ScriptingState,
    #[cfg(feature = "navigation")]
    pub navigation: &'a crate::navigation::SceneNavigation,
    pub physics: &'a crate::physics::ScenePhysics,
}

/// Per-step counts, for status and profiling.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct AiStats {
    pub agents: usize,
    pub ticked: usize,
    pub looked: usize,
    pub events: usize,
    pub step_ms: f64,
}

pub struct SceneAi {
    shared: Rc<AiShared>,
    agents: EntityMap<AgentRuntime>,
    /// The agents in a stable order (by entity id).
    order: Vec<Entity>,
    time: f64,
    frame: u64,
    warned: HashSet<String>,
    files_checked: f64,
    /// Brains that could not be found, and when to look again.
    retry_at: HashMap<String, f64>,
    stats: AiStats,
}

impl Default for SceneAi {
    fn default() -> Self {
        Self::new()
    }
}

/// A host whose fields borrow the step's environment.
macro_rules! host {
    ($env:expr, $nav:expr, $library:expr, $warned:expr, $errors:expr) => {
        SceneHost {
            world: &mut *$env.world,
            scripting: &mut *$env.scripting,
            #[cfg(feature = "navigation")]
            navigation: $env.navigation,
            #[cfg(feature = "navigation")]
            nav_commands: $nav,
            physics: $env.physics,
            library: $library,
            warned: $warned,
            errors: $errors,
        }
    };
}

impl SceneAi {
    pub fn new() -> Self {
        Self {
            shared: Rc::new(AiShared::default()),
            agents: EntityMap::default(),
            order: Vec::new(),
            time: 0.0,
            frame: 0,
            warned: HashSet::new(),
            files_checked: 0.0,
            retry_at: HashMap::new(),
            stats: AiStats::default(),
        }
    }

    /// What scripts reach while they run (`ai_*` functions).
    pub fn shared(&self) -> &Rc<AiShared> {
        &self.shared
    }

    /// Forget agents, brains and boards (play starts or stops).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    pub fn time(&self) -> f64 {
        self.time
    }

    pub fn stats(&self) -> AiStats {
        self.stats
    }

    pub fn agent_count(&self) -> usize {
        self.agents.len()
    }

    pub fn debug_draw(&self) -> AiDebugDraw {
        view_state().lock().map(|v| *v).unwrap_or_default()
    }

    pub fn set_debug_draw(&mut self, debug: AiDebugDraw) {
        if let Ok(mut view) = view_state().lock() {
            *view = debug;
        }
    }

    /// Define (or redefine) a brain; agents running it start it again.
    pub fn define(&mut self, name: &str, brain: Brain) {
        self.shared.library.borrow_mut().define(name, brain);
    }

    /// One frame of AI.
    pub fn step(&mut self, env: AiEnv<'_>, dt: f32) -> AiStepOutput {
        let started = crate::time::Instant::now();
        self.time += dt.max(0.0) as f64;
        self.frame += 1;
        self.shared.time.set(self.time);
        let mut output = AiStepOutput::default();
        #[cfg(feature = "navigation")]
        let nav = &mut output.nav_commands;
        let mut errors: EntityMap<String> = EntityMap::default();
        let shared = self.shared.clone();

        if self.time - self.files_checked >= 1.0 {
            self.files_checked = self.time;
            self.reload_changed_files();
        }

        // 1. Agents in and out (only new and changed components are copied).
        let mut present = EntitySet::default();
        let mut changed: Vec<(Entity, AiAgent)> = Vec::new();
        for (entity, component) in env.world.query::<&AiAgent>().iter() {
            present.insert(entity);
            if self
                .agents
                .get(&entity)
                .is_none_or(|agent| agent.component != *component)
            {
                changed.push((entity, component.clone()));
            }
        }
        let gone: Vec<Entity> = self
            .agents
            .keys()
            .filter(|e| !present.contains(*e))
            .copied()
            .collect();
        for entity in gone {
            if let Some(mut agent) = self.agents.remove(&entity) {
                // Despawned agents just go; a removed component stops what it did.
                if env.world.contains(entity) {
                    let mut host = host!(env, nav, &shared.library, &mut self.warned, &mut errors);
                    halt(
                        &mut agent, entity, &shared, &mut host, self.time, self.frame,
                    );
                }
            }
            shared.agents.borrow_mut().remove(&entity);
        }
        let mut membership_changed = self.order.len() != self.agents.len();
        for (entity, component) in changed {
            match self.agents.get_mut(&entity) {
                Some(agent) => {
                    let restart = agent.component.brain != component.brain
                        || agent.component.enabled != component.enabled;
                    if agent.component.blackboard != component.blackboard {
                        if let Some(board) = shared.board(entity) {
                            board.lock().merge_json(&component.blackboard);
                        }
                    }
                    if let Some(info) = shared.agents.borrow_mut().get_mut(&entity) {
                        info.tags = tags_of(&component);
                        info.group = component.group.clone();
                        info.brain = component.brain.clone();
                    }
                    agent.component = component;
                    if restart {
                        let mut host =
                            host!(env, nav, &shared.library, &mut self.warned, &mut errors);
                        halt(agent, entity, &shared, &mut host, self.time, self.frame);
                    }
                }
                None => {
                    membership_changed = true;
                    let mut board = Blackboard::default();
                    board.merge_json(&component.blackboard);
                    let seed = entity.to_bits().get() ^ 0x9e37_79b9_7f4a_7c15;
                    let mut rng = SmallRng::seed_from_u64(seed);
                    let interval = tick_interval(&component);
                    let look_interval = 1.0 / component.perception.rate.max(0.1) as f64;
                    let next_tick = self.time + rng.gen::<f64>() * interval;
                    let next_look = self.time + rng.gen::<f64>() * look_interval;
                    shared.agents.borrow_mut().insert(
                        entity,
                        AgentShared {
                            board: BlackboardRef::new(board),
                            group: component.group.clone(),
                            tags: tags_of(&component),
                            brain: component.brain.clone(),
                            inbox: Vec::new(),
                            states: Vec::new(),
                            action: None,
                            status: None,
                            error: None,
                            paused: false,
                            senses: Arc::new(Senses::default()),
                        },
                    );
                    self.agents.insert(
                        entity,
                        AgentRuntime {
                            component,
                            brain: None,
                            revision: 0,
                            rng,
                            senses: Arc::new(Senses::default()),
                            next_tick,
                            last_tick: self.time,
                            next_look,
                            joined: self.time,
                        },
                    );
                }
            }
        }
        if membership_changed || self.order.len() != self.agents.len() {
            self.order = self.agents.keys().copied().collect();
            self.order.sort_unstable_by_key(|e| e.to_bits());
        }

        // Brains: find missing ones, restart redefined ones.
        for &entity in &self.order {
            let agent = self.agents.get_mut(&entity).expect("ordered agents exist");
            let name = agent.component.brain.clone();
            if name.is_empty() || !agent.component.enabled {
                continue;
            }
            let revision = shared.library.borrow().revision_of(&name);
            if agent.brain.is_some() && revision != agent.revision {
                let mut host = host!(env, nav, &shared.library, &mut self.warned, &mut errors);
                halt(agent, entity, &shared, &mut host, self.time, self.frame);
            }
            if agent.brain.is_none() && self.retry_at.get(&name).is_none_or(|t| self.time >= *t) {
                match resolve_brain(&shared.library, &name) {
                    Ok(brain) => {
                        agent.brain = Some(BrainRuntime::new(&brain));
                        agent.revision = shared.library.borrow().revision_of(&name);
                        if let Some(info) = shared.agents.borrow_mut().get_mut(&entity) {
                            info.error = None;
                        }
                    }
                    Err(err) => {
                        // Files are tried again now and then, not every frame.
                        if name.ends_with(".json") {
                            self.retry_at.insert(name.clone(), self.time + 2.0);
                        }
                        // Scripts may define it a moment after the agent appears.
                        if self.time - agent.joined > 1.0 {
                            if self.warned.insert(err.clone()) {
                                log::warn!(target: "ai", "{err}");
                            }
                            errors.insert(entity, err);
                        }
                    }
                }
            }
        }

        // 2. Script requests.
        let commands = std::mem::take(&mut *shared.commands.borrow_mut());
        let mut outgoing: Vec<(Option<Entity>, Outgoing, Option<String>)> = Vec::new();
        for command in commands {
            match command {
                AiCommand::Restart(entity) => {
                    if let Some(agent) = self.agents.get_mut(&entity) {
                        let mut host =
                            host!(env, nav, &shared.library, &mut self.warned, &mut errors);
                        halt(agent, entity, &shared, &mut host, self.time, self.frame);
                    }
                }
                AiCommand::Send { to, event, group } => {
                    outgoing.push((event.from, Outgoing::Event { to, event }, group));
                }
                AiCommand::Noise {
                    tag,
                    position,
                    radius,
                    data,
                    from,
                } => outgoing.push((
                    from,
                    Outgoing::Noise {
                        tag,
                        position,
                        radius,
                        data,
                    },
                    None,
                )),
            }
        }
        self.deliver(&mut outgoing, &env);

        // Where everyone is (and what can be seen of them).
        let positions: EntityMap<Vec3> = self
            .order
            .iter()
            .filter(|e| env.world.contains(**e))
            .map(|&e| (e, crate::physics::world_transform(env.world, e).translation))
            .collect();

        // 3. Senses.
        let needs_eyes = self
            .agents
            .values()
            .any(|a| a.component.enabled && a.component.perception.sees_anything());
        let visible: Vec<Visible> = if needs_eyes {
            let agents = shared.agents.borrow();
            self.order
                .iter()
                .filter_map(|e| {
                    let info = agents.get(e)?;
                    if info.tags.is_empty() {
                        return None;
                    }
                    Some(Visible {
                        entity: *e,
                        position: *positions.get(e)?,
                        tags: info.tags.clone(),
                        height: agent_height(env.world, *e),
                    })
                })
                .collect()
        } else {
            Vec::new()
        };
        let grid = VisibleGrid::new(visible, 8.0);
        let mut nearby = Vec::new();
        let mut looked = 0;
        for &entity in &self.order {
            let agent = self.agents.get_mut(&entity).expect("ordered agents exist");
            let settings = &agent.component.perception;
            if !agent.component.enabled || !settings.sees_anything() || self.time < agent.next_look
            {
                continue;
            }
            agent.next_look = self.time + 1.0 / settings.rate.max(0.1) as f64;
            let Some(&feet) = positions.get(&entity) else {
                continue;
            };
            let host = host!(env, nav, &shared.library, &mut self.warned, &mut errors);
            let yaw = host.yaw(entity).unwrap_or(0.0);
            let mut events = Vec::new();
            grid.near(
                feet,
                settings.sight_range.max(settings.near_range),
                &mut nearby,
            );
            look(
                Arc::make_mut(&mut agent.senses),
                settings,
                entity,
                feet,
                yaw,
                &nearby,
                &|e| grid.contains(e),
                &host,
                self.time,
                &mut events,
            );
            looked += 1;
            if let Some(info) = shared.agents.borrow_mut().get_mut(&entity) {
                info.senses = agent.senses.clone();
                for event in events {
                    push_event(&mut info.inbox, event);
                }
            }
        }

        // 4. Think.
        let mut ticked = 0;
        for &entity in &self.order {
            let agent = self.agents.get_mut(&entity).expect("ordered agents exist");
            if !agent.component.enabled || agent.brain.is_none() || !env.world.contains(entity) {
                continue;
            }
            let (paused, events) = {
                let mut agents = shared.agents.borrow_mut();
                let Some(info) = agents.get_mut(&entity) else {
                    continue;
                };
                if info.paused {
                    info.inbox.clear();
                    (true, Vec::new())
                } else if self.time < agent.next_tick {
                    (false, Vec::new())
                } else {
                    (false, std::mem::take(&mut info.inbox))
                }
            };
            if paused || self.time < agent.next_tick {
                continue;
            }
            let interval = tick_interval(&agent.component);
            agent.next_tick = if interval > 0.0 {
                (agent.next_tick + interval).max(self.time + interval * 0.5)
            } else {
                self.time
            };
            let agent_dt = (self.time - agent.last_tick).clamp(0.0, 0.5) as f32;
            agent.last_tick = self.time;
            let Some(boards) = boards_for(&shared, entity) else {
                continue;
            };
            let before = shared
                .agents
                .borrow()
                .get(&entity)
                .map(|a| a.states.clone())
                .unwrap_or_default();
            let mut outbox = Vec::new();
            let mut host = host!(env, nav, &shared.library, &mut self.warned, &mut errors);
            let senses = agent.senses.clone();
            let run_speed = agent.component.run_speed;
            let brain = agent.brain.as_mut().expect("checked above");
            let mut cx = Cx {
                host: &mut host,
                library: &shared.library,
                agent: entity,
                boards: &boards,
                time: self.time,
                dt: agent_dt,
                frame: self.frame,
                events: &events,
                senses: &senses,
                rng: &mut agent.rng,
                outbox: &mut outbox,
                states: Vec::new(),
                state_entered: self.time,
                event: None,
                run_speed,
                it: None,
            };
            let status = brain.tick(&mut cx);
            drop(cx);
            ticked += 1;
            let states = brain.state_path();
            let action = brain.current_action();
            if let Some(info) = shared.agents.borrow_mut().get_mut(&entity) {
                info.status = Some(status);
                info.action = action;
                if info.states != states {
                    info.states = states.clone();
                }
            }
            if before != states && has_script(env.world, entity) {
                let join = |s: &[Arc<str>]| s.iter().map(|s| &**s).collect::<Vec<_>>().join("/");
                output.callbacks.push((
                    entity,
                    "on_ai_state",
                    serde_json::json!({ "from": join(&before), "to": join(&states) }),
                ));
            }
            for message in outbox {
                outgoing.push((Some(entity), message, None));
            }
        }

        // 5. Mail.
        let sent = outgoing.len();
        self.deliver(&mut outgoing, &env);

        if !errors.is_empty() {
            let mut agents = shared.agents.borrow_mut();
            for (entity, error) in errors {
                if let Some(info) = agents.get_mut(&entity) {
                    info.error = Some(error);
                }
            }
        }

        self.stats = AiStats {
            agents: self.agents.len(),
            ticked,
            looked,
            events: sent,
            step_ms: started.elapsed().as_secs_f64() * 1000.0,
        };
        output
    }

    /// Hand out events and noises.
    fn deliver(
        &mut self,
        outgoing: &mut Vec<(Option<Entity>, Outgoing, Option<String>)>,
        env: &AiEnv<'_>,
    ) {
        if outgoing.is_empty() {
            return;
        }
        let shared = self.shared.clone();
        let position = |e: Entity| {
            env.world
                .contains(e)
                .then(|| crate::physics::world_transform(env.world, e).translation)
        };
        for (sender, message, group_override) in outgoing.drain(..) {
            match message {
                Outgoing::Event { to, event } => {
                    let mut agents = shared.agents.borrow_mut();
                    let sender_group = group_override
                        .or_else(|| sender.and_then(|s| agents.get(&s).map(|a| a.group.clone())));
                    let recipients: Vec<Entity> = match &to {
                        Recipients::One(e) => vec![*e],
                        Recipients::Group => match &sender_group {
                            Some(group) if !group.is_empty() => agents
                                .iter()
                                .filter(|(e, a)| a.group == *group && Some(**e) != sender)
                                .map(|(e, _)| *e)
                                .collect(),
                            _ => Vec::new(),
                        },
                        Recipients::Near {
                            center,
                            radius,
                            tag,
                        } => agents
                            .iter()
                            .filter(|(e, a)| {
                                Some(**e) != sender
                                    && tag
                                        .as_ref()
                                        .is_none_or(|t| a.tags.iter().any(|x| **x == **t))
                                    && position(**e).is_some_and(|p| p.distance(*center) <= *radius)
                            })
                            .map(|(e, _)| *e)
                            .collect(),
                        Recipients::All => agents
                            .keys()
                            .filter(|e| Some(**e) != sender)
                            .copied()
                            .collect(),
                    };
                    for recipient in recipients {
                        if let Some(agent) = agents.get_mut(&recipient) {
                            push_event(&mut agent.inbox, event.clone());
                        }
                    }
                }
                Outgoing::Noise {
                    tag,
                    position: origin,
                    radius,
                    data,
                } => {
                    for (&entity, agent) in self.agents.iter_mut() {
                        if Some(entity) == sender || !agent.component.enabled {
                            continue;
                        }
                        let hearing = agent.component.perception.hearing_range;
                        if hearing <= 0.0 {
                            continue;
                        }
                        let Some(here) = position(entity) else {
                            continue;
                        };
                        let distance = here.distance(origin);
                        if distance > radius || distance > hearing {
                            continue;
                        }
                        let mut events = Vec::new();
                        let senses = Arc::make_mut(&mut agent.senses);
                        senses.memory = agent.component.perception.memory;
                        senses.hear(
                            HeardNoise {
                                tag: tag.clone(),
                                position: origin,
                                time: self.time,
                                from: sender,
                                data: data.clone(),
                            },
                            &mut events,
                        );
                        if let Some(info) = shared.agents.borrow_mut().get_mut(&entity) {
                            info.senses = agent.senses.clone();
                            for event in events {
                                push_event(&mut info.inbox, event);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Reload JSON brains whose files changed (native builds; agents restart them).
    fn reload_changed_files(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let paths: Vec<String> = {
                let library = self.shared.library.borrow();
                library
                    .names()
                    .into_iter()
                    .map(|(name, _)| name)
                    .filter(|name| library.file_stamp(name).is_some())
                    .collect()
            };
            for path in paths {
                let stamp = file_stamp(&path);
                let known = self.shared.library.borrow().file_stamp(&path).flatten();
                if stamp.is_some() && stamp != known {
                    match load_brain_file(&path) {
                        Ok(brain) => {
                            let mut library = self.shared.library.borrow_mut();
                            library.define(&path, brain);
                            library.set_file_stamp(&path, stamp);
                            log::info!(target: "ai", "Reloaded brain {path}");
                        }
                        Err(err) => {
                            self.shared
                                .library
                                .borrow_mut()
                                .set_file_stamp(&path, stamp);
                            log::warn!(target: "ai", "{err}");
                        }
                    }
                }
            }
        }
    }

    /// Every agent's brain, state and action, and the brains defined.
    pub fn status_json(&self, world: &World) -> serde_json::Value {
        let agents = self.shared.agents.borrow();
        let mut by_state: std::collections::BTreeMap<String, usize> = Default::default();
        let mut list = Vec::new();
        for entity in &self.order {
            let Some(info) = agents.get(entity) else {
                continue;
            };
            let state = info.state_path();
            *by_state
                .entry(if state.is_empty() {
                    info.brain.clone()
                } else {
                    format!("{}: {state}", info.brain)
                })
                .or_default() += 1;
            list.push(serde_json::json!({
                "entity": entity.to_bits().get(),
                "name": world.get::<&crate::scene::Name>(*entity).map(|n| n.0.clone()).ok(),
                "brain": info.brain,
                "state": state,
                "action": info.action,
                "tags": info.tags.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
                "group": info.group,
                "error": info.error,
                "paused": info.paused,
            }));
        }
        let library = self.shared.library.borrow();
        serde_json::json!({
            "time": self.time,
            "stats": self.stats,
            "debug": self.debug_draw(),
            "brains": library.names().into_iter().map(|(name, kind)| serde_json::json!({"name": name, "kind": kind})).collect::<Vec<_>>(),
            "states": by_state,
            "agents": list,
        })
    }

    /// One agent in depth: its blackboard, brain (tree nodes or machine states and
    /// transitions), senses and pending events.
    pub fn inspect(&self, world: &World, entity: Entity) -> Option<serde_json::Value> {
        let agents = self.shared.agents.borrow();
        let info = agents.get(&entity)?;
        let runtime = self.agents.get(&entity)?;
        let group_board = (!info.group.is_empty()).then(|| self.shared.group_board(&info.group));
        Some(serde_json::json!({
            "entity": entity.to_bits().get(),
            "name": world.get::<&crate::scene::Name>(entity).map(|n| n.0.clone()).ok(),
            "brain": info.brain,
            "enabled": runtime.component.enabled,
            "paused": info.paused,
            "state": info.state_path(),
            "action": info.action,
            "status": info.status.map(Status::name),
            "error": info.error,
            "tags": info.tags.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
            "group": info.group,
            "blackboard": info.board.lock().to_json(),
            "group_blackboard": group_board.map(|b| b.lock().to_json()),
            "global_blackboard": self.shared.global.lock().to_json(),
            "inbox": info.inbox.iter().map(AiEvent::to_json).collect::<Vec<_>>(),
            "senses": runtime.senses.to_json(self.time),
            "runtime": runtime.brain.as_ref().map(|b| b.describe(self.time)),
            "next_tick_in": (runtime.next_tick - self.time).max(0.0),
        }))
    }

    /// Whether there is anything to draw: a debug layer is on, or an agent is selected.
    pub fn wants_debug_geometry(&self, selected: &[Entity]) -> bool {
        self.debug_draw().any() || selected.iter().any(|e| self.agents.contains_key(e))
    }

    /// The debug layers (and the selected agents' senses, goals and state).
    pub fn debug_geometry(
        &self,
        world: &World,
        selected: &[Entity],
    ) -> crate::renderer::debug_draw::DebugGeometry {
        let mut out = crate::renderer::debug_draw::DebugGeometry::default();
        let agents = self.debug_agents(world);
        super::debug::draw(&agents, self.debug_draw(), selected, &mut out);
        out
    }

    /// Agents' feet, facing, senses, goals and states, for debug drawing.
    pub(crate) fn debug_agents(&self, world: &World) -> Vec<super::debug::AgentView> {
        let agents = self.shared.agents.borrow();
        self.order
            .iter()
            .filter(|e| world.contains(**e))
            .filter_map(|&entity| {
                let runtime = self.agents.get(&entity)?;
                let info = agents.get(&entity)?;
                let transform = crate::physics::world_transform(world, entity);
                let forward = transform.rotation * Vec3::Z;
                Some(super::debug::AgentView {
                    entity,
                    position: transform.translation,
                    yaw: forward.x.atan2(forward.z),
                    perception: runtime.component.perception.clone(),
                    goals: runtime
                        .brain
                        .as_ref()
                        .map(BrainRuntime::goals)
                        .unwrap_or_default(),
                    state: info
                        .states
                        .last()
                        .map(|s| s.to_string())
                        .or_else(|| info.action.clone()),
                    senses: runtime.senses.clone(),
                })
            })
            .collect()
    }
}

/// Stop an agent's brain (its leaves and states clean up); it starts again when next
/// ticked.
fn halt(
    agent: &mut AgentRuntime,
    entity: Entity,
    shared: &AiShared,
    host: &mut dyn AiHost,
    time: f64,
    frame: u64,
) {
    let Some(mut brain) = agent.brain.take() else {
        return;
    };
    let Some(boards) = boards_for(shared, entity) else {
        return;
    };
    let mut outbox = Vec::new();
    let senses = agent.senses.clone();
    let mut cx = Cx {
        host,
        library: &shared.library,
        agent: entity,
        boards: &boards,
        time,
        dt: 0.0,
        frame,
        events: &[],
        senses: &senses,
        rng: &mut agent.rng,
        outbox: &mut outbox,
        states: brain.state_path(),
        state_entered: time,
        event: None,
        run_speed: agent.component.run_speed,
        it: None,
    };
    brain.halt(&mut cx);
}

fn boards_for(shared: &AiShared, entity: Entity) -> Option<Boards> {
    let (own, group) = {
        let agents = shared.agents.borrow();
        let info = agents.get(&entity)?;
        (info.board.clone(), info.group.clone())
    };
    Some(Boards {
        own,
        group: shared.group_board(&group),
        global: shared.global.clone(),
    })
}

fn tags_of(component: &AiAgent) -> Arc<[Arc<str>]> {
    component
        .tags
        .iter()
        .map(|t| Arc::from(t.as_str()))
        .collect::<Vec<_>>()
        .into()
}

fn tick_interval(component: &AiAgent) -> f64 {
    if component.tick_rate > 0.0 {
        1.0 / component.tick_rate as f64
    } else {
        0.0
    }
}

fn has_script(world: &World, entity: Entity) -> bool {
    world
        .satisfies::<&crate::scripting::LuaScriptComponent>(entity)
        .unwrap_or(false)
}

fn agent_height(world: &World, entity: Entity) -> f32 {
    #[cfg(feature = "navigation")]
    if let Ok(agent) = world.get::<&crate::navigation::NavAgent>(entity) {
        return agent.height;
    }
    let _ = (world, entity);
    1.7
}

/// A brain by name: defined in the library, or a JSON file in the project.
fn resolve_brain(library: &RefCell<Library>, name: &str) -> Result<Brain, String> {
    if let Some(brain) = library.borrow().get(name) {
        return Ok(brain.clone());
    }
    if name.ends_with(".json") {
        let brain = load_brain_file(name)?;
        let mut library = library.borrow_mut();
        library.define(name, brain.clone());
        library.set_file_stamp(name, file_stamp(name));
        return Ok(brain);
    }
    Err(format!(
        "no brain called {name:?}: define it with ai_tree / ai_machine, or name a JSON file in the project"
    ))
}

/// Read a brain from a JSON file (a tree's root node, or a machine with `states`).
pub fn load_brain_file(path: &str) -> Result<Brain, String> {
    let file = crate::project::resolve_project_path(path);
    let bytes = crate::io::load_binary(&file)?;
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|err| format!("{path}: {err}"))?;
    parse_brain(path, &Def::from_json(&json))
}

fn file_stamp(path: &str) -> Option<std::time::SystemTime> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::fs::metadata(crate::project::resolve_project_path(path))
            .and_then(|m| m.modified())
            .ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = path;
        None
    }
}
