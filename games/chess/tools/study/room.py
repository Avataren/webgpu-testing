"""The study: walls with wainscot, two curtained windows, a marble fireplace with a
painting, bookcases full of books, a rug, a floor lamp and a side table with a
crystal decanter (metres, Blender Z-up; the games table stands at the origin).

The left wall (-X) holds the windows, the back wall (+Y) the fireplace and the
bookcases; white sits at -Y.
"""

import math
import random

import bmesh
import bpy
from mathutils import Matrix, Vector

import blender_common as bc
from furniture import box, box_uv, tube

X0, X1 = -2.4, 2.4
Y0, Y1 = -2.6, 2.4
H = 3.0
WALL = 0.15
WAINSCOT = 0.95
WINDOWS = [(-0.9, 1.1), (0.9, 1.1)]      # (centre y, width) on the left wall
WIN_Z = (0.98, 2.62)
BREAST = (-0.9, 0.9, 0.35)              # chimney breast x range and depth
CASES = [(-2.3, -1.0), (1.0, 2.3)]      # bookcase x ranges on the back wall
CASE_DEPTH = 0.34
CASE_TOP = 2.48


class Mats:
    def __init__(self):
        self.plaster = bc.pbr("Plaster", tex="plaster", normal_strength=0.5)
        self.green = bc.pbr("PaintedGreen", tex="painted_green", normal_strength=0.4)
        self.white = bc.pbr("PaintedWhite", tex="painted_white", normal_strength=0.3)
        self.floor = bc.pbr("OakFloor", tex="oak_floor", normal_strength=0.7)
        self.marble = bc.pbr("Marble", tex="marble", normal_strength=0.3)
        self.wood = bc.pbr("Mahogany", tex="mahogany", normal_strength=0.5)
        self.walnut = bc.pbr("Walnut", tex="walnut_satin", normal_strength=0.4)
        self.books = bc.pbr("Books", tex="books", normal_strength=0.4)
        self.rug = bc.pbr("Rug", tex="rug", normal_strength=0.8)
        self.velvet = bc.pbr("Curtain", tex="velvet", normal_strength=0.5)
        self.painting = bc.pbr("Painting", tex="painting", normal_strength=0.6)
        self.gilt = bc.pbr("Gilt", color=(0.78, 0.58, 0.3), roughness=0.3, metallic=1.0)
        self.brass = bc.pbr("Brass", color=(0.85, 0.62, 0.32), roughness=0.22, metallic=1.0)
        self.silver = bc.pbr("Silver", color=(0.92, 0.92, 0.9), roughness=0.12, metallic=1.0)
        self.iron = bc.pbr("Iron", color=(0.05, 0.05, 0.05), roughness=0.55, metallic=1.0)
        self.soot = bc.pbr("Soot", color=(0.025, 0.02, 0.018), roughness=0.95)
        self.log = bc.pbr("Log", color=(0.09, 0.06, 0.04), roughness=0.9)
        self.ember = bc.pbr("Embers", color=(1.0, 0.35, 0.08), roughness=0.9,
                            emission=(1.0, 0.32, 0.06), emission_strength=6.0)
        self.crystal = bc.pbr("Crystal", color=(1, 1, 1), roughness=0.0, transmission=1.0, ior=1.52)
        self.whisky = bc.pbr("Whisky", color=(0.85, 0.45, 0.12), roughness=0.0, transmission=1.0, ior=1.36)
        self.shade = bc.pbr("LampShade", color=(0.93, 0.85, 0.7), roughness=0.9,
                            emission=(1.0, 0.78, 0.5), emission_strength=1.2)
        self.bulb = bc.pbr("Bulb", color=(1, 0.9, 0.75), roughness=0.5,
                           emission=(1.0, 0.82, 0.6), emission_strength=25.0)
        self.candle = bc.pbr("Candle", color=(0.92, 0.88, 0.8), roughness=0.6)
        self.shade.use_backface_culling = False
        self.velvet.use_backface_culling = False


def plane(name, corners, uvs, mat, parent):
    bm = bmesh.new()
    uvl = bm.loops.layers.uv.new("UVMap")
    f = bm.faces.new([bm.verts.new(c) for c in corners])
    for loop, uv in zip(f.loops, uvs):
        loop[uvl].uv = uv
    return bc.mesh_object(name, bm, mat, parent)


# ----------------------------------------------------------------------------
# Shell
# ----------------------------------------------------------------------------

def shell(m, root):
    plane("Floor", [(X0, Y0, 0), (X1, Y0, 0), (X1, Y1, 0), (X0, Y1, 0)],
          [(X0 / 2.4, Y0 / 2.4), (X1 / 2.4, Y0 / 2.4), (X1 / 2.4, Y1 / 2.4), (X0 / 2.4, Y1 / 2.4)],
          m.floor, root)
    plane("Ceiling", [(X0, Y0, H), (X0, Y1, H), (X1, Y1, H), (X1, Y0, H)],
          [(0, 0), (0, 5), (5, 5), (5, 0)], m.white, root)
    walls = []
    # Back, front and right walls: whole slabs.
    walls.append(box("WallBack", (X1 - X0 + 2 * WALL, WALL, H), ((X0 + X1) / 2, Y1 + WALL / 2, H / 2), m.plaster, root))
    walls.append(box("WallFront", (X1 - X0 + 2 * WALL, WALL, H), ((X0 + X1) / 2, Y0 - WALL / 2, H / 2), m.plaster, root))
    walls.append(box("WallRight", (WALL, Y1 - Y0, H), (X1 + WALL / 2, (Y0 + Y1) / 2, H / 2), m.plaster, root))
    # Left wall with two window openings.
    xc = X0 - WALL / 2
    edges = [Y0]
    for cy, w in WINDOWS:
        edges += [cy - w / 2, cy + w / 2]
    edges.append(Y1)
    for i in range(0, len(edges), 2):
        a, b = edges[i], edges[i + 1]
        walls.append(box(f"WallLeft{i}", (WALL, b - a, H), (xc, (a + b) / 2, H / 2), m.plaster, root))
    for j, (cy, w) in enumerate(WINDOWS):
        walls.append(box(f"WallUnder{j}", (WALL, w, WIN_Z[0]), (xc, cy, WIN_Z[0] / 2), m.plaster, root))
        walls.append(box(f"WallOver{j}", (WALL, w, H - WIN_Z[1]), (xc, cy, (H + WIN_Z[1]) / 2), m.plaster, root))
    # Chimney breast.
    bx0, bx1, bd = BREAST
    walls.append(box("ChimneyBreast", (bx1 - bx0, bd, H), ((bx0 + bx1) / 2, Y1 - bd / 2, H / 2), m.plaster, root))
    for w in walls:
        box_uv(w, 1.6, "Z")


def run_along_walls():
    """Inner wall runs (start, end, inward normal) for wainscot and mouldings."""
    bx0, bx1, bd = BREAST
    runs = [
        (Vector((X0, Y0, 0)), Vector((X1, Y0, 0)), Vector((0, 1, 0))),      # front
        (Vector((X1, Y0, 0)), Vector((X1, Y1, 0)), Vector((-1, 0, 0))),     # right
        (Vector((X1, Y1, 0)), Vector((bx1, Y1, 0)), Vector((0, -1, 0))),    # back right
        (Vector((bx1, Y1, 0)), Vector((bx1, Y1 - bd, 0)), Vector((1, 0, 0))),
        (Vector((bx1, Y1 - bd, 0)), Vector((bx0, Y1 - bd, 0)), Vector((0, -1, 0))),
        (Vector((bx0, Y1 - bd, 0)), Vector((bx0, Y1, 0)), Vector((-1, 0, 0))),
        (Vector((bx0, Y1, 0)), Vector((X0, Y1, 0)), Vector((0, -1, 0))),    # back left
        (Vector((X0, Y1, 0)), Vector((X0, Y0, 0)), Vector((1, 0, 0))),      # left
    ]
    return runs


def strip(name, a, b, normal, z0, z1, proud, mat, root, bevel=0.0):
    """A box along the wall from a to b, from z0 to z1, standing `proud` off the wall."""
    d = b - a
    length = d.length
    if length < 1e-4:
        return None
    centre = (a + b) / 2 + normal * (proud / 2)
    centre.z = (z0 + z1) / 2
    along_x = abs(d.x) > abs(d.y)
    size = (length, proud, z1 - z0) if along_x else (proud, length, z1 - z0)
    o = box(name, size, centre, mat, root, bevel=bevel, segments=2)
    box_uv(o, 1.0, "X" if along_x else "Y")
    return o


def interior_trim(m, root):
    bx0, bx1, bd = BREAST
    for i, (a, b, n) in enumerate(run_along_walls()):
        on_breast = i in (3, 4, 5)
        crown = strip(f"Crown{i}", a, b, n, H - 0.13, H, 0.075, m.white, root, bevel=0.025)
        if i == 4:
            continue  # the marble surround fills the breast front
        # Wainscot boards, cap rail and skirting.
        strip(f"Wainscot{i}", a, b, n, 0.0, WAINSCOT, 0.012, m.green, root)
        strip(f"ChairRail{i}", a, b, n, WAINSCOT - 0.03, WAINSCOT + 0.015, 0.035, m.green, root, bevel=0.008)
        strip(f"Skirting{i}", a, b, n, 0.0, 0.16, 0.026, m.green, root, bevel=0.004)
        if on_breast:
            continue
        # Raised panel frames between skirting and rail.
        length = (b - a).length
        count = max(1, int(length / 0.85))
        pw = length / count
        d = (b - a).normalized()
        for k in range(count):
            p0 = a + d * (k * pw + 0.07)
            p1 = a + d * ((k + 1) * pw - 0.07)
            for z0, z1 in ((0.24, 0.27), (0.80, 0.83)):
                strip(f"PanelRail{i}_{k}_{z0}", p0, p1, n + Vector((0, 0, 0)), z0, z1, 0.024, m.green, root, bevel=0.006)
            for p in (p0, p1):
                q0 = p - d * 0.015
                q1 = p + d * 0.015
                strip(f"PanelStile{i}_{k}", q0, q1, n, 0.24, 0.83, 0.024, m.green, root, bevel=0.006)


def windows(m, root):
    for j, (cy, w) in enumerate(WINDOWS):
        y0, y1 = cy - w / 2, cy + w / 2
        z0, z1 = WIN_Z
        xin, xout = X0, X0 - WALL
        # Reveals.
        for name, size, c in ((f"RevealL{j}", (WALL, 0.02, z1 - z0), (X0 - WALL / 2, y0 + 0.01, (z0 + z1) / 2)),
                              (f"RevealR{j}", (WALL, 0.02, z1 - z0), (X0 - WALL / 2, y1 - 0.01, (z0 + z1) / 2)),
                              (f"RevealTop{j}", (WALL, w, 0.02), (X0 - WALL / 2, cy, z1 - 0.01))):
            o = box(name, size, c, m.white, root)
            box_uv(o, 1.0, "Z")
        sill = box(f"Sill{j}", (WALL + 0.07, w + 0.12, 0.035), (X0 - WALL / 2 + 0.035, cy, z0 + 0.0175), m.white, root,
                   bevel=0.006)
        box_uv(sill, 1.0, "Y")
        # Architrave round the opening.
        for name, size, c in ((f"ArchL{j}", (0.02, 0.09, z1 - z0 + 0.09), (X0 + 0.01, y0 - 0.045, (z0 + z1 + 0.09) / 2)),
                              (f"ArchR{j}", (0.02, 0.09, z1 - z0 + 0.09), (X0 + 0.01, y1 + 0.045, (z0 + z1 + 0.09) / 2)),
                              (f"ArchTop{j}", (0.02, w + 0.18, 0.09), (X0 + 0.01, cy, z1 + 0.045))):
            o = box(name, size, c, m.white, root, bevel=0.005)
            box_uv(o, 1.0, "Z")
        # Sash: frame and glazing bars (no glass, so the sun falls straight in).
        xs = X0 - WALL * 0.55
        fw, fd = 0.055, 0.05
        for name, size, c in ((f"SashL{j}", (fd, fw, z1 - z0), (xs, y0 + fw / 2, (z0 + z1) / 2)),
                              (f"SashR{j}", (fd, fw, z1 - z0), (xs, y1 - fw / 2, (z0 + z1) / 2)),
                              (f"SashB{j}", (fd, w, fw), (xs, cy, z0 + 0.035 + fw / 2)),
                              (f"SashT{j}", (fd, w, fw), (xs, cy, z1 - fw / 2)),
                              (f"SashMeet{j}", (fd + 0.01, w, 0.045), (xs, cy, (z0 + z1) / 2))):
            o = box(name, size, c, m.white, root, bevel=0.004)
            box_uv(o, 1.0, "Z")
        bar = 0.022
        inner0, inner1 = y0 + fw, y1 - fw
        for k in (1, 2):
            yb = inner0 + (inner1 - inner0) * k / 3
            o = box(f"BarV{j}{k}", (fd * 0.7, bar, z1 - z0), (xs, yb, (z0 + z1) / 2), m.white, root, bevel=0.003)
            box_uv(o, 1.0, "Z")
        zc = (z0 + z1) / 2
        for zz in (z0 + 0.035 + fw + (zc - z0 - 0.035 - fw) / 2, zc + (z1 - fw - zc) / 2):
            o = box(f"BarH{j}{zz:.2f}", (fd * 0.7, w, bar), (xs, cy, zz), m.white, root, bevel=0.003)
            box_uv(o, 1.0, "Y")
        # Curtain rod and two curtains gathered to the sides.
        rod_z = z1 + 0.16
        tube(f"Rod{j}", [(X0 + 0.11, y0 - 0.38, rod_z), (X0 + 0.11, y1 + 0.38, rod_z)], 0.012, m.brass, root,
             sides=14, steps=1)
        for k, ye in enumerate((y0 - 0.38, y1 + 0.38)):
            ball = bmesh.new()
            bmesh.ops.create_uvsphere(ball, u_segments=16, v_segments=10, radius=0.028)
            bmesh.ops.translate(ball, vec=Vector((X0 + 0.11, ye + (-0.02 if k == 0 else 0.02), rod_z)), verts=ball.verts)
            o = bc.mesh_object(f"Finial{j}{k}", ball, m.brass, root)
            bc.shade_smooth(o, 80)
        for k, (ya, yb) in enumerate(((y0 - 0.36, y0 + 0.08), (y1 - 0.08, y1 + 0.36))):
            curtain(f"Curtain{j}{k}", X0 + 0.13, ya, yb, 0.03, rod_z - 0.03, m.velvet, root, seed=j * 2 + k)


def curtain(name, x, ya, yb, z0, z1, mat, root, seed=0):
    """Pleated velvet: a sheet with deep folds that loosen toward the hem."""
    rnd = random.Random(seed)
    nu, nv = 90, 24
    folds = (yb - ya) / 0.09
    phase = rnd.uniform(0, 6.28)
    bm = bmesh.new()
    uvl = bm.loops.layers.uv.new("UVMap")
    grid = []
    for j in range(nv + 1):
        t = j / nv
        z = z0 + (z1 - z0) * t
        row = []
        for i in range(nu + 1):
            s = i / nu
            depth = 0.045 + 0.02 * (1 - t) + 0.01 * math.sin(s * 9 + phase)
            xx = x + depth * (0.5 + 0.5 * math.sin(2 * math.pi * folds * s + phase + 0.6 * math.sin(t * 3)))
            row.append(bm.verts.new((xx, ya + (yb - ya) * s, z)))
        grid.append(row)
    for j in range(nv):
        for i in range(nu):
            f = bm.faces.new((grid[j][i], grid[j][i + 1], grid[j + 1][i + 1], grid[j + 1][i]))
            for loop, (a, b) in zip(f.loops, ((i, j), (i + 1, j), (i + 1, j + 1), (i, j + 1))):
                loop[uvl].uv = (a / nu * (yb - ya) * 3, b / nv * (z1 - z0) * 3)
    o = bc.mesh_object(name, bm, mat, root)
    bc.shade_smooth(o, 80)
    return o


# ----------------------------------------------------------------------------
# Fireplace, painting, mantel
# ----------------------------------------------------------------------------

def fireplace(m, root):
    bx0, bx1, bd = BREAST
    face = Y1 - bd
    ox0, ox1, oz = -0.45, 0.45, 0.85
    depth = 0.32
    # Firebox: back, sides and roof in soot, set into the breast.
    for name, size, c in (("FireBack", (ox1 - ox0, 0.02, oz), (0, face + depth, oz / 2)),
                          ("FireSideL", (0.02, depth, oz), (ox0, face + depth / 2, oz / 2)),
                          ("FireSideR", (0.02, depth, oz), (ox1, face + depth / 2, oz / 2)),
                          ("FireRoof", (ox1 - ox0, depth, 0.02), (0, face + depth / 2, oz))):
        box(name, size, c, m.soot, root)
    # The breast is solid: open the hole by carving the slab.
    breast = bpy.data.objects["ChimneyBreast"]
    cutter = box("FireCut", (ox1 - ox0, depth * 2, oz), (0, face + depth - 0.0, oz / 2), None, None)
    mod = breast.modifiers.new("cut", "BOOLEAN")
    mod.operation = "DIFFERENCE"
    mod.solver = "EXACT"
    mod.object = cutter
    bc.apply_modifiers(breast)
    bpy.data.objects.remove(cutter, do_unlink=True)
    box_uv(breast, 1.6, "Z")
    # Marble surround, mantel shelf and hearth.
    proud = 0.06
    for name, size, c in (("SurroundL", (0.3, proud, 1.1), (ox0 - 0.15, face - proud / 2, 0.55)),
                          ("SurroundR", (0.3, proud, 1.1), (ox1 + 0.15, face - proud / 2, 0.55)),
                          ("Lintel", (ox1 - ox0, proud, 1.1 - oz), (0, face - proud / 2, (oz + 1.1) / 2)),
                          ("Mantel", (1.7, 0.2, 0.05), (0, face - 0.1 + 0.02, 1.125)),
                          ("Hearth", (1.8, 0.42, 0.03), (0, face - 0.21 + 0.02, 0.015))):
        o = box(name, size, c, m.marble, root, bevel=0.006)
        box_uv(o, 0.8, "X")
    # Grate, logs and embers.
    for k in range(5):
        x = -0.28 + k * 0.14
        tube(f"GrateBar{k}", [(x, face + 0.08, 0.05), (x, face + 0.22, 0.05), (x, face + 0.26, 0.2)], 0.008,
             m.iron, root, sides=8, steps=4)
    tube("GrateFront", [(-0.32, face + 0.08, 0.05), (0.32, face + 0.08, 0.05)], 0.01, m.iron, root, sides=8, steps=1)
    rnd = random.Random(7)
    for k, (a, b) in enumerate((((-0.28, face + 0.14, 0.1), (0.25, face + 0.18, 0.12)),
                                ((-0.22, face + 0.22, 0.12), (0.3, face + 0.2, 0.15)),
                                ((-0.15, face + 0.17, 0.2), (0.18, face + 0.24, 0.17)))):
        tube(f"Log{k}", [a, b], 0.045 - k * 0.006, m.log, root, sides=12, steps=1)
    bm = bmesh.new()
    for k in range(40):
        p = Vector((rnd.uniform(-0.28, 0.28), face + rnd.uniform(0.1, 0.27), rnd.uniform(0.04, 0.09)))
        bmesh.ops.create_icosphere(bm, subdivisions=1, radius=rnd.uniform(0.012, 0.03),
                                   matrix=Matrix.Translation(p))
    bc.mesh_object("Embers", bm, m.ember, root)
    # Brass fender on the hearth.
    tube("Fender", [(-0.55, face - 0.32, 0.06), (-0.5, face - 0.36, 0.06), (0.5, face - 0.36, 0.06),
                    (0.55, face - 0.32, 0.06)], 0.012, m.brass, root, sides=12, steps=6)
    # Candlesticks on the mantel.
    for sx in (-1, 1):
        stick = bc.lathe(f"Candlestick{sx}", [(0, 0, "S"), (0.045, 0, "S"), (0.045, 0.008), (0.03, 0.018),
                                              (0.012, 0.04), (0.009, 0.12), (0.016, 0.16), (0.009, 0.18),
                                              (0.008, 0.25), (0.022, 0.262), (0.022, 0.27, "S"), (0.0, 0.27, "S")],
                         segments=48, material=m.brass, parent=root, u_repeat=1, v_scale=1)
        stick.data.transform(Matrix.Translation((sx * 0.62, face - 0.08, 1.15)))
        bc.shade_smooth(stick, 40)
        candle = bc.lathe(f"Candle{sx}", [(0, 0, "S"), (0.011, 0, "S"), (0.011, 0.2), (0.006, 0.206), (0, 0.207, "S")],
                          segments=24, material=m.candle, parent=root)
        candle.data.transform(Matrix.Translation((sx * 0.62, face - 0.08, 1.42)))
    # Painting in a gilt frame above the mantel.
    pw, ph = 1.0, 0.71
    cz = 1.98
    plane("Painting", [(-pw / 2, face - 0.025, cz - ph / 2), (pw / 2, face - 0.025, cz - ph / 2),
                       (pw / 2, face - 0.025, cz + ph / 2), (-pw / 2, face - 0.025, cz + ph / 2)],
          [(0, 0), (1, 0), (1, 1), (0, 1)], m.painting, root)
    fw = 0.085
    for name, size, c in (("FrameTop", (pw + 2 * fw, 0.05, fw), (0, face - 0.025, cz + ph / 2 + fw / 2)),
                          ("FrameBottom", (pw + 2 * fw, 0.05, fw), (0, face - 0.025, cz - ph / 2 - fw / 2)),
                          ("FrameLeft", (fw, 0.05, ph), (-pw / 2 - fw / 2, face - 0.025, cz)),
                          ("FrameRight", (fw, 0.05, ph), (pw / 2 + fw / 2, face - 0.025, cz))):
        o = box(name, size, c, m.gilt, root, bevel=0.018, segments=4)
        box_uv(o, 0.5, "X")


# ----------------------------------------------------------------------------
# Bookcases
# ----------------------------------------------------------------------------

SHELVES = [0.1, 0.48, 0.86, 1.24, 1.62, 2.0]


def bookcases(m, root):
    rnd = random.Random(11)
    books = bmesh.new()
    uvl = books.loops.layers.uv.new("UVMap")
    front = Y1 - CASE_DEPTH
    for ci, (xa, xb) in enumerate(CASES):
        t = 0.025
        parts = [("Side0", (t, CASE_DEPTH, CASE_TOP), (xa + t / 2, Y1 - CASE_DEPTH / 2, CASE_TOP / 2)),
                 ("Side1", (t, CASE_DEPTH, CASE_TOP), (xb - t / 2, Y1 - CASE_DEPTH / 2, CASE_TOP / 2)),
                 ("Top", (xb - xa + 0.06, CASE_DEPTH + 0.03, 0.04), ((xa + xb) / 2, Y1 - CASE_DEPTH / 2 - 0.015, CASE_TOP + 0.02)),
                 ("Plinth", (xb - xa, 0.02, 0.1), ((xa + xb) / 2, front + 0.01, 0.05)),
                 ("Back", (xb - xa, 0.01, CASE_TOP), ((xa + xb) / 2, Y1 - 0.005, CASE_TOP / 2))]
        for z in SHELVES:
            parts.append((f"Shelf{z}", (xb - xa - 2 * t, CASE_DEPTH - 0.012, 0.022), ((xa + xb) / 2, Y1 - CASE_DEPTH / 2 - 0.006, z)))
        mid = (xa + xb) / 2
        parts.append(("Divider", (t, CASE_DEPTH - 0.012, CASE_TOP - 0.1), (mid, Y1 - CASE_DEPTH / 2, (CASE_TOP + 0.1) / 2)))
        for name, size, c in parts:
            o = box(f"Case{ci}{name}", size, c, m.wood, root, bevel=0.002)
            box_uv(o, 1.0, "X" if size[0] > size[2] else "Z", offset=(rnd.random(), rnd.random()))
        # Fill each bay of each shelf with books.
        bays = [(xa + t, mid - t / 2), (mid + t / 2, xb - t)]
        for si, z in enumerate(SHELVES):
            top = SHELVES[si + 1] - 0.011 if si + 1 < len(SHELVES) else CASE_TOP - 0.005
            base = z + 0.011
            for b0, b1 in bays:
                fill_shelf(books, uvl, rnd, b0, b1, base, top, front)
    obj = bc.mesh_object("Books", books, m.books, root)
    return obj


def add_book(bm, uvl, rnd, x, y_front, z, thick, depth, height, lean=0.0, lie=False):
    """One book: spine faces -Y. UVs pick a spine design from the atlas; covers use a
    plain strip of it and the page edges use the pages columns."""
    cols = 16
    design = rnd.randrange(14)
    u0, u1 = design / cols, (design + 1) / cols
    pages = (14 / cols + 0.01, 16 / cols - 0.01)
    if lie:
        sx, sy, sz = height, depth, thick
    else:
        sx, sy, sz = thick, depth, height
    corners = [Vector((dx * sx, dy * sy, dz * sz)) for dz in (0, 1) for dy in (0, 1) for dx in (0, 1)]
    rot = Matrix.Rotation(lean, 4, "Y")
    pos = Vector((x, y_front, z))
    vs = [bm.verts.new(pos + (rot @ c)) for c in corners]
    # Faces: spine (y=0), back (y=1), covers (x=0/1), top/bottom (z).
    spine = (vs[0], vs[1], vs[5], vs[4])
    faces = [
        (spine, "spine"), ((vs[3], vs[2], vs[6], vs[7]), "pages"),
        ((vs[2], vs[0], vs[4], vs[6]), "cover"), ((vs[1], vs[3], vs[7], vs[5]), "cover"),
        ((vs[4], vs[5], vs[7], vs[6]), "pages"), ((vs[0], vs[2], vs[3], vs[1]), "pages"),
    ]
    for verts, kind in faces:
        f = bm.faces.new(verts)
        if kind == "spine":
            # The design runs up the spine: along z standing, along x lying flat.
            uvs = ((u0, 0.03), (u1, 0.03), (u1, 0.97), (u0, 0.97)) if not lie else \
                ((u0, 0.03), (u0, 0.97), (u1, 0.97), (u1, 0.03))
        elif kind == "cover":
            uvs = ((u0 + 0.01, 0.45), (u1 - 0.01, 0.45), (u1 - 0.01, 0.55), (u0 + 0.01, 0.55))
        else:
            uvs = ((pages[0], 0.1), (pages[1], 0.1), (pages[1], 0.9), (pages[0], 0.9))
        for loop, uv in zip(f.loops, uvs):
            loop[uvl].uv = uv


def fill_shelf(bm, uvl, rnd, x0, x1, base, top, front):
    room = top - base
    x = x0 + rnd.uniform(0.0, 0.03)
    while x < x1 - 0.03:
        r = rnd.random()
        if r < 0.06:
            x += rnd.uniform(0.04, 0.12)   # a gap
            continue
        if r < 0.12 and x1 - x > 0.3:
            # A short stack lying flat.
            hgt = 0
            depth = rnd.uniform(0.16, 0.22)
            length = rnd.uniform(0.2, min(0.28, room - 0.02))
            for _ in range(rnd.randint(2, 4)):
                th = rnd.uniform(0.025, 0.045)
                if hgt + th > room - 0.02:
                    break
                add_book(bm, uvl, rnd, x + rnd.uniform(0, 0.01), front + 0.01 + rnd.uniform(0, 0.02), base + hgt,
                         th, depth, length, lie=True)
                hgt += th
            x += length + 0.02
            continue
        thick = rnd.uniform(0.018, 0.05)
        height = min(room - 0.01, rnd.uniform(0.2, 0.33))
        depth = rnd.uniform(0.15, 0.24)
        if x + thick > x1:
            break
        lean = 0.0
        if r > 0.95 and x1 - x > 0.15:
            lean = -rnd.uniform(0.12, 0.3)
            add_book(bm, uvl, rnd, x + math.sin(-lean) * height, front + 0.01, base, thick, depth, height, lean=lean)
            x += math.sin(-lean) * height + thick + 0.01
            continue
        add_book(bm, uvl, rnd, x, front + 0.008 + rnd.uniform(0, 0.025), base, thick, depth, height)
        x += thick + rnd.uniform(0.0005, 0.003)


# ----------------------------------------------------------------------------
# Rug, lamp, side table with decanter
# ----------------------------------------------------------------------------

def rug(m, root):
    w, d = 2.6, 1.8
    bm = bmesh.new()
    uvl = bm.loops.layers.uv.new("UVMap")
    # Rug long side along Y (between the players), with a slightly rounded edge.
    corners = [(-d / 2, -w / 2), (d / 2, -w / 2), (d / 2, w / 2), (-d / 2, w / 2)]
    top = [bm.verts.new((x, y, 0.009)) for x, y in corners]
    bot = [bm.verts.new((x * 1.002, y * 1.002, 0.0005)) for x, y in corners]
    f = bm.faces.new(top)
    for loop, (u, v) in zip(f.loops, ((0, 0), (0, 1), (1, 1), (1, 0))):
        loop[uvl].uv = (v, u)
    for i in range(4):
        j = (i + 1) % 4
        f = bm.faces.new((bot[i], bot[j], top[j], top[i]))
        for loop in f.loops:
            loop[uvl].uv = (0.005, 0.005)
    o = bc.mesh_object("Rug", bm, m.rug, root)
    # Texture is 2.6 x 1.8 with the long side along u: map u to Y.
    uv = o.data.uv_layers.active.data
    for poly in o.data.polygons:
        if poly.normal.z > 0.5:
            for li in poly.loop_indices:
                co = o.data.vertices[o.data.loops[li].vertex_index].co
                uv[li].uv = ((co.y + w / 2) / w, (co.x + d / 2) / d)
    return o


def floor_lamp(m, root, at):
    x, y = at
    stand = bc.lathe("LampStand", [(0, 0, "S"), (0.16, 0, "S"), (0.16, 0.012), (0.12, 0.03), (0.03, 0.05),
                                   (0.016, 0.08), (0.014, 0.6), (0.02, 0.62), (0.014, 0.64), (0.012, 1.42),
                                   (0.02, 1.44), (0.02, 1.47, "S"), (0.0, 1.47, "S")],
                     segments=48, material=m.brass, parent=root, u_repeat=1, v_scale=1)
    stand.data.transform(Matrix.Translation((x, y, 0)))
    bc.shade_smooth(stand, 40)
    shade = bc.lathe("LampShadeCone", [(0.25, 1.36, "S"), (0.15, 1.66, "S")], segments=64, steps=1,
                     material=m.shade, parent=root, u_repeat=3, v_scale=2)
    shade.data.transform(Matrix.Translation((x, y, 0)))
    bc.shade_smooth(shade, 80)
    bulb = bmesh.new()
    bmesh.ops.create_uvsphere(bulb, u_segments=24, v_segments=16, radius=0.04,
                              matrix=Matrix.Translation((x, y, 1.5)))
    o = bc.mesh_object("LampBulb", bulb, m.bulb, root)
    bc.shade_smooth(o, 80)


def side_table(m, root, at):
    x, y = at
    top_z = 0.62
    t = bc.lathe("SideTable", [(0, 0, "S"), (0.2, 0, "S"), (0.2, 0.02), (0.16, 0.035), (0.05, 0.06),
                               (0.035, 0.1), (0.03, 0.3), (0.045, 0.45), (0.035, 0.56), (0.06, 0.585),
                               (0.06, 0.59, "S"), (0.28, 0.59, "S"), (0.29, 0.6), (0.29, 0.612), (0.28, top_z, "S"),
                               (0.0, top_z, "S")],
                 segments=64, material=m.wood, parent=root, u_repeat=1, v_scale=1)
    t.data.transform(Matrix.Translation((x, y, 0)))
    bc.shade_smooth(t, 40)
    # Silver tray.
    tray = bc.lathe("Tray", [(0, 0, "S"), (0.17, 0, "S"), (0.185, 0.012), (0.19, 0.016), (0.183, 0.016, "S"),
                             (0.168, 0.004, "S"), (0.0, 0.004, "S")],
                    segments=64, material=m.silver, parent=root)
    tray.data.transform(Matrix.Translation((x, y, top_z)))
    bc.shade_smooth(tray, 50)
    base = top_z + 0.004
    # Faceted crystal decanter (12 facets) with a ball stopper, a dram of whisky inside.
    dec = bc.lathe("Decanter", [(0, 0, "S"), (0.06, 0, "S"), (0.065, 0.01), (0.068, 0.12), (0.06, 0.15),
                                (0.025, 0.175), (0.02, 0.2), (0.024, 0.205, "S"), (0.0, 0.205, "S")],
                   segments=12, steps=4, material=m.crystal, parent=root)
    dec.data.transform(Matrix.Translation((x - 0.05, y + 0.03, base)))
    stopper = bmesh.new()
    bmesh.ops.create_icosphere(stopper, subdivisions=2, radius=0.03,
                               matrix=Matrix.Translation((x - 0.05, y + 0.03, base + 0.235)))
    bc.mesh_object("Stopper", stopper, m.crystal, root)
    liquid = bc.lathe("DecanterWhisky", [(0, 0.012, "S"), (0.052, 0.012, "S"), (0.056, 0.02), (0.058, 0.08, "S"),
                                         (0.0, 0.08, "S")],
                      segments=12, steps=3, material=m.whisky, parent=root)
    liquid.data.transform(Matrix.Translation((x - 0.05, y + 0.03, base)))
    for k, (gx, gy) in enumerate(((0.07, -0.06), (0.09, 0.07))):
        g = bc.lathe(f"Tumbler{k}", [(0, 0, "S"), (0.036, 0, "S"), (0.04, 0.085, "S"), (0.034, 0.085, "S"),
                                     (0.031, 0.018, "S"), (0.0, 0.018, "S")],
                     segments=10, steps=1, material=m.crystal, parent=root)
        g.data.transform(Matrix.Translation((x + gx, y + gy, base)))
        if k == 0:
            dram = bc.lathe("TumblerWhisky", [(0, 0.0185, "S"), (0.0315, 0.0185, "S"), (0.033, 0.04, "S"),
                                              (0.0, 0.04, "S")],
                            segments=10, steps=1, material=m.whisky, parent=root)
            dram.data.transform(Matrix.Translation((x + gx, y + gy, base)))


def build(export=True):
    bc.reset()
    root = bc.empty("Study")
    m = Mats()
    shell(m, root)
    interior_trim(m, root)
    windows(m, root)
    fireplace(m, root)
    bookcases(m, root)
    rug(m, root)
    floor_lamp(m, root, (1.75, 1.55))
    side_table(m, root, (-1.25, 0.55))
    for o in root.children:
        if o.type == "MESH" and not o.data.polygons[0].use_smooth:
            bc.shade_smooth(o, 35)
    if export:
        return bc.export_glb("study_room")
    return root
