#!/usr/bin/env python3
"""Copy Lantern Market's cast from the asset library into the project.

Reads cast.json (characters and clips by library id), copies each one the library has
into content/characters/<id>.glb and content/animations/<clip>.glb, and writes
content/scripts/cast_data.lua, the list the market's director loads: every character
that was found with its groups, height and triangle count, and every clip with its
length and how fast it travels.

    games/lantern-market/tools/fetch_cast.py           copy what is missing or changed
    games/lantern-market/tools/fetch_cast.py --check   only report what the library lacks
    games/lantern-market/tools/fetch_cast.py --prune   also delete files no longer in the cast
"""

import argparse
import importlib.util
import json
import os
import shutil
import sys

PROJECT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REPO = os.path.dirname(os.path.dirname(PROJECT))
CHARACTERS = "content/characters"
ANIMATIONS = "content/animations"
CAST_DATA = "content/scripts/cast_data.lua"
PROVIDER_PREFIXES = ("mixamo-", "quaternius-")


def load_library_module():
    # Importing it must not leave a __pycache__ in the repository's tools folder.
    sys.dont_write_bytecode = True
    path = os.path.join(REPO, "tools", "library", "library.py")
    spec = importlib.util.spec_from_file_location("library", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def clip_key(asset_id):
    """The clip's file stem, which is also the name the animator plays it by."""
    for prefix in PROVIDER_PREFIXES:
        if asset_id.startswith(prefix):
            return asset_id[len(prefix):]
    return asset_id


def copy_if_changed(source, target):
    if os.path.exists(target):
        a, b = os.stat(source), os.stat(target)
        if a.st_size == b.st_size and int(a.st_mtime) == int(b.st_mtime):
            return False
    os.makedirs(os.path.dirname(target), exist_ok=True)
    shutil.copy2(source, target)
    return True


def lua_string(text):
    return json.dumps(text, ensure_ascii=True)


def lua_number(value):
    return f"{value:.4g}" if isinstance(value, float) else str(value)


def write_cast_data(path, characters, clips):
    lines = [
        "-- Written by tools/fetch_cast.py from cast.json and the asset library: do not edit.",
        "return {",
        "    characters = {",
    ]
    for c in characters:
        groups = ", ".join(lua_string(g) for g in c["groups"])
        lines.append(
            f"        {{ id = {lua_string(c['id'])}, name = {lua_string(c['name'])}, "
            f"path = {lua_string(c['path'])}, height = {lua_number(c['height'])}, "
            f"triangles = {c['triangles']}, bones = {c['bones']}, groups = {{ {groups} }} }},"
        )
    lines += ["    },", "    clips = {"]
    for key, c in sorted(clips.items()):
        lines.append(
            f"        [{lua_string(key)}] = {{ path = {lua_string(c['path'])}, "
            f"duration = {lua_number(c['duration'])}, speed = {lua_number(c['speed'])}, "
            f"loop = {'true' if c['loop'] else 'false'} }},"
        )
    lines += ["    },", "}", ""]
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--check", action="store_true", help="only report what is missing")
    parser.add_argument("--prune", action="store_true", help="delete files not in the cast")
    args = parser.parse_args()

    library = load_library_module()
    root = library.library_root()
    with open(os.path.join(root, "library.json"), encoding="utf-8") as f:
        index = {asset["id"]: asset for asset in json.load(f)["assets"]}
    with open(os.path.join(PROJECT, "cast.json"), encoding="utf-8") as f:
        cast = json.load(f)

    missing = []
    copied = 0
    characters = []
    for entry in cast["characters"]:
        asset = index.get(entry["id"])
        if asset is None or asset["kind"] != "character":
            missing.append(entry["id"])
            continue
        rel = f"{CHARACTERS}/{entry['id']}.glb"
        if not args.check:
            source = os.path.join(root, "assets", asset["id"], asset["file"])
            copied += copy_if_changed(source, os.path.join(PROJECT, rel))
        characters.append(
            {
                "id": entry["id"],
                "name": entry.get("name", asset["name"]),
                "path": rel,
                "height": asset.get("height", 1.8),
                "triangles": asset.get("triangles", 0),
                "bones": asset.get("bones", 0),
                "groups": entry.get("groups", []),
            }
        )

    clips = {}
    for asset_id in cast["clips"]:
        asset = index.get(asset_id)
        if asset is None or asset["kind"] != "animation" or not asset.get("clips"):
            missing.append(asset_id)
            continue
        key = clip_key(asset_id)
        rel = f"{ANIMATIONS}/{key}.glb"
        if not args.check:
            source = os.path.join(root, "assets", asset["id"], asset["file"])
            copied += copy_if_changed(source, os.path.join(PROJECT, rel))
        clip = asset["clips"][0]
        clips[key] = {
            "path": rel,
            "duration": clip.get("duration", 1.0),
            "speed": clip.get("root_speed", 0.0),
            "loop": bool(clip.get("loopable", True)),
        }

    if missing:
        print(f"The library lacks {len(missing)} of the cast:", file=sys.stderr)
        for asset_id in missing:
            print(f"  {asset_id}", file=sys.stderr)
        print(
            "Fill it with tools/library/library.py fetch-pack all / fetch-mixamo "
            "(see docs/asset_library.md).",
            file=sys.stderr,
        )
    if args.check:
        print(f"{len(characters)} characters and {len(clips)} clips available")
        return 0 if not missing else 1

    if args.prune:
        wanted = {c["path"] for c in characters} | {c["path"] for c in clips.values()}
        for folder in (CHARACTERS, ANIMATIONS):
            directory = os.path.join(PROJECT, folder)
            for name in os.listdir(directory) if os.path.isdir(directory) else []:
                if f"{folder}/{name}" not in wanted:
                    os.remove(os.path.join(directory, name))
                    print(f"removed {folder}/{name}")

    write_cast_data(os.path.join(PROJECT, CAST_DATA), characters, clips)
    print(
        f"{len(characters)} characters and {len(clips)} clips ready "
        f"({copied} files copied); wrote {CAST_DATA}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
