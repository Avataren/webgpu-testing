#!/usr/bin/env python3
"""Build and verify the chess test project entirely through the stdio engine MCP.

Uses only the Python standard library. Build the editor and MCP first, then run:
    python3 tools/engine-mcp/examples/procedural_chess.py
Each run adds fresh scenes; generated texture recipes use deterministic seeds.
"""

import argparse
import base64
import json
import math
from pathlib import Path
import subprocess


REPO = Path(__file__).resolve().parents[3]
BASE = [[.36, 0], [.43, .035], [.47, .10], [.47, .16], [.43, .22],
        [.39, .24], [.39, .29], [.36, .34], [.33, .37]]
ORDER = ["rook", "knight", "bishop", "queen", "king", "bishop", "knight", "rook"]
KINDS = ["pawn", "rook", "knight", "bishop", "queen", "king"]


class MCP:
    def __init__(self, port):
        self.process = subprocess.Popen(
            [str(REPO / "tools/engine-mcp/target/release/engine-mcp"),
             "--repo", str(REPO), "--port", str(port)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
        )
        self.sequence = 0
        self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                                "clientInfo": {"name": "procedural-chess", "version": "1"}})

    def rpc(self, method, params):
        self.sequence += 1
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.sequence,
                                            "method": method, "params": params}) + "\n")
        self.process.stdin.flush()
        line = self.process.stdout.readline()
        if not line:
            raise RuntimeError("MCP server exited unexpectedly")
        response = json.loads(line)
        if "error" in response:
            raise RuntimeError(response["error"])
        return response["result"]

    def call(self, tool, **arguments):
        result = self.rpc("tools/call", {"name": tool, "arguments": arguments})
        if result.get("isError"):
            raise RuntimeError(f"{tool}: {result['content']}")
        text = next((item["text"] for item in result["content"] if item["type"] == "text"), "{}")
        try:
            report = json.loads(text)
        except json.JSONDecodeError:
            report = {"text": text}
        image = next((item["data"] for item in result["content"] if item["type"] == "image"), None)
        if image:
            report["_image"] = image
        return report

    def close(self):
        self.process.stdin.close()
        self.process.wait(timeout=10)


def lathe(profile, **fields):
    return {"shape": "lathe", "profile": profile, "segments": 64, "crease_angle": 42,
            "bevel": .018, "bevel_segments": 3, **fields}


def ball(radius, height, **fields):
    rings = 12 if radius <= .15 else 20
    segments = 32 if radius <= .15 else 48
    profile = [[radius * math.sin(math.pi * i / rings), height - radius * math.cos(math.pi * i / rings)]
               for i in range(rings + 1)]
    profile[0][0] = profile[-1][0] = 0
    return lathe(profile, segments=segments, crease_angle=180, bevel=0, **fields)


def extrusion(polygon, depth, **fields):
    return {"shape": "extrude", "polygon": polygon, "depth": depth, "bevel_segments": 3, **fields}


def box(width, height, depth, **fields):
    return extrusion([[-width / 2, -height / 2], [width / 2, -height / 2],
                      [width / 2, height / 2], [-width / 2, height / 2]], depth, **fields)


def piece(kind):
    if kind == "pawn":
        return [lathe(BASE + [[.28, .44], [.22, .58], [.16, .89], [.17, 1.02],
                             [.24, 1.08], [.24, 1.14], [.19, 1.20]]), ball(.27, 1.43)]
    if kind == "rook":
        parts = [lathe(BASE + [[.31, .44], [.28, .55], [.25, 1.26], [.29, 1.43],
                              [.37, 1.48], [.39, 1.56], [.39, 1.72], [.31, 1.76]])]
        for i in range(8):
            theta = math.tau * i / 8
            parts.append(box(.19, .24, .17, bevel=.018, translation=[.31 * math.sin(theta), 1.85, .31 * math.cos(theta)],
                             rotation=[0, math.degrees(theta), 0]))
        return parts
    if kind == "knight":
        silhouette = [[-.25, .83], [.23, .83], [.27, 1.10], [.08, 1.45], [.22, 1.72],
                      [.52, 1.69], [.63, 1.79], [.63, 1.98], [.29, 2.15], [.18, 2.33],
                      [.10, 2.18], [-.08, 2.22], [-.15, 2.34], [-.24, 2.15],
                      [-.39, 1.96], [-.47, 1.67], [-.39, 1.26]]
        return [lathe(BASE + [[.30, .45], [.25, .66], [.28, .80], [.31, .86], [.31, .92]]),
                extrusion(silhouette, .46, bevel=.025)]
    if kind == "bishop":
        # An open diagonal notch makes a recognizable mitre without boolean subtraction.
        mitre = [[-.15, 1.60], [.16, 1.60], [.31, 1.79], [.29, 2.00], [.20, 2.21],
                 [.065, 2.43], [.018, 2.37], [.14, 2.08], [.075, 2.035],
                 [-.044, 2.27], [-.19, 2.08], [-.29, 1.88], [-.27, 1.73]]
        return [lathe(BASE + [[.27, .48], [.20, .82], [.15, 1.24], [.19, 1.42],
                             [.29, 1.49], [.29, 1.55], [.21, 1.61]]),
                extrusion(mitre, .32, bevel=.010), ball(.07, 2.44)]
    if kind == "queen":
        parts = [lathe(BASE + [[.29, .47], [.21, .81], [.17, 1.47], [.20, 1.65],
                              [.30, 1.74], [.31, 1.80], [.25, 1.85], [.23, 2.00],
                              [.32, 2.21], [.35, 2.27], [.34, 2.33]])]
        for i in range(10):
            theta = math.tau * i / 10
            parts.append(ball(.065, 2.41, translation=[.30 * math.cos(theta), 0, .30 * math.sin(theta)]))
        parts.append(ball(.15, 2.51))
        return parts
    if kind == "king":
        cross = [[-.065, -.19], [.065, -.19], [.065, -.055], [.20, -.055], [.20, .065],
                 [.065, .065], [.065, .24], [-.065, .24], [-.065, .065],
                 [-.20, .065], [-.20, -.055], [-.065, -.055]]
        return [lathe(BASE + [[.30, .48], [.22, .85], [.18, 1.53], [.21, 1.72],
                             [.33, 1.79], [.34, 1.87], [.26, 1.94], [.27, 2.08],
                             [.32, 2.18], [.29, 2.27], [.18, 2.33]]),
                ball(.13, 2.41), extrusion(cross, .15, bevel=.020, translation=[0, 2.72, 0])]
    raise ValueError(kind)


def texture(mcp, name, pattern, colors, seed, **fields):
    result = mcp.call("texture_generate", name=name, pattern=pattern, colors=colors, seed=seed,
                      size=[256, 256], maps=["base_color", "normal", "metallic_roughness", "occlusion"],
                      image=False, overwrite=True, **fields)
    if result["reload_errors"]:
        raise RuntimeError(result["reload_errors"])
    return result["material"]


def lighting(mcp, camera, target):
    for entity in mcp.call("scene_list_entities", name_contains="Editor Startup Script")["entities"]:
        mcp.call("entity_delete", entity=entity["id"])
    mcp.call("environment_set", clear_color="#101720", ambient_intensity=.5)
    mcp.call("postprocess_set", ssao=True, fxaa=True, bloom=False,
             color={"saturation": 1.02, "contrast": 1.03})
    mcp.call("entity_spawn", kind="directional_light", name="Warm key", translation=[-6, 10, 6],
             rotation=[-48, -32, 0], directional_light={"color": "#fff1da", "intensity": 2.2}, quiet=True)
    mcp.call("entity_spawn", kind="directional_light", name="Cool fill", translation=[6, 9, -6],
             rotation=[-35, 135, 0], directional_light={"color": "#aecaff", "intensity": .8}, cast_shadow=False, quiet=True)
    mcp.call("entity_spawn", kind="camera", name="GameCamera", translation=camera, look_at=target,
             camera={"fov_deg": 42, "near": .1, "far": 100}, quiet=True)
    mcp.call("camera_set", position=camera, target=target)
    mcp.call("entity_spawn", kind="plane", name="Studio floor", rotation=[-90, 0, 0], scale=60,
             translation=[0, -.4, 0], material={"color": "#04070b", "roughness": .8}, quiet=True)


def labels(mcp, study=False):
    mcp.call("lua_api_reference", section="UI API")
    title = "CHESS PIECE STUDY" if study else "PROCEDURAL CHESS"
    source = f'''-- @tool
function on_ui(self, ui)
    ui:screen_label("top_left", 30, 26, "{title}", {{size = 25, color = {{0.94, 0.88, 0.74, 1}}}})
    ui:screen_label("top_left", 30, 59, "IVORY MARBLE / EBONY MARBLE", {{size = 13, color = {{0.62, 0.69, 0.75, 1}}}})
'''
    if study:
        for column, kind in enumerate(KINDS):
            source += f'    ui:world_label({(column - 2.5) * 2.0}, 0.28, 1.94, "{kind.upper()}", {{size = 12, color = {{0.95, 0.89, 0.76, 1}}}})\n'
    source += "end\n"
    mcp.call("entity_spawn", name="Scene labels", script={"source": source}, quiet=True)


def active_scene(mcp):
    return mcp.call("editor_state")["active_scene"]


def rename(mcp, name):
    # Register the new tab and write its file before asking the editor to rename it.
    mcp.call("project_save")
    document = active_scene(mcp)
    existing_names = {doc["name"] for doc in mcp.call("editor_state")["scene_documents"]}
    stem = name
    index = 2
    while name in existing_names:
        name = f"{stem} {index}"
        index += 1
    mcp.call("scene_rename", document_id=document, name=name)
    mcp.call("wait", frames=3)
    renamed = active_scene(mcp)
    assert renamed != document, "Scene rename failed; check engine logs"
    return renamed


def screenshot(mcp, path):
    mcp.call("wait", frames=8)
    shot = mcp.call("screenshot", target="viewport", camera="game", aspect="16:9", max_size=1600)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(base64.b64decode(shot.pop("_image")))


def check_pieces(mcp, expected):
    listing = mcp.call("scene_list_entities", name_contains="Chess.")
    entities = listing["entities"]
    assert len(entities) == expected, (len(entities), expected)
    handles = set()
    for entity in entities:
        details = mcp.call("entity_get", entity=entity["id"])
        assert details["mesh"]["local_bounds"] is not None
        assert "primitive" not in details["mesh"] or details["mesh"]["primitive"] is None
        handles.add(details["mesh"]["handle"])
        textures = details["material"]["textures"]
        assert all(slot in textures for slot in ["base_color_texture", "normal_texture", "metallic_roughness_texture", "occlusion_texture"]), textures
    assert len(handles) == 6, f"Expected six shared piece meshes, got {len(handles)}"
    return {"pieces": len(entities), "unique_piece_meshes": len(handles)}


def update_bevels(mcp, project, preview):
    """Replace only geometry in the generated scenes; keep transforms and materials."""
    mcp.call("project_open", path=str(project))
    documents = {doc["name"]: doc["document_id"] for doc in mcp.call("editor_state")["scene_documents"]}
    scenes = {"board": documents["Procedural Chess"], "study": documents["Chess Piece Study"]}
    reports = {}
    for kind, document in scenes.items():
        mcp.call("scene_activate", document_id=document)
        mcp.call("wait", frames=5)
        for entity in mcp.call("scene_list_entities", name_contains="Editor Startup Script")["entities"]:
            mcp.call("entity_delete", entity=entity["id"])
        for entity in mcp.call("scene_list_entities", name_contains="Chess.")["entities"]:
            piece_type = entity["name"].split(".")[2]
            mcp.call("mesh_compose", entity=entity["id"], parts=piece(piece_type))
        if kind == "study":
            for entity in mcp.call("scene_list_entities", name_contains="Pedestal.")["entities"]:
                mcp.call("mesh_lathe", entity=entity["id"], profile=[[.66, 0], [.71, .035], [.71, .09], [.66, .14]],
                         bevel=.012, bevel_segments=3)
        reports[kind] = check_pieces(mcp, 32 if kind == "board" else 12)
        mcp.call("project_save")
        screenshot(mcp, preview / f"{kind}-beveled.png")
    mcp.call("scene_activate", document_id=scenes["board"])
    mcp.call("wait", frames=5)
    mcp.call("project_save")
    mcp.call("engine_call", method="project.open", params={"path": str(project)})
    mcp.call("wait", frames=15)
    for kind, document in scenes.items():
        mcp.call("scene_activate", document_id=document)
        mcp.call("wait", frames=5)
        reports[f"{kind}_reloaded"] = check_pieces(mcp, 32 if kind == "board" else 12)
        screenshot(mcp, preview / f"{kind}-beveled-reloaded.png")
    play = mcp.call("playtest", seconds=1, lean=True)
    assert not play["failing_scripts"] and not play["warnings"], play
    mcp.call("scene_activate", document_id=scenes["board"])
    mcp.call("wait", frames=5)
    warnings = mcp.call("logs", min_level="warn", limit=30)
    assert not warnings["entries"], warnings
    report = {"project": str(project), **reports, "scenes": scenes, "playtest": play, "warnings": warnings}
    (preview / "verification.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(reports, indent=2), flush=True)
    return report


def build(mcp, project, preview):
    if (project / "project.json").exists():
        mcp.call("project_open", path=str(project))
        mcp.call("scene_new")
        mcp.call("wait", frames=5)
    else:
        mcp.call("project_create", name=project.name, location=str(project.parent), template="empty")
    board_scene = rename(mcp, "Procedural Chess")
    print("Generating seeded material maps through MCP", flush=True)
    white = texture(mcp, "ivory_marble", "marble", ["#d4c1a1", "#fff0cf"], 17,
                    scale=3, normal_strength=.003, roughness=.28, roughness_variation=.06, ao_strength=.15)
    black = texture(mcp, "ebony_marble", "marble", ["#111b23", "#35444c"], 29,
                    scale=3, normal_strength=.003, roughness=.25, roughness_variation=.05, ao_strength=.12)
    maple = texture(mcp, "maple_square", "wood", ["#b9a383", "#e9d5b0"], 5,
                    scale=3, normal_strength=.002, roughness=.4, roughness_variation=.05)
    walnut = texture(mcp, "walnut_square", "wood", ["#302119", "#6b4830"], 11,
                     scale=3, normal_strength=.002, roughness=.4, roughness_variation=.05)
    brass = texture(mcp, "brushed_brass", "noise", ["#a77b36", "#c8a458"], 43,
                    scale=8, normal_strength=.0004, roughness=.27, roughness_variation=.04, metallic=.8)
    print("Building turned bodies and extruded ornaments", flush=True)
    lighting(mcp, [10.8, 11.7, 13.8], [0, .5, 0])
    labels(mcp)
    mcp.call("mesh_extrude", name="Board brass rim", polygon=[[-5.25, -5.25], [5.25, -5.25], [5.25, 5.25], [-5.25, 5.25]],
             depth=.28, bevel=.055, rotation=[-90, 0, 0], translation=[0, -.15, 0], material=brass)
    mcp.call("mesh_extrude", name="Board walnut frame", polygon=[[-5.16, -5.16], [5.16, -5.16], [5.16, 5.16], [-5.16, 5.16]],
             depth=.23, bevel=.04, rotation=[-90, 0, 0], translation=[0, .035, 0], material=walnut)
    mcp.call("mesh_extrude", name="Walnut a1", polygon=[[-.597, -.597], [.597, -.597], [.597, .597], [-.597, .597]],
             depth=.09, bevel=.008, rotation=[-90, 0, 0], translation=[-4.2, .18, 4.2], material=walnut)
    mcp.call("mesh_extrude", name="Maple b1", polygon=[[-.597, -.597], [.597, -.597], [.597, .597], [-.597, .597]],
             depth=.09, bevel=.008, rotation=[-90, 0, 0], translation=[-3., .18, 4.2], material=maple)
    for rank in range(8):
        for file in range(8):
            if rank == 0 and file < 2:
                continue
            square = "Walnut a1" if (rank + file) % 2 == 0 else "Maple b1"
            mcp.call("entity_duplicate", entity=square, name=f"Square {chr(97 + file)}{rank + 1}",
                     translation=[-4.2 + 1.2 * file, .18, 4.2 - 1.2 * rank], quiet=True)
    first = {}
    for side, material, sign in [("White", white, 1), ("Black", black, -1)]:
        for file, kind in enumerate(ORDER):
            name = f"Chess.{side}.{kind}.{file + 1}"
            position = [-4.2 + file * 1.2, .226, sign * 4.2]
            if kind not in first:
                first[kind] = mcp.call("mesh_compose", name=name, parts=piece(kind), translation=position, material=material)["id"]
            else:
                mcp.call("entity_duplicate", entity=first[kind], name=name, translation=position, material=material, quiet=True)
        pawn = mcp.call("mesh_compose", name=f"Chess.{side}.pawn.1", parts=piece("pawn"),
                        translation=[-4.2, .226, sign * 3.0], material=material)
        mcp.call("entity_duplicate", entity=pawn["id"], count=7, step=[1.2, 0, 0], name=f"Chess.{side}.pawn.copy-{{i}}", quiet=True)
    board_report = check_pieces(mcp, 32)
    mcp.call("project_save")
    screenshot(mcp, preview / "board.png")
    print("Building the twelve-piece material and silhouette study", flush=True)
    mcp.call("scene_new")
    mcp.call("wait", frames=5)
    study_scene = rename(mcp, "Chess Piece Study")
    lighting(mcp, [5.5, 5.6, 13.0], [0, 1.1, 0])
    labels(mcp, study=True)
    mcp.call("mesh_extrude", name="Study plinth", polygon=[[-6.5, -2.1], [6.5, -2.1], [6.5, 2.1], [-6.5, 2.1]],
             depth=.24, bevel=.06, rotation=[-90, 0, 0], translation=[0, -.08, 0], material=walnut)
    for column, kind in enumerate(KINDS):
        for side, material, z in [("White", white, 1.0), ("Black", black, -1.0)]:
            x = (column - 2.5) * 2.0
            mcp.call("mesh_lathe", name=f"Pedestal.{side}.{kind}",
                     profile=[[.66, 0], [.71, .035], [.71, .09], [.66, .14]],
                     translation=[x, .05, z], material=brass)
            mcp.call("mesh_compose", name=f"Chess.{side}.{kind}", parts=piece(kind),
                     translation=[x, .191, z], material=material)
    study_report = check_pieces(mcp, 12)
    mcp.call("project_save")
    screenshot(mcp, preview / "piece-study.png")
    # Force a project reload, then verify sharing and material bindings survived saving.
    mcp.call("scene_activate", document_id=board_scene)
    mcp.call("wait", frames=5)
    mcp.call("project_save")
    mcp.call("engine_call", method="project.open", params={"path": str(project)})
    mcp.call("wait", frames=15)
    mcp.call("scene_activate", document_id=board_scene)
    mcp.call("wait", frames=8)
    reloaded = check_pieces(mcp, 32)
    screenshot(mcp, preview / "board-reloaded.png")
    mcp.call("scene_activate", document_id=study_scene)
    mcp.call("wait", frames=5)
    play = mcp.call("playtest", seconds=1, lean=True)
    assert not play["failing_scripts"], play
    assert not play["warnings"], play
    mcp.call("scene_activate", document_id=board_scene)
    mcp.call("wait", frames=5)
    warnings = mcp.call("logs", min_level="warn", limit=30)
    assert not warnings["entries"], warnings
    report = {"project": str(project), "board": board_report, "study": study_report,
              "reloaded": reloaded, "playtest": play, "warnings": warnings,
              "scenes": {"board": board_scene, "study": study_scene}}
    (preview / "verification.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: value for key, value in report.items() if key not in ["playtest", "warnings"]}, indent=2), flush=True)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--port", type=int, default=47819, help="Dedicated editor port; use an unused port")
    parser.add_argument("--project", type=Path, default=REPO / "examples/projects/procedural-chess")
    parser.add_argument("--preview", type=Path, default=REPO / "target/engine-mcp/chess-preview")
    parser.add_argument("--keep-open", action="store_true")
    parser.add_argument("--update-bevels", action="store_true", help="Update geometry in the saved board/study without adding scenes")
    args = parser.parse_args()
    mcp = MCP(args.port)
    launched = False
    report = None
    try:
        status = mcp.call("engine_status")
        # Refuse to change a user-owned editor's current project.
        if status.get("connected") or status.get("listening"):
            raise RuntimeError("This port already has an engine; choose another --port")
        mcp.call("engine_launch", target="editor", build=False)
        launched = True
        generator = update_bevels if args.update_bevels else build
        report = generator(mcp, args.project.resolve(), args.preview.resolve())
    finally:
        if launched and not args.keep_open:
            mcp.call("engine_stop")
        mcp.close()
    if report and not args.keep_open:
        # Compact numerical mesh arrays for the checked-in fixtures. Same scene format.
        for document in report["scenes"].values():
            path = args.project / "content/scenes" / f"{document}.json"
            path.write_text(json.dumps(json.loads(path.read_text()), separators=(",", ":")) + "\n")


if __name__ == "__main__":
    main()
