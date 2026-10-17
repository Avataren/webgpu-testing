#!/usr/bin/env python3
"""Copy Lantern Market's level meshes from content/models/market.glb into the project.

The scene keeps its level meshes embedded (``mesh_data``) and the market package keeps
its own copies (content/gltf/market/meshes/mesh_*.json), so a rebuilt market.glb
(tools/build_level.py) does not reach the game by itself. This converts every glTF
primitive the scene and the package use exactly as the engine's importer does
(src/scene/loader/nodes.rs: attributes as they are, tangents generated from the uvs in
f32), and writes the ones that changed, with their entities' mesh bounds. Unchanged
meshes come out byte for byte, so the diff shows only what the level edit changed.

    games/lantern-market/tools/sync_level_meshes.py           write the changed meshes
    games/lantern-market/tools/sync_level_meshes.py --check   only report them

Run it with the editor closed (or reopen the project afterwards), then re-bake the light
probes and the navmesh in the editor if the change affects them. It needs numpy.
"""

import argparse
import json
import os
import re
import struct
import sys
from decimal import Decimal

import numpy as np

PROJECT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
GLB = os.path.join(PROJECT, "content", "models", "market.glb")
SCENE = os.path.join(PROJECT, "content", "scenes", "9FKKrEIPvZz8.json")
PACKAGE = os.path.join(PROJECT, "content", "gltf", "market")
SOURCE = "content/gltf/market/market.glb.import"

COMPONENTS = {5126: np.float32, 5125: np.uint32, 5123: np.uint16, 5121: np.uint8}
WIDTHS = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4}

# --------------------------------------------------------------------- glTF


def load_glb(path):
    data = open(path, "rb").read()
    json_length, _ = struct.unpack_from("<II", data, 12)
    gltf = json.loads(data[20 : 20 + json_length])
    offset = 20 + json_length
    bin_length, _ = struct.unpack_from("<II", data, offset)
    return gltf, data[offset + 8 : offset + 8 + bin_length]


def accessor(gltf, binary, index):
    a = gltf["accessors"][index]
    view = gltf["bufferViews"][a["bufferView"]]
    width = WIDTHS[a["type"]]
    start = view.get("byteOffset", 0) + a.get("byteOffset", 0)
    values = np.frombuffer(binary, COMPONENTS[a["componentType"]], a["count"] * width, start)
    return values.reshape(a["count"], width) if width > 1 else values


def primitive(gltf, binary, node, index):
    """Positions, normals, uvs and indices of the node's mesh's primitive `index`."""
    prim = gltf["meshes"][gltf["nodes"][node]["mesh"]]["primitives"][index]
    attributes = prim["attributes"]
    unsupported = {"TANGENT", "COLOR_0", "JOINTS_0"} & attributes.keys()
    if unsupported or "NORMAL" not in attributes or "TEXCOORD_0" not in attributes:
        sys.exit(f"node {node} primitive {index}: attributes {sorted(attributes)} are not what this tool converts")
    positions = accessor(gltf, binary, attributes["POSITION"])
    indices = accessor(gltf, binary, prim["indices"]) if "indices" in prim else np.arange(len(positions))
    return positions, accessor(gltf, binary, attributes["NORMAL"]), accessor(gltf, binary, attributes["TEXCOORD_0"]), indices


# --------------------------------------------------------------------- the importer


def dot(a, b):
    # glam's Vec3::dot, in f32 and in its order.
    return np.float32(np.float32(np.float32(a[0] * b[0]) + np.float32(a[1] * b[1])) + np.float32(a[2] * b[2]))


def tangents(positions, normals, uvs, indices):
    """generate_tangents in src/scene/loader/nodes.rs, rounding as it does."""
    sums = np.zeros_like(positions)
    bitangents = np.zeros_like(positions)
    for i0, i1, i2 in indices.reshape(-1, 3).astype(int):
        edge1, edge2 = positions[i1] - positions[i0], positions[i2] - positions[i0]
        duv1, duv2 = uvs[i1] - uvs[i0], uvs[i2] - uvs[i0]
        with np.errstate(divide="ignore"):
            f = np.float32(1.0) / np.float32(duv1[0] * duv2[1] - duv2[0] * duv1[1])
        if np.isfinite(f):
            tangent = f * (duv2[1] * edge1 - duv1[1] * edge2)
            bitangent = f * (-duv2[0] * edge1 + duv1[0] * edge2)
        else:
            tangent, bitangent = np.float32([1, 0, 0]), np.float32([0, 1, 0])
        for i in (i0, i1, i2):
            sums[i] += tangent
            bitangents[i] += bitangent
    out = []
    for t, b, n in zip(sums, bitangents, normals):
        t = t - n * dot(n, t)
        with np.errstate(divide="ignore"):
            rcp = np.float32(1.0) / np.sqrt(dot(t, t))
        t = t * rcp if np.isfinite(rcp) and rcp > 0 else np.zeros(3, np.float32)
        if dot(t, t) < 1e-4:
            axis = np.float32([0, 1, 0]) if abs(n[1]) < 0.999 else np.float32([1, 0, 0])
            t = np.cross(axis, n)
            t = t * (np.float32(1.0) / np.sqrt(dot(t, t)))
        out.append((t[0], t[1], t[2], -1.0 if dot(np.cross(n, t), b) < 0 else 1.0))
    return out


def f32(value):
    """A float that prints as the shortest digits of the f32 `value`, as serde writes it."""
    return float(np.format_float_positional(np.float32(value), unique=True, trim="-"))


def mesh_data(positions, normals, uvs, indices):
    vertices = [
        {"pos": [f32(c) for c in p], "normal": [f32(c) for c in n], "uv": [f32(c) for c in uv], "tangent": [f32(c) for c in t]}
        for p, n, uv, t in zip(positions, normals, uvs, tangents(positions, normals, uvs, indices))
    ]
    return {"vertices": vertices, "indices": [int(i) for i in indices]}


def bounds(mesh):
    points = [v["pos"] for v in mesh["vertices"]]
    return {"min": [min(p[i] for p in points) for i in range(3)], "max": [max(p[i] for p in points) for i in range(3)]}


# --------------------------------------------------------------------- serde_json's layout

SCIENTIFIC = re.compile(r"(?<![\w.])(-?\d+(?:\.\d+)?)e([-+]?\d+)")


def positional(match):
    # Python prints 1e-05 where serde (ryu) writes 0.00001: ryu writes numbers down to 1e-6
    # positionally, and smaller ones with an unpadded exponent.
    mantissa, exponent = match.group(1), int(match.group(2))
    if -6 < exponent + 1 <= 0:
        return format(Decimal(match.group(0)), "f")
    return f"{mantissa}e{exponent}"


def dumps(value):
    return SCIENTIFIC.sub(positional, json.dumps(value, indent=2, ensure_ascii=False))


def load_json(path):
    """A JSON file, checked to come back out byte for byte before anything is changed."""
    text = open(path).read()
    value = json.loads(text)
    if dumps(value) != text:
        sys.exit(f"{os.path.relpath(path, PROJECT)} is not laid out as serde_json writes it; save it from the editor first")
    return value


# --------------------------------------------------------------------- sync


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("--check", action="store_true", help="only report the meshes that differ")
    args = parser.parse_args()

    gltf, binary = load_glb(GLB)
    converted = {}

    def convert(entity):
        key = (entity["gltf_node"], entity["gltf_primitive"])
        if key not in converted:
            converted[key] = mesh_data(*primitive(gltf, binary, *key))
        return converted[key]

    scene = load_json(SCENE)
    package_path = os.path.join(PACKAGE, "scene.asset.json")
    package = load_json(package_path)
    writes = {}

    # The scene's embedded meshes (shared handles are converted once) and its entities' bounds.
    changed = set()
    for entity in scene["entities"]:
        if entity.get("mesh_handle") is None or entity.get("gltf_source") != SOURCE:
            continue
        mesh = convert(entity)
        handle = entity["mesh_handle"]
        if scene["mesh_data"][handle] != mesh:
            scene["mesh_data"][handle] = mesh
            changed.add(entity["name"])
        if entity["mesh_bounds"] != bounds(mesh):
            entity["mesh_bounds"] = bounds(mesh)
            changed.add(entity["name"])
    if changed:
        writes[SCENE] = (scene, sorted(changed))

    # The package's mesh files and its entities' bounds.
    changed = set()
    for entity in package["entities"]:
        if entity.get("mesh_handle") is None:
            continue
        mesh = convert(entity)
        path = os.path.join(PACKAGE, "meshes", f"mesh_{entity['mesh_handle']:04d}.json")
        # Mesh files are written whole, so only their values need to match.
        if json.load(open(path)) != mesh:
            writes[path] = (mesh, [entity["name"]])
        if entity["mesh_bounds"] != bounds(mesh):
            entity["mesh_bounds"] = bounds(mesh)
            changed.add(entity["name"])
    if changed:
        writes[package_path] = (package, sorted(changed))

    for path, (value, names) in sorted(writes.items()):
        print(f"{os.path.relpath(path, PROJECT)}: {', '.join(names)}")
        if not args.check:
            with open(path, "w") as f:
                f.write(dumps(value))
    if not writes:
        print("every mesh matches market.glb")
    elif args.check:
        sys.exit(1)


if __name__ == "__main__":
    main()
