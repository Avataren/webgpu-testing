"""Resource tools: list_textures, list_buffers, list_resources, get_resource_usage."""

from __future__ import annotations

import re
from typing import Optional

from mcp.server.mcpserver import MCPServer

from renderdoc_mcp.session import get_session
from renderdoc_mcp.util import (
    rd,
    to_json,
    make_error,
    name_of,
    serialize_texture_desc,
    serialize_buffer_desc,
    serialize_resource_desc,
)


def _name_pattern(name_pattern: Optional[str]):
    if not name_pattern:
        return None
    try:
        return re.compile(name_pattern, re.IGNORECASE)
    except re.error as e:
        return make_error(f"Invalid regex pattern: {e}", "API_ERROR")


def register(mcp: MCPServer):
    @mcp.tool()
    def list_textures(
        name_pattern: Optional[str] = None,
        filter_format: Optional[str] = None,
        min_width: Optional[int] = None,
        render_targets_only: bool = False,
    ) -> str:
        """List textures with their debug names (wgpu labels), type, size, format, mips,
        layers, bytes and usage, largest first.

        Args:
            name_pattern: Regex matched against the debug name.
            filter_format: Format substring (e.g. "R16G16B16A16", "BC7", "D32").
            min_width: Minimum width.
            render_targets_only: Only color/depth targets and storage textures.
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)
        pattern = _name_pattern(name_pattern)
        if isinstance(pattern, dict):
            return to_json(pattern)

        target_flags = (rd.TextureCategory.ColorTarget | rd.TextureCategory.DepthTarget
                        | rd.TextureCategory.ShaderReadWrite)
        results = []
        for tex in session.controller.GetTextures():
            name = session.resource_name(tex.resourceId)
            if min_width is not None and tex.width < min_width:
                continue
            if filter_format and filter_format.upper() not in str(tex.format.Name()).upper():
                continue
            if render_targets_only and not (tex.creationFlags & target_flags):
                continue
            if pattern and not pattern.search(name):
                continue
            results.append(serialize_texture_desc(tex, name))
        results.sort(key=lambda t: -t["bytes"])
        return to_json({"textures": results, "count": len(results),
                        "mb": round(sum(t["bytes"] for t in results) / 1e6, 1)})

    @mcp.tool()
    def list_buffers(name_pattern: Optional[str] = None, min_size: Optional[int] = None) -> str:
        """List buffers with their debug names, sizes and usage, largest first.

        Args:
            name_pattern: Regex matched against the debug name.
            min_size: Minimum size in bytes.
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)
        pattern = _name_pattern(name_pattern)
        if isinstance(pattern, dict):
            return to_json(pattern)

        results = []
        for buf in session.controller.GetBuffers():
            name = session.resource_name(buf.resourceId)
            if min_size is not None and buf.length < min_size:
                continue
            if pattern and not pattern.search(name):
                continue
            results.append(serialize_buffer_desc(buf, name))
        results.sort(key=lambda b: -b["bytes"])
        return to_json({"buffers": results, "count": len(results),
                        "mb": round(sum(b["bytes"] for b in results) / 1e6, 1)})

    @mcp.tool()
    def list_resources(type_filter: Optional[str] = None, name_pattern: Optional[str] = None) -> str:
        """List all API resources (textures, buffers, views, samplers, shaders, pipelines,
        acceleration structures...) with names and types.

        Args:
            type_filter: Resource type substring (e.g. "Texture", "Shader", "PipelineState",
                         "AccelerationStructure").
            name_pattern: Regex matched against the name.
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)
        pattern = _name_pattern(name_pattern)
        if isinstance(pattern, dict):
            return to_json(pattern)

        results = []
        for res in session.controller.GetResources():
            entry = serialize_resource_desc(res)
            if type_filter and type_filter.lower() not in entry["type"].lower():
                continue
            if pattern and not pattern.search(entry["name"]):
                continue
            results.append(entry)
        return to_json({"resources": results, "count": len(results)})

    @mcp.tool()
    def get_resource_usage(resource_id: str) -> str:
        """Every event that reads or writes a resource, grouped by pass: which passes
        produce a texture and which consume it.

        Args:
            resource_id: Resource ID ("ResourceId::123" or "123") or unique debug name.
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)

        rid = session.resolve_resource_id(resource_id)
        if rid is None:
            return to_json(session.resource_error(resource_id))

        by_pass: list[dict] = []
        for usage in session.controller.GetUsage(rid):
            pass_ = session.pass_of(usage.eventId)
            pass_name = pass_["name"] if pass_ else "(outside passes)"
            kind = name_of(usage.usage, rd.ResourceUsage)
            if not by_pass or by_pass[-1]["pass"] != pass_name:
                by_pass.append({"pass": pass_name, "events": [], "usage": []})
            group = by_pass[-1]
            if usage.eventId not in group["events"]:
                group["events"].append(usage.eventId)
            if kind not in group["usage"]:
                group["usage"].append(kind)

        return to_json({**session.describe(rid), "usage_by_pass": by_pass,
                        "count": sum(len(g["events"]) for g in by_pass)})
