# Future rendering work: WebGL2 fallback and ray tracing

Status: investigation only, nothing implemented. Written against commit `9abfd9f`
(wgpu 30.0.1, naga 30.0.1). Line numbers drift; search for the named symbols.

This is the result of a read-only look at the code, the lockfile and the wgpu 30
sources in `~/.cargo/registry`. Nothing here was built or run on GL, and no
ray-tracing code was tried. Effort figures are rough guesses, not measurements.

## Questions this answers

1. What would it take to support more renderers, in particular a WebGL fallback for
   the web build (and possibly natively, for identical results)?
2. Ray tracing "in WebGPU": can it be a stand-alone renderer, or be used for specific
   effects (reflections etc.) inside the current pipeline?

## Summary

- **"More renderers" should mean capability tiers of the one renderer.** wgpu already
  is the abstraction over Vulkan, DX12, Metal, GL and WebGPU. A second hand-written
  renderer (glow/WebGL2) would duplicate every pass and shader, so don't.
- **The web build has no WebGL fallback today.** `Backends::GL` in
  `src/renderer/internal/context.rs` (`RenderContext::new_internal`) is a no-op on
  wasm: wgpu needs its `webgl` feature for that and `Cargo.toml` uses
  `wgpu = "30.0"` with default features. `cargo tree --target wasm32-unknown-unknown`
  shows no `glow` in the graph.
- **Enabling the `webgl` feature is one line, but the renderer would not run on it.**
  It relies on storage buffers, compute, multisampled `textureLoad` and a 4-attachment
  / 32-byte G-buffer. See "WebGL2 blockers".
- **Ray tracing is native-only.** wgpu 30 has it as `EXPERIMENTAL_RAY_QUERY` on
  Vulkan, DX12 (Tier 1.1) and Metal. Its docs say native-only and its web backend
  (`wgpu-30.0.1/src/backend/webgpu`) has no mapping for it. A web search found
  nothing on a browser proposal; check the gpuweb repo before relying on one. The dev
  machine (RX 7900 XTX, RADV) exposes `VK_KHR_acceleration_structure`,
  `VK_KHR_ray_query` and `VK_KHR_ray_tracing_pipeline`.
- **Ray tracing fits the current pipeline well** as an additive pass: SSR is a clean
  seam, skinned meshes are already deformed into per-instance vertex buffers (see
  `renderer/skinning.rs`) and the G-buffer already has position, normal and smoothness.

## Part 1: WebGL2 fallback (downlevel tier)

### What exists today

- Native: `Backends::all()`, so the GL backend is in the instance, but the renderer
  would fail at pipeline creation. There is no `WGPU_BACKEND`-style override
  (`InstanceDescriptor` is built from explicit `backends`, never from the
  environment).
- Web: `BROWSER_WEBGPU | GL`, but only WebGPU is compiled in (see above).
- The "traditional" per-material texture path already exists for adapters without
  bindless textures (`WGPU_CUBE_FORCE_TRADITIONAL_TEXTURES=1` forces it natively).
- GPU readbacks are already non-blocking (`gpu_particles/system/readback.rs`).
- `Mesh` keeps a CPU copy of its data (`src/asset/mesh.rs`, `MeshData`).
- `ShaderBuilder` already swaps WGSL include sets by capability
  (`bindings_bindless_*.wgsl` vs `bindings_traditional_*.wgsl`). That is the model
  for tier variants.
- `wgpu` types are everywhere outside the renderer (about 1650 `wgpu::` references in
  `src/renderer`, 225 in `src/gpu_particles`, 30 in examples, 41 in `src/bin`;
  `CustomRenderContext` exposes `wgpu::CommandEncoder` and `TextureView`). A
  `trait Renderer` hiding the API is therefore not realistic and not needed.

### WebGL2 blockers

wgpu's `downlevel_webgl2_defaults`: 0 storage buffers, 0 storage textures, no compute,
16 KiB uniform bindings, 16 sampled textures and 16 samplers per stage, 4 colour
attachments.

| Blocker | Where |
|---|---|
| Storage buffers read in vertex/fragment shaders (objects, materials, prev_models, lights, exposure state) | `src/shader/common.wgsl` (`objects`, `materials`, `prev_models`), `lighting_common.wgsl` (`lights`), `postprocess/composite.wgsl` (`composite_exposure`), `depth_prepass.wgsl`, `shadow.wgsl`, `templates/shader_material.wgsl`, `renderer/shader/error_fallback.wgsl`, `particle_render.wgsl`, `gpu_particle_render.wgsl`, `particle_shadow.wgsl` |
| Compute shaders: auto-exposure, all GPU particles (physics, starfield, boids, spatial grid, radix sort, prefix sums) | `postprocess/auto_exposure.wgsl`, `src/gpu_particles/shaders/`, `src/shader/gpu_particles.wgsl` |
| `textureLoad` on multisampled depth (MSAA depth resolve) | `src/shader/postprocess_msaa.wgsl`, `depth_resolve.wgsl`, `bin/editor/postprocess/grid_overlay_msaa.wgsl` |
| sRGB surface with a linear `view_formats` view for egui | `Renderer::ui_view`, `renderer_core.rs` |
| G-buffer: HDR colour + normal/motion + world position (+ pick id) fills 4 attachments and the 32-byte per-sample budget; Rgba16Float/Rgba32Float targets need `EXT_color_buffer_float` | main colour pass, `postprocess/` |
| 16 textures per stage: material slots plus shadow maps, environment cube maps and BRDF LUT in one pipeline | `bindings_traditional_*.wgsl`, `shadows.wgsl`, `environment.wgsl` |
| User-authored WGSL (shader editor, material shaders) that uses storage buffers will fail on GL | `templates/shader_material.wgsl` |

naga translates WGSL to GLSL ES 3.00 itself, so the shaders do not need to be rewritten
in GLSL; they need to stop using the features above.

### Plan

1. **`RenderCaps`**: one struct computed in `RenderContext` from the adapter's
   features, limits and downlevel flags (bindless, compute, vertex/fragment storage,
   multisampled sampling, float targets, ray query). Log it at startup. Replace
   `supports_bindless_textures` / `supports_wireframe` style booleans with it over time.
2. **Backend override**: env var and/or `settings.json` field to force a backend
   natively. Needed to test GL on the dev machine.
3. **Native GL smoke run**: force GL, run the player and note the first failures.
   Cheap and tells us how far the list above is from the truth.
4. **Enable wgpu's `webgl` feature on wasm** and let the adapter choice fall back from
   WebGPU to GL.
5. **Downlevel shader variants** via `ShaderBuilder` markers:
   - per-instance data (model, previous model, material index) from an instance-rate
     vertex buffer instead of `objects[]` / `prev_models[]`;
   - materials from a uniform array or data texture (watch the 16 KiB limit; may mean
     batching per material);
   - lights from a uniform buffer with a smaller light cap;
   - exposure as a uniform instead of a storage buffer;
   - no multisampled sampling: render MSAA with a resolve and lose the depth-based
     effects, or use FXAA in this tier.
6. **Tier policy for passes**: auto-exposure falls back to a fixed value or a
   fragment-shader reduction; GPU particles off (or a CPU simulation); SSR, SSAO, DoF
   off by default; shadows kept (they use uniform data and depth textures).
7. **Visual gate**: compare each tier against the WebGPU tier with `visual_capture`
   (regions, noise tolerance, diff images). Land the variants in small steps, each
   behind a diff check.

Rough effort: caps + override + GL smoke run about 1 day; a decent-looking fallback
(lit, shadowed, tonemapped, no particles or SSR/SSAO) about 3 to 5 weeks.

### "Identical results" natively

Running the downlevel tier on native GL (EGL) lets the web fallback be tested and diffed
locally. Bit-identical output across GPUs and drivers is not realistic (float math,
MSAA sample patterns). Aim for "matches within a tolerance". A deterministic reference
needs a software rasterizer (lavapipe/llvmpipe); neither is installed on the dev machine
(only the RADV ICD).

### Is it worth it?

The user's everyday Brave on Linux has no WebGPU adapter (needs flags; see the
web-build-testing memory), so a fallback matters at least for Linux, and probably for
Android and older Safari. If web reach on non-WebGPU browsers does not matter, skip the
downlevel tier and keep only steps 1 to 3.

## Part 2: ray tracing

**Status (2026-10-15): RT reflections implemented (experiment).** `ssr.ray_traced`
traces reflections with ray queries and falls back to SSR when the device has no ray
queries (`engine_info` `surface.ray_tracing` says why). Code: `renderer/raytracing/`
(support probe, geometry pool, BLAS cache, per-frame TLAS), `postprocess/rt_reflections.rs`
and `shader/postprocess/rt_reflections.wgsl`. Hits the screen shows reuse the lit scene
colour; other hits are shaded from the material (base colour, metallic-roughness, emissive
and occlusion maps from the bindless texture array, mip level from ray cones) with
ray-traced shadow rays. The apply pass weights reflections by the surface's specular
colour (F0, packed with smoothness into the G-buffer position's w), so metals reflect fully
and in their own colour. Skinned instances are traced too: each frame the deformed
vertices are copied into the pool and their BLAS rebuilt (wgpu 30 has no refit). Both
reflection paths accumulate the half-resolution trace over frames before the blur, which
removes the shimmer TAA's jitter caused on curved and detailed mirrors (in `games/rt-lab`,
pixels changing between frames went from 0.47% to 0.15%, the same as with reflections
off). Smooth hit surfaces (roughness below 0.3) trace one more reflection ray, so a
mirror seen in a mirror reflects too; for them the trace no longer reuses the screen's
colour, which lacks their reflection. Reflection and TAA histories restart on camera cuts
(more than 2 m or 15 degrees in a frame). Test scene: `games/rt-lab`. Not yet:
alpha-blended and billboarded geometry, normal maps at hits, blurring the second bounce by
roughness. `ssr.full_resolution` traces one ray per pixel instead of per 2x2 for sharp
mirrors. GPU cost in rt-lab at 1824x1080 on an RX 7900 XTX (`engine_stats` `renderer.gpu`):
whole frame 1.8 ms; reflections 0.21 ms at half resolution without the second bounce,
0.46 ms at full, 0.80 ms at full with it; acceleration structures 0.2 ms (TLAS rebuild
0.1 ms, one skinned BLAS rebuild 0.1 ms). Where a reflection is found on a metal, the apply
pass takes off the flat ambient light the main pass added there, so mirrors no longer look
hazy; with an HDR environment the main pass's environment specular is still added under
the reflection. `WGPU_CUBE_DISABLE_RAY_TRACING=1` tests the fallback.

**RT shadows (2026-10-15).** `rt_shadows.enabled` traces one shadow ray per pixel and
shadow-casting light (up to 4 directional, 4 point and 4 spot) between the depth prepass
and the main pass, into a screen-space R32Uint mask with one bit per light (0-3
directional, 4-7 point, 8-11 spot; `renderer/raytracing/shadows.rs`,
`shader/rt_shadows.wgsl`): the surface comes from depth through the jittered camera, its
geometric normal from neighbouring depths. The main pass reads the mask (lights group
binding 11; `lights.counts.w` says which lights) instead of the shadow maps for opaque
surfaces. `rt_shadows.softness` is the directional lights' apparent size in degrees (0.5
like the sun) and `rt_shadows.light_radius` the point and spot lights' radius (0.05 m):
each pixel and frame aims at another point of the light and TAA averages them, so large
lights look grainy. Unlike shadow maps, one-sided walls block light too (rt-lab's side walls
leaked the sun before). The shadow maps the mask replaces are not rendered unless
something else reads them (transparent batches, custom shadow passes, or lit GPU particles,
which took the lights group the frame before). Falls back to shadow maps without ray
queries, with MSAA, and for transparent surfaces and particles. In the rt-lab editor view
(sun and two lamps) the mask costs about 0.13 ms and the shadow-map pass drops from about
0.16 to 0.01 ms.

**RT ambient occlusion (2026-10-15).** `ssao.ray_traced` replaces the screen-space estimate
with 8 cosine-weighted rays per pixel up to `ssao.radius` (`postprocess/rt_ao.rs`,
`shader/postprocess/rt_ao.wgsl`), written into the SSAO texture so SSAO's blur and the
composite finish it. The ray pattern is fixed per pixel (rotated by interleaved gradient
noise), because the pass runs after TAA. About 0.41 ms against SSAO's 0.15 ms in the rt-lab
editor view at a 0.5 m radius. `engine_stats` `renderer.gpu` now splits `temporal` and
`ambient_occlusion` out of post-processing.

Not yet: a denoiser for large lights' soft shadows, temporal accumulation for RT AO (it
would allow fewer rays), shadow rays for alpha-tested geometry.

### What wgpu 30 provides

- Features: `EXPERIMENTAL_RAY_QUERY` (Vulkan, DX12 Tier 1.1 + SM 6.5, Metal),
  `EXPERIMENTAL_RAY_TRACING_PIPELINES` (Vulkan only, no WGSL stages for it yet),
  `EXPERIMENTAL_RAY_HIT_VERTEX_RETURN` (Vulkan), `ACCELERATION_STRUCTURE_BINDING_ARRAY`
  (DX12, Vulkan), `EXTENDED_ACCELERATION_STRUCTURE_VERTEX_FORMATS`.
- Opt-in: the device descriptor needs `ExperimentalFeatures::enabled()` (unsafe).
  `context.rs` currently passes `ExperimentalFeatures::disabled()`.
- API: `Device::create_blas`, `Device::create_tlas`,
  `CommandEncoder::build_acceleration_structures`; vertex/index buffers feeding a BLAS
  need `BufferUsages::BLAS_INPUT`.
- WGSL: `enable wgpu_ray_query;`, `acceleration_structure` bindings, `ray_query`
  initialize / proceed / committed-intersection calls, usable from compute and fragment
  shaders. Not ray-generation / closest-hit stages.
- Experimental: expect breaking changes between wgpu majors (the repo has tracked
  wgpu 27, 29, 30 already) and bugs. Only Vulkan can be tested on the dev machine.

### What the engine already has

- **Mostly static geometry**: one BLAS per rigid mesh asset, built once. Skinned
  instances are deformed into their own vertex buffers each frame (`SkinningStage`), so
  each gets a BLAS refit from that buffer (add `BLAS_INPUT` to its usage). The TLAS
  rebuilds each frame from `RenderBatcher` batches (instances and model matrices are
  already assembled there). Procedurally edited meshes (voxels, procgen) would need a
  BLAS rebuild on change.
- **G-buffer** with world position (`w` = 1 + smoothness), octahedral normal + motion:
  enough to start a reflection, shadow or AO ray per pixel.
- **SSR seam**: `src/shader/postprocess/ssr.wgsl` and `renderer/postprocess/ssr.rs`
  trace at half resolution into `ssr_half`, then two blur passes and an apply pass add
  the result before TAA. An RT trace pass can write the same texture.
- **TAA and the SSR blur** already exist as denoising building blocks.
- **Bindless textures** on native adapters, so hit shading can sample material textures.
- **`MeshData`** on the CPU, which also serves a software BVH.

### What is missing

- Mesh buffers are `VERTEX` / `INDEX` only (`src/asset/mesh.rs`). Need `BLAS_INPUT`
  and storage access (or a shared geometry pool with per-instance offsets) so a hit
  shader can read position, normal, uv and indices.
- TLAS instance custom data pointing at the object / material index.
- A hit-shading WGSL module: material fetch at the hit uv, direct light with a shadow
  ray, environment on miss. Reuse the PBR code in `lighting_common.wgsl` where possible.
- Settings plumbing (`PostProcessSettings`, volumes, inspector, Lua, remote API).
- Transparent and particle geometry: skip at first; ray queries need manual handling
  of non-opaque candidates.

### Options, by value for effort

All share the infrastructure below.

1. **RT reflections** (first target): SSR where the on-screen march hits, RT for misses
   and off-screen; later, full replacement. Writes into the existing `ssr_half`
   texture. Roughly 1 to 2 weeks including the infrastructure.
2. **RT shadows** (directional and spot): a shadow ray from the G-buffer position,
   jittered for soft edges, accumulated by TAA. Days once the infrastructure exists.
3. **RT ambient occlusion**: replaces the screen-space limits of SSAO. Days.
4. **Stand-alone path tracer** as a render mode: progressive accumulation from the
   camera with ray queries, as a ground-truth view for the editor and for validating
   the raster pipeline and screenshots. Roughly 2 to 3 weeks (full material evaluation,
   light and environment sampling, accumulation, optional denoise). It will not be
   pixel-identical to the raster output.
5. **GI** (DDGI or ReSTIR): large; only after 1 to 3 prove out.

### Shared infrastructure

1. Feature negotiation and the experimental opt-in in `RenderContext`, surfaced in
   `RenderCaps` (`ray_query`).
2. BLAS cache keyed by mesh handle, built when a mesh is created or edited.
3. Per-frame TLAS build from the batcher, with instance custom data and masks.
4. Geometry access for hit shading (storage usage or a geometry pool).
5. Hit-shading WGSL module, plus a `renderer/raytracing/` module with its own pass
   wired into `postprocess` the way `SsrStage` is.
6. Tests: unit tests for the BLAS/TLAS bookkeeping, and `visual_capture` comparisons
   against SSR on a reflective test scene (games/robotron is the usual test bed).

### Web and non-RT GPUs

RT effects on the web, or on GPUs without ray query, would need a software BVH: build
it on the CPU from `MeshData`, upload to storage buffers, traverse in WGSL compute, and
share the hit-shading module with the hardware path. Expect it to be considerably
slower than hardware RT (unmeasured); budget for half resolution and few rays. Design
the hit-shading and geometry-access parts so they do not assume hardware RT.

## Suggested order

1. `RenderCaps`, backend override, native-GL smoke run (about 1 day). Shared by both
   parts.
2. RT reflections on Vulkan (additive, touches no existing shaders).
3. RT shadows and AO.
4. Downlevel tier for WebGL2, only if web reach on non-WebGPU browsers is wanted.
   Invasive: it touches every shader's data fetch.
5. Path-tracer mode and software-BVH path, as wanted.

## Open questions

- How much does web reach on browsers without WebGPU matter? It decides whether
  Part 1 goes past step 3.
- Should RT replace SSR where available, or stay a hybrid with SSR kept as the cheap
  option? Hybrid is the safer first step.
- Is DX12 / Metal ray tracing a target? Only Vulkan can be tested here.
- Is a software-BVH path wanted, or is RT a native-only feature?
- How far should GL/WebGL2 "identical results" go: tolerance diffs against the
  WebGPU tier (cheap) or a deterministic software-rasterizer reference (needs lavapipe
  or llvmpipe installed)?

## How to resume

- Build and run notes are in `docs/ARCHITECTURE.md`; web testing needs a separate
  WebGPU-flagged browser (`./build_web.sh`, port 8080 is taken, pass `PORT=...`).
- For RT work, start with a spike that only enables `EXPERIMENTAL_RAY_QUERY`, builds
  one BLAS and TLAS for a test scene and traces mirror reflections into `ssr_half`.
- For GL work, start with step 2 and 3 of Part 1 to get a real failure list.
