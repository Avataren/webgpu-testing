"""Data extraction tools: save_texture, get_buffer_data, pick_pixel, get_texture_stats,
read_texture_pixels, export_draw_textures, save_render_target, export_mesh."""

from __future__ import annotations

import math
import os
import re
import struct
from typing import Optional

from mcp.server.mcpserver import MCPServer

from renderdoc_mcp.session import get_session
from renderdoc_mcp.util import (
    rd,
    to_json,
    make_error,
    name_of,
    FILE_TYPE_MAP,
    GRAPHICS_STAGES,
)


MAX_BUFFER_READ = 65536

_BUFFER_FORMATS = {"float32": "f", "floats": "f", "uint32": "I", "int32": "i",
                   "uint16": "H", "float16": "e"}


def _save(session, resource_id, path: str, file_type=rd.FileType.PNG, mip: int = 0,
          slice_index: int = 0) -> None:
    texsave = rd.TextureSave()
    texsave.resourceId = resource_id
    texsave.destType = file_type
    texsave.mip = mip
    texsave.slice.sliceIndex = slice_index
    texsave.alpha = rd.AlphaMapping.Preserve
    session.controller.SaveTexture(texsave, path)


def _anomalies(rgb) -> str | None:
    if any(math.isnan(v) for v in rgb):
        return "NaN"
    if any(math.isinf(v) for v in rgb):
        return "Inf"
    if any(v < 0 for v in rgb):
        return "negative"
    return None


def _safe_filename(text: str) -> str:
    return re.sub(r"[^A-Za-z0-9_.-]+", "_", text).strip("_") or "texture"


def register(mcp: MCPServer):
    @mcp.tool()
    def save_texture(
        resource_id: str,
        output_path: str,
        file_type: str = "png",
        mip: int = 0,
        array_slice: int = 0,
        event_id: Optional[int] = None,
    ) -> str:
        """Save a texture as it is at an event to an image file. PNG/JPG/BMP/TGA are
        8-bit (HDR values are clipped); use exr or hdr to keep float data.

        Args:
            resource_id: Texture ID ("ResourceId::123" or "123") or unique debug name.
            output_path: Absolute path of the image to write.
            file_type: png, jpg, bmp, tga, hdr, exr or dds (default png).
            mip: Mip level (default 0; -1 saves all mips, DDS only).
            array_slice: Array layer or cube face (default 0).
            event_id: Event to read the texture at (default: the current event).
        """
        session = get_session()
        err = session.require_open() or session.ensure_event(event_id)
        if err:
            return to_json(err)

        tex_id = session.resolve_resource_id(resource_id)
        if tex_id is None:
            return to_json(session.resource_error(resource_id))
        ft = FILE_TYPE_MAP.get(file_type.lower())
        if ft is None:
            return to_json(make_error(f"Unknown file type: {file_type}. Valid: {list(FILE_TYPE_MAP)}", "API_ERROR"))

        output_path = os.path.normpath(output_path)
        os.makedirs(os.path.dirname(output_path) or ".", exist_ok=True)
        _save(session, tex_id, output_path, ft, mip, array_slice)
        return to_json({"status": "saved", "output_path": output_path, **session.describe(tex_id),
                        "event_id": session.current_event})

    @mcp.tool()
    def get_buffer_data(
        resource_id: str,
        offset: int = 0,
        length: int = 256,
        format: str = "hex",
        event_id: Optional[int] = None,
    ) -> str:
        """Read a buffer's contents at an event.

        Args:
            resource_id: Buffer ID or unique debug name.
            offset: Byte offset (default 0).
            length: Bytes to read (default 256, max 65536).
            format: hex (default), float32, uint32, int32, uint16 or float16.
            event_id: Event to read the buffer at (default: the current event, or the end
                      of the frame when none is selected).
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)
        if event_id is not None or session.current_event is not None:
            err = session.ensure_event(event_id)
            if err:
                return to_json(err)

        buf_id = session.resolve_resource_id(resource_id)
        if buf_id is None:
            return to_json(session.resource_error(resource_id))

        length = max(0, min(length, MAX_BUFFER_READ))
        data = session.controller.GetBufferData(buf_id, offset, length)
        result: dict = {**session.describe(buf_id), "offset": offset, "bytes_read": len(data),
                        "event_id": session.current_event}

        code = _BUFFER_FORMATS.get(format.lower())
        if code is not None:
            size = struct.calcsize(code)
            count = len(data) // size
            values = list(struct.unpack_from(f"<{count}{code}", data))
            result["data"] = [round(v, 6) if isinstance(v, float) else v for v in values]
            result["format"] = format.lower()
        else:
            lines = []
            for i in range(0, len(data), 16):
                chunk = data[i:i + 16]
                lines.append(f"{offset + i:08x}: {' '.join(f'{b:02x}' for b in chunk)}")
            result["data"] = "\n".join(lines)
            result["format"] = "hex"
        return to_json(result)

    @mcp.tool()
    def pick_pixel(
        resource_id: str,
        x: int,
        y: int,
        event_id: Optional[int] = None,
        mip: int = 0,
        array_slice: int = 0,
    ) -> str:
        """The value of one texel at an event (floats, plus raw uints for integer formats).

        Args:
            resource_id: Texture ID or unique debug name.
            x: X coordinate (origin top-left).
            y: Y coordinate.
            event_id: Event to read at (default: the current event).
            mip: Mip level (default 0).
            array_slice: Array layer or cube face (default 0).
        """
        session = get_session()
        err = session.require_open() or session.ensure_event(event_id)
        if err:
            return to_json(err)
        tex_id = session.resolve_resource_id(resource_id)
        if tex_id is None:
            return to_json(session.resource_error(resource_id))

        val = session.controller.PickPixel(tex_id, x, y, rd.Subresource(mip, array_slice, 0),
                                           rd.CompType.Typeless)
        rgba = [round(v, 6) for v in val.floatValue[:4]]
        result = {**session.describe(tex_id), "x": x, "y": y, "event_id": session.current_event,
                  "rgba": rgba}
        if "INT" in result.get("format", "") and "NORM" not in result.get("format", ""):
            result["uint"] = list(val.uintValue[:4])
        anomaly = _anomalies(rgba[:3])
        if anomaly:
            result["anomaly"] = anomaly
        return to_json(result)

    @mcp.tool()
    def get_texture_stats(
        resource_id: str,
        event_id: Optional[int] = None,
        all_slices: bool = False,
        mip: int = 0,
    ) -> str:
        """Per-channel min/max of a texture at an event, flagging NaN, Inf and negative
        values (exact, over every texel).

        Args:
            resource_id: Texture ID or unique debug name.
            event_id: Event to read at (default: the current event).
            all_slices: Report each array layer / cube face separately.
            mip: Mip level (default 0).
        """
        session = get_session()
        err = session.require_open() or session.ensure_event(event_id)
        if err:
            return to_json(err)
        tex_id = session.resolve_resource_id(resource_id)
        if tex_id is None:
            return to_json(session.resource_error(resource_id))
        tex_desc = session.get_texture_desc(str(tex_id))

        def minmax(slice_index: int) -> dict:
            mn, mx = session.controller.GetMinMax(tex_id, rd.Subresource(mip, slice_index, 0),
                                                  rd.CompType.Typeless)
            lo = [round(v, 6) for v in mn.floatValue[:4]]
            hi = [round(v, 6) for v in mx.floatValue[:4]]
            entry: dict = {"min": lo, "max": hi}
            flags = []
            if any(math.isnan(v) for v in lo[:3] + hi[:3]):
                flags.append("NaN")
            if any(math.isinf(v) for v in lo[:3] + hi[:3]):
                flags.append("Inf")
            if any(v < 0 for v in lo[:3]):
                flags.append("negative")
            if flags:
                entry["anomalies"] = flags
            return entry

        result: dict = {**session.describe(tex_id), "event_id": session.current_event, "mip": mip,
                        "channels": "rgba"}
        if all_slices and tex_desc is not None and tex_desc.arraysize > 1:
            faces = ["+X", "-X", "+Y", "-Y", "+Z", "-Z"]
            per_slice = []
            for s in range(tex_desc.arraysize):
                entry = minmax(s)
                if tex_desc.cubemap and s < 6:
                    entry["face"] = faces[s]
                else:
                    entry["slice"] = s
                per_slice.append(entry)
            result["per_slice"] = per_slice
        else:
            result.update(minmax(0))
        return to_json(result)

    @mcp.tool()
    def read_texture_pixels(
        resource_id: str,
        x: int,
        y: int,
        width: int,
        height: int,
        mip_level: int = 0,
        array_slice: int = 0,
        event_id: Optional[int] = None,
    ) -> str:
        """Read the RGBA values of a small region (up to 32x32) of a texture at an event,
        flagging NaN/Inf/negative texels. Useful for LUTs, history buffers, probe atlases.

        Args:
            resource_id: Texture ID or unique debug name.
            x: Left of the region.
            y: Top of the region.
            width: Region width (max 32).
            height: Region height (max 32).
            mip_level: Mip level (default 0).
            array_slice: Array layer or cube face (0=+X 1=-X 2=+Y 3=-Y 4=+Z 5=-Z).
            event_id: Event to read at (default: the current event).
        """
        session = get_session()
        err = session.require_open() or session.ensure_event(event_id)
        if err:
            return to_json(err)
        tex_id = session.resolve_resource_id(resource_id)
        if tex_id is None:
            return to_json(session.resource_error(resource_id))

        width, height = max(1, min(width, 32)), max(1, min(height, 32))
        sub = rd.Subresource(mip_level, array_slice, 0)
        pixels: list[list] = []
        anomalies: list[dict] = []
        for py in range(y, y + height):
            row: list = []
            for px in range(x, x + width):
                val = session.controller.PickPixel(tex_id, px, py, sub, rd.CompType.Typeless)
                pixel = [round(v, 5) for v in val.floatValue[:4]]
                row.append(pixel)
                kind = _anomalies(pixel[:3])
                if kind:
                    anomalies.append({"x": px, "y": py, "type": kind, "value": pixel})
            pixels.append(row)

        result: dict = {
            **session.describe(tex_id),
            "region": [x, y, width, height],
            "event_id": session.current_event,
            "rows": pixels,
        }
        if anomalies:
            result["anomalies"] = anomalies[:50]
            result["anomaly_count"] = len(anomalies)
        return to_json(result)

    @mcp.tool()
    def export_draw_textures(
        event_id: int,
        output_dir: str,
        skip_small: bool = True,
    ) -> str:
        """Save every texture a draw or dispatch reads, from all its shader stages, as PNG
        files named after the binding and the texture's debug name.

        Args:
            event_id: The draw or dispatch.
            output_dir: Directory to write into.
            skip_small: Skip textures of 4x4 or smaller (placeholders). Default true.
        """
        session = get_session()
        err = session.require_open() or session.set_event(event_id)
        if err:
            return to_json(err)

        state = session.controller.GetPipelineState()
        action = session.get_action(event_id)
        stages = ([("compute", rd.ShaderStage.Compute)] if action.flags & rd.ActionFlags.Dispatch
                  else GRAPHICS_STAGES)
        output_dir = os.path.normpath(output_dir)
        os.makedirs(output_dir, exist_ok=True)

        exported, skipped, seen = [], [], set()
        for stage_name, stage in stages:
            refl = state.GetShaderReflection(stage)
            if refl is None:
                continue
            for d in state.GetReadOnlyResources(stage):
                key = str(d.descriptor.resource)
                tex_desc = session.get_texture_desc(key)
                if tex_desc is None or key in seen:
                    continue
                seen.add(key)
                index = d.access.index
                binding = refl.readOnlyResources[index].name if index < len(refl.readOnlyResources) else f"slot{index}"
                info = {"stage": stage_name, "binding": binding, **session.describe(key)}
                if skip_small and tex_desc.width <= 4 and tex_desc.height <= 4:
                    skipped.append(info)
                    continue
                label = session.resource_name(key) or key.split("::")[-1]
                path = os.path.join(output_dir, _safe_filename(f"{binding}_{label}") + ".png")
                _save(session, d.descriptor.resource, path)
                info["output_path"] = path
                exported.append(info)

        return to_json({"event_id": event_id, "exported": exported, "skipped": skipped})

    @mcp.tool()
    def save_render_target(
        event_id: int,
        output_path: str,
        save_depth: bool = False,
        slot: int = 0,
    ) -> str:
        """Save a draw's color target (and optionally its depth target) as PNG, as it is
        after that draw.

        Args:
            event_id: The draw.
            output_path: File path, or an existing directory to auto-name files in.
            save_depth: Also save the depth target (default false).
            slot: Which color target to save (default 0).
        """
        session = get_session()
        err = session.require_open() or session.set_event(event_id)
        if err:
            return to_json(err)

        state = session.controller.GetPipelineState()
        outputs = [o for o in state.GetOutputTargets() if int(o.resource) != 0]
        if slot >= len(outputs):
            return to_json(make_error(f"No color target in slot {slot} at this event ({len(outputs)} bound)", "API_ERROR"))

        output_path = os.path.normpath(output_path)
        as_dir = os.path.isdir(output_path)
        color_path = (os.path.join(output_path, f"rt{slot}_eid{event_id}.png") if as_dir
                      else output_path)
        os.makedirs(os.path.dirname(color_path) or ".", exist_ok=True)
        _save(session, outputs[slot].resource, color_path)
        saved = [{"type": "color", "output_path": color_path, **session.describe(outputs[slot].resource)}]

        if save_depth:
            dt = state.GetDepthTarget()
            if int(dt.resource) != 0:
                base, ext = os.path.splitext(color_path)
                depth_path = (os.path.join(output_path, f"depth_eid{event_id}.png") if as_dir
                              else f"{base}_depth{ext or '.png'}")
                _save(session, dt.resource, depth_path)
                saved.append({"type": "depth", "output_path": depth_path, **session.describe(dt.resource)})
            else:
                saved.append({"type": "depth", "error": "no depth target bound"})

        return to_json({"event_id": event_id, "saved": saved})

    @mcp.tool()
    def export_mesh(
        event_id: int,
        output_path: str,
    ) -> str:
        """Export a draw's vertex-shader output as an OBJ: clip-space positions divided by
        w (so x/y are NDC and z is depth), with normals and UVs when an output is named
        like one. Shows what the draw rasterized; for model-space data read the vertex
        buffers with get_vertex_inputs and get_buffer_data.

        Args:
            event_id: The draw.
            output_path: Path of the .obj to write.
        """
        session = get_session()
        err = session.require_open() or session.set_event(event_id)
        if err:
            return to_json(err)

        mesh = session.controller.GetPostVSData(0, 0, rd.MeshDataStage.VSOut)
        if int(mesh.vertexResourceId) == 0:
            return to_json(make_error("No vertex shader output for this event", "API_ERROR"))
        state = session.controller.GetPipelineState()
        refl = state.GetShaderReflection(rd.ShaderStage.Vertex)
        if refl is None:
            return to_json(make_error("No vertex shader bound", "API_ERROR"))

        layout = _post_vs_layout(refl, state.HasAlignedPostVSData(rd.MeshDataStage.VSOut))
        indices = _post_vs_indices(session, mesh)
        stride = mesh.vertexByteStride
        vertex_count = max(indices) + 1 if indices else 0
        data = session.controller.GetBufferData(mesh.vertexResourceId, mesh.vertexByteOffset,
                                                vertex_count * stride)

        def find(*patterns):
            for name, (offset, count, code, builtin) in layout.items():
                if any(p in name.lower() for p in patterns) and code == "f" and not builtin:
                    return offset, count
            return None

        pos = next(((o, c) for _, (o, c, _code, b) in layout.items() if b == "Position"), (0, 4))
        normal = find("normal")
        uv = find("uv", "texcoord", "tex_coord")

        positions, normals, uvs = [], [], []
        for v in range(vertex_count):
            base = v * stride
            if base + stride > len(data):
                break
            x, y, z, w = struct.unpack_from("<4f", data, base + pos[0])
            w = w if abs(w) > 1e-12 else 1.0
            positions.append((x / w, y / w, z / w))
            if normal:
                normals.append(struct.unpack_from("<3f", data, base + normal[0]))
            if uv:
                uvs.append(struct.unpack_from("<2f", data, base + uv[0]))

        topology = name_of(state.GetPrimitiveTopology(), rd.Topology)
        triangles = _triangles(indices, topology, len(positions))

        output_path = os.path.normpath(output_path)
        os.makedirs(os.path.dirname(output_path) or ".", exist_ok=True)
        lines = [f"# RenderDoc MCP export of event {event_id} ({topology}, VS output / w)"]
        lines += [f"v {p[0]:.6f} {p[1]:.6f} {p[2]:.6f}" for p in positions]
        lines += [f"vn {n[0]:.6f} {n[1]:.6f} {n[2]:.6f}" for n in normals]
        lines += [f"vt {t[0]:.6f} {t[1]:.6f}" for t in uvs]
        has_n, has_t = bool(normals), bool(uvs)
        for tri in triangles:
            a, b, c = (i + 1 for i in tri)
            if has_n and has_t:
                lines.append(f"f {a}/{a}/{a} {b}/{b}/{b} {c}/{c}/{c}")
            elif has_n:
                lines.append(f"f {a}//{a} {b}//{b} {c}//{c}")
            elif has_t:
                lines.append(f"f {a}/{a} {b}/{b} {c}/{c}")
            else:
                lines.append(f"f {a} {b} {c}")
        with open(output_path, "w") as f:
            f.write("\n".join(lines) + "\n")

        return to_json({
            "event_id": event_id,
            "output_path": output_path,
            "topology": topology,
            "vertices": len(positions),
            "indices": len(indices),
            "triangles": len(triangles),
            "normals_from": next((n for n in layout if normal and layout[n][0] == normal[0]), None),
            "uvs_from": next((n for n in layout if uv and layout[n][0] == uv[0]), None),
        })


_VAR_CODES = {"Float": "f", "UInt": "I", "SInt": "i", "Half": "e", "Double": "d"}


def _post_vs_layout(refl, aligned: bool) -> dict[str, tuple[int, int, str, str | None]]:
    """name -> (byte offset, components, struct code, builtin) of each VS output.

    RenderDoc writes the position first, then the other outputs in signature order,
    tightly packed unless the API aligns post-VS data (see RenderDoc's mesh_output.py).
    """
    layout: dict = {}
    outputs = list(refl.outputSignature)
    pos = next((o for o in outputs if o.systemValue == rd.ShaderBuiltin.Position), None)
    offset = 0
    if pos is not None:
        layout[pos.varName or "position"] = (0, pos.compCount, "f", "Position")
        offset = pos.compCount * 4
    for o in outputs:
        if o is pos:
            continue
        width = rd.VarTypeByteSize(o.varType)
        if aligned:
            align = max(4, width) * (4 if o.compCount == 3 else o.compCount)
            if offset % align:
                offset += align - offset % align
        builtin = name_of(o.systemValue, rd.ShaderBuiltin)
        layout[o.varName or o.semanticIdxName] = (
            offset, o.compCount, _VAR_CODES.get(name_of(o.varType, rd.VarType), "f"),
            None if builtin == "Undefined" else builtin)
        offset += o.compCount * width
    return layout


def _post_vs_indices(session, mesh) -> list[int]:
    if int(mesh.indexResourceId) == 0 or mesh.indexByteStride not in (1, 2, 4):
        return list(range(mesh.numIndices))
    raw = session.controller.GetBufferData(mesh.indexResourceId, mesh.indexByteOffset,
                                           mesh.numIndices * mesh.indexByteStride)
    code = {1: "B", 2: "H", 4: "I"}[mesh.indexByteStride]
    count = len(raw) // mesh.indexByteStride
    restart = mesh.restartIndex if mesh.allowRestart else None
    return [i + mesh.baseVertex for i in struct.unpack_from(f"<{count}{code}", raw)
            if restart is None or i != restart]


def _triangles(indices: list[int], topology: str, vertex_count: int) -> list[tuple[int, int, int]]:
    tris = []
    if topology == "TriangleStrip":
        for i in range(len(indices) - 2):
            a, b, c = indices[i], indices[i + 1], indices[i + 2]
            tris.append((a, b, c) if i % 2 == 0 else (a, c, b))
    elif topology == "TriangleFan":
        for i in range(1, len(indices) - 1):
            tris.append((indices[0], indices[i], indices[i + 1]))
    elif topology == "TriangleList":
        for i in range(0, len(indices) - 2, 3):
            tris.append((indices[i], indices[i + 1], indices[i + 2]))
    return [t for t in tris if max(t) < vertex_count and len(set(t)) == 3]
