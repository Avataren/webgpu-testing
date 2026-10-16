"""Build the playground robot: a rigged, animated low-poly character (run inside Blender).

    blender -b --python games/character-playground/tools/robot.py [-- --preview]

Writes ../content/gltf/robot/robot.glb (where the editor imported it, so a rebuild
updates the game): one skinned mesh with several materials on a
humanoid armature with Mixamo-style bone names (Hips, Spine, Spine1, Neck, Head,
LeftShoulder, LeftArm, LeftForeArm, LeftHand, LeftUpLeg, LeftLeg, LeftFoot,
LeftToeBase and the Right* mirror), and in-place clips: Idle, Walk, Run, StrafeLeft,
StrafeRight, Jump, Fall and Land.

Blender is Z-up and the robot faces -Y; the glTF exporter turns that into the
engine's Y-up with the robot facing +Z, the way CharacterController expects models
to face. Its feet stand at the origin and it is 1.8 m tall.

The body is a skin-modifier mannequin weighted to the bones by Blender's bone heat;
the armour (helmet, visor, chest plate, pads, boots, gloves) rides rigidly on single
bones. The clips are keyed procedurally from a few sine and bump curves per joint.
"""

import math
import os
import sys

import bmesh
import bpy
from mathutils import Matrix, Quaternion, Vector

HERE = os.path.dirname(os.path.abspath(__file__))
OUT = os.path.normpath(os.path.join(HERE, "..", "content", "gltf", "robot", "robot.glb"))
PREVIEW_DIR = os.path.join(HERE, "build")
FPS = 30

X = Vector((1, 0, 0))
Y = Vector((0, 1, 0))
Z = Vector((0, 0, 1))
SIDES = (("Left", 1.0), ("Right", -1.0))
# The upper arm leaves the shoulder this far below horizontal (an A pose).
ARM_DROP = 40.0


# ----------------------------------------------------------------------------
# Scene and armature
# ----------------------------------------------------------------------------

def reset():
    bpy.ops.wm.read_homefile(use_empty=True, use_factory_startup=True)
    for collection in (bpy.data.meshes, bpy.data.materials, bpy.data.armatures,
                       bpy.data.actions, bpy.data.objects):
        for block in list(collection):
            collection.remove(block)
    scene = bpy.context.scene
    scene.render.fps = FPS
    scene.frame_start = 0
    scene.unit_settings.system = "METRIC"


def link(obj):
    bpy.context.scene.collection.objects.link(obj)
    return obj


def arm_direction(side):
    drop = math.radians(ARM_DROP)
    return Vector((math.cos(drop) * side, 0.0, -math.sin(drop)))


def bone_layout():
    """(name, head, tail, parent, connected) in armature space (metres)."""
    bones = [
        ("Hips", (0, 0, 0.95), (0, 0, 1.06), None, False),
        ("Spine", (0, 0, 1.06), (0, 0, 1.23), "Hips", True),
        ("Spine1", (0, 0, 1.23), (0, 0, 1.43), "Spine", True),
        ("Neck", (0, 0, 1.43), (0, 0, 1.53), "Spine1", True),
        ("Head", (0, 0, 1.53), (0, 0, 1.80), "Neck", True),
    ]
    for prefix, s in SIDES:
        d = arm_direction(s)
        shoulder = Vector((0.17 * s, 0, 1.38))
        elbow = shoulder + d * 0.28
        wrist = elbow + d * 0.26
        hand = wrist + d * 0.10
        bones += [
            (f"{prefix}Shoulder", (0.035 * s, 0, 1.39), shoulder, "Spine1", False),
            (f"{prefix}Arm", shoulder, elbow, f"{prefix}Shoulder", True),
            (f"{prefix}ForeArm", elbow, wrist, f"{prefix}Arm", True),
            (f"{prefix}Hand", wrist, hand, f"{prefix}ForeArm", True),
            (f"{prefix}UpLeg", (0.1 * s, 0, 0.93), (0.1 * s, -0.012, 0.52), "Hips", False),
            (f"{prefix}Leg", (0.1 * s, -0.012, 0.52), (0.1 * s, 0.0, 0.1), f"{prefix}UpLeg", True),
            (f"{prefix}Foot", (0.1 * s, 0.0, 0.1), (0.1 * s, -0.12, 0.035), f"{prefix}Leg", True),
            (f"{prefix}ToeBase", (0.1 * s, -0.12, 0.035), (0.1 * s, -0.2, 0.035), f"{prefix}Foot", True),
        ]
    return bones


def build_armature():
    data = bpy.data.armatures.new("RobotRig")
    rig = link(bpy.data.objects.new("Robot", data))
    bpy.context.view_layer.objects.active = rig
    rig.select_set(True)
    bpy.ops.object.mode_set(mode="EDIT")
    for name, head, tail, parent, connected in bone_layout():
        bone = data.edit_bones.new(name)
        bone.head = Vector(head)
        bone.tail = Vector(tail)
        bone.roll = 0.0
        if parent:
            bone.parent = data.edit_bones[parent]
            bone.use_connect = connected
    bpy.ops.object.mode_set(mode="OBJECT")
    data.display_type = "STICK"
    return rig


# ----------------------------------------------------------------------------
# Materials
# ----------------------------------------------------------------------------

def material(name, color, roughness, metallic=0.0, emission=None, strength=0.0):
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes.get("Principled BSDF")
    bsdf.inputs["Base Color"].default_value = (*color, 1.0)
    bsdf.inputs["Roughness"].default_value = roughness
    bsdf.inputs["Metallic"].default_value = metallic
    if emission is not None:
        bsdf.inputs["Emission Color"].default_value = (*emission, 1.0)
        bsdf.inputs["Emission Strength"].default_value = strength
    mat.diffuse_color = (*color, 1.0)
    return mat


def materials():
    return {
        "suit": material("RobotSuit", (0.075, 0.08, 0.09), 0.62),
        "armor": material("RobotArmor", (0.86, 0.87, 0.89), 0.32),
        "accent": material("RobotAccent", (1.0, 0.38, 0.06), 0.4),
        "metal": material("RobotMetal", (0.32, 0.33, 0.35), 0.35, metallic=1.0),
        "visor": material("RobotVisor", (0.015, 0.02, 0.03), 0.06),
        "eyes": material("RobotEyes", (0.1, 0.75, 1.0), 0.3, emission=(0.2, 0.85, 1.0), strength=6.0),
    }


# ----------------------------------------------------------------------------
# Meshes
# ----------------------------------------------------------------------------

def object_from_bmesh(name, bm, mat):
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    me.materials.append(mat)
    for poly in me.polygons:
        poly.use_smooth = True
    return link(bpy.data.objects.new(name, me))


def bake_modifiers(obj):
    """Replace the object's mesh with its evaluated one (modifiers applied)."""
    depsgraph = bpy.context.evaluated_depsgraph_get()
    evaluated = obj.evaluated_get(depsgraph)
    mesh = bpy.data.meshes.new_from_object(evaluated)
    old = obj.data
    obj.modifiers.clear()
    obj.data = mesh
    bpy.data.meshes.remove(old)
    return obj


def skin_body(mat):
    """A smooth mannequin from a vertex skeleton through the Skin modifier."""
    verts, radii, edges = [], [], []

    def v(position, radius):
        verts.append(Vector(position))
        radii.append(radius)
        return len(verts) - 1

    def chain(*points):
        for a, b in zip(points, points[1:]):
            edges.append((a, b))

    pelvis = v((0, 0.0, 0.96), (0.17, 0.12))
    waist = v((0, 0.0, 1.08), (0.15, 0.11))
    chest = v((0, -0.005, 1.25), (0.19, 0.13))
    upper_chest = v((0, 0.0, 1.38), (0.18, 0.12))
    neck = v((0, 0.0, 1.5), (0.065, 0.065))
    neck_top = v((0, 0.0, 1.6), (0.07, 0.07))
    chain(pelvis, waist, chest, upper_chest, neck, neck_top)
    for _, s in SIDES:
        d = arm_direction(s)
        shoulder = Vector((0.17 * s, 0, 1.38))
        clavicle = v((0.09 * s, 0, 1.39), (0.095, 0.09))
        joint = v(shoulder, (0.088, 0.088))
        elbow = v(shoulder + d * 0.28, (0.068, 0.068))
        wrist = v(shoulder + d * 0.54, (0.055, 0.055))
        hand = v(shoulder + d * 0.62, (0.05, 0.05))
        chain(upper_chest, clavicle, joint, elbow, wrist, hand)
        hip = v((0.1 * s, 0, 0.92), (0.115, 0.115))
        knee = v((0.1 * s, -0.012, 0.52), (0.085, 0.085))
        ankle = v((0.1 * s, 0.0, 0.11), (0.062, 0.062))
        toe = v((0.1 * s, -0.11, 0.05), (0.055, 0.045))
        chain(pelvis, hip, knee, ankle, toe)

    me = bpy.data.meshes.new("RobotBody")
    me.from_pydata([tuple(p) for p in verts], edges, [])
    body = link(bpy.data.objects.new("RobotBody", me))
    skin = body.modifiers.new("Skin", "SKIN")
    skin.branch_smoothing = 0.6
    skin.use_smooth_shade = True
    layer = me.skin_vertices[0].data
    for index, (rx, ry) in enumerate(radii):
        layer[index].radius = (rx, ry)
    layer[pelvis].use_root = True
    subsurf = body.modifiers.new("Subsurf", "SUBSURF")
    subsurf.levels = 2
    bake_modifiers(body)
    body.data.materials.append(mat)
    for poly in body.data.polygons:
        poly.use_smooth = True
    return body


def sphere(name, center, radius, scale, mat, segments=24, rings=14):
    bm = bmesh.new()
    bmesh.ops.create_uvsphere(bm, u_segments=segments, v_segments=rings, radius=radius)
    bmesh.ops.scale(bm, vec=Vector(scale), verts=bm.verts)
    bmesh.ops.translate(bm, vec=Vector(center), verts=bm.verts)
    return object_from_bmesh(name, bm, mat)


def rounded_box(name, center, size, bevel, mat):
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=Vector(size), verts=bm.verts)
    bmesh.ops.bevel(bm, geom=list(bm.edges) + list(bm.verts), offset=bevel, segments=3,
                    profile=0.5, affect="EDGES")
    bmesh.ops.translate(bm, vec=Vector(center), verts=bm.verts)
    return object_from_bmesh(name, bm, mat)


def torus(name, center, major, minor, scale, mat):
    bm = bmesh.new()
    segments, sides = 32, 10
    grid = []
    for i in range(segments):
        a = 2 * math.pi * i / segments
        ring = []
        for j in range(sides):
            b = 2 * math.pi * j / sides
            r = major + minor * math.cos(b)
            ring.append(bm.verts.new((r * math.cos(a), r * math.sin(a), minor * math.sin(b))))
        grid.append(ring)
    for i in range(segments):
        for j in range(sides):
            a, b = grid[i][j], grid[(i + 1) % segments][j]
            c, d = grid[(i + 1) % segments][(j + 1) % sides], grid[i][(j + 1) % sides]
            bm.faces.new((a, b, c, d))
    bmesh.ops.scale(bm, vec=Vector(scale), verts=bm.verts)
    bmesh.ops.translate(bm, vec=Vector(center), verts=bm.verts)
    return object_from_bmesh(name, bm, mat)


def visor(mat):
    """The front band of a sphere just outside the helmet, given some thickness."""
    bm = bmesh.new()
    bmesh.ops.create_uvsphere(bm, u_segments=48, v_segments=24, radius=0.153)
    bmesh.ops.scale(bm, vec=Vector((1.0, 1.04, 1.06)), verts=bm.verts)
    keep = [v for v in bm.verts if v.co.y < -0.035 and -0.045 < v.co.z < 0.04]
    bmesh.ops.delete(bm, geom=[v for v in bm.verts if v not in set(keep)], context="VERTS")
    bmesh.ops.translate(bm, vec=Vector((0, -0.006, 1.67)), verts=bm.verts)
    obj = object_from_bmesh("RobotVisor", bm, mat)
    solid = obj.modifiers.new("Solidify", "SOLIDIFY")
    solid.thickness = 0.01
    solid.offset = 1.0
    return bake_modifiers(obj)


def rigid(obj, bone):
    group = obj.vertex_groups.new(name=bone)
    group.add(range(len(obj.data.vertices)), 1.0, "REPLACE")
    return obj


def armour(mats):
    pieces = [
        rigid(sphere("Helmet", (0, 0.004, 1.67), 0.15, (1.0, 1.04, 1.1), mats["armor"]), "Head"),
        rigid(visor(mats["visor"]), "Head"),
        rigid(rounded_box("ChestPlate", (0, -0.065, 1.3), (0.36, 0.14, 0.27), 0.045, mats["accent"]), "Spine1"),
        rigid(rounded_box("Backpack", (0, 0.16, 1.29), (0.26, 0.11, 0.3), 0.04, mats["armor"]), "Spine1"),
        rigid(torus("Belt", (0, 0, 0.99), 0.165, 0.032, (1.0, 0.76, 1.0), mats["metal"]), "Hips"),
    ]
    for prefix, s in SIDES:
        d = arm_direction(s)
        shoulder = Vector((0.17 * s, 0, 1.38))
        pieces += [
            rigid(sphere(f"{prefix}Eye", (0.045 * s, -0.159, 1.675), 0.016, (1.0, 0.5, 1.5), mats["eyes"], 12, 8), "Head"),
            rigid(sphere(f"{prefix}Pad", shoulder + Vector((0.025 * s, 0, 0.03)), 0.11, (1.0, 1.0, 0.75), mats["accent"]), f"{prefix}Arm"),
            rigid(sphere(f"{prefix}Glove", shoulder + d * 0.61, 0.07, (1.0, 1.0, 1.0), mats["suit"]), f"{prefix}Hand"),
            rigid(rounded_box(f"{prefix}Boot", (0.1 * s, -0.05, 0.065), (0.15, 0.27, 0.13), 0.04, mats["armor"]), f"{prefix}Foot"),
            rigid(rounded_box(f"{prefix}Sole", (0.1 * s, -0.05, 0.012), (0.16, 0.28, 0.025), 0.01, mats["accent"]), f"{prefix}Foot"),
            rigid(sphere(f"{prefix}KneePad", (0.1 * s, -0.075, 0.53), 0.07, (1.0, 0.7, 1.1), mats["armor"]), f"{prefix}Leg"),
        ]
    return pieces


def build_mesh(rig, mats):
    body = skin_body(mats["suit"])
    bpy.ops.object.select_all(action="DESELECT")
    body.select_set(True)
    rig.select_set(True)
    bpy.context.view_layer.objects.active = rig
    bpy.ops.object.parent_set(type="ARMATURE_AUTO")
    weighted = sum(1 for g in body.vertex_groups)
    if weighted == 0:
        raise RuntimeError("bone heat weighting gave the body no vertex groups")

    pieces = armour(mats)
    bpy.ops.object.select_all(action="DESELECT")
    for piece in pieces:
        piece.select_set(True)
    body.select_set(True)
    bpy.context.view_layer.objects.active = body
    bpy.ops.object.join()
    body.name = "RobotMesh"
    body.data.name = "RobotMesh"
    return body


# ----------------------------------------------------------------------------
# Animation
# ----------------------------------------------------------------------------

def bump(phase, center, width):
    """A periodic Gaussian bump in phase (0..1), 1 at `center`."""
    d = (phase - center + 0.5) % 1.0 - 0.5
    return math.exp(-(d / width) ** 2)


def smooth(t):
    t = max(0.0, min(1.0, t))
    return t * t * (3.0 - 2.0 * t)


def rot(axis, degrees):
    return Quaternion(axis, math.radians(degrees))


def elbow_axis(side):
    """The hinge that swings the forearm forward for positive angles."""
    drop = math.radians(ARM_DROP)
    return Vector((-math.sin(drop), 0.0, -math.cos(drop) * side))


class Pose:
    """Rotations about armature axes per bone (applied after the parent's), and the
    hips' offset. Angles in degrees: thighs swing forward and spines lean forward for
    negative and positive X respectively; knees bend and toes point down for positive
    X."""

    def __init__(self):
        self.rot = {}
        self.hips = Vector((0, 0, 0))

    def add(self, bone, axis, degrees):
        self.rot[bone] = rot(axis, degrees) @ self.rot.get(bone, Quaternion())
        return self

    def arms(self, down, swing_l, swing_r, elbow_l, elbow_r):
        """Lower the arms from the A pose by `down`, swing them forward (negative) or
        back, and bend the elbows forward."""
        for prefix, s, swing, elbow in (("Left", 1, swing_l, elbow_l), ("Right", -1, swing_r, elbow_r)):
            self.add(f"{prefix}Arm", Y, down * s)
            self.add(f"{prefix}Arm", X, swing)
            self.add(f"{prefix}ForeArm", elbow_axis(s), elbow)
        return self

    def leg(self, prefix, thigh, knee, ankle, spread=0.0):
        side = 1 if prefix == "Left" else -1
        self.add(f"{prefix}UpLeg", Y, -spread * side)
        self.add(f"{prefix}UpLeg", X, thigh)
        self.add(f"{prefix}Leg", X, knee)
        self.add(f"{prefix}Foot", X, ankle)
        return self


THIGH, SHIN = 0.41, 0.42


def crouch_drop(thigh, knee):
    """How far the hips sink for the feet to stay down with this leg bend."""
    upright = THIGH + SHIN
    bent = THIGH * math.cos(math.radians(thigh)) + SHIN * math.cos(math.radians(thigh + knee))
    return upright - bent


def idle(t):
    w = 2 * math.pi * t
    breath = math.sin(w)
    p = Pose()
    p.hips = Vector((0, 0, -0.012 - 0.004 * breath))
    p.add("Hips", Y, 1.2 * math.sin(w))
    p.add("Spine", X, 1.0)
    p.add("Spine1", X, -1.8 * breath)
    p.add("Neck", X, 1.0 * breath)
    p.add("Head", Z, 6.0 * math.sin(w))
    p.add("Head", X, 2.0 * math.sin(2 * w))
    for prefix, s in SIDES:
        p.add(f"{prefix}Shoulder", Y, -1.5 * breath * s)
        p.leg(prefix, -4.0, 8.0, -4.0, spread=1.5)
    p.arms(32.0 + 1.5 * breath, 4.0, 4.0, 12.0 + 3.0 * breath, 12.0 + 3.0 * breath)
    return p


def gait(t, run):
    """Walk and run cycles; the left foot strikes at t = 0, the right at t = 0.5."""
    w = 2 * math.pi * t
    p = Pose()
    if run:
        swing, bias, knee_peak, knee_stance, lean = 46.0, -10.0, 105.0, 24.0, 11.0
        arm_swing, arm_bias, elbow, bob = 42.0, -8.0, 88.0, 0.045
        p.hips = Vector((0, 0, -0.05 - bob * math.cos(4 * math.pi * (t - 0.12))))
    else:
        swing, bias, knee_peak, knee_stance, lean = 25.0, 0.0, 62.0, 10.0, 3.0
        arm_swing, arm_bias, elbow, bob = 17.0, 0.0, 16.0, 0.022
        p.hips = Vector((0, 0, -0.015 - bob * math.cos(4 * math.pi * t)))

    for prefix, phase_offset in (("Left", 0.0), ("Right", 0.5)):
        phase = (t + phase_offset) % 1.0
        pw = 2 * math.pi * phase
        thigh = bias - swing * math.cos(pw)
        swing_phase = 0.4 if run else 0.55
        if phase > swing_phase:
            lift = math.sin(math.pi * (phase - swing_phase) / (1.0 - swing_phase))
        else:
            lift = 0.0
        knee = 4.0 + knee_stance * bump(phase, 0.12, 0.1) + knee_peak * max(0.0, lift) ** 1.1
        toe_off = (28.0 if run else 18.0) * bump(phase, swing_phase, 0.06)
        heel = (-12.0 if not run else -6.0) * bump(phase, 0.0, 0.06)
        ankle = -(thigh + knee) * (0.85 if phase < swing_phase else 0.55) + toe_off + heel
        p.leg(prefix, thigh, knee, ankle)

    p.add("Hips", Z, (10.0 if run else 6.0) * math.cos(w))
    p.add("Hips", Y, (2.0 if run else 3.0) * math.sin(w))
    p.add("Hips", X, lean * 0.4)
    p.add("Spine", Z, -(5.0 if run else 3.5) * math.cos(w))
    p.add("Spine", X, lean * 0.4)
    p.add("Spine1", Z, -(6.0 if run else 4.0) * math.cos(w))
    p.add("Spine1", X, lean * 0.4 + 1.5 * math.cos(4 * math.pi * t))
    p.add("Head", X, -lean * 0.8)
    p.add("Head", Z, (4.0 if run else 2.0) * math.cos(w))
    # Arms swing against the legs: the left arm is back while the left leg is forward.
    left = arm_bias + arm_swing * math.cos(w)
    right = arm_bias - arm_swing * math.cos(w)
    forward_l = max(0.0, -math.cos(w))
    forward_r = max(0.0, math.cos(w))
    down = 20.0 if run else 30.0
    p.arms(down, left, right, elbow + 14.0 * forward_l, elbow + 14.0 * forward_r)
    return p


def strafe(t, side):
    """A sideways gallop: the leading leg steps out toward `side` (1 = the robot's right,
    -1 its left) and the trailing leg follows it, without the legs crossing."""
    w = 2 * math.pi * t
    lead, trail = ("Right", "Left") if side > 0 else ("Left", "Right")
    reach = 0.36
    p = Pose()
    p.hips = Vector((0, 0, -0.035 - 0.03 * math.cos(2 * w)))
    # The trailing leg starts out to its own side, so the feet never meet.
    for prefix, amount, base, lag in ((lead, 1.0, 0.0, 0.0), (trail, 0.6, -0.1, 0.33)):
        phase = (t - lag) % 1.0
        pw = 2 * math.pi * phase
        out = base + reach * amount * (0.5 + 0.5 * math.cos(pw))
        # Positive rotation about Y swings a hanging leg toward the robot's right (-X).
        sideways = math.degrees(math.asin(max(-0.95, min(0.95, out / (THIGH + SHIN))))) * side
        if prefix == lead:
            sideways -= 4.0 * side
        lift = max(0.0, -math.sin(pw))
        knee = 8.0 + 46.0 * lift
        thigh = -6.0 - 0.45 * knee
        p.add(f"{prefix}UpLeg", Y, sideways)
        p.add(f"{prefix}UpLeg", X, thigh)
        p.add(f"{prefix}Leg", X, knee)
        p.add(f"{prefix}Foot", X, -(thigh + knee) * 0.8)
    # Lean into the step, head level.
    p.add("Hips", Y, -3.0 * side)
    p.add("Spine", Y, -3.0 * side)
    p.add("Spine", X, 5.0)
    p.add("Spine1", Y, -2.0 * side + 1.5 * math.sin(2 * w))
    p.add("Head", Y, 7.0 * side)
    p.arms(24.0, -8.0 + 10.0 * math.cos(w), -8.0 - 10.0 * math.cos(w), 32.0, 32.0)
    return p


def airborne(t):
    """The tucked pose in the air, gently flailing (loops)."""
    w = 2 * math.pi * t
    p = Pose()
    p.hips = Vector((0, 0, 0.0))
    p.leg("Left", -38.0 + 6.0 * math.sin(w), 62.0 + 8.0 * math.sin(w), 18.0)
    p.leg("Right", -8.0 - 6.0 * math.sin(w), 34.0 - 6.0 * math.sin(w), 22.0)
    p.add("Spine", X, -4.0)
    p.add("Spine1", X, 2.0 * math.sin(w))
    p.add("Head", X, 6.0)
    for prefix, s in SIDES:
        p.add(f"{prefix}Arm", Y, (-48.0 + 8.0 * math.sin(w + s)) * s)
        p.add(f"{prefix}Arm", X, -18.0 + 10.0 * math.sin(w * s))
        p.add(f"{prefix}ForeArm", elbow_axis(s), 28.0 + 8.0 * math.cos(w))
    return p


def crouch(depth):
    """A landing or take-off crouch; `depth` 1 is the deepest."""
    thigh, knee = -42.0 * depth, 84.0 * depth
    p = Pose()
    p.hips = Vector((0, -0.04 * depth, -crouch_drop(thigh, knee)))
    for prefix, _ in SIDES:
        p.leg(prefix, thigh, knee, -(thigh + knee), spread=2.0)
    p.add("Spine", X, 14.0 * depth)
    p.add("Spine1", X, 6.0 * depth)
    p.add("Head", X, -14.0 * depth)
    p.arms(18.0, -24.0 * depth, -24.0 * depth, 34.0 * depth, 34.0 * depth)
    return p


def blend(a, b, t):
    p = Pose()
    for bone in set(a.rot) | set(b.rot):
        p.rot[bone] = a.rot.get(bone, Quaternion()).slerp(b.rot.get(bone, Quaternion()), t)
    p.hips = a.hips.lerp(b.hips, t)
    return p


def jump(t):
    """Push off from a shallow crouch, arms swinging up, into the air pose."""
    push = crouch(0.35)
    push.add("LeftFoot", X, 25.0).add("RightFoot", X, 25.0)
    return blend(push, airborne(0.0), smooth(t))


def land(t):
    """Absorb the impact in a crouch, then rise back to the idle stance."""
    if t < 0.25:
        return blend(airborne(0.0), crouch(1.0), smooth(t / 0.25))
    return blend(crouch(1.0), idle(0.0), smooth((t - 0.25) / 0.75))


CLIPS = [
    # name, frames, pose(t), loops
    ("Idle", 90, idle, True),
    ("Walk", 32, lambda t: gait(t, False), True),
    ("Run", 20, lambda t: gait(t, True), True),
    ("StrafeLeft", 18, lambda t: strafe(t, -1), True),
    ("StrafeRight", 18, lambda t: strafe(t, 1), True),
    ("Jump", 9, jump, False),
    ("Fall", 30, airborne, True),
    ("Land", 14, land, False),
]


def key_clip(rig, name, frames, pose_fn):
    action = bpy.data.actions.new(name)
    action.use_fake_user = True
    rig.animation_data_create()
    rig.animation_data.action = action
    rest = {b.name: b.matrix_local.to_3x3().to_quaternion() for b in rig.data.bones}
    hips_rest = rig.data.bones["Hips"].matrix_local.to_3x3()
    for frame in range(frames + 1):
        pose = pose_fn(frame / frames)
        for bone in rig.pose.bones:
            bone.rotation_mode = "QUATERNION"
            r = rest[bone.name]
            delta = pose.rot.get(bone.name, Quaternion())
            bone.rotation_quaternion = r.inverted() @ delta @ r
            bone.keyframe_insert("rotation_quaternion", frame=frame)
            if bone.name == "Hips":
                bone.location = hips_rest.inverted() @ pose.hips
                bone.keyframe_insert("location", frame=frame)
            else:
                bone.location = (0, 0, 0)
    action.use_frame_range = True
    action.frame_range = (0, frames)
    return action


def animate(rig):
    actions = [key_clip(rig, name, frames, fn) for name, frames, fn, _ in CLIPS]
    rig.animation_data.action = None
    for bone in rig.pose.bones:
        bone.rotation_quaternion = Quaternion()
        bone.location = (0, 0, 0)
    return actions


# ----------------------------------------------------------------------------
# Output
# ----------------------------------------------------------------------------

def export(rig, mesh):
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    bpy.ops.object.select_all(action="DESELECT")
    rig.select_set(True)
    mesh.select_set(True)
    bpy.context.view_layer.objects.active = rig
    bpy.ops.export_scene.gltf(
        filepath=OUT,
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_apply=False,
        export_skins=True,
        export_animations=True,
        export_animation_mode="ACTIONS",
        export_force_sampling=True,
        export_def_bones=False,
        export_reset_pose_bones=True,
        export_cameras=False,
        export_lights=False,
    )
    return OUT


def preview(rig, poses):
    """Workbench renders of the robot in a few clip poses, for checking by eye."""
    os.makedirs(PREVIEW_DIR, exist_ok=True)
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.display.shading.light = "STUDIO"
    scene.display.shading.color_type = "MATERIAL"
    scene.render.resolution_x, scene.render.resolution_y = 640, 720
    camera = link(bpy.data.objects.new("PreviewCamera", bpy.data.cameras.new("PreviewCamera")))
    scene.camera = camera
    target = Vector((0, 0, 0.9))
    camera.data.lens = 50
    outputs = []
    for label, action_name, frame, azimuth in poses:
        rig.animation_data.action = bpy.data.actions[action_name]
        scene.frame_set(frame)
        a = math.radians(azimuth)
        camera.location = target + Vector((math.sin(a) * -4.2, -math.cos(a) * 4.2, 0.6))
        direction = target - camera.location
        camera.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()
        path = os.path.join(PREVIEW_DIR, f"robot_{label}.png")
        scene.render.filepath = path
        bpy.ops.render.render(write_still=True)
        outputs.append(path)
    rig.animation_data.action = None
    return outputs


def build(with_preview=False):
    reset()
    rig = build_armature()
    mats = materials()
    mesh = build_mesh(rig, mats)
    actions = animate(rig)
    path = export(rig, mesh)
    report = {
        "glb": path,
        "vertices": len(mesh.data.vertices),
        "bones": len(rig.data.bones),
        "clips": [a.name for a in actions],
        "groups": sorted(g.name for g in mesh.vertex_groups),
    }
    if with_preview:
        report["previews"] = preview(rig, [
            ("rest_front", "Idle", 0, 0),
            ("walk_side", "Walk", 0, 90),
            ("run_side", "Run", 5, 90),
            ("strafe_front", "StrafeRight", 13, 0),
            ("jump_threequarter", "Fall", 0, 35),
            ("land_side", "Land", 3, 90),
        ])
    return report


if __name__ == "__main__":
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    print(build(with_preview="--preview" in argv))
