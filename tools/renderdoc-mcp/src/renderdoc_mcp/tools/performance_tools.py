"""Performance tools: get_pass_timing, analyze_overdraw, analyze_state_changes."""

from __future__ import annotations

from typing import Optional

from mcp.server.mcpserver import MCPServer

from renderdoc_mcp.session import get_session
from renderdoc_mcp.util import (
    rd,
    to_json,
    make_error,
    GRAPHICS_STAGES,
)


def _counter_values(session, counter) -> dict[int, float] | None:
    """Per-event values of one GPU counter (durations in milliseconds), cached per capture."""
    cache = getattr(session, "_counter_cache", None)
    if cache is None or cache.get("_file") != session.filepath:
        cache = {"_file": session.filepath}
        session._counter_cache = cache
    if counter in cache:
        return cache[counter]
    controller = session.controller
    if counter not in controller.EnumerateCounters():
        cache[counter] = None
        return None
    desc = controller.DescribeCounter(counter)
    values: dict[int, float] = {}
    for r in controller.FetchCounters([counter]):
        if desc.resultType == rd.CompType.Float:
            v = r.value.d if desc.resultByteWidth == 8 else r.value.f
        else:
            v = r.value.u64 if desc.resultByteWidth == 8 else r.value.u32
        if desc.unit == rd.CounterUnit.Seconds:
            v *= 1000.0
        values[r.eventId] = v
    cache[counter] = values
    # Counters replay the frame; put the replay back where the session thinks it is.
    if session.current_event is not None:
        controller.SetFrameEvent(session.current_event, True)
    return values


def _counter(name: str):
    return getattr(rd.GPUCounter, name, None)


def register(mcp: MCPServer):

    @mcp.tool()
    def get_pass_timing(
        granularity: str = "pass",
        top_n: int = 25,
        pass_name: Optional[str] = None,
        counters: Optional[list[str]] = None,
    ) -> str:
        """GPU time per pass or per draw/dispatch, measured by RenderDoc replaying the frame
        with a timestamp around each event (EventGPUDuration), most expensive first.

        Events run one at a time during this replay, so absolute numbers run higher than
        in the live frame; compare passes and draws relative to each other.

        Args:
            granularity: "pass" (default) or "event" (individual draws and dispatches).
            top_n: How many entries to return (default 25).
            pass_name: Only passes matching this regex (or a pass index).
            counters: Extra GPU counters summed per entry, e.g. ["PSInvocations",
                      "CSInvocations", "RasterizedPrimitives", "SamplesPassed",
                      "VSInvocations"].
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)

        durations = _counter_values(session, rd.GPUCounter.EventGPUDuration)
        if durations is None:
            return to_json(make_error("This replay has no GPU duration counter", "API_ERROR"))
        extra: dict[str, dict[int, float]] = {}
        for name in counters or []:
            counter = _counter(name)
            if counter is None:
                valid = [c.name for c in rd.GPUCounter if c.value < int(rd.GPUCounter.FirstAMD)]
                return to_json(make_error(f"Unknown counter {name}. Valid: {valid}", "API_ERROR"))
            values = _counter_values(session, counter)
            if values is not None:
                extra[name] = values

        passes = session.find_passes(pass_name) if pass_name is not None else session.passes()
        if not passes:
            return to_json(make_error(f"No pass matches '{pass_name}'", "API_ERROR"))
        frame_ms = sum(durations.values())
        sf = session.structured_file

        def add_counters(entry: dict, events) -> None:
            for name, values in extra.items():
                entry[name] = int(sum(values.get(e, 0) for e in events))

        if granularity in ("event", "draw_call", "action"):
            entries = []
            for p in passes:
                for eid in p["events"]:
                    action = session.get_action(eid)
                    entry = {
                        "event_id": eid,
                        "name": action.GetName(sf),
                        "pass": p["name"],
                        "gpu_ms": round(durations.get(eid, 0.0), 4),
                    }
                    if action.flags & rd.ActionFlags.Drawcall:
                        entry["indices"] = action.numIndices * max(action.numInstances, 1)
                    add_counters(entry, [eid])
                    entries.append(entry)
            entries.sort(key=lambda e: -e["gpu_ms"])
            return to_json({"granularity": "event", "frame_gpu_ms": round(frame_ms, 3),
                            "entries": entries[:top_n], "total_entries": len(entries)})

        entries = []
        for p in passes:
            ms = sum(durations.get(e, 0.0) for e in p["events"])
            entry = {
                "index": p["index"],
                "name": p["name"],
                "events": [p["start_event"], p["end_event"]],
                "gpu_ms": round(ms, 4),
                "share": f"{100 * ms / frame_ms:.1f}%" if frame_ms else None,
                "draws": p["draws"],
                "dispatches": p["dispatches"],
                "targets": [session.label(o) for o in p["outputs"]],
            }
            add_counters(entry, p["events"])
            entries.append(entry)
        entries.sort(key=lambda e: -e["gpu_ms"])
        # Copies, clears and barriers outside draws/dispatches also take time.
        other = frame_ms - sum(durations.get(e, 0.0) for p in session.passes() for e in p["events"])
        return to_json({
            "granularity": "pass",
            "frame_gpu_ms": round(frame_ms, 3),
            "outside_passes_ms": round(other, 3),
            "passes": entries[:top_n],
            "total_passes": len(entries),
        })

    @mcp.tool()
    def analyze_overdraw(
        pass_name: Optional[str] = None,
        event_id: Optional[int] = None,
        top_n: int = 10,
    ) -> str:
        """Measure quad overdraw per render pass with RenderDoc's quad-overdraw overlay: how
        many 2x2 fragment quads were shaded per pixel over the pass. Reports coverage, the
        mean over covered pixels, the maximum and the share of pixels shaded 4+ times.

        Args:
            pass_name: Only passes matching this regex (or a pass index); default: every
                       pass with draws.
            event_id: Measure the pass containing this draw, up to this draw.
            top_n: How many passes to return, worst mean first (default 10).
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)

        if event_id is not None:
            p = session.pass_of(event_id)
            if p is None or not session.get_action(event_id).flags & rd.ActionFlags.Drawcall:
                return to_json(make_error(f"Event {event_id} is not a draw inside a pass", "API_ERROR"))
            jobs = [(p, event_id)]
        else:
            passes = session.find_passes(pass_name) if pass_name is not None else session.passes()
            jobs = []
            for p in passes:
                draws = [e for e in p["events"] if session.get_action(e).flags & rd.ActionFlags.Drawcall]
                if draws:
                    jobs.append((p, draws[-1]))
        if not jobs:
            return to_json(make_error("No pass with draws matches", "API_ERROR"))

        controller = session.controller
        output = None
        results = []
        saved = session.current_event
        try:
            for p, last_draw in jobs:
                session.set_event(last_draw)
                state = controller.GetPipelineState()
                target = next((o.resource for o in state.GetOutputTargets() if int(o.resource) != 0), None)
                if target is None:
                    depth = state.GetDepthTarget()
                    target = depth.resource if int(depth.resource) != 0 else None
                tex = session.get_texture_desc(str(target)) if target is not None else None
                if tex is None:
                    continue
                if output is None:
                    output = controller.CreateOutput(
                        rd.CreateHeadlessWindowingData(tex.width, tex.height), rd.ReplayOutputType.Texture)
                display = rd.TextureDisplay()
                display.resourceId = target
                display.overlay = rd.DebugOverlay.QuadOverdrawPass
                output.SetTextureDisplay(display)
                output.Display()
                overlay = output.GetDebugOverlayTexID()
                sub = rd.Subresource(0, 0, 0)
                _lo, hi = controller.GetMinMax(overlay, sub, rd.CompType.Typeless)
                peak = hi.floatValue[0]
                entry = {"pass": p["name"], "event": last_draw, "target": session.label(target),
                         "max": round(peak, 1)}
                if peak > 0:
                    # Buckets span [0, peak]; values are whole quad counts.
                    buckets = controller.GetHistogram(overlay, sub, rd.CompType.Typeless, 0.0,
                                                      float(peak), (True, False, False, False))
                    width = peak / len(buckets)
                    total = sum(buckets)
                    covered = 0
                    weighted = 0.0
                    heavy = 0
                    for i, count in enumerate(buckets):
                        value = round((i + 0.5) * width)
                        if value >= 1:
                            covered += count
                            weighted += value * count
                        if value >= 4:
                            heavy += count
                    entry.update({
                        "coverage": f"{100 * covered / total:.1f}%" if total else None,
                        "mean_on_covered": round(weighted / covered, 2) if covered else 0,
                        "pixels_4plus": f"{100 * heavy / total:.1f}%" if total else None,
                    })
                results.append(entry)
        finally:
            if output is not None:
                output.Shutdown()
            if saved is not None:
                session.set_event(saved)

        results.sort(key=lambda e: -(e.get("mean_on_covered") or 0))
        return to_json({"passes": results[:top_n], "measured": len(results),
                        "note": "Quad counts: a fragment quad is 2x2 pixels, so tiny triangles "
                                "inflate them even without layered geometry."})

    @mcp.tool()
    def analyze_state_changes(
        pass_name: Optional[str] = None,
        change_types: Optional[list[str]] = None,
        max_draws: int = 400,
    ) -> str:
        """Count pipeline-state changes between consecutive draws and find runs of draws
        with identical state (candidates for instancing or merging).

        Args:
            pass_name: Only draws in passes matching this regex (or a pass index).
            change_types: Aspects to track; default all of: pipeline, shader, blend, depth,
                          cull, render_target, vertex_buffers.
            max_draws: Stop after this many draws (each one is replayed; default 400).
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)

        valid = {"pipeline", "shader", "blend", "depth", "cull", "render_target", "vertex_buffers"}
        track = set(change_types) if change_types else valid
        if track - valid:
            return to_json(make_error(f"Unknown change_types: {sorted(track - valid)}. Valid: {sorted(valid)}", "API_ERROR"))

        passes = session.find_passes(pass_name) if pass_name is not None else session.passes()
        draws = [(p["name"], e) for p in passes for e in p["events"]
                 if session.get_action(e).flags & rd.ActionFlags.Drawcall]
        if not draws:
            return to_json(make_error("No draws found", "API_ERROR"))

        saved = session.current_event
        counts = {t: 0 for t in sorted(track)}
        runs: list[dict] = []
        run = None
        prev = None
        prev_pass = None
        for pass_label, eid in draws[:max_draws]:
            session.set_event(eid)
            state = session.controller.GetPipelineState()
            cur: dict = {}
            if "pipeline" in track:
                cur["pipeline"] = str(state.GetGraphicsPipelineObject())
            if "shader" in track:
                cur["shader"] = tuple(str(state.GetShader(s)) for _, s in GRAPHICS_STAGES)
            if "blend" in track:
                blends = state.GetColorBlends()
                b = blends[0] if len(blends) else None
                cur["blend"] = (bool(b.enabled), int(b.colorBlend.source), int(b.colorBlend.destination)) if b else None
            if "depth" in track:
                ds = state.GetDepthTestState()
                cur["depth"] = (bool(ds.depthEnable), bool(ds.depthWrites), int(ds.depthFunction))
            if "cull" in track:
                cur["cull"] = int(state.GetRasterState().cullMode)
            if "render_target" in track:
                cur["render_target"] = tuple(str(o) for o in session.get_action(eid).outputs if int(o) != 0)
            if "vertex_buffers" in track:
                cur["vertex_buffers"] = tuple(str(vb.resourceId) for vb in state.GetVBuffers())

            changed = []
            if prev is not None and pass_label == prev_pass:
                changed = [t for t in track if cur.get(t) != prev.get(t)]
                for t in changed:
                    counts[t] += 1
            if prev is not None and pass_label == prev_pass and not changed:
                run["end_event"] = eid
                run["count"] += 1
            else:
                if run and run["count"] >= 2:
                    runs.append(run)
                run = {"pass": pass_label, "start_event": eid, "end_event": eid, "count": 1}
            prev, prev_pass = cur, pass_label
        if run and run["count"] >= 2:
            runs.append(run)
        if saved is not None:
            session.set_event(saved)

        runs.sort(key=lambda r: -r["count"])
        analyzed = min(len(draws), max_draws)
        return to_json({
            "analyzed_draws": analyzed,
            "total_draws": len(draws),
            "changes": counts,
            "identical_state_runs": runs[:10],
            "note": "Changes are counted within a pass only.",
        })
