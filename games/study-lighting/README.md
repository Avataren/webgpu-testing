# Study Lighting

The chess study room (copied from [`games/chess/study`](../chess/study), without the
chess game) as a test scene for the engine's lighting modes. Raster lighting, ray-traced
global illumination and path tracing should look alike; path tracing is the reference.

Open it in the editor and press Play. `content/scripts/lighting_shots.lua` holds the
camera on one of seven fixed shots: `1`-`7` or the arrow keys pick a shot, `H` hides
the overlay, which names the shot and the lighting mode in use.

| # | Shot | Tests |
|---|------|-------|
| 1 | board | the scene's own camera: board, clock, decanter, fireplace |
| 2 | overhead | window-bar shadows across the board and table |
| 3 | sunlit windows | bright sky and curtains beside dark wainscot |
| 4 | fireplace | fire glow, marble, the painting |
| 5 | far wall | the wall facing the windows, lit mostly by bounced light |
| 6 | decanter | glass, refraction and caustics |
| 7 | the room | everything at once |

The lighting mode is a player setting (settings page, or `graphics_set
{global_illumination, path_tracing}` over MCP). To compare modes without changing it,
use `visual_capture` with variants `{global_illumination: {enabled: true}}` and
`{path_tracing: {enabled: true}}` on `camera: "game"`, after `SHOTS.go(n)` through
`lua_eval {entity: "Lighting Shots"}`. Depth of field is off so it cannot blur the
differences; exposure is the study's (auto plus local exposure).

The "Light Probes" volume fills the room with a 0.5 m grid of light probes, which give
raster lighting its bounced light (live with ray tracing; from
`content/lighting/light_probes.probes` on the web). The grid's outer layer sits in the
walls, floor and ceiling; relocation moves those probes into the room where it can. Bake
again (the volume's Bake button in the inspector, or `light_probes_bake`) after changing the
room or its lights; Show probes in the same section draws them.

The room's assets are copies of the study's, so this project can change independently.
