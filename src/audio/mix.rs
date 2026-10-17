//! The project's buses as both backends drive them: names to mixer slots,
//! and the changes that make a mixer match the layout and the audio volumes
//! around the listener.

use std::collections::{HashMap, HashSet};

use cubed_mixer::{BusId, BusSettings, Command, Duck, Effect, EffectKind, SendSettings, MIN_DB};

use super::layout::{
    BusLayout, DuckLayout, EffectLayout, MixerLayout, DEFAULT_BUS, MASTER, MAX_EFFECTS, MAX_SENDS,
    REVERB_BUS,
};

/// The insert slot audio volumes put their low-pass in.
const VOLUME_INSERT: u8 = MAX_EFFECTS as u8;
/// The send slot audio volumes send to the reverb bus through.
const VOLUME_SEND: u8 = MAX_SENDS as u8;
/// A volume low-pass at or above this is open.
pub(super) const OPEN_HZ: f32 = 20_000.0;

/// A change to the mixer. Effects are objects the mixer cannot get as
/// bytes, so the backends build them: natively as an insert, on the web in
/// the worklet from a message.
pub(super) enum MixOp {
    Command(Command),
    /// Puts the effect in insert `slot` of `bus` (`None` empties it).
    Effect {
        bus: BusId,
        slot: u8,
        effect: Option<Effect>,
    },
}

impl From<Command> for MixOp {
    fn from(command: Command) -> Self {
        MixOp::Command(command)
    }
}

/// What the audio volumes around the listener do to one bus.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct BusAmbience {
    /// Added to the bus's fader.
    pub gain_db: f32,
    /// The linear level the bus sends to the reverb bus at (0: none).
    pub reverb: f32,
    /// The cutoff of a low-pass on the bus ([`OPEN_HZ`]: none).
    pub low_pass_hz: f32,
}

impl Default for BusAmbience {
    fn default() -> Self {
        Self {
            gain_db: 0.0,
            reverb: 0.0,
            low_pass_hz: OPEN_HZ,
        }
    }
}

/// The audio volumes' blended effect at the listener.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Ambience {
    pub buses: HashMap<String, BusAmbience>,
    /// Where the buses' reverb sends go.
    pub reverb_bus: String,
    /// The reverb's room size and damping there, over the layout's.
    pub reverb: Option<[f32; 2]>,
}

impl Default for Ambience {
    fn default() -> Self {
        Self {
            buses: HashMap::new(),
            reverb_bus: REVERB_BUS.to_string(),
            reverb: None,
        }
    }
}

impl Ambience {
    fn bus(&self, name: &str) -> BusAmbience {
        self.buses.get(name).copied().unwrap_or_default()
    }
}

/// Bus `i` of the layout is mixer slot `i`, master first.
#[derive(Default)]
pub(super) struct Buses {
    layout: MixerLayout,
    slots: HashMap<String, BusId>,
    /// `set_master_volume`, in dB, applied on top of the master fader.
    master_volume_db: f32,
    ambience: Ambience,
    /// Buses whose volume low-pass is in the mixer.
    low_passes: HashSet<String>,
}

impl Buses {
    pub(super) fn layout(&self) -> &MixerLayout {
        &self.layout
    }

    /// The snapshots, which the mixer only hears through the ambience.
    pub(super) fn snapshots_mut(&mut self) -> &mut Vec<super::layout::SnapshotLayout> {
        &mut self.layout.snapshots
    }

    pub(super) fn slot(&self, name: &str) -> Option<BusId> {
        self.slots.get(name).copied()
    }

    /// The bus a sound plays on: the named one, else `fallback` (or master
    /// if the layout has no such bus).
    pub(super) fn for_sound_or(&self, name: Option<&str>, fallback: &str) -> Result<BusId, String> {
        match name {
            Some(name) => self
                .slot(name)
                .ok_or_else(|| format!("no audio bus named \"{name}\"")),
            None => Ok(self.slot(fallback).unwrap_or(BusId::MASTER)),
        }
    }

    /// The bus a sound plays on: the named one, else sfx.
    pub(super) fn for_sound(&self, name: Option<&str>) -> Result<BusId, String> {
        self.for_sound_or(name, DEFAULT_BUS)
    }

    /// Replaces the layout and returns the changes that turn a mixer running
    /// the old one into the new one.
    pub(super) fn set_layout(&mut self, layout: MixerLayout) -> Vec<MixOp> {
        let old_len = self.slots.len();
        self.slots = layout
            .buses
            .iter()
            .enumerate()
            .map(|(i, bus)| (bus.name.clone(), BusId(i as u16)))
            .collect();
        self.layout = layout;
        let mut ops: Vec<MixOp> = (self.layout.buses.len()..old_len)
            .map(|slot| Command::RemoveBus(BusId(slot as u16)).into())
            .collect();
        ops.extend(self.all());
        ops
    }

    /// The changes that build the whole mix in a mixer: every bus first
    /// feeding master, then routed, ducked, its effects and sends set, once
    /// all exist.
    pub(super) fn all(&mut self) -> Vec<MixOp> {
        let mut ops = Vec::new();
        for (i, bus) in self.layout.buses.iter().enumerate() {
            let settings = BusSettings {
                output: BusId::MASTER,
                duck: None,
                ..self.settings(bus)
            };
            ops.push(Command::SetBus(BusId(i as u16), settings).into());
        }
        for (i, bus) in self.layout.buses.iter().enumerate() {
            let id = BusId(i as u16);
            ops.push(Command::SetBus(id, self.settings(bus)).into());
            for slot in 0..MAX_EFFECTS {
                ops.extend(effect_ops(id, slot as u8, bus.effects.get(slot)));
            }
            for slot in 0..MAX_SENDS {
                ops.push(
                    self.send_command(id, slot as u8, bus.sends.get(slot))
                        .into(),
                );
            }
        }
        // The volumes' parts, as they are now.
        self.low_passes.clear();
        let applied = std::mem::take(&mut self.ambience);
        let mut volume_ops = self.set_ambience(applied);
        ops.append(&mut volume_ops);
        ops
    }

    /// Changes one bus and returns the changes for the mixer.
    pub(super) fn update(
        &mut self,
        name: &str,
        change: impl FnOnce(&mut BusLayout),
    ) -> Result<Vec<MixOp>, String> {
        let bus = self
            .layout
            .bus_mut(name)
            .ok_or_else(|| format!("no audio bus named \"{name}\""))?;
        let before = bus.clone();
        change(bus);
        bus.effects.truncate(MAX_EFFECTS);
        bus.sends.truncate(MAX_SENDS);
        let after = bus.clone();
        let Some(id) = self.slot(name) else {
            return Ok(Vec::new());
        };
        let mut ops: Vec<MixOp> = Vec::new();
        if before.output != after.output || before.duck != after.duck {
            // A whole SetBus; the gain and pan glide from where they are.
            ops.push(Command::SetBus(id, self.settings(&after)).into());
        } else {
            if before.gain_db != after.gain_db {
                ops.push(Command::SetBusGain(id, self.effective_gain(&after)).into());
            }
            if before.pan != after.pan {
                ops.push(Command::SetBusPan(id, after.pan).into());
            }
            if before.mute != after.mute {
                ops.push(Command::SetBusMute(id, after.mute).into());
            }
            if before.solo != after.solo {
                ops.push(Command::SetBusSolo(id, after.solo).into());
            }
        }
        for slot in 0..MAX_EFFECTS {
            let (old, new) = (before.effects.get(slot), after.effects.get(slot));
            if old != new {
                ops.extend(effect_change(id, slot as u8, old, new));
            }
        }
        for slot in 0..MAX_SENDS {
            let (old, new) = (before.sends.get(slot), after.sends.get(slot));
            if old != new {
                ops.push(self.send_command(id, slot as u8, new).into());
            }
        }
        Ok(ops)
    }

    /// Sets the scripts' master volume (linear) and returns the master
    /// fader's command.
    pub(super) fn set_master_volume(&mut self, volume: f32) -> Option<MixOp> {
        self.master_volume_db = if volume > 0.0 {
            20.0 * volume.log10()
        } else {
            MIN_DB * 2.0
        };
        let master = self.layout.bus(MASTER)?;
        Some(Command::SetBusGain(BusId::MASTER, self.effective_gain(master)).into())
    }

    /// Applies the audio volumes' effect at the listener and returns the
    /// changes from the last one.
    pub(super) fn set_ambience(&mut self, ambience: Ambience) -> Vec<MixOp> {
        let old = std::mem::replace(&mut self.ambience, ambience);
        let mut ops: Vec<MixOp> = Vec::new();
        let reverb_slot = self.slot(&self.ambience.reverb_bus);
        let retargeted = old.reverb_bus != self.ambience.reverb_bus;
        for (i, bus) in self.layout.buses.iter().enumerate() {
            let id = BusId(i as u16);
            let (was, now) = (old.bus(&bus.name), self.ambience.bus(&bus.name));
            if (was.gain_db - now.gain_db).abs() > 0.01 {
                ops.push(Command::SetBusGain(id, self.effective_gain(bus)).into());
            }
            let send_changed = retargeted || (was.reverb - now.reverb).abs() > 1e-4;
            if send_changed && Some(id) != reverb_slot {
                // Kept at silence rather than removed, so it glides back up.
                let send = reverb_slot
                    .filter(|_| now.reverb > 0.0 || was.reverb > 0.0)
                    .map(|to| SendSettings {
                        to,
                        level_db: to_db(now.reverb),
                        pre_fader: false,
                    });
                if send.is_some() || retargeted {
                    ops.push(
                        Command::SetSend {
                            bus: id,
                            slot: VOLUME_SEND,
                            send,
                        }
                        .into(),
                    );
                }
            }
            if now.low_pass_hz < OPEN_HZ && !self.low_passes.contains(&bus.name) {
                // Built open, then swept down: the cutoff glides.
                let mut effect = Effect::new(EffectKind::LowPass);
                effect.set("cutoff_hz", OPEN_HZ);
                ops.push(MixOp::Effect {
                    bus: id,
                    slot: VOLUME_INSERT,
                    effect: Some(effect),
                });
                self.low_passes.insert(bus.name.clone());
                ops.push(cutoff_command(id, now.low_pass_hz));
            } else if self.low_passes.contains(&bus.name)
                && (was.low_pass_hz / now.low_pass_hz - 1.0).abs() > 1e-3
            {
                ops.push(cutoff_command(id, now.low_pass_hz));
            }
        }
        if old.reverb != self.ambience.reverb || retargeted {
            ops.extend(self.reverb_params());
        }
        ops
    }

    /// The reverb bus's room size and damping: the volumes' where they set
    /// them, else the layout's.
    fn reverb_params(&self) -> Vec<MixOp> {
        let name = &self.ambience.reverb_bus;
        let (Some(id), Some(bus)) = (self.slot(name), self.layout.bus(name)) else {
            return Vec::new();
        };
        let Some((slot, effect)) = bus.effects.iter().enumerate().find_map(|(i, e)| {
            e.effect()
                .filter(|e| e.kind == EffectKind::Reverb)
                .map(|e| (i, e))
        }) else {
            return Vec::new();
        };
        let values = self
            .ambience
            .reverb
            .unwrap_or([effect.params[0], effect.params[1]]);
        (0..2)
            .map(|param| {
                Command::SetInsertParam {
                    bus: id,
                    slot: slot as u8,
                    param: param as u8,
                    value: values[param],
                }
                .into()
            })
            .collect()
    }

    /// A bus's mixer settings, with its output and duck key as slots.
    fn settings(&self, bus: &BusLayout) -> BusSettings {
        BusSettings {
            output: bus
                .output
                .as_deref()
                .and_then(|o| self.slot(o))
                .unwrap_or(BusId::MASTER),
            gain_db: self.effective_gain(bus),
            pan: bus.pan,
            mute: bus.mute,
            solo: bus.solo,
            duck: bus.duck.as_ref().and_then(|d| self.duck(d)),
            ..BusSettings::default()
        }
    }

    fn duck(&self, duck: &DuckLayout) -> Option<Duck> {
        Some(Duck {
            key: self.slot(&duck.key)?,
            depth_db: duck.depth_db.max(0.0),
            threshold_db: duck.threshold_db,
            attack_ms: duck.attack_ms.max(0.1),
            release_ms: duck.release_ms.max(0.1),
        })
    }

    fn send_command(
        &self,
        bus: BusId,
        slot: u8,
        send: Option<&super::layout::SendLayout>,
    ) -> Command {
        Command::SetSend {
            bus,
            slot,
            send: send.and_then(|s| {
                Some(SendSettings {
                    to: self.slot(&s.to)?,
                    level_db: s.level_db,
                    pre_fader: s.pre_fader,
                })
            }),
        }
    }

    /// A bus's fader with the scripts' master volume on top of master's and
    /// the audio volumes' offset on top of the bus's own.
    fn effective_gain(&self, bus: &BusLayout) -> f32 {
        let volume = self.ambience.bus(&bus.name).gain_db;
        if bus.name == MASTER {
            bus.gain_db + self.master_volume_db + volume
        } else {
            bus.gain_db + volume
        }
    }
}

fn to_db(gain: f32) -> f32 {
    if gain > 0.0 {
        (20.0 * gain.log10()).max(MIN_DB - 1.0)
    } else {
        MIN_DB - 1.0
    }
}

fn cutoff_command(bus: BusId, hz: f32) -> MixOp {
    Command::SetInsertParam {
        bus,
        slot: VOLUME_INSERT,
        param: 0,
        value: hz.min(OPEN_HZ),
    }
    .into()
}

/// The changes that put `effect` (or nothing) in an insert slot.
fn effect_ops(bus: BusId, slot: u8, effect: Option<&EffectLayout>) -> Vec<MixOp> {
    let mut ops = vec![MixOp::Effect {
        bus,
        slot,
        effect: effect.and_then(EffectLayout::effect),
    }];
    if effect.is_some_and(|e| e.bypass) {
        ops.push(
            Command::SetInsertBypass {
                bus,
                slot,
                bypass: true,
            }
            .into(),
        );
    }
    ops
}

/// The changes from one effect to another in a running slot: the same kind
/// keeps running with new parameters (so a reverb's tail goes on), anything
/// else is rebuilt.
fn effect_change(
    bus: BusId,
    slot: u8,
    old: Option<&EffectLayout>,
    new: Option<&EffectLayout>,
) -> Vec<MixOp> {
    let (Some(old), Some(new)) = (old, new) else {
        return effect_ops(bus, slot, new);
    };
    let (Some(before), Some(after)) = (old.effect(), new.effect()) else {
        return effect_ops(bus, slot, Some(new));
    };
    if before.kind != after.kind {
        return effect_ops(bus, slot, Some(new));
    }
    let mut ops: Vec<MixOp> = Vec::new();
    for (param, (a, b)) in before.params.iter().zip(&after.params).enumerate() {
        if a != b {
            ops.push(
                Command::SetInsertParam {
                    bus,
                    slot,
                    param: param as u8,
                    value: *b,
                }
                .into(),
            );
        }
    }
    if old.bypass != new.bypass {
        ops.push(
            Command::SetInsertBypass {
                bus,
                slot,
                bypass: new.bypass,
            }
            .into(),
        );
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::layout::{SendLayout, MUSIC_BUS};

    fn commands(ops: &[MixOp]) -> Vec<String> {
        ops.iter()
            .map(|op| match op {
                MixOp::Command(c) => format!("{c:?}"),
                MixOp::Effect { bus, slot, effect } => {
                    format!("Effect({bus:?}, {slot}, {:?})", effect.map(|e| e.kind))
                }
            })
            .collect()
    }

    #[test]
    fn a_parameter_edit_keeps_the_effect_running() {
        let mut buses = Buses::default();
        let mut layout = MixerLayout::default();
        layout
            .bus_mut(MUSIC_BUS)
            .unwrap()
            .effects
            .push(EffectLayout::new(EffectKind::Delay));
        buses.set_layout(layout);
        let ops = buses
            .update(MUSIC_BUS, |bus| {
                bus.effects[0].params.insert("feedback".into(), 0.5);
            })
            .unwrap();
        let text = commands(&ops);
        assert_eq!(text.len(), 1, "{text:?}");
        assert!(text[0].starts_with("SetInsertParam"), "{text:?}");

        let ops = buses
            .update(MUSIC_BUS, |bus| {
                bus.effects[0] = EffectLayout::new(EffectKind::Chorus);
                bus.sends.push(SendLayout {
                    to: REVERB_BUS.into(),
                    level_db: -6.0,
                    pre_fader: false,
                });
                bus.duck = Some(DuckLayout::default());
            })
            .unwrap();
        let text = commands(&ops);
        assert!(text.iter().any(|t| t.starts_with("SetBus(")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("Some(Chorus)")), "{text:?}");
        assert!(text.iter().any(|t| t.starts_with("SetSend")), "{text:?}");
    }

    #[test]
    fn volumes_add_a_reverb_send_a_low_pass_and_a_fader_offset() {
        let mut buses = Buses::default();
        buses.set_layout(MixerLayout::default());
        let mut ambience = Ambience::default();
        ambience.buses.insert(
            DEFAULT_BUS.into(),
            BusAmbience {
                gain_db: -3.0,
                reverb: 0.5,
                low_pass_hz: 2_000.0,
            },
        );
        ambience.reverb = Some([0.9, 0.2]);
        let text = commands(&buses.set_ambience(ambience.clone()));
        assert!(text.iter().any(|t| t.contains("SetBusGain")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("slot: 3")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("Some(LowPass)")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("value: 2000")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("value: 0.9")), "{text:?}");
        // The same again changes nothing.
        assert!(buses.set_ambience(ambience).is_empty());
        // Leaving: the send goes silent, the filter opens, the reverb is the
        // layout's again; the low-pass is not built twice.
        let text = commands(&buses.set_ambience(Ambience::default()));
        assert!(!text.iter().any(|t| t.starts_with("Effect")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("level_db: -81")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("value: 20000")), "{text:?}");
        assert!(text.iter().any(|t| t.contains("value: 0.8")), "{text:?}");
    }
}
