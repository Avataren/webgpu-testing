# Future improvements

A running list of worthwhile work that is not started, with what is known about each
item: where the cost is today, what the fix would be and what it would touch. Entries
with numbers were measured on the reference machine (Ryzen 9 5950X, RX 7900 XTX, RADV)
unless marked as estimates. Started 2026-10-17 after the pooled-skinning pass; add to it
rather than keeping lists elsewhere. Feature-specific investigations stay in their own
documents (`rendering_features_future.md`, `skinning_feature.md`), which this file links
to rather than repeats.

## Crowds and skinning

The test bed is `games/marionette` (its README has the benchmark numbers). The stadium
(1,040 characters, 2,933 skinned meshes, 14.5M skinned vertices, 80k entities) runs at
61 fps; the frame is CPU-bound at about 16 ms with the GPU at 9 ms.

- **One draw per mesh instead of one per skinned instance.** Submission is 6.3 ms of
  the stadium frame, nearly all of it wgpu's per-call cost for 6,866 draws (each skinned
  instance is its own draw in every pass: shadows, prepass, main, transparent). Since
  the instances' vertices already sit in a few pool buffers, the vertex shader could read
  them from a storage buffer at `instance.pool_base + vertex_index` (vertex pulling) and
  all instances of a mesh would draw with one `draw_indexed` and an instance range. That
  needs a vertex-pulling variant of every pass's vertex stage (shadows, prepass,
  G-buffer, picking, transparency, shader materials; `ShaderBuilder` already swaps
  include sets by capability, which is the model). Natively, `multi_draw_indexed_indirect`
  with per-instance base vertices is a smaller change but is not available on WebGPU,
  so it would need the vertex-pulling path as the web fallback anyway. Estimate: most of
  the 6.3 ms, since the draw count would fall from thousands to the number of distinct
  meshes (139 in the stadium).
- **Propagate transforms once per frame.** `scheduler.rs` propagates twice (after the
  update stage and again before rendering, for systems that move entities in between).
  Both passes together cost 2.4 ms in the stadium (parallel over root subtrees since
  2026-10-17). A dirty flag set by the GPU-stage systems, remote commands and the editor
  (all of which already call `propagate_transforms` themselves) would skip the second
  pass in the common case; cheaper still would be letting animators write bone world
  transforms directly, since bones are 71k of the 80k entities.
- **Cull skinned instances.** Skinned batches carry no bounds (`bounds` stays empty in
  `batches.rs`) because the bind-pose box does not follow the pose, so the stadium skins
  and shadows every character even when the camera looks at a corner of the stands.
  Bounds from joint positions plus a per-joint radius (computed with the palette, which
  already walks every joint) would let the existing culling and the skinning stage skip
  instances outside the view and the shadow frustum. Shared with "animated bounds" in
  `skinning_feature.md` (picking, `scene_query`, selection boxes use the bind pose too).
- **Skin fewer bytes.** The pool stores whole 64-byte vertices (UVs and colours are
  copied unchanged) and the stage reads 80 bytes and writes 64 per vertex: about 2 GB of
  traffic a frame for the stadium, 2.3 ms of GPU time. Skinning only positions, normals
  and tangents (48 bytes) and keeping UVs and colours in the source buffer needs a
  second vertex stream in every pass, so it pairs naturally with the vertex-pulling
  change above. Half-float normals and tangents would halve that again.
- **Level of detail.** Nothing reduces far characters: a 10k-vertex Mixamo character in
  the top row of the stands covers a few pixels. Mesh LODs from the library (decimated at
  import, `meshoptimizer` or Blender) picked by screen size per instance, and a pose
  update rate that drops with distance (animate every second or fourth frame far away),
  would cut skinning, posing and propagation together.
- **Shadow maps for crowds.** Every skinned caster is drawn into every shadow view each
  frame (the shadow cache is bypassed for skinned and GPU-placed casters, see
  `renderer-perf-pass` notes in `shadows.rs`). Cascade-aware culling of casters and a
  lower-resolution caster set for far cascades would help the 1.1 ms shadow pass and the
  1,961 shadow draws.
- **Ray tracing with crowds.** With ray tracing on, 1,000 skinned characters rebuild
  about 2,900 BLAS a frame and fill VRAM, so the show runs raster only. Skinned BLAS are
  now rebuilt only when the pose changed, and off-screen ones every 2-4 frames
  (`skinned_rebuild_due` in `raytracing/scene.rs`), but every visible animated character
  still rebuilds each frame (Lantern Market's wide shot: 149 meshes, 2.6 ms). BLAS refits
  instead of rebuilds would be next, but wgpu-core 30 turns `update_mode: PreferUpdate`
  into a full build ("only rebuild is implemented"), so they wait for wgpu; one BLAS per
  character rather than per mesh is the other step.
- **Batching is still sequential.** `PreparedBatches::from_batcher` sorts and lays out
  2,940 batches on one thread, and `RenderBatcher::add` hashes every object into a map
  each frame. A per-frame arena keyed by a stable batch id, and sorting only the batches
  whose order can change, are small wins (under 1 ms) once the draw cost is gone.

## Animation

Open items are listed in `skinning_feature.md` ("Not done"); the ones most likely to
matter for games:

- Limb motion in motion vectors: the previous-pose stream exists per instance now
  (`previous_vertex_buffer` in `skinning.rs`), the G-buffer pass already binds it, so
  TAA and motion blur on characters should be checked and tuned rather than built.
- Animation events (clip end, footsteps at a time), additive layers, root rotation in
  root motion, IK for feet and look-at, ragdolls.
- Editor scrubbing and a timeline for clips while editing.
- Morph targets (blend shapes); `web/assets/morph/` still renders statically.

## Renderer

- **Editor UI cost.** egui's `ui_build` is the editor's largest CPU item (about 4 ms a
  frame on the earlier perf pass; the profiler's top symbol is `create_widget`). Caching
  the hierarchy and inspector panels when nothing changed, or building them only on the
  frames the mouse is over them, would make the editor as fast as the player for perf
  work.
- **Untextured materials sample textures anyway.** The main pass samples every material
  slot; `textureSampleGrad` would allow branching on the material flags without
  derivative problems. Noted on the 2026-10-16 perf pass, not measured since.
- **World-position G-buffer precision.** Positions are stored as half floats, which
  wobbles far from the origin; a view-space or depth-reconstructed position would fix
  large levels.
- **Ray-traced GI gaps** (see the RTGI notes in the memory of the 2026-10-16 sessions):
  surfaces the screen does not show get almost no indirect light in reflections, so a
  world-space irradiance cache (shared with the probe grid) is the next structural step;
  billboards keep raster ambient; pixels the GI cannot match to the acceleration
  structure get no ambient.
- **Light probe grid**: spread probe updates over frames instead of a round-robin cap
  per frame; `shader_reload` does not rebuild the update pipelines; the baked and live
  results still differ in direct light on the web (shadow maps versus traced shadows).
- **WebGL2 tier and web ray tracing**: `rendering_features_future.md` is the
  investigation. The pooled skinning keeps a CPU path for a downlevel tier, but the
  G-buffer, storage buffers and compute passes are the blockers it lists.
- **HDR output** is supported but untested on a real HDR display (the development
  machine has none); the simulate mode is the only check so far.

## Engine and tooling

- **Build and test times.** The full release build of the player and editor is about a
  minute incremental; `cargo test --all-features` builds a second copy. A shared
  `target` profile for tests and a smaller default feature set for the player would
  help iteration.
- **Web export size.** The wasm player is about 20 MB before compression; the wasm
  profile is already size-optimized, so the next steps are dropping unused Lua APIs and
  shaders from the web build and lazy-loading content.
- **Asset library**: decimated LOD variants and precomputed bounds at ingest (both used
  by the crowd items above), and a Blender-free FBX converter (`ufbx` in a separate
  crate) for machines without Blender.
- **Engine MCP**: the server has to be restarted after a rebuild for new tools to
  appear; `web_build` needs a running engine while `web_playtest` does not, which is
  confusing. Keep the measurement rules in mind when comparing runs: another program on
  the GPU halves the crowd numbers.
