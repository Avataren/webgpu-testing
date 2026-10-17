//! Remote control of the mix: a bus's fader, effects, sends and ducking,
//! music with crossfades, and fades.

use serde_json::{json, Value};

use super::super::json::{opt_bool, opt_f64, opt_str, opt_u64, req_str};
use super::RemoteResult;
use crate::audio::{DuckLayout, EffectLayout, SendLayout, MAX_EFFECTS, MAX_SENDS};

/// `audio.bus {bus, gain_db?, pan?, mute?, solo?, output?, effects?, effect?, sends?,
/// duck?, save?}`
pub(super) fn bus(params: &Value) -> RemoteResult {
    let name = req_str(params, "bus")?;
    let gain_db = opt_f64(params, "gain_db").map(|v| v as f32);
    let pan = opt_f64(params, "pan").map(|v| (v as f32).clamp(-1.0, 1.0));
    let (mute, solo) = (opt_bool(params, "mute"), opt_bool(params, "solo"));
    let output = match params.get("output") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(s)) if s.is_empty() || s == crate::audio::MASTER => Some(None),
        Some(Value::String(s)) => Some(Some(s.clone())),
        Some(other) => return Err(format!("output must be a bus name or null, not {other}")),
    };
    let effects = params.get("effects").map(effects_from_json).transpose()?;
    let effect = params.get("effect").map(effect_patch).transpose()?;
    let sends = params.get("sends").map(sends_from_json).transpose()?;
    let duck = params.get("duck").map(duck_from_json).transpose()?;
    let names: Vec<String> = crate::audio::buses()
        .into_iter()
        .map(|b| b.layout.name)
        .collect();
    let known = |bus: &str, what: &str| {
        if names.iter().any(|n| n == bus) {
            Ok(())
        } else {
            Err(format!("{what}: no audio bus named \"{bus}\""))
        }
    };
    if let Some(Some(output)) = &output {
        known(output, "output")?;
    }
    for send in sends.iter().flatten() {
        known(&send.to, "sends")?;
    }
    if let Some(Some(duck)) = &duck {
        known(&duck.key, "duck.key")?;
    }
    let mut patch_error = None;
    crate::audio::update_bus(name, |bus| {
        if let Some(gain_db) = gain_db {
            bus.gain_db = gain_db.clamp(cubed_mixer::MIN_DB, 24.0);
        }
        if let Some(pan) = pan {
            bus.pan = pan;
        }
        if let Some(mute) = mute {
            bus.mute = mute;
        }
        if let Some(solo) = solo {
            bus.solo = solo;
        }
        if let Some(output) = output {
            bus.output = output;
        }
        if let Some(effects) = effects {
            bus.effects = effects;
        }
        if let Some((slot, bypass, params)) = effect {
            let count = bus.effects.len();
            match bus.effects.get_mut(slot) {
                Some(existing) => {
                    // Checked on a copy, so a bad name changes nothing.
                    let mut changed = existing.clone();
                    if let Some(bypass) = bypass {
                        changed.bypass = bypass;
                    }
                    match set_params(&mut changed, params.as_ref()) {
                        Ok(()) => *existing = changed,
                        Err(err) => patch_error = Some(format!("effect: {err}")),
                    }
                }
                None => {
                    patch_error = Some(format!(
                        "effect: the bus has {count} effects, no slot {slot}"
                    ))
                }
            }
        }
        if let Some(sends) = sends {
            bus.sends = sends;
        }
        if let Some(duck) = duck {
            bus.duck = duck;
        }
    })?;
    if let Some(err) = patch_error {
        return Err(err);
    }
    let saved = if opt_bool(params, "save").unwrap_or(false) {
        Some(crate::audio::save_layout()?.display().to_string())
    } else {
        None
    };
    Ok(json!({ "saved": saved, "mixer": crate::audio::status() }))
}

fn effect_from_json(value: &Value) -> Result<EffectLayout, String> {
    let kind_name = value
        .get("kind")
        .and_then(Value::as_str)
        .ok_or("each effect needs a kind")?;
    let kind = cubed_mixer::EffectKind::from_name(kind_name)
        .ok_or_else(|| format!("unknown effect {kind_name:?}; audio.effect_kinds lists them"))?;
    let mut effect = EffectLayout::new(kind);
    effect.bypass = value
        .get("bypass")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    set_params(&mut effect, value.get("params"))?;
    Ok(effect)
}

/// Sets `params` (`{name: value}`) on an effect, checking the names.
fn set_params(effect: &mut EffectLayout, params: Option<&Value>) -> Result<(), String> {
    let Some(params) = params else {
        return Ok(());
    };
    let object = params.as_object().ok_or("params must be an object")?;
    let kind = cubed_mixer::EffectKind::from_name(&effect.kind).ok_or("unknown effect")?;
    for (name, value) in object {
        let info = kind
            .params()
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| {
                let names: Vec<_> = kind.params().iter().map(|p| p.name).collect();
                format!(
                    "{} has no parameter {name:?} (it has {names:?})",
                    effect.kind
                )
            })?;
        let value = value
            .as_f64()
            .ok_or_else(|| format!("{name} must be a number"))? as f32;
        effect
            .params
            .insert(name.clone(), value.clamp(info.min, info.max));
    }
    Ok(())
}

fn effects_from_json(value: &Value) -> Result<Vec<EffectLayout>, String> {
    let list = value.as_array().ok_or("effects must be an array")?;
    if list.len() > MAX_EFFECTS {
        return Err(format!("a bus has at most {MAX_EFFECTS} effects"));
    }
    list.iter().map(effect_from_json).collect()
}

/// `effect: {slot, params?, bypass?}`: a change to one effect where it is (its
/// parameter names are checked once the effect is known).
fn effect_patch(value: &Value) -> Result<(usize, Option<bool>, Option<Value>), String> {
    let slot = value
        .get("slot")
        .and_then(Value::as_u64)
        .ok_or("effect needs a slot (0 is the first effect)")? as usize;
    Ok((
        slot,
        value.get("bypass").and_then(Value::as_bool),
        value.get("params").cloned(),
    ))
}

fn sends_from_json(value: &Value) -> Result<Vec<SendLayout>, String> {
    let list = value.as_array().ok_or("sends must be an array")?;
    if list.len() > MAX_SENDS {
        return Err(format!("a bus has at most {MAX_SENDS} sends"));
    }
    list.iter()
        .map(|send| {
            Ok(SendLayout {
                to: send
                    .get("to")
                    .and_then(Value::as_str)
                    .ok_or("each send needs a to (a bus name)")?
                    .to_string(),
                level_db: send.get("level_db").and_then(Value::as_f64).unwrap_or(0.0) as f32,
                pre_fader: send
                    .get("pre_fader")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            })
        })
        .collect()
}

fn duck_from_json(value: &Value) -> Result<Option<DuckLayout>, String> {
    if value.is_null() {
        return Ok(None);
    }
    let defaults = serde_json::to_value(DuckLayout::default()).map_err(|e| e.to_string())?;
    let duck = crate::remote::json::patch_serde(&DuckLayout::default(), value)
        .map_err(|err| format!("duck: {err} (fields: {defaults})"))?;
    Ok(Some(duck))
}

/// `audio.effect_kinds`: every built-in effect with its parameters.
pub(super) fn effect_kinds() -> RemoteResult {
    let kinds: Vec<Value> = cubed_mixer::EffectKind::ALL
        .iter()
        .map(|kind| {
            json!({
                "kind": kind.name(),
                "params": kind.params().iter().map(|p| json!({
                    "name": p.name,
                    "min": p.min,
                    "max": p.max,
                    "default": p.default,
                    "unit": p.unit,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(json!({ "effects": kinds, "max_effects": MAX_EFFECTS, "max_sends": MAX_SENDS }))
}

/// `audio.music {path, volume?, bus?, crossfade?, loop?, subsong?}` plays a track;
/// `{stop: true, fade?}` fades it out; `{}` tells what is playing.
pub(super) fn music(params: &Value) -> RemoteResult {
    if opt_bool(params, "stop").unwrap_or(false) {
        let fade = opt_f64(params, "fade").unwrap_or(1.0) as f32;
        return Ok(json!({ "stopped": crate::audio::stop_music(fade) }));
    }
    let Some(path) = opt_str(params, "path") else {
        return Ok(json!({ "music": music_json() }));
    };
    let defaults = crate::audio::MusicOptions::default();
    let options = crate::audio::MusicOptions {
        volume: opt_f64(params, "volume").map_or(defaults.volume, |v| v.max(0.0) as f32),
        bus: opt_str(params, "bus").map(str::to_string),
        crossfade: opt_f64(params, "crossfade").map_or(defaults.crossfade, |v| v as f32),
        looped: opt_bool(params, "loop").unwrap_or(defaults.looped),
        subsong: opt_u64(params, "subsong").unwrap_or(0) as usize,
    };
    let since = crate::audio::next_id();
    let id = crate::audio::play_music(path, options)?;
    Ok(json!({ "id": id, "played": crate::audio::history(since)[0].clone() }))
}

pub(crate) fn music_json() -> Value {
    match crate::audio::music() {
        Some((id, path)) => {
            let project = crate::project::active_project_root();
            let path = project
                .as_ref()
                .and_then(|root| path.strip_prefix(root).ok())
                .unwrap_or(&path);
            json!({ "id": id, "path": path.display().to_string() })
        }
        None => Value::Null,
    }
}

/// `audio.snapshot {name?, fade?, define?, remove?, save?}`: fades to a snapshot
/// (`name: null` fades back to none), adds or replaces one (`define: {name, buses,
/// reverb_room_size, reverb_damping}`) or removes one; returns the snapshots.
pub(super) fn snapshot(params: &Value) -> RemoteResult {
    if let Some(define) = params.get("define") {
        let snapshot: crate::audio::SnapshotLayout =
            crate::remote::json::patch_serde(&crate::audio::SnapshotLayout::default(), define)
                .map_err(|err| format!("define: {err}"))?;
        if snapshot.name.is_empty() {
            return Err("define needs a name".into());
        }
        crate::audio::update_snapshots(|list| {
            match list.iter_mut().find(|s| s.name == snapshot.name) {
                Some(existing) => *existing = snapshot,
                None => list.push(snapshot),
            }
        });
    }
    if let Some(name) = opt_str(params, "remove") {
        let removed = crate::audio::update_snapshots(|list| {
            let before = list.len();
            list.retain(|s| s.name != name);
            before != list.len()
        });
        if !removed {
            return Err(format!("no audio snapshot named \"{name}\""));
        }
    }
    let fade = opt_f64(params, "fade").unwrap_or(0.5) as f32;
    match params.get("name") {
        Some(Value::Null) => crate::audio::set_snapshot(None, fade)?,
        Some(Value::String(name)) => crate::audio::set_snapshot(Some(name), fade)?,
        Some(other) => return Err(format!("name must be a snapshot name or null, not {other}")),
        None => {}
    }
    let saved = if opt_bool(params, "save").unwrap_or(false) {
        Some(crate::audio::save_layout()?.display().to_string())
    } else {
        None
    };
    let snapshots = crate::audio::update_snapshots(|list| list.clone());
    Ok(json!({
        "snapshots": serde_json::to_value(snapshots).map_err(|e| e.to_string())?,
        "active": crate::audio::snapshot(),
        "saved": saved,
    }))
}

/// `audio.fade {id, volume, seconds?, stop?}`: glides a sound's volume; `stop: true`
/// fades it out and stops it.
pub(super) fn fade(params: &Value) -> RemoteResult {
    let id = opt_u64(params, "id").ok_or("audio.fade needs the sound's id")?;
    let seconds = opt_f64(params, "seconds").unwrap_or(1.0) as f32;
    let playing = if opt_bool(params, "stop").unwrap_or(false) {
        crate::audio::stop_after(id, seconds)
    } else {
        let volume = opt_f64(params, "volume").ok_or("audio.fade needs a volume")? as f32;
        crate::audio::fade(id, volume, seconds)
    };
    Ok(json!({ "playing": playing }))
}
