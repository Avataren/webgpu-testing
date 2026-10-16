//! The [`AnimationTree`] component: a state machine of blended clips driven by
//! parameters, in the manner of Godot's AnimationTree or Unity's Animator Controller.
//!
//! - **States** play a motion: one clip, or a blend space that mixes clips by one
//!   parameter (`blend_1d`, e.g. idle/walk/run by speed) or two (`blend_2d`, e.g. walking
//!   forward, backward and strafing by the velocity relative to the character's facing).
//!   Blend spaces keep their clips in step and can speed playback up past their
//!   outermost clips so feet do not slide.
//! - **Transitions** move from one state (or any, `"*"`) to another when their condition
//!   holds (`"!grounded && air_time > 0.15"`), optionally only once the state has
//!   played a share of its motion (`after`), cross-fading over `fade` seconds. The first
//!   transition in the list that applies wins.
//! - **Parameters** are numbers (true is 1). Scripts set them (`animation_set`) or fire
//!   one-frame triggers (`animation_trigger`); with `from_character` the character the
//!   tree belongs to feeds `speed`, `forward_speed`, `side_speed`, `vertical_speed`,
//!   `grounded`, `air_time`, `landing_speed` and the triggers `jumped` and `landed`.
//!
//! The tree drives the [`Animator`] on its entity, or the nearest one below (an imported
//! model under a character root) or above it. [`AnimationTree::locomotion`] builds a
//! ready tree from whatever clips a character has, by their names.

use std::collections::{BTreeMap, HashMap, HashSet};

use glam::Vec2;
use hecs::{Entity, World};
use serde::{Deserialize, Serialize};

use super::animator::{Animator, BlendClip, BlendOptions, ClipInfo};
use crate::scene::components::{Children, Parent};

fn one() -> f32 {
    1.0
}

fn yes() -> bool {
    true
}

fn default_fade() -> f32 {
    0.2
}

/// A clip at a place in a 1D blend space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlendPoint {
    pub at: f32,
    pub clip: String,
    /// Playback rate of the clip here; negative plays it backwards.
    #[serde(default = "one")]
    pub speed: f32,
}

/// A clip at a place in a 2D blend space.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BlendPoint2d {
    pub at: [f32; 2],
    pub clip: String,
    #[serde(default = "one")]
    pub speed: f32,
}

/// What a state plays.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Motion {
    Clip {
        clip: String,
        #[serde(default = "one")]
        speed: f32,
    },
    /// Clips placed along `parameter`; between two points both play, weighted by
    /// distance.
    Blend1d {
        parameter: String,
        points: Vec<BlendPoint>,
        /// Past the outermost point, play faster in proportion (feet keep pace).
        #[serde(default)]
        scale_speed: bool,
    },
    /// Clips placed in the plane of parameters `x` and `y`; inside a triangle of points
    /// the three play, weighted by position.
    Blend2d {
        x: String,
        y: String,
        points: Vec<BlendPoint2d>,
        #[serde(default)]
        scale_speed: bool,
    },
}

impl Motion {
    /// The clips the motion names.
    pub fn clips(&self) -> Vec<&str> {
        match self {
            Motion::Clip { clip, .. } => vec![clip.as_str()],
            Motion::Blend1d { points, .. } => points.iter().map(|p| p.clip.as_str()).collect(),
            Motion::Blend2d { points, .. } => points.iter().map(|p| p.clip.as_str()).collect(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationTreeState {
    pub name: String,
    pub motion: Motion,
    /// Loop the motion (locomotion), or play it once and hold its end (a jump).
    #[serde(default = "yes")]
    pub looping: bool,
    /// Playback rate of the whole state.
    #[serde(default = "one")]
    pub speed: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnimationTreeTransition {
    /// The state it leaves, or `"*"` for any other state.
    pub from: String,
    pub to: String,
    /// When it may happen: parameters (non-zero is true), `!`, comparisons
    /// (`< <= > >= == !=` with numbers), `&&`, `||` and parentheses. Empty always holds.
    #[serde(default)]
    pub when: String,
    /// Cross-fade time (s).
    #[serde(default = "default_fade")]
    pub fade: f32,
    /// Only once the state has played this share of its motion (0..1), e.g. 0.9 so a
    /// landing finishes first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<f32>,
}

/// A parameter-driven state machine of blended clips. See the [module docs](self).
///
/// ```
/// use wgpu_cube::scene::skeletal::{AnimationTree, Motion};
///
/// let tree: AnimationTree = serde_json::from_str(r#"{
///     "start": "Idle",
///     "states": [
///         { "name": "Idle", "motion": { "kind": "clip", "clip": "Idle" } },
///         { "name": "Wave", "motion": { "kind": "clip", "clip": "Wave" }, "looping": false }
///     ],
///     "transitions": [
///         { "from": "Idle", "to": "Wave", "when": "waving" },
///         { "from": "Wave", "to": "Idle", "after": 1.0 }
///     ]
/// }"#).unwrap();
/// assert!(matches!(tree.states[0].motion, Motion::Clip { .. }));
/// ```
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnimationTree {
    /// Read the standard parameters from the character (CharacterController) this
    /// entity is, or belongs to.
    pub from_character: bool,
    /// The state play starts in.
    pub start: String,
    pub states: Vec<AnimationTreeState>,
    pub transitions: Vec<AnimationTreeTransition>,
    /// Starting values of parameters scripts set.
    pub parameters: BTreeMap<String, f32>,
    /// Playback state; not saved.
    #[serde(skip)]
    pub runtime: TreeRuntime,
}

impl Default for AnimationTree {
    fn default() -> Self {
        Self {
            from_character: true,
            start: String::new(),
            states: Vec::new(),
            transitions: Vec::new(),
            parameters: BTreeMap::new(),
            runtime: TreeRuntime::default(),
        }
    }
}

/// The running side of an [`AnimationTree`].
#[derive(Debug, Clone, Default)]
pub struct TreeRuntime {
    /// The current state (by name), once started.
    pub state: Option<String>,
    /// The state before it.
    pub previous: Option<String>,
    /// Seconds in the current state.
    pub time_in_state: f32,
    /// How far its motion has played (0..1).
    pub progress: f32,
    /// Parameter values (set ones and fed ones).
    pub values: HashMap<String, f32>,
    /// Triggers fired since the last evaluation.
    pub triggers: HashSet<String>,
    /// A state a script asked to go to (`animation_travel`) and the fade.
    pub travel: Option<(String, f32)>,
    /// Parsed conditions, by transition index (with the text they were parsed from).
    conditions: Vec<(String, Result<Condition, String>)>,
    /// Problems already logged.
    warned: HashSet<String>,
}

impl PartialEq for TreeRuntime {
    /// Playback state never makes two trees different (saving, undo and diffs compare
    /// the authored fields).
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

/// A change scripts ask of a tree (applied after their hook).
#[derive(Debug, Clone, PartialEq)]
pub enum TreeCommand {
    Set(String, f32),
    Trigger(String),
    Travel { state: String, fade: f32 },
}

impl AnimationTree {
    pub fn apply(&mut self, command: TreeCommand) {
        match command {
            TreeCommand::Set(name, value) => self.set(&name, value),
            TreeCommand::Trigger(name) => self.trigger(&name),
            TreeCommand::Travel { state, fade } => self.travel(&state, fade),
        }
    }

    /// A parameter's value: a fired trigger is 1, else the value set or fed, else the
    /// starting value, else 0.
    pub fn value(&self, name: &str) -> f32 {
        if self.runtime.triggers.contains(name) {
            return 1.0;
        }
        self.runtime
            .values
            .get(name)
            .or_else(|| self.parameters.get(name))
            .copied()
            .unwrap_or(0.0)
    }

    pub fn set(&mut self, name: &str, value: f32) {
        self.runtime.values.insert(name.to_string(), value);
    }

    pub fn trigger(&mut self, name: &str) {
        self.runtime.triggers.insert(name.to_string());
    }

    /// Go to `state` now (cross-fading over `fade`), whatever the transitions say.
    pub fn travel(&mut self, state: &str, fade: f32) {
        self.runtime.travel = Some((state.to_string(), fade.max(0.0)));
    }

    fn state_index(&self, name: &str) -> Option<usize> {
        self.states.iter().position(|state| state.name == name)
    }

    fn warn_once(&mut self, message: String) {
        if self.runtime.warned.insert(message.clone()) {
            log::warn!("Animation tree: {message}");
        }
    }

    /// Re-parse conditions whose text changed.
    fn refresh_conditions(&mut self) {
        let conditions = &mut self.runtime.conditions;
        conditions.truncate(self.transitions.len());
        for (index, transition) in self.transitions.iter().enumerate() {
            match conditions.get(index) {
                Some((text, _)) if *text == transition.when => {}
                _ => {
                    let parsed = Condition::parse(&transition.when);
                    let entry = (transition.when.clone(), parsed);
                    if index < conditions.len() {
                        conditions[index] = entry;
                    } else {
                        conditions.push(entry);
                    }
                }
            }
        }
    }

    /// Advance by `dt`: take a transition if one applies, then play the current state's
    /// blend on `animator`.
    pub fn evaluate(&mut self, dt: f32, animator: &mut Animator) {
        if self.states.is_empty() {
            return;
        }
        self.refresh_conditions();
        let mut entered: Option<f32> = None;

        let current = self
            .runtime
            .state
            .as_deref()
            .and_then(|name| self.state_index(name));
        let mut current = match current {
            Some(index) => index,
            None => {
                let start = self.state_index(&self.start).unwrap_or(0);
                entered = Some(0.0);
                start
            }
        };

        if let Some((target, fade)) = self.runtime.travel.take() {
            match self.state_index(&target) {
                Some(index) => {
                    current = index;
                    entered = Some(fade);
                }
                None => self.warn_once(format!("no state named {target:?} to travel to")),
            }
        } else if entered.is_none() {
            self.runtime.time_in_state += dt;
            let from = self.states[current].name.clone();
            let mut taken = None;
            for (index, transition) in self.transitions.iter().enumerate() {
                if transition.from != from && !(transition.from == "*" && transition.to != from) {
                    continue;
                }
                if let Some(after) = transition.after {
                    if self.runtime.progress < after.min(1.0) - 1.0e-4 {
                        continue;
                    }
                }
                let holds = match &self.runtime.conditions[index].1 {
                    Ok(condition) => condition.eval(&|name| self.value(name)),
                    Err(_) => false,
                };
                if holds {
                    taken = Some(index);
                    break;
                }
            }
            if let Some(index) = taken {
                let transition = &self.transitions[index];
                match self.state_index(&transition.to) {
                    Some(target) => {
                        entered = Some(transition.fade);
                        current = target;
                    }
                    None => {
                        let message = format!(
                            "transition from {:?} goes to unknown state {:?}",
                            transition.from, transition.to
                        );
                        self.warn_once(message);
                    }
                }
            }
            let errors: Vec<String> = self
                .runtime
                .conditions
                .iter()
                .filter_map(|(text, parsed)| {
                    parsed
                        .as_ref()
                        .err()
                        .map(|err| format!("condition {text:?}: {err}"))
                })
                .collect();
            for error in errors {
                self.warn_once(error);
            }
        }

        let name = self.states[current].name.clone();
        if let Some(fade) = entered {
            self.runtime.previous = self.runtime.state.replace(name.clone());
            self.runtime.time_in_state = 0.0;
            self.runtime.progress = 0.0;
            self.play(current, fade, true, animator);
        } else {
            self.play(current, 0.2, false, animator);
        }
        if let Some((progress, _)) = animator.blend_progress(&name, 0) {
            self.runtime.progress = progress;
        }
        self.runtime.triggers.clear();
    }

    /// Hand state `index`'s blend for the current parameters to the animator.
    fn play(&self, index: usize, fade: f32, entering: bool, animator: &mut Animator) {
        let state = &self.states[index];
        let (clips, rate) = motion_blend(&state.motion, &|name| self.value(name));
        let clips: Vec<BlendClip> = clips
            .into_iter()
            .map(|mut clip| {
                clip.speed *= rate * state.speed;
                clip
            })
            .collect();
        animator.play_blend(
            &state.name,
            &clips,
            BlendOptions {
                fade,
                looping: state.looping,
                sync: true,
                layer: 0,
                restart: entering,
            },
        );
    }
}

/// The clips of `motion` with their shares for the current parameter values, and the
/// rate to play them at.
fn motion_blend(motion: &Motion, value: &dyn Fn(&str) -> f32) -> (Vec<BlendClip>, f32) {
    let clip = |name: &str, weight: f32, speed: f32| BlendClip {
        clip: name.to_string(),
        weight,
        speed,
    };
    match motion {
        Motion::Clip { clip: name, speed } => (vec![clip(name, 1.0, *speed)], 1.0),
        Motion::Blend1d {
            parameter,
            points,
            scale_speed,
        } => {
            if points.is_empty() {
                return (Vec::new(), 1.0);
            }
            let v = value(parameter);
            let mut sorted: Vec<&BlendPoint> = points.iter().collect();
            sorted.sort_by(|a, b| a.at.total_cmp(&b.at));
            let first = sorted[0];
            let last = sorted[sorted.len() - 1];
            let all = |active: &[(usize, f32)]| -> Vec<BlendClip> {
                sorted
                    .iter()
                    .enumerate()
                    .map(|(i, point)| {
                        let weight = active
                            .iter()
                            .find(|(index, _)| *index == i)
                            .map_or(0.0, |(_, w)| *w);
                        clip(&point.clip, weight, point.speed)
                    })
                    .collect()
            };
            if v <= first.at {
                return (all(&[(0, 1.0)]), 1.0);
            }
            if v >= last.at {
                let rate = if *scale_speed && last.at.abs() > 1.0e-3 {
                    (v / last.at).clamp(0.25, 3.0)
                } else {
                    1.0
                };
                return (all(&[(sorted.len() - 1, 1.0)]), rate);
            }
            let upper = sorted.iter().position(|point| point.at > v).unwrap_or(1);
            let (a, b) = (sorted[upper - 1], sorted[upper]);
            let t = ((v - a.at) / (b.at - a.at).max(1.0e-6)).clamp(0.0, 1.0);
            (all(&[(upper - 1, 1.0 - t), (upper, t)]), 1.0)
        }
        Motion::Blend2d {
            x,
            y,
            points,
            scale_speed,
        } => {
            let at = Vec2::new(value(x), value(y));
            let positions: Vec<Vec2> = points.iter().map(|p| Vec2::from(p.at)).collect();
            let (weights, reached) = blend_2d_weights(&positions, at);
            let rate =
                if *scale_speed && reached.length() > 1.0e-3 && at.length() > reached.length() {
                    (at.length() / reached.length()).clamp(0.25, 3.0)
                } else {
                    1.0
                };
            (
                points
                    .iter()
                    .zip(weights)
                    .map(|(point, weight)| clip(&point.clip, weight, point.speed))
                    .collect(),
                rate,
            )
        }
    }
}

/// Shares of 2D blend points for position `at`: barycentric in the smallest triangle of
/// points that contains it, else along the closest edge (or at the closest point) of
/// the outline. Also returns where in the blend space that lands.
pub(crate) fn blend_2d_weights(points: &[Vec2], at: Vec2) -> (Vec<f32>, Vec2) {
    let n = points.len();
    let mut weights = vec![0.0; n];
    if n == 0 {
        return (weights, at);
    }
    if n == 1 {
        weights[0] = 1.0;
        return (weights, points[0]);
    }
    // Inside a triangle: the smallest one wins, which keeps blends local.
    let mut best: Option<(f32, [usize; 3], [f32; 3])> = None;
    for i in 0..n {
        for j in i + 1..n {
            for k in j + 1..n {
                let (a, b, c) = (points[i], points[j], points[k]);
                let area = (b - a).perp_dot(c - a);
                if area.abs() < 1.0e-6 {
                    continue;
                }
                let w1 = (b - at).perp_dot(c - at) / area;
                let w2 = (c - at).perp_dot(a - at) / area;
                let w3 = 1.0 - w1 - w2;
                let eps = -1.0e-5;
                if w1 >= eps
                    && w2 >= eps
                    && w3 >= eps
                    && best.is_none_or(|(size, _, _)| area.abs() < size)
                {
                    best = Some((
                        area.abs(),
                        [i, j, k],
                        [w1.max(0.0), w2.max(0.0), w3.max(0.0)],
                    ));
                }
            }
        }
    }
    if let Some((_, indices, shares)) = best {
        let total: f32 = shares.iter().sum();
        for (index, share) in indices.iter().zip(shares) {
            weights[*index] = share / total.max(1.0e-6);
        }
        return (weights, at);
    }
    // Outside: the closest point on any segment between two points.
    let mut closest = (f32::INFINITY, 0usize, 0usize, 0.0f32, points[0]);
    for i in 0..n {
        for j in i..n {
            let (a, b) = (points[i], points[j]);
            let ab = b - a;
            let t = if ab.length_squared() > 1.0e-9 {
                ((at - a).dot(ab) / ab.length_squared()).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let p = a + ab * t;
            let d = p.distance_squared(at);
            if d < closest.0 {
                closest = (d, i, j, t, p);
            }
        }
    }
    let (_, i, j, t, p) = closest;
    weights[i] += 1.0 - t;
    weights[j] += t;
    (weights, p)
}

// ------------------------------------------------------------------------------------
// Conditions
// ------------------------------------------------------------------------------------

/// Check a transition's `when` text: `Err` says what is wrong with it.
pub fn check_condition(text: &str) -> Result<(), String> {
    Condition::parse(text).map(|_| ())
}

#[derive(Debug, Clone, PartialEq)]
enum Condition {
    Value(String),
    Number(f32),
    Not(Box<Condition>),
    And(Box<Condition>, Box<Condition>),
    Or(Box<Condition>, Box<Condition>),
    Compare(Box<Condition>, Op, Box<Condition>),
    Always,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
}

impl Condition {
    fn parse(text: &str) -> Result<Self, String> {
        let tokens = tokenize(text)?;
        if tokens.is_empty() {
            return Ok(Condition::Always);
        }
        let mut parser = Parser { tokens, at: 0 };
        let condition = parser.or()?;
        if parser.at < parser.tokens.len() {
            return Err(format!("unexpected {:?}", parser.tokens[parser.at]));
        }
        Ok(condition)
    }

    fn number(&self, value: &dyn Fn(&str) -> f32) -> f32 {
        match self {
            Condition::Value(name) => value(name),
            Condition::Number(n) => *n,
            other => {
                if other.eval(value) {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    fn eval(&self, value: &dyn Fn(&str) -> f32) -> bool {
        match self {
            Condition::Always => true,
            Condition::Value(name) => value(name) != 0.0,
            Condition::Number(n) => *n != 0.0,
            Condition::Not(inner) => !inner.eval(value),
            Condition::And(a, b) => a.eval(value) && b.eval(value),
            Condition::Or(a, b) => a.eval(value) || b.eval(value),
            Condition::Compare(a, op, b) => {
                let (a, b) = (a.number(value), b.number(value));
                match op {
                    Op::Less => a < b,
                    Op::LessEqual => a <= b,
                    Op::Greater => a > b,
                    Op::GreaterEqual => a >= b,
                    Op::Equal => (a - b).abs() < 1.0e-6,
                    Op::NotEqual => (a - b).abs() >= 1.0e-6,
                }
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Name(String),
    Number(f32),
    Not,
    And,
    Or,
    Open,
    Close,
    Op(Op),
}

fn tokenize(text: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            ' ' | '\t' | '\n' => i += 1,
            '(' => {
                tokens.push(Token::Open);
                i += 1;
            }
            ')' => {
                tokens.push(Token::Close);
                i += 1;
            }
            '&' if next == Some('&') => {
                tokens.push(Token::And);
                i += 2;
            }
            '|' if next == Some('|') => {
                tokens.push(Token::Or);
                i += 2;
            }
            '!' if next == Some('=') => {
                tokens.push(Token::Op(Op::NotEqual));
                i += 2;
            }
            '!' => {
                tokens.push(Token::Not);
                i += 1;
            }
            '<' | '>' | '=' => {
                let equal = next == Some('=');
                let op = match (c, equal) {
                    ('<', true) => Op::LessEqual,
                    ('<', false) => Op::Less,
                    ('>', true) => Op::GreaterEqual,
                    ('>', false) => Op::Greater,
                    ('=', true) => Op::Equal,
                    _ => return Err("use == to compare".into()),
                };
                tokens.push(Token::Op(op));
                i += if equal { 2 } else { 1 };
            }
            c if c.is_ascii_digit()
                || c == '.'
                || (c == '-' && next.is_some_and(|n| n.is_ascii_digit() || n == '.')) =>
            {
                let start = i;
                i += 1;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                let number = text
                    .parse::<f32>()
                    .map_err(|_| format!("bad number {text:?}"))?;
                tokens.push(Token::Number(number));
            }
            c if c.is_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();
                tokens.push(match word.as_str() {
                    "and" => Token::And,
                    "or" => Token::Or,
                    "not" => Token::Not,
                    "true" => Token::Number(1.0),
                    "false" => Token::Number(0.0),
                    _ => Token::Name(word),
                });
            }
            other => return Err(format!("unexpected {other:?}")),
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn or(&mut self) -> Result<Condition, String> {
        let mut left = self.and()?;
        while self.peek() == Some(&Token::Or) {
            self.at += 1;
            let right = self.and()?;
            left = Condition::Or(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Condition, String> {
        let mut left = self.unary()?;
        while self.peek() == Some(&Token::And) {
            self.at += 1;
            let right = self.unary()?;
            left = Condition::And(Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Condition, String> {
        if self.peek() == Some(&Token::Not) {
            self.at += 1;
            return Ok(Condition::Not(Box::new(self.unary()?)));
        }
        let left = self.term()?;
        if let Some(Token::Op(op)) = self.peek().cloned() {
            self.at += 1;
            let right = self.term()?;
            return Ok(Condition::Compare(Box::new(left), op, Box::new(right)));
        }
        Ok(left)
    }

    fn term(&mut self) -> Result<Condition, String> {
        let token = self
            .tokens
            .get(self.at)
            .cloned()
            .ok_or("incomplete condition")?;
        self.at += 1;
        match token {
            Token::Name(name) => Ok(Condition::Value(name)),
            Token::Number(n) => Ok(Condition::Number(n)),
            Token::Open => {
                let inner = self.or()?;
                if self.peek() != Some(&Token::Close) {
                    return Err("missing )".into());
                }
                self.at += 1;
                Ok(inner)
            }
            other => Err(format!("unexpected {other:?}")),
        }
    }
}

// ------------------------------------------------------------------------------------
// Finding the animator, feeding parameters
// ------------------------------------------------------------------------------------

/// The entity whose [`Animator`] a tree on `entity` drives: itself, else the nearest
/// below it (breadth first), else the nearest above.
pub fn tree_animator(world: &World, entity: Entity) -> Option<Entity> {
    if world.satisfies::<&Animator>(entity).unwrap_or(false) {
        return Some(entity);
    }
    let mut queue = std::collections::VecDeque::new();
    if let Ok(children) = world.get::<&Children>(entity) {
        queue.extend(children.0.iter().copied());
    }
    while let Some(current) = queue.pop_front() {
        if world.satisfies::<&Animator>(current).unwrap_or(false) {
            return Some(current);
        }
        if let Ok(children) = world.get::<&Children>(current) {
            queue.extend(children.0.iter().copied());
        }
    }
    super::animator::animator_entity(world, entity)
}

/// The entity whose [`AnimationTree`] drives `entity` (a character root, its model or
/// a bone): itself, else the nearest above, else the nearest below.
pub fn tree_entity(world: &World, entity: Entity) -> Option<Entity> {
    let mut current = Some(entity);
    for _ in 0..256 {
        let Some(candidate) = current else { break };
        if world
            .satisfies::<&AnimationTree>(candidate)
            .unwrap_or(false)
        {
            return Some(candidate);
        }
        current = world.get::<&Parent>(candidate).ok().map(|parent| parent.0);
    }
    let mut queue = std::collections::VecDeque::from([entity]);
    while let Some(current) = queue.pop_front() {
        if world.satisfies::<&AnimationTree>(current).unwrap_or(false) {
            return Some(current);
        }
        if let Ok(children) = world.get::<&Children>(current) {
            queue.extend(children.0.iter().copied());
        }
    }
    None
}

/// Evaluate every tree in `world` and hand its blend to its animator. Runs before the
/// animators advance.
pub(crate) fn update_trees(world: &mut World, dt: f32) {
    let trees: Vec<Entity> = world
        .query::<&AnimationTree>()
        .iter()
        .map(|(entity, _)| entity)
        .collect();
    for entity in trees {
        let Some(target) = tree_animator(world, entity) else {
            continue;
        };
        let Ok(mut tree) = world
            .get::<&mut AnimationTree>(entity)
            .map(|mut tree| std::mem::take(&mut *tree))
        else {
            continue;
        };
        if let Ok(mut animator) = world.get::<&mut Animator>(target) {
            tree.evaluate(dt, &mut animator);
        }
        if let Ok(mut slot) = world.get::<&mut AnimationTree>(entity) {
            *slot = tree;
        }
    }
}

// ------------------------------------------------------------------------------------
// A tree from a character's clips
// ------------------------------------------------------------------------------------

/// Lower-case letters and digits only, for matching clip names.
fn key(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The first clip whose key is one of `exact`, else the first that contains one of
/// `any` and none of `not`.
fn find_clip<'a>(
    clips: &'a [ClipInfo],
    exact: &[&str],
    any: &[&str],
    not: &[&str],
) -> Option<&'a ClipInfo> {
    clips
        .iter()
        .find(|clip| exact.contains(&key(&clip.name).as_str()))
        .or_else(|| {
            clips.iter().find(|clip| {
                let k = key(&clip.name);
                any.iter().any(|word| k.contains(word)) && !not.iter().any(|word| k.contains(word))
            })
        })
}

/// The clips a locomotion tree uses, picked from a character's clips by name.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LocomotionClips {
    pub idle: Option<String>,
    pub walk: Option<String>,
    pub run: Option<String>,
    pub back: Option<String>,
    pub strafe_left: Option<String>,
    pub strafe_right: Option<String>,
    pub jump: Option<String>,
    pub fall: Option<String>,
    pub land: Option<String>,
}

impl LocomotionClips {
    /// Pick clips by the names they usually have (Mixamo's included: "Walking",
    /// "Running", "Left Strafe", "Falling Idle", ...).
    pub fn find(clips: &[ClipInfo]) -> Self {
        let name = |clip: Option<&ClipInfo>| clip.map(|clip| clip.name.clone());
        let moving = ["walk", "run", "jog", "strafe", "step"];
        let left_right = ["back", "left", "right", "strafe", "turn"];
        Self {
            idle: name(find_clip(
                clips,
                &["idle"],
                &["idle"],
                &["fall", "crouch", "jump", "swim", "combat"],
            )),
            walk: name(find_clip(
                clips,
                &["walk", "walking"],
                &["walk"],
                &[&left_right[..], &["crouch", "start", "stop"]].concat(),
            )),
            run: name(find_clip(
                clips,
                &["run", "running", "jog", "jogging"],
                &["run", "jog"],
                &[&left_right[..], &["jump", "start", "stop", "slide"]].concat(),
            )),
            back: name(find_clip(
                clips,
                &[],
                &["backward", "back"],
                &["left", "right", "flip", "fall"],
            ))
            .filter(|clip| moving.iter().any(|word| key(clip).contains(word))),
            strafe_left: name(find_clip(
                clips,
                &["strafeleft", "leftstrafe"],
                &["left"],
                &["turn", "punch", "kick"],
            ))
            .filter(|clip| moving.iter().any(|word| key(clip).contains(word))),
            strafe_right: name(find_clip(
                clips,
                &["straferight", "rightstrafe"],
                &["right"],
                &["turn", "punch", "kick"],
            ))
            .filter(|clip| moving.iter().any(|word| key(clip).contains(word))),
            jump: name(find_clip(
                clips,
                &["jump", "jumping", "jumpup"],
                &["jump"],
                &["run", "land", "fall", "down", "over"],
            )),
            fall: name(find_clip(
                clips,
                &["fall", "falling", "fallingidle", "fallidle", "fallloop"],
                &["fall"],
                &["land", "flat", "death", "dying", "back", "impact"],
            )),
            land: name(find_clip(clips, &["land", "landing"], &["land"], &[])),
        }
        .with_air_loop(clips)
    }

    /// Packs without a "fall" clip often loop the air pose as "Jump Loop" or "Jump
    /// Idle": that one falls, and the jump is the take-off ("Jump Start").
    fn with_air_loop(mut self, clips: &[ClipInfo]) -> Self {
        let in_air = |k: &str| ["loop", "air", "idle"].iter().any(|word| k.contains(word));
        if self.fall.is_none() {
            self.fall = clips
                .iter()
                .find(|clip| {
                    let k = key(&clip.name);
                    k.contains("jump") && in_air(&k)
                })
                .map(|clip| clip.name.clone());
        }
        if self.jump.is_some() && self.jump == self.fall {
            self.jump = clips
                .iter()
                .find(|clip| {
                    let k = key(&clip.name);
                    k.contains("jump")
                        && !in_air(&k)
                        && !["land", "fall", "run", "down", "over"]
                            .iter()
                            .any(|word| k.contains(word))
                })
                .map(|clip| clip.name.clone());
        }
        self
    }
}

impl AnimationTree {
    /// A locomotion tree for a character with these clips: a 2D blend space of idle,
    /// walk, run, backpedal and strafes by the velocity relative to the character's
    /// facing (points at the speeds the clips travel at, where they travel), then jump,
    /// fall and land states as far as the clips exist. Missing pieces fall back: no back
    /// clip plays the walk backwards, one strafe side plays the other backwards.
    pub fn locomotion(clips: &[ClipInfo]) -> Result<Self, String> {
        let found = LocomotionClips::find(clips);
        let speed_of = |name: &Option<String>, fallback: f32| -> f32 {
            name.as_ref()
                .and_then(|name| clips.iter().find(|clip| &clip.name == name))
                .map(|clip| clip.root_speed)
                .filter(|speed| *speed > 0.3)
                .unwrap_or(fallback)
        };
        let idle = found
            .idle
            .clone()
            .or_else(|| found.walk.clone())
            .or_else(|| clips.first().map(|clip| clip.name.clone()))
            .ok_or("the character has no clips (import or add some animations first)")?;

        let mut points = vec![BlendPoint2d {
            at: [0.0, 0.0],
            clip: idle.clone(),
            speed: 1.0,
        }];
        let walk_speed = speed_of(&found.walk, 1.4);
        let run_speed = speed_of(&found.run, 4.6);
        if let Some(walk) = &found.walk {
            points.push(BlendPoint2d {
                at: [0.0, walk_speed],
                clip: walk.clone(),
                speed: 1.0,
            });
        }
        if let Some(run) = &found.run {
            points.push(BlendPoint2d {
                at: [0.0, run_speed.max(walk_speed + 0.5)],
                clip: run.clone(),
                speed: 1.0,
            });
        }
        match (&found.back, &found.walk) {
            (Some(back), _) => points.push(BlendPoint2d {
                at: [0.0, -speed_of(&found.back, walk_speed)],
                clip: back.clone(),
                speed: 1.0,
            }),
            (None, Some(walk)) => points.push(BlendPoint2d {
                at: [0.0, -walk_speed],
                clip: walk.clone(),
                speed: -1.0,
            }),
            _ => {}
        }
        // Strafes are usually jogs; "Left Strafe Walking" and the like are walks.
        let strafe_speed = |name: &str| {
            let fallback = if key(name).contains("walk") {
                walk_speed
            } else {
                3.0
            };
            speed_of(&Some(name.to_string()), fallback)
        };
        match (&found.strafe_left, &found.strafe_right) {
            (Some(left), Some(right)) => {
                points.push(BlendPoint2d {
                    at: [-strafe_speed(left), 0.0],
                    clip: left.clone(),
                    speed: 1.0,
                });
                points.push(BlendPoint2d {
                    at: [strafe_speed(right), 0.0],
                    clip: right.clone(),
                    speed: 1.0,
                });
            }
            (Some(one), None) | (None, Some(one)) => {
                let side = if found.strafe_left.is_some() {
                    -1.0
                } else {
                    1.0
                };
                let speed = strafe_speed(one);
                points.push(BlendPoint2d {
                    at: [speed * side, 0.0],
                    clip: one.clone(),
                    speed: 1.0,
                });
                points.push(BlendPoint2d {
                    at: [-speed * side, 0.0],
                    clip: one.clone(),
                    speed: -1.0,
                });
            }
            (None, None) => {}
        }

        let mut tree = AnimationTree {
            start: "Locomotion".into(),
            ..Default::default()
        };
        tree.states.push(AnimationTreeState {
            name: "Locomotion".into(),
            motion: if points.len() > 1 {
                Motion::Blend2d {
                    x: "side_speed".into(),
                    y: "forward_speed".into(),
                    points,
                    scale_speed: true,
                }
            } else {
                Motion::Clip {
                    clip: idle,
                    speed: 1.0,
                }
            },
            looping: true,
            speed: 1.0,
        });
        let state = |name: &str, clip: &str, looping: bool| AnimationTreeState {
            name: name.into(),
            motion: Motion::Clip {
                clip: clip.into(),
                speed: 1.0,
            },
            looping,
            speed: 1.0,
        };
        let transition = |from: &str, to: &str, when: &str, fade: f32, after: Option<f32>| {
            AnimationTreeTransition {
                from: from.into(),
                to: to.into(),
                when: when.into(),
                fade,
                after,
            }
        };
        // The air: a jump clip, a falling loop, or the jump held at its end for both.
        let air = found.fall.clone().or_else(|| found.jump.clone());
        if let Some(jump) = &found.jump {
            tree.states.push(state("Jump", jump, false));
        }
        if let Some(fall) = &air {
            tree.states.push(state("Fall", fall, found.fall.is_some()));
        }
        if let Some(land) = &found.land {
            tree.states.push(state("Land", land, false));
        }
        let has = |name: &str| tree.states.iter().any(|state| state.name == name);
        let (jump, fall, land) = (has("Jump"), has("Fall"), has("Land"));
        let mut transitions = Vec::new();
        if jump {
            transitions.push(transition("*", "Jump", "jumped", 0.08, None));
        } else if fall {
            transitions.push(transition("*", "Fall", "jumped", 0.1, None));
        }
        if land {
            transitions.push(transition(
                "Fall",
                "Land",
                "landed && landing_speed > 8",
                0.06,
                None,
            ));
            if jump {
                transitions.push(transition(
                    "Jump",
                    "Land",
                    "landed && landing_speed > 8",
                    0.06,
                    None,
                ));
            }
        }
        if jump {
            transitions.push(transition("Jump", "Locomotion", "grounded", 0.15, None));
            if fall {
                transitions.push(transition("Jump", "Fall", "", 0.2, Some(0.95)));
            }
        }
        if fall {
            transitions.push(transition("Fall", "Locomotion", "grounded", 0.15, None));
            transitions.push(transition(
                "Locomotion",
                "Fall",
                "!grounded && air_time > 0.15",
                0.25,
                None,
            ));
        }
        if land {
            transitions.push(transition("Land", "Locomotion", "speed > 1", 0.15, None));
            transitions.push(transition("Land", "Locomotion", "", 0.2, Some(0.75)));
        }
        tree.transitions = transitions;
        Ok(tree)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn info(name: &str, duration: f32, root_speed: f32) -> ClipInfo {
        ClipInfo {
            name: name.into(),
            source_name: name.into(),
            library: PathBuf::new(),
            duration,
            tracks: 20,
            bound: 20,
            missing: Vec::new(),
            root_speed,
        }
    }

    #[test]
    fn conditions_parse_and_evaluate() {
        let values = |name: &str| match name {
            "grounded" => 0.0,
            "air_time" => 0.3,
            "speed" => 2.5,
            _ => 0.0,
        };
        let check = |text: &str| Condition::parse(text).unwrap().eval(&values);
        assert!(check("!grounded && air_time > 0.15"));
        assert!(check("speed >= 2.5 and not grounded"));
        assert!(!check("grounded || speed < -1"));
        assert!(check("(grounded || speed != 0) && true"));
        assert!(check(""));
        assert!(Condition::parse("speed = 2").is_err());
        assert!(Condition::parse("(speed > 2").is_err());
    }

    #[test]
    fn one_dimensional_blends_interpolate_and_scale() {
        let motion = Motion::Blend1d {
            parameter: "speed".into(),
            points: vec![
                BlendPoint {
                    at: 0.0,
                    clip: "Idle".into(),
                    speed: 1.0,
                },
                BlendPoint {
                    at: 2.0,
                    clip: "Walk".into(),
                    speed: 1.0,
                },
                BlendPoint {
                    at: 6.0,
                    clip: "Run".into(),
                    speed: 1.0,
                },
            ],
            scale_speed: true,
        };
        let (clips, rate) = motion_blend(&motion, &|_| 4.0);
        assert_eq!(rate, 1.0);
        assert!((clips[1].weight - 0.5).abs() < 1e-5 && (clips[2].weight - 0.5).abs() < 1e-5);
        let (clips, rate) = motion_blend(&motion, &|_| 9.0);
        assert_eq!(clips[2].weight, 1.0);
        assert!((rate - 1.5).abs() < 1e-5);
    }

    #[test]
    fn two_dimensional_blends_use_the_enclosing_triangle() {
        let points = [
            Vec2::ZERO,
            Vec2::new(0.0, 2.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(-2.0, 0.0),
            Vec2::new(0.0, -2.0),
        ];
        let (weights, _) = blend_2d_weights(&points, Vec2::new(1.0, 1.0));
        assert!(
            (weights[1] - 0.5).abs() < 1e-4 && (weights[2] - 0.5).abs() < 1e-4,
            "{weights:?}"
        );
        let (weights, _) = blend_2d_weights(&points, Vec2::ZERO);
        assert!((weights[0] - 1.0).abs() < 1e-4, "{weights:?}");
        // Outside: the closest edge, and how far it is reached.
        let (weights, reached) = blend_2d_weights(&points, Vec2::new(0.0, 4.0));
        assert!((weights[1] - 1.0).abs() < 1e-4, "{weights:?}");
        assert!((reached.y - 2.0).abs() < 1e-4);
    }

    #[test]
    fn mixamo_names_make_a_locomotion_tree() {
        let clips = [
            info("Breathing Idle", 4.0, 0.0),
            info("Walking", 1.1, 1.35),
            info("Running", 0.7, 4.9),
            info("Walking Backwards", 1.2, 1.2),
            info("Left Strafe", 0.8, 2.8),
            info("Right Strafe", 0.8, 2.8),
            info("Jumping", 1.0, 0.0),
            info("Falling Idle", 1.5, 0.0),
            info("Falling To Landing", 0.6, 0.0),
            info("Turn Left", 1.0, 0.0),
        ];
        let found = LocomotionClips::find(&clips);
        assert_eq!(found.idle.as_deref(), Some("Breathing Idle"));
        assert_eq!(found.walk.as_deref(), Some("Walking"));
        assert_eq!(found.run.as_deref(), Some("Running"));
        assert_eq!(found.back.as_deref(), Some("Walking Backwards"));
        assert_eq!(found.strafe_left.as_deref(), Some("Left Strafe"));
        assert_eq!(found.strafe_right.as_deref(), Some("Right Strafe"));
        assert_eq!(found.jump.as_deref(), Some("Jumping"));
        assert_eq!(found.fall.as_deref(), Some("Falling Idle"));
        assert_eq!(found.land.as_deref(), Some("Falling To Landing"));

        let tree = AnimationTree::locomotion(&clips).unwrap();
        let names: Vec<&str> = tree.states.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Locomotion", "Jump", "Fall", "Land"]);
        let Motion::Blend2d { points, .. } = &tree.states[0].motion else {
            panic!("locomotion should be a 2D blend");
        };
        // Points sit where the clips travel.
        let walk = points.iter().find(|p| p.clip == "Walking").unwrap();
        assert!((walk.at[1] - 1.35).abs() < 1e-5);
        assert_eq!(points.len(), 6);
    }

    #[test]
    fn an_air_loop_falls_and_the_take_off_jumps() {
        let clips: Vec<ClipInfo> = [
            "Idle Loop",
            "Jump Land",
            "Jump Loop",
            "Jump Start",
            "Walk Loop",
        ]
        .iter()
        .map(|name| info(name, 1.0, 0.0))
        .collect();
        let found = LocomotionClips::find(&clips);
        assert_eq!(found.jump.as_deref(), Some("Jump Start"));
        assert_eq!(found.fall.as_deref(), Some("Jump Loop"));
        assert_eq!(found.land.as_deref(), Some("Jump Land"));
    }

    #[test]
    fn missing_clips_fall_back() {
        let clips = [
            info("Idle", 3.0, 0.0),
            info("Walk", 1.0, 0.0),
            info("StrafeRight", 0.6, 0.0),
        ];
        let tree = AnimationTree::locomotion(&clips).unwrap();
        assert_eq!(tree.states.len(), 1, "no air states without air clips");
        let Motion::Blend2d { points, .. } = &tree.states[0].motion else {
            panic!("expected a 2D blend");
        };
        // The walk backwards, and the right strafe backwards for the left.
        assert!(points
            .iter()
            .any(|p| p.clip == "Walk" && p.speed < 0.0 && p.at[1] < 0.0));
        assert!(points
            .iter()
            .any(|p| p.clip == "StrafeRight" && p.speed < 0.0 && p.at[0] < 0.0));
        assert!(AnimationTree::locomotion(&[]).is_err());
    }

    #[test]
    fn transitions_follow_conditions_triggers_and_exit_times() {
        let mut tree: AnimationTree = serde_json::from_str(
            r#"{
                "start": "Idle",
                "states": [
                    { "name": "Idle", "motion": { "kind": "clip", "clip": "Idle" } },
                    { "name": "Wave", "motion": { "kind": "clip", "clip": "Wave" }, "looping": false }
                ],
                "transitions": [
                    { "from": "Idle", "to": "Wave", "when": "waving" },
                    { "from": "Wave", "to": "Idle", "after": 1.0 }
                ]
            }"#,
        )
        .unwrap();
        let mut animator = Animator::default();
        tree.evaluate(0.016, &mut animator);
        assert_eq!(tree.runtime.state.as_deref(), Some("Idle"));
        tree.trigger("waving");
        tree.evaluate(0.016, &mut animator);
        assert_eq!(tree.runtime.state.as_deref(), Some("Wave"));
        // Triggers last one evaluation.
        assert_eq!(tree.value("waving"), 0.0);
        // The wave has not played yet (the animator has no clips bound): it stays.
        tree.evaluate(0.016, &mut animator);
        assert_eq!(tree.runtime.state.as_deref(), Some("Wave"));
        tree.travel("Idle", 0.1);
        tree.evaluate(0.016, &mut animator);
        assert_eq!(tree.runtime.state.as_deref(), Some("Idle"));
        assert_eq!(tree.runtime.previous.as_deref(), Some("Wave"));
    }
}
