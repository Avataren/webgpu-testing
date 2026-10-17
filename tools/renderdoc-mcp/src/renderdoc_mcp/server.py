"""MCP server: registers the tools and prompts."""

from __future__ import annotations

import atexit

from mcp.server.mcpserver import MCPServer

from renderdoc_mcp.session import get_session
from renderdoc_mcp.tools import (
    session_tools,
    event_tools,
    pipeline_tools,
    resource_tools,
    data_tools,
    shader_tools,
    advanced_tools,
    performance_tools,
    diagnostic_tools,
)

mcp = MCPServer(
    "renderdoc",
    instructions=(
        "Analyzes RenderDoc frame captures (.rdc) of the wgpu engine. Take one with the engine "
        "MCP server: engine_launch {renderdoc: true}, then renderdoc_capture, which returns the "
        ".rdc path for open_capture. Start with get_frame_overview (passes in order, named after "
        "wgpu pass labels) and get_pass_timing; then inspect draws and dispatches with "
        "get_draw_call_state, the WGSL source with disassemble_shader, values with "
        "get_cbuffer_contents / pick_pixel / pixel_history, and step through a shader with "
        "debug_shader_at_pixel or debug_compute_thread. Resources can be named by debug name "
        "(wgpu label) instead of ResourceId."
    ),
)

for module in (session_tools, event_tools, pipeline_tools, resource_tools, data_tools,
               shader_tools, advanced_tools, performance_tools, diagnostic_tools):
    module.register(mcp)


@mcp.prompt()
def debug_draw_call(event_id: int) -> str:
    """Walk through one draw call: state, shaders, inputs and outputs."""
    return f"""Analyze the draw at event {event_id}:

1. get_draw_call_state(event_id={event_id}) for the pass, pipeline state, shaders and bound resources.
2. disassemble_shader for the vertex and fragment stages (WGSL source when embedded); use search= to find the relevant code.
3. get_cbuffer_contents for the uniform blocks the code reads (filter= narrows by name).
4. get_vertex_inputs / get_post_vs_data if the geometry looks wrong; export_mesh to see what was rasterized.
5. save_render_target(event_id={event_id}) and pixel_history on a suspicious pixel; debug_shader_at_pixel to step through the fragment shader there.
6. Summarize what the draw renders and anything wrong with it."""


@mcp.prompt()
def find_rendering_issue(description: str) -> str:
    """Systematically track down a visual problem."""
    return f"""The user reports: "{description}"

1. get_frame_overview: find the passes that produce the affected image.
2. For each candidate pass, save_texture of its outputs at the end of the pass (event_id = the pass's last event) to see where the problem first appears.
3. sample_pixel_region / get_texture_stats on that output for NaN, Inf, negative or out-of-range values; diagnose_negative_values scans every float target.
4. pixel_history on a bad pixel to find the draw or dispatch that wrote it.
5. debug_shader_at_pixel (draws) or debug_compute_thread (dispatches) at that pixel, watching the variables involved.
6. Cross-check uniforms with get_cbuffer_contents and bindings with get_shader_bindings.
7. Explain the root cause and point at the WGSL or Rust code to change."""


@mcp.prompt()
def analyze_performance() -> str:
    """Find where the frame's GPU time goes."""
    return """Analyze GPU cost:

1. get_pass_timing() for the most expensive passes (relative cost; replay times each event alone).
2. get_pass_timing(granularity="event", pass_name=<costly pass>) for its draws and dispatches; add counters=["PSInvocations","CSInvocations"] to see shading work.
3. analyze_overdraw(pass_name=<costly raster pass>) for quad overdraw.
4. analyze_state_changes(pass_name=...) for redundant state changes and instancing candidates.
5. For a costly shader, disassemble_shader with target "KHR_pipeline_executable_properties" (register usage, spills) or the ISA target.
6. Rank optimizations by expected gain."""


def _cleanup():
    get_session().shutdown()


atexit.register(_cleanup)


def main():
    mcp.run()


if __name__ == "__main__":
    main()
