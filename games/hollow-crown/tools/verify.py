#!/usr/bin/env python3
"""Replay the expanded dungeon's saved scenarios through the actual engine MCP."""
import argparse
import importlib.util
import json
import sys
from pathlib import Path

sys.dont_write_bytecode = True
REPO = Path(__file__).resolve().parents[3]
PROJECT = REPO / 'games/hollow-crown'
spec = importlib.util.spec_from_file_location('engine_examples', REPO / 'tools/engine-mcp/examples/procedural_chess.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument('--port', type=int, default=47811)
args = parser.parse_args()
client = module.MCP(args.port)
reports = {}
try:
    info = client.call('engine_info')
    root = Path(info.get('project_root') or '')
    if not root.is_absolute():
        root = REPO / root
    if root.resolve() != PROJECT or not info.get('editor') or info['runtime_mode'] != 'editor':
        raise RuntimeError('Open hollow-crown in the editor and stop Play before replaying tests.')
    for name in ('victory', 'defeat', 'equipment', 'recovery', 'manual', 'manual-arrival'):
        report = client.call('playtest', scenario=f'games/hollow-crown/tests/expansion/{name}.json')
        reports[name] = report
        print(f'{name}: {"PASS" if report.get("ok") else "FAIL"}', flush=True)
    path = PROJECT / 'tests/expansion/last-replay.json'
    path.write_text(json.dumps(reports, indent=2) + '\n')
    print(f'Results: {path}')
    if not all(report.get('ok') for report in reports.values()):
        raise SystemExit(1)
finally:
    client.close()
