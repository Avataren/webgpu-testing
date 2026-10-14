#!/usr/bin/env python3
"""Run a JSON batch through the engine MCP; save image blocks to requested paths."""
import argparse
import base64
import importlib.util
import json
import sys
from pathlib import Path

sys.dont_write_bytecode = True
root = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("chess_assets", root / "tools/engine-mcp/examples/procedural_chess.py")
assets = importlib.util.module_from_spec(spec)
spec.loader.exec_module(assets)
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--port", type=int, required=True)
parser.add_argument("batch", type=Path)
args = parser.parse_args()
mcp = assets.MCP(args.port)
try:
    for operation in json.loads(args.batch.read_text()):
        result = mcp.call(operation["tool"], **operation.get("args", {}))
        image = result.pop("_image", None)
        if image and "image_path" in operation:
            path = Path(operation["image_path"])
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(base64.b64decode(image))
        print(json.dumps({"tool": operation["tool"], "result": result}), flush=True)
finally:
    mcp.close()
