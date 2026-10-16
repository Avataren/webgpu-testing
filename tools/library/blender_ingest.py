"""Turn one source file into asset-library files, inside Blender.

    blender -b --factory-startup --python blender_ingest.py -- \\
        --input SOURCE --out DIR [--reference CHARACTER.glb] [--name NAME]
        [--height-range MIN MAX] [--clips no]

SOURCE is an FBX (Mixamo included), glTF/GLB, .blend or OBJ. The script writes to DIR:

- `character.glb` (with `character.png`): the meshes and armature without animations,
  when the file has meshes.
- `clip-<n>.glb` (with `clip-<n>.png`): one file per animation, holding the armature and
  that one animation. A single animation in a file is named NAME (or the file name;
  Mixamo calls every clip "mixamo.com").

A character outside --height-range (metres; Mixamo's files come at odd scales) is scaled
to 1.8 m. Textures are made game-ready: at most 2048 px, and opaque ones stored as JPEG
(those with transparency, hair and lashes, stay PNG). --clips no skips the animations a
character file may carry.

Thumbnails are 256x256 Workbench renders from the front: a character in its rest pose,
an animation at 40% of its length on the file's own character or, for animation-only
files, on --reference (a character GLB with the same bone names).

Prints one line `INGEST {json}` with what it wrote and measured: per animation its name,
duration, how fast and how far the root travels over the ground, and whether it stays
in place; per character its height, triangle and bone counts and bone names.
"""

import json
import math
import os
import sys

import bpy
import mathutils


def args():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    out = {"reference": None, "name": None, "height_range": None, "clips": "yes"}
    i = 0
    while i < len(argv):
        key = argv[i].lstrip("-").replace("-", "_")
        if key == "height_range":
            out[key] = (float(argv[i + 1]), float(argv[i + 2]))
            i += 3
            continue
        out[key] = argv[i + 1]
        i += 2
    return out


MAX_TEXTURE = 2048
TARGET_HEIGHT = 1.8


def optimize_textures(work_dir):
    """At most MAX_TEXTURE px; opaque images are swapped for JPEG files (smaller; the
    exporter keeps each image's own format), images with transparency stay PNG.
    Returns how many images changed."""
    import numpy
    changed = 0
    for index, image in enumerate(list(bpy.data.images)):
        if image.type != "IMAGE" or image.size[0] == 0 or image.users == 0:
            continue
        width, height = image.size
        resized = max(width, height) > MAX_TEXTURE
        if resized:
            factor = MAX_TEXTURE / max(width, height)
            image.scale(max(1, round(width * factor)), max(1, round(height * factor)))
        opaque = True
        if image.channels == 4:
            pixels = numpy.empty(len(image.pixels), dtype=numpy.float32)
            image.pixels.foreach_get(pixels)
            opaque = bool((pixels[3::4] > 0.99).all())
        if not opaque:
            changed += resized
            continue
        # Named after the image: the exporter (and a project's texture folder) use it.
        name = image.name
        stem = "".join(c if c.isalnum() or c in "-_" else "_" for c in name) or "texture"
        path = os.path.join(work_dir, f"{stem}-{index}.jpg")
        image.file_format = "JPEG"
        try:
            image.save(filepath=path, quality=90)
        except TypeError:  # older Blender: no arguments
            image.filepath_raw = path
            image.save()
        jpeg = bpy.data.images.load(path)
        jpeg.colorspace_settings.name = image.colorspace_settings.name
        image.user_remap(jpeg)
        bpy.data.images.remove(image)
        jpeg.name = name
        changed += 1
    return changed


def load(path):
    ext = os.path.splitext(path)[1].lower()
    if ext == ".blend":
        bpy.ops.wm.open_mainfile(filepath=path)
        return
    if ext == ".fbx":
        bpy.ops.import_scene.fbx(filepath=path)
    elif ext in (".glb", ".gltf"):
        bpy.ops.import_scene.gltf(filepath=path)
    elif ext == ".obj":
        bpy.ops.wm.obj_import(filepath=path)
    else:
        raise SystemExit(f"cannot ingest {ext} files")


def assign(armature, action):
    """Make `action` the armature's only animation."""
    data = armature.animation_data or armature.animation_data_create()
    for track in list(data.nla_tracks):
        data.nla_tracks.remove(track)
    data.action = action
    if action is not None and getattr(data, "action_slot", None) is None and len(action.slots):
        data.action_slot = action.slots[0]


def fps():
    scene = bpy.context.scene
    return scene.render.fps / scene.render.fps_base


def set_frame(frame):
    scene = bpy.context.scene
    whole = math.floor(frame)
    scene.frame_set(whole, subframe=frame - whole)
    bpy.context.view_layer.update()


def root_bones(armature):
    """The top bone and its children: whichever travels shows the clip's ground motion
    (Mixamo's hips are the top bone; Unreal-style rigs move the pelvis under a fixed
    root, or the root itself in root-motion clips)."""
    tops = [bone for bone in armature.pose.bones if bone.parent is None]
    return tops + [child for top in tops for child in top.children]


def measure(armature, action):
    assign(armature, action)
    start, end = action.frame_range
    duration = max((end - start) / fps(), 1.0 / fps())
    bones = root_bones(armature)

    def positions():
        return [armature.matrix_world @ bone.head for bone in bones]

    set_frame(start)
    first = positions()
    set_frame(end)
    last = positions()
    # Blender is Z up: ground travel is in X and Y.
    travel = max(
        (math.hypot(b.x - a.x, b.y - a.y) for a, b in zip(first, last)), default=0.0
    )
    return {
        "duration": round(duration, 4),
        "frames": round(end - start + 1),
        "travel": round(travel, 4),
        "root_speed": round(travel / duration, 4),
        "in_place": travel / duration < 0.1,
    }


def meshes():
    return [obj for obj in bpy.data.objects if obj.type == "MESH"]


def armatures():
    return [obj for obj in bpy.data.objects if obj.type == "ARMATURE"]


def bounds(objects):
    """World box of the meshes as they are posed now (skinning included)."""
    depsgraph = bpy.context.evaluated_depsgraph_get()
    low = [math.inf] * 3
    high = [-math.inf] * 3
    for obj in objects:
        evaluated = obj.evaluated_get(depsgraph)
        mesh = evaluated.to_mesh()
        matrix = evaluated.matrix_world
        for vertex in mesh.vertices:
            point = matrix @ vertex.co
            for axis in range(3):
                low[axis] = min(low[axis], point[axis])
                high[axis] = max(high[axis], point[axis])
        evaluated.to_mesh_clear()
    return mathutils.Vector(low), mathutils.Vector(high)


def triangles(objects):
    depsgraph = bpy.context.evaluated_depsgraph_get()
    count = 0
    for obj in objects:
        mesh = obj.evaluated_get(depsgraph).to_mesh()
        mesh.calc_loop_triangles()
        count += len(mesh.loop_triangles)
        obj.evaluated_get(depsgraph).to_mesh_clear()
    return count


def setup_render():
    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.render.resolution_x = scene.render.resolution_y = 256
    scene.render.film_transparent = True
    scene.render.image_settings.file_format = "PNG"
    shading = scene.display.shading
    shading.light = "STUDIO"
    shading.color_type = "MATERIAL"
    shading.show_cavity = True
    if scene.camera is None:
        camera = bpy.data.objects.new("LibraryCamera", bpy.data.cameras.new("LibraryCamera"))
        scene.collection.objects.link(camera)
        scene.camera = camera
    return scene.camera


def textured(objects):
    """Whether the objects' materials use image textures."""
    for obj in objects:
        for slot in obj.material_slots:
            material = slot.material
            if material and material.use_nodes and any(
                node.type == "TEX_IMAGE" and node.image for node in material.node_tree.nodes
            ):
                return True
    return False


def render(camera, objects, path):
    """Frame `objects` from the front (characters face -Y in Blender) and render."""
    # Textured models show their textures; plain ones their material colours.
    bpy.context.scene.display.shading.color_type = "TEXTURE" if textured(objects) else "MATERIAL"
    low, high = bounds(objects)
    if not all(map(math.isfinite, low)):
        return False
    centre = (low + high) / 2
    size = max(high.z - low.z, high.x - low.x, high.y - low.y, 0.1)
    camera.data.lens = 50
    # A 50 mm lens sees about 0.72 of the distance across: fill the frame with margin.
    distance = size * 1.55
    camera.location = centre + mathutils.Vector((distance * 0.35, -distance, size * 0.1))
    camera.rotation_euler = (centre - camera.location).to_track_quat("-Z", "Y").to_euler()
    bpy.context.scene.render.filepath = path
    bpy.ops.render.render(write_still=True)
    return True


def export(path, objects, animations):
    bpy.ops.object.select_all(action="DESELECT")
    for obj in objects:
        obj.hide_set(False)
        obj.select_set(True)
    bpy.ops.export_scene.gltf(
        filepath=path,
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_apply=False,
        export_animations=animations,
        export_animation_mode="ACTIVE_ACTIONS",
        export_force_sampling=True,
        export_skins=True,
        export_def_bones=False,
        export_reset_pose_bones=True,
        export_cameras=False,
        export_lights=False,
        export_image_format="AUTO",
        export_jpeg_quality=90,
    )


def clean_name(name, stem, single):
    if single:
        return stem
    parts = [p for p in name.split("|") if p and p != "Armature" and not p.startswith("Layer")]
    return parts[-1] if parts else name


def main():
    opts = args()
    source = os.path.abspath(opts["input"])
    out = os.path.abspath(opts["out"])
    os.makedirs(out, exist_ok=True)
    stem = opts["name"] or os.path.splitext(os.path.basename(source))[0]

    bpy.ops.wm.read_homefile(use_empty=True)
    load(source)
    rigs = armatures()
    bodies = [m for m in meshes() if m.data.polygons]
    # With a skeleton, the body is what it deforms (packs carry stray helper meshes).
    skinned = [m for m in bodies if any(mod.type == "ARMATURE" for mod in m.modifiers)]
    if rigs and skinned:
        bodies = skinned
    # A character download may carry its rest pose as a one-frame "animation".
    actions = [
        a for a in bpy.data.actions
        if a.frame_range[1] - a.frame_range[0] >= 2 or not meshes()
    ]
    for action in bpy.data.actions:
        action.use_fake_user = True
    report = {"source": source, "character": None, "clips": []}
    camera = setup_render()
    armature = rigs[0] if rigs else None

    # Animation thumbnails need a body: the file's own, else the reference character.
    stage = bodies
    stage_rig = armature
    if not bodies and opts["reference"] and armature is not None:
        before = set(bpy.data.objects)
        bpy.ops.import_scene.gltf(filepath=opts["reference"])
        added = [obj for obj in bpy.data.objects if obj not in before]
        stage = [obj for obj in added if obj.type == "MESH"]
        stage_rig = next((obj for obj in added if obj.type == "ARMATURE"), None)
        if stage_rig is not None and armature is not None:
            # Same bone names: the reference plays the clip; keep it beside the source.
            stage_rig.location = armature.location

    if armature is not None:
        bone_names = [bone.name for bone in armature.data.bones]
    else:
        bone_names = []

    if bodies:
        if armature is not None:
            assign(armature, None)
            for bone in armature.pose.bones:
                bone.matrix_basis = mathutils.Matrix.Identity(4)
        bpy.context.view_layer.update()
        low, high = bounds(bodies)
        scaled = None
        if opts["height_range"]:
            lowest, highest = opts["height_range"]
            height = high.z - low.z
            if height > 0 and not lowest <= height <= highest:
                scaled = TARGET_HEIGHT / height
                for obj in bpy.data.objects:
                    if obj.parent is None and obj.type in ("ARMATURE", "MESH", "EMPTY"):
                        obj.scale = obj.scale * scaled
                        obj.location = obj.location * scaled
                bpy.context.view_layer.update()
                low, high = bounds(bodies)
        optimized = optimize_textures(out)
        export_objects = bodies + rigs
        path = os.path.join(out, "character.glb")
        export(path, export_objects, animations=False)
        thumb = os.path.join(out, "character.png")
        rendered = render(camera, bodies, thumb)
        report["character"] = {
            "file": path,
            "thumbnail": thumb if rendered else None,
            "height": round(high.z - low.z, 4),
            "triangles": triangles(bodies),
            "bones": len(bone_names),
            "meshes": len(bodies),
            "scaled": round(scaled, 4) if scaled else None,
            "textures_optimized": optimized,
        }

    if opts["clips"] == "no":
        actions = []
    single = len(actions) == 1
    for index, action in enumerate(actions):
        if armature is None:
            break
        name = clean_name(action.name, stem, single)
        info = measure(armature, action)
        info["name"] = name
        path = os.path.join(out, f"clip-{index}.glb")
        action.name = name
        assign(armature, action)
        export(path, [armature], animations=True)
        info["file"] = path
        thumb = None
        if stage and stage_rig is not None:
            if stage_rig is not armature:
                assign(stage_rig, action)
            start, end = action.frame_range
            set_frame(start + (end - start) * 0.4)
            thumb = os.path.join(out, f"clip-{index}.png")
            if not render(camera, stage, thumb):
                thumb = None
            if stage_rig is not armature:
                assign(stage_rig, None)
        info["thumbnail"] = thumb
        report["clips"].append(info)

    report["bones"] = bone_names
    print("INGEST " + json.dumps(report), flush=True)


if __name__ == "__main__":
    main()
