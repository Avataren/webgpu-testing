//! The asset library: characters, animations and models shared by every project.
//!
//! It lives outside projects (and outside git) in `$WGPU_CUBE_LIBRARY`, or
//! `<user data dir>/wgpu-cube/library` (`~/.local/share/wgpu-cube/library` on Linux).
//! Each asset is a folder `assets/<id>/` holding `asset.json`, one GLB and a thumbnail;
//! `library.json` indexes them. `tools/library/library.py` fills it: it converts and splits
//! files with Blender (one asset per animation), measures clips, renders thumbnails and
//! fetches CC0 packs and Mixamo animations.
//!
//! Projects never reference the library: animations are copied into the project's
//! `content/animations` ([`Library::copy_into_project`]) and characters and models are
//! imported like any model, so a game carries only the assets it took, and its export
//! only those it uses.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// One asset of the library (its `asset.json`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryAsset {
    /// Unique, stable: `mixamo-walking`, `quaternius-ual2-sword-regular-a`.
    pub id: String,
    /// `character` (a rigged model), `animation` (one clip) or `model`.
    pub kind: String,
    pub name: String,
    /// The GLB, in the asset's folder.
    pub file: String,
    #[serde(default)]
    pub thumbnail: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// The bone naming: `mixamo`, `unreal`, `rigify` or `custom`. Clips bind across
    /// them by humanoid role.
    #[serde(default)]
    pub skeleton: Option<String>,
    #[serde(default)]
    pub clips: Vec<LibraryClip>,
    /// Characters and models: height in metres, triangles, bones.
    #[serde(default)]
    pub height: Option<f32>,
    #[serde(default)]
    pub triangles: Option<u64>,
    #[serde(default)]
    pub bones: Option<u32>,
    #[serde(default)]
    pub source: LibrarySource,
    #[serde(default)]
    pub added: Option<String>,
}

/// A clip of an animation asset, as measured when it was added.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryClip {
    pub name: String,
    pub duration: f32,
    /// How fast the root travels over the ground (m/s).
    pub root_speed: f32,
    pub in_place: bool,
}

/// Where an asset came from and on what terms.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LibrarySource {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub pack: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Deserialize)]
struct Index {
    assets: Vec<LibraryAsset>,
}

/// What [`Library::search`] looks for; empty fields match everything.
#[derive(Debug, Clone, Default)]
pub struct LibraryQuery {
    /// Words that must all appear in the id, name or tags.
    pub text: String,
    pub kind: Option<String>,
    pub tags: Vec<String>,
    pub skeleton: Option<String>,
}

/// The library as read from disk.
#[derive(Debug, Clone, Default)]
pub struct Library {
    pub root: PathBuf,
    pub assets: Vec<LibraryAsset>,
}

impl Library {
    /// `$WGPU_CUBE_LIBRARY`, or `<user data dir>/wgpu-cube/library`.
    pub fn default_root() -> Option<PathBuf> {
        let env = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty());
        if let Some(root) = env("WGPU_CUBE_LIBRARY") {
            return Some(PathBuf::from(root));
        }
        let data = if cfg!(target_arch = "wasm32") {
            None
        } else if cfg!(windows) {
            env("APPDATA").map(PathBuf::from)
        } else if cfg!(target_os = "macos") {
            env("HOME").map(|home| PathBuf::from(home).join("Library/Application Support"))
        } else {
            env("XDG_DATA_HOME")
                .map(PathBuf::from)
                .or_else(|| env("HOME").map(|home| PathBuf::from(home).join(".local/share")))
        };
        data.map(|dir| dir.join("wgpu-cube").join("library"))
    }

    /// Read the library at `root` (its index, else every `asset.json`). A missing
    /// library is an empty one.
    pub fn open(root: &Path) -> Result<Self, String> {
        let index = root.join("library.json");
        let assets = if index.is_file() {
            let text = std::fs::read_to_string(&index)
                .map_err(|err| format!("cannot read {}: {err}", index.display()))?;
            serde_json::from_str::<Index>(&text)
                .map_err(|err| format!("{} is not a library index: {err}", index.display()))?
                .assets
        } else {
            let mut assets = Vec::new();
            if let Ok(entries) = std::fs::read_dir(root.join("assets")) {
                for entry in entries.flatten() {
                    let path = entry.path().join("asset.json");
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        match serde_json::from_str(&text) {
                            Ok(asset) => assets.push(asset),
                            Err(err) => log::warn!("Skipping {}: {err}", path.display()),
                        }
                    }
                }
            }
            assets.sort_by(|a: &LibraryAsset, b| a.id.cmp(&b.id));
            assets
        };
        Ok(Self {
            root: root.to_path_buf(),
            assets,
        })
    }

    /// [`Library::open`] at [`Library::default_root`].
    pub fn open_default() -> Result<Self, String> {
        let root = Self::default_root().ok_or("no user data directory for the library")?;
        Self::open(&root)
    }

    pub fn get(&self, id: &str) -> Option<&LibraryAsset> {
        self.assets.iter().find(|asset| asset.id == id)
    }

    /// Assets matching `query`, best first: name matches before tag-only matches.
    pub fn search(&self, query: &LibraryQuery) -> Vec<&LibraryAsset> {
        let words: Vec<String> = query
            .text
            .to_lowercase()
            .split_whitespace()
            .map(str::to_owned)
            .collect();
        let mut found: Vec<(u32, &LibraryAsset)> = self
            .assets
            .iter()
            .filter(|asset| query.kind.as_ref().is_none_or(|kind| &asset.kind == kind))
            .filter(|asset| {
                query
                    .skeleton
                    .as_ref()
                    .is_none_or(|skeleton| asset.skeleton.as_ref() == Some(skeleton))
            })
            .filter(|asset| query.tags.iter().all(|tag| asset.tags.contains(tag)))
            .filter_map(|asset| {
                let name = asset.name.to_lowercase();
                let haystack =
                    format!("{} {} {}", asset.id, name, asset.tags.join(" ")).to_lowercase();
                if !words.iter().all(|word| haystack.contains(word.as_str())) {
                    return None;
                }
                let in_name = words
                    .iter()
                    .filter(|word| name.contains(word.as_str()))
                    .count() as u32;
                Some((in_name, asset))
            })
            .collect();
        found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.name.cmp(&b.1.name)));
        found.into_iter().map(|(_, asset)| asset).collect()
    }

    pub fn asset_dir(&self, asset: &LibraryAsset) -> PathBuf {
        self.root.join("assets").join(&asset.id)
    }

    pub fn file_path(&self, asset: &LibraryAsset) -> PathBuf {
        self.asset_dir(asset).join(&asset.file)
    }

    pub fn thumbnail_path(&self, asset: &LibraryAsset) -> Option<PathBuf> {
        asset
            .thumbnail
            .as_ref()
            .map(|thumbnail| self.asset_dir(asset).join(thumbnail))
    }

    /// Copy an animation's file into the project at `project_root`, as
    /// `content/animations/<name>.glb` so the clip plays under its name. Returns the
    /// project-relative path. An identical file already there is reused; a different one
    /// with that name gets a numbered name. (Characters and models are imported like any
    /// model instead, from a [`Library::staged_copy`].)
    pub fn copy_into_project(
        &self,
        asset: &LibraryAsset,
        project_root: &Path,
    ) -> Result<PathBuf, String> {
        let folder = "content/animations";
        let source = self.file_path(asset);
        let bytes = std::fs::read(&source)
            .map_err(|err| format!("cannot read {}: {err}", source.display()))?;
        let stem = file_stem_for(&asset.name);
        let dir = project_root.join(folder);
        std::fs::create_dir_all(&dir)
            .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
        for index in 1.. {
            let name = if index == 1 {
                format!("{stem}.glb")
            } else {
                format!("{stem} {index}.glb")
            };
            let target = dir.join(&name);
            match std::fs::read(&target) {
                Ok(existing) if existing == bytes => return Ok(Path::new(folder).join(name)),
                Ok(_) => continue,
                Err(_) => {
                    std::fs::write(&target, &bytes)
                        .map_err(|err| format!("cannot write {}: {err}", target.display()))?;
                    return Ok(Path::new(folder).join(name));
                }
            }
        }
        unreachable!("an unused name is always found")
    }
}

impl Library {
    /// Copy `asset`'s file into `dir` as `<name>.glb`, for an import that names what it
    /// makes after the file.
    pub fn staged_copy(&self, asset: &LibraryAsset, dir: &Path) -> Result<PathBuf, String> {
        std::fs::create_dir_all(dir)
            .map_err(|err| format!("cannot create {}: {err}", dir.display()))?;
        let target = dir.join(format!("{}.glb", file_stem_for(&asset.name)));
        std::fs::copy(self.file_path(asset), &target)
            .map_err(|err| format!("cannot copy {}: {err}", asset.id))?;
        Ok(target)
    }
}

/// A file name for an asset name: path separators and other troublesome characters
/// become spaces.
fn file_stem_for(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_().,'".contains(c) {
                c
            } else {
                ' '
            }
        })
        .collect();
    let cleaned = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if cleaned.is_empty() {
        "asset".into()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(id: &str, kind: &str, name: &str, tags: &[&str]) -> LibraryAsset {
        LibraryAsset {
            id: id.into(),
            kind: kind.into(),
            name: name.into(),
            file: format!("{id}.glb"),
            thumbnail: None,
            tags: tags.iter().map(|t| t.to_string()).collect(),
            skeleton: Some("mixamo".into()),
            clips: Vec::new(),
            height: None,
            triangles: None,
            bones: None,
            source: LibrarySource::default(),
            added: None,
        }
    }

    #[test]
    fn search_matches_words_kinds_and_tags_names_first() {
        let library = Library {
            root: PathBuf::from("/library"),
            assets: vec![
                asset("mixamo-walking", "animation", "Walking", &["locomotion"]),
                asset("mixamo-run", "animation", "Running", &["locomotion"]),
                asset(
                    "q-walk-carry",
                    "animation",
                    "Walk Carry Loop",
                    &["locomotion"],
                ),
                asset("mixamo-y-bot", "character", "Y Bot", &["character"]),
            ],
        };
        let ids = |query: LibraryQuery| -> Vec<String> {
            library
                .search(&query)
                .into_iter()
                .map(|a| a.id.clone())
                .collect()
        };
        assert_eq!(
            ids(LibraryQuery {
                text: "walk".into(),
                ..Default::default()
            }),
            ["q-walk-carry", "mixamo-walking"]
        );
        assert_eq!(
            ids(LibraryQuery {
                text: "locomotion".into(),
                ..Default::default()
            })
            .len(),
            3
        );
        assert_eq!(
            ids(LibraryQuery {
                kind: Some("character".into()),
                ..Default::default()
            }),
            ["mixamo-y-bot"]
        );
    }

    #[test]
    fn assets_copy_into_projects_once_under_their_names() {
        let dir = std::env::temp_dir().join(format!("library-copy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = dir.join("library");
        let project = dir.join("project");
        let walk = asset("mixamo-walking", "animation", "Walking", &[]);
        std::fs::create_dir_all(root.join("assets/mixamo-walking")).unwrap();
        std::fs::write(
            root.join("assets/mixamo-walking/mixamo-walking.glb"),
            b"walk",
        )
        .unwrap();
        let library = Library {
            root,
            assets: vec![walk.clone()],
        };
        let path = library.copy_into_project(&walk, &project).unwrap();
        assert_eq!(path, PathBuf::from("content/animations/Walking.glb"));
        assert_eq!(library.copy_into_project(&walk, &project).unwrap(), path);
        std::fs::write(project.join(&path), b"another walk").unwrap();
        assert_eq!(
            library.copy_into_project(&walk, &project).unwrap(),
            PathBuf::from("content/animations/Walking 2.glb")
        );
        assert_eq!(file_stem_for("Sword/Slash: A"), "Sword Slash A");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
