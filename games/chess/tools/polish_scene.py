#!/usr/bin/env python3
"""Polish the existing chess scene through the actual engine MCP (no piece regeneration)."""
import argparse
import base64
import importlib.util
import json
import sys
from pathlib import Path

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("chess_assets", REPO / "tools/engine-mcp/examples/procedural_chess.py")
assets = importlib.util.module_from_spec(spec)
spec.loader.exec_module(assets)


def reuse_repository_hdri():
    """Keep the shared repository HDR instead of the copy made by project_save."""
    project = REPO / "examples/projects/procedural-chess"
    manifest_path = project / "project.json"
    manifest = json.loads(manifest_path.read_text())
    manifest["environment"]["hdr"]["image_path"] = "../../../web/assets/hdr/pine_attic_4k.hdr"
    manifest_path.write_text(json.dumps(manifest, indent=2) + "\n")

    # Like the engine's other example assets, scene HDR paths use the repository
    # working directory. The spectator and launchers set that directory explicitly.
    scene_path = project / "content/scenes/Procedural_Chess.scene.json"
    scene = json.loads(scene_path.read_text())
    for entity in scene["entities"]:
        environment = entity.get("environment")
        if environment and environment.get("hdr"):
            environment["hdr"]["path"] = "web/assets/hdr/pine_attic_4k.hdr"
    scene_path.write_text(json.dumps(scene, separators=(",", ":")) + "\n")

    copied = project / "content/environment/pine_attic_4k.hdr"
    shared = REPO / "web/assets/hdr/pine_attic_4k.hdr"
    if copied.exists():
        if copied.read_bytes() != shared.read_bytes():
            raise RuntimeError("Project HDR differs from the shared HDR; refusing to remove it")
        copied.unlink()
        if not any(copied.parent.iterdir()):
            copied.parent.rmdir()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=47821)
    args = parser.parse_args()
    mcp = assets.MCP(args.port)
    try:
        mcp.call("project_open", path=str(REPO / "examples/projects/procedural-chess"))
        mcp.call("scene_activate", document_id="Procedural_Chess.scene")
        mcp.call("wait", frames=3)
        mcp.call("environment_set", ambient_intensity=0.22, hdr={"enabled": True, "intensity": 0.7, "path": str(REPO / "web/assets/hdr/pine_attic_4k.hdr")})
        mcp.call("postprocess_set", tonemapping="agx", exposure={"mode": "manual", "compensation": 0.35},
                 bloom={"enabled": True, "intensity": 0.12, "threshold": 1.3, "scatter": 0.62},
                 ssao={"enabled": True, "radius": 0.3, "bias": 0.02, "intensity": 0.8, "power": 1.2},
                 ssr={"enabled": True, "intensity": 0.38, "max_distance": 18, "thickness": 0.18, "blur": 0.35},
                 vignette={"enabled": True, "intensity": 0.2, "smoothness": 0.5, "roundness": 0.8},
                 depth_of_field={"enabled": True, "mode": "manual", "near_start": 0, "near_end": 3, "far_start": 32, "far_end": 48, "max_blur": 40},
                 color={"temperature": 6, "saturation": 0.98, "contrast": 1.04},
                 sharpen={"enabled": True, "strength": 0.18}, anti_aliasing="taa")
        mcp.call("entity_update", entity="Warm key", directional_light={"color": "#ffe4bf", "intensity": 1.8, "shadow_size": 20})
        mcp.call("entity_update", entity="Cool fill", directional_light={"color": "#c6d8ef", "intensity": 0.85}, cast_shadow=False)
        mcp.call("entity_update", entity="Studio floor", visible=False)
        if "script" in mcp.call("entity_get", entity="Scene labels")["components"]:
            mcp.call("entity_update", entity="Scene labels", script=None)
        mcp.call("camera_set", entity="GameCamera", position=[8.4, 15.6, 17.9], target=[0, 0.35, 0], fov_deg=42)
        for side, colors, seed, roughness in [("White", ["#c8b185", "#efe0bd"], 91, 0.2), ("Black", ["#14191c", "#272e30"], 92, 0.22)]:
            result = mcp.call("texture_generate", name=f"exhibition_{side.lower()}", pattern="noise", colors=colors, seed=seed,
                              size=[512, 512], scale=4, normal_strength=0.0012, roughness=roughness,
                              roughness_variation=0.035, metallic=0.04, overwrite=True, image=False)
            mcp.call("entity_update", name_contains=f"Chess.{side}.", material=result["material"])
        # Cooler, darker light squares separate the warm ivory silhouettes from the board.
        maple = mcp.call("texture_generate", name="exhibition_maple", pattern="wood", colors=["#73796e", "#a3a88f"],
                         seed=5, size=[512, 512], scale=3, normal_strength=0.0007, roughness=0.48,
                         roughness_variation=0.025, overwrite=True, image=False)
        squares = mcp.call("scene_list_entities", limit=250)["entities"]
        light_ids = []
        for square in squares:
            name = square["name"]
            if name.startswith(("Square ", "Maple ")):
                coordinate = name.rsplit(" ", 1)[-1]
                if len(coordinate) == 2 and coordinate[0] in "abcdefgh" and coordinate[1] in "12345678":
                    if ((ord(coordinate[0])-ord('a')) + int(coordinate[1])-1) % 2 == 1:
                        light_ids.append(square["id"])
        mcp.call("entity_update", entities=light_ids, material=maple["material"])
        # Keep all original pieces and squares; only add furniture and move inlays.
        entities = mcp.call("scene_list_entities", limit=250)["entities"]
        names = {entity["name"] for entity in entities}
        for name, width, height, y, depth, color, metallic, roughness in [
            ("Exhibition plinth", 15.7, 13.4, -0.79, 0.9, "#04090b", 0.08, 0.38),
            ("Plinth brass reveal", 15.78, 13.48, -1.22, 0.07, "#a38043", 0.8, 0.27),
            ("Plinth lower shadow", 15.3, 13.0, -1.39, 0.3, "#11181b", 0.18, 0.44),
        ]:
            fields = {"name": name, "polygon": [[-width/2, -height/2], [width/2, -height/2], [width/2, height/2], [-width/2, height/2]],
                      "depth": depth, "bevel": min(depth*0.25, 0.15), "bevel_segments": 5, "rotation": [-90, 0, 0],
                      "translation": [0, y, 0], "material": {"color": color, "metallic": metallic, "roughness": roughness}}
            if name in names:
                fields["entity"] = name
            mcp.call("mesh_extrude", **fields)
        for index in range(8):
            name = f"Move.Inlay.{index}"
            if name not in names:
                mcp.call("entity_spawn", kind="cube", name=name, translation=[0, 0.236, 0],
                         scale=[1.1, 0.012, 0.022] if index % 4 < 2 else [0.022, 0.012, 1.1],
                         material={"color": "#d4aa5c", "metallic": 0.65, "roughness": 0.24, "emissive": 0.45},
                         cast_shadow=False, visible=False)
        for name, preset, duration, frequency, end_frequency, noise, cutoff, volume in [
            ("chess_move", "hit", 0.11, 520, 170, 0.14, 2400, 0.65),
            ("chess_capture", "hit", 0.18, 380, 85, 0.25, 1800, 0.72),
            ("chess_check", "tone", 0.4, 660, 880, 0, 2600, 0.32),
            ("chess_end", "tone", 1.1, 440, 220, 0, 1800, 0.38),
        ]:
            result = mcp.call("sound_generate", name=name, preset=preset, waveform="sine", duration=duration,
                              frequency=frequency, end_frequency=end_frequency, noise_mix=noise, lowpass_hz=cutoff,
                              attack=0.002 if preset == "hit" else 0.025, release=duration*0.7, decay=5 if preset == "hit" else 1.5,
                              volume=volume, seed=73, overwrite=True, image=False)
            print(f"Generated {name}: {str(result)[:180]}", flush=True)
        mcp.call("project_save")
        reuse_repository_hdri()
        mcp.call("wait", frames=12)
        shot = mcp.call("screenshot", target="viewport", camera="game", aspect="16:9")
        destination = REPO / "target/engine-mcp/chess-preview/exhibition.png"
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_bytes(base64.b64decode(shot["_image"]))
        print(f"Saved scene and preview: {destination}", flush=True)
    finally:
        mcp.close()


if __name__ == "__main__":
    main()
