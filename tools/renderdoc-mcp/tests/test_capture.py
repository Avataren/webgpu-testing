"""End-to-end: drive the server over MCP stdio against a real capture of the engine.

Replays on the GPU, so it only runs when RENDERDOC_MCP_TEST_CAPTURE names a .rdc taken
with WGPU_DEBUG=1 (engine_launch {renderdoc: true} sets it), e.g.

    RENDERDOC_MCP_TEST_CAPTURE=target/engine-mcp/captures/editor_frame123.rdc \
        tools/renderdoc-mcp/.venv/bin/python -m pytest tools/renderdoc-mcp/tests
"""

import asyncio
import json
import os
import sys

import pytest

CAPTURE = os.environ.get("RENDERDOC_MCP_TEST_CAPTURE")
pytestmark = pytest.mark.skipif(not CAPTURE, reason="set RENDERDOC_MCP_TEST_CAPTURE to a .rdc")


def run_session(steps):
    """Start the server, run `steps(call)` and return its result."""
    from mcp import ClientSession, StdioServerParameters
    from mcp.client.stdio import stdio_client

    async def main():
        params = StdioServerParameters(command=sys.executable, args=["-P", "-m", "renderdoc_mcp"],
                                       env=dict(os.environ))
        async with stdio_client(params) as (read, write):
            async with ClientSession(read, write) as session:
                await session.initialize()

                async def call(name, **args):
                    result = await session.call_tool(name, args)
                    text = "".join(getattr(c, "text", "") for c in result.content)
                    assert not result.is_error, f"{name}: {text}"
                    data = json.loads(text)
                    assert "error" not in data, f"{name}: {data}"
                    return data

                return await steps(call)

    return asyncio.run(main())


def test_capture_end_to_end(tmp_path):
    async def steps(call):
        opened = await call("open_capture", filepath=os.path.abspath(CAPTURE))
        assert opened["named_resources"] > 0, "capture has no debug names; was WGPU_DEBUG=1 set?"
        overview = await call("get_frame_overview")
        assert overview["passes"]
        named = [p for p in overview["passes"] if not p["name"].startswith("(")]
        assert named, "no wgpu pass labels found"

        draws = await call("find_draws", min_vertices=3, max_results=200)
        for match in draws["matches"]:
            draw = match["event_id"]
            state = await call("get_draw_call_state", event_id=draw)
            assert state["topology"] in ("TriangleList", "TriangleStrip", "LineList", "PointList")
            assert "warnings" not in state, state.get("warnings")
            assert state["depth"]["test"] in (True, False)
            if "pixel" in state["shaders"]:
                break
        else:
            pytest.fail("no draw with a fragment shader")

        source = await call("disassemble_shader", stage="fragment", event_id=draw)
        assert source["target"] == "source" and "fn " in source["text"]
        reflection = await call("get_shader_reflection", stage="vertex", event_id=draw)
        if reflection["uniform_blocks"]:
            values = await call("get_cbuffer_contents", stage="vertex", event_id=draw)
            assert values["variables"]

        timing = await call("get_pass_timing", top_n=5)
        assert timing["frame_gpu_ms"] > 0 and timing["passes"][0]["gpu_ms"] >= timing["passes"][-1]["gpu_ms"]
        await call("analyze_overdraw", top_n=3)
        await call("diagnose_negative_values", trace_depth=2)

        # Debug a fragment shader at a pixel its draw covers: a full-screen pass's draw.
        # RenderDoc cannot debug every shader (ray queries, for one), so try several.
        for p in overview["passes"]:
            if p["draws"] != 1 or p["dispatches"]:
                continue
            fullscreen = (await call("find_draws", pass_name=str(p["index"])))["matches"][0]["event_id"]
            actions = await call("get_action", event_id=fullscreen)
            target = actions["outputs"][0]
            width, height = (int(v) for v in target["size"].split("x")[:2])
            try:
                trace = await call("debug_shader_at_pixel", event_id=fullscreen,
                                   pixel_x=width // 2, pixel_y=height // 2)
            except AssertionError as e:
                if "cannot be debugged" in str(e):
                    continue
                raise
            assert trace["steps"] > 0 and trace["trace"]
            break
        else:
            return
        history = await call("pixel_history", resource_id=target["resource_id"],
                             x=width // 2, y=height // 2, event_id=fullscreen)
        assert any(m["event_id"] == fullscreen for m in history["modifications"])
        saved = await call("save_render_target", event_id=fullscreen,
                           output_path=str(tmp_path / "rt.png"))
        assert os.path.getsize(saved["saved"][0]["output_path"]) > 0

    run_session(steps)
