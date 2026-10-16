# Skeletal meshes and animation: status

Status as of 2026-10-15. The engine side (skinned rendering, the animation runtime, Lua and
MCP control) is implemented and tested. Asset sourcing (Mixamo download, FBX conversion) and
editor UI are not. This document replaces the original plan; the parts of it that still
apply are kept under "Not done".

How it works is summarised in `docs/ARCHITECTURE.md` ("Skeletal animation"). Script-facing
functions are in `scripts/LUA_SCRIPTING_README.md` ("Skeletal animation"), the MCP surface
in `tools/engine-mcp/README.md` and `.claude/skills/engine-mcp/SKILL.md`.

## Done

### Data and import

- **Skin stream.** `MeshData::skin: Option<Vec<SkinVertex>>` (`src/asset/mesh.rs`,
  `src/renderer/vertex.rs`): four joint indices (`u16`) and four weights (unorm16) per
  vertex, 16 bytes. `SkinVertex::from_influences` keeps the strongest four, renormalizes and
  quantizes so the weights sum to exactly 65535. Rigid meshes serialize as before (the field
  is omitted), so old scene files load unchanged.
- **glTF loader** (`src/scene/loader/nodes.rs`): reads `JOINTS_0/WEIGHTS_0` (and a second
  set, merged to the top four), includes the skin in the mesh dedup signature, generates
  smooth normals when a file has none, and scales inverse bind matrices with the import
  scale (`scale_inverse_bind`). Unnamed nodes are named `node_<index>`, the same name clips
  use for them.
- **`SkinnedMesh { joints: Vec<Entity>, inverse_bind: Arc<[Mat4]> }`**
  (`src/scene/skeletal/skin.rs`) on every mesh entity of a skinned node. Joints are ordinary
  entities, so the hierarchy, transforms, inspection and parenting props to bones all work.
  A joint outside the loaded scene is an error naming the joint; skins over 1024 joints are
  rejected.
- **Embedded textures** of glTF imports are decoded to RGBA8 whatever their channel layout
  (RGB JPEGs crashed the import before), and imports now save materials with the paths of
  the package's extracted texture files instead of runtime texture indices (both the
  instanced copy and the import's own scene document), so textures survive a reload.

### Rendering

- **Compute pre-skinning instead of vertex-shader skinning.** `build_render_objects`
  computes each skinned entity's palette (`inverse(mesh_world) * joint_world *
  inverse_bind`) and passes it as a `SkinBinding`. Before the shadow pass,
  `SkinningStage` (`src/renderer/skinning.rs`) deforms every skinned instance into its own
  vertex buffer (`src/shader/skinning.wgsl`, one dispatch per instance). Each instance is
  its own batch; every pass (shadows, prepass, G-buffer, picking, transparency, user shader
  materials, gizmo outlines) draws it like a rigid mesh. No vertex shader knows about
  skinning.
- **CPU fallback** with the same math (`skin_vertices`), chosen automatically on devices
  without compute or storage buffers (the planned WebGL2/GLES tier).
  `WGPU_CUBE_FORCE_CPU_SKINNING=1` forces it natively. A GPU test compares the two on every
  vertex of the Fox sample.
- Skinned buffers are evicted after 120 frames unused and rebuilt if the mesh changes.
  `RendererStats` reports skinned instances and vertices.

### Animation runtime

- **`ClipLibrary`** (`src/scene/skeletal/clip.rs`): every animation of a glTF/GLB file as a
  `SkeletalClip` of bone-named tracks, plus the file's rest pose. Images and meshes are never
  decoded; buffers load through `crate::io`, so it works on the web. Cached per path in
  `Assets::clip_library`.
- **`Animator`** (`src/scene/skeletal/animator.rs`) on a character root:
  - binds tracks to the entities at or below it by normalized bone name (case-insensitive,
    without a `namespace:` prefix or Mixamo's `mixamorig`/`mixamorig1:`/`mixamorig_`),
    shallowest first;
  - names clips by their file name when unique, `<file stem>/<clip>` otherwise, and by the
    file stem when a file holds one clip (`walk.glb` -> `walk`);
  - cross-fades (from the rest pose too), loops or plays once (and restarts a finished clip
    when asked again), per-clip and global speed, negative speed;
  - layers (up to 8) with weights, fades and bone masks (a bone and everything below it);
  - retargets translations per bone by the ratio of target to source rest length, so clips
    authored in centimetres or on other proportions keep their feet on the ground;
  - root motion: `keep`, `in_place` (removes horizontal travel of the top-most translated
    bone, keeps the bob) or `apply` (moves the animator entity instead, across loop wraps);
  - reports unknown clip names and unreadable libraries once instead of every frame.
- Animators run only while the scene plays (`Scene::set_animation_playback(true)`), so the
  editor always shows and saves the rest pose; stopping clears playback state.
- **Skinned imports get an animator automatically** on each root node holding joints, with
  the file itself as its library and its first clip as `default_clip`. Their animations no
  longer also become entity-bound clips (which would drive the joints twice). The editor
  copies the source GLB into the import package so the library path stays valid.

### Persistence and copying

- `SceneAssetEntity` gains `skin` (joint entity indices plus inverse bind matrices) and
  `animator` (libraries, default clip, speed, root motion). Both are omitted when absent.
- Joint references are remapped on every copy path: `SceneAsset::instantiate` (snapshots,
  undo, prefabs, scene documents), `merge_world_as_child` (imports) and the remote
  `entity.duplicate`.

### Lua and MCP

- Lua: `animator_play`, `animator_stop`, `animator_set_speed`, `animator_set_layer`,
  `animator_add_clips`, `animator_set_root_motion`, `animator_state`, `animator_clips`,
  `find_bone`. Each accepts the character root or any entity below it.
- MCP / remote:
  - `entity_get` shows `skinned_mesh` (joints, vertices, driving animator) and `animator`
    (libraries, every clip with duration and bone coverage, missing bone examples, what each
    layer plays). Works in edit mode.
  - `entity_update {animator: {...}}`: `libraries`, `add_clips`, `default_clip`, `speed`,
    `root_motion`, `play`, `stop`, `layer`; `null` removes it. While editing, `play` sets
    `default_clip`.
  - `asset_import {kind: "animation"}` (or any GLB without meshes) copies a clip library into
    `content/animations` and lists its clips.
  - `animation_inspect {path}` lists a file's clips; `debug_skeleton {enabled}` draws bones as
    overlays in the viewport and screenshots.

### Test assets

Khronos samples with their licence files under `web/assets/skinned/`: CesiumMan, RiggedSimple
and RiggedFigure (CC-BY 4.0), Fox (CC0 model, CC-BY rig and animation), SimpleSkin (CC0).

### Tests

Unit tests for skin weights, mesh serialization, the CPU skinner, WGSL validation, the GPU vs
CPU comparison, palettes, bone-name normalization, clip loading (GLB and data-URI glTF), the
loader's skin stream and normals, inverse-bind scaling, and the animator (naming, retargeting,
cross-fades, one-shot clips, root motion, layers and masks, error reporting, serialization).
`tests/skinning.rs` covers import, JSON round trip, play versus edit mode and remapping on
merge; a Lua regression test covers the script functions.

## Not done

### Rendering

- **Limb motion in motion vectors.** Motion vectors use the previous model matrix only, so
  TAA and motion blur treat a running character as rigid (both are off by default). Fix:
  keep last frame's skinned buffer per instance and add it as a second vertex stream in the
  G-buffer pass.
- **Animated bounds.** Picking (GPU picking is exact), `scene_query`, `scene_raycast`,
  placement and editor selection boxes use the bind-pose `MeshBounds`. Compute bounds from
  joint positions plus a per-joint radius if culling or precise queries need it.
- **Instancing of crowds.** Each skinned instance is one draw per pass. Profile a crowd
  (`profile_capture`, e.g. 100 Foxes) before optimising; options are batching instances
  that share a mesh into one larger output buffer, or skipping unchanged palettes.
- **Ray tracing.** Skinned buffers are ready for BLAS refits (add `BLAS_INPUT` usage) once
  the work in `docs/rendering_features_future.md` starts.
- **Morph targets** (blend shapes) are not supported; `web/assets/morph/` still renders
  statically.

### Animation

- **Humanoid retargeting.** Rest-relative rotation transfer (other bone axes, armature
  turns, files facing other ways) and a humanoid bone map (Mixamo, Unreal mannequin and
  Rigify names) are done. Rigs with different rest postures (T-pose versus A-pose) still
  need a pose-matching step.
- **Root rotation** (turning on the spot in root motion) is not extracted; only translation.
- **Blend trees and state machines**: done as `AnimationTree` (states, 1D/2D blend
  spaces, transitions; see `docs/characters.md`). Not done: nested state machines, sync
  markers (footfalls) between clips of different cycles.
- **Additive layers**: layers override; there is no additive mode.
- **Animation events** (footsteps at a time in a clip): scripts can poll `animator_state`,
  but clips carry no events and nothing is emitted when a clip ends.
- **IK** (foot placement, look-at), ragdolls and physics.
- **Editor preview**: clips only play in play mode or under `runtime_step`; there is no
  scrubbing in edit mode.

### Editor UI

Done: inspector sections for `Animator` (clip files, clips with length, ground speed and
bone coverage, default clip, root motion) and `AnimationTree` (states, blend points,
transitions, live state in play mode), "Make Character", FBX import. Not done: a
`SkinnedMesh` section, collapsing joints in the hierarchy, a skeleton toggle in the
viewport, a timeline and clip preview while editing.

### Asset sources

- **The asset library** (`docs/asset_library.md`) now holds Quaternius's CC0 Universal
  Animation Libraries 1 and 2 (89 clips, three mannequins) and fetches Mixamo with the
  user's token; the notes below are kept for reference.
- **CC0 glTF packs first.** Quaternius's
  [Universal Animation Library](https://quaternius.com/packs/universalanimationlibrary.html)
  (CC0, 120+ animations on one humanoid rig, glTF, described as Mixamo-compatible) works
  through `asset_import {kind: "animation"}` with no new code, and CC0 assets could ship with
  examples. Not yet tried in the engine: check bone names and coverage with
  `animation_inspect` and `entity_get`.
- **FBX conversion** goes through Blender (`tools/blender/convert_to_gltf.py`, run by the
  editor's import). A Blender-free converter (`ufbx` in a separate crate, so the C library
  stays out of the engine and the wasm build) would help machines without Blender.
- **Mixamo tools** are not started. Findings from the earlier survey, unverified against the
  live service:
  - No public API. Community scripts use the web app's JSON API
    (`https://www.mixamo.com/api/v1/`, `Authorization: Bearer <token>` from the logged-in
    page's `localStorage.access_token`, `X-Api-Key: mixamo2`): `products` search,
    `products/{id}` for the motion parameters, `animations/export` to start an export and
    `characters/{id}/monitor` to poll it. Format names have changed over time.
  - Downloads are FBX or Collada, never glTF; about 65 bones named `mixamorig:*`,
    centimetre units, T-pose characters, Hips carrying the translation. The animator's name
    normalization and translation retargeting already cover these conventions.
  - Licence (2022 FAQ copy, current terms unconfirmed): free for games, no redistribution of
    raw files, no ML training. So: keep raw downloads in an uncommitted cache, convert into
    the user's project with a provenance note, one asset per request, no bulk download, never
    in engine tests or examples. Token from an environment variable or a 0600 file, never
    echoed.
  - Suggested shape: a Node client next to the MCP server (`mixamo.mjs`, like
    `web_playtest.mjs`), tools `mixamo_status`, `mixamo_search`, `mixamo_import_character`,
    `mixamo_import_animation`, tested against a local mock server.
