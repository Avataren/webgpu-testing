"""Build Lantern Market's level in Blender and export it as a GLB the engine imports.

Run it inside Blender (a headless one works):

    blender -b --python games/lantern-market/tools/build_level.py

or through the Blender MCP: ``runpy.run_path("<this file>")``. It writes
``content/models/market.glb``.

Everything is built from code in the engine's frame (x east, y up, z south), so the
numbers here match the scene and the director script: the plaza with its fountain,
stalls and stage, the arcade under the north terrace (walkable on both floors), the
canal whose towpaths pass under two bridges, the houses around, and the movable
pieces (two handcarts and the bridge's portcullis) as separate objects.

Static geometry is joined into one object per material; materials are plain colours
here and get their textures in the engine. UVs are box-mapped in world space, so the
tileable textures run on across pieces, except on the window panes: each window is one
uv cell of its own (window k spans u k..k+1, v 0..1 from the top), the room the
engine's interior mapping shows behind it.
"""

import math
import os

import bpy

HERE = os.path.dirname(os.path.abspath(__file__))
PROJECT = os.path.dirname(HERE)
OUT = os.path.join(PROJECT, "content", "models", "market.glb")

# Base colour (linear), roughness, metres per texture tile, emission strength.
MATERIALS = {
    "Cobble": ((0.30, 0.27, 0.24), 0.85, 2.5, 0.0),
    "Stone": ((0.52, 0.47, 0.40), 0.80, 2.0, 0.0),
    "Plaster": ((0.70, 0.60, 0.46), 0.90, 3.0, 0.0),
    "PlasterRose": ((0.66, 0.43, 0.36), 0.90, 3.0, 0.0),
    "PlasterBlue": ((0.40, 0.48, 0.55), 0.90, 3.0, 0.0),
    "Timber": ((0.20, 0.12, 0.07), 0.75, 1.5, 0.0),
    "Wood": ((0.42, 0.27, 0.15), 0.70, 1.5, 0.0),
    "Roof": ((0.38, 0.16, 0.10), 0.75, 2.0, 0.0),
    "ClothRed": ((0.55, 0.08, 0.06), 0.85, 2.0, 0.0),
    "ClothGold": ((0.75, 0.50, 0.12), 0.85, 2.0, 0.0),
    "ClothTeal": ((0.06, 0.32, 0.33), 0.85, 2.0, 0.0),
    "Iron": ((0.06, 0.06, 0.07), 0.45, 1.0, 0.0),
    "Water": ((0.03, 0.10, 0.12), 0.05, 4.0, 0.0),
    "LanternGlow": ((1.0, 0.62, 0.25), 0.5, 1.0, 6.0),
    "WindowGlow": ((1.0, 0.70, 0.36), 0.5, 1.0, 2.2),
    "Foliage": ((0.10, 0.20, 0.06), 0.9, 2.0, 0.0),
    "Bark": ((0.16, 0.11, 0.07), 0.9, 1.5, 0.0),
    "Fruit": ((0.65, 0.20, 0.05), 0.6, 1.0, 0.0),
}

# --------------------------------------------------------------------- geometry


def sub(a, b):
    return (a[0] - b[0], a[1] - b[1], a[2] - b[2])


def add(a, b):
    return (a[0] + b[0], a[1] + b[1], a[2] + b[2])


def dot(a, b):
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]


def newell(points):
    nx = ny = nz = 0.0
    n = len(points)
    for i in range(n):
        x0, y0, z0 = points[i]
        x1, y1, z1 = points[(i + 1) % n]
        nx += (y0 - y1) * (z0 + z1)
        ny += (z0 - z1) * (x0 + x1)
        nz += (x0 - x1) * (y0 + y1)
    return (nx, ny, nz)


def centroid(points):
    n = len(points)
    return (sum(p[0] for p in points) / n, sum(p[1] for p in points) / n, sum(p[2] for p in points) / n)


class Part:
    """Faces of one Blender object, in engine coordinates."""

    def __init__(self, name):
        self.name = name
        self.verts = []
        self.faces = []  # (vertex indices, material, smooth, engine uvs or None)

    def add_verts(self, points):
        base = len(self.verts)
        self.verts.extend(points)
        return list(range(base, base + len(points)))

    def face_idx(self, idx, mat, smooth=False, ref=None, out=None):
        points = [self.verts[i] for i in idx]
        n = newell(points)
        flip = False
        if out is not None:
            flip = dot(n, out) < 0
        elif ref is not None:
            flip = dot(n, sub(centroid(points), ref)) < 0
        if flip:
            idx = list(reversed(idx))
        self.faces.append((idx, mat, smooth, None))

    def face(self, points, mat, ref=None, out=None, smooth=False):
        self.face_idx(self.add_verts(points), mat, smooth, ref, out)


def box(part, mat, lo, hi, top=None, bottom=None, open_top=False):
    """An axis-aligned box from corner lo to hi; `top` gives its top face another material,
    `open_top` leaves it out (nothing walks on it then)."""
    x0, y0, z0 = lo
    x1, y1, z1 = hi
    c = ((x0 + x1) / 2, (y0 + y1) / 2, (z0 + z1) / 2)
    p = [(x0, y0, z0), (x1, y0, z0), (x1, y0, z1), (x0, y0, z1), (x0, y1, z0), (x1, y1, z0), (x1, y1, z1), (x0, y1, z1)]
    faces = [
        ([4, 5, 6, 7], top or mat),
        ([0, 1, 2, 3], bottom or mat),
        ([0, 1, 5, 4], mat),
        ([3, 2, 6, 7], mat),
        ([0, 3, 7, 4], mat),
        ([1, 2, 6, 5], mat),
    ]
    for ids, m in faces[1:] if open_top else faces:
        part.face([p[i] for i in ids], m, ref=c)
    if part is S and mat not in ("Timber", "Iron"):
        SOLIDS.append((lo, hi, lambda q: all(lo[i] < q[i] < hi[i] for i in range(3))))


WINDOWS = [0]
# Windows are as wide and high as their rooms are drawn (build_interiors.py's ROOM_W x
# ROOM_H), so the rooms behind them keep their proportions.
WINDOW_W, WINDOW_H = 1.2, 1.6
# Masonry built so far (its bounds and a test for a point inside), which windows keep out of.
SOLIDS = []


def covered(lo, hi, out):
    """Whether masonry built so far covers any of a window's pane: one behind the terrace,
    a stair or a neighbour's wall would not be seen, or only in part."""
    axis = next(a for a in range(3) if out[a])
    a, b = [i for i in range(3) if i != axis]
    face = (hi if out[axis] > 0 else lo)[axis] + 0.03 * out[axis]
    near = [inside for s_lo, s_hi, inside in SOLIDS if s_lo[axis] < face < s_hi[axis] and s_lo[a] < hi[a] and s_hi[a] > lo[a] and s_lo[b] < hi[b] and s_hi[b] > lo[b]]
    for i in range(9):
        for j in range(9):
            q = [0.0, 0.0, 0.0]
            q[axis] = face
            q[a] = lo[a] + (hi[a] - lo[a]) * i / 8
            q[b] = lo[b] + (hi[b] - lo[b]) * j / 8
            if any(inside(q) for inside in near):
                return True
    return False


def window(part, lo, hi, out):
    """A window: a box from lo to hi standing out of a wall toward `out` (a unit axis
    vector), its pane facing out in WindowGlow with a uv cell of its own, its four sides
    timber. The side against the wall is left out."""
    axis = next(a for a in range(3) if out[a])
    x0, y0, z0 = lo
    x1, y1, z1 = hi
    c = ((x0 + x1) / 2, (y0 + y1) / 2, (z0 + z1) / 2)
    p = [(x0, y0, z0), (x1, y0, z0), (x1, y0, z1), (x0, y0, z1), (x0, y1, z0), (x1, y1, z0), (x1, y1, z1), (x0, y1, z1)]
    faces = {
        (1, 1): [4, 5, 6, 7],
        (1, -1): [0, 1, 2, 3],
        (2, -1): [0, 1, 5, 4],
        (2, 1): [3, 2, 6, 7],
        (0, -1): [0, 3, 7, 4],
        (0, 1): [1, 2, 6, 5],
    }
    for (a, sign), ids in faces.items():
        if a == axis and sign != out[axis]:
            continue
        points = [p[i] for i in ids]
        if a != axis:
            part.face(points, "Timber", ref=c)
            continue
        # The pane: u across as seen from outside (right = -out x up), v down from the top.
        right = (out[2], 0.0, -out[0])
        across = [dot(q, right) for q in points]
        left, width = min(across), max(across) - min(across)
        k = WINDOWS[0]
        WINDOWS[0] += 1
        uvs = [(k + (dot(q, right) - left) / width, (y1 - q[1]) / (y1 - y0)) for q in points]
        base = len(part.verts)
        part.face(points, "WindowGlow", ref=c)
        # face() may have turned the corners round to face out; the uvs follow them.
        idx, mat, smooth, _ = part.faces[-1]
        part.faces[-1] = (idx, mat, smooth, [uvs[i - base] for i in idx])


def rot_y(deg):
    a = math.radians(deg)
    c, s = math.cos(a), math.sin(a)
    return ((c, 0, s), (0, 1, 0), (-s, 0, c))


def rot_x(deg):
    a = math.radians(deg)
    c, s = math.cos(a), math.sin(a)
    return ((1, 0, 0), (0, c, -s), (0, s, c))


def rot_z(deg):
    a = math.radians(deg)
    c, s = math.cos(a), math.sin(a)
    return ((c, -s, 0), (s, c, 0), (0, 0, 1))


def mat_mul(a, b):
    return tuple(tuple(sum(a[i][k] * b[k][j] for k in range(3)) for j in range(3)) for i in range(3))


def apply(m, v):
    return (
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    )


def obox(part, mat, center, size, rot):
    """A box of full `size` turned by matrix `rot` about its centre."""
    hx, hy, hz = size[0] / 2, size[1] / 2, size[2] / 2
    corners = []
    for sx, sy, sz in [(-1, -1, -1), (1, -1, -1), (1, -1, 1), (-1, -1, 1), (-1, 1, -1), (1, 1, -1), (1, 1, 1), (-1, 1, 1)]:
        corners.append(add(center, apply(rot, (sx * hx, sy * hy, sz * hz))))
    for ids in [[4, 5, 6, 7], [0, 1, 2, 3], [0, 1, 5, 4], [3, 2, 6, 7], [0, 3, 7, 4], [1, 2, 6, 5]]:
        part.face([corners[i] for i in ids], mat, ref=center)


def cylinder(part, mat, base, radius, height, seg=16, caps=True, cap_mat=None, smooth=True, axis=None):
    """A cylinder standing on `base` (or along `axis`, a rotation matrix, centred on base)."""
    rings = []
    for y in (0.0, height):
        ring = []
        for i in range(seg):
            a = 2 * math.pi * i / seg
            v = (math.cos(a) * radius, y, math.sin(a) * radius)
            if axis is not None:
                v = apply(axis, (v[0], v[1] - height / 2, v[2]))
            ring.append(add(base, v))
        rings.append(part.add_verts(ring))
    centre = add(base, (0, height / 2, 0)) if axis is None else base
    for i in range(seg):
        j = (i + 1) % seg
        part.face_idx([rings[0][i], rings[0][j], rings[1][j], rings[1][i]], mat, smooth, ref=centre)
    if caps:
        part.face([part.verts[i] for i in rings[0]], cap_mat or mat, ref=centre)
        part.face([part.verts[i] for i in rings[1]], cap_mat or mat, ref=centre)


def lathe(part, mat, centre, profile, seg=24, cap_top=True, smooth=True):
    """A surface of revolution about the vertical through `centre` from (radius, height)
    pairs, bottom to top, faces pointing out from the axis (into the bowl for inner walls
    when the profile turns back)."""
    rings = []
    for r, y in profile:
        ring = []
        for i in range(seg):
            a = 2 * math.pi * i / seg
            ring.append((centre[0] + math.cos(a) * r, centre[1] + y, centre[2] + math.sin(a) * r))
        rings.append(part.add_verts(ring))
    for k in range(len(profile) - 1):
        (r0, y0), (r1, y1) = profile[k], profile[k + 1]
        # Outward is away from the axis along the profile's normal (right of its direction).
        dr, dy = r1 - r0, y1 - y0
        for i in range(seg):
            j = (i + 1) % seg
            a = 2 * math.pi * (i + 0.5) / seg
            radial = (math.cos(a), 0, math.sin(a))
            out = (radial[0] * dy, -dr, radial[2] * dy)
            part.face_idx([rings[k][i], rings[k][j], rings[k + 1][j], rings[k + 1][i]], mat, smooth, out=out)
    if cap_top and profile[-1][0] > 0:
        part.face([part.verts[i] for i in rings[-1]], mat, out=(0, 1, 0))


def wedge(part, mat, x0, x1, z_low, z_high, y_low, y_high, base_y=0.0, top=None):
    """A ramp between x0..x1 rising from y_low at z_low to y_high at z_high, solid down to base_y."""
    a = (x0, base_y, z_low)
    b = (x1, base_y, z_low)
    c = (x1, base_y, z_high)
    d = (x0, base_y, z_high)
    e = (x0, y_low, z_low)
    f = (x1, y_low, z_low)
    g = (x1, y_high, z_high)
    h = (x0, y_high, z_high)
    centre = centroid([a, b, c, d, e, f, g, h])
    part.face([e, f, g, h], top or mat, ref=centre)
    part.face([a, b, c, d], mat, ref=centre)
    part.face([a, b, f, e], mat, ref=centre)
    part.face([d, c, g, h], mat, ref=centre)
    part.face([a, d, h, e], mat, ref=centre)
    part.face([b, c, g, f], mat, ref=centre)
    if part is S:
        def inside(q):
            t = (q[2] - z_low) / (z_high - z_low)
            return x0 < q[0] < x1 and 0.0 < t < 1.0 and base_y < q[1] < y_low + (y_high - y_low) * t

        SOLIDS.append(((x0, base_y, min(z_low, z_high)), (x1, max(y_low, y_high), max(z_low, z_high)), inside))


def gable_roof(part, mat, x0, x1, z0, z1, eave, ridge, along="x", overhang=0.4, thick=0.18):
    """A gable roof over a rectangle, its ridge running along x or z."""
    if along == "x":
        za, zb, zm = z0 - overhang, z1 + overhang, (z0 + z1) / 2
        xa, xb = x0 - overhang, x1 + overhang
        slopes = [
            [(xa, eave, za), (xb, eave, za), (xb, ridge, zm), (xa, ridge, zm)],
            [(xa, eave, zb), (xb, eave, zb), (xb, ridge, zm), (xa, ridge, zm)],
        ]
    else:
        xa, xb, xm = x0 - overhang, x1 + overhang, (x0 + x1) / 2
        za, zb = z0 - overhang, z1 + overhang
        slopes = [
            [(xa, eave, za), (xa, eave, zb), (xm, ridge, zb), (xm, ridge, za)],
            [(xb, eave, za), (xb, eave, zb), (xm, ridge, zb), (xm, ridge, za)],
        ]
    for quad in slopes:
        upper = quad
        lower = [(p[0], p[1] - thick, p[2]) for p in quad]
        mid = centroid(upper + lower)
        part.face(upper, mat, out=(0, 1, 0))
        part.face(lower, mat, out=(0, -1, 0))
        for i in range(4):
            j = (i + 1) % 4
            part.face([upper[i], upper[j], lower[j], lower[i]], mat, ref=mid)


def gable_wall(part, mat, x0, x1, z0, z1, base, eave, ridge, along="x"):
    """The walls of a house with a gable roof: a box to the eaves and the triangular gables."""
    box(part, mat, (x0, base, z0), (x1, eave, z1))
    if along == "x":
        zm = (z0 + z1) / 2
        for x in (x0, x1):
            part.face([(x, eave, z0), (x, eave, z1), (x, ridge, zm)], mat, out=(1 if x == x1 else -1, 0, 0))
    else:
        xm = (x0 + x1) / 2
        for z in (z0, z1):
            part.face([(x0, eave, z), (x1, eave, z), (xm, ridge, z)], mat, out=(0, 0, 1 if z == z1 else -1))


def arch_panel(part, mat, xa, xb, spring, top, z0, z1, rise, seg=12):
    """The wall above an elliptical arch opening between xa and xb, from z0 to z1."""
    xc = (xa + xb) / 2
    rx = (xb - xa) / 2
    pts = []
    for i in range(seg + 1):
        t = math.pi * i / seg
        pts.append((xc - rx * math.cos(t), spring + rise * math.sin(t)))
    for i in range(seg):
        (ax, ay), (bx, by) = pts[i], pts[i + 1]
        for z, out in ((z0, (0, 0, -1)), (z1, (0, 0, 1))):
            part.face([(ax, ay, z), (bx, by, z), (bx, top, z), (ax, top, z)], mat, out=out)
        # The underside of the arch.
        mid_x = (ax + bx) / 2
        inward = (xc - mid_x, spring - (ay + by) / 2, 0)
        part.face([(ax, ay, z0), (bx, by, z0), (bx, by, z1), (ax, ay, z1)], mat, out=inward)
    # Short walls down to the springing at both ends.
    for x, out in ((xa, (-1, 0, 0)), (xb, (1, 0, 0))):
        part.face([(x, spring, z0), (x, top, z0), (x, top, z1), (x, spring, z1)], mat, out=out)


def blob(part, mat, centre, radius, squash=0.8, seed=1, seg=10):
    """A lumpy sphere (tree crowns)."""
    rings = []
    rows = seg // 2 + 1
    for r in range(rows):
        phi = math.pi * r / (rows - 1)
        ring = []
        for i in range(seg):
            theta = 2 * math.pi * i / seg
            n = 1.0 + 0.16 * math.sin(theta * 3 + seed) * math.sin(phi * 2 + seed * 0.7)
            rr = radius * n
            ring.append(
                (
                    centre[0] + rr * math.sin(phi) * math.cos(theta),
                    centre[1] + rr * squash * math.cos(phi),
                    centre[2] + rr * math.sin(phi) * math.sin(theta),
                )
            )
        rings.append(part.add_verts(ring))
    for r in range(rows - 1):
        for i in range(seg):
            j = (i + 1) % seg
            part.face_idx([rings[r][i], rings[r][j], rings[r + 1][j], rings[r + 1][i]], mat, True, ref=centre)


# ------------------------------------------------------------------------ level

S = Part("Market")  # static scenery, split per material on export
WATER = Part("Water")
ROOFS = Part("Roofs")
FOLIAGE = Part("Foliage")
LANTERNS = Part("Lanterns")

PLAZA_X = 28.0
PLAZA_NORTH = -16.0
CANAL_N, CANAL_S = 12.5, 19.5
TOW_N = (12.5, 14.5)
TOW_S = (17.5, 19.5)
TOW_Y = -2.3
WATER_Y = -2.7
TERRACE_Y = 3.8


def build_ground():
    # The plaza and the street south of the canal (deep slabs: their faces are the canal walls).
    box(S, "Stone", (-PLAZA_X, -2.5, PLAZA_NORTH), (PLAZA_X, 0.0, CANAL_N), top="Cobble")
    box(S, "Stone", (-PLAZA_X, -2.5, CANAL_S), (PLAZA_X, 0.0, 25.0), top="Cobble")
    # Towpaths (with landings at plaza level where their stairs start) and the canal bed.
    box(S, "Stone", (-18.0, -3.4, TOW_N[0]), (PLAZA_X, TOW_Y, TOW_N[1]))
    box(S, "Stone", (-PLAZA_X, -3.4, TOW_S[0]), (18.0, TOW_Y, TOW_S[1]))
    box(S, "Stone", (-PLAZA_X, -2.5, TOW_N[0]), (-24.0, 0.0, TOW_N[1]), top="Cobble")
    box(S, "Stone", (24.0, -2.5, TOW_S[0]), (PLAZA_X, 0.0, TOW_S[1]), top="Cobble")
    box(S, "Stone", (-PLAZA_X, -3.6, TOW_N[1]), (PLAZA_X, -3.4, TOW_S[0]))
    WATER.face([(-PLAZA_X, WATER_Y, TOW_N[1]), (PLAZA_X, WATER_Y, TOW_N[1]), (PLAZA_X, WATER_Y, TOW_S[0]), (-PLAZA_X, WATER_Y, TOW_S[0])], "Water", out=(0, 1, 0))
    # Stairs down to the towpaths: ten steps of 0.23 m.
    steps = 10
    rise = -TOW_Y / steps
    for i in range(steps):
        top_y = -rise * (i + 1)
        x0 = -24.0 + 0.6 * i
        box(S, "Stone", (x0, -3.4, TOW_N[0]), (x0 + 0.6, top_y, TOW_N[1]))
        x1 = 24.0 - 0.6 * i
        box(S, "Stone", (x1 - 0.6, -3.4, TOW_S[0]), (x1, top_y, TOW_S[1]))
    # Parapets along the canal, open at the bridges and stair landings.
    def parapet(z0, z1, gaps):
        x = -PLAZA_X
        for g0, g1 in sorted(gaps) + [(PLAZA_X, PLAZA_X)]:
            if g0 > x:
                box(S, "Stone", (x, 0.0, z0), (g0, 0.65, z1))
            x = max(x, g1)

    parapet(CANAL_N - 0.3, CANAL_N, [(-PLAZA_X, -24.0), (-15.0, -13.0), (-2.5, 2.5)])
    parapet(CANAL_S, CANAL_S + 0.3, [(24.0, PLAZA_X), (-15.0, -13.0), (-2.5, 2.5)])
    box(S, "Stone", (-PLAZA_X, 0.0, TOW_N[1] - 0.3), (-24.0, 0.65, TOW_N[1]))
    box(S, "Stone", (24.0, 0.0, TOW_S[0]), (PLAZA_X, 0.65, TOW_S[0] + 0.3))
    # Iron railings beside the stairs, over the water.
    for x0, x1, z in ((-24.0, -18.0, TOW_N[1] - 0.05), (18.0, 24.0, TOW_S[0] + 0.05)):
        for k in range(7):
            x = x0 + (x1 - x0) * k / 6
            y_step = TOW_Y * (k / 6) if x0 < 0 else TOW_Y * (1 - k / 6)
            box(S, "Iron", (x - 0.03, y_step, z - 0.03), (x + 0.03, y_step + 1.0, z + 0.03))
        y0 = 1.0 if x0 < 0 else TOW_Y + 1.0
        y1 = TOW_Y + 1.0 if x0 < 0 else 1.0
        obox(S, "Iron", ((x0 + x1) / 2, (y0 + y1) / 2, z), (math.hypot(x1 - x0, y1 - y0), 0.05, 0.05), rot_z(math.degrees(math.atan2(y1 - y0, x1 - x0))))


# Where the canal leaves the market at both ends it runs on under the houses: a vaulted
# tunnel as wide as the water, its springing and depth (it stays under the end houses,
# 7 m deep, which keep the sky off it).
CULVERT_SPRING = -2.0
CULVERT_DEPTH = 6.5


def build_culverts():
    """The canal's ends: a stone headwall under the end houses with an arch over the water,
    an iron grate in it, and the vaulted tunnel the water runs on into, out of the light.
    The towpath on the other side ends against the wall. The tunnel's faces only face in,
    so closed blocks of masonry round it keep the light out (shadows and light probes
    see only front faces)."""
    z0, z1 = TOW_N[1], TOW_S[0]
    zc, r = (z0 + z1) / 2, (z1 - z0) / 2
    ring, proud, seg = 0.32, 0.08, 16
    bed = -3.4
    # The arch (intrados) from the north foot over to the south one, and the arch stones'
    # outer edge.
    arc = [(zc - r * math.cos(math.pi * i / seg), CULVERT_SPRING + r * math.sin(math.pi * i / seg)) for i in range(seg + 1)]
    outer = [(zc - (r + ring) * math.cos(math.pi * i / seg), CULVERT_SPRING + (r + ring) * math.sin(math.pi * i / seg)) for i in range(seg + 1)]
    for sign in (-1, 1):
        # d: metres beyond the market's edge, into the tunnel; facing the market is -d.
        def w(d, y, z):
            return (sign * (PLAZA_X + d), y, z)

        def o(dd, dy, dz):
            return (sign * dd, dy, dz)

        def wbox(mat, d0, y0, za, d1, y1, zb, open_top=False):
            a, b = w(d0, y0, za), w(d1, y1, zb)
            box(S, mat, tuple(map(min, a, b)), tuple(map(max, a, b)), open_top=open_top)

        facing = o(-1, 0, 0)
        # The headwall's face, round the arch (beside it, the towpath's end and what the
        # landing hides; over it, the spandrel up to the houses).
        S.face([w(0, bed, 12.5), w(0, bed, z0), w(0, 0.0, z0), w(0, 0.0, 12.5)], "Stone", out=facing)
        S.face([w(0, bed, z1), w(0, bed, CANAL_S), w(0, 0.0, CANAL_S), w(0, 0.0, z1)], "Stone", out=facing)
        for (za, ya), (zb, yb) in zip(arc, arc[1:]):
            S.face([w(0, ya, za), w(0, yb, zb), w(0, 0.0, zb), w(0, 0.0, za)], "Stone", out=facing)
        # The arch stones, standing a little proud of the wall, and the keystone.
        for i in range(seg):
            (za, ya), (zb, yb) = arc[i], arc[i + 1]
            (oa, pa), (ob, pb) = outer[i], outer[i + 1]
            S.face([w(-proud, ya, za), w(-proud, yb, zb), w(-proud, pb, ob), w(-proud, pa, oa)], "Stone", out=facing)
            mz, my = (oa + ob) / 2 - zc, (pa + pb) / 2 - CULVERT_SPRING
            S.face([w(-proud, pa, oa), w(-proud, pb, ob), w(0, pb, ob), w(0, pa, oa)], "Stone", out=o(0, my, mz))
            mz, my = (za + zb) / 2 - zc, (ya + yb) / 2 - CULVERT_SPRING
            S.face([w(-proud, ya, za), w(-proud, yb, zb), w(0, yb, zb), w(0, ya, za)], "Stone", out=o(0, -my, -mz))
        for za, zb in ((outer[0][0], z0), (z1, outer[-1][0])):
            S.face([w(-proud, CULVERT_SPRING, za), w(-proud, CULVERT_SPRING, zb), w(0, CULVERT_SPRING, zb), w(0, CULVERT_SPRING, za)], "Stone", out=(0, -1, 0))
        top = CULVERT_SPRING + r
        wbox("Stone", -proud - 0.06, top - 0.06, zc - 0.2, 0.0, top + ring + 0.1, zc + 0.2)
        # The tunnel: its walls, vault, bed, water and far end.
        for z, n in ((z0, 1), (z1, -1)):
            S.face([w(0, bed, z), w(CULVERT_DEPTH, bed, z), w(CULVERT_DEPTH, CULVERT_SPRING, z), w(0, CULVERT_SPRING, z)], "Stone", out=(0, 0, n))
        for (za, ya), (zb, yb) in zip(arc, arc[1:]):
            inward = o(0, CULVERT_SPRING - (ya + yb) / 2, zc - (za + zb) / 2)
            S.face([w(0, ya, za), w(0, yb, zb), w(CULVERT_DEPTH, yb, zb), w(CULVERT_DEPTH, ya, za)], "Stone", out=inward)
        wbox("Stone", 0.0, -3.6, z0, CULVERT_DEPTH, bed, z1)
        WATER.face([w(0, WATER_Y, z0), w(CULVERT_DEPTH, WATER_Y, z0), w(CULVERT_DEPTH, WATER_Y, z1), w(0, WATER_Y, z1)], "Water", out=(0, 1, 0))
        S.face([w(CULVERT_DEPTH, bed, z0)] + [w(CULVERT_DEPTH, y, z) for z, y in arc] + [w(CULVERT_DEPTH, bed, z1)], "Stone", out=facing)
        # The masonry it runs through, up to the houses: beside it, over it and behind its
        # end, a little back from its faces. Open at the top, inside the (hollow) houses,
        # where it would be floor to walk on.
        gap, end = 0.1, CULVERT_DEPTH + 0.5
        crown = CULVERT_SPRING + r + gap
        wbox("Stone", 0.3, -3.6, 12.5, end, 0.0, z0 - gap, open_top=True)
        wbox("Stone", 0.3, -3.6, z1 + gap, end, 0.0, CANAL_S, open_top=True)
        wbox("Stone", 0.3, crown, z0 - gap, end, 0.0, z1 + gap, open_top=True)
        wbox("Stone", CULVERT_DEPTH + gap, -3.6, z0 - gap, end, crown, z1 + gap)
        # The grate, set a little into the arch: bars from the bed into the vault, and two
        # rails across.
        bars = 14
        for k in range(bars):
            z = z0 + (k + 0.5) * (z1 - z0) / bars
            y_top = CULVERT_SPRING + math.sqrt(max(r * r - (z - zc) ** 2, 0.0)) + 0.04
            wbox("Iron", 0.14, bed, z - 0.025, 0.19, y_top, z + 0.025)
        for y in (-2.25, -1.15):
            half = r if y <= CULVERT_SPRING else math.sqrt(r * r - (y - CULVERT_SPRING) ** 2)
            wbox("Iron", 0.12, y - 0.035, zc - half, 0.21, y + 0.035, zc + half)


def build_bridges():
    # The stone bridge in the middle: a flat deck at plaza level (2 m clear over the
    # towpaths) on an arch over the water, with parapets and a gate house at its north end.
    box(S, "Stone", (-2.5, -0.3, CANAL_N), (2.5, 0.0, CANAL_S), top="Cobble")
    arch_panel(S, "Stone", TOW_N[1], TOW_S[0], WATER_Y + 0.1, -0.3, -2.3, 2.3, 1.3)
    for x0, x1 in ((-2.8, -2.5), (2.5, 2.8)):
        box(S, "Stone", (x0, -0.3, CANAL_N), (x1, 0.9, CANAL_S))
    # The gatehouse: towers and a room over the way that the raised portcullis hides in.
    for x0, x1 in ((-3.4, -2.5), (2.5, 3.4)):
        box(S, "Stone", (x0, 0.0, CANAL_N - 0.6), (x1, 6.0, CANAL_N + 0.6))
    box(S, "Stone", (-2.5, 2.9, CANAL_N - 0.6), (2.5, 6.0, CANAL_N + 0.6))
    box(S, "Stone", (-3.6, 6.0, CANAL_N - 0.8), (3.6, 6.3, CANAL_N + 0.8))
    gable_roof(ROOFS, "Roof", -3.6, 3.6, CANAL_N - 0.8, CANAL_N + 0.8, 6.3, 7.6, along="x", overhang=0.2)
    window(S, (-WINDOW_W / 2, 3.8, CANAL_N + 0.6), (WINDOW_W / 2, 3.8 + WINDOW_H, CANAL_N + 0.64), (0, 0, 1))
    # The wooden footbridge to the west.
    box(S, "Wood", (-15.0, -0.25, CANAL_N), (-13.0, 0.0, CANAL_S))
    for z in (CANAL_N + 0.3, (CANAL_N + CANAL_S) / 2, CANAL_S - 0.3):
        box(S, "Timber", (-15.0, -1.4, z - 0.15), (-13.0, -0.25, z + 0.15))
    for x in (-15.0, -13.08):
        for k in range(8):
            z = CANAL_N + 0.1 + (CANAL_S - CANAL_N - 0.2) * k / 7
            box(S, "Timber", (x, 0.0, z - 0.04), (x + 0.08, 1.0, z + 0.04))
        box(S, "Timber", (x - 0.02, 0.95, CANAL_N), (x + 0.1, 1.05, CANAL_S))


def build_terrace():
    # The block of shops under the terrace, the terrace slab over it and the arcade, and
    # its balustrade.
    box(S, "PlasterRose", (-24.0, 0.0, -20.0), (24.0, 3.4, -16.0))
    box(S, "Stone", (-24.0, 3.4, -20.0), (24.0, TERRACE_Y, -12.0), top="Cobble")
    for k in range(6):
        x = -20.0 + 8.0 * k
        box(S, "Timber", (x - 0.8, 0.0, -16.05), (x + 0.8, 2.4, -15.95))
        w = x + 2.8 - WINDOW_W / 2
        box(S, "Timber", (w - 0.1, 0.6, -16.05), (w + WINDOW_W + 0.1, 0.8 + WINDOW_H, -15.95))
        window(S, (w, 0.7, -15.95), (w + WINDOW_W, 0.7 + WINDOW_H, -15.93), (0, 0, 1))
    columns = [-24.0 + 4.0 * k for k in range(13)]
    for x in columns:
        cylinder(S, "Stone", (x, 0.0, -12.4), 0.32, 3.4, seg=14)
        box(S, "Stone", (x - 0.45, 0.0, -12.85), (x + 0.45, 0.35, -11.95))
        box(S, "Stone", (x - 0.42, 2.85, -12.82), (x + 0.42, 3.4, -11.98))
    for a, b in zip(columns, columns[1:]):
        arch_panel(S, "Stone", a + 0.42, b - 0.42, 2.0, 3.4, -12.82, -11.98, 1.2)
    box(S, "Stone", (-24.0, TERRACE_Y, -12.3), (24.0, TERRACE_Y + 1.0, -12.0))
    for x in columns:
        box(S, "Stone", (x - 0.25, TERRACE_Y + 1.0, -12.35), (x + 0.25, TERRACE_Y + 1.15, -11.95))
    # Lanterns hanging in the arcade.
    for a, b in zip(columns, columns[1:]):
        x = (a + b) / 2
        box(S, "Iron", (x - 0.02, 2.9, -13.62), (x + 0.02, 3.4, -13.58))
        box(LANTERNS, "Iron", (x - 0.2, 2.82, -13.8), (x + 0.2, 2.9, -13.4))
        box(LANTERNS, "LanternGlow", (x - 0.16, 2.45, -13.76), (x + 0.16, 2.82, -13.44))
        box(LANTERNS, "Iron", (x - 0.18, 2.4, -13.78), (x + 0.18, 2.45, -13.42))
    # The grand stair on the west: sixteen steps up to a landing at terrace height.
    steps = 16
    rise = TERRACE_Y / steps
    for i in range(steps):
        z1 = -1.0 - 0.625 * i
        box(S, "Stone", (-28.0, 0.0, z1 - 0.625), (-24.0, rise * (i + 1), z1))
    box(S, "Stone", (-28.0, 0.0, -16.0), (-24.0, TERRACE_Y, -11.0), top="Cobble")
    wedge(S, "Stone", -24.3, -24.0, -1.0, -11.0, 0.9, TERRACE_Y + 0.9)
    box(S, "Stone", (-24.3, TERRACE_Y, -12.0), (-24.0, TERRACE_Y + 0.9, -11.0))
    # The ramp on the east, with low walls.
    wedge(S, "Stone", 24.0, 28.0, 4.0, -11.0, 0.0, TERRACE_Y, top="Cobble")
    box(S, "Stone", (24.0, 0.0, -16.0), (28.0, TERRACE_Y, -11.0), top="Cobble")
    wedge(S, "Stone", 24.0, 24.3, 4.0, -11.0, 0.8, TERRACE_Y + 0.8)
    box(S, "Stone", (24.0, TERRACE_Y, -11.0), (24.3, TERRACE_Y + 0.8, -12.0))


HOUSE_PLASTERS = ["Plaster", "PlasterRose", "PlasterBlue", "Plaster"]

# Doors on the plaza level: where along each row's plaza face they stand. None in the
# west row at z -10 or the east row at -10 and 2, where the grand stair and the ramp run
# along the wall.
DOORS = {"x1": (2.0, 10.0), "x0": (10.0,), "z0": (-17.5, -3.5, 10.5, 24.5)}
DOOR_HALF = 0.7


def clear_of_doors(face, at, half):
    """A ground-floor window `half` wide each side of `at` misses the face's doors."""
    return all(abs(at - door) >= DOOR_HALF + half for door in DOORS.get(face, ()))


def house(x0, x1, z0, z1, base, eave, along, plaster, windows_on, seed, ground=None):
    ridge = eave + (3.4 if along != "x" else 2.6)
    gable_wall(S, plaster, x0, x1, z0, z1, base, eave, ridge, along=along)
    gable_roof(ROOFS, "Roof", x0, x1, z0, z1, eave, ridge + 0.05, along=along)
    # Timber framing (corner posts, a band per floor at this house's own floor height)
    # and windows onto rooms on the side facing the plaza. Floors count from the street
    # in front (`ground`, the terrace for the row behind it); each has a row of windows
    # 0.85 m up, clear of the bands, where it fits under the eaves.
    face, sign = windows_on
    ground = base if ground is None else ground
    storey = 2.45 + 0.15 * (seed % 4)
    floors = int((eave - 0.25 - ground - 0.85 - WINDOW_H) // storey) + 1
    bands = [y for y in (ground + storey * (f + 1) + 0.2 for f in range(floors + 1)) if y + 0.3 < eave]
    if face in ("z0", "z1"):
        z = z0 if face == "z0" else z1
        dz = -sign * 0.06
        span = (x0, x1)
        for x in (x0, x1):
            box(S, "Timber", (x - 0.14, base, min(z, z + dz * 1.4)), (x + 0.14, eave, max(z, z + dz * 1.4)))
        for y in bands:
            box(S, "Timber", (x0, y, min(z, z + dz)), (x1, y + 0.24, max(z, z + dz)))
        count = max(2, int((span[1] - span[0]) // 2.3))
        for f in range(floors):
            for k in range(count):
                x = span[0] + (span[1] - span[0]) * (k + 0.5) / count
                if (k * 7 + f * 3 + seed) % 5 == 0 or (f == 0 and not clear_of_doors(face, x, WINDOW_W / 2 + 0.35)):
                    continue
                y = ground + storey * f + 0.85
                lo, hi = (x - WINDOW_W / 2, y, min(z, z + dz)), (x + WINDOW_W / 2, y + WINDOW_H, max(z, z + dz))
                if covered(lo, hi, (0, 0, -sign)):
                    continue
                window(S, lo, hi, (0, 0, -sign))
                box(S, "Timber", (x - WINDOW_W / 2 - 0.15, y - 0.12, min(z, z + dz * 1.5)), (x + WINDOW_W / 2 + 0.15, y, max(z, z + dz * 1.5)))
    else:
        x = x0 if face == "x0" else x1
        dx = -sign * 0.06
        count = max(2, int((z1 - z0) // 2.3))
        for z in (z0, z1):
            box(S, "Timber", (min(x, x + dx * 1.4), base, z - 0.14), (max(x, x + dx * 1.4), eave, z + 0.14))
        for y in bands:
            box(S, "Timber", (min(x, x + dx), y, z0), (max(x, x + dx), y + 0.24, z1))
        for f in range(floors):
            for k in range(count):
                z = z0 + (z1 - z0) * (k + 0.5) / count
                if (k * 5 + f * 3 + seed) % 4 == 0 or (f == 0 and not clear_of_doors(face, z, WINDOW_W / 2 + 0.2)):
                    continue
                y = ground + storey * f + 0.85
                lo, hi = (min(x, x + dx), y, z - WINDOW_W / 2), (max(x, x + dx), y + WINDOW_H, z + WINDOW_W / 2)
                if covered(lo, hi, (-sign, 0, 0)):
                    continue
                window(S, lo, hi, (-sign, 0, 0))


def build_houses():
    # North row above the terrace: gable ends to the plaza.
    eaves = [9.0, 10.5, 9.5, 11.0, 9.0, 10.0, 11.5, 9.5]
    for k in range(8):
        x0 = -28.0 + 7.0 * k
        house(x0, x0 + 7.0, -27.0, -20.0, 0.0, eaves[k], "z", HOUSE_PLASTERS[k % 4], ("z1", -1), k, ground=TERRACE_Y)
    # South row beyond the canal.
    eaves = [8.0, 9.0, 7.5, 8.5, 9.5, 8.0, 7.5, 9.0]
    for k in range(8):
        x0 = -28.0 + 7.0 * k
        house(x0, x0 + 7.0, 25.0, 31.0, 0.0, eaves[k], "z", HOUSE_PLASTERS[(k + 2) % 4], ("z0", 1), k + 3)
    # West and east rows, ridges along z.
    spans = [(-27.0, -16.0), (-16.0, -4.0), (-4.0, 8.0), (8.0, 19.0), (19.0, 31.0)]
    for k, (z0, z1) in enumerate(spans):
        house(-35.0, -28.0, z0, z1, 0.0, [8.5, 7.5, 9.0, 8.0, 7.0][k], "z", HOUSE_PLASTERS[(k + 1) % 4], ("x1", -1), k + 5)
        house(28.0, 35.0, z0, z1, 0.0, [7.5, 9.0, 8.0, 8.5, 7.5][k], "z", HOUSE_PLASTERS[(k + 3) % 4], ("x0", 1), k + 9)
    # Doors on the plaza level, standing out of the walls (in them, their faces would
    # fight the plaster's).
    h = DOOR_HALF
    for z in DOORS["x1"]:
        box(S, "Timber", (-28.0, 0.0, z - h), (-27.94, 2.3, z + h))
    for z in DOORS["x0"]:
        box(S, "Timber", (27.94, 0.0, z - h), (28.0, 2.3, z + h))
    for x in DOORS["z0"]:
        box(S, "Timber", (x - h, 0.0, 24.94), (x + h, 2.3, 25.0))


def build_fountain():
    lathe(S, "Stone", (0.0, 0.0, 0.0), [(3.3, 0.0), (3.3, 0.55), (3.35, 0.6), (3.35, 0.7), (2.95, 0.7), (2.95, 0.3)], seg=40, cap_top=False)
    lathe(S, "Stone", (0.0, 0.0, 0.0), [(0.6, 0.3), (0.45, 0.9), (0.35, 1.4), (1.3, 1.7), (1.35, 1.85), (1.15, 1.85), (1.1, 1.75)], seg=32, cap_top=False)
    lathe(S, "Stone", (0.0, 0.0, 0.0), [(0.22, 1.75), (0.16, 2.3), (0.3, 2.5), (0.0, 2.75)], seg=20)
    lathe(WATER, "Water", (0.0, 0.0, 0.0), [(2.95, 0.48), (0.55, 0.48)], seg=40, cap_top=False)
    lathe(WATER, "Water", (0.0, 0.0, 0.0), [(1.12, 1.78), (0.2, 1.78)], seg=32, cap_top=False)


def stall(cx, cz, yaw, cloth, seed):
    """A market stall facing along its local +z (`yaw` degrees about y): counter, posts,
    sloping awning and goods."""
    r = rot_y(yaw)

    def at(x, y, z):
        v = apply(r, (x, y, z))
        return (cx + v[0], v[1], cz + v[2])

    obox(S, "Wood", at(0.0, 0.5, 0.55), (3.0, 1.0, 0.8), r)
    obox(S, "Wood", at(0.0, 1.02, 0.6), (3.2, 0.06, 1.0), r)
    for px, pz in ((-1.5, -0.9), (1.5, -0.9), (-1.5, 1.0), (1.5, 1.0)):
        height = 2.7 if pz < 0 else 2.3
        obox(S, "Timber", at(px, height / 2, pz), (0.12, height, 0.12), r)
    slope = math.degrees(math.atan2(0.5, 2.2))
    obox(S, cloth, at(0.0, 2.55, 0.1), (3.4, 0.04, 2.5), mat_mul(r, rot_x(slope)))
    # Fruit and crates on the counter, and a back table.
    for k in range(5):
        x = -1.2 + 0.6 * k
        obox(S, "Wood", at(x, 1.15, 0.6), (0.5, 0.2, 0.4), r)
        blob(S, "Fruit", at(x, 1.33, 0.6), 0.16, squash=0.6, seed=seed + k, seg=8)
    obox(S, "Wood", at(0.0, 0.4, -0.6), (2.6, 0.8, 0.5), r)


STALLS = [
    (-7.5, -6.0, 90, "ClothRed"),
    (-7.5, 5.5, 90, "ClothGold"),
    (7.5, -6.0, -90, "ClothTeal"),
    (7.5, 5.5, -90, "ClothRed"),
    (-4.5, 9.0, 180, "ClothTeal"),
    (4.5, 9.0, 180, "ClothGold"),
]


def build_market():
    for k, (x, z, yaw, cloth) in enumerate(STALLS):
        stall(x, z, yaw, cloth, k * 7)
    # The stage on the west: a raised dais with steps on its west side; agents jump down
    # off its east edge (NavLinks in the scene).
    box(S, "Stone", (-20.0, 0.0, -1.0), (-12.0, 1.1, 7.0), top="Wood")
    for i in range(3):
        x1 = -20.0 - 0.75 * i
        box(S, "Stone", (x1 - 0.75, 0.0, 0.5), (x1, 1.1 - 0.275 * (i + 1), 5.5))
    # Posts up to the canopy's underside (into it, their sides would fight its edges).
    for z in (-1.0, 7.0):
        box(S, "Timber", (-20.0, 1.1, z - 0.1), (-19.8, 3.2, z + 0.1))
        box(S, "Timber", (-12.2, 1.1, z - 0.1), (-12.0, 3.2, z + 0.1))
    box(S, "ClothRed", (-20.05, 3.2, -1.1), (-11.95, 3.5, 7.1))
    # Trees in planters.
    for k, (x, z) in enumerate([(-16.0, -9.0), (16.0, -9.0), (16.0, 2.0), (-22.5, 10.0), (21.0, 10.0)]):
        box(S, "Stone", (x - 0.9, 0.0, z - 0.9), (x + 0.9, 0.55, z + 0.9), top="Cobble")
        cylinder(S, "Bark", (x, 0.55, z), 0.2, 2.8, seg=10)
        blob(FOLIAGE, "Foliage", (x, 4.2, z), 2.0, squash=0.85, seed=k * 3 + 1, seg=12)
        blob(FOLIAGE, "Foliage", (x + 0.8, 3.6, z + 0.5), 1.2, squash=0.8, seed=k * 3 + 2, seg=10)
    # Benches round the fountain.
    for k in range(4):
        a = math.radians(45 + 90 * k)
        cx, cz = math.cos(a) * 5.2, math.sin(a) * 5.2
        r = rot_y(-math.degrees(a) + 90)
        obox(S, "Wood", (cx, 0.45, cz), (2.0, 0.08, 0.5), r)
        for side in (-0.8, 0.8):
            v = apply(r, (side, 0, 0))
            obox(S, "Stone", (cx + v[0], 0.22, cz + v[2]), (0.15, 0.45, 0.45), r)
    # Crates and barrels, clear of the way between the stage and the west stalls (the
    # crowd's road to the footbridge).
    for x, z in [(-10.5, -8.8), (11.0, -8.6), (10.5, 8.6), (-26.2, 4.0), (26.0, 6.0), (-6.0, -14.6), (14.0, -14.8)]:
        box(S, "Wood", (x - 0.45, 0.0, z - 0.45), (x + 0.45, 0.8, z + 0.45))
        cylinder(S, "Wood", (x + 1.0, 0.0, z + 0.2), 0.38, 0.95, seg=12, cap_mat="Timber")
    # Lamp posts.
    for x, z in [(-11.5, -10.5), (11.5, -10.5), (-11.0, 11.4), (11.0, 11.4), (-21.0, -6.0), (21.0, -4.0), (-8.0, 23.8), (8.0, 23.8)]:
        lamp_post(x, z)


def lamp_post(x, z):
    cylinder(S, "Iron", (x, 0.0, z), 0.12, 0.3, seg=10)
    cylinder(S, "Iron", (x, 0.3, z), 0.06, 3.0, seg=8)
    box(LANTERNS, "Iron", (x - 0.24, 3.25, z - 0.24), (x + 0.24, 3.3, z + 0.24))
    box(LANTERNS, "LanternGlow", (x - 0.18, 3.3, z - 0.18), (x + 0.18, 3.75, z + 0.18))
    lathe(LANTERNS, "Iron", (x, 0.0, z), [(0.3, 3.75), (0.05, 4.05)], seg=8)


# -------------------------------------------------------------- movable pieces


def build_cart(name):
    cart = Part(name)
    box(cart, "Wood", (-0.55, 0.45, -0.85), (0.55, 0.55, 0.85))
    for x0, x1 in ((-0.6, -0.55), (0.55, 0.6)):
        box(cart, "Wood", (x0, 0.55, -0.85), (x1, 0.95, 0.85))
    for z0, z1 in ((-0.9, -0.85), (0.85, 0.9)):
        box(cart, "Wood", (-0.6, 0.55, z0), (0.6, 0.95, z1))
    for x in (-0.66, 0.66):
        cylinder(cart, "Timber", (x, 0.38, 0.1), 0.38, 0.08, seg=14, axis=rot_z(90), smooth=False)
        cylinder(cart, "Iron", (x, 0.38, 0.1), 0.08, 0.12, seg=8, axis=rot_z(90), smooth=False)
    for x in (-0.4, 0.4):
        obox(cart, "Timber", (x, 0.7, -1.35), (0.07, 0.07, 1.1), rot_x(-12))
    box(cart, "Wood", (-0.25, 0.0, 0.7), (0.25, 0.45, 0.78))
    # A load: sacks and fruit.
    blob(cart, "ClothGold", (-0.25, 0.75, -0.3), 0.32, squash=0.7, seed=4, seg=8)
    blob(cart, "ClothGold", (0.2, 0.78, 0.25), 0.34, squash=0.7, seed=9, seg=8)
    for k in range(4):
        blob(cart, "Fruit", (-0.3 + 0.2 * k, 0.65, 0.6), 0.12, squash=0.8, seed=k, seg=6)
    return cart


def build_gate():
    gate = Part("Gate")
    for k in range(17):
        x = -2.4 + 0.3 * k
        box(gate, "Iron", (x - 0.04, 0.0, -0.05), (x + 0.04, 2.7, 0.05))
        gate.face([(x - 0.04, 0.0, 0.0), (x + 0.04, 0.0, 0.0), (x, -0.18, 0.0)], "Iron", out=(0, 0, 1))
    for k in range(6):
        y = 0.35 + 0.45 * k
        box(gate, "Iron", (-2.45, y - 0.04, -0.06), (2.45, y + 0.04, 0.06))
    return gate


# ----------------------------------------------------------------------- export


def to_blender(v):
    return (v[0], -v[2], v[1])


def make_object(part, location=(0.0, 0.0, 0.0), only=None):
    """A Blender object of `part`'s faces (only those of material `only` when given)."""
    faces = [f for f in part.faces if only is None or f[1] == only]
    if not faces:
        return None
    used = sorted({i for f in faces for i in f[0]})
    remap = {old: new for new, old in enumerate(used)}
    verts = [to_blender(sub(part.verts[i], location)) for i in used]
    polys = [[remap[i] for i in f[0]] for f in faces]
    name = part.name if only is None else f"{part.name}_{only}"
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(verts, [], polys)
    mats = sorted({f[1] for f in faces})
    for m in mats:
        mesh.materials.append(material(m))
    uv = mesh.uv_layers.new(name="UVMap")
    for poly, (idx, mat, smooth, own_uvs) in zip(mesh.polygons, faces):
        poly.material_index = mats.index(mat)
        poly.use_smooth = smooth
        if own_uvs:
            # Engine uvs; the glTF export flips v (the engine's v is 1 - Blender's).
            for loop_index, (u, v) in zip(poly.loop_indices, own_uvs):
                uv.data[loop_index].uv = (u, 1.0 - v)
            continue
        tile = MATERIALS[mat][2]
        pts = [part.verts[i] for i in idx]
        n = newell(pts)
        ax = max(range(3), key=lambda a: abs(n[a]))
        for loop_index, p in zip(poly.loop_indices, pts):
            if ax == 1:
                u, v = p[0], -p[2] if n[1] > 0 else p[2]
            elif ax == 0:
                u, v = (-p[2] if n[0] > 0 else p[2]), p[1]
            else:
                u, v = (p[0] if n[2] > 0 else -p[0]), p[1]
            uv.data[loop_index].uv = (u / tile, v / tile)
    mesh.validate()
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    obj.location = to_blender(location)
    bpy.context.scene.collection.objects.link(obj)
    return obj


_materials = {}


def material(name):
    if name in _materials:
        return _materials[name]
    colour, roughness, _, emission = MATERIALS[name]
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Base Color"].default_value = (*colour, 1.0)
    bsdf.inputs["Roughness"].default_value = roughness
    bsdf.inputs["Metallic"].default_value = 0.6 if name == "Iron" else 0.0
    if emission > 0:
        bsdf.inputs["Emission Color"].default_value = (*colour, 1.0)
        bsdf.inputs["Emission Strength"].default_value = emission
    _materials[name] = mat
    return mat


def build():
    bpy.ops.wm.read_homefile(use_empty=True, use_factory_startup=True)
    build_ground()
    build_bridges()
    build_terrace()
    build_houses()
    build_fountain()
    build_market()
    # Last, so the meshes that were there before keep their vertex order.
    build_culverts()
    objects = []
    for mat in sorted({f[1] for f in S.faces}):
        objects.append(make_object(S, only=mat))
    for part in (WATER, ROOFS, FOLIAGE, LANTERNS):
        objects.append(make_object(part))
    # Movable pieces, built round their own origin (where scripts place them) and put at
    # their starting places: the carts by the arcade and the stalls, the portcullis
    # closed across the stone bridge.
    for part, place in ((build_cart("CartA"), (-5.0, 0.0, -10.5)), (build_cart("CartB"), (12.0, 0.0, 1.0)), (build_gate(), (0.0, 0.0, CANAL_N))):
        obj = make_object(part)
        obj.location = to_blender(place)
        objects.append(obj)
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    bpy.ops.export_scene.gltf(
        filepath=OUT,
        export_format="GLB",
        export_yup=True,
        export_apply=True,
        export_cameras=False,
        export_lights=False,
    )
    triangles = 0
    for obj in objects:
        if obj:
            obj.data.calc_loop_triangles()
            triangles += len(obj.data.loop_triangles)
    return {"glb": OUT, "objects": [o.name for o in objects if o], "triangles": triangles}


if __name__ == "__main__" or "result" not in globals():
    result = build()
