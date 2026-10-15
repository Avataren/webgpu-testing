# RT Lab

A test room for the ray-traced rendering work (`ssr.ray_traced`). It is not a game:
it exists to show, in one frame, what each reflection path gets right and wrong. In play
mode the game camera runs a looping tour (`content/scripts/camera_tour.lua`): slow sweeps
past each effect, gliding from one into the next, with captions. Space pauses, the arrow
keys skip between shots, H hides the captions. Stop play (or remove the script) to use the
fixed camera for comparisons.

| Object | What it tests |
| --- | --- |
| Mirror panel (back wall, metallic, roughness 0) | Off-screen reflections: it reflects the room behind the camera, which screen-space reflections cannot see. The scene traces at full resolution (`ssr.full_resolution`) so it stays sharp |
| Brick wall and neon strips behind the camera | Material textures and emissive surfaces at ray hits (only visible through the mirror and the mirror sphere) |
| Mirror sphere, chrome torus knot, copper Suzanne (the knot and Suzanne on the outer pedestals) | Metal reflections in the surface's own colour (F0); the knot spins in play mode |
| Gold spheres, roughness 0 / 0.15 / 0.3 / 0.5 / 0.8 | Roughness-dependent reflection blur |
| Chrome knot, neon strips seen in the mirror | Shimmer: thin bright things in curved mirrors flicker under TAA jitter unless reflections are accumulated (compare frames with `visual_capture` `frames: 8`; read `compare.noise`) |
| Checker cube | Texture level of detail at hits (ray cones) |
| Red and green side walls | Colour bleeding into floor and mirror reflections, and screen-edge fade with SSR |
| Polished marble floor (textured roughness) | Per-pixel smoothness from a metallic-roughness map |
| CesiumMan (skinned, walks in play mode) | Skinned meshes in the TLAS: his reflection follows the animation |
| Sun and two lamps with ray-traced shadows (`rt_shadows`) | Per-pixel shadow rays: the side walls shadow the room (shadow maps let the sun through them), the lamps cast soft shadows (5 cm radius), including CesiumMan's |
| Glass sphere (transmission 1, ior 1.5, dispersion 0.5) on the left pedestal | Ray-traced refraction: an inverted, magnified view of the room, rainbow fringes at the rim; no solid shadow |
| Round brilliant gem (Blender, 81 facets, ior 2.42, pale blue, dispersion 1.0, about three times diamond's) on the right pedestal, turning in play mode | Total internal reflection across many facets, absorption tint, fire (dispersion), sparkle from the emissive light bar overhead |
| Ray-traced ambient occlusion (`ssao.ray_traced`, 0.5 m) | Contact darkening at the pedestal bases, wall corners and inside the knot, from geometry on or off screen |
| Spark fountain (GPU particles, in front of the mirror sphere) | Upscaling (`upscaling.render_scale` below 1): fast, small, bright particles over the scene are what FSR's reactive mask keeps from smearing. In the player they render before post-processing and go through the upscaler; the editor draws them afterwards at output resolution |

The torus knot, Suzanne and the gem were modelled in Blender (through the Blender MCP server) and
imported as GLB; the textures come from `texture_generate`.

## Run

Open this directory in the editor (`project_open games/rt-lab`) or run
`cargo run --release --all-features --bin player -- games/rt-lab`.

Compare the paths with the engine MCP, from the game camera:

```
visual_capture {camera: "game", variants: [
  {name: "ssr", postprocess: {ssr: {ray_traced: false}}},
  {name: "off", postprocess: {ssr: {enabled: false}}}]}
```

Upscaling, as the scene renders at two thirds and half the resolution with FSR and TAAU:

```
visual_capture {camera: "game", variants: [
  {name: "fsr067", postprocess: {upscaling: {render_scale: 0.67}}},
  {name: "fsr050", postprocess: {upscaling: {render_scale: 0.5}}},
  {name: "taau050", postprocess: {upscaling: {render_scale: 0.5, method: "taau"}}}]}
```

`engine_info` reports whether hardware ray tracing is available
(`surface.ray_tracing`); without it `ray_traced` falls back to screen space.
`WGPU_CUBE_DISABLE_RAY_TRACING=1` forces that fallback.
