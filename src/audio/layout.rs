//! A project's mixer buses, saved as `content/audio/mixer.json`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Where a project keeps its bus layout, relative to the project root.
pub const LAYOUT_FILE: &str = "content/audio/mixer.json";

/// The bus every mix ends in.
pub const MASTER: &str = "master";
/// Where sounds play unless they name another bus.
pub const DEFAULT_BUS: &str = "sfx";

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

/// The buses of a project, master first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MixerLayout {
    pub buses: Vec<BusLayout>,
}

impl Default for MixerLayout {
    fn default() -> Self {
        Self {
            buses: [MASTER, "music", DEFAULT_BUS, "ambience", "ui", "voice"]
                .into_iter()
                .map(BusLayout::named)
                .collect(),
        }
    }
}

impl MixerLayout {
    /// The project's layout, or the default one if it has none. A file that
    /// cannot be read is reported and the default used.
    pub fn load(project_root: Option<&Path>) -> MixerLayout {
        let Some(path) = project_root.map(|root| root.join(LAYOUT_FILE)) else {
            return MixerLayout::default();
        };
        if !path.is_file() {
            return MixerLayout::default();
        }
        let parsed = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|text| serde_json::from_str::<MixerLayout>(&text).map_err(|e| e.to_string()));
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

    /// Master first and once, names unique and non-empty, outputs naming
    /// buses that exist (others feed master), and no output loops.
    fn normalized(mut self) -> MixerLayout {
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
}
