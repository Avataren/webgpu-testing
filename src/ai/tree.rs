//! Running a behaviour tree: one [`TreeRuntime`] per agent over a shared [`TreeDef`].
//!
//! Each tick walks down from the root to the running node(s). A node is entered the
//! first time it is ticked and exited when it ends or is halted (a guard aborting it,
//! a timeout, its parent moving on, the state it belongs to being left); leaves clean
//! up on exit (a walk stops, an animation fades out, a Lua `exit` runs).

use std::collections::HashMap;
use std::sync::Arc;

use rand::Rng;

use super::actions::{self, Leaf};
use super::context::Cx;
use super::def::{NodeKind, Status, Threshold, TreeDef};

/// Per-agent state of one node.
#[derive(Debug, Clone, Default)]
struct NodeRt {
    active: bool,
    /// The running child: an index into `order` when the node has one, else into its
    /// children.
    child: usize,
    /// Children's positions in the order they run (shuffled, by weight or by score).
    order: Vec<usize>,
    count: u32,
    started: f64,
    deadline: f64,
    /// When the child of a cooldown last ended.
    mark: Option<f64>,
    /// The result a `once` keeps.
    kept: Option<Status>,
    /// A parallel's finished children.
    results: Vec<Option<Status>>,
    /// A utility node's last scores, by child position.
    scores: Vec<Option<f64>>,
    scored_at: f64,
    /// Its result when it was last ticked, and when.
    last: Option<Status>,
    last_frame: u64,
    leaf: Leaf,
}

/// An agent's run of a behaviour tree.
pub struct TreeRuntime {
    def: Arc<TreeDef>,
    nodes: Vec<NodeRt>,
    /// Trees run by `subtree` nodes.
    subtrees: HashMap<usize, TreeRuntime>,
    /// The frame of the last tick.
    frame: u64,
    pub last: Option<Status>,
}

impl TreeRuntime {
    pub fn new(def: Arc<TreeDef>) -> Self {
        let nodes = vec![NodeRt::default(); def.nodes.len()];
        Self {
            def,
            nodes,
            subtrees: HashMap::new(),
            frame: 0,
            last: None,
        }
    }

    pub fn def(&self) -> &Arc<TreeDef> {
        &self.def
    }

    pub fn is_running(&self) -> bool {
        self.nodes.first().is_some_and(|n| n.active)
    }

    /// Tick from the root. When the root ends, the next tick starts the tree again.
    pub fn tick(&mut self, cx: &mut Cx) -> Status {
        let def = self.def.clone();
        self.frame = cx.frame;
        let status = self.tick_node(&def, TreeDef::ROOT, cx);
        self.last = Some(status);
        status
    }

    /// Stop whatever runs (its leaves clean up), as if the tree were left.
    pub fn halt(&mut self, cx: &mut Cx) {
        let def = self.def.clone();
        self.halt_node(&def, TreeDef::ROOT, cx);
    }

    fn tick_node(&mut self, def: &TreeDef, id: usize, cx: &mut Cx) -> Status {
        let status = if self.nodes[id].active {
            self.run(def, id, cx)
        } else {
            let keep = &self.nodes[id];
            self.nodes[id] = NodeRt {
                active: true,
                started: cx.time,
                mark: keep.mark,
                kept: keep.kept,
                ..NodeRt::default()
            };
            match self.enter(def, id, cx) {
                Some(status) => status,
                None => self.run(def, id, cx),
            }
        };
        let rt = &mut self.nodes[id];
        rt.last = Some(status);
        rt.last_frame = cx.frame;
        if status != Status::Running && rt.active {
            self.exit(def, id, cx, Some(status));
        }
        status
    }

    /// Entering a node: set up its timers and order. A result ends it at once.
    fn enter(&mut self, def: &TreeDef, id: usize, cx: &mut Cx) -> Option<Status> {
        let node = &def.nodes[id];
        let label = node.label.as_str();
        match &node.kind {
            NodeKind::ShuffleSequence | NodeKind::ShuffleSelector => {
                let mut order: Vec<usize> = (0..node.children.len()).collect();
                if matches!(node.kind, NodeKind::ShuffleSelector) {
                    // Weighted shuffle: repeatedly pick by weight.
                    let mut weights: Vec<f64> =
                        node.children.iter().map(|&c| def.nodes[c].weight).collect();
                    let mut picked = Vec::with_capacity(order.len());
                    while !order.is_empty() {
                        let k = weighted_pick(&weights, cx);
                        picked.push(order.remove(k));
                        weights.remove(k);
                    }
                    order = picked;
                } else {
                    for i in (1..order.len()).rev() {
                        order.swap(i, cx.rng.gen_range(0..=i));
                    }
                }
                self.nodes[id].order = order;
            }
            NodeKind::Random => {
                let weights: Vec<f64> =
                    node.children.iter().map(|&c| def.nodes[c].weight).collect();
                let k = weighted_pick(&weights, cx);
                self.nodes[id].order = vec![k];
            }
            NodeKind::Utility { .. } => {
                self.score_children(def, id, cx);
                if self.nodes[id].order.is_empty() {
                    return Some(Status::Failure);
                }
            }
            NodeKind::Parallel { .. } => {
                self.nodes[id].results = vec![None; node.children.len()];
            }
            NodeKind::Timeout { time } | NodeKind::Delay { time } => {
                let seconds = cx.eval_num(time, label).unwrap_or(0.0);
                self.nodes[id].deadline = cx.time + seconds;
            }
            NodeKind::Cooldown { time } => {
                if let Some(mark) = self.nodes[id].mark {
                    let seconds = cx.eval_num(time, label).unwrap_or(0.0);
                    if cx.time - mark < seconds {
                        return Some(Status::Failure);
                    }
                }
            }
            NodeKind::Once => {
                if let Some(kept) = self.nodes[id].kept {
                    return Some(kept);
                }
            }
            NodeKind::Chance { p } => {
                let p = cx.eval_num(p, label).unwrap_or(0.0);
                if cx.rng.gen::<f64>() >= p {
                    return Some(Status::Failure);
                }
            }
            NodeKind::Guard { cond, .. } => {
                if !cx.eval_cond(cond, label) {
                    return Some(Status::Failure);
                }
            }
            NodeKind::Subtree { name } => match cx.tree_named(name) {
                Some(tree) => {
                    if !self
                        .subtrees
                        .get(&id)
                        .is_some_and(|rt| Arc::ptr_eq(&rt.def, &tree))
                    {
                        self.subtrees.insert(id, TreeRuntime::new(tree));
                    }
                }
                None => {
                    let agent = cx.agent;
                    cx.host.warn(
                        agent,
                        &format!("{label}: no tree called {name:?} is defined"),
                    );
                    return Some(Status::Failure);
                }
            },
            NodeKind::Leaf(action) => {
                let mut leaf = Leaf::None;
                let status = actions::enter(action, &mut leaf, label, cx);
                self.nodes[id].leaf = leaf;
                return status;
            }
            _ => {}
        }
        None
    }

    fn run(&mut self, def: &TreeDef, id: usize, cx: &mut Cx) -> Status {
        let node = &def.nodes[id];
        let children = &node.children;
        match &node.kind {
            NodeKind::Sequence | NodeKind::ShuffleSequence | NodeKind::Random => loop {
                let rt = &self.nodes[id];
                let Some(child) = self.child_at(def, id, rt.child) else {
                    return Status::Success;
                };
                match self.tick_node(def, child, cx) {
                    Status::Success => self.nodes[id].child += 1,
                    other => return other,
                }
            },
            NodeKind::Selector | NodeKind::ShuffleSelector | NodeKind::Utility { .. } => {
                if let NodeKind::Utility {
                    interval: Some(interval),
                    hysteresis,
                } = node.kind
                {
                    self.rescore(def, id, interval, hysteresis, cx);
                }
                self.check_lower_aborts(def, id, cx);
                loop {
                    let rt = &self.nodes[id];
                    let Some(child) = self.child_at(def, id, rt.child) else {
                        return Status::Failure;
                    };
                    match self.tick_node(def, child, cx) {
                        Status::Failure => self.nodes[id].child += 1,
                        other => return other,
                    }
                }
            }
            NodeKind::ReactiveSequence | NodeKind::ReactiveSelector => {
                let selector = matches!(node.kind, NodeKind::ReactiveSelector);
                let mut result = if selector {
                    Status::Failure
                } else {
                    Status::Success
                };
                let mut stopped_at = children.len();
                for (i, &child) in children.iter().enumerate() {
                    let status = self.tick_node(def, child, cx);
                    let goes_on = if selector {
                        status == Status::Failure
                    } else {
                        status == Status::Success
                    };
                    if !goes_on {
                        result = status;
                        stopped_at = i;
                        break;
                    }
                }
                // Whatever ran before but was not reached now stops.
                for &child in children.iter().skip(stopped_at + 1) {
                    self.halt_node(def, child, cx);
                }
                result
            }
            NodeKind::Parallel { success, failure } => {
                for (k, &child) in children.iter().enumerate() {
                    if self.nodes[id].results[k].is_some() {
                        continue;
                    }
                    let status = self.tick_node(def, child, cx);
                    if status != Status::Running {
                        self.nodes[id].results[k] = Some(status);
                    }
                }
                let results = &self.nodes[id].results;
                let succeeded = results
                    .iter()
                    .filter(|r| **r == Some(Status::Success))
                    .count();
                let failed = results
                    .iter()
                    .filter(|r| **r == Some(Status::Failure))
                    .count();
                let need = |t: Threshold| match t {
                    Threshold::All => children.len(),
                    Threshold::Count(n) => n.min(children.len()),
                };
                let outcome = if succeeded >= need(*success) {
                    Some(Status::Success)
                } else if failed >= need(*failure) {
                    Some(Status::Failure)
                } else if succeeded + failed == children.len() {
                    // Everything ended without reaching either count.
                    Some(Status::Failure)
                } else {
                    None
                };
                match outcome {
                    Some(status) => {
                        for &child in children {
                            self.halt_node(def, child, cx);
                        }
                        status
                    }
                    None => Status::Running,
                }
            }
            NodeKind::Invert => match self.tick_node(def, children[0], cx) {
                Status::Success => Status::Failure,
                Status::Failure => Status::Success,
                Status::Running => Status::Running,
            },
            NodeKind::ForceSuccess => match self.tick_node(def, children[0], cx) {
                Status::Running => Status::Running,
                _ => Status::Success,
            },
            NodeKind::ForceFailure => match self.tick_node(def, children[0], cx) {
                Status::Running => Status::Running,
                _ => Status::Failure,
            },
            NodeKind::Repeat { count, until } => {
                let status = self.tick_node(def, children[0], cx);
                match (status, until) {
                    (Status::Running, _) => Status::Running,
                    (s, Some(u)) if s == *u => Status::Success,
                    (Status::Failure, None) => Status::Failure,
                    (Status::Success, Some(Status::Success)) => Status::Success,
                    _ => {
                        let rt = &mut self.nodes[id];
                        rt.count += 1;
                        if count.is_some_and(|n| rt.count >= n) {
                            Status::Success
                        } else {
                            // Again from the next tick.
                            Status::Running
                        }
                    }
                }
            }
            NodeKind::Retry { count } => match self.tick_node(def, children[0], cx) {
                Status::Failure => {
                    let rt = &mut self.nodes[id];
                    rt.count += 1;
                    if rt.count > *count {
                        Status::Failure
                    } else {
                        Status::Running
                    }
                }
                other => other,
            },
            NodeKind::Timeout { .. } => {
                if cx.time >= self.nodes[id].deadline {
                    self.halt_node(def, children[0], cx);
                    Status::Failure
                } else {
                    self.tick_node(def, children[0], cx)
                }
            }
            NodeKind::Delay { .. } => {
                if cx.time < self.nodes[id].deadline {
                    Status::Running
                } else {
                    self.tick_node(def, children[0], cx)
                }
            }
            NodeKind::Cooldown { .. } => {
                let status = self.tick_node(def, children[0], cx);
                if status != Status::Running {
                    self.nodes[id].mark = Some(cx.time);
                }
                status
            }
            NodeKind::Once => {
                let status = self.tick_node(def, children[0], cx);
                if status != Status::Running {
                    self.nodes[id].kept = Some(status);
                }
                status
            }
            NodeKind::Chance { .. } => self.tick_node(def, children[0], cx),
            NodeKind::Guard { cond, abort } => {
                let child_running = self.nodes[children[0]].active;
                if child_running && abort.aborts_self() && !cx.eval_cond(cond, &node.label) {
                    self.halt_node(def, children[0], cx);
                    return Status::Failure;
                }
                self.tick_node(def, children[0], cx)
            }
            NodeKind::Subtree { .. } => match self.subtrees.get_mut(&id) {
                Some(sub) => sub.tick(cx),
                None => Status::Failure,
            },
            NodeKind::Leaf(action) => {
                let mut leaf = std::mem::take(&mut self.nodes[id].leaf);
                let status = actions::tick(action, &mut leaf, &node.label, cx);
                self.nodes[id].leaf = leaf;
                status
            }
        }
    }

    /// The child at position `index` of the run order.
    fn child_at(&self, def: &TreeDef, id: usize, index: usize) -> Option<usize> {
        let node = &def.nodes[id];
        let rt = &self.nodes[id];
        let uses_order = matches!(
            node.kind,
            NodeKind::ShuffleSequence
                | NodeKind::ShuffleSelector
                | NodeKind::Random
                | NodeKind::Utility { .. }
        );
        if uses_order {
            rt.order.get(index).map(|&k| node.children[k])
        } else {
            node.children.get(index).copied()
        }
    }

    /// A selector running a later child gives way to an earlier guard with a
    /// lower-priority abort whose condition holds now.
    fn check_lower_aborts(&mut self, def: &TreeDef, id: usize, cx: &mut Cx) {
        let current = self.nodes[id].child;
        let Some(running) = self.child_at(def, id, current) else {
            return;
        };
        if current == 0 || !self.nodes[running].active {
            return;
        }
        for position in 0..current {
            let Some(child) = self.child_at(def, id, position) else {
                continue;
            };
            if let NodeKind::Guard { cond, abort } = &def.nodes[child].kind {
                if abort.aborts_lower() && cx.eval_cond(cond, &def.nodes[child].label) {
                    self.halt_node(def, running, cx);
                    self.nodes[id].child = position;
                    return;
                }
            }
        }
    }

    /// Score a utility node's children and order them best first (negative or missing
    /// scores leave a child out).
    fn score_children(&mut self, def: &TreeDef, id: usize, cx: &mut Cx) {
        let node = &def.nodes[id];
        let mut scores = Vec::with_capacity(node.children.len());
        for &child in &node.children {
            let child_def = &def.nodes[child];
            let score = match &child_def.score {
                Some(num) => cx.eval_num(num, &child_def.label),
                None => Some(0.0),
            };
            scores.push(score.filter(|s| s.is_finite() && *s >= 0.0));
        }
        let mut order: Vec<usize> = (0..scores.len()).filter(|&k| scores[k].is_some()).collect();
        order.sort_by(|&a, &b| {
            scores[b]
                .unwrap_or(0.0)
                .total_cmp(&scores[a].unwrap_or(0.0))
        });
        let rt = &mut self.nodes[id];
        rt.scores = scores;
        rt.order = order;
        rt.child = 0;
        rt.scored_at = cx.time;
    }

    /// Re-score a running utility node; switch when another child now beats the
    /// running one by `hysteresis`.
    fn rescore(&mut self, def: &TreeDef, id: usize, interval: f32, hysteresis: f64, cx: &mut Cx) {
        if cx.time - self.nodes[id].scored_at < interval as f64 {
            return;
        }
        let running_position = self.nodes[id].order.get(self.nodes[id].child).copied();
        let running_node = self.child_at(def, id, self.nodes[id].child);
        let old_order = std::mem::take(&mut self.nodes[id].order);
        let old_child = self.nodes[id].child;
        self.score_children(def, id, cx);
        let rt = &self.nodes[id];
        let best = rt.order.first().copied();
        let switch = match (running_position, best) {
            (Some(running), Some(best)) if running != best => {
                let score = |k: usize| rt.scores.get(k).copied().flatten();
                match (score(best), score(running)) {
                    (Some(b), Some(r)) => b > r + hysteresis,
                    (Some(_), None) => true,
                    _ => false,
                }
            }
            (Some(_), None) => true,
            _ => false,
        };
        if switch {
            if let Some(running) = running_node {
                self.halt_node(def, running, cx);
            }
        } else {
            // Keep going where it was, with the new scores shown.
            let rt = &mut self.nodes[id];
            rt.order = old_order;
            rt.child = old_child;
        }
    }

    fn exit(&mut self, def: &TreeDef, id: usize, cx: &mut Cx, status: Option<Status>) {
        let node = &def.nodes[id];
        match &node.kind {
            NodeKind::Leaf(action) => {
                let mut leaf = std::mem::take(&mut self.nodes[id].leaf);
                actions::exit(action, &mut leaf, &node.label, cx, status);
            }
            NodeKind::Subtree { .. } => {
                if let Some(sub) = self.subtrees.get_mut(&id) {
                    if status.is_none() {
                        sub.halt(cx);
                    }
                }
            }
            _ => {}
        }
        self.nodes[id].active = false;
    }

    /// Stop a node and everything running under it.
    fn halt_node(&mut self, def: &TreeDef, id: usize, cx: &mut Cx) {
        if !self.nodes[id].active {
            return;
        }
        for &child in &def.nodes[id].children {
            self.halt_node(def, child, cx);
        }
        self.exit(def, id, cx, None);
    }

    /// The tree as it ran: each node with its result, what is running and node details
    /// (utility scores, time left), for inspectors.
    pub fn describe(&self, now: f64) -> serde_json::Value {
        let nodes: Vec<serde_json::Value> = self
            .def
            .nodes
            .iter()
            .zip(&self.nodes)
            .map(|(node, rt)| {
                let mut entry = serde_json::json!({
                    "label": node.label,
                    "type": node.kind.type_name(),
                    "depth": node.depth,
                    "active": rt.active,
                    "last": rt.last.map(Status::name),
                    "this_tick": rt.last_frame == self.frame && rt.last.is_some(),
                });
                if !rt.scores.is_empty() {
                    entry["scores"] = serde_json::json!(rt
                        .scores
                        .iter()
                        .zip(&node.children)
                        .map(|(s, &c)| serde_json::json!({
                            "child": self.def.nodes[c].label,
                            "score": s,
                        }))
                        .collect::<Vec<_>>());
                }
                if rt.active {
                    if rt.deadline > now {
                        entry["time_left"] = serde_json::json!(rt.deadline - now);
                    }
                    if let Some(detail) = actions::describe(&rt.leaf, now) {
                        entry["detail"] = detail;
                    }
                    entry["running_for"] = serde_json::json!(now - rt.started);
                }
                if rt.count > 0 {
                    entry["count"] = serde_json::json!(rt.count);
                }
                entry
            })
            .collect();
        let mut out = serde_json::json!({
            "name": self.def.name,
            "status": self.last.map(Status::name),
            "nodes": nodes,
        });
        if !self.subtrees.is_empty() {
            let mut subs = serde_json::Map::new();
            for (id, sub) in &self.subtrees {
                if self.nodes[*id].active {
                    subs.insert(self.def.nodes[*id].label.clone(), sub.describe(now));
                }
            }
            out["subtrees"] = serde_json::Value::Object(subs);
        }
        out
    }

    /// The labels of the running path, root to leaf (the deepest running leaf last).
    pub fn running_path(&self) -> Vec<String> {
        let mut path = Vec::new();
        let mut id = TreeDef::ROOT;
        loop {
            if !self.nodes[id].active {
                break;
            }
            path.push(self.def.nodes[id].label.clone());
            if let Some(sub) = self.subtrees.get(&id) {
                path.extend(sub.running_path());
                break;
            }
            match self.def.nodes[id]
                .children
                .iter()
                .find(|&&c| self.nodes[c].active)
            {
                Some(&child) => id = child,
                None => break,
            }
        }
        path
    }

    /// Where running walks are headed (this tree's and its subtrees').
    pub fn goals(&self) -> Vec<glam::Vec3> {
        let mut goals: Vec<glam::Vec3> = self
            .nodes
            .iter()
            .filter(|rt| rt.active)
            .filter_map(|rt| actions::goal(&rt.leaf))
            .collect();
        for (id, sub) in &self.subtrees {
            if self.nodes[*id].active {
                goals.extend(sub.goals());
            }
        }
        goals
    }

    /// The running leaf's label (what the agent is doing).
    pub fn current_action(&self) -> Option<String> {
        self.running_path().pop()
    }
}

fn weighted_pick(weights: &[f64], cx: &mut Cx) -> usize {
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return cx.rng.gen_range(0..weights.len().max(1));
    }
    let mut pick = cx.rng.gen::<f64>() * total;
    for (k, w) in weights.iter().enumerate() {
        pick -= w;
        if pick <= 0.0 {
            return k;
        }
    }
    weights.len() - 1
}
