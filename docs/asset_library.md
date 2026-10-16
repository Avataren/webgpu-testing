# The asset library

A library of characters, animations and models that every project can draw from, so
building a game starts from a shelf of ready assets instead of an empty folder. Projects
copy in only what they take, and a game's export carries only what it uses.

## Where it is and what is in it

The library lives outside projects and outside git:

- `$WGPU_CUBE_LIBRARY`, or
- `<user data dir>/wgpu-cube/library` (`~/.local/share/wgpu-cube/library` on Linux,
  `%APPDATA%\wgpu-cube\library` on Windows, `~/Library/Application Support/...` on macOS).

```
library/
  library.json              index of every asset (read by the editor and MCP)
  assets/<id>/asset.json    one asset: kind, name, tags, clip measurements, source, licence
  assets/<id>/<id>.glb      its file
  assets/<id>/thumbnail.png 256x256 render
  cache/                    downloaded packs
  inbox/                    downloaded Mixamo FBX files before ingesting
```

Each asset is one of:

- **character**: a rigged model (meshes and skeleton, no animations), with its height,
  triangle and bone counts;
- **animation**: one clip on its skeleton, with its length, how fast it travels over the
  ground (`root_speed`, m/s) and whether it stays in place;
- **model**: a static model.

Packs are split on the way in: a GLB with 46 animations becomes 46 animation assets, so a
game that uses two of them carries two.

Characters and models are made game-ready on the way in: textures at most 2048 px, opaque
ones as quality-90 JPEG (those with transparency, hair and lashes, stay PNG), which takes
a Mixamo character from 50-150 MB to 10-20 MB. Mixamo characters outside 1.0-2.6 m tall
(its files come at odd scales: Remy 3.8 m, the pirate 0.65 m) are scaled to 1.8 m; the
asset records `scaled`.

Assets record their **skeleton** naming: `mixamo` (`mixamorig:LeftArm`), `unreal`
(`upperarm_l`, the Unreal mannequin and Quaternius's Universal Animation Library 2) or
`rigify` (`DEF-upper_arm.L`). Clips bind to bones by name and, failing that, by humanoid
role (hips, spine, upper arm, shin, fingers...), so any of these clips drives any of these
characters as long as both rest in the same posture (T-pose). See
`humanoid_key` in `src/scene/skeletal/clip.rs`.

## Filling it

`tools/library/library.py` (Python 3, standard library only; Blender converts, measures
and renders thumbnails, found through `BLENDER_PATH`, the `PATH` or its usual install
places):

```sh
tools/library/library.py fetch-pack all               # Quaternius CC0 animation libraries
tools/library/library.py fetch-mixamo                 # the curated Mixamo set (needs a token)
tools/library/library.py fetch-mixamo --query "Sword And Shield Kick" --only "Sword And Shield Kick"
tools/library/library.py ingest Knight.fbx --provider "my studio" --license "own work"
tools/library/library.py list --kind animation --query sword
tools/library/library.py remove mixamo-taunt
```

Every command takes `--json` for machine-readable output. `fetch-mixamo` skips assets the
library already has; `fetch-pack` and `ingest` replace them.

### Free (CC0) packs

`fetch-pack` downloads Quaternius's Universal Animation Library (Standard, 45 animations
on a Rigify-named mannequin) and Universal Animation Library 2 (Standard, 43 animations
on an Unreal-named mannequin, plus a female mannequin) from OpenGameArt. CC0: free for
any use, redistribution included.

### Mixamo

Mixamo has no public API; the fetcher uses the web app's API with your own session token,
as the community scripts do. Adobe's terms do not allow automated access, so this is at
your account's risk, and Mixamo's licence lets you use the files in your projects but not
redistribute the raw files: they stay in the library, never in a repository.

1. Log in at mixamo.com (a free Adobe account works).
2. Open the browser console on that page (Ctrl+Shift+J) and run
   `copy(localStorage.access_token)`. It answers `undefined`; the token is on the
   clipboard.
3. Without copying anything else, run `tools/library/library.py save-mixamo-token`. It
   checks the clipboard holds a token, saves it to `~/.config/wgpu-cube/mixamo-token`
   (readable only by you) and prints when it expires (about a day). `MIXAMO_TOKEN` works
   too. When the fetcher reports a 401, repeat these steps.

Several Mixamo animations share a name ("Waving" is also "Waving With Both Hands"); the
fetcher takes the one whose description matches the name exactly, and records Mixamo's
description and whether the clip loops. Clip lengths are Mixamo's own: "Waving" is a
0.53 s loop.

`fetch-mixamo` downloads the characters and animations listed in
`tools/library/mixamo_set.json`: about 100 animations (locomotion, jumps, combat,
weapons, magic, hits, interactions, emotes) and 44 characters, from the Y Bot / X Bot
mannequins and stand-ins (Mannequin, Dummy, Passive Marker Man) to knights, archers,
peasants, monsters, soldiers, zombies, everyday people and two cartoon figures. Every
Mixamo character shares one skeleton, so every animation plays on every character.
`mixamo-catalog --characters` lists all of Mixamo's characters (108) and
`mixamo-catalog --query sword` its animations with their descriptions; add names to the
set, or fetch them directly with `fetch-mixamo --query NAME --only NAME`. Animations are exported
for Y Bot without skin, with their ground travel kept (the engine measures it and plays
clips in place on characters).

### Your own files

`ingest` takes FBX, glTF/GLB, .blend and OBJ. Every animation in a file becomes its own
asset (thumbnailed on a library character with the same skeleton when the file has no
mesh); a file with meshes also gives a character (rigged) or model asset.

## Using it

### In the editor

The **Library** tab (next to Assets) lists the library with thumbnails: search, filter by
kind and skeleton, and pick an asset to see its measurements, source and licence.

- **Add to scene** (characters and models): imports it into the project and the open
  scene, like File > Import Model.
- **Add to <character>** (animations, with a character selected): copies the clip into
  `content/animations/<name>.glb` and adds it to that character's Animator. **Add to
  project** only copies it.

Double-clicking does the same. Then the character's Animation Tree section can "Generate
from clips", or Make Character sets everything up (see `docs/characters.md`).

### Over MCP

- `library_search {query, kind, tags, skeleton}` lists assets (with thumbnails' paths).
- `library_info {id}` gives the full entry.
- `library_add {ids, entity}` takes assets into the open project, animations onto
  `entity`'s animator.
- `library_fetch {source, names}` and `library_ingest {paths}` fill the library;
  `mixamo_catalog {characters, query}` shows what Mixamo offers.

```
library_search {query: "sword", kind: "animation"}
library_add {ids: ["quaternius-ual2-sword-regular-a", "quaternius-ual2-sword-block"], entity: "Player"}
entity_update {entity: "Player", animation_tree: {generate: true}}
```

## What a game ships

Projects never point into the library; they get copies. And an export only takes what
the game uses (`src/export.rs`): starting from the main scene (the first in
`project.json`), it follows the prefabs it instances, every file their scenes, materials
and model packages name, and the scripts' string literals (`play_sound("content/audio/x.wav")`,
`require` modules). A script that builds paths at run time keeps them by naming their
folder as a literal (`local folder = "content/audio/steps/"`), which ships the folder.

The editor's Build menu (which saves first), `build_web.sh --project` and MCP `web_build`
all stage only those files, with a `project.json` listing only the used scenes.
`project_content` over MCP shows what a build would carry and what it leaves out.
