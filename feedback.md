# Engine MCP feedback — The Hollow Crown

Notes from building a fantasy dungeon crawler through the engine MCP, 2026-10-14.

## Launch and discovery

- **Desktop environment is missing from MCP launches.** `engine_launch {target: "editor"}` compiled successfully, then exited because neither `DISPLAY` nor `WAYLAND_DISPLAY` was set. The terminal environment has `DISPLAY=:0` and a working desktop socket. Workaround: launch the built editor from the terminal with the desktop environment and let MCP attach. Suggest exposing launch environment overrides or inheriting the desktop session environment, and checking it before spending time on a build.
- **Tool descriptions repeat a long workflow.** Discovering a group of tools produces large amounts of duplicate text, obscuring their argument schemas and actual results. Suggest a short shared workflow resource and concise per-tool descriptions.

## Scene authoring

- **Only four point lights and four spot lights are rendered.** The MCP light schemas do not advertise this limit or explain which lights win. A multi-room torchlit dungeon needs light selection. Workaround: use four movable light entities for the active chamber. Suggest nearest/influence-based light selection and explicit capacity reporting in engine_info.
- **Mesh composition has an undocumented 128-part limit.** A chamber wall made of masonry blocks failed with `compose needs 1..=128 parts`. The schema advertises only the vertex budget. Workaround: split masonry into smaller composed meshes. Include this limit in the schema and preflight reports.
- **Failed atomic batches return enormous successful-call payloads.** The rollback worked, but the error included every part of previously generated geometry and repeated full recipes. Suggest a quiet batch option that returns identifiers and a concise failure summary, with full geometry available on request.

## Gameplay and visual verification

- **HUD font coverage is unclear.** The documented text API accepted heavy-line health bars and diamond map markers, but screenshots showed empty boxes. Workaround: use supported ASCII characters. Suggest documenting bundled font coverage or supporting a project font through the HUD API.
- **The editor capture limits screenshot resolution.** Requesting a 1600-pixel screenshot still produced a 1045-pixel-wide image because it uses the actual docked viewport. This makes fine visual review awkward. Suggest explicit offscreen render sizing rather than only shrinking to fit the current viewport.
- **Project saves are very verbose.** The response lists hundreds of material paths, twice across text and structured results. A written-file count and scene/project paths would be more useful by default.
- **Repeated saves leave orphaned material files.** Three saves left 676 material JSON files on disk while the current scene references 229 materials. New random filenames are written each time, and the earlier files remain. Suggest stable material identities or an explicit safe orphan-cleanup report.

- **Scenario saving requires manually creating parent directories.** `playtest {save_scenario: "games/hollow-crown/tests/endurance.json"}` failed with an OS error because `tests/` did not exist. Suggest creating parent directories or failing in an explicit preflight before any simulation work.
- **Embedded mesh JSON becomes large quickly.** Five small chambers with approximately 153,000 vertices produced a 63 MiB scene document. It is portable and loads, but pretty-printed per-vertex JSON is bulky for version control and distribution. Suggest shared binary mesh assets or a compact serialization option.
- **An attached process replacement needs an extra call to reconnect.** After switching our saved editor project to the player on the same port, the first `engine_launch` reported a closed connection to the old editor even though the player was listening. The next Lua call connected successfully. Suggest probing and reconnecting before returning the stale-process error.

## Engine geometry encountered through MCP

- **Primitive cylinder caps face inward.** The Moonwell's flat cylinder appeared as an empty stone plate from above. Inspecting `src/renderer/primitives.rs:cylinder_mesh` showed that cap triangle winding points opposite to the declared end normals. A flattened sphere immediately made the teal water visible. Workaround: change the water surface in the game's Lua initialization; the engine sources remain unchanged. Suggest correcting cap winding and testing both ends from outside.

## Lua hot reload

- **Returning spawned handles in preserved state breaks after reload.** Following the scripting guide's `on_destroyed(..., "reload")` / `on_created(..., previous)` pattern retained the game table, but the projectile pool and trap marker still had temporary IDs -1 through -13. Their entities existed (the count remained 295), yet commands after resuming emitted `failed to mutate ECS world: no such entity` every frame. Workaround: resolve spawned entities by their unique names on the first update and rebind the pool on reload. Suggest remapping preserved handles across VMs, or documenting that spawn handles are temporary and providing a stable-handle resolver. An entity-count-only reload test is insufficient; check mutations and new warnings after resuming too.

## What worked well

- Fixed-step assertions verified complete expeditions, floor boundaries, manual input, pause, defeat recovery, and automatic restart. A 340-second seeded run completed three victories without script failures or warnings.
- Atomic batch rollback preserved the scene after geometry validation failures.
- Project validation found no portability or script syntax issues, and the saved scene loaded correctly in the native player.
- After rebinding temporary handles by name, script hot reload preserved state, kept all effect handles valid, and produced no new warnings while resumed (295 entities before and after).
- The player profile showed a 13.19 ms median / 14.81 ms p95 frame time over 300 frames, with no entity or asset growth. Editor UI overhead made the same content appear much slower during authoring.

## Expansion: sixteen chambers, branching routes, equipment, and danger

- **Concave navigation must be authored separately.** The new oval, irregular, L-shaped, and cruciform rooms cannot use the spatial tools' world AABBs as walkable areas. We shared the actual footprint polygons, prop obstacles, and corridor polylines between scene generation and a cached Lua navigation grid. Suggest a walkable-polygon/navmesh authoring interface or exact mesh spatial queries; bounding-box rays remain useful for rough placement.
- **The console and script module environments differ.** `lua_eval` without an attached script returned a nil global for `require`, although ordinary scripts can require modules. Workaround: attach the director and evaluate inside that entity's script. Suggest clearly documenting the console's module-loading limitation alongside the full-API description.
- **Frozen manual sessions make reload review ambiguous.** After editing a required HUD module and calling script_reload plus frame waits, a frozen manual screenshot still showed the earlier HUD. Starting a fresh play session showed the update. A pending-reload/version indicator, or an explicit note about required simulation steps, would make this easier to diagnose.
- **Large scene serialization remains costly.** The expanded scene has 827 saved entities and 434 embedded meshes. MCP project_save wrote 201,951,349 bytes of pretty-printed JSON. Removing whitespace without changing the JSON data reduced it to 67,229,933 bytes; that compact file validated and loaded in the player. Suggest a compact-save option as an immediate improvement, before introducing binary mesh assets. Editor saves re-expand the file.
- **Save cleanup is needed before committing an example.** The final expanded project contained 1,974 material files, but its saved scene referenced only 725. The 1,249 unused files were moved out of the example before committing. A built-in dependency scan and safe orphan cleanup would avoid this manual packaging step.
- **The text-only HUD lacks native health bars, a graph map, and overlay layering controls.** Character bars use ASCII and the atlas is a room list. Text labels rendered over an equipment panel until we explicitly hid the ordinary HUD while that panel was open. Suggest progress bars, lightweight drawing primitives, and explicit overlay layering for in-game UI.

The expansion's ten-seed sweep produced three victories and seven defeats in 10.7–13.5 minutes of game time; a separate rendered victory and defeat matched their presentation-free simulations exactly. Equipment benefits, rejecting downgrades, class compatibility, permanent wounds, manual arrival, and defeat recovery all passed. After adding four torches to the cruciform room, the final native player retained 859 entities through a 300-frame profile, with zero asset growth and a 16.24 ms median / 18.13 ms p95 frame time. Selecting lights from visible room/corridor torches prevents hidden branches from supplying the active light pool.
