"""Session tools: open_capture, close_capture, get_capture_info, get_frame_overview."""

from __future__ import annotations

import os

from mcp.server.mcpserver import MCPServer

from renderdoc_mcp.session import get_session
from renderdoc_mcp.util import rd, to_json, name_of


def register(mcp: MCPServer):
    @mcp.tool()
    def open_capture(filepath: str) -> str:
        """Open a RenderDoc capture (.rdc) for analysis, closing any open one.

        The engine's renderdoc_capture tool returns such a path. Returns the API, action,
        draw, dispatch and pass counts and how many resources carry debug names (wgpu
        labels; zero means the engine ran without WGPU_DEBUG=1).

        Args:
            filepath: Absolute path to the .rdc capture file.
        """
        filepath = os.path.normpath(os.path.expanduser(filepath))
        return to_json(get_session().open(filepath))

    @mcp.tool()
    def close_capture() -> str:
        """Close the open capture and free its replay resources."""
        return to_json(get_session().close())

    @mcp.tool()
    def get_capture_info() -> str:
        """The open capture's API, GPU vendor, frame number, file sizes and what the replay
        supports here (shader debugging, pixel history), plus API-call statistics and any
        debug messages recorded with the frame."""
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)

        controller = session.controller
        props = controller.GetAPIProperties()
        frame = controller.GetFrameInfo()
        info: dict = {
            "filepath": session.filepath,
            "api": session.driver_name,
            "vendor": name_of(props.vendor, rd.GPUVendor),
            "frame_number": frame.frameNumber,
            "file_mb": round(frame.compressedFileSize / 1e6, 1),
            "uncompressed_mb": round(frame.uncompressedFileSize / 1e6, 1),
            "replay_supports": {
                "shader_debugging": bool(props.shaderDebugging),
                "pixel_history": bool(props.pixelHistory),
            },
            "degraded_replay": bool(props.degraded),
            "current_event": session.current_event,
        }
        try:
            stats = frame.stats
            if stats.recorded:
                info["api_calls"] = {
                    "draws": stats.draws.calls,
                    "indirect_draws": stats.draws.indirect,
                    "dispatches": stats.dispatches.calls,
                    "resource_updates": stats.updates.calls,
                }
        except Exception:
            pass
        messages = [
            {"event_id": m.eventId, "severity": name_of(m.severity), "message": m.description}
            for m in controller.GetDebugMessages()
        ]
        if messages:
            info["debug_messages"] = messages[:50]
        return to_json(info)

    @mcp.tool()
    def get_frame_overview() -> str:
        """Frame-level overview: draw/dispatch/clear/copy counts, texture and buffer memory,
        the passes in order (wgpu pass labels) and the most-drawn render targets. Start
        here, then drill into a pass with list_actions or get_pass_timing."""
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)

        controller = session.controller
        counts = {"draws": 0, "dispatches": 0, "clears": 0, "copies": 0}
        rt_draws: dict[str, int] = {}
        for action in session.action_map.values():
            flags = action.flags
            if flags & rd.ActionFlags.Drawcall:
                counts["draws"] += 1
                for o in action.outputs:
                    if int(o) != 0:
                        rt_draws[str(o)] = rt_draws.get(str(o), 0) + 1
            if flags & rd.ActionFlags.Dispatch:
                counts["dispatches"] += 1
            if flags & rd.ActionFlags.Clear:
                counts["clears"] += 1
            if flags & rd.ActionFlags.Copy:
                counts["copies"] += 1

        textures = controller.GetTextures()
        buffers = controller.GetBuffers()
        render_targets = []
        for rid, n in sorted(rt_draws.items(), key=lambda x: -x[1])[:12]:
            entry = session.describe(rid)
            entry["draws"] = n
            render_targets.append(entry)

        passes = [
            {
                "index": p["index"],
                "name": p["name"],
                "events": [p["start_event"], p["end_event"]],
                "draws": p["draws"],
                "dispatches": p["dispatches"],
                "outputs": [session.label(o) for o in p["outputs"]],
            }
            for p in session.passes()
        ]

        return to_json({
            "api": session.driver_name,
            "total_actions": len(session.action_map),
            **counts,
            "textures": {"count": len(textures),
                         "mb": round(sum(t.byteSize for t in textures) / 1e6, 1)},
            "buffers": {"count": len(buffers),
                        "mb": round(sum(b.length for b in buffers) / 1e6, 1)},
            "passes": passes,
            "top_render_targets": render_targets,
        })
