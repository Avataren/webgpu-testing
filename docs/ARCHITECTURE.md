# Engine architecture

A map of the engine for people (and agents) new to the code: how a frame runs, where
each subsystem lives, and the invariants that are easy to break. Read this before
making non-trivial changes. Keep it current when you change something it describes.

Lua scripting from a script author's point of view is documented in
[`scripts/LUA_SCRIPTING_README.md`](../scripts/LUA_SCRIPTING_README.md).
Contribution rules (formatting, clippy, commits) are in [`Agents.md`](../Agents.md).

## At a glance

- One crate, `wgpu-cube` (lib `wgpu_cube`), about 76k lines of Rust.
- Stack: wgpu 30 / naga 30, egui 0.36 (+ egui_tiles 0.17), winit 0.30, hecs 0.10 (ECS),
  glam 0.28, mlua 0.10 (Lua 5.4, vendored).
- Requires Rust 1.95 (`rust-version` in `Cargo.toml`).
- Binaries:
  - `editor` (`--features egui`): the scene editor.
  - `player [project-dir]`: runs a saved project; it exits with `Io(NotFound)` if the
    directory has no `project.json`.
  - About 23 examples under `examples/`. Some need the glTF assets in `web/assets/`.
- Run binaries from the repo root. Plugin manifests, built-in scripts and example assets
  are resolved relative to the working directory.
- **Per-machine settings** (`settings.json` in the working directory, gitignored;
  `RenderSettings` in `src/settings.rs`): MSAA, shadow map size, resolution, present
  mode and `monitor` (1-based index, output name like `HDMI-A-0`, or `primary`). The
  `ENGINE_MONITOR` environment variable overrides the monitor. The window is centered on
  that monitor and shrunk to fit; positioning only works on X11, not Wayland.
- Native and web players run Lua gameplay. `build_web.sh --project <dir> --no-serve`
  packages saved projects; the vendored Lua shim requires clang and matching wasm-bindgen.

## Source map

| Path | What lives there |
|------|------------------|
| `src/render_application.rs` | `RenderApplication` trait (the app-facing API) and `run_application`; `DefaultUI` |
| `src/app/core/` | `App` core: `scheduler.rs` (startup/update/GPU stages), runtime modes, render hooks |
| `src/app/platform_winit/` | winit event loop; `mod.rs` holds the per-frame sequence (`RedrawRequested`) |
| `src/app/editor.rs` | Glue between the app loop and egui (`EditorState`) |
| `src/renderer/` | `Renderer` (`renderer_core.rs`), pipelines, shadows, post-processing, `ShaderBuilder` |
| `src/shader/` | WGSL modules composed by `ShaderBuilder` |
| `src/scene/` | `Scene`, `SceneWorkspace`, components, transforms, glTF loading, serialization, snapshots |
| `src/asset/` | Shared `Assets` (meshes, materials, textures) and handles |
| `src/procgen/` | Profile lathes, beveled polygon extrusions, mesh composition, voxel meshes, seeded height fields/PBR maps and PCM sound synthesis |
| `src/physics/` | Rapier physics: `RigidBody`/`Collider`/`CharacterController`/`SpringCamera` components, the per-scene `ScenePhysics`, collider wireframes |
| `src/navigation/` | Navmeshes (feature `navigation`): Recast-style generation, tiled navmesh and queries, the crowd (ORCA avoidance), components and the per-scene `SceneNavigation` |
| `src/ai/` | AI: behaviour trees, state machines, blackboards, expressions, senses, the `AiAgent` component and the per-scene `SceneAi` |
| `src/sky/` | The procedural sky's settings, clock and frame (`ProceduralSky`, `SkyClock`, `SkyFrame`), sun/moon/star positions (`celestial.rs`) and light probe day bakes (`DayBake`) |
| `src/scripting/lua/` | Lua runtime: `state.rs` (`ScriptingState`), `api/` (functions exposed to Lua), `commands.rs` |
| `src/gpu_particles/` | Compute-driven particle systems and behaviours |
| `src/ui/` | egui integration (`egui_integration.rs`), shared windows (hierarchy, stats, log, environment) and widgets (`FineSlider`) |
| `src/project.rs` | Project manifest (`project.json`), scene documents, active project root |
| `src/bin/editor/` | The editor: `application/` (systems, commands, UI), inspector, asset browser, shader/script editors |
| `src/bin/player/` | Standalone project player |
| `src/remote/` | Remote control: TCP server, request hub, built-in handlers, frame capture, log tap |
| `tools/engine-mcp/` | MCP server (separate crate) that exposes remote control to AI agents |
| `examples/scripts/` | Built-in Lua scripts and the editor UI plugin manifest (`ui_plugins.toml`) |

## Frame lifecycle

Each frame, `WindowEvent::RedrawRequested` in `src/app/platform_winit/mod.rs` runs:

1. `sync_runtime_state`: apply any pending Editor ↔ Playing transition.
2. `begin_frame`, then the **update stage** (`scheduler.rs::run_update_stage`):
   - `Playing`: `Scene::update_with_mode(dt, false)`. This runs animations, then game
     scripts, then the physics step (see Physics), and advances scene time.
   - `Editor`: `Scene::run_editor_scripts(dt)`, which ticks only `@editor`/`@tool` scripts.
   - App update systems run (`RenderApplication::update`, plus systems registered on the
     `AppBuilder`), then transforms propagate.
3. **UI** (egui feature only): `EditorState::run_ui_frame` runs the egui frame.
   `RenderApplication::ui` executes here.
4. **GPU stage**: GPU systems run (`RenderApplication::gpu_update`). Workspace requests
   (open/close/swap scenes) are applied, and texture bind groups are rebuilt if needed.
5. **Render**: `Renderer::render` (see below), then egui is drawn on top, then
   `Renderer::present`.
6. If the frame can't be acquired (`renderer::SurfaceError`: occluded, timeout, outdated,
   lost), the frame is skipped. The surface is reconfigured when it is lost or outdated.

`RenderApplication::initial_runtime_mode()` defaults to `Playing`; the editor overrides it
to `Editor`. The editor starts and stops play mode through `RuntimeStateHandle`.

## Rendering

`Renderer::render` (`src/renderer/renderer_core.rs`) runs these passes in order:
skinning → acceleration structures (when ray tracing runs) → light culling → shadow maps → depth prepass
→ ray-traced shadow mask → light probes → fog volume (volumetric fog) → main colour pass →
path tracing → shader-material colour → fog over the opaque scene →
custom stage (`BeforePostprocess`) → transparent pass → post-processing → custom stage
(`AfterPostprocess`) → surface passes (overlay, gizmos) → pick pass (only with a pending
pick) → UI hook. What runs is decided once per frame in a `FramePlan` (`plan_frame`)
before any pass is recorded; passes read it rather than flags left by earlier passes.
The camera is set after the frame's render region and post-processing settings, since
its TAA jitter and texture LOD bias depend on both.

- **Culling** (`renderer/internal/culling.rs`): every instance gets a world-space bounding
  sphere from its mesh's local box (`Mesh::local_bounds`) and transform when the batches
  are prepared; the depth prepass, main, shader-material and transparent passes skip
  instances outside the jittered camera frustum (`OrderedBatch::camera_visible`), and
  each shadow view skips those outside its own. Draws are split into runs of visible
  instances, so the objects buffer layout does not change. A skinned instance's box
  follows its pose: each joint's bind-pose box around the vertices it moves (cached per
  mesh, `culling::joint_bounds`) moved by its palette matrix (`skinned_bounds`). GPU-placed
  instances and shader materials (which may move vertices) are never culled; neither are
  the overlay, gizmo and pick passes, drawn with the unjittered camera.
- **Lights (Forward+, clustered)** (`renderer/lights.rs`, `renderer/light_culling.rs`,
  `shader/light_culling.wgsl`): up to 4 directional lights in a fixed array, and any
  number of point and spot lights ("local" lights, one list, up to `MAX_LOCAL_LIGHTS` =
  4096; beyond it the ones nearest the camera) in the lights storage buffer after a
  512-byte header. The view is cut into clusters: 16 columns, rows that keep tiles about
  square, 24 depth slices logarithmic from 0.5 to 300 m (linear for an orthographic
  camera). One compute dispatch before the shadow pass tests every light's sphere of reach
  (and a spot's cone) against every cluster's view-space box, lights loaded 64 at a time
  into workgroup memory (only those reaching the box around the workgroup's 64 clusters
  are tested one by one), and writes each cluster's lights (in light order, at most
  `MAX_LIGHTS_PER_CLUSTER` = 256) into a shared pool of indices taken with one atomic add
  (`light_lists`: header, an (offset, count) pair per cluster and per grid cell, then the
  pool). The pool's fill is read back without stalling and the pool grows when a frame
  needed more (lights left out meanwhile are counted, `cluster_lights_dropped`). A lit
  surface finds its cluster from its world position (the unjittered view projection and
  view depth in the header, `light_cluster` in `lighting_common.wgsl`) and loops over that
  list only: the main and transparent passes, lit particles and shader materials alike.
  Passes that shade ray hits (reflections, refraction, light probe updates, path tracing,
  global illumination) cannot use the camera's clusters, so the same dispatch also fills a
  world-space grid over the lights' reach (at most 16384 roughly cubic cells, within
  256 m of the camera; `light_grid_cell`) on frames that trace. Reflections and refraction
  loop over a hit's cell with a shadow ray each; light probe updates trace every light of
  the cell when it has at most 4 and otherwise one, picked in proportion to what each
  would add unshadowed (the probes average many updates); the path tracer's primary hits
  take every light of the cell up to 8 and otherwise 4 such picks, and its path vertices
  (and global illumination) one pick among the directional and cell lights. Caustic photons
  come from the 12 lights bringing the most light to the casters. A light without a
  `range` gets one where its inverse-square falloff drops below a small cutoff, and is
  faded out toward it like any other.
- **Directional light shadows** (`renderer/internal/cascades.rs`, `shadows.rs`,
  `shader/shadows.wgsl`): each shadow-casting directional light has up to 4 cascades
  (graphics setting `shadow_cascades`), layers `light * cascades + cascade` of one
  Depth32Float array of `shadow_cascade_size` (2048) texels across. Each frame
  (`ShadowResources::plan_directional`, in `prepare_lights`) the cascades are fitted to the
  unjittered camera: the view depths they cover run from the nearest receiver the camera
  sees to the farthest (sphere bounds of the visible opaque and transparent instances, the
  whole view when one has none), within the light's `shadow_distance` (default 150 m) and
  the camera's far plane, both ends rounded outward in steps of 2^(1/8) so they hold still
  while the view barely changes. That range is split mostly logarithmically (lambda 0.9).
  A cascade covers the sphere around its slice of the view, whose size depends only on the
  projection (kept with 8% hysteresis), in a light space that only rotates with the light,
  centred on whole texels: a world point keeps its place in its texel as the camera moves
  or turns, so edges do not shimmer. Its depth range reaches from the sphere's far side
  back past the caster nearest the light (rounded in coarse steps); casters nearer the
  light than that (no bounds) are flattened onto the near plane by the shadow vertex
  shader. Casters under half a texel are left out. A cascade's map is kept while its view
  and casters stay; when only dynamic casters changed (as for local faces, below) it is
  rebuilt from a static-casters copy in a second array, restored only in the texel
  rectangle the dynamic casters covered last frame or cover now (a scissored
  fullscreen-triangle depth copy), then those casters are drawn; when the camera or light
  moved it, it is drawn anew. The two farthest cascades take turns while the camera moves
  (each keeps last frame's view every other frame). The shaders pick, per point, the
  nearest cascade whose map holds it (inside its square, with room for the filter, and
  before its far plane) rather than by view depth, so a cascade a frame behind still
  serves what it covers; the last few percent of a map blend into the next cascade, and
  shadows cut off by the shadow distance fade out over its last tenth. Filtering is
  percentage-closer soft shadows: 12 point taps look for casters within the light's
  apparent size (`rt_shadows.softness`, the same sun size the ray-traced shadows use)
  times 32 m, then 16 bilinear compare taps over the penumbra those casters give (1 to 16
  texels), both on a Vogel disc turned per pixel and frame for TAA to average, with a
  receiver-plane depth slope from the normal against acne under wide filters; points with
  no casters (or only casters) in reach skip the filter. `engine_stats` shows the splits
  (`renderer.lights.cascade_splits`) and the GPU stage `shadow_cascades`.
- **Local light shadows** (`renderer/internal/shadow_atlas.rs`, `shadows.rs`,
  `shadow_casters.rs`): every point and spot light's shadow maps are square tiles of one
  Depth32Float atlas (six cube faces for a point light, one for a spot light), rendered
  with a 2-texel border so the 3x3 filter's taps stay in the tile. Each frame the
  shadow-casting lights the view can reach are scored (`shadow_priority` x the pixels their
  sphere of reach covers x brightness, a little more for lights that held tiles) and ask
  for tiles of a power of two near their size on screen (128 up to `shadow_map_size`, at
  most an eighth of the atlas); when the atlas cannot hold them the lowest scores shrink,
  then go without (unshadowed; at most `MAX_SHADOWED_LOCAL_LIGHTS` = 64 have maps). Tiles
  come from a buddy allocator and stay put while a light's size class stays (a light that
  leaves the view keeps them for 600 frames unless another needs the room; a light that is
  gone frees them at once); the atlas grows by powers of two with demand up to the
  graphics setting `shadow_atlas_size` (8192 natively, 4096 on the web), and shrinks (or is
  released) once the lights have needed at most half of a smaller one for 600 frames. A
  face is kept in two parts: its static casters' depth in a second atlas of the same
  layout, and the map itself, rebuilt from a copy of that (a
  fullscreen-triangle depth copy: WebGPU cannot copy part of a depth texture) plus the
  dynamic casters (skinned, GPU-placed, without bounds, or changed within the last 60
  frames) in its view. Static depth is rendered again only when a static caster near the
  face changes (appears, goes, starts moving, settles), found by comparing the casters
  with last frame's rather than hashing each face's casters every frame. Faces the camera
  cannot see (their frustum and the camera's do not meet) and faces with nothing new are
  kept. At most `shadow_updates_per_frame` (48) faces are rebuilt a frame: faces never
  rendered first, then any that waited 4 frames, then by their light's score; the others
  keep last frame's map (`shadow_views_deferred`). With ray-traced shadows the 28
  highest-ranked shadow casters take bits of the mask instead, and their tiles are skipped
  when only the main pass reads the maps. The renderer keeps each light's state for
  `Renderer::light_shadow_state` (the inspector's light section, `entity_get`'s `shadow`).
- **Opaque order**: opaque batches draw front to back by their nearest instance, then by
  mesh, so the prepass rejects hidden fragments early and coplanar surfaces resolve the
  same way every frame (the batcher's map has no order of its own).
- **Depth prepass**: opaque PBR batches, cut-out surfaces through an alpha-tested
  pipeline (`depth_prepass_masked.wgsl`), so the ray-traced shadow mask traced from this
  depth sees them. Shader materials (their own vertex stage may move vertices) write their
  depth in the main pass.
- **Directional shadow maps are cached** (`renderer/internal/shadows.rs`): each map
  remembers a hash of what it was rendered from (the light's view projection and every
  caster's geometry identity, transform, shadow flag and, for cut-outs, material and
  texture generation, order-independent); a frame whose hash matches keeps the map.
  Skinned or GPU-placed casters, and custom shadow passes, render every frame. Every
  rendered view's view projection (directional layers and local faces) goes into one
  uniform buffer bound at a dynamic offset. Custom shadow passes get the atlas tile as
  `CustomRenderContext::shadow_viewport` for local faces.
  `RendererStats::shadow_views_rendered` / `shadow_views_cached` /
  `shadow_views_deferred` show the split, `lights` the lights and shadow budget, and
  `culled_instances` the culling.
- **Acceleration structures are kept** across frames with the same instances (geometry,
  transforms, masks, shader-side entries) and no BLAS to build (`raytracing/scene.rs`,
  `instance_signature`); otherwise the TLAS is rebuilt as before.
- **TLAS instance masks** (`MASK_*` in `raytracing/scene.rs`, `RT_MASK_*` in
  `rt_scene.wgsl`) separate opaque surfaces, opaque surfaces that cast no shadow
  (`cast_shadow: false`), transmissive surfaces and particles. Reflection, refraction,
  probe and path-tracer bounce rays see all surfaces; shadow and ambient occlusion rays
  pass the surfaces that cast no shadow, so a mesh left out of the shadow maps is left
  out of the ray-traced shadows too (a light inside a lantern that casts no shadow still
  gets out).
- **Shader materials** fill the G-buffer from their own shader: a generated
  `fs_main_gbuffer` wraps the user's `fs_main` (`renderer/internal/shader_material.rs`)
  when it takes a struct with `world_pos` and `normal`; motion vectors come from the
  camera when the module's `globals` has `unjittered_view_proj`/`prev_view_proj`. Other
  shapes fall back to the PBR shader for their G-buffer and draw their colour in the
  shader-material pass. That pass also redraws every shader material when path tracing
  replaced the opaque lighting. With `// @include_lighting` a shader material reads the
  lights through `scene_lights_at`, `scene_light_count` and `scene_light`
  (`lighting_common.wgsl`: the directional lights, then its cluster's point and spot
  lights), not the lights buffer's arrays.
- **Cut-out surfaces** (`Material::alpha_cutoff`, glTF `MASK`) are opaque surfaces that
  discard below the cutoff: in the depth prepass, the main pass, the pick pass, shadow
  maps (an alpha-tested shadow pipeline) and ray queries (non-opaque BLAS; every query loop confirms candidates
  through `rt_candidate_blocks` in `postprocess/rt_alpha.wgsl`).
- **Double-sided materials** (`MaterialFlags::DOUBLE_SIDED`, glTF `doubleSided`, which
  Blender exports by default) are drawn without back-face culling in every pass: the
  render object's `cull_mode` comes from the material, and the depth prepass and shadow
  maps have a no-culling pipeline of each kind (`depth_prepass(double_sided)`,
  `ShadowResources::pipeline[1]`). `shade_fragment` flips the normal on back faces
  (`@builtin(front_facing)`), so they are lit from the side the camera sees.
- **Transparent sorting** is per instance across all transparent batches; consecutive
  instances of one batch share a draw.
- **Interior mapping** (`MaterialFlags::INTERIOR`, `src/shader/interior.wgsl`) turns a
  surface into windows onto rooms traced in the shader: each uv unit is a room (the
  integer part picks it by hash with the object's position: image, mirroring, lamp on or
  off and its colour, curtains or a blind), the fraction is where the view ray enters a
  box behind the glass `interior_depth` room widths deep. The emissive texture holds the
  room images (one-point perspective views with the back wall filling the middle half, in
  an `interior_atlas` grid), lit by the emissive strength; base colour texture alpha marks
  the frame. The room box's axes come from the surface's uv frame (world change per uv
  unit), from screen derivatives in `shade_fragment` and from the hit triangle's edges in
  `rt_material`, so rooms keep their proportions whatever the mesh's tangents and every
  ray-traced pass (reflections, probes, GI, path tracing) sees the rooms the main pass
  draws. The four settings ride in `MaterialData.interior` (`Material::interior_bits`);
  the room images are sampled with explicit gradients (`sample_emissive_texture_grad`),
  since their uvs jump at the room's corners.

The scene renders in HDR (`Rgba16Float`, `postprocess::HDR_FORMAT`), transparent
surfaces included, so light above 1.0 survives to bloom and tonemapping; scene shaders
write linear radiance and leave exposure and tonemapping to post-processing. The G-buffer
holds the octahedral normal and motion (`GBUFFER_NORMAL_FORMAT`) and the world position,
whose `w` is 0 for background and on a surface at least 1, with the smoothness
(1 − roughness, 0 when unlit) and specular colour F0 packed into the half float's bits
(`encode_surface_w` in `common.wgsl`); the colour target's alpha is -transmission on
transmissive surfaces drawn in the opaque pass for ray-traced refraction. Post-processing
(`src/renderer/postprocess/`) runs: MSAA depth resolve → ray-traced refraction (when glass
is on screen) → reflections (half-res, or full with `ssr.full_resolution`,
trace through the G-buffer positions, or ray traced with `ssr.ray_traced`, accumulated
over frames, separable blur, added to the HDR scene weighted by smoothness² and Fresnel
from F0, minus the flat ambient a found reflection replaces on metals) → TAA (Halton-jittered camera,
closest-depth motion vector, Catmull-Rom history, YCoCg variance clipping) → motion blur
→ SSAO → bloom (13-tap Karis downsample chain, tent upsample blended by scatter) → depth
of field (half-res CoC prefilter, golden-angle bokeh gather) → auto exposure (compute:
luminance histogram, adapted EV in a storage buffer) → composite (AO, DoF, bloom,
exposure, vignette, white balance and grading, tonemapping, lift/gamma/gain, `.cube`
LUT; into a half-float display target) → final (FXAA, CAS sharpening, film grain,
dither; onto the surface). Later passes read the TAA/motion-blur output when those run.
Overlays and gizmos draw after that, untonemapped, through an unjittered camera
(`CameraBuffer::stable_bind_group`).
Screen-space reflection and ambient-occlusion projections use the same TAA jitter
as the G-buffer rasterization; motion vectors continue to compare unjittered
matrices. Using a stable projection to look up jittered surfaces causes shading
to flicker, especially at a distant horizon.
- **Procedural sky and day/night** (`src/sky/`, `src/renderer/internal/sky.rs`,
  `src/shader/sky.wgsl`, `sky_generate.wgsl`): an option of the scene's environment
  (`EnvironmentComponent::sky`, a `ProceduralSky`; off unless a scene enables it), which
  stands in for the HDR image while enabled. Its settings hold a date, local clock time,
  latitude, longitude, UTC offset and north heading; `sky::celestial` places the sun
  (Astronomical Almanac formulas) and the moon (the largest terms of Meeus ch. 47, with
  parallax and refraction) for them, in the engine's compass: north is -Z and east +X at
  north heading 0. The scene's `SkyClock` runs the time on (play mode, or the editor's
  preview) at `day_length` real seconds per day, turning the date over, and starts over
  whenever the date or time is set; a light probe day bake holds it at an hour.
  `Scene::sky_frame` gives a `SkyFrame` each frame: the sun's and moon's directions and
  transmittance (the same atmosphere marched on the CPU), the moon's phase, the stars'
  rotation (`world_from_equatorial`) and the light directional lights with
  `follow_sky` take: the sun by day, from 3° under the horizon the moon (fading in as the
  dusk deepens), nothing when both are down. The light is overridden when the lights are
  collected (`scene/internal/lights.rs`); the entity's rotation, colour and intensity are
  left alone and apply again without the sky. The render bridge hands the frame to the
  renderer (`Renderer::set_sky`): its settings ride in `EnvironmentUniform::sky`
  (`SkyParams` in sky.wgsl, at the environment's binding 8), and `SkyResources` renders
  a transmittance table (256x64, Bruneton's layout) and a multiple scattering table
  (32x32, Hillaire 2020) when the air or the ground change, and the sky into a 1024x512
  equirectangular environment map with box-filtered mips whenever the sun or moon moved
  (the map replaces the HDR image for IBL, ray-traced reflections and misses, probe
  updates and the path tracer). The scattering is Rayleigh and Mie single scattering with
  ozone, marched along the view (32 samples, as O'Neil's GPU Gems 2 chapter models it
  without his horizon-limited fit) plus the multiple scattering table, for the sun and the
  moon, with the lit ground below the horizon. The background (`environment_background.wgsl`)
  samples the map and adds per pixel what is too sharp for it: the sun's limb-darkened disc
  (capped below half-float range), the moon's disc shaded in its phase by the sun with
  procedural maria (its face turned toward the celestial north), and stars (four cube-cell
  layers of procedural stars with a magnitude power law, sized to the pixel's footprint)
  with the Milky Way along the galactic plane, dimmed by the air's airmass. Nights are
  dark by default: moonlight is 1:200 of sunlight (the real ratio is about 1:400,000), so
  the moonlit side of things can be made out while lamps and windows carry the scene;
  stars and the Milky Way fade in from sunset to 12° under the horizon, and the moon's
  disc is drawn at a fixed share of its light (a pale disc by day). How dark a night
  looks depends on how far auto exposure may adapt: `min_ev` about -4 keeps nights dark
  and dusk visible; lower values (and more `moon_intensity`) make brighter nights.
  `sky.get`/`sky.set` (MCP `sky_get`/`sky_set`), Lua `sky_info()` and
  `set_time_of_day(hours)`, and the environment section of the inspector control it.
- **Fog** (`src/fog.rs`, `src/renderer/fog.rs`, `src/shader/fog*.wgsl`): an option of the
  scene's environment (`EnvironmentComponent::fog`, a `Fog`; off unless a scene enables
  it). It is height fog: extinction `density` up to `height`, falling off exponentially
  above it (`height_falloff`, the height over which it falls to 37%; 0 keeps it uniform),
  scattering by `albedo` with a Henyey-Greenstein phase (`anisotropy`), plus `emission`.
  Near the camera it is volumetric when the graphics setting `volumetric_fog`
  (`VolumetricFog`: off, low, medium, high) is on: a froxel volume follows the unjittered
  view (a froxel per 16, 8 or 6 pixels of the scene's viewport, at most 320x240, and 32, 64
  or 96 slices of view depth out to the fog's `volumetric_distance`, slice w starting at
  depth far·w², so near slices are thin). After the shadow maps, light culling and light
  probes, `cs_fog_scatter` (fog_volume.wgsl, a compute pass that binds the lights group,
  visible to compute shaders for it) lights one point per froxel, jittered inside it from
  frame to frame (an R3 sequence started per froxel by a hash): the directional lights
  through one filtered compare in their nearest cascade, the point and spot lights of the
  point's cluster (`light_cluster`) through one compare in their atlas tile, with their
  inverse-square falloff held at 0.2 m, and the ambient light (the light probes' mean from
  above and below where a probe volume covers the point, else the flat ambient plus the
  environment map's lowest mip); then blends it with the froxel's last value where the
  froxel centre was last frame (85% kept; restarted on camera cuts, a new volume size or
  distance, and shader reloads). The scattering direction comes from the froxel's view ray,
  not the point: a point in the first slice can round onto the camera itself, and a
  degenerate direction made the phase function blow up into single bright froxels.
  `cs_fog_integrate` walks each column outward (Hillaire's energy-conserving integration)
  into an integrated volume: the light scattered toward the camera and the transmittance
  up to each slice's far end. After the opaque passes (path tracing and shader materials
  included, before the custom stage, particles and transparent surfaces), `fs_fog_apply`
  (fog_apply.wgsl) blends it over the scene from the depth buffer (colour × transmittance
  + light, the blend state keeping the alpha that marks glass; sample 0 of the depth with
  MSAA); the background gets the fog along its view ray out to `max_distance`. Past the
  volume, and everywhere without it, the fog is the height fog's closed form
  (`fog_optical_depth`, split at the base height), lit by the ambient light and the
  directional lights without shadows. Transparent surfaces (`fs_transparent`, the PBR
  shader's entry point for the transparent pass: `PipelineKey::fog`) and GPU particles fog
  themselves through `fog_apply` / `fog_at` (fog.wgsl, lights group bindings 16-17; additive
  particles are only dimmed); reflections and ray-traced refraction, added after the fog
  pass, are dimmed by the transmittance to their surface (`fog_transmittance_at`; the
  reflection apply pass binds the uniform and volume at 164-165). Volumetric fog reads the
  shadow maps, so they are rendered even where ray-traced shadows replace them for the
  main pass (`FramePlan::only_main_pass_reads_shadow_maps`). Shader materials and the
  custom stages are not fogged. GPU stages `fog` and `fog_apply`, stats
  `RendererStats::fog_froxels`. At a 920x600 scene view (Lantern Market, 20 shadowed
  lanterns, 5 probe volumes): 0.07 / 0.15 / 0.29 ms for low / medium / high, the apply
  pass 0.02 ms.
- **Light probes that follow the time of day** (`src/renderer/light_probe_keyframes.rs`):
  a light probe bake with `times_of_day` (or the inspector's "Bake day") holds the sky
  at each hour in turn while every volume bakes there (`sky::DayBake`, stepped by
  `app::core::light_probes`), then writes each volume's `.probes` file as version 4: the
  hour nearest noon's region (distances, offsets, open/inside flags, and the light shown
  without a sky) followed by each hour's irradiance tile rows as RGB9E5. Loaded, the hours
  go into a 2D array texture, and each frame the renderer blends the two around the sky's
  hour into the volume's tile rows (`blend_keyframes`, before the passes that read the
  probes; only when the blend changed), for volumes that show their bake (no ray tracing,
  `use_baked`) and for live ones until their first update lands on top. Light adds up, so a
  blend of two hours' light is close to the light between them; the default hours follow the
  sun's path (midnight, noon, and the sun at -6, -1, 4, 12 and 30 degrees rising and
  setting).
- **HDR display output** (`src/renderer/hdr_output.rs`), opt-in through `settings.json`
  (`"hdr_output": "off" | "auto" | "simulate"`, `"hdr_paper_white_nits"`,
  `"hdr_peak_nits"`) or the `ENGINE_HDR` environment variable. `auto` picks scRGB
  (`Rgba16Float` + extended linear sRGB), else HDR10 (`Rgb10a2Unorm` + BT.2100 PQ), else
  logs a warning and stays SDR. In HDR mode the final post pass, overlays and the
  `AfterPostprocess` stage draw into a half-float composition texture (linear, 1.0 =
  paper white; `Renderer::output_format`), egui draws into its own 8-bit layer
  (`ui_format`/`ui_view`), and `PresentStage` composites both and encodes for the display
  when the frame is presented or captured (`Renderer::resolve_output`). Tonemapping then
  rolls highlights off toward the display's peak instead of 1.0; lift/gamma/gain and the
  LUT grade the paper-white range. `simulate` runs that path on an SDR surface, which is
  how it is tested on machines without an HDR display; the scRGB/PQ encoders have unit
  tests but have not been seen on an HDR screen.
- **G-buffer**: colour (HDR), normal, world position and (while picking) the pick id —
  exactly WebGPU's 32-byte colour-attachment limit, so motion vectors share the normal
  texel: octahedral normal in xy, screen motion (UV per frame) in zw. Motion vectors
  compare `CameraUniform::unjittered_view_proj` with `prev_view_proj`, and each
  instance's model matrix with last frame's (`prev_models`, group 1 binding 2, matched
  by pick id + mesh).

- **Shadow sampling** (`shadows.wgsl`, `lighting_with_shadows.wgsl`) uses
  `textureSampleCompareLevel` (the maps have one mip, so it equals the implicit-LOD
  sample) and so needs no uniform control flow: a light that cannot reach a fragment
  (facing away, out of range, outside the cone, no shadow data, or already answered by
  the ray-traced mask) is skipped before its PCF taps.
- **Bind groups** for the main pipeline: 0 = camera, 1 = objects, 2 = lights,
  3 = textures. Group 2 holds the lights buffer (0) and the cluster lists (15), both
  storage, the shadows uniform (1: directional shadows and the local shadow slots' face
  matrices and atlas rects), the directional shadow layers (2), one comparison sampler
  (3), the local shadow atlas (4), the environment (8-10), the ray-traced shadow mask (11),
  the light probes (12-14) and the fog (16: uniform, 17: integrated froxel volume): 2
  storage buffers and 7 sampled textures, so the main pass stays within WebGPU's per-stage
  limits (4 storage buffers, 12 sampled textures with the traditional texture bindings).
  It is visible to fragment and compute shaders (the fog volume's lighting). Group 3 is bindless (`binding_array`, up to `MAX_TEXTURES = 256`) when the
  adapter supports it. Otherwise it is "traditional", with one binding per material slot.
  Browsers never offer bindless textures, so every web build takes the traditional path;
  GPU particle systems bind their own material's textures there
  (`Renderer::material_textures_bind_group`). `WGPU_CUBE_FORCE_TRADITIONAL_TEXTURES=1`
  forces that path natively for testing.
- **Web asset requests** (`src/io.rs`) percent-encode each file path (`percent_encode_path`),
  so a file name holding a literal `%2e` is requested as `%252e`. glTF URIs are already
  encoded and are decoded before they are joined to a path.
- **GPU readbacks never block** (`gpu_particles/system/readback.rs`): a browser cannot wait on
  the GPU, so the particle dead list and the alpha-sort copy are recorded in one frame, mapped
  in the next and consumed once the map callback has run.
- **`ShaderBuilder`** composes WGSL from the modules in `src/shader/`. It patches
  constants and expands markers such as `// @include_lighting`.
- **Surface**: the swapchain uses an sRGB format but also registers a non-sRGB
  `view_formats` entry. egui renders through `Renderer::ui_view()` (the linear view)
  because it does its own gamma handling; the sRGB view makes the UI look washed out.
- **Render region**: `RenderRegion` restricts the scene to a viewport rectangle (the
  editor's Scene/Game panes).
- **Custom rendering**: apps hook in through `RenderApplication::custom_render` and
  `custom_render_stage`.
- **Picking**: on a frame with a pick request (`request_pick`), a pick pass draws every
  pickable batch's entity id (opaque, transparent, overlays, then gizmos) into a
  single-sample `Rg32Uint` target at output resolution through the unjittered camera,
  whatever the MSAA sample count or upscaling, and one texel is read back asynchronously
  (`poll_pick_result`; `renderer/internal/pick.rs`). Scene passes do not write ids.
- **Post-processing settings** (`PostProcessSettings`) come from `PostProcessVolume`
  components: each frame `AppCore::render_scene` blends the scene's enabled volumes at
  the camera (`scene::blend_post_process_volumes`, lowest priority first; infinite
  volumes apply everywhere, box/sphere ones fade over `blend_distance`) and hands the
  result to `Renderer::set_postprocess_settings`. A scene without volumes gets the
  defaults. Projects saved before volumes stored effects in `project.json`
  (`postprocess`) and grading in the environment; `SceneAsset::migrate_legacy_post_processing`
  turns those into an infinite volume entity on load, and they are no longer written.

## Scenes, assets and projects

- **`SceneWorkspace`** (`src/scene/workspace.rs`) holds the open scenes (editor tabs). It
  tracks the active one and owns the shared `Assets` cache.
- **`Scene`** wraps a hecs `World`, plus the scene graph, camera, environment, gizmos,
  imports and the scripting runtime. Hierarchy uses `Parent`/`Children` components.
  `TransformComponent` holds the local transform; `WorldTransform` is derived by
  `propagate_transforms`.
- **Snapshots**: `SceneStateSnapshot` captures a whole scene (assets, environment,
  camera, tree). The editor takes one when entering play mode and restores it on Stop.
  Undo history (`src/bin/editor/history.rs`) also stores full snapshots, up to 64.
- **Serialization**: a scene exports as a `SceneTreeAsset`; script components serialize as
  `SerializedLuaScript`.
- **Project layout on disk**:
  ```
  <project>/project.json                      # ProjectManifest: metadata, scene list, environment, editor camera
  <project>/content/scenes/<id>/scene.json
  <project>/content/scripts/*.lua
  <project>/content/materials/*.mat.json
  <project>/content/environment/*.hdr
  ```
- **Active project root**: `project::set_active_project_root` stores it in a global, and
  `resolve_project_path` resolves relative asset paths against it. Code handed a project
  root (saving a scene's materials) must resolve against that root, not the global: the
  global still names the previous project while another one opens.
- **Exports carry only used content** (`src/export.rs`): `used_content` follows
  references from the main scene (the first in `project.json`): prefab `scene_ref`s,
  every string in scene/material/glTF/import JSON that names a project file, a model
  import's whole package folder, and Lua string literals (files, `require` modules, and
  folders named as literals, for run-time paths). `export_content` copies them with a
  `project.json` trimmed to the used scenes. The editor's Build menu (after saving),
  `build_web.sh --project` (through `player --export-content`) and MCP `web_build` use
  it; remote `project.content` reports it.
- **Asset library** (`src/library.rs`, `tools/library/`, `docs/asset_library.md`):
  characters, animations and models shared by all projects in
  `<user data dir>/wgpu-cube/library` (outside git; Mixamo files must not be
  redistributed). `library.py` fills it (CC0 packs, the Mixamo web API with the user's
  token, own files) through Blender (`blender_ingest.py`: one asset per animation,
  measured, with a Workbench thumbnail). The editor's Library pane and remote
  `library.search/info/add` read `library.json`; adding copies an animation into
  `content/animations/<name>.glb` (onto a character's animator) or imports a model.
- **glTF**: `SceneLoader` / `src/scene/loader/` import glTF. Each texture is sampled as sRGB
  when a material uses it for colour (base colour, emissive) and as stored when it holds
  data (normal, metallic-roughness, occlusion), embedded or not (`srgb_textures`).
  Animation (`scene/animation.rs`) drives node transforms and material properties with
  step, linear or cubic-spline keyframes, bound to the entities a file was imported with.
- **Skeletal animation** (`src/scene/skeletal/`):
  - Skinned glTF primitives keep `JOINTS_0`/`WEIGHTS_0` as a second stream on `MeshData`
    (`skin: Vec<SkinVertex>`, 16 bytes per vertex, four influences, weights quantized to
    sum exactly to 65535). Joints are ordinary entities; `SkinnedMesh` on the mesh entity
    lists them with their inverse bind matrices (scaled with the import scale).
  - Each frame `build_render_objects` computes a joint palette per skinned entity
    (`inverse(mesh_world) * joint_world * inverse_bind`) and hands it to the batcher as a
    `SkinBinding`. Before the shadow pass, `renderer/skinning.rs` deforms every skinned
    instance into a vertex buffer of its own (compute shader `shader/skinning.wgsl`, or
    the identical CPU `skin_vertices` on devices without compute/storage buffers;
    `WGPU_CUBE_FORCE_CPU_SKINNING=1` forces it). Each instance is its own batch, and all
    passes draw it like a rigid mesh, so no vertex shader knows about skinning. Each
    instance has two output buffers that take turns, so last frame's pose stays
    available: the main pass reads it as a second vertex stream (`@location(5)`,
    `Vertex::previous_position_layout`; the mesh itself for rigid batches), and motion
    vectors include limb motion. Ray tracing copies the deformed vertices into its pool.
  - Skinned imports get an `Animator` on their root (also files without animations, so
    a T-pose download can take clips from others; its own file stays the first library).
    It plays `SkeletalClip`s read from glTF/GLB files (`ClipLibrary`, cached in
    `Assets::clip_library`), bound to bones by normalized name, or failing that by
    humanoid role (`humanoid_key`: Mixamo, Unreal-mannequin and Rigify names map to one
    set of roles), so animation-only files drive any humanoid rig. Layers with bone masks,
    cross-fades, per-bone
    translation retargeting (a key's offset from the clip file's rest, scaled by the
    rest-length ratio, moves the bone from its own rest) and root motion
    (`keep`/`in_place`/`apply`). Animators run only while
    `Scene::set_animation_playback(true)` (play mode), so the editor saves rest poses.
  - Rest-relative retargeting (`Binding::finish_frames`): a clip from another export of
    the rig (a Mixamo FBX through Blender, whose bones point along Y, on a GLB that kept
    Mixamo's joint orientations and turns its armature -90 degrees) has its keys turned
    `pre * key * post`, with `pre = T(parent)⁻¹ F⁻¹ S(parent)` and
    `post = S(bone)⁻¹ F T(bone)` from the world rest rotations in the clip's file
    (`BoneRest::parent_rotation`) and in the model's own file (found through the
    animator's `GltfSource`, else read from the entities), and `F` the turn about Y that
    makes the model's file face the way the clip's file does (`skeleton_facing`, from the
    rest positions of the left and right legs: the three.js Soldier faces -Z, Mixamo
    files +Z). Each bone then turns away from its rest as in the clip, in the character's
    own frame. Identical rigs get no correction; different rest postures (T versus A)
    are not handled.
  - Blends (`Animator::play_blend`): a group of clips posed by weight with one shared
    phase (a walk and a run keep step; a clip with `sync` off, such as an idle, keeps its
    own), negative speed plays backwards (the same clip forward and backwards plays as
    two), each clip's `root_speed` (ground travel per second) is measured from its root
    track.
  - `AnimationTree` (`skeletal/tree.rs`, Godot-style): states playing a clip or a 1D/2D
    blend space (2D: the smallest enclosing triangle, barycentric weights), transitions
    with parsed conditions (`&& || !`, comparisons) and exit times, parameters fed from
    the `CharacterController` after the physics step (`ScenePhysics::feed_animation_trees`:
    speeds along the facing, grounded, air time, landing speed, jumped/landed triggers).
    `update_trees` runs before `update_animators` and plays the current state through
    `play_blend` on layer 0. `AnimationTree::locomotion` builds one from clip names
    (Mixamo's included), placing blend points at the clips' root speeds and recording
    the animator's world scale (`measured_scale`; speed blends scale with a clone's
    size). In speed blends the point at zero is unsynced and strides slow to 60% of
    their pace before it blends in (`slow_strides`). `tree_animator`
    finds the animator a tree drives (itself, below, above); `animator_entity` finds the
    one a script means (itself, above, below).
  - Copy paths remap joints: `SceneAsset::instantiate` (entity indices),
    `merge_world_as_child` and the remote duplicate. Lua: `animator_*`, `animation_*`,
    `find_bone`. MCP: `entity_get`/`entity_update {animator, animation_tree}`,
    `animation_inspect`, `debug_skeleton`, `character_make`.
  - FBX, .blend and OBJ import through Blender (`src/bin/editor/application/blender_convert.rs`
    runs `tools/blender/convert_to_gltf.py` in the background): a model becomes a GLB
    imported like any glTF; a file without meshes (Mixamo "Without Skin"), or one imported
    as animations, becomes a clip library in `content/animations`, named after the file
    (Mixamo names every clip "mixamo.com"). Blender is found through `BLENDER_PATH`, the
    `PATH` or its usual install places.
  - "Make Character" (`physics::make_character`, also remote `character.make`): the
    model goes under a new entity with a `CharacterController` sized from its bounds
    (skinned meshes measured through their joints: `joint × inverse bind`), a generated
    tree, a `SpringCamera` on the active camera and optionally a script requiring
    `engine.third_person` (a built-in Lua module, `scripting/lua/modules/`). A model whose
    toes point to -Z is turned to face +Z.

## Physics

`src/physics/` integrates [Rapier](https://rapier.rs) 0.36 (pure Rust, so it builds for
the web too).

- **Components** (saved with the scene, edited in the inspector's Physics menu, over the
  remote API and through Lua `set_component`): `Collider` (shape; alone it is fixed
  scenery), `RigidBody` (dynamic, fixed or kinematic), `CharacterController` and
  `SpringCamera`. `physics::edit` reads and patches them as JSON for both the remote API
  and Lua.
- **`ScenePhysics`** (one per `Scene`, main world only, like scripts) is derived state:
  nothing in it is saved; `reset_script_runtime` (play start) clears it and it is rebuilt
  from the components, and a scene restored on Stop gets a fresh one. It steps only in
  play mode, in `Scene::step_physics` after the scripts: it takes the physics commands
  scripts queued (`ScriptCommands::physics`), syncs bodies with the components, steps,
  moves characters, writes back, places spring cameras, and hands scripts their callbacks
  for the next frame (`ScriptingState::queue_callbacks`, only for entities with scripts).
- **Sync**: each wanted entity's body is keyed by a signature (role, component values,
  world scale, mesh handles and, for `include_children`, the descendants' meshes and
  relative transforms); a changed signature rebuilds the body, keeping its velocity.
  Fixed and kinematic bodies follow the entity's transform (computed through the parent
  chain, so it is right before transform propagation); dynamic bodies and characters
  remember the local transform they last wrote, and anything else there is a teleport.
  Collider user data is the entity's bits.
- **Stepping**: a frame runs up to 6 steps of at most 1/60 s; kinematic bodies and
  characters glide to their target over them (`set_next_kinematic_position`), so pushed
  bodies see smooth motion. Never call `PhysicsWorld::detect_collisions` after inserting
  bodies: it consumes their "modified" flags without registering them with the island
  manager, and new dynamic bodies then never move.
- **Characters** (`character.rs`) move once per frame after the step, by move-and-slide
  (Rapier's `KinematicCharacterController`, autostep and snapping off) with a *floating*
  capsule that starts `step_height` above the feet; a sphere swept down from it finds the
  ground (a steep edge normal is checked with a short ray past the contact), holds the
  feet on it, snaps them down slopes and stairs, and climbs steps at a capped speed.
  Rapier's own autostep only nudges the capsule by what is left of a frame's movement, so
  it failed on ordinary stairs at high frame rates. The body's collider is the full
  feet-to-head capsule, which is what sensors, props and scene queries meet.
- **Spring cameras** (`spring_camera.rs`) orbit a pivot above the target, smooth it per
  axis, take mouse look from `InputState` (while `cursor_locked` or the right button is
  held) and sweep a ball from the pivot to pull in before fixed scenery; they insert a
  `LookAtTarget`, so directional shadows stay centred on the character.
- **Cursor capture**: Lua `set_cursor_locked` sets a global request
  (`input_state::request_cursor_lock`); the winit loop applies it while playing
  (`apply_cursor_lock`: `Locked`, else `Confined` plus re-centring on X11), feeds raw
  `DeviceEvent::MouseMotion` into the mouse delta while captured, puts the cursor back
  where it was captured on release (drag-to-look), and releases it on Escape (even when
  egui takes the key), focus loss or leaving play.
- **Mouse in the editor**: the game's view is an egui panel, so egui takes every pointer
  event over it. While playing, the winit loop still hands the game cursor movement,
  presses over the game's view (or with the cursor captured), the releases of those
  presses and the wheel over the view (`game_takes_pointer`); injected remote input skips
  this path, so test real mouse handling by hand.
- **Facing**: characters turn toward their movement unless a script set a facing
  (`character_face`, `CharacterRuntime::facing`), which is how strafing, backpedalling
  and mouse steering work.
- **Debugging**: the editor outlines the selected entity's collider (built from the
  components); `ScenePhysics::set_debug_draw` (Lua `physics_debug_draw`, remote
  `debug.physics`) draws every simulated collider, play mode included.

## Navigation

`src/navigation/` (cargo feature `navigation`, on by default; every use outside the module is
`#[cfg(feature = "navigation")]`, so `--no-default-features` must keep building). The user
guide is [`docs/navigation.md`](navigation.md).

- **Generation** (`recast/`, `builder.rs`, `geometry.rs`): a port of Recast's pipeline, one
  tile at a time on the thread pool. `NavGeometry` collects triangles (render meshes and/or
  fixed colliders, `NavModifier` areas) and marks slopes; each tile rasterizes them into a
  heightfield of spans per column (several floors per column), filters it (low obstacles,
  ledges, low ceilings), builds the compact heightfield, erodes it by the agent radius,
  partitions it into watershed regions, traces and simplifies contours, triangulates them
  into convex polygons and adds height detail. Spans keep the exact height of their top
  (`Span::top`), so agents stand on the floor, not up to a voxel above it.
  `FORBIDDEN_AREA` (a `NavModifier` with `walkable: false`) marks spans the low-obstacle
  filter must not make walkable; plain `NULL_AREA` tops (too steep) may still be stepped
  onto when they are within a climb of the floor, like a curb.
- **Navmesh** (`navmesh.rs`, `query.rs`): Detour-like tiles with `PolyRef` (salt, tile,
  poly), internal and external links with portal ranges, off-mesh connections as two-vertex
  polygons. Queries: A*, funnel straight path, corridor smoothing by raycasts, raycast,
  move along surface, nearest polygon, random point, wall segments. A rebuilt tile gets a
  new salt, so stale refs fail `is_valid`.
- **Tile cache**: a bake can keep each tile's compact heightfield; carving obstacles
  rebuild only the tiles they touch from it (carves are applied before erosion, so holes
  grow by the agent radius), at most `TILE_REBUILDS_PER_FRAME` a frame.
- **Crowd** (`crowd.rs`, `avoidance.rs`): Detour-crowd-like corridors (merge on move,
  visibility shortcuts), corners and smooth steering, separation, ORCA (an RVO2 port with
  the navmesh's wall segments as obstacles of a 5 cm radius: the navmesh already keeps the
  agent radius from walls, so padding them again would close gaps the navmesh leaves open),
  collision resolution and moves along the surface. A keep-to-one-side lean when someone
  comes the other way, and giving way when stuck face to face, break the symmetric
  deadlocks ORCA cannot. Replanning happens when the corridor turns invalid, a partial path
  times out, the mesh generation changes, or an agent makes no progress.
- **`SceneNavigation`** (`scene_nav.rs`, one per `Scene`, derived state like
  `ScenePhysics`): loads bakes (a process-wide cache keyed by path and file stamp, shared
  by play sessions), builds surfaces without bakes at play start, runs bake threads with
  progress, syncs agents, obstacles and links from components, applies `NavCommand`s
  queued by Lua (`ScriptCommands::navigation`) and remote calls, steps the crowd in
  `Scene::step_navigation` after the scripts in play mode (while editing it only runs
  bakes, commands and the debug view: `update_editor`), writes positions and facing back (or steers `CharacterController`s), feeds animation
  trees and hands scripts their callbacks.
- **Debug drawing** (`debug.rs` builds lines and triangles into `renderer::DebugGeometry`;
  `renderer/debug_draw.rs` draws them after the scene, depth tested, plus a faint x-ray pass
  with the depth test reversed). The debug flags and the editor's test path are
  process-wide (`scene_nav::view_state`), because the editor replaces the `Scene` on
  play, stop and undo.
- **Editor**: `src/bin/editor/inspector/navigation.rs` (component sections, bake button and
  progress, debug toggles, the test path tool), `application/navigation_system.rs` and
  `action_handlers/navigation.rs`; the path tool takes Scene-view clicks in
  `application/input.rs` and picks the navmesh with `SceneNavigation::pick`.
- **Lua and remote**: `scripting/lua/api/navigation.rs` (queries run against the scene's
  navigation through `NavigationGuard`, set while scripts run), `remote/handlers/navigation.rs`.

## AI

`src/ai/` (always built; its navigation actions need the `navigation` feature). The user
guide is [`docs/ai.md`](ai.md).

- **Definitions** (`def.rs`): brains are read from a `Def` (a Lua table or JSON, the same
  parser for both) into immutable `TreeDef` (nodes depth-first in a flat list; shared node
  parameters such as `when`, `cooldown` become wrapper decorators) and `MachineDef`, kept by
  name in a `Library` with a revision per name (redefining restarts the agents running it).
  Lua functions in definitions are registry keys with the script that defined them
  (`LuaFn::owner`). Conditions, scores and values are `Expr`s (`expr.rs`) or callables.
- **Runtimes**: `TreeRuntime` (`tree.rs`) keeps per-node state for one agent over a shared
  `TreeDef` (enter on first tick, exit on finish or halt, halting recurses; guards with
  `abort = lower` are checked by their selector before it resumes a later child; utility
  nodes re-score at `interval`). Leaves are in `actions.rs` (walks keep a `Mover`; a walk's
  result is trusted only on a frame after its command, since the navigation steps after
  the AI). `MachineRuntime` (`machine.rs`) checks machine-wide then state transitions,
  runs the state's `update`, tree and nested machine, then `on` transitions.
- **`AiHost`** (`context.rs`) is everything a brain asks of the world. `SceneHost`
  (`host.rs`) implements it over the world, `SceneNavigation` (commands go back in issue
  order: scripts' first, then the AI's, including those Lua leaves make), `ScenePhysics`
  (line of sight), animators (applied directly) and Lua: `ScriptingState::run_in_script`
  runs a function with the full API in the defining script's context (its handle
  registry, state, `get_self()`) or a shared AI scope, and applies its commands after.
  Tests use a toy host (`src/ai/tests.rs`), and headless scenes for Lua brains, the
  whole of Lantern Market (`lantern_market_simulation`, ignored by default) and the AI's
  cost for crowds (`ai_crowd_benchmark`). Senses bucket agents on an 8 m grid
  (`VisibleGrid`), and entity-keyed maps use `EntityMap` (a multiply hash), so a step
  stays linear in the number of agents.
- **`SceneAi`** (`scene_ai.rs`, one per `Scene`, reset on play start/stop) steps in
  `Scene::step_ai` after the scripts and before the navigation: sync agents from
  `AiAgent`s, resolve brains (library or JSON file, files re-read when their stamp
  changes), apply script commands, look (perception at its own rate, `perception.rs`),
  tick due agents (staggered by tick rate), then deliver events and noises for the next
  tick. `on_ai_state` callbacks go to agents' scripts through `queue_callbacks`.
- **Lua reach** (`scripting/lua/api/ai.rs`): `AiShared` (library, per-agent blackboards,
  inboxes, state snapshots, group/global boards, a command queue) sits behind an `Rc`
  that `AiGuard` puts in a thread-local while scene scripts, `on_ui`, the console and the
  AI step run. Nothing holds a `RefCell` borrow of it across a Lua call (brains may define
  brains or send events from inside a leaf). Blackboards are `Arc<Mutex<_>>` because
  their Lua proxies are userdata, which mlua's `send` feature requires to be `Send`.
- **Debug drawing** (`debug.rs`) joins the navigation's `DebugGeometry` in the render
  bridge; the flags are process-wide like the navigation's.
- **Editor and remote**: `src/bin/editor/inspector/ai.rs` and `application/ai_panel_system.rs`
  (the selected agent's `SceneAi::inspect` each frame in play), `remote/handlers/ai.rs`
  (`ai.*` methods).

## Scripting (Lua)

- **One `ScriptingState` per world** (`src/scripting/lua/state.rs`), owned by the
  scene's runtime. Script slots are keyed by `Entity` and validated by the
  `LuaScriptComponent`'s **binding id**. That id is a process-unique counter; a cloned
  component gets a new id.
  - Never identify scripts or worlds by memory address.
  - Never persist a "has started" flag in the component. A restored or reloaded scene
    must run `on_created` again.
- **Hooks**: `on_created(entity)`, `update(entity, dt)`, `on_destroyed(entity)`, `on_ui(...)`.
  - Every hook can read the world through `WorldGuard`.
  - Structural changes (spawn, despawn, `set_parent`, ...) are queued as commands and
    applied after the hook returns. Each command is applied independently, so one stale
    entity doesn't block the rest.
  - `set_parent` rejects cycles.
- **Modes** come from a header annotation:
  - none: play mode only
  - `-- @editor`: editor mode only
  - `-- @tool`: both modes
- **Events**: `emit_event` queues an event, and handlers run after the emitting hook in the
  same frame. At most `MAX_EVENTS_PER_FRAME` (4096) are dispatched per frame, which stops
  feedback loops.
- **Commands and events from `on_ui`** are deferred to the next update.
- **Coroutines**: `wait(seconds)` and `wait_frames(n)` yield tagged values from Lua. Rust
  resumes them without holding a borrow across the resume.
- **Errors**: a compile failure is cached. Each distinct error is logged once
  (`last_error()`) and cleared when the hook succeeds again. Editor plugin windows show
  the error inline.
- **Script UI**: `on_ui` records `UiCommand`s and egui renders them later. Widget ids come
  from `claim_id(kind, key)`. Value widgets report changes only, so scripts can reset
  values. `ui:anchored_panel` places HUD panels relative to the game viewport, and
  `ui:get_viewport_size()` and `ui:get_pixels_per_point()` expose the viewport.
- **Editor UI plugins** (`examples/scripts/ui_plugins.toml`, for example the welcome
  screen) run in the editor's own `UiPluginManager` with a separate `ScriptingState` and
  world. Their state survives play/stop and scene switches. They can't see the scene
  world.
- **Hot reload** (`src/bin/editor/application/script_watcher.rs`):
  - Watches `examples/scripts`, `scripts` and the open project's content root,
    recursively, with a 500 ms debounce.
  - Reloads only the changed script (`reload_path`) and keeps its state.
- **File sandbox** (`api/file_io.rs`):
  - Lua file access is limited to `ALLOWED_DIRS` (`scripts`, `examples/scripts`) plus
    paths granted with `grant_path_access`. Grants come from file dialogs, the text
    editor bridge and the open project directory.
  - Paths are resolved through `..` and symlinks before checking.
  - Don't add another way to reach the filesystem from Lua without going through this check.

## egui integration and the editor

- **The integration** (`src/ui/egui_integration.rs`) runs each frame through
  `egui::Context::run_ui`. The UI callback, and therefore `RenderApplication::ui`,
  receives egui's root `&mut egui::Ui`:
  - Panels (`egui::Panel::top/left`, `CentralPanel`) must be shown inside that `Ui`.
  - Windows use `ui.ctx()`.
  - Using `run_ui` (rather than `begin_pass`/`end_pass`) keeps egui's built-in plugins
    working: label text selection, and the drag and drop that egui_tiles relies on.
- **`max_passes` is 1 on purpose.** The editor's UI code has side effects (it queues
  actions and renders script UI), and a second, discarded pass would repeat them.
- **Texture deltas** (font atlas, images) must always reach the GPU. In debug builds,
  egui panics if a `TexturesDelta` is dropped without being applied. Unrendered output is
  merged into the next frame instead of being dropped.
- **Editor structure** (`src/bin/editor/application/`):
  - `EditorApplication` holds an `EditorSharedState` and a list of `EditorSystem`s
    (camera, selection, history, project, script editor, shader editor, asset browser,
    particles, light probes, animation panel). Each system implements `update`,
    `gpu_update` and `ui`. Inspector sections that show live engine state read a status
    a system writes each frame (`LightProbePanelStatus`; `AnimationPanelStatus`: the
    selected character's clips, bound once per animator and library list, and in play
    mode its tree's state and parameters).
  - UI code doesn't mutate the scene directly. It queues `EditorCommand`s (inspector
    edits arrive as `InspectorAction`s). The next frame's update stage drains the queue
    (`drain_update_commands`) and applies them through `action_handlers/`, recording undo
    history. Script, shader, scene-creation and undo/redo commands stay on the queue for
    the `EditorSystem` that owns them.
  - The dock layout is an `egui_tiles::Tree<EditorPane>`.
  - Play mode snapshots the scene (see above); the Game pane being active implies
    Playing.

## GPU particles

`src/gpu_particles/` runs simulation in compute shaders. Behaviours: physics, starfield,
boids, and "optimized boids", which uses a spatial grid and a GPU radix sort.

- **Alpha sorting** reads particle data back asynchronously (`system/sorting.rs`) and
  sorts on the CPU.
- **Dead-list compaction** also uses a readback.
- **Wiring**: `ParticleSystemComponent` and `ParticleEmitterComponent` configure systems
  from the ECS; the editor drives them through its particle system.

## Remote control and MCP

A running app can be driven over a local socket, which is how AI agents (through the
MCP server in `tools/engine-mcp/`) inspect and edit scenes, run Lua, inject input and
take screenshots. The `engine-mcp` skill (`.claude/skills/engine-mcp/`) describes the
agent workflow.

- **Enabling**: the editor listens on `127.0.0.1:47811`
  (`remote::DEFAULT_EDITOR_PORT`). Other apps opt in through
  `RenderApplication::remote_control_port` or the `ENGINE_REMOTE_PORT` environment
  variable, which also overrides the port; `ENGINE_REMOTE_PORT=off` disables it.
- **Protocol**: newline-delimited JSON, `{"id", "method", "params"}` in, one
  `{"id", "result"}` or `{"id", "error": {"message"}}` out per request. `engine.methods`
  lists what the running app supports.
- **Threading**: `remote/server.rs` accepts connections on background threads and
  forwards requests over a channel. Nothing touches the scene off the main thread.
- **Frame history and noise freeze**: the hub keeps the last 240 frames' renderer
  statistics and GPU timings (`stats {history}`); `render.freeze_noise` /
  `Renderer::set_frozen_noise` holds the TAA jitter, the post-process clock (grain,
  dither) and live light probe updates still so captures compare exactly;
  `visual.compare` runs the capture comparison over saved images; `screenshot` and
  `visual.capture` can hide the FPS readout and editor grid (`overlays: false`, read by the
  UI through `remote::overlays_hidden`).
- **Execution**: `remote::process_requests` runs as a GPU system registered after the
  app's own (see `build_winit_app_internal`). Each request goes first to
  `RenderApplication::remote_command` (the editor handles projects, scene tabs,
  undo/redo, selection and imports in `src/bin/editor/application/remote.rs`), then to
  the built-in handlers in `remote/handlers/`. After requests change the scene,
  `remote_scene_modified` runs once; the editor records an undo step there.
- **Frame-loop hooks** (`app/platform_winit/mod.rs`): injected input is applied at the
  start of a frame, before the update stage, so scripts see `just_pressed` exactly as
  for real keys. Screenshots copy the swapchain image after egui is drawn and before
  `present` (the surface is configured with `COPY_SRC` when supported). `end_frame`
  advances the hub's frame counter and answers `wait` requests.
- **Post-processing**: `postprocess.set` edits a volume entity (the lowest-priority
  infinite one by default, created if missing), so changes save with the scene.
- **Procedural assets** (`remote/handlers/procedural.rs`): `texture.generate` writes
  tileable PNG map sets, `texture.maps` derives maps from an existing height image,
  and `audio.generate` writes seeded PCM WAV effects. Files and parameter recipes
  save immediately under the project's content folder; output paths and compatible
  material bindings are returned. Existing textures reload, and replaced sound files
  are evicted from the playback cache. CPU algorithms live in `src/procgen/` and can
  be tested without a GPU or audio device. Generation tools are native authoring
  features; their output assets also load on the web.
- **Procedural geometry** (`remote/handlers/geometry.rs`): `mesh.lathe` turns and bevels
  radius/height profiles, `mesh.extrude` triangulates and bevels concave polygons with
  taper/twist, and `mesh.compose` joins transformed parts. Outputs have UVs, normals,
  tangents, content-based GPU sharing and bounds. Scene saves embed their `MeshData`.
- **Logs**: every log record also goes to `remote::log_tap`, a ring buffer with
  sequence numbers, so clients can poll for new lines. Panics are recorded there too.
- **Lua console**: `ScriptingState::eval` runs code with the full scripting API, either
  in a persistent console environment or inside one entity's script environment.
- **Explicit simulation**: `RuntimeStateHandle` stores manual mode, seed and step count.
  Entering manual play initializes scripts once; ordinary update frames skip Lua/animation.
  `runtime.step` applies step-indexed input, updates scenes and realizes script assets for
  each fixed step. GPU particles, UI and custom app systems are outside this scope.
- **Scene transactions**: `scene.transaction` validates whitelisted scene calls in a
  speculative snapshot, resolves aliases to previous results and commits once. Errors and
  dry runs restore the original scene, including its scripting runtime; file/audio/Lua
  operations are excluded. The editor records one undo entry on commit.
- **Spatial tools** use transformed world AABBs, visibility inheritance and hierarchy
  exclusions. Ray normals describe box faces; placement/scatter are approximate authoring.
- **Profiling**: `profile.capture` samples consecutive frames through the remote GPU system.
  Percentiles use nearest rank; CPU stage/submit cost and cumulative Lua hook counters are
  available, along with entity/asset growth. GPU execution timestamps are not measured.
- **Prefab authoring** exports a selected subtree into a registered scene document. Source
  updates affect future/reopened instances; current instance values are preserved.
  Project exports include linked child scene nodes as prefab references with placement and
  visibility overrides; snapshots preserve origin metadata in serialized entity order.
- **Verification**: read-only `project.validate` checks saved structure/assets/Lua syntax.
  MCP `web_build` wraps the non-serving build script; `web_playtest` uses Node's WebSocket
  and Chromium CDP with a disposable loopback server/browser profile. Results contain
  structured JSON, output schemas and tool annotations.

## Invariants and gotchas

- **GPU structs** uploaded from Rust need `#[repr(C)]`, 16-byte alignment and explicit
  padding that matches WGSL. WGSL struct members are separated by `,`.
- **Bindless shaders** must start with `enable wgpu_binding_array;` (naga 30).
  `ShaderBuilder::finalize` adds it automatically whenever the source uses
  `binding_array<`. Hand-written WGSL that bypasses the builder must add it itself.
- **wgpu 30 API changes**:
  - Pipeline layouts take `Option<&BindGroupLayout>` and `immediate_size` (push constants
    are now "immediates").
  - Vertex buffer layouts are `Option`s.
  - `get_mapped_range` returns a `Result`.
  - Frames are presented with `queue.present` (`Renderer::present`).
- **Camera**: `SceneCamera::apply_from_entity` keeps the current camera when the entity
  hasn't moved, while syncing its up vector when roll changes. Equivalent saved up
  hints (such as world Y) are preserved across undo.
  Rebuilding an unchanged view resets the orbit focus distance. Lua `look_at` and
  MCP camera placement share a Y-up, zero-roll look rotation so a full camera orbit
  keeps the horizon level.
  Lua `look_at` also supplies a transient `LookAtTarget` hint. While it still agrees
  with the entity's world forward direction, the camera keeps that actual focus point
  instead of replacing it with `eye + forward`. Directional shadows then stay centered
  on an orbit's subject, including elevated subjects and shallow viewing angles. A
  stale hint is ignored after the camera moves or turns away; scripts regenerate hints
  at runtime rather than serializing them into scene assets.
- **Script lists**: the inspector's script picker and the file sandbox are relative to the
  working directory. Run from the repo root.

## Verifying changes

- **Required before committing**: `cargo fmt --all`,
  `cargo clippy --all-targets --all-features -- -D warnings` and
  `cargo test --all-features`. CI (`.github/workflows/ci.yml`) runs the same.
- **Ignored tests**: `cargo test --all-features -- --ignored` runs GPU-heavy and slow
  tests. `packaged_gltf_roundtrip_without_source` is a known failure (see below).
- **Run it on the GPU**: tests don't create real pipelines, so naga or wgpu validation
  failures and egui debug assertions only appear at runtime. Use the `smoke-test` skill
  (`.claude/skills/smoke-test/smoke.sh`). It builds the editor and examples, runs each
  one briefly, scans the logs and can screenshot the engine window.
- **Lua regression tests** live in `src/scripting/lua/regression_tests.rs` and
  `integration_tests.rs`.

## Known issues and roadmap

- **Web filesystem limits**: Lua uses the vendored wasm libc/SjLj shim. File watching and
  authoring remain native-only; player assets load through HTTP, and audio needs a gesture.
- **glTF data-URI images**: images embedded as `data:` URIs are treated as file paths, so
  `packaged_gltf_roundtrip_without_source` fails.
- **Inspector script picker** lists only `scripts/` and `examples/scripts/`, not the open
  project's `content/scripts/`.
- **`player` error**: without a project directory it prints a bare
  `Io(NotFound)` instead of saying no `project.json` was found.
- **Editor UI plugins can't access the scene world**, so tools such as scene statistics
  or quick actions can't be written as plugins yet.
- **Undo history** stores a full scene snapshot per change, which may get expensive for
  large scenes.
- **Project environment is not restored on load**: `ProjectManifest` saves the scene
  environment in `project.json`, but neither the editor nor the player applies it when
  loading. Only an Environment entity in the scene round-trips; the remote
  `environment.set` creates one for that reason.
- **Recent projects** are stored in the repo (`examples/scripts/.recent_projects.txt`)
  rather than in a per-user config directory.
- **Dependencies not yet upgraded**: mlua 0.12, glam 0.34, hecs 0.11, thiserror 2,
  toml 1.x, rand 0.10, base64 0.23, pollster 1.0, rfd 0.17.
- **In-game UI roadmap**: anchored panels and viewport queries are done. A WYSIWYG design
  mode (dragging anchored panels in the editor and saving offsets to the entity) and
  flex/grid layout containers are not implemented. The original plan is in git history
  (`INGAME_UI_PLAN.md`).
- **Not yet reviewed in depth**: the asset browser, the project system and the renderer
  internals (shadows, post-processing, batching).
