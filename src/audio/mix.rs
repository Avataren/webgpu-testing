//! The project's buses as both backends drive them: names to mixer slots,
//! and the commands that make a mixer match the layout.

use std::collections::HashMap;

use cubed_mixer::{BusId, BusSettings, Command};

use super::layout::{BusLayout, MixerLayout, DEFAULT_BUS, MASTER};

/// Bus `i` of the layout is mixer slot `i`, master first.
#[derive(Default)]
pub(super) struct Buses {
    layout: MixerLayout,
    slots: HashMap<String, BusId>,
    /// `set_master_volume`, in dB, applied on top of the master fader.
    master_volume_db: f32,
}

impl Buses {
    pub(super) fn layout(&self) -> &MixerLayout {
        &self.layout
    }

    pub(super) fn slot(&self, name: &str) -> Option<BusId> {
        self.slots.get(name).copied()
    }

    /// The bus a sound plays on: the named one, else sfx (or master if the
    /// layout has no sfx bus).
    pub(super) fn for_sound(&self, name: Option<&str>) -> Result<BusId, String> {
        match name {
            Some(name) => self
                .slot(name)
                .ok_or_else(|| format!("no audio bus named \"{name}\"")),
            None => Ok(self.slot(DEFAULT_BUS).unwrap_or(BusId::MASTER)),
        }
    }

    /// Replaces the layout and returns the commands that turn a mixer
    /// running the old one into the new one.
    pub(super) fn set_layout(&mut self, layout: MixerLayout) -> Vec<Command> {
        let old_len = self.slots.len();
        self.slots = layout
            .buses
            .iter()
            .enumerate()
            .map(|(i, bus)| (bus.name.clone(), BusId(i as u16)))
            .collect();
        self.layout = layout;
        let mut commands: Vec<Command> = (self.layout.buses.len()..old_len)
            .map(|slot| Command::RemoveBus(BusId(slot as u16)))
            .collect();
        commands.extend(self.all());
        commands
    }

    /// The commands that build the whole layout in a mixer: every bus first
    /// feeding master, then routed once all exist.
    pub(super) fn all(&self) -> Vec<Command> {
        let mut commands = Vec::new();
        for (i, bus) in self.layout.buses.iter().enumerate() {
            commands.push(Command::SetBus(
                BusId(i as u16),
                BusSettings {
                    output: BusId::MASTER,
                    gain_db: self.effective_gain(bus),
                    pan: bus.pan,
                    mute: bus.mute,
                    solo: bus.solo,
                    ..BusSettings::default()
                },
            ));
        }
        for (i, bus) in self.layout.buses.iter().enumerate() {
            if let Some(output) = bus.output.as_deref().and_then(|o| self.slot(o)) {
                commands.push(Command::SetBusOutput(BusId(i as u16), output));
            }
        }
        commands
    }

    /// Changes one bus and returns the commands for the change.
    pub(super) fn update(
        &mut self,
        name: &str,
        change: impl FnOnce(&mut BusLayout),
    ) -> Result<Vec<Command>, String> {
        let bus = self
            .layout
            .bus_mut(name)
            .ok_or_else(|| format!("no audio bus named \"{name}\""))?;
        let before = bus.clone();
        change(bus);
        let after = bus.clone();
        let Some(id) = self.slot(name) else {
            return Ok(Vec::new());
        };
        let mut commands = Vec::new();
        if before.gain_db != after.gain_db {
            commands.push(Command::SetBusGain(id, self.effective_gain(&after)));
        }
        if before.pan != after.pan {
            commands.push(Command::SetBusPan(id, after.pan));
        }
        if before.mute != after.mute {
            commands.push(Command::SetBusMute(id, after.mute));
        }
        if before.solo != after.solo {
            commands.push(Command::SetBusSolo(id, after.solo));
        }
        Ok(commands)
    }

    /// Sets the scripts' master volume (linear) and returns the master
    /// fader's command.
    pub(super) fn set_master_volume(&mut self, volume: f32) -> Option<Command> {
        self.master_volume_db = if volume > 0.0 {
            20.0 * volume.log10()
        } else {
            cubed_mixer::MIN_DB * 2.0
        };
        let master = self.layout.bus(MASTER)?;
        Some(Command::SetBusGain(
            BusId::MASTER,
            self.effective_gain(master),
        ))
    }

    /// A bus's fader with the scripts' master volume on top of master's.
    fn effective_gain(&self, bus: &BusLayout) -> f32 {
        if bus.name == MASTER {
            bus.gain_db + self.master_volume_db
        } else {
            bus.gain_db
        }
    }
}
