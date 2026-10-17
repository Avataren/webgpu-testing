# RenderDoc MCP server

An MCP server that analyzes [RenderDoc](https://renderdoc.org/) frame captures (`.rdc`)
of the engine through RenderDoc's Python replay API: passes, draws and dispatches,
pipeline state, WGSL source, uniform values, textures, pixel history, per-pixel and
per-thread shader debugging, GPU timings and overdraw. It is registered in `.mcp.json`
as `renderdoc` and started by `run.sh`.

It started as [Linkingooo/renderdoc-mcp](https://github.com/Linkingooo/renderdoc-mcp)
(MIT, see `LICENSE`) and was reworked for RenderDoc 1.46, the MCP Python SDK 2.x and this
engine's wgpu/Vulkan captures: enum names come from RenderDoc itself (the original's
hand-written tables no longer matched), pipeline state uses the 1.46 API, uniform
buffers are read at their bound range, the shader debugger steps traces through
`ContinueDebug`, passes come from wgpu's pass labels, timings are real GPU durations, and
resources are shown and found by their wgpu labels. The mobile-GPU, planar-reflection and
bandwidth-estimate tools were dropped.

## Workflow

1. Start the engine under RenderDoc with the engine MCP server:
   `engine_launch {renderdoc: true}`. That loads RenderDoc's Vulkan layer
   (`ENABLE_VULKAN_RENDERDOC_CAPTURE=1`), sets `WGPU_DEBUG=1` so wgpu passes its labels
   and WGSL source to the driver, and points captures at `target/engine-mcp/captures/`.
2. `renderdoc_capture` captures the next frame and returns the `.rdc` path. Under
   qrenderdoc or `renderdoccmd capture` the remote method `renderdoc.capture` works the same.
3. Here: `open_capture {filepath}`, then `get_frame_overview` and `get_pass_timing`, and
   drill down from there (the server's instructions and prompts list the usual paths).

RenderDoc's shader debugger does not support ray queries (`SPV_KHR_ray_query`), so
shaders that trace rays (RT shadows, path tracing) can be inspected but not stepped.

Captures run tens to hundreds of MB; delete them when done. RenderDoc hides Vulkan
features it cannot capture and adds CPU overhead, so don't measure frame rates under it.

## Setup

RenderDoc's Python module (`renderdoc.so`) is not part of distribution packages (Arch's
`renderdoc` ships only qrenderdoc and renderdoccmd), and it has to match the RenderDoc
version that wrote the captures. `build_renderdoc_module.sh` clones the source of the
installed version into `~/src/renderdoc` and builds just the module (about five minutes
at 16 jobs):

```sh
tools/renderdoc-mcp/build_renderdoc_module.sh   # prints the module path
```

`run.sh` then creates `.venv/` on first start (and when `pyproject.toml` changes) and
looks for the module in `~/src/renderdoc/build/lib`, or `RENDERDOC_MODULE_PATH`. After a
RenderDoc upgrade, rerun the build script. Restart the server with `/mcp` after changing
it.

## Tests

```sh
tools/renderdoc-mcp/.venv/bin/python -m pytest tools/renderdoc-mcp/tests
```

The unit tests need only the module. `test_capture.py` replays a real capture on the GPU
and runs when `RENDERDOC_MCP_TEST_CAPTURE` names one taken with `WGPU_DEBUG=1`.

## Tools

| Area | Tools |
|------|-------|
| Session | `open_capture`, `close_capture`, `get_capture_info`, `get_frame_overview` |
| Events | `list_actions`, `get_action`, `set_event`, `search_actions`, `find_draws` |
| Pipeline | `get_pipeline_state`, `get_shader_bindings`, `get_vertex_inputs`, `get_draw_call_state` |
| Resources | `list_textures`, `list_buffers`, `list_resources`, `get_resource_usage` |
| Data | `save_texture`, `get_buffer_data`, `pick_pixel`, `get_texture_stats`, `read_texture_pixels`, `export_draw_textures`, `save_render_target`, `export_mesh` |
| Shaders | `disassemble_shader` (WGSL source, SPIR-V, AMD ISA, driver statistics), `get_shader_reflection`, `get_cbuffer_contents` |
| Debugging | `pixel_history`, `debug_shader_at_pixel`, `debug_compute_thread`, `get_post_vs_data`, `diff_draw_calls`, `analyze_render_passes`, `sample_pixel_region` |
| Performance | `get_pass_timing` (GPU durations, optional pipeline counters), `analyze_overdraw` (quad overdraw overlay), `analyze_state_changes` |
| Diagnostics | `diagnose_negative_values`, `diagnose_precision_issues` |

Anything that takes a `resource_id` also accepts the bare number or the resource's
unique debug name, e.g. `"HdrSceneColor"`.
