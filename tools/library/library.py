#!/usr/bin/env python3
"""The engine's asset library: characters, animations and models shared by every project.

The library lives outside projects and outside git, in `$WGPU_CUBE_LIBRARY` or
`<user data dir>/wgpu-cube/library` (`~/.local/share/wgpu-cube/library` on Linux). Each
asset is a folder `assets/<id>/` with `asset.json`, one GLB and a thumbnail;
`library.json` indexes them for the editor and the MCP server. Projects copy in only the
assets they use (the editor's Library panel, or MCP `library_add`), so a game's export
carries only those.

    library.py list [--kind animation] [--query walk] [--json]
    library.py info ID
    library.py ingest FILE... [--provider P] [--license L] [--author A] [--url U]
                              [--tags a,b] [--name NAME] [--prefix ID_PREFIX]
    library.py fetch-pack quaternius-ual1|quaternius-ual2|all
    library.py fetch-mixamo [--set tools/library/mixamo_set.json] [--only NAME...]
                            [--characters] [--animations] [--query TEXT]
    library.py remove ID...
    library.py reindex

FBX, .blend and OBJ files go through Blender (found like the editor finds it:
`BLENDER_PATH`, the PATH, or the usual install places). Every file is split into one
asset per animation, measured (length, ground speed, in place or not) and given a
thumbnail; files with meshes also give a character (or model) asset.

Mixamo downloads use your own session token, read from `$MIXAMO_TOKEN` or
`~/.config/wgpu-cube/mixamo-token` and never printed. Mixamo's terms allow the files in
your projects, not redistributing them, so they stay in the library (never commit them).
"""

import argparse
import datetime
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import zipfile

HERE = os.path.dirname(os.path.abspath(__file__))
USER_AGENT = "wgpu-cube-library/1.0"
KINDS = ("character", "animation", "model")

# --------------------------------------------------------------------------------------
# Places
# --------------------------------------------------------------------------------------


def data_dir():
    if sys.platform == "win32":
        return os.environ.get("APPDATA") or os.path.expanduser("~")
    if sys.platform == "darwin":
        return os.path.expanduser("~/Library/Application Support")
    return os.environ.get("XDG_DATA_HOME") or os.path.expanduser("~/.local/share")


def library_root(override=None):
    root = override or os.environ.get("WGPU_CUBE_LIBRARY") or os.path.join(
        data_dir(), "wgpu-cube", "library"
    )
    os.makedirs(os.path.join(root, "assets"), exist_ok=True)
    return root


def find_blender():
    path = os.environ.get("BLENDER_PATH")
    if path and os.path.isfile(path):
        return path
    found = shutil.which("blender")
    if found:
        return found
    home = os.path.expanduser("~")
    for candidate in (
        f"{home}/.local/share/Steam/steamapps/common/Blender/blender",
        f"{home}/.steam/steam/steamapps/common/Blender/blender",
        "/opt/blender/blender",
        "/snap/bin/blender",
        "/var/lib/flatpak/exports/bin/org.blender.Blender",
        f"{home}/.local/share/flatpak/exports/bin/org.blender.Blender",
        "/Applications/Blender.app/Contents/MacOS/Blender",
        r"C:\Program Files (x86)\Steam\steamapps\common\Blender\blender.exe",
    ):
        if os.path.isfile(candidate):
            return candidate
    raise SystemExit("Blender was not found: install it, or set BLENDER_PATH")


# --------------------------------------------------------------------------------------
# Catalog
# --------------------------------------------------------------------------------------


def slug(text):
    text = re.sub(r"([a-z0-9])([A-Z])", r"\1-\2", text)
    return re.sub(r"[^a-z0-9]+", "-", text.lower()).strip("-") or "asset"


def load_assets(root):
    assets = []
    base = os.path.join(root, "assets")
    for name in sorted(os.listdir(base)):
        path = os.path.join(base, name, "asset.json")
        if os.path.isfile(path):
            with open(path, encoding="utf-8") as file:
                assets.append(json.load(file))
    return assets


def write_index(root):
    assets = load_assets(root)
    index = {
        "version": 1,
        "updated": now(),
        "assets": assets,
    }
    path = os.path.join(root, "library.json")
    with open(path + ".tmp", "w", encoding="utf-8") as file:
        json.dump(index, file, indent=1)
    os.replace(path + ".tmp", path)
    return assets


def now():
    return datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat()


def skeleton_family(bones):
    names = " ".join(bones).lower()
    if "mixamorig" in names or ("leftupleg" in names and "hips" in names):
        return "mixamo"
    if "pelvis" in names and "thigh_l" in names:
        return "unreal"
    if "def-" in names:
        return "rigify"
    return "custom" if bones else None


# Words that put a clip in a category, for browsing.
CATEGORIES = {
    "locomotion": ["walk", "run", "jog", "sprint", "strafe", "crouch", "sneak", "turn",
                   "backward", "swim", "crawl", "slide", "climb", "jump", "fall", "land",
                   "roll", "dodge", "dash"],
    "idle": ["idle", "breathing", "standing"],
    "combat": ["sword", "punch", "kick", "jab", "hook", "cross", "block", "attack", "slash",
               "stab", "shoot", "fire", "pistol", "rifle", "gun", "reload", "aim", "throw",
               "shield", "spell", "magic", "melee", "boxing", "mma", "combo"],
    "hit": ["hit", "death", "dying", "die", "knockback", "knock", "stumble", "getting up",
            "get up", "injured", "defeat"],
    "emote": ["dance", "dancing", "wave", "waving", "cheer", "clap", "victory", "talk",
              "laugh", "bow", "salute", "thumb", "yes", "no", "angry", "yell", "shrug",
              "think", "taunt", "celebrat"],
    "interaction": ["pick", "interact", "push", "pull", "sit", "chest", "open", "farm",
                    "harvest", "plant", "water", "consume", "drink", "eat", "carry",
                    "chop", "fix", "kneel", "type", "typing", "lever", "drive", "phone",
                    "lantern", "torch", "rail", "lay", "lying", "stand up"],
}


def tags_for(name, extra=()):
    words = re.sub(r"([a-z])([A-Z])", r"\1 \2", name).replace("_", " ").lower()
    tags = set(extra)
    for category, keys in CATEGORIES.items():
        if any(key in words for key in keys):
            tags.add(category)
    if "loop" in words.split():
        tags.add("loop")
    if words.endswith(" rm") or " rm " in words:
        tags.add("root-motion")
    return sorted(tags)


def clean_clip_name(name):
    """`Sword_Regular_A` -> `Sword Regular A`; Mixamo names stay as they are."""
    return re.sub(r"\s+", " ", name.replace("_", " ")).strip()


# --------------------------------------------------------------------------------------
# Ingest
# --------------------------------------------------------------------------------------


def run_blender(source, out, reference=None, name=None):
    # At most 8 threads, at low priority: the machine is shared.
    command = [
        find_blender(), "-b", "--factory-startup", "--threads", "8", "--python",
        os.path.join(HERE, "blender_ingest.py"), "--", "--input", source, "--out", out,
    ]
    if reference:
        command += ["--reference", reference]
    if name:
        command += ["--name", name]
    result = subprocess.run(
        command, capture_output=True, text=True,
        preexec_fn=(lambda: os.nice(10)) if hasattr(os, "nice") else None,
    )
    report = None
    for line in result.stdout.splitlines():
        if line.startswith("INGEST "):
            report = json.loads(line[len("INGEST "):])
    if result.returncode != 0 or report is None:
        tail = "\n".join((result.stdout + result.stderr).strip().splitlines()[-12:])
        raise RuntimeError(f"Blender could not read {source}:\n{tail}")
    return report


def reference_character(root, family):
    """A library character with this skeleton, to show animation-only files on."""
    preferred = {"mixamo": "mixamo-y-bot", "unreal": "quaternius-ual2-mannequin"}.get(family)
    assets = load_assets(root)
    by_id = {asset["id"]: asset for asset in assets}
    choice = by_id.get(preferred) if preferred else None
    if choice is None:
        choice = next(
            (a for a in assets if a["kind"] == "character" and a.get("skeleton") == family),
            None,
        )
    if choice is None:
        return None
    return os.path.join(root, "assets", choice["id"], choice["file"])


def store(root, asset_id, kind, name, glb, thumbnail, fields):
    folder = os.path.join(root, "assets", asset_id)
    if os.path.isdir(folder):
        shutil.rmtree(folder)
    os.makedirs(folder)
    file_name = f"{asset_id}.glb"
    shutil.copyfile(glb, os.path.join(folder, file_name))
    entry = {"id": asset_id, "kind": kind, "name": name, "file": file_name}
    if thumbnail and os.path.isfile(thumbnail):
        shutil.copyfile(thumbnail, os.path.join(folder, "thumbnail.png"))
        entry["thumbnail"] = "thumbnail.png"
    entry.update(fields)
    entry["added"] = now()
    with open(os.path.join(folder, "asset.json"), "w", encoding="utf-8") as file:
        json.dump(entry, file, indent=2)
    return entry


def ingest(root, source, provider="local", license_text="", author="", url="",
           tags=(), name=None, prefix=None, kind=None, character_name=None,
           source_extra=None, family_hint=None):
    """Add every animation and the character in `source`; returns the new entries."""
    stem = name or os.path.splitext(os.path.basename(source))[0]
    prefix = prefix or provider
    work = tempfile.mkdtemp(prefix="library-ingest-")
    try:
        # Animation-only files are shown on a library character with the same skeleton
        # (a first pass finds out which; Mixamo's is the common case).
        reference = reference_character(root, family_hint) if family_hint else None
        report = run_blender(source, work, reference=reference, name=name)
        family = skeleton_family(report.get("bones", []))
        if reference is None and report["character"] is None and family:
            reference = reference_character(root, family)
            if reference:
                shutil.rmtree(work)
                work = tempfile.mkdtemp(prefix="library-ingest-")
                report = run_blender(source, work, reference=reference, name=name)
        source_info = {"provider": provider, "license": license_text}
        if author:
            source_info["author"] = author
        if url:
            source_info["url"] = url
        source_info["file"] = os.path.basename(source)
        if source_extra:
            source_info.update(source_extra)
        entries = []
        character = report["character"]
        if character is not None:
            label = character_name or stem
            entry_kind = kind or ("character" if report.get("bones") else "model")
            entries.append(store(
                root, slug(f"{prefix}-{label}"), entry_kind, label, character["file"],
                character["thumbnail"],
                {
                    "tags": sorted(set(tags) | {entry_kind}),
                    "skeleton": family if entry_kind == "character" else None,
                    "height": character["height"],
                    "triangles": character["triangles"],
                    "bones": character["bones"],
                    "source": source_info,
                },
            ))
        for clip in report["clips"]:
            clip_name = clean_clip_name(clip["name"])
            entries.append(store(
                root, slug(f"{prefix}-{clip_name}"), "animation", clip_name, clip["file"],
                clip["thumbnail"],
                {
                    "tags": tags_for(clip["name"], tags),
                    "skeleton": family,
                    "clips": [{
                        "name": clip_name,
                        "duration": clip["duration"],
                        "root_speed": clip["root_speed"],
                        "in_place": clip["in_place"],
                    }],
                    "bones": len(report.get("bones", [])),
                    "source": source_info,
                },
            ))
        return entries
    finally:
        shutil.rmtree(work, ignore_errors=True)


# --------------------------------------------------------------------------------------
# CC0 packs
# --------------------------------------------------------------------------------------

CC0 = "CC0 1.0 (public domain)"

PACKS = {
    "quaternius-ual1": {
        "title": "Quaternius Universal Animation Library (Standard)",
        "url": "https://opengameart.org/sites/default/files/universal_animation_librarystandard.zip",
        "page": "https://quaternius.com/packs/universalanimationlibrary.html",
        "files": [
            {"path": "Godot/AnimationLibrary_Godot_Standard.glb", "prefix": "quaternius-ual1",
             "character": "Mannequin (Rigify)"},
        ],
    },
    "quaternius-ual2": {
        "title": "Quaternius Universal Animation Library 2 (Standard)",
        "url": "https://opengameart.org/sites/default/files/universal_animation_library_2standard.zip",
        "page": "https://opengameart.org/content/universal-animation-library-2",
        "files": [
            {"path": "Unreal-Godot/UAL2_Standard.glb", "prefix": "quaternius-ual2",
             "character": "Mannequin"},
            {"path": "Female Mannequin/Unreal-Godot/Mannequin_F.glb",
             "prefix": "quaternius-ual2", "character": "Mannequin F"},
        ],
    },
}


def download(url, path, headers=None):
    request = urllib.request.Request(url, headers={"User-Agent": USER_AGENT, **(headers or {})})
    with urllib.request.urlopen(request, timeout=300) as response, open(path + ".part", "wb") as out:
        shutil.copyfileobj(response, out)
    os.replace(path + ".part", path)


def fetch_pack(root, key):
    pack = PACKS[key]
    cache = os.path.join(root, "cache", key)
    os.makedirs(cache, exist_ok=True)
    archive = os.path.join(cache, os.path.basename(pack["url"]))
    if not os.path.isfile(archive):
        print(f"downloading {pack['url']}", file=sys.stderr)
        download(pack["url"], archive)
    extracted = os.path.join(cache, "files")
    if not os.path.isdir(extracted):
        with zipfile.ZipFile(archive) as zipped:
            zipped.extractall(extracted)
    entries = []
    for item in pack["files"]:
        matches = [
            os.path.join(base, name)
            for base, _, names in os.walk(extracted)
            for name in names
            if os.path.join(base, name).replace(os.sep, "/").endswith(item["path"])
        ]
        if not matches:
            print(f"{key}: {item['path']} is not in the archive", file=sys.stderr)
            continue
        print(f"{key}: ingesting {item['path']}", file=sys.stderr)
        entries += ingest(
            root, matches[0], provider="quaternius", license_text=CC0, author="Quaternius",
            url=pack["page"], tags=["quaternius"], prefix=item["prefix"],
            character_name=item.get("character"), source_extra={"pack": pack["title"]},
        )
    return entries


# --------------------------------------------------------------------------------------
# Mixamo
# --------------------------------------------------------------------------------------

MIXAMO_API = "https://www.mixamo.com/api/v1"
MIXAMO_LICENSE = (
    "Mixamo (Adobe): royalty-free in your personal, commercial and non-profit projects; "
    "do not redistribute the raw files"
)


class Mixamo:
    def __init__(self):
        token = os.environ.get("MIXAMO_TOKEN")
        path = os.path.expanduser("~/.config/wgpu-cube/mixamo-token")
        if not token and os.path.isfile(path):
            with open(path, encoding="utf-8") as file:
                token = file.read().strip()
        if not token:
            raise SystemExit(
                "No Mixamo token. Log in at mixamo.com, run copy(localStorage.access_token) "
                f"in the browser console and paste it into {path} (chmod 600)."
            )
        self.token = token

    def call(self, method, path, body=None, attempts=5):
        url = path if path.startswith("http") else MIXAMO_API + path
        data = json.dumps(body).encode() if body is not None else None
        for attempt in range(attempts):
            request = urllib.request.Request(url, data=data, method=method, headers={
                "Accept": "application/json",
                "Content-Type": "application/json",
                "Authorization": f"Bearer {self.token}",
                "X-Api-Key": "mixamo2",
                "X-Requested-With": "XMLHttpRequest",
                "User-Agent": USER_AGENT,
            })
            try:
                with urllib.request.urlopen(request, timeout=60) as response:
                    text = response.read().decode() or "{}"
                    return response.status, json.loads(text)
            except urllib.error.HTTPError as error:
                if error.code == 429 and attempt + 1 < attempts:
                    time.sleep(2 + attempt * 3)
                    continue
                if error.code == 401:
                    raise SystemExit(
                        "Mixamo refused the token (401): it has expired. Copy a fresh one "
                        "into ~/.config/wgpu-cube/mixamo-token."
                    ) from None
                detail = error.read().decode(errors="replace")[:300]
                raise RuntimeError(f"Mixamo {method} {path}: HTTP {error.code} {detail}") from None
        raise RuntimeError(f"Mixamo {method} {path}: rate limited")

    def search(self, query, kind="Motion,MotionPack", limit=24):
        params = urllib.parse.urlencode(
            {"page": 1, "limit": limit, "order": "", "type": kind, "query": query}
        )
        _, result = self.call("GET", f"/products?{params}")
        return result.get("results", [])

    def find(self, name, kind="Motion,MotionPack"):
        results = self.search(name, kind)
        wanted = name.lower()
        for item in results:
            if (item.get("name") or "").lower() == wanted:
                return item
        for item in results:
            if (item.get("description") or "").lower() == wanted:
                return item
        singles = [item for item in results if "," not in (item.get("name") or "")]
        return singles[0] if singles else None

    def wait_for_export(self, character_id, timeout=180):
        deadline = time.time() + timeout
        while time.time() < deadline:
            _, message = self.call("GET", f"/characters/{character_id}/monitor")
            status = message.get("status")
            if status == "completed":
                return message["job_result"]
            if status not in ("processing", None, "pending", "queued"):
                raise RuntimeError(f"Mixamo export {status}: {message.get('message')}")
            time.sleep(1.0)
        raise RuntimeError("Mixamo export timed out")

    def export_motion(self, character_id, product, in_place=False):
        _, details = self.call(
            "GET", f"/products/{product['id']}?similar=0&character_id={character_id}"
        )
        gms = dict(details["details"]["gms_hash"])
        params = gms.get("params", [])
        values = []
        for param in params:
            value = param[1] if isinstance(param, list) and len(param) > 1 else param
            values.append(json.dumps(value) if isinstance(value, bool) else str(value))
        gms["params"] = ",".join(values)
        if in_place and "inplace" in gms:
            gms["inplace"] = True
        self.call("POST", "/animations/export", {
            "character_id": character_id,
            "gms_hash": [gms],
            "preferences": {"format": "fbx7", "skin": "false", "fps": "30", "reducekf": "0"},
            "product_name": product.get("name") or product.get("description"),
            "type": "Motion",
        })
        return self.wait_for_export(character_id)

    def export_character(self, character_id, name):
        self.call("POST", "/animations/export", {
            "character_id": character_id,
            "gms_hash": None,
            "preferences": {"format": "fbx7", "mesh": "t-pose"},
            "product_name": name,
            "type": "Character",
        })
        return self.wait_for_export(character_id)


def fetch_mixamo(root, set_path, only=None, characters=True, animations=True, queries=()):
    with open(set_path, encoding="utf-8") as file:
        wanted = json.load(file)
    client = Mixamo()
    inbox = os.path.join(root, "inbox", "mixamo")
    os.makedirs(inbox, exist_ok=True)
    have = {asset["id"] for asset in load_assets(root)}
    entries, failures = [], []
    only = {name.lower() for name in only or []}

    # The skeleton (and proportions) animations are exported for.
    base = wanted.get("animation_character", "Y Bot")
    base_product = client.find(base, kind="Character")
    if base_product is None:
        raise SystemExit(f"Mixamo has no character called {base!r}")
    base_id = base_product["id"]

    if characters:
        for name in wanted.get("characters", []):
            if only and name.lower() not in only:
                continue
            asset_id = slug(f"mixamo-{name}")
            if asset_id in have:
                continue
            try:
                product = client.find(name, kind="Character")
                if product is None:
                    raise RuntimeError("not found")
                url = client.export_character(product["id"], name)
                path = os.path.join(inbox, f"{slug(name)}.fbx")
                download(url, path)
                entries += ingest(
                    root, path, provider="mixamo", license_text=MIXAMO_LICENSE,
                    author="Adobe Mixamo", url="https://www.mixamo.com/", tags=["mixamo"],
                    name=name, prefix="mixamo", character_name=name,
                    source_extra={"mixamo_id": product["id"]},
                )
                print(f"mixamo character: {name}", file=sys.stderr)
            except Exception as error:  # noqa: BLE001 - report and carry on
                failures.append({"name": name, "error": str(error)})
                print(f"mixamo character {name}: {error}", file=sys.stderr)
            time.sleep(1.0)

    if animations:
        items = list(wanted.get("animations", []))
        items += [{"name": query, "tags": []} for query in queries]
        for item in items:
            name = item["name"]
            if only and name.lower() not in only:
                continue
            asset_id = slug(f"mixamo-{name}")
            if asset_id in have:
                continue
            try:
                product = client.find(name)
                if product is None:
                    raise RuntimeError("not found")
                found = product.get("name") or product.get("description") or name
                url = client.export_motion(base_id, product, in_place=item.get("in_place", False))
                path = os.path.join(inbox, f"{slug(name)}.fbx")
                download(url, path)
                entries += ingest(
                    root, path, provider="mixamo", license_text=MIXAMO_LICENSE,
                    author="Adobe Mixamo", url="https://www.mixamo.com/",
                    tags=["mixamo", *item.get("tags", [])], name=name, prefix="mixamo",
                    source_extra={"mixamo_id": product["id"], "mixamo_name": found},
                    family_hint="mixamo",
                )
                print(f"mixamo animation: {name} ({found})", file=sys.stderr)
            except SystemExit:
                raise
            except Exception as error:  # noqa: BLE001 - report and carry on
                failures.append({"name": name, "error": str(error)})
                print(f"mixamo animation {name}: {error}", file=sys.stderr)
            time.sleep(1.0)
    return entries, failures


# --------------------------------------------------------------------------------------
# Command line
# --------------------------------------------------------------------------------------


def summary(asset):
    out = {key: asset.get(key) for key in ("id", "kind", "name", "tags", "skeleton")}
    if asset.get("clips"):
        clip = asset["clips"][0]
        out.update(duration=clip["duration"], root_speed=clip["root_speed"],
                   in_place=clip["in_place"])
    if asset.get("height"):
        out["height"] = asset["height"]
    out["provider"] = asset.get("source", {}).get("provider")
    return out


def matches(asset, query, kind, tags):
    if kind and asset["kind"] != kind:
        return False
    if tags and not set(tags) <= set(asset.get("tags", [])):
        return False
    if query:
        haystack = " ".join([asset["id"], asset["name"], *asset.get("tags", [])]).lower()
        return all(word in haystack for word in query.lower().split())
    return True


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--root", help="library folder (default: the user library)")
    parser.add_argument("--json", action="store_true", help="print JSON")
    commands = parser.add_subparsers(dest="command", required=True)

    listing = commands.add_parser("list")
    listing.add_argument("--kind", choices=KINDS)
    listing.add_argument("--query")
    listing.add_argument("--tags")

    info = commands.add_parser("info")
    info.add_argument("ids", nargs="+")

    add = commands.add_parser("ingest")
    add.add_argument("files", nargs="+")
    add.add_argument("--provider", default="local")
    add.add_argument("--license", default="")
    add.add_argument("--author", default="")
    add.add_argument("--url", default="")
    add.add_argument("--tags", default="")
    add.add_argument("--name")
    add.add_argument("--prefix")
    add.add_argument("--kind", choices=KINDS)

    pack = commands.add_parser("fetch-pack")
    pack.add_argument("pack", choices=[*PACKS, "all"])

    mixamo = commands.add_parser("fetch-mixamo")
    mixamo.add_argument("--set", default=os.path.join(HERE, "mixamo_set.json"))
    mixamo.add_argument("--only", nargs="*")
    mixamo.add_argument("--query", nargs="*", default=[])
    mixamo.add_argument("--characters", action="store_true", help="only characters")
    mixamo.add_argument("--animations", action="store_true", help="only animations")

    remove = commands.add_parser("remove")
    remove.add_argument("ids", nargs="+")

    commands.add_parser("reindex")

    args = parser.parse_args()
    root = library_root(args.root)

    def emit(value, text):
        print(json.dumps(value, indent=1) if args.json else text)

    if args.command == "list":
        tags = [t for t in (args.tags or "").split(",") if t]
        assets = [a for a in load_assets(root) if matches(a, args.query, args.kind, tags)]
        rows = [summary(a) for a in assets]
        text = "\n".join(
            f"{r['id']:44s} {r['kind']:9s} {r.get('duration', r.get('height', '')) or '':>7} "
            f"{','.join(r['tags'] or [])}"
            for r in rows
        ) + f"\n{len(rows)} assets in {root}"
        emit({"root": root, "count": len(rows), "assets": rows}, text)
    elif args.command == "info":
        by_id = {a["id"]: a for a in load_assets(root)}
        found = [by_id[i] for i in args.ids if i in by_id]
        emit(found, json.dumps(found, indent=2))
    elif args.command == "ingest":
        tags = [t for t in args.tags.split(",") if t]
        entries = []
        for path in args.files:
            entries += ingest(
                root, os.path.abspath(path), provider=args.provider,
                license_text=args.license, author=args.author, url=args.url, tags=tags,
                name=args.name, prefix=args.prefix, kind=args.kind,
            )
        write_index(root)
        emit([summary(e) for e in entries], f"added {len(entries)} assets")
    elif args.command == "fetch-pack":
        keys = list(PACKS) if args.pack == "all" else [args.pack]
        entries = []
        for key in keys:
            entries += fetch_pack(root, key)
            write_index(root)
        emit([summary(e) for e in entries], f"added {len(entries)} assets")
    elif args.command == "fetch-mixamo":
        characters = args.characters or not args.animations
        animations = args.animations or not args.characters
        entries, failures = fetch_mixamo(
            root, args.set, only=args.only, characters=characters, animations=animations,
            queries=args.query,
        )
        write_index(root)
        emit({"added": [summary(e) for e in entries], "failed": failures},
             f"added {len(entries)} assets, {len(failures)} failed"
             + "".join(f"\n  {f['name']}: {f['error']}" for f in failures))
    elif args.command == "remove":
        for asset_id in args.ids:
            folder = os.path.join(root, "assets", asset_id)
            if os.path.isdir(folder):
                shutil.rmtree(folder)
        write_index(root)
        emit({"removed": args.ids}, f"removed {len(args.ids)}")
    elif args.command == "reindex":
        assets = write_index(root)
        emit({"count": len(assets)}, f"{len(assets)} assets indexed")


if __name__ == "__main__":
    main()
