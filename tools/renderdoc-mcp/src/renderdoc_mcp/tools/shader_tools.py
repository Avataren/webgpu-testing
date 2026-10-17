"""Shader tools: disassemble_shader, get_shader_reflection, get_cbuffer_contents."""

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
    parse_stage,
    stage_error,
    serialize_shader_variable,
    serialize_sig_element,
)

SOURCE_TARGET = "source"
MAX_UNFILTERED_LINES = 400


def _shader_at(session, stage: str, event_id):
    """(state, stage enum, reflection) or an error dict."""
    err = session.require_open() or session.ensure_event(event_id)
    if err:
        return err
    stage_enum = parse_stage(stage)
    if stage_enum is None:
        return stage_error(stage)
    state = session.controller.GetPipelineState()
    refl = state.GetShaderReflection(stage_enum)
    if refl is None:
        return make_error(f"No shader bound at stage '{stage}' at event {session.current_event}", "API_ERROR")
    return state, stage_enum, refl


def _pipeline_object(state, stage_enum):
    if stage_enum == rd.ShaderStage.Compute:
        return state.GetComputePipelineObject()
    return state.GetGraphicsPipelineObject()


def source_files(refl) -> list:
    """Shader source embedded in the capture: wgpu's WGSL when built with WGPU_DEBUG=1."""
    try:
        return [f for f in refl.debugInfo.files if f.contents]
    except Exception:
        return []


def _filter_lines(text: str, line_range, search: Optional[str]) -> tuple[str, str | None]:
    lines = text.splitlines()
    total = len(lines)
    if line_range is not None and len(line_range) == 2:
        start = max(1, line_range[0])
        end = min(total, line_range[1])
        body = "\n".join(f"{i:5d}: {lines[i - 1]}" for i in range(start, end + 1))
        return body, f"Lines {start}-{end} of {total}"
    if search:
        try:
            pattern = re.compile(search, re.IGNORECASE)
        except re.error:
            pattern = re.compile(re.escape(search), re.IGNORECASE)
        context = 4
        hits = [i for i, line in enumerate(lines) if pattern.search(line)]
        if not hits:
            return "", f"No matches for '{search}' in {total} lines"
        include = sorted({j for i in hits for j in range(max(0, i - context), min(total, i + context + 1))})
        out, prev = [], -2
        for i in include[:600]:
            if i > prev + 1:
                out.append("...")
            out.append(f"{i + 1:5d}: {lines[i]}")
            prev = i
        return "\n".join(out), f"{len(hits)} matching lines of {total}"
    if total > MAX_UNFILTERED_LINES:
        body = "\n".join(f"{i + 1:5d}: {lines[i]}" for i in range(MAX_UNFILTERED_LINES))
        return body, f"First {MAX_UNFILTERED_LINES} of {total} lines; use line_range or search for the rest"
    return "\n".join(f"{i + 1:5d}: {line}" for i, line in enumerate(lines)), None


def register(mcp: MCPServer):
    @mcp.tool()
    def disassemble_shader(
        stage: str,
        target: Optional[str] = None,
        event_id: Optional[int] = None,
        line_range: Optional[list[int]] = None,
        search: Optional[str] = None,
    ) -> str:
        """Show the shader bound at a stage: its WGSL source when the capture embeds it
        (wgpu with WGPU_DEBUG=1), else a disassembly. Other targets include "SPIR-V
        (RenderDoc)" and, on AMD, "AMD GCN ISA" and "KHR_pipeline_executable_properties"
        (register usage and other driver statistics, for performance work).

        Args:
            stage: vertex, fragment (or pixel), compute, task, mesh, ...
            target: "source" or a disassembly target; omitted = source if present.
            event_id: Event to inspect (default: the current event).
            line_range: [first, last] line numbers (1-based) to return.
            search: Regex; returns matching lines with 4 lines of context.
        """
        session = get_session()
        found = _shader_at(session, stage, event_id)
        if isinstance(found, dict):
            return to_json(found)
        state, stage_enum, refl = found

        targets = list(session.controller.GetDisassemblyTargets(True))
        files = source_files(refl)
        available = ([SOURCE_TARGET] if files else []) + targets
        if target is None:
            target = available[0] if available else None
        if target not in available:
            return to_json(make_error(f"Unknown target '{target}'. Available: {available}", "API_ERROR"))

        result: dict = {
            "stage": stage,
            "event_id": session.current_event,
            "entry_point": refl.entryPoint,
            "target": target,
            "available_targets": available,
        }
        if target == SOURCE_TARGET:
            # wgpu embeds one file per module; several only when the source was split.
            text = "\n".join(f.contents for f in files)
            result["file"] = files[0].filename
            entry = re.search(rf"\bfn\s+{re.escape(refl.entryPoint)}\b", text)
            if entry:
                result["entry_line"] = text.count("\n", 0, entry.start()) + 1
        else:
            text = session.controller.DisassembleShader(_pipeline_object(state, stage_enum), refl, target)
        body, note = _filter_lines(text or "", line_range, search)
        result["total_lines"] = len((text or "").splitlines())
        result["text"] = body
        if note:
            result["note"] = note
        return to_json(result)

    @mcp.tool()
    def get_shader_reflection(
        stage: str,
        event_id: Optional[int] = None,
    ) -> str:
        """A shader's interface: inputs/outputs (with builtins), uniform blocks with their
        members and offsets, textures/storage resources and samplers with their
        (set, binding) numbers.

        Args:
            stage: vertex, fragment (or pixel), compute, task, mesh, ...
            event_id: Event to inspect (default: the current event).
        """
        session = get_session()
        found = _shader_at(session, stage, event_id)
        if isinstance(found, dict):
            return to_json(found)
        _state, _stage_enum, refl = found

        def binding(res) -> list[int]:
            return [res.fixedBindSetOrSpace, res.fixedBindNumber]

        def members(variables) -> list[dict]:
            out = []
            for v in variables:
                entry = {"name": v.name, "offset": v.byteOffset,
                         "type": v.type.name or name_of(v.type.baseType, rd.VarType)}
                if v.type.elements > 1:
                    entry["elements"] = v.type.elements
                out.append(entry)
            return out

        result: dict = {
            "stage": stage,
            "event_id": session.current_event,
            "resource_id": str(refl.resourceId),
            "entry_point": refl.entryPoint,
            "inputs": [serialize_sig_element(s) for s in refl.inputSignature],
            "outputs": [serialize_sig_element(s) for s in refl.outputSignature],
            "uniform_blocks": [
                {"index": i, "name": cb.name, "binding": binding(cb), "bytes": cb.byteSize,
                 "members": members(cb.variables)}
                for i, cb in enumerate(refl.constantBlocks)
            ],
            "read_only": [
                {"index": i, "name": r.name, "binding": binding(r),
                 "kind": name_of(r.descriptorType, rd.DescriptorType),
                 "type": name_of(r.textureType, rd.TextureType)}
                for i, r in enumerate(refl.readOnlyResources)
            ],
            "read_write": [
                {"index": i, "name": r.name, "binding": binding(r),
                 "kind": name_of(r.descriptorType, rd.DescriptorType),
                 "type": name_of(r.textureType, rd.TextureType)}
                for i, r in enumerate(refl.readWriteResources)
            ],
            "samplers": [{"index": i, "name": s.name, "binding": binding(s)}
                         for i, s in enumerate(refl.samplers)],
        }
        if refl.stage == rd.ShaderStage.Compute:
            result["workgroup_size"] = list(refl.dispatchThreadsDimension)
        files = source_files(refl)
        result["source_embedded"] = bool(files)
        return to_json(result)

    @mcp.tool()
    def get_cbuffer_contents(
        stage: str,
        cbuffer_index: int = 0,
        cbuffer_name: Optional[str] = None,
        event_id: Optional[int] = None,
        filter: Optional[str] = None,
    ) -> str:
        """The values in a uniform (constant) buffer as a shader stage sees them at an event,
        read from the bound buffer range.

        Args:
            stage: vertex, fragment (or pixel), compute, ...
            cbuffer_index: Index of the block (see get_shader_reflection); default 0.
            cbuffer_name: Block name instead of the index (e.g. "globals", "camera").
            event_id: Event to inspect (default: the current event).
            filter: Case-insensitive substring; only variables whose name (or a member's
                    name) contains it are returned.
        """
        session = get_session()
        found = _shader_at(session, stage, event_id)
        if isinstance(found, dict):
            return to_json(found)
        state, stage_enum, refl = found

        blocks = list(refl.constantBlocks)
        if not blocks:
            return to_json(make_error(f"The {stage} shader has no uniform blocks", "API_ERROR"))
        if cbuffer_name is not None:
            names = [b.name for b in blocks]
            if cbuffer_name not in names:
                return to_json(make_error(f"No uniform block '{cbuffer_name}'. Blocks: {names}", "API_ERROR"))
            cbuffer_index = names.index(cbuffer_name)
        if not 0 <= cbuffer_index < len(blocks):
            return to_json(make_error(f"cbuffer_index {cbuffer_index} out of range (0-{len(blocks) - 1})", "API_ERROR"))

        desc = state.GetConstantBlock(stage_enum, cbuffer_index, 0).descriptor
        variables = session.controller.GetCBufferVariableContents(
            _pipeline_object(state, stage_enum), refl.resourceId, stage_enum,
            state.GetShaderEntryPoint(stage_enum), cbuffer_index,
            desc.resource, desc.byteOffset, desc.byteSize,
        )
        values = [serialize_shader_variable(v) for v in variables]
        # A WGSL uniform struct arrives wrapped in one member; show its fields directly.
        if (len(values) == 1 and "members" in values[0]
                and (values[0]["name"].startswith("_child") or values[0]["name"] == blocks[cbuffer_index].name)):
            values = values[0]["members"]

        if filter:
            kw = filter.lower()

            def matches(var: dict) -> bool:
                return kw in var.get("name", "").lower() or any(matches(m) for m in var.get("members", []))

            values = [v for v in values if matches(v)]

        result = {
            "stage": stage,
            "event_id": session.current_event,
            "block": blocks[cbuffer_index].name,
            "buffer": session.describe(desc.resource) if int(desc.resource) else None,
            "offset": desc.byteOffset,
            "variables": values,
        }
        return to_json(result)
