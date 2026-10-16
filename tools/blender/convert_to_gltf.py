"""Convert a model for the engine's importer: FBX or .blend in, GLB out.

The editor runs this inside Blender when you import an .fbx or .blend file (like Godot
does for .blend files), and you can run it yourself:

    blender -b --factory-startup --python tools/blender/convert_to_gltf.py -- \\
        SOURCE OUTPUT.glb [--animations-only]

- FBX files are imported as Blender reads them, Mixamo's included (centimetres, Y up,
  `mixamorig:` bone names; the engine binds clips by bone name without the prefix).
  .blend files are opened as they are.
- A file with one animation names it after the file, so "Running.fbx" plays as
  "Running" (Mixamo calls every clip "mixamo.com"). Several animations keep their
  names, without Blender's "Armature|...|Layer0" decoration.
- With --animations-only, or when the file has no meshes, only the armature and its
  animations are written: a clip library for characters with the same bone names
  (Mixamo's "Without Skin" downloads).

Prints one line `CONVERTED {json}` describing what it wrote.
"""

import json
import os
import sys

import bpy


def clean_action_name(name, stem, single):
    if single:
        return stem
    # "Armature|Walk|Layer0" -> "Walk"
    parts = [part for part in name.split("|") if part and part not in ("Armature",)]
    parts = [part for part in parts if not part.startswith("Layer")]
    return parts[-1] if parts else name


def main():
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    if len(argv) < 2:
        raise SystemExit("usage: convert_to_gltf.py -- SOURCE OUTPUT.glb [--animations-only]")
    source, output = os.path.abspath(argv[0]), os.path.abspath(argv[1])
    animations_only = "--animations-only" in argv[2:]
    extension = os.path.splitext(source)[1].lower()
    stem = os.path.splitext(os.path.basename(source))[0]

    if extension == ".blend":
        bpy.ops.wm.open_mainfile(filepath=source)
    else:
        bpy.ops.wm.read_homefile(use_empty=True)
        if extension == ".fbx":
            bpy.ops.import_scene.fbx(filepath=source)
        elif extension == ".obj":
            bpy.ops.wm.obj_import(filepath=source)
        else:
            raise SystemExit(f"cannot convert {extension} files")

    actions = list(bpy.data.actions)
    single = len(actions) == 1
    names = set()
    for action in actions:
        name = clean_action_name(action.name, stem, single)
        base, index = name, 2
        while name in names:
            name = f"{base} {index}"
            index += 1
        names.add(name)
        action.name = name
        # Keep every animation for the exporter, assigned or not.
        action.use_fake_user = True

    meshes = [obj for obj in bpy.data.objects if obj.type == "MESH"]
    armatures = [obj for obj in bpy.data.objects if obj.type == "ARMATURE"]
    clips_only = animations_only or (not meshes and bool(actions))
    if clips_only and not armatures:
        raise SystemExit("no armature to take animations from")

    # Export the visible scene contents (or just the armatures for a clip library).
    for obj in bpy.data.objects:
        obj.hide_set(False)
        obj.select_set(obj in armatures if clips_only else True)

    os.makedirs(os.path.dirname(output), exist_ok=True)
    bpy.ops.export_scene.gltf(
        filepath=output,
        export_format="GLB",
        use_selection=True,
        export_yup=True,
        export_apply=False,
        export_animations=bool(actions),
        export_animation_mode="ACTIONS",
        export_force_sampling=True,
        export_skins=True,
        export_def_bones=False,
        export_reset_pose_bones=True,
        export_cameras=False,
        export_lights=False,
        export_image_format="AUTO",
    )
    report = {
        "output": output,
        "clips_only": clips_only,
        "clips": sorted(names),
        "meshes": len(meshes) if not clips_only else 0,
        "armatures": len(armatures),
        "bones": sum(len(arm.data.bones) for arm in armatures),
    }
    print("CONVERTED " + json.dumps(report), flush=True)


if __name__ == "__main__":
    main()
