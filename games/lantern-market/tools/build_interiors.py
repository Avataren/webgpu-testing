"""Render Lantern Market's room images for interior mapping and pack them into an atlas.

Run it inside Blender (a headless one works):

    blender -b --threads 8 --factory-startup --python games/lantern-market/tools/build_interiors.py

It writes ``content/textures/interiors.png``: eight rooms of the townhouses round the
market (a kitchen, a bedroom, a workshop, a hall, a study, a storeroom, a tavern room and
a parlour), four across and two down, each 384 x 512. The window material uses it as its
room images (``interior`` with ``atlas: [4, 2]``; see interior.wgsl in the engine).

Each image is the room seen square-on through its window from outside, in one-point
perspective with the back wall filling the middle half: the camera stands one room depth
in front of the window and its view just spans the opening. The room is as wide and
high as the image is (ROOM_W x ROOM_H) and ROOM_D deep, so the material's room depth is
ROOM_D / ROOM_W. Rooms are lit as at dusk by their own lamp, candles or fire.

It also draws the windows' glazing, ``window_lead_base_color.png`` and
``window_lead_orm.png``: an oak frame with a mullion and a transom, and leaded diamond
panes. Base colour alpha marks the frame and lead (opaque) apart from the glass, through
which the rooms show.
"""

import math
import os
import random
import tempfile

import bpy
import numpy as np

HERE = os.path.dirname(os.path.abspath(__file__))
PROJECT = os.path.dirname(HERE)
OUT = os.path.join(PROJECT, "content", "textures", "interiors.png")
LEAD_BASE = os.path.join(PROJECT, "content", "textures", "window_lead_base_color.png")
LEAD_ORM = os.path.join(PROJECT, "content", "textures", "window_lead_orm.png")

# The room box: x across (-W/2..W/2), y in from the window (0..D), z up from the floor.
ROOM_W = 2.25
ROOM_H = 3.0
ROOM_D = 3.3
COLUMNS, ROWS = 4, 2
CELL = (384, 512)
SAMPLES = 96

# --------------------------------------------------------------------- materials

_materials = {}


def material(name, colour, roughness=0.8, emission=0.0, texture=None):
    """A Principled material; `texture` adds a procedural pattern ("plaster", "planks",
    "wood", "stone", "cloth")."""
    key = (name, colour, roughness, emission, texture)
    if key in _materials:
        return _materials[key]
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    nodes = mat.node_tree.nodes
    links = mat.node_tree.links
    bsdf = nodes.get("Principled BSDF")
    bsdf.inputs["Roughness"].default_value = roughness
    base = (*colour, 1.0)
    if texture:
        coords = nodes.new("ShaderNodeTexCoord")
        ramp = nodes.new("ShaderNodeValToRGB")
        dark = tuple(c * 0.72 for c in colour)
        ramp.color_ramp.elements[0].color = (*dark, 1.0)
        ramp.color_ramp.elements[1].color = base
        if texture == "planks":
            tex = nodes.new("ShaderNodeTexBrick")
            tex.inputs["Scale"].default_value = 1.0
            tex.inputs["Mortar Size"].default_value = 0.008
            tex.inputs["Brick Width"].default_value = 1.6
            tex.inputs["Row Height"].default_value = 0.18
            tex.inputs["Color1"].default_value = base
            tex.inputs["Color2"].default_value = (*dark, 1.0)
            tex.inputs["Mortar"].default_value = (*(c * 0.35 for c in colour), 1.0)
            links.new(coords.outputs["Object"], tex.inputs["Vector"])
            links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
        elif texture == "stone":
            tex = nodes.new("ShaderNodeTexBrick")
            tex.inputs["Scale"].default_value = 3.0
            tex.inputs["Mortar Size"].default_value = 0.03
            tex.inputs["Color1"].default_value = base
            tex.inputs["Color2"].default_value = (*dark, 1.0)
            tex.inputs["Mortar"].default_value = (*(c * 0.5 for c in colour), 1.0)
            mapping = nodes.new("ShaderNodeMapping")
            mapping.inputs["Rotation"].default_value = (math.pi / 2, 0.0, 0.0)
            links.new(coords.outputs["Object"], mapping.inputs["Vector"])
            links.new(mapping.outputs["Vector"], tex.inputs["Vector"])
            links.new(tex.outputs["Color"], bsdf.inputs["Base Color"])
        else:
            tex = nodes.new("ShaderNodeTexNoise")
            scale = {"plaster": 3.0, "wood": 14.0, "cloth": 40.0}[texture]
            tex.inputs["Scale"].default_value = scale
            tex.inputs["Detail"].default_value = 6.0
            if texture == "wood":
                mapping = nodes.new("ShaderNodeMapping")
                mapping.inputs["Scale"].default_value = (1.0, 1.0, 12.0)
                links.new(coords.outputs["Object"], mapping.inputs["Vector"])
                links.new(mapping.outputs["Vector"], tex.inputs["Vector"])
            else:
                links.new(coords.outputs["Object"], tex.inputs["Vector"])
            ramp.color_ramp.elements[0].position = 0.35
            ramp.color_ramp.elements[1].position = 0.65
            links.new(tex.outputs["Fac"], ramp.inputs["Fac"])
            links.new(ramp.outputs["Color"], bsdf.inputs["Base Color"])
    else:
        bsdf.inputs["Base Color"].default_value = base
    if emission > 0:
        bsdf.inputs["Emission Color"].default_value = base
        bsdf.inputs["Emission Strength"].default_value = emission
    _materials[key] = mat
    return mat


# --------------------------------------------------------------------- geometry


def mesh_object(name, verts, faces, mat):
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(verts, [], faces)
    mesh.validate()
    mesh.materials.append(mat)
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.scene.collection.objects.link(obj)
    return obj


def box(mat, lo, hi, name="box"):
    (x0, y0, z0), (x1, y1, z1) = lo, hi
    v = [(x0, y0, z0), (x1, y0, z0), (x1, y1, z0), (x0, y1, z0), (x0, y0, z1), (x1, y0, z1), (x1, y1, z1), (x0, y1, z1)]
    f = [(0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)]
    return mesh_object(name, v, f, mat)


def cyl(mat, centre, radius, height, seg=20, bulge=0.0, axis="z", name="cyl"):
    """A cylinder standing on `centre` (its base), `bulge` widening its middle (barrels)."""
    rings = 5 if bulge else 2
    v = []
    for r in range(rings):
        t = r / (rings - 1)
        rad = radius * (1.0 + bulge * math.sin(math.pi * t))
        for s in range(seg):
            a = 2 * math.pi * s / seg
            p = (rad * math.cos(a), rad * math.sin(a), height * t)
            if axis == "x":
                p = (p[2], p[0], p[1])
            elif axis == "y":
                p = (p[0], p[2], p[1])
            v.append((centre[0] + p[0], centre[1] + p[1], centre[2] + p[2]))
    f = []
    for r in range(rings - 1):
        for s in range(seg):
            a, b = r * seg + s, r * seg + (s + 1) % seg
            f.append((a, b, b + seg, a + seg))
    f.append(tuple(reversed(range(seg))))
    f.append(tuple(range((rings - 1) * seg, rings * seg)))
    return mesh_object(name, v, f, mat)


def light(kind, location, colour, power, radius=0.05):
    data = bpy.data.lights.new("light", kind)
    data.color = colour
    data.energy = power
    data.shadow_soft_size = radius
    obj = bpy.data.objects.new("light", data)
    obj.location = location
    bpy.context.scene.collection.objects.link(obj)
    return obj


# Palettes (linear).
PLASTERS = [(0.62, 0.55, 0.42), (0.58, 0.47, 0.36), (0.50, 0.50, 0.44), (0.66, 0.60, 0.50), (0.52, 0.42, 0.34)]
TIMBER = (0.12, 0.07, 0.04)
OAK = (0.32, 0.19, 0.09)
PINE = (0.45, 0.30, 0.15)
LAMP = (1.0, 0.80, 0.58)
FLAME = (1.0, 0.55, 0.20)

W2 = ROOM_W / 2


class Door:
    """A doorway in the wall `wall` ("back", "left" or "right") from `a` to `b` along it
    (x on the back wall, y on the side walls), hinged at `a`, its leaf swung `open`
    degrees into the room; open doors show a passage beyond, with a stair going up when
    `stair`."""

    HEIGHT = 2.05

    def __init__(self, wall, a, b, open=0.0, stair=False):
        self.wall, self.a, self.b, self.open, self.stair = wall, a, b, open, stair


def on_wall(wall, along, d, z):
    """A point `d` out through `wall` (negative: into the room) at `along` and height z."""
    if wall == "back":
        return (along, ROOM_D + d, z)
    if wall == "left":
        return (-W2 - d, along, z)
    return (W2 + d, along, z)


def wall_box(mat, wall, along0, along1, d0, d1, z0, z1, name="wall"):
    p0 = on_wall(wall, along0, d0, z0)
    p1 = on_wall(wall, along1, d1, z1)
    lo = tuple(min(u, v) for u, v in zip(p0, p1))
    hi = tuple(max(u, v) for u, v in zip(p0, p1))
    return box(mat, lo, hi, name)


def wall_run(mat, wall, doors, d0, d1, z0, z1, name="wall"):
    """A wall's slab from d0 to d1 and z0 to z1, round the doorways in it."""
    start, end = (-W2, W2) if wall == "back" else (-0.3, ROOM_D + 0.1)
    cuts = sorted((min(door.a, door.b), max(door.a, door.b)) for door in doors if door.wall == wall)
    at = start
    for a, b in cuts:
        wall_box(mat, wall, at, a, d0, d1, z0, z1, name)
        if Door.HEIGHT < z1:
            wall_box(mat, wall, a, b, d0, d1, max(z0, Door.HEIGHT), z1, name)
        at = b
    wall_box(mat, wall, at, end, d0, d1, z0, z1, name)


def doorway(rng, door, plaster):
    timber = material("timber", TIMBER, 0.7, texture="wood")
    planks = material("door", (0.30, 0.18, 0.09), 0.7, texture="wood")
    iron = material("iron", (0.04, 0.04, 0.04), 0.5)
    w, a, b, h = door.wall, door.a, door.b, Door.HEIGHT
    # The frame: two posts and a lintel standing a little proud of the wall.
    wall_box(timber, w, a - 0.1, a, -0.04, 0.1, 0.0, h + 0.1, "jamb")
    wall_box(timber, w, b, b + 0.1, -0.04, 0.1, 0.0, h + 0.1, "jamb")
    wall_box(timber, w, a - 0.1, b + 0.1, -0.04, 0.1, h, h + 0.14, "lintel")
    # The leaf: boards with two battens, iron straps and a ring, hinged at `a`.
    angle = math.radians(door.open)
    width = b - a
    sign = 1.0 if b > a else -1.0

    def leaf_point(u, v, z):
        along = a + sign * (u * math.cos(angle) + v * math.sin(angle))
        d = -u * math.sin(angle) + v * math.cos(angle)
        return on_wall(w, along, d + 0.02, z)

    def leaf_box(mat, u0, u1, v0, v1, z0, z1, name):
        corners = [leaf_point(u, v, z) for z in (z0, z1) for (u, v) in ((u0, v0), (u1, v0), (u1, v1), (u0, v1))]
        faces = [(0, 3, 2, 1), (4, 5, 6, 7), (0, 1, 5, 4), (1, 2, 6, 5), (2, 3, 7, 6), (3, 0, 4, 7)]
        mesh_object(name, corners, faces, mat)

    leaf_box(planks, 0.0, width, -0.05, 0.0, 0.0, h - 0.01, "leaf")
    for z in (0.4, h - 0.45):
        leaf_box(planks, 0.06, width - 0.06, -0.08, -0.05, z, z + 0.12, "batten")
        leaf_box(iron, 0.0, width * 0.7, 0.0, 0.012, z + 0.03, z + 0.08, "strap")
    leaf_box(iron, width - 0.14, width - 0.1, 0.0, 0.03, 0.95, 1.05, "ring")
    if door.open <= 0:
        return
    # The passage beyond: dim plaster, a candle somewhere along it, perhaps a stair.
    depth = 1.6
    t = 0.1
    lo, hi = min(a, b) - 0.05, max(a, b) + 0.05
    wall_box(plaster, w, lo, hi, t, t + depth, -0.1, 0.0, "passage")
    wall_box(plaster, w, lo, hi, t, t + depth, h + 0.6, h + 0.7, "passage")
    wall_box(plaster, w, lo - 0.1, lo, t, t + depth, 0.0, h + 0.6, "passage")
    wall_box(plaster, w, hi, hi + 0.1, t, t + depth, 0.0, h + 0.6, "passage")
    wall_box(plaster, w, lo, hi, t + depth, t + depth + 0.1, 0.0, h + 0.6, "passage")
    if door.stair:
        steps = material("steps", (0.30, 0.20, 0.11), 0.8, texture="wood")
        for k in range(7):
            d = t + 0.25 + k * 0.2
            wall_box(steps, w, lo, hi, d, t + depth, 0.0, 0.2 * (k + 1), "step")
    light("POINT", on_wall(w, (a + b) / 2, t + depth * 0.6, 1.7), (1.0, 0.65, 0.4), rng.uniform(4.0, 9.0), 0.05)


def shell(rng, plaster=None, floor="planks", panelled=False, doors=()):
    """Walls round their doorways, floor, ceiling with beams and the timber frame of the
    back wall."""
    wall = material("plaster", plaster or rng.choice(PLASTERS), 0.95, texture="plaster")
    timber = material("timber", TIMBER, 0.7, texture="wood")
    if floor == "planks":
        floor_mat = material("floor", PINE, 0.6, texture="planks")
    else:
        floor_mat = material("flags", (0.32, 0.29, 0.25), 0.9, texture="stone")
    t = 0.1
    box(floor_mat, (-W2 - t, -0.3, -t), (W2 + t, ROOM_D + t, 0.0), "floor")
    box(wall, (-W2 - t, -0.3, ROOM_H), (W2 + t, ROOM_D + t, ROOM_H + t), "ceiling")
    for side in ("left", "right", "back"):
        wall_run(wall, side, doors, 0.0, t, 0.0, ROOM_H, side)
    # Beams across the ceiling, posts in the back corners, a rail along the back wall.
    for k in range(3):
        y = 0.5 + k * (ROOM_D - 0.6) / 2
        box(timber, (-W2, y - 0.09, ROOM_H - 0.2), (W2, y + 0.09, ROOM_H), "beam")
    for x in (-W2, W2 - 0.16):
        box(timber, (x, ROOM_D - 0.16, 0.0), (x + 0.16, ROOM_D, ROOM_H), "post")
    box(timber, (-W2, ROOM_D - 0.06, ROOM_H - 0.45), (W2, ROOM_D, ROOM_H - 0.32), "rail")
    if panelled:
        panel = material("panel", OAK, 0.6, texture="wood")
        for side in ("left", "right", "back"):
            wall_run(panel, side, doors, -0.03, 0.0, 0.0, 1.0, "panel")
    for door in doors:
        doorway(rng, door, wall)
    return wall, timber


def table(mat, x, y, w, d, h=0.78, name="table"):
    box(mat, (x - w / 2, y - d / 2, h - 0.05), (x + w / 2, y + d / 2, h), name)
    for sx in (-1, 1):
        for sy in (-1, 1):
            lx, ly = x + sx * (w / 2 - 0.06), y + sy * (d / 2 - 0.06)
            box(mat, (lx - 0.03, ly - 0.03, 0.0), (lx + 0.03, ly + 0.03, h - 0.05), name)


def chair(mat, x, y, facing, name="chair"):
    """A chair at (x, y) whose back is on the `facing` side (+1: back toward the window)."""
    table(mat, x, y, 0.4, 0.4, 0.45, name)
    by = y - facing * 0.18
    box(mat, (x - 0.2, by - 0.02, 0.45), (x + 0.2, by + 0.02, 1.0), name)


def shelf(wood, x0, x1, y, z, depth=0.25):
    box(wood, (x0, y - depth, z), (x1, y, z + 0.04), "shelf")


def jars(rng, x0, x1, y, z, wall="back"):
    """Jars on a shelf from x0 to x1 at depth y (a side wall's shelf: along y from x0 to
    x1, at x = y)."""
    clays = [(0.45, 0.22, 0.12), (0.55, 0.45, 0.30), (0.25, 0.30, 0.22), (0.60, 0.55, 0.48)]
    n = int((x1 - x0) / 0.16)
    for k in range(n):
        if rng.random() < 0.25:
            continue
        r = rng.uniform(0.04, 0.07)
        h = rng.uniform(0.1, 0.25)
        m = material("clay", rng.choice(clays), 0.6)
        p = x0 + (k + 0.5) * (x1 - x0) / n
        centre = (p, y - 0.12, z + 0.04) if wall == "back" else (y, p, z + 0.04)
        cyl(m, centre, r, h, seg=12, bulge=0.25)


def barrel(x, y, z=0.0, r=0.26, h=0.75, lying=False):
    wood = material("barrel", OAK, 0.7, texture="wood")
    iron = material("hoop", (0.05, 0.05, 0.05), 0.5)
    if lying:
        cyl(wood, (x - h / 2, y, z + r), r, h, bulge=0.12, axis="x")
        for t in (0.12, 0.88):
            cyl(iron, (x - h / 2 + h * t - 0.02, y, z + r), r * 1.06, 0.04, axis="x")
    else:
        cyl(wood, (x, y, z), r, h, bulge=0.12)
        for t in (0.12, 0.88):
            cyl(iron, (x, y, z + h * t - 0.02), r * 1.07, 0.04)


def candle(x, y, z, power=6.0):
    wax = material("wax", (0.85, 0.80, 0.65), 0.5)
    flame = material("flame", FLAME, 0.5, emission=30.0)
    cyl(wax, (x, y, z), 0.025, 0.16, seg=10)
    cyl(flame, (x, y, z + 0.16), 0.01, 0.04, seg=8)
    light("POINT", (x, y, z + 0.22), FLAME, power, 0.02)


def hanging_lamp(x, y, power=120.0, colour=LAMP, drop=0.75):
    """A lantern hanging `drop` below the ceiling on a chain."""
    iron = material("iron", (0.04, 0.04, 0.04), 0.5)
    glow = material("lamp", colour, 0.5, emission=3.0)
    top = ROOM_H - drop
    box(iron, (x - 0.01, y - 0.01, top), (x + 0.01, y + 0.01, ROOM_H), "chain")
    box(iron, (x - 0.1, y - 0.1, top - 0.03), (x + 0.1, y + 0.1, top), "cap")
    box(iron, (x - 0.1, y - 0.1, top - 0.3), (x + 0.1, y + 0.1, top - 0.27), "base")
    for sx in (-1, 1):
        for sy in (-1, 1):
            box(iron, (x + sx * 0.09 - 0.01, y + sy * 0.09 - 0.01, top - 0.27), (x + sx * 0.09 + 0.01, y + sy * 0.09 + 0.01, top - 0.03), "bar")
    cyl(glow, (x, y, top - 0.25), 0.06, 0.18, seg=12)
    light("POINT", (x, y, top - 0.16), colour, power, 0.08)


def tapestry_side(rng, y0, y1, z0, z1):
    """A tapestry on the left wall from y0 to y1."""
    tapestry(rng, y0, y1, z0, z1, wall="left")


def tapestry(rng, x0, x1, z0, z1, y=ROOM_D - 0.01, wall="back"):
    a = rng.choice([(0.32, 0.04, 0.03), (0.05, 0.10, 0.22), (0.07, 0.16, 0.08)])
    mat = bpy.data.materials.new("tapestry")
    mat.use_nodes = True
    nodes, links = mat.node_tree.nodes, mat.node_tree.links
    # Woven bands of a deep dye and gold.
    wave = nodes.new("ShaderNodeTexWave")
    wave.inputs["Scale"].default_value = 3.0
    wave.inputs["Distortion"].default_value = 4.0
    ramp = nodes.new("ShaderNodeValToRGB")
    ramp.color_ramp.elements[0].color = (*a, 1.0)
    ramp.color_ramp.elements[1].color = (0.45, 0.32, 0.10, 1.0)
    ramp.color_ramp.elements[0].position = 0.55
    links.new(wave.outputs["Fac"], ramp.inputs["Fac"])
    links.new(ramp.outputs["Color"], nodes["Principled BSDF"].inputs["Base Color"])
    nodes["Principled BSDF"].inputs["Roughness"].default_value = 1.0
    gold = material("rod", (0.6, 0.45, 0.15), 0.4)
    if wall == "back":
        box(mat, (x0, y - 0.02, z0), (x1, y, z1), "tapestry")
        box(gold, (x0 - 0.05, y - 0.05, z1), (x1 + 0.05, y, z1 + 0.04), "rod")
    else:
        box(mat, (-W2, x0, z0), (-W2 + 0.02, x1, z1), "tapestry")
        box(gold, (-W2, x0 - 0.05, z1), (-W2 + 0.05, x1 + 0.05, z1 + 0.04), "rod")


# --------------------------------------------------------------------- the rooms


def kitchen(rng):
    shell(rng, floor="stone", doors=[Door("right", 2.75, 1.95, open=70.0)])
    stone = material("hearth", (0.35, 0.32, 0.28), 0.9, texture="stone")
    soot = material("soot", (0.03, 0.025, 0.02), 1.0)
    fire = material("fire", FLAME, 0.5, emission=40.0)
    # The hearth in the back wall, its fire lighting the room.
    box(stone, (-0.75, ROOM_D - 0.5, 0.0), (-0.55, ROOM_D, 1.2), "hearth")
    box(stone, (0.55, ROOM_D - 0.5, 0.0), (0.75, ROOM_D, 1.2), "hearth")
    box(stone, (-0.8, ROOM_D - 0.55, 1.2), (0.8, ROOM_D, 1.5), "mantel")
    box(soot, (-0.55, ROOM_D - 0.05, 0.0), (0.55, ROOM_D, 1.2), "back")
    for k in range(5):
        cyl(fire, (rng.uniform(-0.3, 0.3), ROOM_D - 0.25 + rng.uniform(-0.08, 0.08), 0.0), 0.06, rng.uniform(0.2, 0.45), seg=8)
    light("POINT", (0.0, ROOM_D - 0.3, 0.35), FLAME, 260.0, 0.25)
    iron = material("pot", (0.05, 0.05, 0.05), 0.4)
    cyl(iron, (0.0, ROOM_D - 0.3, 0.45), 0.16, 0.22, seg=16, bulge=0.15)
    # A table with bowls, shelves of jars, pots hanging from a beam.
    wood = material("table", OAK, 0.7, texture="wood")
    table(wood, 0.35, 1.3, 1.0, 0.6)
    bowl = material("bowl", (0.50, 0.35, 0.22), 0.6)
    for x in (0.1, 0.45, 0.7):
        cyl(bowl, (x, 1.3 + rng.uniform(-0.1, 0.1), 0.78), 0.08, 0.06, seg=12)
    for z in (1.1, 1.6):
        box(wood, (-W2, 0.5, z), (-W2 + 0.3, 2.3, z + 0.04), "shelf")
        jars(rng, 0.5, 2.3, -W2 + 0.15, z, wall="side")
    for k in range(4):
        y = 1.0 + 0.35 * k
        box(iron, (W2 - 0.41, y - 0.01, ROOM_H - 0.6), (W2 - 0.39, y + 0.01, ROOM_H - 0.2), "hook")
        cyl(iron, (W2 - 0.4, y, ROOM_H - 0.75), 0.1, 0.15, seg=12)
    candle(0.6, 1.25, 0.78, 4.0)


def bedroom(rng):
    shell(rng, panelled=True, doors=[Door("left", 2.7, 1.9)])
    wood = material("bed", OAK, 0.7, texture="wood")
    blanket = material("blanket", rng.choice([(0.40, 0.05, 0.04), (0.10, 0.16, 0.32)]), 1.0, texture="cloth")
    linen = material("linen", (0.75, 0.70, 0.60), 1.0, texture="cloth")
    # A bed against the right wall, its head at the back.
    box(wood, (W2 - 1.0, ROOM_D - 2.0, 0.0), (W2, ROOM_D, 0.45), "bed")
    box(wood, (W2 - 1.0, ROOM_D - 0.08, 0.0), (W2, ROOM_D, 1.1), "headboard")
    box(linen, (W2 - 0.95, ROOM_D - 1.95, 0.45), (W2 - 0.05, ROOM_D - 0.1, 0.6), "mattress")
    box(blanket, (W2 - 0.98, ROOM_D - 1.98, 0.5), (W2 - 0.02, ROOM_D - 0.7, 0.66), "blanket")
    box(linen, (W2 - 0.85, ROOM_D - 0.55, 0.6), (W2 - 0.15, ROOM_D - 0.15, 0.75), "pillow")
    # A chest at its foot, a stool with a candle, a cloak on a peg.
    chest = material("chest", PINE, 0.7, texture="wood")
    box(chest, (W2 - 0.9, ROOM_D - 2.55, 0.0), (W2 - 0.1, ROOM_D - 2.05, 0.5), "chest")
    table(wood, -W2 + 0.4, ROOM_D - 0.4, 0.35, 0.35, 0.5, "stool")
    candle(-W2 + 0.4, ROOM_D - 0.4, 0.5, 22.0)
    cloak = material("cloak", (0.18, 0.22, 0.12), 1.0, texture="cloth")
    box(cloak, (-W2 + 0.02, 0.9, 0.9), (-W2 + 0.12, 1.25, 1.9), "cloak")
    candle(W2 - 0.3, ROOM_D - 2.3, 0.5, 16.0)


def workshop(rng):
    shell(rng, floor="planks", doors=[Door("left", 1.55, 2.4, open=35.0, stair=True)])
    bench = material("bench", PINE, 0.7, texture="wood")
    # A workbench along the back wall with tools on a rack above it.
    box(bench, (-W2, ROOM_D - 0.7, 0.85), (W2, ROOM_D, 0.95), "bench")
    for x in (-W2 + 0.1, 0.0, W2 - 0.1):
        box(bench, (x - 0.05, ROOM_D - 0.65, 0.0), (x + 0.05, ROOM_D - 0.55, 0.85), "leg")
    box(bench, (-W2 + 0.2, ROOM_D - 0.04, 1.3), (W2 - 0.2, ROOM_D, 1.9), "rack")
    iron = material("tool", (0.08, 0.08, 0.09), 0.4)
    for k in range(9):
        x = -W2 + 0.35 + k * 0.19
        h = rng.uniform(0.25, 0.45)
        box(iron, (x - 0.015, ROOM_D - 0.08, 1.75 - h), (x + 0.015, ROOM_D - 0.04, 1.75), "tool")
        box(iron, (x - 0.05, ROOM_D - 0.08, 1.75 - h - 0.05), (x + 0.05, ROOM_D - 0.04, 1.75 - h), "head")
    # Leather and shoes on the bench, lasts, a barrel and boards against the wall.
    leather = material("leather", (0.30, 0.15, 0.07), 0.8)
    for k in range(4):
        x = -0.8 + k * 0.45
        box(leather, (x, ROOM_D - 0.55, 0.95), (x + 0.25, ROOM_D - 0.3, 0.98 + rng.uniform(0.0, 0.1)), "work")
    barrel(-W2 + 0.35, 0.9)
    for k in range(4):
        x = W2 - 0.08 - k * 0.05
        box(bench, (x - 0.025, 0.6 + k * 0.1, 0.0), (x + 0.025, 0.85 + k * 0.1, 2.0 - k * 0.1), "board")
    chair(bench, 0.2, ROOM_D - 1.1, 1)
    candle(-0.4, ROOM_D - 0.3, 0.95, 8.0)
    hanging_lamp(-0.3, ROOM_D - 0.9, 100.0, drop=1.1)


def hall(rng):
    shell(rng, plaster=(0.60, 0.52, 0.40), floor="stone", doors=[Door("back", -0.45, 0.45, open=80.0, stair=True)])
    oak = material("oak", OAK, 0.6, texture="wood")
    # A long table down the room with benches, a tapestry on the back wall.
    table(oak, 0.0, 1.8, 0.8, 2.2)
    for x in (-0.65, 0.65):
        box(oak, (x - 0.15, 0.8, 0.42), (x + 0.15, 2.8, 0.47), "bench")
        for y in (0.9, 2.7):
            box(oak, (x - 0.1, y - 0.03, 0.0), (x + 0.1, y + 0.03, 0.42), "bench")
    tapestry_side(rng, 1.0, 2.6, 1.1, 2.4)
    pewter = material("pewter", (0.45, 0.45, 0.45), 0.35)
    for k in range(5):
        cyl(pewter, (rng.uniform(-0.25, 0.25), 0.9 + k * 0.4, 0.78), 0.05, 0.12, seg=12)
    for y in (1.3, 2.3):
        candle(0.0, y, 0.78, 12.0)
    hanging_lamp(0.0, 1.1, 70.0)
    hanging_lamp(0.0, 2.5, 70.0)


def study(rng):
    shell(rng, panelled=True, doors=[Door("back", -0.95, -0.27)])
    oak = material("desk", OAK, 0.6, texture="wood")
    # A bookcase on the left wall, a desk with a candle and open book at the back.
    box(oak, (-W2, 0.6, 0.0), (-W2 + 0.35, 2.4, 2.3), "case")
    covers = [(0.35, 0.06, 0.04), (0.10, 0.20, 0.10), (0.30, 0.20, 0.08), (0.08, 0.10, 0.25), (0.45, 0.38, 0.25)]
    for z in (0.15, 0.7, 1.25, 1.8):
        y = 0.65
        while y < 2.35:
            thick = rng.uniform(0.04, 0.09)
            h = rng.uniform(0.3, 0.45)
            if rng.random() < 0.85:
                box(material("book", rng.choice(covers), 0.7), (-W2 + 0.37, y, z), (-W2 + 0.4, y + thick, z + h), "spine")
            y += thick + 0.005
        box(oak, (-W2 + 0.35, 0.6, z - 0.04), (-W2 + 0.4, 2.4, z), "board")
    table(oak, 0.3, ROOM_D - 0.55, 1.1, 0.6)
    chair(oak, 0.3, ROOM_D - 1.15, 1)
    page = material("page", (0.80, 0.75, 0.60), 0.8)
    box(page, (0.1, ROOM_D - 0.65, 0.78), (0.5, ROOM_D - 0.4, 0.8), "book")
    candle(0.75, ROOM_D - 0.5, 0.78, 28.0)
    chest = material("chest", PINE, 0.7, texture="wood")
    box(chest, (W2 - 0.55, 0.9, 0.0), (W2 - 0.05, 1.7, 0.55), "chest")
    candle(W2 - 0.3, 1.1, 0.55, 14.0)


def storeroom(rng):
    shell(rng, plaster=(0.48, 0.42, 0.34), floor="stone", doors=[Door("right", 1.1, 1.95)])
    crate = material("crate", PINE, 0.8, texture="wood")
    sack = material("sack", (0.50, 0.42, 0.28), 1.0, texture="cloth")
    # Barrels, crates and sacks stacked to the back, herbs hanging from the beams.
    for k, x in enumerate((-0.8, -0.25, 0.3)):
        barrel(x, ROOM_D - 0.35)
        if k != 1:
            barrel(x, ROOM_D - 0.35, 0.75)
    for x, y, z, s in ((0.8, ROOM_D - 0.4, 0.0, 0.5), (0.8, ROOM_D - 0.4, 0.5, 0.45), (0.75, ROOM_D - 1.0, 0.0, 0.5)):
        box(crate, (x - s / 2, y - s / 2, z), (x + s / 2, y + s / 2, z + s), "crate")
    for k in range(5):
        x = -W2 + 0.3 + rng.uniform(-0.05, 0.05)
        y = 0.8 + k * 0.4
        cyl(sack, (x, y, 0.0), 0.2, 0.5 + rng.uniform(0.0, 0.2), seg=12, bulge=0.3)
    herb = material("herbs", (0.16, 0.22, 0.08), 1.0)
    for k in range(6):
        y = 0.8 + k * 0.35
        x = rng.uniform(-0.5, 0.5)
        box(herb, (x - 0.06, y - 0.06, ROOM_H - 0.75), (x + 0.06, y + 0.06, ROOM_H - 0.45), "herbs")
        box(material("string", (0.3, 0.25, 0.15), 1.0), (x - 0.005, y - 0.005, ROOM_H - 0.45), (x + 0.005, y + 0.005, ROOM_H), "string")
    barrel(0.0, 1.0, lying=True)
    hanging_lamp(0.35, 1.8, 100.0, (1.0, 0.7, 0.45), drop=0.9)


def tavern(rng):
    shell(rng, plaster=(0.55, 0.45, 0.32), floor="planks", doors=[Door("right", 2.5, 1.75, open=60.0)])
    oak = material("oak", OAK, 0.6, texture="wood")
    # Casks on a rack along the back wall, a table with mugs, stools.
    box(oak, (-W2, ROOM_D - 0.7, 0.0), (W2, ROOM_D - 0.6, 0.5), "rack")
    for x in (-0.7, 0.0, 0.7):
        barrel(x, ROOM_D - 0.35, 0.5, r=0.3, h=0.7, lying=True)
    shelf(oak, -W2 + 0.2, W2 - 0.2, ROOM_D, 1.9)
    jars(rng, -W2 + 0.2, W2 - 0.2, ROOM_D, 1.9)
    table(oak, -0.2, 1.5, 0.9, 0.9)
    mug = material("mug", (0.40, 0.28, 0.16), 0.6)
    for k in range(3):
        cyl(mug, (-0.4 + 0.25 * k, 1.4 + rng.uniform(-0.15, 0.15), 0.78), 0.05, 0.13, seg=12)
    for x, y in ((-0.85, 1.5), (0.45, 1.5), (-0.2, 2.15)):
        table(oak, x, y, 0.35, 0.35, 0.48, "stool")
    candle(-0.1, 1.55, 0.78, 10.0)
    hanging_lamp(0.5, 1.0, 110.0)


def parlour(rng):
    shell(rng, plaster=rng.choice(PLASTERS), floor="planks", doors=[Door("left", 0.75, 1.6, open=165.0, stair=True)])
    oak = material("oak", OAK, 0.6, texture="wood")
    rug = material("rug", (0.35, 0.10, 0.06), 1.0, texture="cloth")
    # A rug, an armchair by a cupboard, a painting over a side table.
    box(rug, (-0.8, 0.9, 0.0), (0.8, 2.6, 0.015), "rug")
    cushion = material("cushion", (0.12, 0.20, 0.30), 1.0, texture="cloth")
    box(oak, (-0.85, 1.9, 0.0), (-0.25, 2.5, 0.42), "chair")
    box(cushion, (-0.82, 1.93, 0.42), (-0.28, 2.47, 0.52), "seat")
    box(oak, (-0.85, 2.42, 0.42), (-0.25, 2.5, 1.2), "back")
    box(oak, (W2 - 0.5, ROOM_D - 1.1, 0.0), (W2, ROOM_D - 0.1, 1.9), "cupboard")
    door = material("door", PINE, 0.6, texture="wood")
    box(door, (W2 - 0.52, ROOM_D - 1.05, 0.1), (W2 - 0.5, ROOM_D - 0.62, 1.8), "door")
    box(door, (W2 - 0.52, ROOM_D - 0.58, 0.1), (W2 - 0.5, ROOM_D - 0.15, 1.8), "door")
    table(oak, -0.2, ROOM_D - 0.3, 0.6, 0.35, 0.7, "side")
    frame = material("frame", (0.5, 0.35, 0.1), 0.4)
    art = material("painting", rng.choice([(0.25, 0.30, 0.18), (0.30, 0.22, 0.12)]), 0.8, texture="plaster")
    box(frame, (-0.6, ROOM_D - 0.03, 1.3), (0.2, ROOM_D, 2.0), "frame")
    box(art, (-0.54, ROOM_D - 0.04, 1.36), (0.14, ROOM_D - 0.03, 1.94), "painting")
    candle(-0.35, ROOM_D - 0.3, 0.7, 10.0)
    candle(-0.05, ROOM_D - 0.3, 0.7, 10.0)
    hanging_lamp(-0.4, 1.4, 60.0, drop=0.9)


ROOMS = [kitchen, bedroom, workshop, hall, study, storeroom, tavern, parlour]

# --------------------------------------------------------------------- render


def setup_scene():
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.device = "CPU"
    scene.cycles.samples = SAMPLES
    scene.cycles.use_denoising = True
    scene.render.threads_mode = "FIXED"
    scene.render.threads = 8
    scene.render.resolution_x, scene.render.resolution_y = CELL
    scene.render.resolution_percentage = 100
    scene.render.image_settings.file_format = "PNG"
    scene.render.image_settings.color_mode = "RGB"
    scene.view_settings.view_transform = "Standard"
    scene.view_settings.look = "None"
    scene.view_settings.exposure = -0.4
    world = bpy.data.worlds.new("dusk")
    world.use_nodes = True
    world.node_tree.nodes["Background"].inputs["Color"].default_value = (0.006, 0.007, 0.012, 1.0)
    scene.world = world
    # The camera one room depth in front of the window, its view just spanning it.
    cam = bpy.data.cameras.new("camera")
    cam.sensor_fit = "VERTICAL"
    cam.angle_y = 2.0 * math.atan((ROOM_H / 2) / ROOM_D)
    cam.clip_start = 0.05
    obj = bpy.data.objects.new("camera", cam)
    obj.location = (0.0, -ROOM_D, ROOM_H / 2)
    obj.rotation_euler = (math.pi / 2, 0.0, 0.0)
    scene.collection.objects.link(obj)
    scene.camera = obj
    return scene


def render_room(index, build, directory):
    bpy.ops.wm.read_homefile(use_empty=True, use_factory_startup=True)
    _materials.clear()
    scene = setup_scene()
    build(random.Random(1000 + index))
    path = os.path.join(directory, f"room_{index}.png")
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)
    return path


def pack(paths):
    width, height = CELL
    atlas = np.zeros((ROWS * height, COLUMNS * width, 4), dtype=np.float32)
    for index, path in enumerate(paths):
        image = bpy.data.images.load(path)
        image.colorspace_settings.name = "Non-Color"
        pixels = np.array(image.pixels[:], dtype=np.float32).reshape(height, width, 4)
        column, row = index % COLUMNS, index // COLUMNS
        # Blender's pixels run bottom to top; the atlas's first row is its top.
        top = (ROWS - 1 - row) * height
        atlas[top : top + height, column * width : (column + 1) * width] = pixels
        bpy.data.images.remove(image)
    atlas[..., 3] = 1.0
    out = bpy.data.images.new("interiors", COLUMNS * width, ROWS * height, alpha=False)
    out.colorspace_settings.name = "Non-Color"
    out.pixels[:] = atlas.ravel()
    out.filepath_raw = OUT
    out.file_format = "PNG"
    out.save()
    return OUT


def save_png(path, rgba):
    """Write an (h, w, 4) array of 0-1 values, first row on top, as stored (no colour
    conversion)."""
    height, width = rgba.shape[:2]
    image = bpy.data.images.new(os.path.basename(path), width, height, alpha=True)
    image.colorspace_settings.name = "Non-Color"
    image.pixels[:] = np.ascontiguousarray(rgba[::-1]).ravel()
    image.filepath_raw = path
    image.file_format = "PNG"
    image.save()
    bpy.data.images.remove(image)


def glazing(width=192, height=256, supersample=4):
    """The window's frame and leaded glass, as base colour (sRGB, alpha: frame) and ORM."""
    w, h = width * supersample, height * supersample
    y, x = np.mgrid[0:h, 0:w].astype(np.float32)
    u = (x + 0.5) / w
    v = (y + 0.5) / h
    aspect = height / width
    border = 0.07
    frame = (u < border) | (u > 1 - border) | (v < border / aspect) | (v > 1 - border / aspect)
    frame |= np.abs(u - 0.5) < 0.025
    frame |= np.abs(v - 0.36) < 0.025 / aspect
    # Diamond quarries: lead cames at 45 degrees, about six across a pane.
    spacing = 0.16
    a = (u + v * aspect) / spacing
    b = (u - v * aspect) / spacing
    lead = (np.abs(a - np.round(a)) < 0.045) | (np.abs(b - np.round(b)) < 0.045)
    lead &= ~frame
    oak = np.array([0.28, 0.18, 0.10])
    came = np.array([0.20, 0.20, 0.21])
    base = np.zeros((h, w, 4), dtype=np.float32)
    base[..., :3] = oak
    base[lead, :3] = came
    base[..., 3] = (frame | lead).astype(np.float32)
    # Occlusion, roughness, metallic: smooth glass, rough oak, dull lead.
    orm = np.ones((h, w, 4), dtype=np.float32)
    orm[..., 1] = 0.06
    orm[frame, 1] = 0.75
    orm[lead, 1] = 0.45
    orm[..., 2] = 0.0
    orm[lead, 2] = 0.6

    def down(image):
        return image.reshape(height, supersample, width, supersample, 4).mean(axis=(1, 3))

    save_png(LEAD_BASE, down(base))
    save_png(LEAD_ORM, down(orm))
    return [LEAD_BASE, LEAD_ORM]


def build():
    glass = glazing()
    with tempfile.TemporaryDirectory() as directory:
        paths = [render_room(i, room, directory) for i, room in enumerate(ROOMS)]
        atlas = pack(paths)
    return {"atlas": atlas, "glazing": glass, "rooms": [r.__name__ for r in ROOMS], "room_depth": ROOM_D / ROOM_W}


if __name__ == "__main__" or "result" not in globals():
    result = build()
    print(result)
