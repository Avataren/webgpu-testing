"""Games table, analog chess clock and bentwood chairs (metres, Blender Z-up).

Each builder returns its root empty; export() writes one GLB per piece of
furniture so the scene can place and duplicate them.
"""

import math

import bmesh
import bpy
from mathutils import Matrix, Vector

import blender_common as bc

TABLE_TOP = 0.75


# ----------------------------------------------------------------------------
# Geometry helpers
# ----------------------------------------------------------------------------

def box(name, size, center, mat, parent=None, bevel=0.0, segments=3):
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=Vector(size), verts=bm.verts)
    bmesh.ops.translate(bm, vec=Vector(center), verts=bm.verts)
    obj = bc.mesh_object(name, bm, mat, parent)
    if bevel:
        mod = obj.modifiers.new("bevel", "BEVEL")
        mod.width = bevel
        mod.segments = segments
        mod.limit_method = "ANGLE"
        bc.apply_modifiers(obj)
    return obj


def box_uv(obj, scale=1.0, grain="X", offset=(0.0, 0.0)):
    """Planar UVs per face from its dominant axis; wood grain (texture V) follows `grain`
    wherever the face contains that axis."""
    me = obj.data
    if not me.uv_layers:
        me.uv_layers.new(name="UVMap")
    uv = me.uv_layers.active.data
    gi = "XYZ".index(grain)
    for poly in me.polygons:
        n = poly.normal
        axis = max(range(3), key=lambda i: abs(n[i]))
        others = [i for i in range(3) if i != axis]
        if gi in others:
            vi = gi
            ui = others[0] if others[1] == gi else others[1]
        else:
            ui, vi = others
        for li in poly.loop_indices:
            co = me.vertices[me.loops[li].vertex_index].co
            uv[li].uv = (co[ui] / scale + offset[0], co[vi] / scale + offset[1])


def tube(name, points, radii, mat, parent=None, sides=14, steps=6, closed=False, v_scale=1 / 0.6,
         caps=True):
    """Sweep a circle along a Catmull-Rom path through points, radius interpolated.
    UV v follows the length so wood grain runs along the tube."""
    pts = [Vector(p) for p in points]
    n = len(pts)
    if isinstance(radii, (int, float)):
        radii = [radii] * n
    path = []
    rad = []
    segs = n if closed else n - 1
    for i in range(segs):
        p0 = pts[(i - 1) % n] if (closed or i > 0) else pts[0] * 2 - pts[1]
        p1 = pts[i]
        p2 = pts[(i + 1) % n]
        p3 = pts[(i + 2) % n] if (closed or i + 2 < n) else pts[-1] * 2 - pts[-2]
        for s in range(steps):
            t = s / steps
            path.append(Vector(bc.catmull(p0, p1, p2, p3, t)))
            rad.append(radii[i] + (radii[(i + 1) % n] - radii[i]) * t)
    if not closed:
        path.append(pts[-1])
        rad.append(radii[-1])
    # Parallel-transport frames.
    tangents = []
    m = len(path)
    for i in range(m):
        a = path[i - 1] if (i > 0 or closed) else path[i]
        b = path[(i + 1) % m] if (i < m - 1 or closed) else path[i]
        tangents.append((b - a).normalized())
    ref = Vector((0, 0, 1)) if abs(tangents[0].z) < 0.9 else Vector((1, 0, 0))
    normal = tangents[0].cross(ref).normalized()
    frames = []
    for i in range(m):
        if i > 0:
            axis = tangents[i - 1].cross(tangents[i])
            if axis.length > 1e-8:
                ang = tangents[i - 1].angle(tangents[i])
                normal = (Matrix.Rotation(ang, 3, axis.normalized()) @ normal).normalized()
        binormal = tangents[i].cross(normal).normalized()
        frames.append((normal.copy(), binormal))
    bm = bmesh.new()
    uvl = bm.loops.layers.uv.new("UVMap")
    rings = []
    length = [0.0]
    for i in range(1, m):
        length.append(length[-1] + (path[i] - path[i - 1]).length)
    for i in range(m):
        nrm, bin_ = frames[i]
        ring = []
        for j in range(sides + 1):
            a = 2 * math.pi * (j % sides) / sides
            ring.append(bm.verts.new(path[i] + (nrm * math.cos(a) + bin_ * math.sin(a)) * rad[i]))
        rings.append(ring)
    count = m if closed else m - 1
    for i in range(count):
        r0, r1 = rings[i], rings[(i + 1) % m]
        v0 = length[i] * v_scale
        v1 = (length[i + 1] if i + 1 < m else length[-1] + (path[0] - path[-1]).length) * v_scale
        for j in range(sides):
            f = bm.faces.new((r0[j], r0[j + 1], r1[j + 1], r1[j]))
            for loop, (u, v) in zip(f.loops, ((j / sides, v0), ((j + 1) / sides, v0),
                                              ((j + 1) / sides, v1), (j / sides, v1))):
                loop[uvl].uv = (u * 0.25, v)
    if caps and not closed:
        for ring, flip in ((rings[0], True), (rings[-1], False)):
            verts = ring[:-1]
            f = bm.faces.new(list(reversed(verts)) if flip else verts)
            for loop in f.loops:
                loop[uvl].uv = (0.0, 0.0)
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-6)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    obj = bc.mesh_object(name, bm, mat, parent)
    bc.shade_smooth(obj, 50.0)
    return obj


def disc(name, radius, center, normal, mat, parent=None, segments=64, uv_full=True):
    bm = bmesh.new()
    uvl = bm.loops.layers.uv.new("UVMap")
    verts = []
    for i in range(segments):
        a = 2 * math.pi * i / segments
        verts.append(bm.verts.new((radius * math.cos(a), radius * math.sin(a), 0.0)))
    f = bm.faces.new(verts)
    for loop in f.loops:
        co = loop.vert.co
        loop[uvl].uv = (0.5 + co.x / (2 * radius), 0.5 + co.y / (2 * radius))
    obj = bc.mesh_object(name, bm, mat, parent)
    rot = Vector((0, 0, 1)).rotation_difference(Vector(normal).normalized()).to_matrix().to_4x4()
    obj.data.transform(Matrix.Translation(center) @ rot)
    return obj


# ----------------------------------------------------------------------------
# Table
# ----------------------------------------------------------------------------

LEG_PROFILE = [
    (0, 0, "S"), (0.019, 0, "S"), (0.021, 0.004), (0.023, 0.014), (0.020, 0.024, "S"),
    (0.0165, 0.030), (0.018, 0.045), (0.0205, 0.07), (0.020, 0.10), (0.017, 0.16),
    (0.0145, 0.26), (0.0135, 0.33, "S"), (0.017, 0.336, "S"), (0.019, 0.345), (0.017, 0.354, "S"),
    (0.0145, 0.36), (0.016, 0.40), (0.019, 0.46), (0.020, 0.50), (0.019, 0.54), (0.016, 0.565, "S"),
    (0.020, 0.570, "S"), (0.0225, 0.58), (0.0225, 0.596, "S"), (0.0, 0.60, "S"),
]


def table():
    root = bc.empty("GamesTable")
    wood = bc.pbr("Mahogany", tex="mahogany", normal_strength=0.5)
    leather = bc.pbr("TableLeather", tex="leather_panel", normal_strength=0.6)
    top = box("TableTop", (1.0, 0.82, 0.032), (0, 0, TABLE_TOP - 0.016), wood, root, bevel=0.007,
              segments=5)
    box_uv(top, 1.0, "X")
    # Leather writing surface laid into the top.
    lw, ld = 0.70, 0.54
    bm = bmesh.new()
    uvl = bm.loops.layers.uv.new("UVMap")
    vs = [bm.verts.new((x, y, TABLE_TOP + 0.0004)) for x, y in
          ((-lw / 2, -ld / 2), (lw / 2, -ld / 2), (lw / 2, ld / 2), (-lw / 2, ld / 2))]
    f = bm.faces.new(vs)
    for loop, uvc in zip(f.loops, ((0, 0), (1, 0), (1, 1), (0, 1))):
        loop[uvl].uv = uvc
    bc.mesh_object("TableLeather", bm, leather, root)
    # Apron and legs.
    ax, ay, t, h = 0.43, 0.34, 0.022, 0.09
    z = TABLE_TOP - 0.032 - h / 2
    for i, (size, c) in enumerate((((2 * ax, t, h), (0, ay - t / 2, z)), ((2 * ax, t, h), (0, -ay + t / 2, z)),
                                   ((t, 2 * ay, h), (ax - t / 2, 0, z)), ((t, 2 * ay, h), (-ax + t / 2, 0, z)))):
        a = box(f"Apron{i}", size, c, wood, root, bevel=0.002)
        box_uv(a, 1.0, "X" if size[0] > size[1] else "Y", offset=(0.3 * i, 0.17 * i))
    for i, (sx, sy) in enumerate(((1, 1), (-1, 1), (1, -1), (-1, -1))):
        x, y = sx * (ax - 0.023), sy * (ay - 0.023)
        leg = bc.lathe(f"Leg{i}", LEG_PROFILE, segments=48, material=wood, parent=root,
                       u_repeat=0.25, v_scale=1 / 1.0)
        leg.data.transform(Matrix.Translation((x, y, 0)))
        block = box(f"LegBlock{i}", (0.046, 0.046, TABLE_TOP - 0.032 - 0.6), (x, y, (0.6 + TABLE_TOP - 0.032) / 2),
                    wood, root, bevel=0.002)
        box_uv(block, 1.0, "Z", offset=(0.13 * i, 0.0))
    for o in root.children:
        if o.type == "MESH":
            bc.shade_smooth(o, 40.0)
    return root


# ----------------------------------------------------------------------------
# Chess clock
# ----------------------------------------------------------------------------

def chess_clock():
    """Analog double clock, long axis along Y, dials facing +X, standing on z = 0."""
    root = bc.empty("ChessClock")
    case_mat = bc.pbr("ClockCase", tex="walnut_satin", normal_strength=0.4)
    brass = bc.pbr("Brass", color=(0.85, 0.62, 0.32), roughness=0.22, metallic=1.0)
    dark_brass = bc.pbr("BrassAged", color=(0.55, 0.40, 0.22), roughness=0.38, metallic=1.0)
    dial_mat = bc.pbr("Dial", tex="dial", roughness=0.35)
    glass = bc.pbr("ClockGlass", color=(1, 1, 1), roughness=0.02, transmission=1.0, ior=1.5)
    black = bc.pbr("HandBlack", color=(0.02, 0.02, 0.02), roughness=0.3, metallic=0.6)
    red = bc.pbr("FlagRed", color=(0.6, 0.04, 0.03), roughness=0.4)

    depth, length, height = 0.07, 0.20, 0.10
    front_top = 0.012
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=Vector((depth, length, height)), verts=bm.verts)
    bmesh.ops.translate(bm, vec=Vector((0, 0, height / 2 + 0.006)), verts=bm.verts)
    for v in bm.verts:
        if v.co.x > 0 and v.co.z > 0.05:
            v.co.x = front_top
    case = bc.mesh_object("ClockCase", bm, case_mat, root)
    mod = case.modifiers.new("bevel", "BEVEL")
    mod.width = 0.003
    mod.segments = 4
    mod.limit_method = "ANGLE"
    bc.apply_modifiers(case)
    box_uv(case, 0.5, "Y")

    # Front face plane: from (depth/2, z0) to (front_top, z1).
    z0, z1 = 0.006, 0.006 + height
    fx0, fx1 = depth / 2, front_top
    slope = Vector((fx1 - fx0, 0, z1 - z0)).normalized()
    normal = Vector((slope.z, 0, -slope.x)).normalized()

    def on_face(y, s, out=0.0):
        """Point on the front face at height parameter s (0 bottom .. 1 top)."""
        p = Vector((fx0 + (fx1 - fx0) * s, y, z0 + (z1 - z0) * s))
        return p + normal * out

    rot_face = Vector((0, 0, 1)).rotation_difference(normal).to_matrix().to_4x4()
    for side, (y, hours, minutes) in enumerate(((-0.05, 4.9, 52.0), (0.05, 5.2, 11.0))):
        c = on_face(y, 0.5)
        right = slope.cross(normal).normalized()
        basis = Matrix((right, slope, normal)).transposed().to_4x4()
        dial = disc(f"Dial{side}", 0.031, (0, 0, 0), (0, 0, 1), dial_mat, root)
        dial.data.transform(Matrix.Translation(c + normal * 0.0012) @ basis)
        ring_pts = [(0.0335 * math.cos(a), 0.0335 * math.sin(a), 0.0)
                    for a in [2 * math.pi * i / 48 for i in range(48)]]
        ring = tube(f"Bezel{side}", ring_pts, 0.0035, brass, root, sides=14, steps=2, closed=True)
        ring.data.transform(Matrix.Translation(c + normal * 0.0025) @ rot_face)
        # Domed crystal.
        cap = bmesh.new()
        bmesh.ops.create_uvsphere(cap, u_segments=48, v_segments=24, radius=0.06)
        bmesh.ops.delete(cap, geom=[v for v in cap.verts if v.co.z < 0.06 - 0.007], context="VERTS")
        bmesh.ops.translate(cap, vec=Vector((0, 0, -(0.06 - 0.007))), verts=cap.verts)
        crystal = bc.mesh_object(f"Crystal{side}", cap, glass, root)
        crystal.data.transform(Matrix.Translation(c + normal * 0.002) @ rot_face)
        bc.shade_smooth(crystal, 80)
        # Hands, rotated in the dial plane. 12 o'clock points up the slope.

        def hand(name, angle_deg, length, width, lift, mat):
            bmh = bmesh.new()
            bmesh.ops.create_cube(bmh, size=1.0)
            bmesh.ops.scale(bmh, vec=Vector((width, length, 0.0006)), verts=bmh.verts)
            bmesh.ops.translate(bmh, vec=Vector((0, length * 0.42, 0)), verts=bmh.verts)
            for v in bmh.verts:
                if v.co.y > length * 0.6:
                    v.co.x *= 0.45
            o = bc.mesh_object(name, bmh, mat, root)
            o.data.transform(Matrix.Translation(c + normal * (0.0018 + lift)) @ basis
                             @ Matrix.Rotation(-math.radians(angle_deg), 4, "Z"))
            return o

        hand(f"HourHand{side}", hours * 30, 0.017, 0.0028, 0.0, black)
        hand(f"MinuteHand{side}", minutes * 6, 0.025, 0.0018, 0.0007, black)
        hub = disc(f"Hub{side}", 0.0022, c + normal * 0.0028, normal, brass, root, segments=24)
        # Flag: a little red triangle hanging from a pin near 12.
        bmf = bmesh.new()
        tri = [bmf.verts.new(p) for p in ((0, 0, 0), (0.0035, -0.0075, 0), (-0.0012, -0.0068, 0))]
        bmf.faces.new(tri)
        flag = bc.mesh_object(f"Flag{side}", bmf, red, root)
        flag.data.transform(Matrix.Translation(c + normal * 0.0016) @ basis @ Matrix.Translation((0.004, 0.0215, 0)))
        flag.data.materials[0].use_backface_culling = False

    # Plungers on top, one pressed.
    top_z = z1
    top_x = (-depth / 2 + front_top) / 2
    for side, (y, up) in enumerate(((-0.05, 0.016), (0.05, 0.008))):
        stem = bc.lathe(f"Plunger{side}", [(0, 0, "S"), (0.0055, 0, "S"), (0.0055, up, "S"),
                                            (0.0095, up, "S"), (0.0098, up + 0.004), (0.0085, up + 0.007),
                                            (0.0, up + 0.0085, "S")],
                        segments=40, material=brass, parent=root)
        stem.data.transform(Matrix.Translation((top_x, y, top_z - 0.001)))
        bc.shade_smooth(stem, 40)
        collar = bc.lathe(f"PlungerCollar{side}", [(0.0, 0, "S"), (0.012, 0, "S"), (0.012, 0.002),
                                                   (0.009, 0.003, "S"), (0.0, 0.003, "S")],
                          segments=40, material=dark_brass, parent=root)
        collar.data.transform(Matrix.Translation((top_x, y, top_z - 0.0005)))
    # Bun feet.
    for sx in (-1, 1):
        for sy in (-1, 1):
            foot = bc.lathe("Foot", [(0, 0, "S"), (0.004, 0), (0.0062, 0.003), (0.0055, 0.0055), (0, 0.0065, "S")],
                            segments=24, material=brass, parent=root)
            foot.data.transform(Matrix.Translation((sx * 0.025, sy * 0.085, 0)))
            bc.shade_smooth(foot, 60)
    # A small brass plate between the dials.
    box("Plate", (0.0008, 0.03, 0.009), on_face(0.0, 0.2, 0.0005), brass, root, bevel=0.0003)
    for o in root.children:
        if o.type == "MESH" and not o.name.startswith(("Dial", "Flag", "Hub")):
            bc.shade_smooth(o, 35)
    return root


# ----------------------------------------------------------------------------
# Bentwood chair
# ----------------------------------------------------------------------------

def chair():
    """Thonet-style bentwood chair, seat at 0.46 m, facing +Y."""
    root = bc.empty("BentwoodChair")
    wood = bc.pbr("ChairWood", tex="walnut_satin", normal_strength=0.4)
    seat_mat = bc.pbr("SeatVelvet", tex="velvet", normal_strength=0.5)
    seat_z = 0.445
    half = [(0.19, -0.27, 0.0), (0.176, -0.215, 0.22), (0.156, -0.158, seat_z), (0.150, -0.178, 0.60),
            (0.145, -0.212, 0.75), (0.12, -0.236, 0.845), (0.065, -0.246, 0.89)]
    back = [(-x, y, z) for x, y, z in half] + [(0.0, -0.248, 0.90)] + list(reversed(half))
    back_r = [0.0105, 0.0135, 0.0155, 0.0145, 0.013, 0.012, 0.0115, 0.0115, 0.0115, 0.012, 0.013,
              0.0145, 0.0155, 0.0135, 0.0105]
    tube("BackFrame", back, back_r, wood, root, sides=16, steps=6)
    inner_half = [(0.1, -0.16, seat_z + 0.012), (0.096, -0.19, 0.6), (0.075, -0.212, 0.70)]
    inner = [(-x, y, z) for x, y, z in inner_half] + [(0.0, -0.22, 0.735)] + list(reversed(inner_half))
    tube("BackLoop", inner, 0.0095, wood, root, sides=12, steps=6)
    for sx in (-1, 1):
        tube(f"FrontLeg{sx}", [(sx * 0.14, 0.14, seat_z - 0.005), (sx * 0.152, 0.17, 0.22), (sx * 0.165, 0.2, 0.0)],
             [0.0155, 0.0135, 0.0105], wood, root, sides=14, steps=5)
    ring = [(0.24 * math.cos(a), -0.031 + 0.24 * math.sin(a), 0.205)
            for a in [2 * math.pi * i / 32 for i in range(32)]]
    tube("LegRing", ring, 0.0085, wood, root, sides=14, steps=3, closed=True)
    seat_ring = [(0.205 * math.cos(a), 0.205 * math.sin(a), seat_z - 0.004)
                 for a in [2 * math.pi * i / 40 for i in range(40)]]
    tube("SeatRing", seat_ring, 0.0135, wood, root, sides=16, steps=3, closed=True)
    pad = bc.lathe("SeatPad", [(0.0, seat_z - 0.006, "S"), (0.198, seat_z - 0.006, "S"), (0.203, seat_z + 0.004),
                               (0.199, seat_z + 0.014), (0.17, seat_z + 0.024), (0.09, seat_z + 0.03),
                               (0.0, seat_z + 0.031, "S")],
                   segments=64, material=seat_mat, parent=root, u_repeat=4.0, v_scale=6.0)
    bc.shade_smooth(pad, 60)
    return root


def export(which=("table", "clock", "chair")):
    paths = {}
    for name in which:
        bc.reset()
        {"table": table, "clock": chess_clock, "chair": chair}[name]()
        paths[name] = bc.export_glb({"table": "games_table", "clock": "chess_clock", "chair": "bentwood_chair"}[name])
    return paths
