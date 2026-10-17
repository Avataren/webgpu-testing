"""Pipeline inspection tools: get_pipeline_state, get_shader_bindings, get_vertex_inputs, get_draw_call_state."""

from __future__ import annotations

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
    serialize_blend,
    GRAPHICS_STAGES,
)


def _is_compute(session) -> bool:
    action = session.get_action(session.current_event)
    return bool(action is not None and action.flags & rd.ActionFlags.Dispatch)


def _bound_shaders(session, state, compute: bool) -> dict:
    stages = [("compute", rd.ShaderStage.Compute)] if compute else GRAPHICS_STAGES
    shaders = {}
    for name, stage in stages:
        refl = state.GetShaderReflection(stage)
        if refl is not None:
            shaders[name] = {
                "entry_point": state.GetShaderEntryPoint(stage),
                "resource_id": str(refl.resourceId),
            }
            module = session.resource_name(state.GetShader(stage))
            if module:
                shaders[name]["module"] = module
    return shaders


def _viewport(state) -> dict | None:
    try:
        vp = state.GetViewport(0)
        sc = state.GetScissor(0)
    except Exception:
        return None
    return {
        "viewport": [vp.x, vp.y, vp.width, vp.height],
        "depth_range": [vp.minDepth, vp.maxDepth],
        "scissor": [sc.x, sc.y, sc.width, sc.height] if sc.enabled else None,
    }


def _raster(session, state) -> dict:
    rs = state.GetRasterState()
    result = {
        "cull": name_of(rs.cullMode, rd.CullMode),
        "fill": name_of(rs.fillMode, rd.FillMode),
        "front_ccw": bool(rs.frontCCW),
    }
    if state.IsCaptureVK():
        vk = session.controller.GetVulkanPipelineState()
        r = vk.rasterizer
        result.update({
            "depth_clamp": bool(r.depthClampEnable),
            "depth_clip": bool(r.depthClipEnable),
            "discard": bool(r.rasterizerDiscardEnable),
            "line_width": r.lineWidth,
            "samples": vk.multisample.rasterSamples,
        })
        if r.depthBiasEnable:
            result["depth_bias"] = {"constant": r.depthBias, "slope": r.slopeScaledDepthBias,
                                    "clamp": r.depthBiasClamp}
        if int(r.conservativeRasterization):
            result["conservative"] = name_of(r.conservativeRasterization)
    return result


def _depth_stencil(state) -> tuple[dict, dict]:
    ds = state.GetDepthTestState()
    depth = {
        "test": bool(ds.depthEnable),
        "write": bool(ds.depthWrites),
        "function": name_of(ds.depthFunction, rd.CompareFunction) if ds.depthEnable else None,
    }
    if ds.depthBounds:
        depth["bounds"] = [ds.minDepthBounds, ds.maxDepthBounds]
    stencil: dict = {"enabled": bool(state.IsStencilTestEnabled())}
    if stencil["enabled"]:
        faces = state.GetStencilFaces()
        for label, face in zip(("front", "back"), faces):
            stencil[label] = {
                "function": name_of(face.function, rd.CompareFunction),
                "pass": name_of(face.passOperation, rd.StencilOperation),
                "fail": name_of(face.failOperation, rd.StencilOperation),
                "depth_fail": name_of(face.depthFailOperation, rd.StencilOperation),
                "reference": face.reference,
                "compare_mask": face.compareMask,
                "write_mask": face.writeMask,
            }
    return depth, stencil


def _targets(session, state) -> tuple[list[dict], dict | None]:
    outputs = []
    blends = list(state.GetColorBlends())
    for index, o in enumerate(state.GetOutputTargets()):
        if int(o.resource) == 0:
            continue
        entry = session.describe(o.resource)
        entry["slot"] = index
        if index < len(blends):
            entry["blend"] = serialize_blend(blends[index])
        outputs.append(entry)
    depth = state.GetDepthTarget()
    return outputs, (session.describe(depth.resource) if int(depth.resource) != 0 else None)


def _pipeline_state(session, state) -> dict:
    """Everything about the pipeline at the current event."""
    action = session.get_action(session.current_event)
    compute = _is_compute(session)
    result: dict = {
        "event_id": session.current_event,
        "action": action.GetName(session.structured_file) if action else None,
        "shaders": _bound_shaders(session, state, compute),
    }
    if compute:
        result["dispatch_groups"] = list(action.dispatchDimension)
        refl = state.GetShaderReflection(rd.ShaderStage.Compute)
        if refl is not None:
            result["workgroup_size"] = list(refl.dispatchThreadsDimension)
        return result

    warnings: list[str] = []

    def attempt(key: str, fn):
        try:
            value = fn()
            if value is not None:
                result[key] = value
        except Exception as e:  # report instead of failing the whole call
            warnings.append(f"{key}: {type(e).__name__}: {e}")

    attempt("topology", lambda: name_of(state.GetPrimitiveTopology(), rd.Topology))
    attempt("viewport", lambda: _viewport(state))
    attempt("rasterizer", lambda: _raster(session, state))

    def depth_stencil():
        result["depth"], result["stencil"] = _depth_stencil(state)

    attempt("depth_stencil", depth_stencil)

    def targets():
        result["render_targets"], result["depth_target"] = _targets(session, state)

    attempt("targets", targets)
    attempt("blend_constant", lambda: list(state.GetBlendFactor())
            if any(b.enabled for b in state.GetColorBlends()) else None)
    if warnings:
        result["warnings"] = warnings
    return result


def _bindings(session, state, stage_enum) -> dict | None:
    """Constant buffers, read-only and read-write resources and samplers of one stage."""
    refl = state.GetShaderReflection(stage_enum)
    if refl is None:
        return None

    def by_index(descriptors) -> dict:
        grouped: dict = {}
        for d in descriptors:
            grouped.setdefault(d.access.index, []).append(d)
        return grouped

    cbs = []
    for i, cb in enumerate(refl.constantBlocks):
        entry: dict = {"index": i, "name": cb.name, "bytes": cb.byteSize}
        try:
            desc = state.GetConstantBlock(stage_enum, i, 0).descriptor
            if int(desc.resource) != 0:
                entry["buffer"] = session.describe(desc.resource)
                entry["offset"] = desc.byteOffset
        except Exception as e:
            entry["error"] = str(e)
        cbs.append(entry)

    def resources(reflected, descriptors) -> list[dict]:
        grouped = by_index(descriptors)
        out = []
        for i, res in enumerate(reflected):
            entry: dict = {"index": i, "name": res.name,
                           "kind": name_of(res.descriptorType, rd.DescriptorType)}
            if res.isTexture:
                entry["type"] = name_of(res.textureType, rd.TextureType)
            bound = [session.describe(d.descriptor.resource) for d in grouped.get(i, [])
                     if int(d.descriptor.resource) != 0]
            if len(bound) == 1:
                entry["bound"] = bound[0]
            elif bound:
                entry["bound"] = bound[:16]
                if len(bound) > 16:
                    entry["bound_count"] = len(bound)
            out.append(entry)
        return out

    samplers = []
    grouped = by_index(state.GetSamplers(stage_enum))
    for i, s in enumerate(refl.samplers):
        entry = {"index": i, "name": s.name}
        bound = grouped.get(i, [])
        if bound:
            sd = bound[0].sampler
            f = sd.filter
            entry["filter"] = {"min": name_of(f.minify, rd.FilterMode),
                               "mag": name_of(f.magnify, rd.FilterMode),
                               "mip": name_of(f.mip, rd.FilterMode)}
            entry["address"] = [name_of(a, rd.AddressMode) for a in (sd.addressU, sd.addressV, sd.addressW)]
            if f.filter == rd.FilterFunction.Comparison:
                entry["compare"] = name_of(sd.compareFunction, rd.CompareFunction)
            if sd.maxAnisotropy > 1:
                entry["anisotropy"] = sd.maxAnisotropy
        samplers.append(entry)

    return {
        "constant_buffers": cbs,
        "read_only": resources(refl.readOnlyResources, state.GetReadOnlyResources(stage_enum)),
        "read_write": resources(refl.readWriteResources, state.GetReadWriteResources(stage_enum)),
        "samplers": samplers,
    }


def register(mcp: MCPServer):
    @mcp.tool()
    def get_pipeline_state(event_id: Optional[int] = None) -> str:
        """Full pipeline state at an event: bound shaders, topology, viewport/scissor,
        rasterizer (cull, depth bias/clamp, samples), depth and stencil tests, render
        targets with their blend state, and the depth target. For a dispatch: the compute
        shader and dispatch size.

        Args:
            event_id: Event to inspect (default: the current event).
        """
        session = get_session()
        err = session.require_open() or session.ensure_event(event_id)
        if err:
            return to_json(err)
        return to_json(_pipeline_state(session, session.controller.GetPipelineState()))

    @mcp.tool()
    def get_shader_bindings(stage: str, event_id: Optional[int] = None) -> str:
        """What a shader stage has bound at an event: uniform/constant buffers (with the
        bound buffer and offset), textures and storage resources (with names, sizes and
        formats) and samplers.

        Args:
            stage: vertex, fragment (or pixel), compute, task, mesh, ...
            event_id: Event to inspect (default: the current event).
        """
        session = get_session()
        err = session.require_open() or session.ensure_event(event_id)
        if err:
            return to_json(err)
        stage_enum = parse_stage(stage)
        if stage_enum is None:
            return to_json(stage_error(stage))
        result = _bindings(session, session.controller.GetPipelineState(), stage_enum)
        if result is None:
            return to_json(make_error(f"No shader bound at stage '{stage}'", "API_ERROR"))
        return to_json({"stage": stage, "event_id": session.current_event, **result})

    @mcp.tool()
    def get_vertex_inputs(event_id: Optional[int] = None) -> str:
        """Vertex attributes (name, format, buffer, offset, per-instance), the bound vertex
        buffers and the index buffer at an event.

        Args:
            event_id: Event to inspect (default: the current event).
        """
        session = get_session()
        err = session.require_open() or session.ensure_event(event_id)
        if err:
            return to_json(err)

        state = session.controller.GetPipelineState()
        ib = state.GetIBuffer()
        vbs = [
            {"slot": i, **session.describe(vb.resourceId), "offset": vb.byteOffset,
             "stride": vb.byteStride}
            for i, vb in enumerate(state.GetVBuffers()) if int(vb.resourceId) != 0
        ]
        attrs = [
            {
                "name": a.name,
                "format": str(a.format.Name()),
                "buffer": a.vertexBuffer,
                "offset": a.byteOffset,
                "per_instance": bool(a.perInstance),
            }
            for a in state.GetVertexInputs() if a.used
        ]
        result: dict = {
            "event_id": session.current_event,
            "topology": name_of(state.GetPrimitiveTopology(), rd.Topology),
            "vertex_attributes": attrs,
            "vertex_buffers": vbs,
        }
        if int(ib.resourceId) != 0:
            result["index_buffer"] = {**session.describe(ib.resourceId), "offset": ib.byteOffset,
                                      "index_bytes": ib.byteStride}
        return to_json(result)

    @mcp.tool()
    def get_draw_call_state(event_id: int) -> str:
        """Everything about one draw or dispatch in a single call: the pass it belongs to,
        counts, pipeline state (see get_pipeline_state) and each bound stage's textures
        and buffers.

        Args:
            event_id: The draw or dispatch to inspect.
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)
        return to_json(_get_draw_state_dict(session, event_id))


def _get_draw_state_dict(session, event_id: int) -> dict:
    """Pipeline state plus per-stage bindings of a draw; shared with diff_draw_calls."""
    err = session.set_event(event_id)
    if err:
        return err
    action = session.get_action(event_id)
    state = session.controller.GetPipelineState()
    result = _pipeline_state(session, state)
    pass_ = session.pass_of(event_id)
    result["pass"] = pass_["name"] if pass_ else None
    if action.flags & rd.ActionFlags.Drawcall:
        result["indices"] = action.numIndices
        result["instances"] = action.numInstances

    resources: dict = {}
    for stage_name in result["shaders"]:
        stage_enum = parse_stage(stage_name)
        bindings = _bindings(session, state, stage_enum)
        if bindings is None:
            continue
        summary = {
            "uniforms": [cb["name"] for cb in bindings["constant_buffers"]],
            "read_only": [_bound_summary(r) for r in bindings["read_only"]],
        }
        if bindings["read_write"]:
            summary["read_write"] = [_bound_summary(r) for r in bindings["read_write"]]
        resources[stage_name] = summary
    result["resources"] = resources
    return result


def _bound_summary(resource: dict) -> dict:
    bound = resource.get("bound")
    entry = {"name": resource["name"]}
    if isinstance(bound, dict):
        entry.update({k: v for k, v in bound.items() if k != "name"})
        if bound.get("name"):
            entry["resource"] = bound["name"]
    elif isinstance(bound, list):
        entry["array"] = len(bound)
    return entry
