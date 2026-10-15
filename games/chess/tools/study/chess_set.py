"""The board with coordinates, 32 pieces, move markers and capture anchors.

Exported as chess_set.glb. The engine's chess spectator finds pieces by name
(Chess.<White|Black>.<kind>.<square>), works out the square size from the rooks
and the board centre from all pieces, so the set can sit anywhere in the room.
White is at Blender -Y (engine +Z).
"""

import math

import bmesh
import bpy
from mathutils import Matrix, Vector

import blender_common as bc
import pieces

SQUARE = 0.055
BOARD = 0.52
THICK = 0.02
BACK_RANK = ["rook", "knight", "bishop", "queen", "king", "bishop", "knight", "rook"]


def square_xy(file, rank):
    return ((file - 3.5) * SQUARE, (rank - 3.5) * SQUARE)


def board_mesh(top_mat, side_mat, felt_mat):
    """A slab whose top shows the whole board texture, with a small bevel on the top
    edge; sides reuse the outer frame of the texture so the grain carries round."""
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("UVMap")
    h = BOARD / 2
    bev = 0.0015
    # Top face (inset by the bevel), bevel ring, sides, bottom.
    ring_top = [(-h + bev, -h + bev), (h - bev, -h + bev), (h - bev, h - bev), (-h + bev, h - bev)]
    ring_mid = [(-h, -h), (h, -h), (h, h), (-h, h)]
    vt = [bm.verts.new((x, y, THICK)) for x, y in ring_top]
    vb = [bm.verts.new((x, y, THICK - bev)) for x, y in ring_mid]
    vf = [bm.verts.new((x, y, 0.0)) for x, y in ring_mid]

    def uv_top(co):
        return ((co[0] + h) / BOARD, (co[1] + h) / BOARD)

    f = bm.faces.new(vt)
    f.material_index = 0
    for loop in f.loops:
        loop[uv].uv = uv_top(loop.vert.co)
    for i in range(4):
        j = (i + 1) % 4
        f = bm.faces.new((vb[i], vb[j], vt[j], vt[i]))
        f.material_index = 0
        for loop in f.loops:
            loop[uv].uv = uv_top(loop.vert.co)
        f = bm.faces.new((vf[i], vf[j], vb[j], vb[i]))
        f.material_index = 1
        # Sides: a strip from the outer border of the texture, grain along the edge.
        for loop in f.loops:
            co = loop.vert.co
            along = co[0] if i in (0, 2) else co[1]
            loop[uv].uv = ((along + h) / BOARD, 0.004 + co[2] / BOARD * 0.6) if i in (0, 2) else \
                (0.004 + co[2] / BOARD * 0.6, (along + h) / BOARD)
    f = bm.faces.new(list(reversed(vf)))
    f.material_index = 2
    for loop in f.loops:
        loop[uv].uv = uv_top(loop.vert.co)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    obj = bc.mesh_object("Board", bm)
    for m in (top_mat, side_mat, felt_mat):
        obj.data.materials.append(m)
    bc.shade_smooth(obj, 30.0)
    return obj


def marker_bar(name, length, along_x, mat, parent):
    w, t = 0.0013, 0.0005
    size = (length, w, t) if along_x else (w, length, t)
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=Vector(size), verts=bm.verts)
    obj = bc.mesh_object(name, bm, mat, parent)
    obj.location = (0, 0, THICK + t / 2)
    return obj


def build(export=True, preview=False):
    bc.reset()
    root = bc.empty("ChessSet")
    board_top = bc.pbr("Board", tex="board", normal_strength=0.4)
    board_side = bc.pbr("BoardSide", tex="board", normal_strength=0.4)
    felt = bc.pbr("Felt", color=(0.03, 0.08, 0.05), roughness=1.0)
    board = board_mesh(board_top, board_side, felt)
    board.parent = root

    boxwood, ebony = pieces.materials()
    proto_white = pieces.build(boxwood)
    proto_black = {}
    for kind, obj in proto_white.items():
        # Same geometry, ebony material: a linked copy with its own mesh data so the
        # glTF gets one mesh per colour.
        me = obj.data.copy()
        me.materials.clear()
        me.materials.append(ebony)
        me.name = f"{kind}_black"
        proto_black[kind] = me
    for obj in proto_white.values():
        obj.data.name = f"{obj.data.name}_white"

    def place(kind, white, file, rank):
        me = proto_white[kind].data if white else proto_black[kind]
        sq = "abcdefgh"[file] + str(rank + 1)
        obj = bpy.data.objects.new(f"Chess.{'White' if white else 'Black'}.{kind}.{sq}", me)
        bc.link(obj, root)
        x, y = square_xy(file, rank)
        obj.location = (x, y, THICK)
        if not white:
            obj.rotation_euler = (0, 0, math.pi)
        return obj

    for file in range(8):
        place(BACK_RANK[file], True, file, 0)
        place("pawn", True, file, 1)
        place("pawn", False, file, 6)
        place(BACK_RANK[file], False, file, 7)
    for obj in proto_white.values():
        bpy.data.objects.remove(obj, do_unlink=True)

    # Last-move markers: thin glowing brass lines on the edges of the from/to squares.
    # 0/1 and 4/5 lie along files (X), 2/3 and 6/7 along ranks (Y).
    glow = bc.pbr("MoveMarker", color=(1.0, 0.72, 0.32), roughness=0.3, metallic=0.0,
                  emission=(1.0, 0.68, 0.3), emission_strength=4.0)
    for i in range(8):
        marker_bar(f"Move.Inlay.{i}", SQUARE * 0.92, i % 4 < 2, glow, root)

    # Captured pieces stand in a grid off the h-file side (the clock stands on the
    # a-file side), ivory toward black's end and ebony toward white's; the spectator
    # steps away from the board.
    bc.empty("Chess.Captures.White", (0.30, 0.05, 0.0), root)
    bc.empty("Chess.Captures.Black", (0.30, -0.05, 0.0), root)

    if preview:
        return root
    if export:
        return bc.export_glb("chess_set")
    return root
