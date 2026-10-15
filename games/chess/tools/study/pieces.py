"""Staunton chess pieces, built in Blender (run through blender_common helpers).

Sizes follow a tournament set for 55 mm squares: king 95 mm tall on a 42 mm base.
Lathe pieces are exact surfaces of revolution; the knight's head is a signed
distance field meshed through OpenVDB, so carved details blend smoothly.
"""

import math
import os
import time

import bmesh
import bpy
import numpy as np
from mathutils import Matrix, Vector

import blender_common as bc

MM = 0.001


def base_points(radius, height_scale=None):
    """Plinth, bead and scotia shared by every piece, scaled from a 21 mm radius base."""
    k = radius / 21.0
    kz = height_scale if height_scale is not None else k
    canon = [
        (0, 0, "S"), (20.4, 0, "S"), (21, 0.6, "S"), (21, 2.6, "S"), (20.4, 3.3, "S"),
        (19.6, 3.5, "S"), (19.6, 4.2), (19.0, 4.6, "S"),
        (18.9, 5.0), (19.5, 5.9), (19.0, 6.9), (18.0, 7.4, "S"),
        (16.6, 7.8), (14.4, 9.0), (12.8, 11.0), (11.8, 13.5),
    ]
    return [(p[0] * k, p[1] * kz) + tuple(p[2:]) for p in canon]


def ball(cz, r, from_r, steps=10):
    """Points over the top of a ball of radius r centred at height cz, starting where
    its radius is from_r on the lower half."""
    a0 = -math.acos(min(1.0, from_r / r))
    pts = []
    for i in range(1, steps + 1):
        a = a0 + (math.pi / 2 - a0) * i / steps
        pts.append((r * math.cos(a), cz + r * math.sin(a)))
    pts[-1] = (0.0, cz + r, "S")
    return pts


def collar(z, inner, outer, thick=1.0):
    """A disc collar: step out, rounded edge, step back in."""
    return [
        (inner + 0.4, z, "S"), (outer - 0.6, z + 0.25), (outer, z + thick * 0.55),
        (outer - 0.6, z + thick * 1.1, "S"), (inner + 0.4, z + thick * 1.3, "S"),
    ]


def king_profile():
    pts = base_points(21.0)
    pts += [(10.6, 20), (9.2, 30), (8.1, 40), (7.6, 46, "S")]
    pts += [(9.6, 46.6, "S"), (10.8, 47.2), (10.9, 48.0), (9.8, 48.5, "S"),
            (9.0, 48.7, "S"), (12.3, 49.3), (13.2, 50.4), (12.3, 51.6, "S"),
            (10.0, 51.9, "S"), (10.8, 52.5), (10.8, 53.2), (9.6, 53.7, "S"), (8.0, 53.9, "S")]
    pts += [(8.2, 55.5), (9.4, 59), (11.2, 63.5), (12.6, 67.5), (13.1, 70, "S")]
    pts += [(13.9, 70.4, "S"), (14.2, 71.3), (13.4, 72.3, "S")]
    pts += [(12.2, 72.7), (10.2, 74.6), (7.4, 76.4), (4.4, 77.4), (3.0, 77.9, "S")]
    pts += [(2.8, 78.4), (3.6, 79.3), (3.4, 80.0), (0.0, 80.3, "S")]
    return pts


def queen_profile():
    pts = base_points(20.0)
    pts += [(10.2, 19), (8.8, 28), (7.6, 37), (7.1, 42, "S")]
    pts += [(9.2, 42.6, "S"), (10.4, 43.2), (10.5, 44.0), (9.4, 44.5, "S"),
            (8.6, 44.7, "S"), (11.8, 45.3), (12.6, 46.4), (11.8, 47.6, "S"),
            (9.6, 47.9, "S"), (10.2, 48.5), (10.2, 49.1), (9.0, 49.6, "S"), (7.6, 49.8, "S")]
    pts += [(7.9, 51.2), (8.9, 54.8), (10.6, 59.0), (11.9, 62.8), (12.5, 65.2, "S")]
    pts += [(13.1, 65.5, "S"), (13.3, 66.4), (12.6, 67.2, "S")]
    pts += [(11.2, 67.4), (9.6, 68.8), (7.4, 70.4), (4.8, 71.5), (3.0, 72.0, "S")]
    pts += [(2.7, 72.6), (3.0, 73.3, "S")]
    pts += ball(77.0, 4.2, 3.0)
    return pts


def bishop_profile():
    pts = base_points(18.0)
    pts += [(9.4, 17), (7.8, 25), (6.6, 33), (6.1, 37.2, "S")]
    pts += [(8.6, 37.8, "S"), (9.8, 38.6), (9.9, 39.4), (8.6, 40.0, "S"), (6.0, 40.3, "S")]
    pts += [(6.3, 41.3), (8.4, 43.6), (9.8, 47.0), (10.1, 50.6), (9.4, 54.6), (7.7, 58.2),
            (5.2, 61.0), (2.8, 62.8), (1.7, 63.5, "S")]
    pts += [(1.6, 64.0), (2.0, 64.4, "S")]
    pts += ball(66.5, 2.5, 2.0, 8)
    return pts


def rook_profile():
    pts = base_points(18.5)
    pts += [(11.8, 13.0), (11.3, 16.5), (11.5, 24), (12.1, 31), (12.6, 36.4, "S")]
    pts += [(14.2, 36.8, "S"), (14.8, 37.7), (14.3, 38.6, "S"), (13.0, 38.9, "S")]
    pts += [(13.2, 39.6), (14.4, 41.4), (15.1, 43.6), (15.3, 46.0, "S"), (15.3, 51.0, "S"),
            (14.9, 51.6, "S"), (12.0, 51.6, "S"), (11.5, 51.2, "S"), (11.5, 47.6, "S"),
            (0.0, 47.6, "S")]
    return pts


def pawn_profile():
    pts = base_points(15.5)
    pts += [(7.9, 13.5), (6.8, 18.0), (6.1, 22.4, "S")]
    pts += [(8.4, 23.0, "S"), (9.7, 23.9), (9.8, 24.9), (8.6, 25.6, "S"), (5.2, 26.0, "S")]
    pts += [(4.6, 26.8), (4.9, 28.0, "S")]
    pts += ball(35.6, 8.0, 4.9, 14)
    return pts


def knight_base_profile():
    pts = base_points(18.0)
    pts += [(9.8, 13.6), (9.4, 15.5, "S"), (12.6, 16.0, "S"), (13.4, 17.0), (12.9, 18.2, "S"),
            (12.0, 18.5, "S"), (12.0, 19.0, "S"), (0.0, 19.0, "S")]
    return pts


# ----------------------------------------------------------------------------
# Booleans and small parts
# ----------------------------------------------------------------------------

def box_bm(size, center=(0, 0, 0), bevel=0.0):
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=Vector(size), verts=bm.verts)
    bmesh.ops.translate(bm, vec=Vector(center), verts=bm.verts)
    if bevel:
        bmesh.ops.bevel(bm, geom=bm.edges[:] + bm.verts[:], offset=bevel, segments=3,
                        profile=0.5, affect="EDGES")
    return bm


def boolean_difference(target, cutters):
    for c in cutters:
        mod = target.modifiers.new("cut", "BOOLEAN")
        mod.operation = "DIFFERENCE"
        mod.solver = "EXACT"
        mod.object = c
        bc.apply_modifiers(target)
        bpy.data.objects.remove(c, do_unlink=True)


def cylinder_uv(obj, u_repeat=2.0, v_scale=1 / 80.0):
    """Cylindrical UVs around Z for parts that were not lathed."""
    me = obj.data
    if not me.uv_layers:
        me.uv_layers.new(name="UVMap")
    uv = me.uv_layers.active.data
    for poly in me.polygons:
        cx = sum(me.vertices[v].co.x for v in poly.vertices) / len(poly.vertices)
        cy = sum(me.vertices[v].co.y for v in poly.vertices) / len(poly.vertices)
        base_a = math.atan2(cy, cx)
        for li in poly.loop_indices:
            co = me.vertices[me.loops[li].vertex_index].co
            a = math.atan2(co.y, co.x)
            # Keep a face on one side of the seam.
            while a - base_a > math.pi:
                a -= 2 * math.pi
            while a - base_a < -math.pi:
                a += 2 * math.pi
            uv[li].uv = (a / (2 * math.pi) * u_repeat, co.z * v_scale)


def king(mat):
    body = bc.lathe("king_body", king_profile(), material=mat)
    # Cross pattee: a vertical bar and a crossbar, gently flared and bevelled.
    parts = []
    for size, center in (((3.8, 3.6, 14.2), (0, 0, 87.4)), ((12.2, 3.3, 3.6), (0, 0, 89.8))):
        bm = box_bm(size, center, bevel=0.55)
        parts.append(bc.mesh_object("king_cross", bm, mat))
    # Flare the arm ends a little.
    for p in parts:
        for v in p.data.vertices:
            if abs(v.co.x) > 4.0:
                v.co.z = 89.8 + (v.co.z - 89.8) * 1.25
            if v.co.z > 92.5:
                v.co.x *= 1.22
            if v.co.z < 81.5 and abs(v.co.x) < 2.0:
                v.co.x *= 0.9
    for p in parts:
        cylinder_uv(p)
    obj = bc.join([body] + parts, "king")
    return obj


def queen(mat):
    body = bc.lathe("queen_body", queen_profile(), material=mat)
    balls = []
    for i in range(10):
        a = 2 * math.pi * i / 10
        bm = bmesh.new()
        bmesh.ops.create_uvsphere(bm, u_segments=16, v_segments=10, radius=1.55)
        bmesh.ops.translate(bm, vec=Vector((12.2 * math.cos(a), 12.2 * math.sin(a), 67.8)), verts=bm.verts)
        o = bc.mesh_object("queen_ball", bm, mat)
        cylinder_uv(o)
        balls.append(o)
    return bc.join([body] + balls, "queen")


def bishop(mat):
    body = bc.lathe("bishop", bishop_profile(), material=mat)
    # Mitre cut: a slot sloping down toward the front of the head.
    bm = box_bm((30, 1.7, 12), (0, 7.5, 0))
    cutter = bc.mesh_object("bishop_cut", bm)
    cutter.matrix_world = Matrix.Translation((0, 0, 55.5)) @ Matrix.Rotation(math.radians(-38), 4, "X")
    bc.apply_modifiers(cutter)
    cutter.data.transform(cutter.matrix_world)
    cutter.matrix_world = Matrix()
    boolean_difference(body, [cutter])
    return body


def rook(mat):
    body = bc.lathe("rook", rook_profile(), material=mat)
    cutters = []
    for i in range(6):
        a = math.pi * i / 6
        bm = box_bm((40, 3.6, 6), (0, 0, 51.6))
        c = bc.mesh_object("rook_cut", bm)
        c.data.transform(Matrix.Rotation(a + math.pi / 12, 4, "Z") @ Matrix.Translation((0, 0, 0)))
        # Only cut the turret wall, not the floor of the bowl.
        for v in c.data.vertices:
            if v.co.z < 49.0:
                v.co.z = 48.8
        cutters.append(c)
    boolean_difference(body, cutters)
    return body


def pawn(mat):
    return bc.lathe("pawn", pawn_profile(), material=mat)


# ----------------------------------------------------------------------------
# Knight head as an SDF
# ----------------------------------------------------------------------------

KNIGHT_SILHOUETTE = [
    # (x, z) in mm, horse facing +X; "S" marks creases.
    (8.6, 18.6, "S"), (9.2, 22.5), (8.8, 26.5), (7.6, 30.0), (6.4, 32.6),   # chest
    (6.0, 34.2),                                                           # throat
    (8.6, 34.4), (12.0, 33.6), (15.6, 32.6), (18.6, 32.4),                  # under the jaw
    (20.6, 33.0), (21.6, 34.4),                                            # chin, lower lip
    (21.3, 35.4, "S"), (22.0, 35.9),                                       # mouth corner
    (22.8, 37.3), (22.9, 39.0), (22.0, 40.4),                              # nose
    (19.0, 42.6), (15.0, 45.6), (11.0, 48.8), (7.2, 52.0), (3.6, 54.8),     # face
    (0.6, 56.6), (-2.4, 56.4), (-5.2, 54.6), (-7.6, 51.4), (-9.6, 47.0),    # poll, crest
    (-10.9, 41.8), (-11.5, 36.2), (-11.5, 30.4), (-11.0, 24.6), (-10.4, 18.6, "S"),
]


def closed_smooth(points, steps=8):
    """Closed Catmull-Rom loop; creases ('S') stay sharp."""
    n = len(points)
    pts = [(p[0], p[1]) for p in points]
    sharp = [len(p) > 2 and p[2] == "S" for p in points]
    out = []
    for i in range(n):
        p1, p2 = pts[i], pts[(i + 1) % n]
        out.append(p1)
        if sharp[i] and sharp[(i + 1) % n]:
            continue
        p0 = pts[i - 1] if not sharp[i] else (2 * p1[0] - p2[0], 2 * p1[1] - p2[1])
        p3 = pts[(i + 2) % n] if not sharp[(i + 1) % n] else (2 * p2[0] - p1[0], 2 * p2[1] - p1[1])
        for s in range(1, steps):
            out.append(bc.catmull(p0, p1, p2, p3, s / steps))
    return np.array(out)


def polygon_sdf(px, pz, poly):
    """Signed distance (negative inside) from grid points to a closed polygon."""
    d = np.full(px.shape, np.inf)
    inside = np.zeros(px.shape, dtype=bool)
    n = len(poly)
    for i in range(n):
        ax, az = poly[i]
        bx, bz = poly[(i + 1) % n]
        ex, ez = bx - ax, bz - az
        wx, wz = px - ax, pz - az
        t = np.clip((wx * ex + wz * ez) / (ex * ex + ez * ez + 1e-12), 0, 1)
        dx, dz = wx - ex * t, wz - ez * t
        d = np.minimum(d, dx * dx + dz * dz)
        cond = ((az > pz) != (bz > pz)) & (px < (bx - ax) * (pz - az) / (bz - az + 1e-12) + ax)
        inside ^= cond
    return np.where(inside, -1, 1) * np.sqrt(d)


def smin(a, b, k):
    h = np.clip(0.5 + 0.5 * (b - a) / k, 0, 1)
    return b + (a - b) * h - k * h * (1 - h)


def smax(a, b, k):
    return -smin(-a, -b, k)


def capsule(x, y, z, a, b, ra, rb):
    """Round cone from a (radius ra) to b (radius rb)."""
    a = np.array(a)
    b = np.array(b)
    ba = b - a
    l2 = ba.dot(ba)
    t = np.clip(((x - a[0]) * ba[0] + (y - a[1]) * ba[1] + (z - a[2]) * ba[2]) / l2, 0, 1)
    dx = x - (a[0] + ba[0] * t)
    dy = y - (a[1] + ba[1] * t)
    dz = z - (a[2] + ba[2] * t)
    return np.sqrt(dx * dx + dy * dy + dz * dz) - (ra + (rb - ra) * t)


def ellipsoid(x, y, z, c, r):
    k0 = np.sqrt(((x - c[0]) / r[0]) ** 2 + ((y - c[1]) / r[1]) ** 2 + ((z - c[2]) / r[2]) ** 2)
    return (k0 - 1.0) * min(r)


def knight_half_width(x, z):
    hw = 6.5 - 0.02 * (z - 20.0)
    hw = hw * (1.0 - 0.5 * bc_smoothstep(5.0, 22.0, x))
    return hw * (1.0 - 0.2 * bc_smoothstep(50.0, 57.0, z))


def side_carve(x, y, z, body, a, b, radius, depth, min_y=2.2):
    """A groove along the line a-b (x, z) cut only into the side faces, depth deep."""
    line = capsule(x, np.zeros_like(y), z, (a[0], 0, a[1]), (b[0], 0, b[1]), radius, radius)
    return np.maximum(np.maximum(line, -(body + depth)), min_y - np.abs(y))


def knight_head_sdf(x, y, z):
    poly = closed_smooth(KNIGHT_SILHOUETTE)
    d2 = polygon_sdf(x[:, 0, :], z[:, 0, :], poly)[:, None, :]
    hw = knight_half_width(x, z)
    rr = 2.6
    qx = d2 + rr
    qy = np.abs(y) - hw + rr
    body = np.sqrt(np.maximum(qx, 0) ** 2 + np.maximum(qy, 0) ** 2) + np.minimum(np.maximum(qx, qy), 0) - rr

    for s in (-1, 1):
        # Flat cheek (the jaw muscle) and the soft bulb of the muzzle.
        body = smin(body, ellipsoid(x, y, z, (7.4, s * 3.0, 39.6), (4.8, 2.6, 4.2)), 1.4)
        body = smin(body, ellipsoid(x, y, z, (20.4, s * 1.5, 36.6), (2.6, 1.9, 3.0)), 1.2)
        # Brow ridge over the eye.
        body = smin(body, ellipsoid(x, y, z, (5.4, s * 4.3, 52.2), (2.3, 1.1, 0.9)), 0.8)
        # Nostril flare.
        body = smin(body, ellipsoid(x, y, z, (21.0, s * 2.3, 39.4), (1.6, 1.2, 1.4)), 0.8)

    # Mane: a low crest down the back of the neck, carved into locks that sweep
    # back and down the sides (chevrons seen from behind).
    zone = np.maximum(x - (-2.5 + np.maximum(z - 48.0, 0) * 0.9), 22.0 - z)  # <0 in the zone
    zone = np.maximum(zone, x - 0.0)
    crest = np.maximum(np.maximum(body - 0.6, np.abs(y) - 3.4), zone)
    body = smin(body, crest, 0.5)
    phi = np.arctan2(z - 36.0, x + 1.0)
    u = (phi * 12.0 + 0.85 * np.abs(y) + 0.25 * np.sin(phi * 9.0)) / 1.7
    stripe = np.abs(u - np.floor(u) - 0.5) * 1.7 - 0.2
    depth = 0.55 * (1.0 - bc_smoothstep(2.8, 4.2, np.abs(y)))
    groove = np.maximum(np.maximum(stripe, -(body + depth)), zone + 0.6)
    groove = np.maximum(groove, np.abs(y) - 3.7)
    body = smax(body, -groove, 0.3)
    for s in (-1, 1):
        edge = np.abs(y - s * 3.8) - 0.28
        channel = np.maximum(np.maximum(edge, -(body + 0.35)), zone + 0.4)
        channel = np.maximum(channel, z - 53.0)
        body = smax(body, -channel, 0.25)

    # Ears: leaf shaped and upright on the poll, hollowed at the front.
    for s in (-1, 1):
        ear = capsule(x, y * 1.25, z, (-0.4, s * 3.3, 54.0), (-1.0, s * 3.8, 60.0), 2.0, 0.6)
        body = smin(body, ear, 0.9)
        body = smax(body, -ellipsoid(x, y, z, (0.7, s * 2.8, 57.6), (0.8, 0.7, 2.2)), 0.3)

    for s in (-1, 1):
        ex, ez = 5.8, 50.4
        ey = s * (float(knight_half_width(ex, ez)) - 0.9)
        # Eye: a socket under the brow with the eyeball sitting in it.
        body = smax(body, -ellipsoid(x, y, z, (ex, ey + s * 1.5, ez), (1.9, 1.4, 1.5)), 0.4)
        body = smin(body, ellipsoid(x, y, z, (ex + 0.2, ey + s * 0.4, ez), (1.3, 1.0, 1.05)), 0.25)
        body = smax(body, -ellipsoid(x, y, z, (21.8, s * 2.4, 39.4), (0.85, 0.7, 0.7)), 0.3)

    # Mouth line along the muzzle, the jaw line around the cheek, and the
    # crease from the nostril.
    body = smax(body, -side_carve(x, y, z, body, (23.5, 35.7), (19.4, 36.4), 0.36, 0.7, 0.0), 0.25)
    jaw_pts = [(12.6, 34.5), (8.4, 35.0), (5.6, 36.8), (4.5, 39.8), (4.9, 43.2)]
    jaw = side_carve(x, y, z, body, jaw_pts[0], jaw_pts[1], 0.3, 0.32)
    for a, b in zip(jaw_pts[1:], jaw_pts[2:]):
        jaw = np.minimum(jaw, side_carve(x, y, z, body, a, b, 0.3, 0.32))
    body = smax(body, -jaw, 0.3)
    body = smax(body, -side_carve(x, y, z, body, (20.4, 41.0), (17.4, 38.4), 0.3, 0.3, 1.0), 0.25)

    # The neck flares into the top of the base with a fillet.
    disk = np.maximum(np.sqrt(x * x + y * y) - 11.6, np.abs(z - 18.8) - 0.6)
    body = smin(body, disk, 2.2)
    body = np.maximum(body, 18.4 - z)
    return body


def bc_smoothstep(a, b, x):
    t = np.clip((x - a) / (b - a), 0, 1)
    return t * t * (3 - 2 * t)


def sdf_to_mesh(name, sdf_fn, lo, hi, voxel):
    """Sample sdf_fn over a box and mesh its zero level with OpenVDB."""
    import openvdb as vdb

    xs = np.arange(lo[0], hi[0] + voxel, voxel)
    ys = np.arange(lo[1], hi[1] + voxel, voxel)
    zs = np.arange(lo[2], hi[2] + voxel, voxel)
    x, y, z = np.meshgrid(xs, ys, zs, indexing="ij")
    d = sdf_fn(x, y, z)
    dens = np.clip(0.5 - d / voxel, 0.0, 1.0).astype(np.float32)
    grid = vdb.FloatGrid()
    grid.copyFromArray(dens)
    grid.name = "density"
    grid.transform = vdb.createLinearTransform(voxelSize=voxel)
    # Blender caches grids by file path: a fresh name per build avoids stale shapes.
    path = os.path.join(bc.OUT, f"_{name}_{time.time_ns()}.vdb")
    os.makedirs(bc.OUT, exist_ok=True)
    vdb.write(path, grids=[grid])
    bpy.ops.object.volume_import(filepath=path)
    vol = bpy.context.active_object
    holder = bc.link(bpy.data.objects.new(name, bpy.data.meshes.new(name)))
    ng = bpy.data.node_groups.new("vol_to_mesh", "GeometryNodeTree")
    ng.interface.new_socket("Geometry", in_out="OUTPUT", socket_type="NodeSocketGeometry")
    info = ng.nodes.new("GeometryNodeObjectInfo")
    info.inputs["Object"].default_value = vol
    v2m = ng.nodes.new("GeometryNodeVolumeToMesh")
    v2m.inputs["Threshold"].default_value = 0.5
    out = ng.nodes.new("NodeGroupOutput")
    ng.links.new(info.outputs["Geometry"], v2m.inputs["Volume"])
    ng.links.new(v2m.outputs["Mesh"], out.inputs[0])
    mod = holder.modifiers.new("v2m", "NODES")
    mod.node_group = ng
    dg = bpy.context.evaluated_depsgraph_get()
    me = bpy.data.meshes.new_from_object(holder.evaluated_get(dg))
    holder.modifiers.clear()
    old = holder.data
    holder.data = me
    bpy.data.meshes.remove(old)
    volume = vol.data
    bpy.data.objects.remove(vol, do_unlink=True)
    bpy.data.volumes.remove(volume)
    bpy.data.node_groups.remove(ng)
    os.remove(path)
    # Grid index space to millimetres.
    holder.data.transform(Matrix.Translation(Vector(lo)))
    me.name = name
    return holder


def knight(mat, voxel=0.12):
    head = sdf_to_mesh("knight_head", knight_head_sdf, (-13.0, -12.5, 18.0), (25.0, 12.5, 62.0), voxel)
    # Relax the voxel stair-steps, then reduce to a sensible triangle count.
    sm = head.modifiers.new("smooth", "CORRECTIVE_SMOOTH")
    sm.iterations = 6
    sm.smooth_type = "LENGTH_WEIGHTED"
    sm.use_only_smooth = True
    dec = head.modifiers.new("dec", "DECIMATE")
    dec.ratio = 0.045
    bc.apply_modifiers(head)
    head.data.materials.clear()
    head.data.materials.append(mat)
    for p in head.data.polygons:
        p.material_index = 0
    # Grain runs up the neck: project from the side.
    me = head.data
    me.uv_layers.new(name="UVMap")
    uv = me.uv_layers.active.data
    for li, loop in enumerate(me.loops):
        co = me.vertices[loop.vertex_index].co
        uv[li].uv = ((co.x + co.y * 0.35) / 40.0, co.z / 80.0)
    base = bc.lathe("knight_base", knight_base_profile(), material=mat)
    obj = bc.join([base, head], "knight")
    return obj


# ----------------------------------------------------------------------------

def materials():
    ivory = bc.pbr("Boxwood", tex="boxwood", normal_strength=0.35)
    ebony = bc.pbr("Ebony", tex="ebony", normal_strength=0.3)
    return ivory, ebony


def build(mat, kinds=("king", "queen", "bishop", "knight", "rook", "pawn")):
    """Meshes in metres, origin at the centre of the base, standing on z = 0."""
    makers = {"king": king, "queen": queen, "bishop": bishop, "knight": knight,
              "rook": rook, "pawn": pawn}
    out = {}
    for kind in kinds:
        obj = makers[kind](mat)
        bc.apply_scale(obj, MM)
        bc.shade_smooth(obj, 40.0 if kind != "knight" else 60.0)
        # The knight looks along +X as modelled; turn it to face the far side (+Y).
        if kind == "knight":
            obj.data.transform(Matrix.Rotation(math.radians(90), 4, "Z"))
        obj.name = kind
        obj.data.name = kind
        out[kind] = obj
    return out
