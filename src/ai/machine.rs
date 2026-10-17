//! Running a state machine: one [`MachineRuntime`] per agent (and per nested machine)
//! over a shared [`MachineDef`].
//!
//! Each tick: the machine's transitions are checked (machine-wide ones first, then the
//! current state's; the first that holds is taken), then the state runs (its `update`,
//! its tree, its nested machine). When the tree or nested machine ends, transitions with
//! `on` are checked; if none is taken the tree starts again.

use std::collections::VecDeque;
use std::sync::Arc;

use rand::Rng;

use super::actions::store_event;
use super::context::{AiEvent, CallPhase, Cx};
use super::def::{BrainRef, DoneOn, MachineDef, Status, TransitionDef, TransitionTarget};
use super::tree::TreeRuntime;
use super::value::Value;

/// Transitions one machine may take in a tick (more means a loop in its rules).
const MAX_TRANSITIONS_PER_TICK: usize = 8;
const HISTORY: usize = 12;

#[derive(Debug, Clone)]
struct Taken {
    from: String,
    to: String,
    time: f64,
    why: String,
}

pub struct MachineRuntime {
    def: Arc<MachineDef>,
    current: usize,
    previous: Option<usize>,
    entered: f64,
    /// When each `after` transition is due: machine-wide ones first, then the state's.
    deadlines: Vec<Option<f64>>,
    tree: Option<TreeRuntime>,
    sub: Option<Box<MachineRuntime>>,
    /// What the state's tree or machine ended with this tick.
    done: Option<Status>,
    /// A final state was entered.
    finished: Option<Status>,
    started: bool,
    history: VecDeque<Taken>,
    pub transitions: u64,
}

/// A transition: machine-wide (`true`) or the current state's, and its index.
type TransitionRef = (bool, usize);

impl MachineRuntime {
    pub fn new(def: Arc<MachineDef>) -> Self {
        Self {
            current: def.initial,
            def,
            previous: None,
            entered: 0.0,
            deadlines: Vec::new(),
            tree: None,
            sub: None,
            done: None,
            finished: None,
            started: false,
            history: VecDeque::new(),
            transitions: 0,
        }
    }

    pub fn def(&self) -> &Arc<MachineDef> {
        &self.def
    }

    pub fn state_name(&self) -> &str {
        &self.def.states[self.current].name
    }

    pub fn finished(&self) -> Option<Status> {
        self.finished
    }

    /// The active states, this machine's first.
    pub fn state_path(&self) -> Vec<Arc<str>> {
        let mut path = vec![Arc::from(self.state_name())];
        if let Some(sub) = &self.sub {
            if sub.started {
                path.extend(sub.state_path());
            }
        }
        path
    }

    /// Tick at nesting `depth` (`cx.states[depth]` is this machine's state).
    pub fn tick(&mut self, cx: &mut Cx, depth: usize) -> Status {
        if !self.started {
            self.started = true;
            self.set_path(cx, depth);
            self.enter_state(self.def.initial, cx, depth, None);
        }
        if let Some(finished) = self.finished {
            return finished;
        }
        self.set_path(cx, depth);

        for _ in 0..MAX_TRANSITIONS_PER_TICK {
            let Some((t, event)) = self.pick_transition(cx) else {
                break;
            };
            self.take(t, event, cx, depth);
            if let Some(finished) = self.finished {
                return finished;
            }
        }

        // Run the state.
        let def = self.def.clone();
        let state = &def.states[self.current];
        if let Some(update) = &state.update {
            cx.state_entered = self.entered;
            let node = format!("{}.update", state.name);
            if let Some(Value::Str(next)) =
                cx.call_or_warn(update, &node, CallPhase::StateUpdate, None, None)
            {
                // `update` may name the state to go to.
                match def.state_index(&next) {
                    Some(index) if index != self.current => {
                        self.go(index, None, cx, depth, format!("update -> {next}"));
                        return Status::Running;
                    }
                    Some(_) => {}
                    None => {
                        let agent = cx.agent;
                        cx.host.warn(
                            agent,
                            &format!("{node}: returned {next:?}, which is not a state"),
                        );
                    }
                }
            }
        }
        if let Some(tree) = &mut self.tree {
            cx.state_entered = self.entered;
            let status = tree.tick(cx);
            if status != Status::Running {
                self.done = Some(status);
            }
        }
        if let Some(sub) = &mut self.sub {
            let status = sub.tick(cx, depth + 1);
            cx.state_entered = self.entered;
            if status != Status::Running {
                self.done = Some(status);
            }
        }

        if self.done.is_some() {
            if let Some((t, event)) = self.pick_transition(cx) {
                self.take(t, event, cx, depth);
            } else if self.sub.as_ref().is_some_and(|s| s.finished.is_some()) {
                // Nobody took the nested machine's end: it starts over.
                let restart = self.sub.as_ref().map(|s| s.def.clone());
                if let Some(def) = restart {
                    self.sub = Some(Box::new(MachineRuntime::new(def)));
                }
            }
            self.done = None;
        }
        self.finished.unwrap_or(Status::Running)
    }

    /// Leave whatever state is active (the brain stops or changes).
    pub fn exit_all(&mut self, cx: &mut Cx) {
        if self.started && self.finished.is_none() {
            self.exit_state(cx);
        } else if let Some(tree) = &mut self.tree {
            tree.halt(cx);
        }
        self.started = false;
    }

    fn set_path(&self, cx: &mut Cx, depth: usize) {
        let name: Arc<str> = Arc::from(self.state_name());
        if cx.states.len() <= depth {
            cx.states.resize(depth + 1, name.clone());
        }
        cx.states[depth] = name;
        cx.state_entered = self.entered;
    }

    fn transition(&self, (machine_wide, index): TransitionRef) -> &TransitionDef {
        if machine_wide {
            &self.def.transitions[index]
        } else {
            &self.def.states[self.current].transitions[index]
        }
    }

    fn pick_transition(&mut self, cx: &mut Cx) -> Option<(TransitionRef, Option<AiEvent>)> {
        let def = self.def.clone();
        let wide = def
            .transitions
            .iter()
            .enumerate()
            .filter(|(_, t)| !t.except.contains(&self.current))
            .map(|(i, t)| ((true, i), t, i));
        let own_offset = def.transitions.len();
        let own = def.states[self.current]
            .transitions
            .iter()
            .enumerate()
            .map(|(i, t)| ((false, i), t, own_offset + i));
        for (reference, t, deadline_index) in wide.chain(own) {
            if let Some(on) = t.on {
                let matches = match (on, self.done) {
                    (_, None) => false,
                    (DoneOn::Done, Some(_)) => true,
                    (DoneOn::Success, Some(s)) => s == Status::Success,
                    (DoneOn::Failure, Some(s)) => s == Status::Failure,
                };
                if !matches {
                    continue;
                }
            }
            if t.after.is_some() {
                match self.deadlines.get(deadline_index).copied().flatten() {
                    Some(due) if cx.time >= due => {}
                    _ => continue,
                }
            }
            let event = match &t.event {
                Some(name) => match cx.events.iter().find(|e| &*e.name == name.as_str()) {
                    Some(event) => Some(event.clone()),
                    None => continue,
                },
                None => None,
            };
            if let Some(when) = &t.when {
                cx.state_entered = self.entered;
                cx.event = event.clone();
                let holds = cx.eval_cond(when, &self.def.states[self.current].name);
                cx.event = None;
                if !holds {
                    continue;
                }
            }
            if let Some(chance) = t.chance {
                if cx.rng.gen::<f64>() >= chance {
                    continue;
                }
            }
            return Some((reference, event));
        }
        None
    }

    fn take(&mut self, t: TransitionRef, event: Option<AiEvent>, cx: &mut Cx, depth: usize) {
        let def = self.def.clone();
        let transition = self.transition(t).clone();
        if let Some(event) = &event {
            store_event(
                cx,
                event,
                &transition.store,
                &transition.store_from,
                &transition.store_position,
            );
        }
        let target = match transition.to {
            TransitionTarget::State(index) => index,
            TransitionTarget::Previous => self.previous.unwrap_or(self.current),
            TransitionTarget::Restart => self.current,
        };
        let why = transition.describe(&def);
        self.go(target, event, cx, depth, why);
    }

    fn go(
        &mut self,
        target: usize,
        event: Option<AiEvent>,
        cx: &mut Cx,
        depth: usize,
        why: String,
    ) {
        let from = self.state_name().to_owned();
        self.exit_state(cx);
        self.history.push_back(Taken {
            from,
            to: self.def.states[target].name.clone(),
            time: cx.time,
            why,
        });
        while self.history.len() > HISTORY {
            self.history.pop_front();
        }
        self.transitions += 1;
        self.enter_state(target, cx, depth, event);
    }

    fn enter_state(&mut self, index: usize, cx: &mut Cx, depth: usize, event: Option<AiEvent>) {
        let def = self.def.clone();
        if self.started && self.history.back().is_some() {
            self.previous = Some(self.current);
        }
        self.current = index;
        self.entered = cx.time;
        self.done = None;
        cx.states.truncate(depth + 1);
        self.set_path(cx, depth);

        let state = &def.states[index];
        self.deadlines = def
            .transitions
            .iter()
            .chain(&state.transitions)
            .map(|t| {
                t.after
                    .as_ref()
                    .and_then(|after| cx.eval_num(after, &state.name))
                    .map(|s| cx.time + s)
            })
            .collect();

        for (key, value) in &state.set {
            let value = cx.eval_operand(value, &state.name);
            cx.boards.set(key, value);
        }
        if let Some(enter) = &state.enter {
            cx.event = event;
            let node = format!("{}.enter", state.name);
            cx.call_or_warn(enter, &node, CallPhase::StateEnter, None, None);
            cx.event = None;
        }
        self.tree = match &state.tree {
            Some(BrainRef::Inline(tree)) => Some(TreeRuntime::new(tree.clone())),
            Some(BrainRef::Named(name)) => match cx.tree_named(name) {
                Some(tree) => Some(TreeRuntime::new(tree)),
                None => {
                    let agent = cx.agent;
                    cx.host.warn(
                        agent,
                        &format!("state {}: no tree called {name:?} is defined", state.name),
                    );
                    None
                }
            },
            None => None,
        };
        self.sub = match &state.machine {
            Some(BrainRef::Inline(machine)) => Some(Box::new(MachineRuntime::new(machine.clone()))),
            Some(BrainRef::Named(name)) => match cx.machine_named(name) {
                Some(machine) => Some(Box::new(MachineRuntime::new(machine))),
                None => {
                    let agent = cx.agent;
                    cx.host.warn(
                        agent,
                        &format!(
                            "state {}: no machine called {name:?} is defined",
                            state.name
                        ),
                    );
                    None
                }
            },
            None => None,
        };
        if let Some(status) = state.finish {
            self.finished = Some(status);
        }
    }

    fn exit_state(&mut self, cx: &mut Cx) {
        let def = self.def.clone();
        let state = &def.states[self.current];
        if let Some(mut sub) = self.sub.take() {
            sub.exit_all(cx);
        }
        if let Some(mut tree) = self.tree.take() {
            tree.halt(cx);
        }
        if let Some(exit) = &state.exit {
            let node = format!("{}.exit", state.name);
            cx.call_or_warn(exit, &node, CallPhase::StateExit, None, None);
        }
        self.done = None;
    }

    /// The running tree's current leaf, innermost machine first.
    pub fn current_action(&self) -> Option<String> {
        if let Some(sub) = &self.sub {
            if let Some(action) = sub.current_action() {
                return Some(action);
            }
        }
        self.tree.as_ref().and_then(TreeRuntime::current_action)
    }

    pub fn trees(&self) -> Vec<&TreeRuntime> {
        let mut out: Vec<&TreeRuntime> = self.tree.iter().collect();
        if let Some(sub) = &self.sub {
            out.extend(sub.trees());
        }
        out
    }

    pub fn describe(&self, now: f64) -> serde_json::Value {
        let def = &self.def;
        let state = &def.states[self.current];
        let transitions: Vec<String> = def
            .transitions
            .iter()
            .filter(|t| !t.except.contains(&self.current))
            .chain(&state.transitions)
            .map(|t| t.describe(def))
            .collect();
        let mut out = serde_json::json!({
            "machine": def.name,
            "state": state.name,
            "time_in_state": now - self.entered,
            "previous": self.previous.map(|p| def.states[p].name.clone()),
            "finished": self.finished.map(Status::name),
            "states": def.states.iter().map(|s| s.name.clone()).collect::<Vec<_>>(),
            "transitions": transitions,
            "history": self.history.iter().rev().map(|t| serde_json::json!({
                "from": t.from, "to": t.to, "ago": now - t.time, "why": t.why,
            })).collect::<Vec<_>>(),
        });
        if let Some(tree) = &self.tree {
            out["tree"] = tree.describe(now);
        }
        if let Some(sub) = &self.sub {
            out["machine_in_state"] = sub.describe(now);
        }
        out
    }
}
