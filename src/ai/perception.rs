//! Senses: what an agent sees (other agents' tags within a view cone and range, with a
//! line of sight) and hears (noises made near it), and remembers for a while.
//!
//! Changes become events for its brain: `see` and `see:<tag>` when something comes into
//! view, `lose` and `lose:<tag>` when it goes out of view, `hear` and `hear:<tag>` for a
//! noise. Expressions ask `sees('thief')`, `heard('alarm')`, `count('child', 5)`,
//! `nearest('guard')`.

use std::collections::HashMap;
use std::sync::Arc;

use glam::Vec3;
use hecs::Entity;
use serde::{Deserialize, Serialize};

use super::context::{AiEvent, AiHost, LineOfSight};
use super::value::Value;

/// How an agent perceives (part of [`super::AiAgent`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PerceptionSettings {
    /// How far it sees (m); 0: blind.
    pub sight_range: f32,
    /// Its field of view (degrees, the whole cone).
    pub sight_angle: f32,
    /// Closer than this it notices things whichever way it faces (m).
    pub near_range: f32,
    /// How far it hears (m); a noise is heard within both this and the noise's radius.
    pub hearing_range: f32,
    /// How long it remembers what it no longer sees or hears (s).
    pub memory: f32,
    /// The tags it pays attention to (empty: anything with a tag).
    pub interests: Vec<String>,
    pub line_of_sight: LineOfSight,
    /// Eye height over its feet (m), for the line of sight.
    pub eye_height: f32,
    /// Looks per second.
    pub rate: f32,
}

impl Default for PerceptionSettings {
    fn default() -> Self {
        Self {
            sight_range: 0.0,
            sight_angle: 120.0,
            near_range: 1.5,
            hearing_range: 15.0,
            memory: 5.0,
            interests: Vec::new(),
            line_of_sight: LineOfSight::Navmesh,
            eye_height: 1.6,
            rate: 4.0,
        }
    }
}

impl PerceptionSettings {
    pub fn sees_anything(&self) -> bool {
        self.sight_range > 0.0
    }

    fn interested(&self, tags: &[Arc<str>]) -> bool {
        !tags.is_empty()
            && (self.interests.is_empty()
                || tags
                    .iter()
                    .any(|t| self.interests.iter().any(|i| **i == **t)))
    }
}

/// Something an agent perceives or remembers.
#[derive(Debug, Clone)]
pub struct Percept {
    pub entity: Entity,
    pub tags: Arc<[Arc<str>]>,
    /// Where it was last seen or heard.
    pub position: Vec3,
    pub visible: bool,
    pub last_seen: f64,
    pub last_heard: f64,
}

impl Percept {
    fn has_tag(&self, tag: Option<&str>) -> bool {
        tag.is_none_or(|tag| self.tags.iter().any(|t| &**t == tag))
    }
}

/// A noise an agent heard.
#[derive(Debug, Clone)]
pub struct HeardNoise {
    pub tag: Arc<str>,
    pub position: Vec3,
    pub time: f64,
    pub from: Option<Entity>,
    pub data: Value,
}

/// What an agent perceives and remembers.
#[derive(Debug, Clone, Default)]
pub struct Senses {
    pub percepts: Vec<Percept>,
    pub noises: Vec<HeardNoise>,
    /// How long things are remembered (from the settings).
    pub memory: f32,
    /// When it last looked.
    pub looked: f64,
}

impl Senses {
    /// Whether it sees something (with `tag`) now.
    pub fn sees(&self, tag: Option<&str>) -> bool {
        self.percepts.iter().any(|p| p.visible && p.has_tag(tag))
    }

    /// Whether it heard a noise (with `tag`) lately.
    pub fn heard(&self, tag: Option<&str>, now: f64) -> bool {
        self.noises
            .iter()
            .any(|n| tag.is_none_or(|tag| &*n.tag == tag) && now - n.time <= self.memory as f64)
    }

    /// How many things (with `tag`) it sees, within `radius` of `here` when given.
    pub fn count(&self, tag: Option<&str>, radius: Option<f32>, here: Vec3) -> usize {
        self.percepts
            .iter()
            .filter(|p| {
                p.visible && p.has_tag(tag) && radius.is_none_or(|r| p.position.distance(here) <= r)
            })
            .count()
    }

    /// The nearest thing (with `tag`) it sees, or remembers unless `visible_only`.
    pub fn nearest(&self, tag: Option<&str>, here: Vec3, visible_only: bool) -> Option<Entity> {
        self.candidates(tag, visible_only)
            .min_by(|a, b| {
                a.position
                    .distance_squared(here)
                    .total_cmp(&b.position.distance_squared(here))
            })
            .map(|p| p.entity)
    }

    pub fn candidates<'a>(
        &'a self,
        tag: Option<&'a str>,
        visible_only: bool,
    ) -> impl Iterator<Item = &'a Percept> + 'a {
        self.percepts
            .iter()
            .filter(move |p| p.has_tag(tag) && (p.visible || !visible_only))
    }

    pub fn percept(&self, entity: Entity) -> Option<&Percept> {
        self.percepts.iter().find(|p| p.entity == entity)
    }

    pub fn to_json(&self, now: f64) -> serde_json::Value {
        serde_json::json!({
            "percepts": self.percepts.iter().map(|p| serde_json::json!({
                "entity": p.entity.to_bits().get(),
                "tags": p.tags.iter().map(|t| t.to_string()).collect::<Vec<_>>(),
                "visible": p.visible,
                "position": [p.position.x, p.position.y, p.position.z],
                "seen_ago": (p.last_seen > f64::NEG_INFINITY).then_some(now - p.last_seen),
                "heard_ago": (p.last_heard > f64::NEG_INFINITY).then_some(now - p.last_heard),
            })).collect::<Vec<_>>(),
            "noises": self.noises.iter().map(|n| serde_json::json!({
                "tag": &*n.tag,
                "ago": now - n.time,
                "position": [n.position.x, n.position.y, n.position.z],
                "from": n.from.map(|e| e.to_bits().get()),
            })).collect::<Vec<_>>(),
        })
    }

    /// Hear a noise: remember it, and the event for the brain.
    pub fn hear(&mut self, noise: HeardNoise, events: &mut Vec<AiEvent>) {
        if let Some(from) = noise.from {
            if let Some(p) = self.percepts.iter_mut().find(|p| p.entity == from) {
                p.last_heard = noise.time;
                if !p.visible {
                    p.position = noise.position;
                }
            }
        }
        for name in ["hear".to_string(), format!("hear:{}", noise.tag)] {
            events.push(AiEvent {
                name: Arc::from(name),
                data: noise.data.clone(),
                from: noise.from,
                position: Some(noise.position),
            });
        }
        self.noises
            .retain(|n| !(n.tag == noise.tag && n.from == noise.from));
        self.noises.push(noise);
        if self.noises.len() > 16 {
            self.noises.remove(0);
        }
    }
}

/// Something that can be seen: an agent with tags.
#[derive(Debug, Clone)]
pub struct Visible {
    pub entity: Entity,
    pub position: Vec3,
    pub tags: Arc<[Arc<str>]>,
    /// Its height, for the line of sight to its middle.
    pub height: f32,
}

/// What can be seen, bucketed on a horizontal grid so an agent looks only at the
/// cells within its sight range (crowds of thousands stay cheap).
pub struct VisibleGrid {
    cell: f32,
    cells: HashMap<(i32, i32), Vec<usize>>,
    items: Vec<Visible>,
    index: super::value::EntityMap<usize>,
}

impl VisibleGrid {
    pub fn new(items: Vec<Visible>, cell: f32) -> Self {
        let cell = cell.max(1.0);
        let mut cells: HashMap<(i32, i32), Vec<usize>> = HashMap::new();
        let mut index = super::value::EntityMap::default();
        for (i, item) in items.iter().enumerate() {
            let key = (
                (item.position.x / cell).floor() as i32,
                (item.position.z / cell).floor() as i32,
            );
            cells.entry(key).or_default().push(i);
            index.insert(item.entity, i);
        }
        Self {
            cell,
            cells,
            items,
            index,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn contains(&self, entity: Entity) -> bool {
        self.index.contains_key(&entity)
    }

    /// Everything within `radius` of `center` (horizontally; a few beyond it too), in
    /// the order the grid was given them (so what agents notice does not depend on
    /// hashing).
    pub fn near<'a>(&'a self, center: Vec3, radius: f32, out: &mut Vec<&'a Visible>) {
        out.clear();
        let r = radius.max(0.0);
        let x0 = ((center.x - r) / self.cell).floor() as i32;
        let x1 = ((center.x + r) / self.cell).floor() as i32;
        let z0 = ((center.z - r) / self.cell).floor() as i32;
        let z1 = ((center.z + r) / self.cell).floor() as i32;
        let mut found: Vec<usize> = Vec::new();
        // Huge ranges over sparse grids: walk the cells there are instead.
        let span = (x1 - x0 + 1) as i64 * (z1 - z0 + 1) as i64;
        if span > self.cells.len() as i64 {
            for (&(x, z), indices) in &self.cells {
                if (x0..=x1).contains(&x) && (z0..=z1).contains(&z) {
                    found.extend(indices);
                }
            }
        } else {
            for x in x0..=x1 {
                for z in z0..=z1 {
                    if let Some(indices) = self.cells.get(&(x, z)) {
                        found.extend(indices);
                    }
                }
            }
        }
        found.sort_unstable();
        out.extend(found.into_iter().map(|i| &self.items[i]));
    }
}

/// Look around from `feet` facing `yaw`: what of `nearby` it sees now, what it lost
/// sight of (`see` / `lose` events), and forget what is gone (`exists` is false) or has
/// not been seen or heard for longer than its memory.
#[allow(clippy::too_many_arguments)]
pub fn look(
    senses: &mut Senses,
    settings: &PerceptionSettings,
    me: Entity,
    feet: Vec3,
    yaw: f32,
    nearby: &[&Visible],
    exists: &dyn Fn(Entity) -> bool,
    host: &dyn AiHost,
    now: f64,
    events: &mut Vec<AiEvent>,
) {
    senses.memory = settings.memory;
    senses.looked = now;
    let forward = Vec3::new(yaw.sin(), 0.0, yaw.cos());
    let half_angle = (settings.sight_angle.clamp(0.0, 360.0) * 0.5).to_radians();
    let cos_half = half_angle.cos();
    let mut seen_now: Vec<Entity> = Vec::new();

    for candidate in nearby {
        if candidate.entity == me || !settings.interested(&candidate.tags) {
            continue;
        }
        let to = candidate.position - feet;
        let distance = to.length();
        let mut visible = settings.sees_anything() && distance <= settings.sight_range;
        if visible && distance > settings.near_range && half_angle < std::f32::consts::PI {
            let flat = Vec3::new(to.x, 0.0, to.z).normalize_or_zero();
            visible = flat.dot(forward) >= cos_half;
        }
        if visible && settings.line_of_sight != LineOfSight::None {
            let eye_height = settings.eye_height.min(candidate.height * 0.9);
            visible = host.line_of_sight(
                feet,
                candidate.position,
                eye_height,
                settings.line_of_sight,
                &[me, candidate.entity],
            );
        }
        if !visible {
            continue;
        }
        seen_now.push(candidate.entity);
        match senses
            .percepts
            .iter_mut()
            .find(|p| p.entity == candidate.entity)
        {
            Some(p) => {
                if !p.visible {
                    push_sight(
                        events,
                        "see",
                        candidate.entity,
                        candidate.position,
                        &candidate.tags,
                    );
                }
                p.visible = true;
                p.position = candidate.position;
                p.last_seen = now;
                p.tags = candidate.tags.clone();
            }
            None => {
                senses.percepts.push(Percept {
                    entity: candidate.entity,
                    tags: candidate.tags.clone(),
                    position: candidate.position,
                    visible: true,
                    last_seen: now,
                    last_heard: f64::NEG_INFINITY,
                });
                push_sight(
                    events,
                    "see",
                    candidate.entity,
                    candidate.position,
                    &candidate.tags,
                );
            }
        }
    }

    // Out of sight now: remembered where it was last seen.
    for p in &mut senses.percepts {
        if p.visible && !seen_now.contains(&p.entity) {
            p.visible = false;
            push_sight(events, "lose", p.entity, p.position, &p.tags);
        }
    }

    // Forget what is gone, and what has not been seen or heard for a while.
    let memory = settings.memory.max(0.0) as f64;
    senses.percepts.retain(|p| {
        exists(p.entity) && (p.visible || now - p.last_seen.max(p.last_heard) <= memory)
    });
    senses.noises.retain(|n| now - n.time <= memory);
}

fn push_sight(
    events: &mut Vec<AiEvent>,
    kind: &str,
    entity: Entity,
    position: Vec3,
    tags: &[Arc<str>],
) {
    let mut push = |name: String| {
        events.push(AiEvent {
            name: Arc::from(name),
            data: Value::entity(entity),
            from: Some(entity),
            position: Some(position),
        })
    };
    push(kind.to_owned());
    for tag in tags {
        push(format!("{kind}:{tag}"));
    }
}
