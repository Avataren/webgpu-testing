//! A project's mixer buses, saved as `content/audio/mixer.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use cubed_mixer::{Effect, EffectKind};
use serde::{Deserialize, Serialize};

/// Where a project keeps its bus layout, relative to the project root.
pub const LAYOUT_FILE: &str = "content/audio/mixer.json";

/// The bus every mix ends in.
pub const MASTER: &str = "master";
/// Where sounds play unless they name another bus.
pub const DEFAULT_BUS: &str = "sfx";
/// Where music plays unless it names another bus.
pub const MUSIC_BUS: &str = "music";
/// The reverb return that audio volumes send to unless they name another.
pub const REVERB_BUS: &str = "reverb";

/// Effects a bus's layout can have: the mixer's last insert slot is kept for
/// audio volumes' low-pass.
pub const MAX_EFFECTS: usize = cubed_mixer::MAX_INSERTS - 1;
/// Sends a bus's layout can have: the mixer's last send slot is kept for
/// audio volumes' reverb send.
pub const MAX_SENDS: usize = cubed_mixer::MAX_SENDS - 1;

/// An effect in a bus's insert chain. `params` holds the values that differ
/// from the effect's defaults (all of them, once edited in the mixer).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct EffectLayout {
    /// `reverb`, `delay`, `low_pass`, `high_pass`, `compressor`, `limiter`,
    /// `chorus`, `saturation` or `bitcrusher`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bypass: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, f32>,
}

impl EffectLayout {
    pub fn new(kind: EffectKind) -> Self {
        Self {
            kind: kind.name().to_string(),
            bypass: false,
            params: BTreeMap::new(),
        }
    }

    /// The mixer's effect: its defaults with the saved values over them.
    /// `None` for an unknown kind.
    pub fn effect(&self) -> Option<Effect> {
        let mut effect = Effect::new(EffectKind::from_name(&self.kind)?);
        for (name, value) in &self.params {
            effect.set(name, *value);
        }
        Some(effect)
    }
}

/// A send from a bus to another (a reverb or delay return).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SendLayout {
    pub to: String,
    #[serde(default)]
    pub level_db: f32,
    /// Taken before the bus's fader instead of after it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pre_fader: bool,
}

/// The bus goes down by `depth_db` while the `key` bus is louder than
/// `threshold_db` (music under effects or dialogue).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DuckLayout {
    pub key: String,
    pub depth_db: f32,
    pub threshold_db: f32,
    pub attack_ms: f32,
    pub release_ms: f32,
}

impl Default for DuckLayout {
    fn default() -> Self {
        let duck = cubed_mixer::Duck::new(cubed_mixer::BusId::MASTER, 9.0);
        Self {
            key: DEFAULT_BUS.to_string(),
            depth_db: duck.depth_db,
            threshold_db: duck.threshold_db,
            attack_ms: duck.attack_ms,
            release_ms: duck.release_ms,
        }
    }
}

/// One bus. Gains are in dB; `output` names the bus this one feeds
/// (`None` = master).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BusLayout {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    pub gain_db: f32,
    pub pan: f32,
    pub mute: bool,
    pub solo: bool,
    /// The insert chain, in order (at most [`MAX_EFFECTS`]).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<EffectLayout>,
    /// At most [`MAX_SENDS`].
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<SendLayout>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duck: Option<DuckLayout>,
}

impl Default for BusLayout {
    fn default() -> Self {
        Self {
            name: String::new(),
            output: None,
            gain_db: 0.0,
            pan: 0.0,
            mute: false,
            solo: false,
            effects: Vec::new(),
            sends: Vec::new(),
            duck: None,
        }
    }
}

impl BusLayout {
    fn named(name: &str) -> Self {
        Self {
            name: name.to_string(),
            ..Self::default()
        }
    }
}

/// The project-wide settings of positional sound.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SpatialLayout {
    /// Pitch shifts from motion; each `AudioSource` also has its own scale.
    pub doppler: bool,
    /// World units per second (343 for metres).
    pub speed_of_sound: f32,
}

impl Default for SpatialLayout {
    fn default() -> Self {
        Self {
            doppler: true,
            speed_of_sound: 343.0,
        }
    }
}

/// A named change to the mix that scripts fade to (`set_audio_snapshot`), such
/// as "paused" muffling sfx and music, and that audio volumes can name: the
/// same per-bus changes an `AudioVolume` lists.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SnapshotLayout {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverb_room_size: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reverb_damping: Option<f32>,
    pub buses: Vec<super::components::AudioVolumeBus>,
}

/// The buses of a project, master first, its snapshots and its spatial
/// settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MixerLayout {
    pub buses: Vec<BusLayout>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub snapshots: Vec<SnapshotLayout>,
    #[serde(default)]
    pub spatial: SpatialLayout,
}

impl Default for MixerLayout {
    fn default() -> Self {
        let mut buses: Vec<BusLayout> = [MASTER, MUSIC_BUS, DEFAULT_BUS, "ambience", "ui", "voice"]
            .into_iter()
            .map(BusLayout::named)
            .collect();
        buses.push(BusLayout::reverb_return());
        Self {
            buses,
            snapshots: Vec::new(),
            spatial: SpatialLayout::default(),
        }
    }
}

impl BusLayout {
    /// The default reverb return: all reverb, no dry signal, so buses (and
    /// audio volumes) send to it.
    pub fn reverb_return() -> Self {
        let mut reverb = EffectLayout::new(EffectKind::Reverb);
        reverb.params.insert("wet".into(), 1.0);
        reverb.params.insert("dry".into(), 0.0);
        Self {
            effects: vec![reverb],
            ..Self::named(REVERB_BUS)
        }
    }
}

impl MixerLayout {
    /// The project's layout, or the default one if it has none. A file that
    /// cannot be read is reported and the default used. On the web it is
    /// fetched relative to the page, where the project is served.
    pub fn load(project_root: Option<&Path>) -> MixerLayout {
        let Some(path) = project_root.map(|root| root.join(LAYOUT_FILE)) else {
            return MixerLayout::default();
        };
        #[cfg(not(target_arch = "wasm32"))]
        if !path.is_file() {
            return MixerLayout::default();
        }
        let text = match crate::io::read_to_string(&path) {
            Ok(text) => text,
            // The web cannot ask whether a file exists: a project without a
            // layout fails to fetch it.
            Err(err) if cfg!(target_arch = "wasm32") => {
                log::debug!(target: "audio", "{}: {err}; using the default buses", path.display());
                return MixerLayout::default();
            }
            Err(err) => {
                log::warn!(target: "audio", "{}: {err}; using the default buses", path.display());
                return MixerLayout::default();
            }
        };
        let parsed = serde_json::from_str::<MixerLayout>(&text).map_err(|e| e.to_string());
        match parsed {
            Ok(layout) => layout.normalized(),
            Err(err) => {
                log::warn!(target: "audio", "{}: {err}; using the default buses", path.display());
                MixerLayout::default()
            }
        }
    }

    /// Writes the layout to the project.
    pub fn save(&self, project_root: &Path) -> Result<PathBuf, String> {
        let path = project_root.join(LAYOUT_FILE);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, text + "\n").map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(path)
    }

    pub fn bus(&self, name: &str) -> Option<&BusLayout> {
        self.buses.iter().find(|b| b.name == name)
    }

    pub fn bus_mut(&mut self, name: &str) -> Option<&mut BusLayout> {
        self.buses.iter_mut().find(|b| b.name == name)
    }

    pub fn snapshot(&self, name: &str) -> Option<&SnapshotLayout> {
        self.snapshots.iter().find(|s| s.name == name)
    }

    /// Master first and once, names unique and non-empty, outputs naming
    /// buses that exist (others feed master), no output loops, and effects,
    /// sends and ducking within the mixer's limits.
    pub(super) fn normalized(mut self) -> MixerLayout {
        let mut seen = std::collections::HashSet::new();
        self.buses
            .retain(|b| !b.name.is_empty() && b.name != MASTER && seen.insert(b.name.clone()));
        let master = BusLayout::named(MASTER);
        self.buses.insert(0, master);
        let names: Vec<String> = self.buses.iter().map(|b| b.name.clone()).collect();
        for bus in &mut self.buses {
            if bus
                .output
                .as_ref()
                .is_some_and(|o| o == MASTER || o == &bus.name || !names.contains(o))
            {
                bus.output = None;
            }
        }
        // Break loops: a bus whose output chain comes back to it feeds master.
        for i in 0..self.buses.len() {
            let mut at = self.buses[i].output.clone();
            for _ in 0..self.buses.len() {
                match at {
                    Some(ref name) if name == &self.buses[i].name => {
                        self.buses[i].output = None;
                        break;
                    }
                    Some(ref name) => at = self.bus(name).and_then(|b| b.output.clone()),
                    None => break,
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        self.snapshots
            .retain(|s| !s.name.is_empty() && seen.insert(s.name.clone()));
        for bus in &mut self.buses {
            bus.effects.retain(|e| {
                let known = EffectKind::from_name(&e.kind).is_some();
                if !known {
                    log::warn!(target: "audio", "bus {}: unknown effect {:?} left out", bus.name, e.kind);
                }
                known
            });
            bus.effects.truncate(MAX_EFFECTS);
            let name = bus.name.clone();
            bus.sends
                .retain(|s| s.to != name && names.contains(&s.to) && s.to != MASTER);
            bus.sends.truncate(MAX_SENDS);
            if bus
                .duck
                .as_ref()
                .is_some_and(|d| d.key == name || !names.contains(&d.key))
            {
                bus.duck = None;
            }
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_layout_file_round_trips_and_is_repaired() {
        let dir = tempfile::tempdir().unwrap();
        let mut layout = MixerLayout::default();
        layout.bus_mut("music").unwrap().gain_db = -6.0;
        layout.bus_mut("ui").unwrap().output = Some("music".into());
        layout.save(dir.path()).unwrap();
        assert_eq!(MixerLayout::load(Some(dir.path())), layout);

        let broken = r#"{"buses": [
            {"name": "sfx", "output": "a"}, {"name": "a", "output": "sfx"},
            {"name": "sfx"}, {"name": "", "gain_db": 3}
        ]}"#;
        std::fs::write(dir.path().join(LAYOUT_FILE), broken).unwrap();
        let repaired = MixerLayout::load(Some(dir.path()));
        let names: Vec<_> = repaired.buses.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, [MASTER, "sfx", "a"]);
        assert!(
            repaired.bus("sfx").unwrap().output.is_none()
                || repaired.bus("a").unwrap().output.is_none()
        );
    }

    #[test]
    fn effects_sends_and_ducking_save_and_are_checked() {
        let dir = tempfile::tempdir().unwrap();
        let mut layout = MixerLayout::default();
        let music = layout.bus_mut(MUSIC_BUS).unwrap();
        let mut low_pass = EffectLayout::new(EffectKind::LowPass);
        low_pass.params.insert("cutoff_hz".into(), 800.0);
        music.effects.push(low_pass);
        music.sends.push(SendLayout {
            to: REVERB_BUS.into(),
            level_db: -12.0,
            pre_fader: false,
        });
        music.duck = Some(DuckLayout::default());
        layout.save(dir.path()).unwrap();
        let loaded = MixerLayout::load(Some(dir.path()));
        assert_eq!(loaded, layout);
        let effect = loaded.bus(MUSIC_BUS).unwrap().effects[0].effect().unwrap();
        assert_eq!(effect.get("cutoff_hz"), Some(800.0));
        assert_eq!(
            loaded.bus(REVERB_BUS).unwrap().effects[0]
                .effect()
                .unwrap()
                .get("dry"),
            Some(0.0)
        );

        let broken = r#"{"buses": [
            {"name": "music", "effects": [{"kind": "flanger"}, {"kind": "delay"}],
             "sends": [{"to": "music"}, {"to": "nowhere"}, {"to": "sfx"}],
             "duck": {"key": "music"}},
            {"name": "sfx", "duck": {"key": "music", "depth_db": 6}}
        ]}"#;
        std::fs::write(dir.path().join(LAYOUT_FILE), broken).unwrap();
        let repaired = MixerLayout::load(Some(dir.path()));
        let music = repaired.bus(MUSIC_BUS).unwrap();
        assert_eq!(music.effects.len(), 1);
        assert_eq!(music.sends.len(), 1);
        assert!(music.duck.is_none(), "a bus cannot duck under itself");
        let sfx = repaired.bus("sfx").unwrap();
        assert_eq!(sfx.duck.as_ref().unwrap().depth_db, 6.0);
        assert_eq!(sfx.duck.as_ref().unwrap().release_ms, 400.0);
    }
}
