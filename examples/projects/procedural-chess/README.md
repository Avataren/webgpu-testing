# Procedural chess

A saved chess scene built through the engine MCP, with procedural models and
material maps. Open this folder in the editor's project picker, or run
`./runchess.sh` from the repository root for the
[AI spectator game](../../../games/chess/README.md).

- **Procedural Chess**: all 32 pieces in the starting position on an 8×8 board, with
  beveled wood squares, a walnut frame and brass rim. White's h1 square is light.
- **Chess Piece Study**: pawn, rook, knight, bishop, queen and king in ivory and ebony,
  arranged on brass pedestals with labels so silhouettes and materials are easy to inspect.

Turned bases, stems and finials use `mesh_lathe`, with rounded bevels on bases,
collars and crowns. Knights and notched bishop heads,
king crosses, rook crenellations, board squares and plinths use `mesh_extrude` with
beveled rims. `mesh_compose` joins each complete piece into a single mesh; repeated
pieces share one mesh per type. Textures use seeded `texture_generate` recipes for
ivory/ebony marble, maple, walnut and brushed brass, including base color, normal,
metallic/roughness and occlusion maps. The JSON recipes are beside the PNGs.

The scenes contain their mesh data and portable texture paths. They reopen and run
without regenerating anything. The board scene includes the spectator game's
display plinth, warmer ivory pieces, cooler light squares, generated sounds, and
soft-focus attic HDRI. Self-play uses the dedicated native game executable; the
generic editor and player display the saved scene.

To regenerate through the actual stdio MCP server:

```sh
cargo build --all-features --bin editor --bin player
cargo build --release --manifest-path tools/engine-mcp/Cargo.toml
python3 tools/engine-mcp/examples/procedural_chess.py
```

The script launches its own editor on port 47819 and stops it after saving. It refuses
to use a port with a running engine. Use `--port` to select another port, `--project`
for another destination, and `--keep-open` to inspect the result in its editor.
Regeneration adds new scene tabs with unique names and replaces this example's
generated textures with the same seeded output.

Verification checks 32 board pieces, 12 study pieces, six shared meshes in each scene,
all four material maps, save/reload, and a short playtest without script errors or
warnings. Screenshots and a report go to `target/engine-mcp/chess-preview`.

Use `--update-bevels` to replace the geometry in the existing two scenes while
preserving their materials, transforms and layout, instead of adding scene tabs.
