//! The built-in leaves: what an agent does (walk somewhere, play a clip, call Lua, send
//! an event) and checks (a condition, an event, something it perceives).

use glam::Vec3;
use hecs::Entity;
use rand::Rng;

use super::context::{AiEvent, CallPhase, Cx, NavPhase, Outgoing, PlayRequest, Recipients};
use super::def::{Action, Callable, Num, Operand, Recipient, Status};
use super::value::Value;

/// A walk in progress.
#[derive(Debug, Clone)]
pub struct Mover {
    goal: Vec3,
    /// Following an entity: its position is the goal, re-planned as it moves.
    entity: Option<Entity>,
    radius: f32,
    repath: f32,
    issued_frame: u64,
    repathed: f64,
    give_up: Option<f64>,
    accept_partial: Option<f32>,
    /// The speed it had before this walk changed it.
    saved_speed: Option<f32>,
    /// Re-issued after the navigation reported it idle.
    retries: u32,
    /// Ended because it got within `radius`, not by arriving.
    within_radius: bool,
    /// Since when it has stood at the end of a path that stops short (the way shut).
    blocked_since: Option<f64>,
}

/// How long a walk waits at the end of a blocked way (a shut gate, a parked cart) before
/// it fails, so the brain can do something else.
const BLOCKED_GIVE_UP: f64 = 6.0;

/// Per-agent state of a running leaf.
#[derive(Debug, Clone, Default)]
pub enum Leaf {
    #[default]
    None,
    Until(f64),
    Move(Mover),
    Follow {
        mover: Mover,
        distance: f32,
        moving: bool,
    },
    Patrol {
        mover: Mover,
        index: usize,
        count: usize,
    },
    Play {
        clip: String,
        layer: usize,
        until: Option<f64>,
        entered_frame: u64,
        stop: bool,
        fade: f32,
    },
    Lua {
        memory: Value,
    },
}

fn status_of(value: &Value, nil: Status) -> Status {
    match value {
        Value::Nil => nil,
        Value::Bool(true) => Status::Success,
        Value::Bool(false) => Status::Failure,
        Value::Str(s) => match &**s {
            "running" => Status::Running,
            "failure" | "fail" | "failed" => Status::Failure,
            _ => Status::Success,
        },
        _ => Status::Success,
    }
}

fn flat_distance(a: Vec3, b: Vec3) -> f32 {
    Vec3::new(a.x - b.x, 0.0, a.z - b.z).length()
}

/// `{key}` in a message becomes the key's value.
fn interpolate(message: &str, cx: &mut Cx) -> String {
    let mut out = String::with_capacity(message.len());
    let mut rest = message;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        match after.find('}') {
            Some(close) => {
                let key = after[..close].trim();
                let value = match key {
                    "self" => Value::entity(cx.agent),
                    "state" => Value::str(cx.state_name().unwrap_or("")),
                    _ => cx.boards.get(key),
                };
                out.push_str(&value.to_string());
                rest = &after[close + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

fn start_move(
    cx: &mut Cx,
    goal: Vec3,
    entity: Option<Entity>,
    radius: f32,
    speed: Option<&Num>,
    run: bool,
    label: &str,
) -> Mover {
    let agent = cx.agent;
    let mut saved_speed = None;
    let wanted = match speed {
        Some(num) => cx.eval_num(num, label).map(|s| s as f32),
        None if run => Some(cx.run_speed),
        None => None,
    };
    if let Some(speed) = wanted {
        saved_speed = cx.host.agent_speed(agent);
        cx.host.set_agent_speed(agent, speed.max(0.0));
    }
    cx.host.nav_move(agent, goal);
    Mover {
        goal,
        entity,
        radius,
        repath: 0.5,
        issued_frame: cx.frame,
        repathed: cx.time,
        give_up: None,
        accept_partial: None,
        saved_speed,
        retries: 0,
        within_radius: false,
        blocked_since: None,
    }
}

fn tick_move(m: &mut Mover, cx: &mut Cx) -> Status {
    let agent = cx.agent;
    if let Some(target) = m.entity {
        let Some(p) = cx.host.position(target) else {
            return Status::Failure;
        };
        if cx.time - m.repathed >= m.repath as f64 && p.distance(m.goal) > 0.35 {
            m.goal = p;
            m.repathed = cx.time;
            m.issued_frame = cx.frame;
            cx.host.nav_move(agent, p);
        }
    }
    let here = cx.position();
    if m.radius > 0.0 && flat_distance(here, m.goal) <= m.radius && (here.y - m.goal.y).abs() < 2.0
    {
        m.within_radius = true;
        return Status::Success;
    }
    if m.give_up.is_some_and(|t| cx.time >= t) {
        return Status::Failure;
    }
    if cx.frame <= m.issued_frame {
        return Status::Running;
    }
    let Some(info) = cx.host.nav_info(agent) else {
        cx.host.warn(
            agent,
            "move: the agent has no NavAgent (or no navmesh to walk)",
        );
        return Status::Failure;
    };
    match info.phase {
        NavPhase::Pending => Status::Running,
        NavPhase::Arrived => Status::Success,
        NavPhase::Failed => Status::Failure,
        NavPhase::Moving => {
            let at_blocked_end = info.partial && info.remaining.is_some_and(|r| r <= 1.0);
            if info.partial {
                if let (Some(accept), Some(remaining)) = (m.accept_partial, info.remaining) {
                    if remaining <= accept {
                        return Status::Success;
                    }
                }
            }
            if at_blocked_end {
                let since = *m.blocked_since.get_or_insert(cx.time);
                if cx.time - since > BLOCKED_GIVE_UP {
                    return Status::Failure;
                }
            } else {
                m.blocked_since = None;
            }
            Status::Running
        }
        NavPhase::Idle => {
            if flat_distance(here, m.goal) < 0.5 {
                Status::Success
            } else if m.retries < 3 {
                m.retries += 1;
                m.issued_frame = cx.frame;
                cx.host.nav_move(agent, m.goal);
                Status::Running
            } else {
                Status::Failure
            }
        }
    }
}

fn finish_move(m: &Mover, cx: &mut Cx, status: Option<Status>) {
    let agent = cx.agent;
    if status != Some(Status::Success) || m.within_radius {
        cx.host.nav_stop(agent);
    }
    if let Some(speed) = m.saved_speed {
        cx.host.set_agent_speed(agent, speed);
    }
}

/// A random navmesh point within `radius` of `center`, at least `min_distance` from
/// the agent if it can find one.
fn pick_point(cx: &mut Cx, center: Vec3, radius: f32, min_distance: f32) -> Option<Vec3> {
    let here = cx.position();
    let mut fallback = None;
    for _ in 0..10 {
        let Some(p) = cx.host.nav_random_point(center, radius.max(0.1)) else {
            continue;
        };
        if p.distance(here) >= min_distance {
            return Some(p);
        }
        fallback.get_or_insert(p);
    }
    fallback
}

fn recipients(to: &Recipient, cx: &mut Cx, label: &str) -> Option<Recipients> {
    Some(match to {
        Recipient::Me => Recipients::One(cx.agent),
        Recipient::Group => Recipients::Group,
        Recipient::All => Recipients::All,
        Recipient::Near { radius, tag } => Recipients::Near {
            center: cx.position(),
            radius: cx.eval_num(radius, label)? as f32,
            tag: tag.clone(),
        },
        Recipient::To(operand) => {
            let value = cx.eval_operand(operand, label);
            let entity = match value.as_entity() {
                Some(entity) => entity,
                None => cx.resolve_target(&value)?.1?,
            };
            Recipients::One(entity)
        }
    })
}

fn optional_value(operand: Option<&Operand>, cx: &mut Cx, label: &str) -> Value {
    operand
        .map(|op| cx.eval_operand(op, label))
        .unwrap_or_default()
}

/// Enter a leaf; a result ends it at once.
pub fn enter(action: &Action, leaf: &mut Leaf, label: &str, cx: &mut Cx) -> Option<Status> {
    let agent = cx.agent;
    match action {
        Action::Succeed => Some(Status::Success),
        Action::Fail => Some(Status::Failure),
        Action::Running => None,
        Action::Wait { time } => {
            let seconds = cx.eval_num(time, label).unwrap_or(0.0);
            *leaf = Leaf::Until(cx.time + seconds);
            None
        }
        Action::Log { message } => {
            let text = interpolate(message, cx);
            cx.host.log(agent, &text);
            Some(Status::Success)
        }
        Action::Set { key, value } => {
            let value = cx.eval_operand(value, label);
            cx.boards.set(key, value);
            Some(Status::Success)
        }
        Action::Clear { key } => {
            cx.boards.set(key, Value::Nil);
            Some(Status::Success)
        }
        Action::Add {
            key,
            amount,
            min,
            max,
        } => {
            let amount = cx.eval_num(amount, label).unwrap_or(0.0);
            let mut value = cx.boards.get(key).as_f64().unwrap_or(0.0) + amount;
            if let Some(min) = min {
                value = value.max(*min);
            }
            if let Some(max) = max {
                value = value.min(*max);
            }
            cx.boards.set(key, Value::Num(value));
            Some(Status::Success)
        }
        Action::Check { cond } => Some(if cx.eval_cond(cond, label) {
            Status::Success
        } else {
            Status::Failure
        }),
        Action::WaitUntil { .. } => None,
        Action::Lua { run, enter, .. } => {
            let memory = cx.host.new_memory();
            let mut result = None;
            if let Some(function) = enter {
                match cx.call(function, label, CallPhase::Enter, Some(&memory), None) {
                    Ok(Value::Nil) => {}
                    Ok(value) => result = Some(status_of(&value, Status::Success)),
                    Err(err) => {
                        cx.host.warn(agent, &format!("{label}: {err}"));
                        result = Some(Status::Failure);
                    }
                }
            }
            if result.is_none() && run.is_none() {
                result = Some(Status::Success);
            }
            *leaf = Leaf::Lua { memory };
            result
        }
        Action::Send { event, to, data } => {
            let data = optional_value(data.as_ref(), cx, label);
            let Some(to) = recipients(to, cx, label) else {
                return Some(Status::Failure);
            };
            let position = cx.host.position(agent);
            cx.outbox.push(Outgoing::Event {
                to,
                event: AiEvent {
                    name: event.as_str().into(),
                    data,
                    from: Some(agent),
                    position,
                },
            });
            Some(Status::Success)
        }
        Action::Noise {
            tag,
            radius,
            data,
            at,
        } => {
            let data = optional_value(data.as_ref(), cx, label);
            let position = match at {
                Some(at) => {
                    let value = cx.eval_operand(at, label);
                    match cx.resolve_position(&value) {
                        Some(p) => p,
                        None => return Some(Status::Failure),
                    }
                }
                None => cx.position(),
            };
            let radius = cx.eval_num(radius, label).unwrap_or(10.0) as f32;
            cx.outbox.push(Outgoing::Noise {
                tag: tag.as_str().into(),
                position,
                radius,
                data,
            });
            Some(Status::Success)
        }
        Action::Emit { event, data } => {
            let data = optional_value(data.as_ref(), cx, label);
            cx.host.emit(event, &data);
            Some(Status::Success)
        }
        Action::WaitEvent { .. } => None,
        Action::MoveTo {
            target,
            radius,
            speed,
            run,
            repath,
            give_up,
            accept_partial,
        } => {
            let value = cx.eval_operand(target, label);
            let Some((goal, entity)) = cx.resolve_target(&value) else {
                return Some(Status::Failure);
            };
            if *radius > 0.0 && flat_distance(cx.position(), goal) <= *radius {
                return Some(Status::Success);
            }
            let mut mover = start_move(cx, goal, entity, *radius, speed.as_ref(), *run, label);
            mover.repath = *repath;
            mover.give_up = give_up
                .as_ref()
                .and_then(|g| cx.eval_num(g, label))
                .map(|s| cx.time + s);
            mover.accept_partial = *accept_partial;
            *leaf = Leaf::Move(mover);
            None
        }
        Action::Wander {
            center,
            radius,
            min_distance,
            speed,
            store,
        } => {
            let center = match center {
                Some(op) => {
                    let value = cx.eval_operand(op, label);
                    cx.resolve_position(&value)
                }
                None => None,
            }
            .unwrap_or_else(|| cx.position());
            let radius = cx.eval_num(radius, label).unwrap_or(8.0) as f32;
            let Some(goal) = pick_point(cx, center, radius, *min_distance) else {
                return Some(Status::Failure);
            };
            if let Some(store) = store {
                cx.boards.set(store, Value::Vec3(goal));
            }
            *leaf = Leaf::Move(start_move(
                cx,
                goal,
                None,
                0.0,
                speed.as_ref(),
                false,
                label,
            ));
            None
        }
        Action::Flee {
            from,
            distance,
            speed,
            run,
        } => {
            let value = cx.eval_operand(from, label);
            let Some(threat) = cx.resolve_position(&value) else {
                return Some(Status::Failure);
            };
            let distance = cx.eval_num(distance, label).unwrap_or(8.0) as f32;
            let here = cx.position();
            let mut away = Vec3::new(here.x - threat.x, 0.0, here.z - threat.z).normalize_or_zero();
            if away == Vec3::ZERO {
                let a = cx.rng.gen::<f32>() * std::f32::consts::TAU;
                away = Vec3::new(a.sin(), 0.0, a.cos());
            }
            let mut best: Option<(f32, Vec3)> = None;
            for degrees in [0.0f32, 35.0, -35.0, 70.0, -70.0, 110.0, -110.0] {
                let (s, c) = degrees.to_radians().sin_cos();
                let dir = Vec3::new(away.x * c - away.z * s, 0.0, away.x * s + away.z * c);
                let Some(p) = cx.host.nav_closest(here + dir * distance, distance * 0.5) else {
                    continue;
                };
                // Far from the threat, and not back past it.
                let score = flat_distance(p, threat) - degrees.abs() * 0.01;
                if best.is_none_or(|(b, _)| score > b) {
                    best = Some((score, p));
                }
            }
            let Some((_, goal)) = best else {
                return Some(Status::Failure);
            };
            *leaf = Leaf::Move(start_move(cx, goal, None, 0.0, speed.as_ref(), *run, label));
            None
        }
        Action::Follow {
            target,
            distance,
            speed,
            run,
        } => {
            let value = cx.eval_operand(target, label);
            let Some((goal, entity)) = cx.resolve_target(&value) else {
                return Some(Status::Failure);
            };
            let mut mover = start_move(cx, goal, entity, 0.0, speed.as_ref(), *run, label);
            mover.repath = 0.4;
            *leaf = Leaf::Follow {
                mover,
                distance: *distance,
                moving: true,
            };
            None
        }
        Action::Face { target, yaw, wait } => {
            let yaw = match (target, yaw) {
                (Some(target), _) => {
                    let value = cx.eval_operand(target, label);
                    let Some(p) = cx.resolve_position(&value) else {
                        return Some(Status::Failure);
                    };
                    let here = cx.position();
                    let (dx, dz) = (p.x - here.x, p.z - here.z);
                    if dx * dx + dz * dz < 1e-6 {
                        return Some(Status::Success);
                    }
                    dx.atan2(dz)
                }
                (None, Some(yaw)) => match cx.eval_num(yaw, label) {
                    Some(yaw) => yaw as f32,
                    None => return Some(Status::Failure),
                },
                (None, None) => return Some(Status::Failure),
            };
            cx.host.face(agent, yaw);
            if *wait > 0.0 {
                *leaf = Leaf::Until(cx.time + *wait as f64);
                None
            } else {
                Some(Status::Success)
            }
        }
        Action::Stop => {
            cx.host.nav_stop(agent);
            Some(Status::Success)
        }
        Action::Patrol {
            points,
            index_key,
            radius,
            speed,
        } => {
            let value = cx.eval_operand(points, label);
            let points = cx.host.positions(&value);
            if points.is_empty() {
                return Some(Status::Failure);
            }
            let stored = cx.boards.get(index_key).as_f64().unwrap_or(1.0);
            let index = ((stored.max(1.0) as usize) - 1) % points.len();
            let mover = start_move(
                cx,
                points[index],
                None,
                *radius,
                speed.as_ref(),
                false,
                label,
            );
            *leaf = Leaf::Patrol {
                mover,
                index,
                count: points.len(),
            };
            None
        }
        Action::PickPoint {
            store,
            around,
            radius,
            min_distance,
        } => {
            let center = match around {
                Some(op) => {
                    let value = cx.eval_operand(op, label);
                    cx.resolve_position(&value)
                }
                None => None,
            }
            .unwrap_or_else(|| cx.position());
            let radius = cx.eval_num(radius, label).unwrap_or(8.0) as f32;
            match pick_point(cx, center, radius, *min_distance) {
                Some(p) => {
                    cx.boards.set(store, Value::Vec3(p));
                    Some(Status::Success)
                }
                None => Some(Status::Failure),
            }
        }
        Action::Play {
            clip,
            layer,
            looping,
            fade,
            speed,
            time,
            stop_on_exit,
        } => {
            let clip = match cx.eval_operand(clip, label) {
                Value::Str(s) => s.to_string(),
                other => {
                    cx.host.warn(
                        agent,
                        &format!("{label}: the clip is {}, not a name", other.type_name()),
                    );
                    return Some(Status::Failure);
                }
            };
            let seconds = time.as_ref().and_then(|t| cx.eval_num(t, label));
            let looping = looping.unwrap_or(seconds.is_some());
            let request = PlayRequest {
                clip: clip.clone(),
                layer: *layer,
                looping,
                fade: *fade,
                speed: *speed,
            };
            if !cx.host.play(agent, &request) {
                return Some(Status::Failure);
            }
            if looping && seconds.is_none() {
                // Left playing.
                return Some(Status::Success);
            }
            *leaf = Leaf::Play {
                clip,
                layer: *layer,
                until: seconds.map(|s| cx.time + s),
                entered_frame: cx.frame,
                stop: *stop_on_exit,
                fade: *fade,
            };
            None
        }
        Action::StopAnimation { layer, fade } => {
            cx.host.stop_animation(agent, *layer, *fade);
            Some(Status::Success)
        }
        Action::AnimParam { name, value } => {
            let value = cx.eval_operand(value, label);
            cx.host.set_anim_param(agent, name, &value);
            Some(Status::Success)
        }
        Action::AnimTrigger { name } => {
            cx.host.anim_trigger(agent, name);
            Some(Status::Success)
        }
        Action::Find {
            tag,
            store,
            range,
            visible,
            filter,
        } => {
            let here = cx.position();
            let mut candidates: Vec<(f32, Entity)> = cx
                .senses
                .candidates(tag.as_deref(), *visible)
                .map(|p| (p.position.distance(here), p.entity))
                .filter(|(d, _)| range.is_none_or(|r| *d <= r))
                .collect();
            candidates.sort_by(|a, b| a.0.total_cmp(&b.0));
            let mut found = None;
            for (_, entity) in candidates {
                let keep = match filter {
                    Some(cond) => {
                        cx.it = Some(entity);
                        let ok = cx.eval_cond(cond, label);
                        cx.it = None;
                        ok
                    }
                    None => true,
                };
                if keep {
                    found = Some(entity);
                    break;
                }
            }
            match found {
                Some(entity) => {
                    cx.boards.set(store, Value::entity(entity));
                    Some(Status::Success)
                }
                None => {
                    cx.boards.set(store, Value::Nil);
                    Some(Status::Failure)
                }
            }
        }
    }
}

pub fn tick(action: &Action, leaf: &mut Leaf, label: &str, cx: &mut Cx) -> Status {
    match (action, leaf) {
        (Action::Running, _) => Status::Running,
        (_, Leaf::Until(until)) => {
            if cx.time >= *until {
                Status::Success
            } else {
                Status::Running
            }
        }
        (Action::WaitUntil { cond }, _) => {
            if cx.eval_cond(cond, label) {
                Status::Success
            } else {
                Status::Running
            }
        }
        (
            Action::WaitEvent {
                event,
                store,
                store_from,
                store_position,
            },
            _,
        ) => match cx.events.iter().find(|e| &*e.name == event.as_str()) {
            Some(found) => {
                let found = found.clone();
                store_event(cx, &found, store, store_from, store_position);
                Status::Success
            }
            None => Status::Running,
        },
        (Action::Lua { run, .. }, Leaf::Lua { memory }) => {
            let Some(run) = run else {
                return Status::Success;
            };
            let memory = memory.clone();
            match cx.call(run, label, CallPhase::Run, Some(&memory), None) {
                Ok(value) => status_of(&value, Status::Success),
                Err(err) => {
                    let agent = cx.agent;
                    cx.host.warn(agent, &format!("{label}: {err}"));
                    Status::Failure
                }
            }
        }
        (_, Leaf::Move(mover)) => tick_move(mover, cx),
        (
            _,
            Leaf::Follow {
                mover,
                distance,
                moving,
            },
        ) => {
            let agent = cx.agent;
            let Some(target) = mover.entity.and_then(|e| cx.host.position(e)) else {
                return Status::Failure;
            };
            let gap = flat_distance(cx.position(), target);
            if gap > *distance + 0.6 {
                if !*moving || cx.time - mover.repathed >= mover.repath as f64 {
                    mover.goal = target;
                    mover.repathed = cx.time;
                    mover.issued_frame = cx.frame;
                    cx.host.nav_move(agent, target);
                    *moving = true;
                }
            } else if gap <= *distance && *moving {
                cx.host.nav_stop(agent);
                *moving = false;
            }
            Status::Running
        }
        (
            _,
            Leaf::Patrol {
                mover,
                index,
                count,
            },
        ) => {
            let status = tick_move(mover, cx);
            // On to the next point when this one is reached, or cannot be for now.
            if status != Status::Running {
                if let Action::Patrol { index_key, .. } = action {
                    let next = (*index + 1) % (*count).max(1);
                    cx.boards.set(index_key, Value::Int(next as i64 + 1));
                }
            }
            status
        }
        (
            _,
            Leaf::Play {
                clip,
                layer,
                until,
                entered_frame,
                ..
            },
        ) => match until {
            Some(until) => {
                if cx.time >= *until {
                    Status::Success
                } else {
                    Status::Running
                }
            }
            None => {
                let agent = cx.agent;
                if cx.frame > *entered_frame + 1 && cx.host.animation_done(agent, *layer, clip) {
                    Status::Success
                } else {
                    Status::Running
                }
            }
        },
        _ => Status::Success,
    }
}

pub fn exit(action: &Action, leaf: &mut Leaf, label: &str, cx: &mut Cx, status: Option<Status>) {
    let agent = cx.agent;
    match std::mem::take(leaf) {
        Leaf::Move(mover) | Leaf::Patrol { mover, .. } => finish_move(&mover, cx, status),
        Leaf::Follow { mover, .. } => {
            cx.host.nav_stop(agent);
            if let Some(speed) = mover.saved_speed {
                cx.host.set_agent_speed(agent, speed);
            }
        }
        Leaf::Play {
            layer, stop, fade, ..
        } => {
            if stop {
                cx.host.stop_animation(agent, Some(layer), fade);
            }
        }
        Leaf::Lua { memory } => {
            if let Action::Lua {
                exit: Some(function),
                ..
            } = action
            {
                let result = match status {
                    Some(status) => status.name(),
                    None => "aborted",
                };
                call_exit(function, label, cx, &memory, result);
            }
        }
        Leaf::None | Leaf::Until(_) => {}
    }
}

fn call_exit(function: &Callable, label: &str, cx: &mut Cx, memory: &Value, result: &str) {
    if let Err(err) = cx.call(function, label, CallPhase::Exit, Some(memory), Some(result)) {
        let agent = cx.agent;
        cx.host.warn(agent, &format!("{label} (exit): {err}"));
    }
}

/// Put an event's data, sender and position on the blackboard.
pub fn store_event(
    cx: &mut Cx,
    event: &AiEvent,
    store: &Option<String>,
    store_from: &Option<String>,
    store_position: &Option<String>,
) {
    if let Some(key) = store {
        cx.boards.set(key, event.data.clone());
    }
    if let Some(key) = store_from {
        cx.boards
            .set(key, event.from.map(Value::entity).unwrap_or_default());
    }
    if let Some(key) = store_position {
        cx.boards
            .set(key, event.position.map(Value::Vec3).unwrap_or_default());
    }
}

/// What a running leaf is up to, for inspectors.
pub fn describe(leaf: &Leaf, now: f64) -> Option<serde_json::Value> {
    let mover_json = |m: &Mover| {
        serde_json::json!({
            "goal": [m.goal.x, m.goal.y, m.goal.z],
            "entity": m.entity.map(|e| e.to_bits().get()),
        })
    };
    Some(match leaf {
        Leaf::None | Leaf::Lua { .. } => return None,
        Leaf::Until(until) => serde_json::json!({ "time_left": (until - now).max(0.0) }),
        Leaf::Move(m) => mover_json(m),
        Leaf::Follow { mover, moving, .. } => {
            let mut json = mover_json(mover);
            json["moving"] = serde_json::json!(moving);
            json
        }
        Leaf::Patrol { mover, index, .. } => {
            let mut json = mover_json(mover);
            json["waypoint"] = serde_json::json!(index + 1);
            json
        }
        Leaf::Play { clip, until, .. } => serde_json::json!({
            "clip": clip,
            "time_left": until.map(|u| (u - now).max(0.0)),
        }),
    })
}

/// Where a running leaf is walking to, if anywhere (for debug drawing).
pub fn goal(leaf: &Leaf) -> Option<Vec3> {
    match leaf {
        Leaf::Move(m) | Leaf::Patrol { mover: m, .. } | Leaf::Follow { mover: m, .. } => {
            Some(m.goal)
        }
        _ => None,
    }
}
