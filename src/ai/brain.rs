//! An agent's running brain: a tree or a state machine.

use std::sync::Arc;

use glam::Vec3;

use super::context::Cx;
use super::def::{Brain, Status};
use super::machine::MachineRuntime;
use super::tree::TreeRuntime;

pub enum BrainRuntime {
    Tree(TreeRuntime),
    Machine(MachineRuntime),
}

impl BrainRuntime {
    pub fn new(brain: &Brain) -> Self {
        match brain {
            Brain::Tree(tree) => BrainRuntime::Tree(TreeRuntime::new(tree.clone())),
            Brain::Machine(machine) => BrainRuntime::Machine(MachineRuntime::new(machine.clone())),
        }
    }

    /// Think once. A tree that ends starts again next tick; a machine that reaches a
    /// final state stays there.
    pub fn tick(&mut self, cx: &mut Cx) -> Status {
        match self {
            BrainRuntime::Tree(tree) => tree.tick(cx),
            BrainRuntime::Machine(machine) => {
                cx.states = machine.state_path();
                machine.tick(cx, 0)
            }
        }
    }

    /// Stop: running leaves clean up and states run their `exit`.
    pub fn halt(&mut self, cx: &mut Cx) {
        match self {
            BrainRuntime::Tree(tree) => tree.halt(cx),
            BrainRuntime::Machine(machine) => machine.exit_all(cx),
        }
    }

    pub fn state_path(&self) -> Vec<Arc<str>> {
        match self {
            BrainRuntime::Tree(_) => Vec::new(),
            BrainRuntime::Machine(machine) => machine.state_path(),
        }
    }

    pub fn current_action(&self) -> Option<String> {
        match self {
            BrainRuntime::Tree(tree) => tree.current_action(),
            BrainRuntime::Machine(machine) => machine.current_action(),
        }
    }

    /// Where its running walks are headed.
    pub fn goals(&self) -> Vec<Vec3> {
        match self {
            BrainRuntime::Tree(tree) => tree.goals(),
            BrainRuntime::Machine(machine) => machine
                .trees()
                .into_iter()
                .flat_map(TreeRuntime::goals)
                .collect(),
        }
    }

    pub fn describe(&self, now: f64) -> serde_json::Value {
        match self {
            BrainRuntime::Tree(tree) => serde_json::json!({ "tree": tree.describe(now) }),
            BrainRuntime::Machine(machine) => {
                serde_json::json!({ "machine": machine.describe(now) })
            }
        }
    }
}
