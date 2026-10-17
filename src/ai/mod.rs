//! AI: behaviour trees and state machines that give entities a mind of their own.
//!
//! - An [`AiAgent`] component names the entity's **brain**: a behaviour tree or a
//!   (hierarchical) state machine, defined by a script (`ai_tree`, `ai_machine`) or in
//!   a JSON file, and shared by every agent running it. Each agent keeps its own
//!   runtime state, a **blackboard** of named values (`group.` and `global.` boards are
//!   shared) and its **senses** (sight cone, hearing, memory).
//! - **Behaviour trees** ([`tree`]): sequence, selector, their reactive kinds,
//!   parallel, shuffled, random and utility (score-driven) composites; decorators
//!   (invert, repeat, retry, timeout, delay, cooldown, once, chance, guard with
//!   observer aborts); leaves that walk the navmesh, play clips, call Lua, send events,
//!   make noise, check conditions and edit the blackboard ([`actions`]).
//! - **State machines** ([`machine`]): states with `enter` / `update` / `exit` hooks,
//!   a tree or a nested machine; transitions on events, conditions, time and the end of
//!   the state's tree, machine-wide transitions, `@previous`.
//! - Conditions, scores and values are small **expressions** ([`expr`]:
//!   `hunger > 0.6 and not sees('guard')`) or Lua functions, which run in the context of
//!   the script that defined them.
//! - [`SceneAi`] runs the agents of a scene in play mode, between the scripts and the
//!   navigation, at each agent's tick rate.
//!
//! The guide is `docs/ai.md`.

pub(crate) mod actions;
pub mod agent;
pub mod blackboard;
mod brain;
pub mod context;
mod debug;
pub mod def;
pub mod expr;
mod host;
pub mod machine;
pub mod perception;
mod scene_ai;
pub mod tree;
pub mod value;

/// Reading and changing the `AiAgent` component by name, as JSON (the remote API, Lua
/// `get_component` / `set_component` and the inspector).
pub mod edit {
    pub use super::agent::{
        canonical_name, clone_components, component_json, has_component, patch_component,
        COMPONENT_NAME,
    };
}

pub use agent::AiAgent;
pub use blackboard::{split_key, Blackboard, BlackboardRef, BoardScope};
pub use context::{AiEvent, AiHost, LineOfSight, NavInfo, NavPhase, PlayRequest};
pub use debug::state_color;
pub use def::{
    parse_brain, parse_machine, parse_tree, Brain, Callable, Def, Library, LuaFn, MachineDef,
    Status, TreeDef,
};
pub use expr::{Expr, ExprEnv, Function};
pub use perception::{PerceptionSettings, Senses};
pub use scene_ai::{
    load_brain_file, AgentShared, AiCommand, AiDebugDraw, AiEnv, AiShared, AiStats, AiStepOutput,
    SceneAi,
};
pub use value::{LuaRef, Value};

pub(crate) use scene_ai::view_state as scene_ai_view;

#[cfg(test)]
mod tests;
