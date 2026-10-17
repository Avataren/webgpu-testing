//! Drawing what agents think over the scene: view cones and hearing ranges, where they
//! walk, what they see or remember, and a ring at their feet coloured by their state.

use std::sync::Arc;

use glam::Vec3;
use hecs::Entity;

use super::perception::{PerceptionSettings, Senses};
use super::scene_ai::AiDebugDraw;
use crate::renderer::debug_draw::{DebugColor, DebugGeometry};

/// An agent as the debug view draws it.
pub(crate) struct AgentView {
    pub entity: Entity,
    pub position: Vec3,
    pub yaw: f32,
    pub perception: PerceptionSettings,
    pub goals: Vec<Vec3>,
    /// Its innermost state (or, for a tree, its action).
    pub state: Option<String>,
    pub senses: Arc<Senses>,
}

const CONE: DebugColor = [1.0, 0.85, 0.3, 0.55];
const HEARING: DebugColor = [0.45, 0.7, 1.0, 0.25];
const GOAL: DebugColor = [1.0, 0.95, 0.35, 0.9];
const SEEN: DebugColor = [0.35, 1.0, 0.45, 0.85];
const REMEMBERED: DebugColor = [0.7, 0.7, 0.7, 0.5];

/// A colour for a state name: the same name always gets the same hue.
pub fn state_color(name: &str) -> DebugColor {
    let mut hash: u32 = 2166136261;
    for byte in name.bytes() {
        hash ^= byte as u32;
        hash = hash.wrapping_mul(16777619);
    }
    let hue = (hash % 360) as f32 / 60.0;
    let x = 1.0 - (hue % 2.0 - 1.0).abs();
    let (r, g, b) = match hue as u32 {
        0 => (1.0, x, 0.0),
        1 => (x, 1.0, 0.0),
        2 => (0.0, 1.0, x),
        3 => (0.0, x, 1.0),
        4 => (x, 0.0, 1.0),
        _ => (1.0, 0.0, x),
    };
    // Pastel enough to read over dark ground.
    [0.35 + 0.65 * r, 0.35 + 0.65 * g, 0.35 + 0.65 * b, 0.95]
}

pub(crate) fn draw(
    agents: &[AgentView],
    debug: AiDebugDraw,
    selected: &[Entity],
    out: &mut DebugGeometry,
) {
    let lift = Vec3::Y * 0.06;
    for agent in agents {
        let chosen = selected.contains(&agent.entity);
        let feet = agent.position + lift;
        let settings = &agent.perception;
        if (debug.senses || chosen) && settings.sees_anything() {
            let range = settings.sight_range;
            let half = (settings.sight_angle.clamp(0.0, 360.0) * 0.5).to_radians();
            let dir = |a: f32| Vec3::new((agent.yaw + a).sin(), 0.0, (agent.yaw + a).cos());
            if half < std::f32::consts::PI {
                out.line(feet, feet + dir(-half) * range, 1.5, CONE);
                out.line(feet, feet + dir(half) * range, 1.5, CONE);
            }
            let steps = 16;
            for i in 0..steps {
                let a0 = -half + 2.0 * half * i as f32 / steps as f32;
                let a1 = -half + 2.0 * half * (i + 1) as f32 / steps as f32;
                out.line(feet + dir(a0) * range, feet + dir(a1) * range, 1.5, CONE);
            }
            if settings.near_range > 0.0 {
                out.circle(feet, settings.near_range, 1.0, CONE);
            }
        }
        if (debug.senses || chosen) && settings.hearing_range > 0.0 && chosen {
            out.circle(feet, settings.hearing_range, 1.0, HEARING);
        }
        if debug.goals || chosen {
            for goal in &agent.goals {
                out.line(feet, *goal + lift, 2.0, GOAL);
                out.circle(*goal + lift, 0.25, 2.0, GOAL);
            }
        }
        if debug.percepts || chosen {
            let head = agent.position + Vec3::Y * 1.7;
            for percept in &agent.senses.percepts {
                let color = if percept.visible { SEEN } else { REMEMBERED };
                out.line(head, percept.position + Vec3::Y * 1.2, 1.5, color);
            }
            for noise in &agent.senses.noises {
                out.circle(noise.position + lift, 0.4, 2.0, HEARING);
            }
        }
        if debug.states || chosen {
            if let Some(state) = &agent.state {
                out.circle(feet, 0.5, 4.0, state_color(state));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_colours_are_stable_and_bright() {
        assert_eq!(state_color("patrol"), state_color("patrol"));
        assert_ne!(state_color("patrol"), state_color("chase"));
        assert!(state_color("x")[..3].iter().all(|c| *c >= 0.35));
    }
}
