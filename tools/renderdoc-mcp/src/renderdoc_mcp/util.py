"""Utilities: loading the renderdoc module, enum names, serialization helpers."""

from __future__ import annotations

import enum
import json
import os
import sys
from typing import Any


def load_renderdoc():
    """Load the renderdoc Python module, configuring paths from environment."""
    if "renderdoc" in sys.modules:
        return sys.modules["renderdoc"]

    module_path = os.environ.get("RENDERDOC_MODULE_PATH", "")
    if module_path:
        if module_path not in sys.path:
            sys.path.insert(0, module_path)
        if sys.platform == "win32" and hasattr(os, "add_dll_directory"):
            os.add_dll_directory(module_path)

    import renderdoc  # noqa: E402

    return renderdoc


rd = load_renderdoc()

# ── Shader stages ──

# Names accepted for `stage` parameters. WGSL calls the pixel stage "fragment".
SHADER_STAGE_MAP: dict[str, Any] = {
    "vertex": rd.ShaderStage.Vertex,
    "hull": rd.ShaderStage.Hull,
    "domain": rd.ShaderStage.Domain,
    "geometry": rd.ShaderStage.Geometry,
    "pixel": rd.ShaderStage.Pixel,
    "fragment": rd.ShaderStage.Pixel,
    "compute": rd.ShaderStage.Compute,
    "task": rd.ShaderStage.Task,
    "mesh": rd.ShaderStage.Mesh,
}

# Stages of a graphics pipeline, in pipeline order, with the names reported for them.
GRAPHICS_STAGES: list[tuple[str, Any]] = [
    ("task", rd.ShaderStage.Task),
    ("mesh", rd.ShaderStage.Mesh),
    ("vertex", rd.ShaderStage.Vertex),
    ("hull", rd.ShaderStage.Hull),
    ("domain", rd.ShaderStage.Domain),
    ("geometry", rd.ShaderStage.Geometry),
    ("pixel", rd.ShaderStage.Pixel),
]


def parse_stage(stage: str):
    """The ShaderStage for a stage name, or None."""
    return SHADER_STAGE_MAP.get(stage.lower())


def stage_error(stage: str) -> dict:
    return make_error(
        f"Unknown shader stage: {stage}. Valid: {', '.join(SHADER_STAGE_MAP)}", "API_ERROR"
    )


FILE_TYPE_MAP: dict[str, Any] = {
    "png": rd.FileType.PNG,
    "jpg": rd.FileType.JPG,
    "bmp": rd.FileType.BMP,
    "tga": rd.FileType.TGA,
    "hdr": rd.FileType.HDR,
    "exr": rd.FileType.EXR,
    "dds": rd.FileType.DDS,
}

MESH_DATA_STAGE_MAP: dict[str, Any] = {
    "vsin": rd.MeshDataStage.VSIn,
    "vsout": rd.MeshDataStage.VSOut,
    "gsout": rd.MeshDataStage.GSOut,
}


# ── Enum names ──

def name_of(value: Any, enum_type: Any = None) -> str:
    """Readable name of a RenderDoc enum value.

    RenderDoc's enums are Python IntEnums, whose str() is only the number since
    Python 3.11, so names come from `.name`. A plain int is looked up in
    `enum_type` when given.
    """
    if isinstance(value, enum.Enum):
        return value.name or str(value.value)
    if enum_type is not None:
        try:
            return enum_type(int(value)).name
        except (TypeError, ValueError):
            pass
    return str(value)


# ── Blending ──

_BLEND_FACTORS: dict[str, str] = {
    "Zero": "0", "One": "1",
    "SrcCol": "src.rgb", "InvSrcCol": "(1-src.rgb)",
    "DstCol": "dst.rgb", "InvDstCol": "(1-dst.rgb)",
    "SrcAlpha": "src.a", "InvSrcAlpha": "(1-src.a)",
    "DstAlpha": "dst.a", "InvDstAlpha": "(1-dst.a)",
    "SrcAlphaSat": "sat(src.a)",
    "FactorRGB": "constant.rgb", "InvFactorRGB": "(1-constant.rgb)",
    "FactorAlpha": "constant.a", "InvFactorAlpha": "(1-constant.a)",
    "Src1Col": "src1.rgb", "InvSrc1Col": "(1-src1.rgb)",
    "Src1Alpha": "src1.a", "InvSrc1Alpha": "(1-src1.a)",
}


def blend_formula(color_src: str, color_dst: str, color_op: str,
                  alpha_src: str, alpha_dst: str, alpha_op: str) -> str:
    """A readable blend formula from blend factor and operation names."""
    def _op(op: str, a: str, b: str) -> str:
        return {
            "Add": f"{a} + {b}",
            "Subtract": f"{a} - {b}",
            "ReversedSubtract": f"{b} - {a}",
            "Minimum": f"min({a}, {b})",
            "Maximum": f"max({a}, {b})",
        }.get(op, f"{a} {op} {b}")

    def _term(factor: str, value: str) -> str:
        f = _BLEND_FACTORS.get(factor, factor)
        if f == "0":
            return "0"
        if f == "1":
            return value
        return f"{f}*{value}"

    color = _op(color_op, _term(color_src, "src.rgb"), _term(color_dst, "dst.rgb"))
    alpha = _op(alpha_op, _term(alpha_src, "src.a"), _term(alpha_dst, "dst.a"))
    return f"rgb = {color} | a = {alpha}"


def serialize_blend(blend) -> dict:
    """One render target's ColorBlend state."""
    result: dict = {"enabled": bool(blend.enabled), "write_mask": f"0x{int(blend.writeMask):x}"}
    if blend.enabled:
        c, a = blend.colorBlend, blend.alphaBlend
        names = [name_of(x) for x in (c.source, c.destination, c.operation,
                                       a.source, a.destination, a.operation)]
        result["color"] = {"source": names[0], "destination": names[1], "operation": names[2]}
        result["alpha"] = {"source": names[3], "destination": names[4], "operation": names[5]}
        result["formula"] = blend_formula(*names)
    if blend.logicOperationEnabled:
        result["logic_operation"] = name_of(blend.logicOperation)
    return result


# ── Errors and JSON ──

def make_error(message: str, code: str = "API_ERROR") -> dict:
    """Return a standardized error dict."""
    return {"error": message, "code": code}


def to_json(obj: Any) -> str:
    """Serialize to compact JSON."""
    return json.dumps(obj, separators=(",", ":"), ensure_ascii=False, default=str)


def rounded(values, digits: int = 6) -> list:
    return [round(v, digits) if isinstance(v, float) else v for v in values]


# ── Action flags ──

_ACTION_FLAG_NAMES: dict[int, str] | None = None


def _build_flag_names() -> dict[int, str]:
    """Single-bit ActionFlags values and their names."""
    names: dict[int, str] = {}
    for attr in dir(rd.ActionFlags):
        if attr.startswith("_"):
            continue
        val = getattr(rd.ActionFlags, attr)
        if isinstance(val, int) and val != 0 and (int(val) & (int(val) - 1)) == 0:
            names[int(val)] = attr
    return names


def flags_to_list(flags: int) -> list[str]:
    """ActionFlags bitmask as a list of flag names."""
    global _ACTION_FLAG_NAMES
    if _ACTION_FLAG_NAMES is None:
        _ACTION_FLAG_NAMES = _build_flag_names()
    flags = int(flags)
    return [name for bit, name in _ACTION_FLAG_NAMES.items() if flags & bit]


WORK_FLAGS = None  # set lazily: Drawcall | Dispatch | MeshDispatch | DispatchRay


def is_work(action) -> bool:
    """Whether an action does GPU work worth timing: a draw or a dispatch."""
    global WORK_FLAGS
    if WORK_FLAGS is None:
        WORK_FLAGS = rd.ActionFlags.Drawcall | rd.ActionFlags.Dispatch
        for extra in ("MeshDispatch", "DispatchRay"):
            if hasattr(rd.ActionFlags, extra):
                WORK_FLAGS |= getattr(rd.ActionFlags, extra)
    return bool(action.flags & WORK_FLAGS)


# ── Serialization ──

def serialize_action(action, structured_file, depth: int = 0, max_depth: int = 2) -> dict:
    """An ActionDescription and its children down to max_depth."""
    result: dict = {
        "event_id": action.eventId,
        "name": action.GetName(structured_file),
        "flags": flags_to_list(action.flags),
    }
    if action.numIndices:
        result["num_indices"] = action.numIndices
    if action.numInstances > 1:
        result["num_instances"] = action.numInstances
    if action.flags & rd.ActionFlags.Dispatch:
        result["dispatch"] = list(action.dispatchDimension)
    if depth < max_depth and len(action.children) > 0:
        result["children"] = [
            serialize_action(c, structured_file, depth + 1, max_depth) for c in action.children
        ]
    elif len(action.children) > 0:
        result["children_count"] = len(action.children)
    return result


def marker_path(action, structured_file) -> list[str]:
    """Names of the debug-marker regions enclosing an action, outermost first."""
    path = []
    parent = action.parent
    while parent is not None:
        if parent.flags & rd.ActionFlags.PushMarker:
            path.append(parent.GetName(structured_file))
        parent = parent.parent
    return path[::-1]


def serialize_action_detail(action, structured_file) -> dict:
    """One ActionDescription in full, without its children."""
    result: dict = {
        "event_id": action.eventId,
        "name": action.GetName(structured_file),
        "flags": flags_to_list(action.flags),
        "markers": marker_path(action, structured_file),
        "num_indices": action.numIndices,
        "num_instances": action.numInstances,
        "index_offset": action.indexOffset,
        "base_vertex": action.baseVertex,
        "vertex_offset": action.vertexOffset,
        "instance_offset": action.instanceOffset,
        "draw_index": action.drawIndex,
    }
    if action.flags & rd.ActionFlags.Dispatch:
        result["dispatch_groups"] = list(action.dispatchDimension)
        result["threads_per_group"] = list(action.dispatchThreadsDimension)
    result["outputs"] = [str(o) for o in action.outputs if int(o) != 0]
    result["depth_output"] = str(action.depthOut) if int(action.depthOut) != 0 else None
    if action.parent:
        result["parent_event_id"] = action.parent.eventId
    if action.previousAction:
        result["previous_event_id"] = action.previousAction.eventId
    if action.nextAction:
        result["next_event_id"] = action.nextAction.eventId
    result["children_count"] = len(action.children)
    result["api_calls"] = len(action.events)
    return result


def texture_size(tex) -> str:
    size = f"{tex.width}x{tex.height}"
    if tex.depth > 1:
        size += f"x{tex.depth}"
    return size


def serialize_texture_desc(tex, name: str = "") -> dict:
    """A TextureDescription; `name` is the resource's debug name."""
    result = {
        "resource_id": str(tex.resourceId),
        "name": name,
        "type": name_of(tex.type, rd.TextureType),
        "size": texture_size(tex),
        "format": str(tex.format.Name()),
        "mips": tex.mips,
        "array_size": tex.arraysize,
        "bytes": tex.byteSize,
        "usage": [f.name for f in rd.TextureCategory if f.value and tex.creationFlags & f.value],
    }
    if tex.msSamp > 1:
        result["samples"] = tex.msSamp
    return result


def serialize_buffer_desc(buf, name: str = "") -> dict:
    """A BufferDescription; `name` is the resource's debug name."""
    return {
        "resource_id": str(buf.resourceId),
        "name": name,
        "bytes": buf.length,
        "usage": [f.name for f in rd.BufferCategory if f.value and buf.creationFlags & f.value],
    }


def serialize_resource_desc(res) -> dict:
    return {
        "resource_id": str(res.resourceId),
        "name": res.name,
        "type": name_of(res.type, rd.ResourceType),
    }


# VarType name -> (ShaderValue accessor, reported type)
_VALUE_ACCESSORS: dict[str, tuple[str, str]] = {
    "Float": ("f32v", "float"),
    "Half": ("f16v", "half"),
    "Double": ("f64v", "double"),
    "SInt": ("s32v", "int"),
    "SShort": ("s16v", "int"),
    "SByte": ("s8v", "int"),
    "SLong": ("s64v", "int"),
    "UInt": ("u32v", "uint"),
    "UShort": ("u16v", "uint"),
    "UByte": ("u8v", "uint"),
    "ULong": ("u64v", "uint"),
    "Bool": ("u32v", "bool"),
    "Enum": ("u32v", "uint"),
    "GPUPointer": ("u64v", "pointer"),
}


def value_accessor(var_type) -> tuple[str, str]:
    """(ShaderValue attribute, type name) for reading a variable of `var_type`."""
    return _VALUE_ACCESSORS.get(name_of(var_type, rd.VarType), ("f32v", "float"))


def shader_values(var, var_type=None) -> list:
    """The flat list of a leaf ShaderVariable's components."""
    accessor, type_name = value_accessor(var.type if var_type is None else var_type)
    raw = getattr(var.value, accessor)
    count = max(var.rows, 1) * max(var.columns, 1)
    values = list(raw[:count])
    if type_name == "half":
        values = [float(v) for v in values]
    if type_name == "bool":
        values = [bool(v) for v in values]
    return rounded(values)


def serialize_shader_variable(var, max_depth: int = 10, depth: int = 0) -> dict:
    """A ShaderVariable and its members."""
    result: dict[str, Any] = {"name": var.name}
    if len(var.members) == 0:
        values = shader_values(var)
        result["type"] = value_accessor(var.type)[1]
        if var.rows > 1:
            result["value"] = [values[r * var.columns:(r + 1) * var.columns] for r in range(var.rows)]
        else:
            result["value"] = values
    elif depth < max_depth:
        members = [serialize_shader_variable(m, max_depth, depth + 1) for m in var.members]
        # SPIR-V wraps a WGSL uniform's struct in a block with one member; drop the wrapper.
        if len(members) == 1 and members[0]["name"].startswith("_child"):
            inner = members[0]
            result.update({k: v for k, v in inner.items() if k != "name"})
        else:
            result["members"] = members
    return result


def serialize_usage_entry(usage) -> dict:
    return {"event_id": usage.eventId, "usage": name_of(usage.usage, rd.ResourceUsage)}


def serialize_sig_element(sig) -> dict:
    """A SigParameter (shader input or output)."""
    result = {
        "name": sig.varName or sig.semanticIdxName,
        "type": name_of(sig.varType, rd.VarType),
        "components": sig.compCount,
        "location": sig.regIndex,
    }
    builtin = name_of(sig.systemValue, rd.ShaderBuiltin)
    if builtin != "Undefined":
        result["builtin"] = builtin
    if sig.semanticName:
        result["semantic"] = sig.semanticIdxName or sig.semanticName
    return result
