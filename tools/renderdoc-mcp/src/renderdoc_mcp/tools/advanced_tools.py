"""Advanced tools: pixel_history, debug_shader_at_pixel, debug_compute_thread,
get_post_vs_data, diff_draw_calls, analyze_render_passes, sample_pixel_region."""

from __future__ import annotations

import bisect
import math
import struct
from typing import Optional

from mcp.server.mcpserver import MCPServer

from renderdoc_mcp.session import get_session
from renderdoc_mcp.util import (
    rd,
    to_json,
    make_error,
    shader_values,
    MESH_DATA_STAGE_MAP,
)

MAX_TRACE_LINES = 160


def _color_target(session, state):
    return next((o.resource for o in state.GetOutputTargets() if int(o.resource) != 0), None)


def _rgba(value) -> list:
    return [round(v, 6) for v in value.floatValue[:4]]


# ── Shader debugging ──

def _flatten_register(var, prefix: str = "") -> dict[str, list]:
    """Debugger register (ShaderVariable) → {name: component values}, structs flattened."""
    name = prefix + var.name
    if len(var.members) == 0:
        return {name: shader_values(var)}
    out: dict = {}
    for m in var.members:
        out.update(_flatten_register(m, name + "."))
    return out


def _source_file(files, index: int):
    if index < 0 or not files:
        return None
    # wgpu embeds a single WGSL file, which line info numbers from 1.
    if len(files) == 1:
        return files[0]
    return files[index] if index < len(files) else files[-1]


def _run_trace(session, trace, refl, watch: Optional[list[str]]) -> dict:
    """Step a shader debug trace to the end and summarize it at the source level."""
    controller = session.controller
    files = [f for f in refl.debugInfo.files if f.contents]
    file_lines: dict[int, list[str]] = {}

    infos = sorted(trace.instInfo, key=lambda i: i.instruction)
    info_keys = [i.instruction for i in infos]

    def info_for(instruction):
        if instruction is None or not infos:
            return None
        pos = bisect.bisect_right(info_keys, instruction) - 1
        return infos[pos] if pos >= 0 else None

    def code_at(line_info) -> str | None:
        f = _source_file(files, line_info.fileIndex)
        if f is None or line_info.lineStart <= 0:
            return None
        key = id(f)
        if key not in file_lines:
            file_lines[key] = f.contents.splitlines()
        lines = file_lines[key]
        return lines[line_info.lineStart - 1].strip() if line_info.lineStart <= len(lines) else None

    watch_l = [w.lower() for w in watch] if watch else None

    def watched(name: str) -> bool:
        # naga's loop_bound counters and unnamed temporaries are not WGSL variables.
        if name == "loop_bound" or (name.startswith("_") and not name.startswith("_output")):
            return False
        return watch_l is None or any(w in name.lower() for w in watch_l)

    registers: dict[str, list] = {}

    def source_value(mapping):
        values = []
        for ref in mapping.variables:
            reg = registers.get(ref.name)
            if reg is None or ref.component >= len(reg):
                return None
            values.append(reg[ref.component])
        return values[0] if len(values) == 1 else values

    entries: list[dict] = []
    finals: dict[str, object] = {}
    steps = 0
    prev_instruction = None
    while True:
        states = controller.ContinueDebug(trace.debugger)
        if not states:
            break
        for state in states:
            steps += 1
            changed = set()
            for change in state.changes:
                if change.after.name:
                    flat = _flatten_register(change.after)
                    registers.update(flat)
                    changed.update(flat)
            info = info_for(prev_instruction) if prev_instruction is not None else None
            prev_instruction = state.nextInstruction
            if info is None or not changed:
                continue
            values = {}
            for mapping in info.sourceVars:
                if not watched(mapping.name):
                    continue
                if any(ref.name in changed for ref in mapping.variables):
                    value = source_value(mapping)
                    if value is not None:
                        values[mapping.name] = value
                        finals[mapping.name] = value
            if not values:
                continue
            line = info.lineInfo.lineStart
            if entries and entries[-1]["line"] == line:
                entries[-1]["values"].update(values)
            else:
                entry = {"line": line, "values": values}
                code = code_at(info.lineInfo)
                if code:
                    entry["code"] = code
                entries.append(entry)

    for mapping in trace.sourceVars:
        if mapping.name.startswith("_output") or (watch_l and watched(mapping.name)):
            value = source_value(mapping)
            if value is not None:
                finals[mapping.name] = value

    for entry in entries:
        for name, value in entry["values"].items():
            entry["values"][name] = _mark(value)
    result: dict = {"steps": steps, "source_lines_executed": len(entries)}
    if len(entries) > MAX_TRACE_LINES:
        head = MAX_TRACE_LINES // 3
        tail = MAX_TRACE_LINES - head
        result["trace"] = entries[:head] + [{"skipped_lines": len(entries) - MAX_TRACE_LINES}] + entries[-tail:]
    else:
        result["trace"] = entries
    result["final_values"] = {k: _mark(v) for k, v in finals.items()}
    if not files:
        result["note"] = ("No source embedded (run the engine with WGPU_DEBUG=1); lines refer to "
                          "the SPIR-V disassembly")
    return result


def _mark(value):
    """Flag NaN/Inf in a debugger value so they stand out."""
    flat = value if isinstance(value, list) else [value]
    if any(isinstance(v, float) and math.isnan(v) for v in flat):
        return {"value": value, "anomaly": "NaN"}
    if any(isinstance(v, float) and math.isinf(v) for v in flat):
        return {"value": value, "anomaly": "Inf"}
    return value


def register(mcp: MCPServer):

    @mcp.tool()
    def pixel_history(
        resource_id: str,
        x: int,
        y: int,
        event_id: Optional[int] = None,
    ) -> str:
        """Every event that touched one pixel of a render target or storage texture up to
        an event: the pass, the value before and after, the shader output, and why a
        fragment was rejected (depth/stencil test, culling, scissor, discard).

        Args:
            resource_id: Texture ID or unique debug name.
            x: X coordinate (origin top-left).
            y: Y coordinate.
            event_id: Last event to consider (default: the current event, or the end of the frame).
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)
        if event_id is None and session.current_event is None:
            event_id = max(session.action_map)
        err = session.ensure_event(event_id)
        if err:
            return to_json(err)
        tex_id = session.resolve_resource_id(resource_id)
        if tex_id is None:
            return to_json(session.resource_error(resource_id))

        history = session.controller.PixelHistory(tex_id, x, y, rd.Subresource(0, 0, 0),
                                                  rd.CompType.Typeless)
        reasons = [("backfaceCulled", "backface_culled"), ("depthTestFailed", "depth_test"),
                   ("stencilTestFailed", "stencil_test"), ("scissorClipped", "scissor"),
                   ("shaderDiscarded", "discard"), ("depthClipped", "depth_clip"),
                   ("depthBoundsFailed", "depth_bounds"), ("sampleMasked", "sample_mask"),
                   ("viewClipped", "view_clip"), ("predicationSkipped", "predication")]
        results = []
        for mod in history:
            pass_ = session.pass_of(mod.eventId)
            entry: dict = {
                "event_id": mod.eventId,
                "pass": pass_["name"] if pass_ else None,
                "passed": bool(mod.Passed()),
                "before": _rgba(mod.preMod.col),
                "after": _rgba(mod.postMod.col),
            }
            if mod.preMod.depth >= 0 or mod.postMod.depth >= 0:
                entry["depth"] = [round(mod.preMod.depth, 7), round(mod.postMod.depth, 7)]
            if not mod.unboundPS and not mod.directShaderWrite:
                entry["shader_output"] = _rgba(mod.shaderOut.col)
            if mod.directShaderWrite:
                entry["storage_write"] = True
            if mod.primitiveID >= 0 and not mod.directShaderWrite:
                entry["primitive"] = mod.primitiveID
            failed = [label for attr, label in reasons if getattr(mod, attr, False)]
            if failed:
                entry["rejected_by"] = failed
            results.append(entry)

        return to_json({**session.describe(tex_id), "x": x, "y": y,
                        "up_to_event": session.current_event, "modifications": results})

    @mcp.tool()
    def debug_shader_at_pixel(
        event_id: int,
        pixel_x: int,
        pixel_y: int,
        watch_variables: Optional[list[str]] = None,
        sample: Optional[int] = None,
    ) -> str:
        """Run the fragment shader of a draw for one pixel in RenderDoc's debugger and return
        a source-level trace: each WGSL line executed with the values of the variables it
        set, and the final values (NaN/Inf flagged). Needs the shader source embedded
        (WGPU_DEBUG=1) for line text and variable names. RenderDoc cannot debug shaders
        that use ray queries; it says so.

        Args:
            event_id: The draw.
            pixel_x: X coordinate in the render target (origin top-left).
            pixel_y: Y coordinate.
            watch_variables: Only report variables whose name contains one of these.
            sample: MSAA sample to debug (default: let RenderDoc choose).
        """
        session = get_session()
        err = session.require_open() or session.set_event(event_id)
        if err:
            return to_json(err)
        state = session.controller.GetPipelineState()
        refl = state.GetShaderReflection(rd.ShaderStage.Pixel)
        if refl is None:
            return to_json(make_error("No fragment shader bound at this event", "API_ERROR"))
        if not refl.debugInfo.debuggable:
            return to_json(make_error(f"Shader cannot be debugged: {refl.debugInfo.debugStatus}", "API_ERROR"))

        inputs = rd.DebugPixelInputs()
        inputs.sample = sample if sample is not None else rd.ReplayController.NoPreference
        inputs.view = rd.ReplayController.NoPreference
        inputs.primitive = rd.ReplayController.NoPreference
        result: dict = {"event_id": event_id, "pixel": [pixel_x, pixel_y],
                        "entry_point": refl.entryPoint}

        target = _color_target(session, state)
        if target is not None:
            history = session.controller.PixelHistory(target, pixel_x, pixel_y,
                                                      rd.Subresource(0, 0, 0), rd.CompType.Typeless)
            mine = [m for m in history if m.eventId == event_id]
            if not mine:
                writers = sorted({m.eventId for m in history})
                return to_json(make_error(
                    f"Event {event_id} does not touch pixel ({pixel_x}, {pixel_y}) of its color "
                    f"target; events that do: {writers[-12:]}", "API_ERROR"))
            passed = [m for m in mine if m.Passed()]
            chosen = passed[-1] if passed else mine[-1]
            inputs.primitive = chosen.primitiveID
            result["fragment_passed"] = bool(chosen.Passed())
            result["output_written"] = _rgba(chosen.postMod.col)

        session.controller.SetFrameEvent(event_id, True)
        trace = session.controller.DebugPixel(pixel_x, pixel_y, inputs)
        if trace is None or trace.debugger is None:
            if trace is not None:
                session.controller.FreeTrace(trace)
            return to_json(make_error("RenderDoc could not debug this pixel", "API_ERROR"))
        try:
            result.update(_run_trace(session, trace, refl, watch_variables))
        finally:
            session.controller.FreeTrace(trace)
        return to_json(result)

    @mcp.tool()
    def debug_compute_thread(
        event_id: int,
        group: list[int],
        thread: list[int],
        watch_variables: Optional[list[str]] = None,
    ) -> str:
        """Run one compute shader invocation of a dispatch in RenderDoc's debugger and return
        a source-level trace (see debug_shader_at_pixel). The global invocation id is
        group * workgroup_size + thread.

        Args:
            event_id: The dispatch.
            group: Workgroup id [x, y, z].
            thread: Local invocation id within the workgroup [x, y, z].
            watch_variables: Only report variables whose name contains one of these.
        """
        session = get_session()
        err = session.require_open() or session.set_event(event_id)
        if err:
            return to_json(err)
        action = session.get_action(event_id)
        if not action.flags & rd.ActionFlags.Dispatch:
            return to_json(make_error(f"Event {event_id} is not a dispatch", "API_ERROR"))
        state = session.controller.GetPipelineState()
        refl = state.GetShaderReflection(rd.ShaderStage.Compute)
        if refl is None or not refl.debugInfo.debuggable:
            status = refl.debugInfo.debugStatus if refl is not None else "no compute shader"
            return to_json(make_error(f"Shader cannot be debugged: {status}", "API_ERROR"))
        if len(group) != 3 or len(thread) != 3:
            return to_json(make_error("group and thread take three numbers each", "API_ERROR"))
        dims = list(action.dispatchDimension)
        if any(g >= d for g, d in zip(group, dims)):
            return to_json(make_error(f"group {group} is outside the dispatch {dims}", "API_ERROR"))

        trace = session.controller.DebugThread(tuple(group), tuple(thread))
        if trace is None or trace.debugger is None:
            if trace is not None:
                session.controller.FreeTrace(trace)
            return to_json(make_error("RenderDoc could not debug this thread", "API_ERROR"))
        result: dict = {"event_id": event_id, "group": group, "thread": thread,
                        "entry_point": refl.entryPoint,
                        "workgroup_size": list(refl.dispatchThreadsDimension)}
        try:
            result.update(_run_trace(session, trace, refl, watch_variables))
        finally:
            session.controller.FreeTrace(trace)
        return to_json(result)

    @mcp.tool()
    def get_post_vs_data(
        stage: str = "vsout",
        max_vertices: int = 32,
        event_id: Optional[int] = None,
    ) -> str:
        """Vertex data of a draw after the vertex shader (vsout, the default) or geometry
        shader (gsout): each output by name, per vertex in index order.

        Args:
            stage: vsout or gsout.
            max_vertices: How many vertices to return (default 32).
            event_id: The draw (default: the current event).
        """
        from renderdoc_mcp.tools.data_tools import _post_vs_layout, _post_vs_indices

        session = get_session()
        err = session.require_open() or session.ensure_event(event_id)
        if err:
            return to_json(err)
        mesh_stage = MESH_DATA_STAGE_MAP.get(stage.lower())
        if mesh_stage is None or stage.lower() == "vsin":
            return to_json(make_error("stage must be vsout or gsout; read inputs with get_vertex_inputs", "API_ERROR"))
        mesh = session.controller.GetPostVSData(0, 0, mesh_stage)
        if int(mesh.vertexResourceId) == 0:
            return to_json(make_error("No post-transform data for this event", "API_ERROR"))

        state = session.controller.GetPipelineState()
        shader_stage = rd.ShaderStage.Geometry if stage.lower() == "gsout" else rd.ShaderStage.Vertex
        refl = state.GetShaderReflection(shader_stage)
        if refl is None:
            return to_json(make_error("No shader bound for that stage", "API_ERROR"))
        layout = _post_vs_layout(refl, state.HasAlignedPostVSData(mesh_stage))
        indices = _post_vs_indices(session, mesh)[:max(1, max_vertices)]
        stride = mesh.vertexByteStride
        needed = (max(indices) + 1) * stride if indices else 0
        data = session.controller.GetBufferData(mesh.vertexResourceId, mesh.vertexByteOffset, needed)

        vertices = []
        for index in indices:
            base = index * stride
            vertex: dict = {"index": index}
            for name, (offset, count, code, _builtin) in layout.items():
                size = struct.calcsize(f"<{count}{code}")
                if base + offset + size <= len(data):
                    values = struct.unpack_from(f"<{count}{code}", data, base + offset)
                    vertex[name] = [round(v, 5) if isinstance(v, float) else v for v in values]
            vertices.append(vertex)

        return to_json({
            "stage": stage,
            "event_id": session.current_event,
            "outputs": {name: {"components": c, "builtin": b} if b else {"components": c}
                        for name, (_o, c, _code, b) in layout.items()},
            "total_indices": mesh.numIndices,
            "vertices": vertices,
        })

    @mcp.tool()
    def diff_draw_calls(eid1: int, eid2: int) -> str:
        """Compare the full state of two draws (or dispatches): pipeline state, shaders,
        targets and blend, and each stage's bound resources. Returns only what differs.

        Args:
            eid1: First event.
            eid2: Second event.
        """
        from renderdoc_mcp.tools.pipeline_tools import _get_draw_state_dict

        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)
        state1 = _get_draw_state_dict(session, eid1)
        if "error" in state1:
            return to_json(state1)
        state2 = _get_draw_state_dict(session, eid2)
        if "error" in state2:
            return to_json(state2)
        for s in (state1, state2):
            s.pop("event_id", None)
            s.pop("action", None)

        differences = _flatten_diff(_diff(state1, state2))
        return to_json({"eid1": eid1, "eid2": eid2, "identical": not differences,
                        "differences": differences})

    @mcp.tool()
    def analyze_render_passes() -> str:
        """The frame's passes in order, from wgpu's pass labels (debug markers): event
        range, draw and dispatch counts, and the targets written with size and format.
        Work outside any marker is grouped by render-target changes."""
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)
        passes = []
        for p in session.passes():
            passes.append({
                "index": p["index"],
                "name": p["name"],
                "events": [p["start_event"], p["end_event"]],
                "draws": p["draws"],
                "dispatches": p["dispatches"],
                "targets": [session.describe(o) for o in p["outputs"]],
            })
        return to_json({"passes": passes, "count": len(passes)})

    @mcp.tool()
    def sample_pixel_region(
        event_id: Optional[int] = None,
        resource_id: Optional[str] = None,
        region: Optional[dict] = None,
        sample_count: int = 256,
        anomaly_threshold: float = 10.0,
    ) -> str:
        """Sample a grid of pixels from a texture and report NaN / Inf / negative / very
        bright values with their locations, plus per-channel statistics. Use the hotspots
        with pixel_history or debug_shader_at_pixel.

        Args:
            event_id: Event to sample at (default: the current event).
            resource_id: Texture to sample (default: the event's first color target).
            region: {"x", "y", "width", "height"} to sample (default: the whole texture).
            sample_count: Number of samples (default 256, max 4096).
            anomaly_threshold: Values above this count as very bright (default 10).
        """
        session = get_session()
        err = session.require_open() or session.ensure_event(event_id)
        if err:
            return to_json(err)

        if resource_id is not None:
            tex_id = session.resolve_resource_id(resource_id)
            if tex_id is None:
                return to_json(session.resource_error(resource_id))
        else:
            tex_id = _color_target(session, session.controller.GetPipelineState())
            if tex_id is None:
                return to_json(make_error("No color target at this event; pass resource_id", "API_ERROR"))
        tex_desc = session.get_texture_desc(str(tex_id))
        if tex_desc is None:
            return to_json(make_error("Not a texture", "API_ERROR"))

        w, h = tex_desc.width, tex_desc.height
        region = region or {}
        rx, ry = max(0, region.get("x", 0)), max(0, region.get("y", 0))
        rw = max(1, min(region.get("width", w), w - rx))
        rh = max(1, min(region.get("height", h), h - ry))
        sample_count = max(1, min(sample_count, 4096))
        cols = max(1, int(math.sqrt(sample_count * rw / rh)))
        rows = max(1, sample_count // cols)
        points = [(rx + (c * rw) // cols + rw // (2 * cols), ry + (r * rh) // rows + rh // (2 * rows))
                  for r in range(rows) for c in range(cols)]

        counts = {"nan": 0, "inf": 0, "negative": 0, "bright": 0}
        hotspots: list[dict] = []
        channels: list[list[float]] = [[], [], [], []]
        sub = rd.Subresource(0, 0, 0)
        for px, py in points:
            v = _rgba(session.controller.PickPixel(tex_id, px, py, sub, rd.CompType.Typeless))
            rgb = v[:3]
            kind = None
            if any(math.isnan(c) for c in rgb):
                kind = "nan"
            elif any(math.isinf(c) for c in rgb):
                kind = "inf"
            elif any(c < 0 for c in rgb):
                kind = "negative"
            elif max(rgb) > anomaly_threshold:
                kind = "bright"
            if kind:
                counts[kind] += 1
                if len(hotspots) < 20:
                    hotspots.append({"pixel": [px, py], "value": v, "type": kind})
            if not any(math.isnan(c) or math.isinf(c) for c in v):
                for i in range(4):
                    channels[i].append(v[i])

        stats = {
            ch: {"min": round(min(vals), 5), "max": round(max(vals), 5),
                 "mean": round(sum(vals) / len(vals), 5)}
            for ch, vals in zip("rgba", channels) if vals
        }
        return to_json({
            **session.describe(tex_id),
            "event_id": session.current_event,
            "samples": len(points),
            "anomalies": counts,
            "hotspots": hotspots,
            "statistics": stats,
        })


def _diff(a, b):
    if isinstance(a, dict) and isinstance(b, dict):
        out = {}
        for key in list(a) + [k for k in b if k not in a]:
            if a.get(key) != b.get(key):
                out[key] = _diff(a.get(key), b.get(key))
        return out
    return {"eid1": a, "eid2": b}


def _flatten_diff(diff: dict, path: str = "") -> list[dict]:
    out = []
    for key, value in diff.items():
        key_path = f"{path}.{key}" if path else key
        if isinstance(value, dict) and set(value) != {"eid1", "eid2"}:
            out.extend(_flatten_diff(value, key_path))
        else:
            out.append({"field": key_path, "eid1": value["eid1"], "eid2": value["eid2"]})
    return out
