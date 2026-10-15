"""Shared helpers for the chess study Blender scripts (run inside Blender).

Geometry is authored in millimetres for the small props and scaled to metres on
export. Blender is Z-up; the glTF exporter converts to the engine's Y-up, so
Blender -Y (the near side) becomes engine +Z.
"""

import math
import os

import bmesh
import bpy
from mathutils import Matrix, Vector

HERE = os.path.dirname(os.path.abspath(__file__))
TEX = os.path.join(HERE, "build", "textures")
OUT = os.path.join(HERE, "build", "glb")
PREVIEW = os.path.join(HERE, "build", "preview")


def reset():
    bpy.ops.wm.read_homefile(use_empty=True, use_factory_startup=True)
    for coll in (bpy.data.meshes, bpy.data.materials, bpy.data.images, bpy.data.curves):
        for block in list(coll):
            coll.remove(block)


def link(obj, parent=None):
    bpy.context.scene.collection.objects.link(obj)
    if parent is not None:
        obj.parent = parent
    return obj


def empty(name, location=(0, 0, 0), parent=None):
    obj = bpy.data.objects.new(name, None)
    obj.location = location
    return link(obj, parent)


def mesh_object(name, bm, material=None, parent=None):
    me = bpy.data.meshes.new(name)
    bm.to_mesh(me)
    bm.free()
    obj = bpy.data.objects.new(name, me)
    if material is not None:
        me.materials.append(material)
    return link(obj, parent)


# ----------------------------------------------------------------------------
# Profiles and lathe
# ----------------------------------------------------------------------------

def catmull(p0, p1, p2, p3, t):
    t2 = t * t
    t3 = t2 * t
    return tuple(
        0.5 * ((2 * b) + (-a + c) * t + (2 * a - 5 * b + 4 * c - d) * t2 + (-a + 3 * b - 3 * c + d) * t3)
        for a, b, c, d in zip(p0, p1, p2, p3)
    )


def smooth_profile(points, steps=6):
    """points: [(r, z) or (r, z, 'S')]; 'S' marks a crease. Smooth runs between creases
    become Catmull-Rom curves through every point."""
    pts = [(p[0], p[1]) for p in points]
    sharp = [len(p) > 2 and p[2] == "S" for p in points]
    sharp[0] = sharp[-1] = True
    out = [pts[0]]
    for i in range(len(pts) - 1):
        p1, p2 = pts[i], pts[i + 1]
        if sharp[i] and sharp[i + 1]:
            out.append(p2)
            continue
        p0 = pts[i - 1] if (i > 0 and not sharp[i]) else (2 * p1[0] - p2[0], 2 * p1[1] - p2[1])
        p3 = pts[i + 2] if (i + 2 < len(pts) and not sharp[i + 1]) else (2 * p2[0] - p1[0], 2 * p2[1] - p1[1])
        for s in range(1, steps + 1):
            out.append(catmull(p0, p1, p2, p3, s / steps))
    return out, sharp


def lathe_bmesh(profile, segments=96, u_repeat=2.0, v_scale=1 / 80.0, z_offset=0.0):
    """Turn a dense (r, z) polyline around Z. UVs: u wraps around, v follows height."""
    bm = bmesh.new()
    uv = bm.loops.layers.uv.new("UVMap")
    rows = []
    for r, z in profile:
        row = []
        for j in range(segments + 1):
            a = 2 * math.pi * (j % segments) / segments
            row.append(bm.verts.new((r * math.cos(a), r * math.sin(a), z + z_offset)))
        rows.append(row)
    for i in range(len(rows) - 1):
        for j in range(segments):
            quad = (rows[i][j], rows[i][j + 1], rows[i + 1][j + 1], rows[i + 1][j])
            if len({v.co.to_tuple(6) for v in quad}) < 3:
                continue
            f = bm.faces.new(quad)
            coords = ((j, i), (j + 1, i), (j + 1, i + 1), (j, i + 1))
            for loop, (jj, ii) in zip(f.loops, coords):
                loop[uv].uv = (jj / segments * u_repeat, (profile[ii][1] + z_offset) * v_scale)
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    return bm


def lathe(name, points, segments=64, steps=4, material=None, parent=None, **kw):
    dense, _ = smooth_profile(points, steps)
    return mesh_object(name, lathe_bmesh(dense, segments, **kw), material, parent)


def shade_smooth(obj, angle=35.0):
    me = obj.data
    for p in me.polygons:
        p.use_smooth = True
    try:
        me.set_sharp_from_angle(angle=math.radians(angle))
    except AttributeError:
        pass


def apply_scale(obj, factor):
    obj.data.transform(Matrix.Scale(factor, 4))
    obj.data.update()


def join(objs, name):
    bpy.ops.object.select_all(action="DESELECT")
    for o in objs:
        o.select_set(True)
    bpy.context.view_layer.objects.active = objs[0]
    bpy.ops.object.join()
    obj = bpy.context.view_layer.objects.active
    obj.name = name
    obj.data.name = name
    return obj


def apply_modifiers(obj):
    bpy.context.view_layer.objects.active = obj
    bpy.ops.object.select_all(action="DESELECT")
    obj.select_set(True)
    for m in list(obj.modifiers):
        bpy.ops.object.modifier_apply(modifier=m.name)


# ----------------------------------------------------------------------------
# Materials
# ----------------------------------------------------------------------------

def image(path, color=True):
    img = bpy.data.images.load(path, check_existing=True)
    img.colorspace_settings.name = "sRGB" if color else "Non-Color"
    return img


def pbr(name, color=(0.8, 0.8, 0.8), roughness=0.5, metallic=0.0, tex=None, normal_strength=1.0,
        transmission=0.0, ior=1.5, emission=None, emission_strength=0.0, alpha=1.0):
    """Principled material in the layout the glTF exporter understands. tex is a texture
    prefix under build/textures (<tex>_base_color/_mr/_normal.png)."""
    mat = bpy.data.materials.new(name)
    mat.use_nodes = True
    nt = mat.node_tree
    bsdf = nt.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (*color, 1.0)
    bsdf.inputs["Roughness"].default_value = roughness
    bsdf.inputs["Metallic"].default_value = metallic
    bsdf.inputs["IOR"].default_value = ior
    if transmission:
        bsdf.inputs["Transmission Weight"].default_value = transmission
    if emission is not None:
        bsdf.inputs["Emission Color"].default_value = (*emission, 1.0)
        bsdf.inputs["Emission Strength"].default_value = emission_strength
    if tex:
        base = nt.nodes.new("ShaderNodeTexImage")
        base.image = image(os.path.join(TEX, f"{tex}_base_color.png"))
        nt.links.new(base.outputs["Color"], bsdf.inputs["Base Color"])
        mr_path = os.path.join(TEX, f"{tex}_mr.png")
        if os.path.exists(mr_path):
            mr = nt.nodes.new("ShaderNodeTexImage")
            mr.image = image(mr_path, color=False)
            sep = nt.nodes.new("ShaderNodeSeparateColor")
            nt.links.new(mr.outputs["Color"], sep.inputs["Color"])
            nt.links.new(sep.outputs["Green"], bsdf.inputs["Roughness"])
            nt.links.new(sep.outputs["Blue"], bsdf.inputs["Metallic"])
        n_path = os.path.join(TEX, f"{tex}_normal.png")
        if os.path.exists(n_path):
            nimg = nt.nodes.new("ShaderNodeTexImage")
            nimg.image = image(n_path, color=False)
            nmap = nt.nodes.new("ShaderNodeNormalMap")
            nmap.inputs["Strength"].default_value = normal_strength
            nt.links.new(nimg.outputs["Color"], nmap.inputs["Color"])
            nt.links.new(nmap.outputs["Normal"], bsdf.inputs["Normal"])
    return mat


# ----------------------------------------------------------------------------
# Export and preview
# ----------------------------------------------------------------------------

def export_glb(name, objects=None):
    os.makedirs(OUT, exist_ok=True)
    path = os.path.join(OUT, f"{name}.glb")
    if objects is not None:
        bpy.ops.object.select_all(action="DESELECT")
        for o in objects:
            o.select_set(True)
    # The exporter logs a line per primitive; keep the MCP replies short.
    import contextlib
    import io
    with contextlib.redirect_stdout(io.StringIO()):
        _export(path, objects)
    return path


def _export(path, objects):
    bpy.ops.export_scene.gltf(
        filepath=path,
        export_format="GLB",
        export_yup=True,
        use_selection=objects is not None,
        export_apply=True,
        export_image_format="AUTO",
        export_cameras=False,
        export_lights=False,
    )


def preview(name, target=(0, 0, 0.04), distance=0.35, elevation=18.0, azimuth=-60.0,
            size=(960, 640), samples=48, lens=60, world=0.6):
    """Quick Cycles render for checking shapes (does not touch exported data)."""
    os.makedirs(PREVIEW, exist_ok=True)
    scene = bpy.context.scene
    scene.render.engine = "CYCLES"
    scene.cycles.samples = samples
    scene.cycles.use_denoising = True
    scene.cycles.device = "CPU"
    scene.render.threads_mode = "FIXED"
    scene.render.threads = 24
    scene.render.resolution_x, scene.render.resolution_y = size
    scene.render.resolution_percentage = 100
    scene.view_settings.view_transform = "AgX"
    if scene.world is None:
        scene.world = bpy.data.worlds.new("World")
    scene.world.use_nodes = True
    bg = scene.world.node_tree.nodes["Background"]
    bg.inputs["Color"].default_value = (0.55, 0.6, 0.68, 1)
    bg.inputs["Strength"].default_value = world
    cam = bpy.data.objects.get("_PreviewCam")
    if cam is None:
        cam = link(bpy.data.objects.new("_PreviewCam", bpy.data.cameras.new("_PreviewCam")))
        key = link(bpy.data.objects.new("_PreviewKey", bpy.data.lights.new("_PreviewKey", "AREA")))
        key.data.energy = 60
        key.data.size = 0.4
        key.location = (-0.4, -0.5, 0.6)
        key.rotation_euler = (math.radians(50), 0, math.radians(-40))
        rim = link(bpy.data.objects.new("_PreviewRim", bpy.data.lights.new("_PreviewRim", "AREA")))
        rim.data.energy = 30
        rim.data.size = 0.3
        rim.location = (0.5, 0.5, 0.4)
        rim.rotation_euler = (math.radians(-55), 0, math.radians(135))
    cam.data.lens = lens
    cam.data.clip_start = 0.005
    t = Vector(target)
    el = math.radians(elevation)
    az = math.radians(azimuth)
    cam.location = t + Vector((math.cos(el) * math.sin(az), -math.cos(el) * math.cos(az), math.sin(el))) * distance
    cam.rotation_euler = (t - cam.location).to_track_quat("-Z", "Y").to_euler()
    scene.camera = cam
    path = os.path.join(PREVIEW, f"{name}.png")
    scene.render.filepath = path
    bpy.ops.render.render(write_still=True)
    return path


def remove_preview_rig():
    for n in ("_PreviewCam", "_PreviewKey", "_PreviewRim"):
        o = bpy.data.objects.get(n)
        if o:
            bpy.data.objects.remove(o, do_unlink=True)
