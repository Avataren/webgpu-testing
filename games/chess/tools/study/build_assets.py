"""Build every chess study GLB headless:

    python3 games/chess/tools/study/textures.py
    blender -b --python games/chess/tools/study/build_assets.py [-- names...]

names: chess_set, furniture, room (default all). The GLBs land in build/glb; import
them into games/chess/study with the engine MCP's asset_import.
"""

import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import chess_set  # noqa: E402
import furniture  # noqa: E402
import room  # noqa: E402

args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
jobs = {
    "chess_set": lambda: chess_set.build(export=True),
    "furniture": furniture.export,
    "room": lambda: room.build(export=True),
}
for name in args or jobs:
    print(name, "->", jobs[name]())
