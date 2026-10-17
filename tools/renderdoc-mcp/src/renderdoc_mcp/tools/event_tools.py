"""Event navigation tools: list_actions, get_action, set_event, search_actions, find_draws."""

from __future__ import annotations

import re
from typing import Optional

from mcp.server.mcpserver import MCPServer

from renderdoc_mcp.session import get_session
from renderdoc_mcp.util import (
    rd,
    to_json,
    serialize_action,
    serialize_action_detail,
    flags_to_list,
    make_error,
    marker_path,
    GRAPHICS_STAGES,
)

# event_type shorthand → ActionFlags mask
_EVENT_TYPE_MAP: dict[str, int] = {
    "draw": int(rd.ActionFlags.Drawcall),
    "dispatch": int(rd.ActionFlags.Dispatch),
    "clear": int(rd.ActionFlags.Clear),
    "copy": int(rd.ActionFlags.Copy),
    "resolve": int(rd.ActionFlags.Resolve),
    "marker": int(rd.ActionFlags.PushMarker),
}


def _flag_mask(names: list[str]) -> int | dict:
    mask = 0
    for name in names:
        val = getattr(rd.ActionFlags, name, None)
        if val is None:
            valid = [f.name for f in rd.ActionFlags]
            return make_error(f"Unknown ActionFlag: {name}. Valid: {valid}", "API_ERROR")
        mask |= int(val)
    return mask


def register(mcp: MCPServer):
    @mcp.tool()
    def list_actions(
        max_depth: int = 2,
        pass_name: Optional[str] = None,
        filter_flags: Optional[list[str]] = None,
        filter: Optional[str] = None,
        event_type: Optional[str] = None,
    ) -> str:
        """List the action tree (API calls grouped under wgpu's pass markers).

        The whole frame is large; prefer get_frame_overview for the pass list, then
        list one pass with pass_name.

        Args:
            max_depth: How deep to expand children (default 2).
            pass_name: Only the pass(es) whose name matches this regex, or a pass index.
            filter_flags: ActionFlags names (e.g. ["Drawcall", "Clear"]); keeps actions
                          matching any of them, plus their ancestors.
            filter: Case-insensitive substring of action or marker names.
            event_type: Shorthand filter: draw, dispatch, clear, copy, resolve, marker.
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)

        flag_mask = 0
        if event_type is not None and event_type.lower() != "all":
            flag_mask = _EVENT_TYPE_MAP.get(event_type.lower(), -1)
            if flag_mask == -1:
                return to_json(make_error(
                    f"Unknown event_type: {event_type}. Valid: all, {', '.join(_EVENT_TYPE_MAP)}",
                    "API_ERROR",
                ))
        elif filter_flags:
            mask = _flag_mask(filter_flags)
            if isinstance(mask, dict):
                return to_json(mask)
            flag_mask = mask

        name_filter = filter.lower() if filter else None
        sf = session.structured_file

        if pass_name is not None:
            passes = session.find_passes(pass_name)
            if not passes:
                return to_json(make_error(f"No pass matches '{pass_name}'", "API_ERROR"))
            roots = []
            for p in passes:
                marker = session.get_action(p["marker_event"]) if p["marker_event"] else None
                roots.extend([marker] if marker else [session.get_action(e) for e in p["events"]])
        else:
            roots = list(session.get_root_actions())

        def include(action) -> bool:
            if flag_mask and not (action.flags & flag_mask):
                return False
            if name_filter and name_filter not in action.GetName(sf).lower():
                return False
            return True

        def filtered(action, depth: int) -> dict | None:
            children = []
            for c in action.children:
                child = filtered(c, depth + 1)
                if child is not None:
                    children.append(child)
            if not include(action) and not children:
                return None
            result = {
                "event_id": action.eventId,
                "name": action.GetName(sf),
                "flags": flags_to_list(action.flags),
            }
            if action.numIndices:
                result["num_indices"] = action.numIndices
            if children:
                if depth < max_depth:
                    result["children"] = children
                else:
                    result["matching_children"] = len(children)
            return result

        if flag_mask or name_filter:
            actions = [r for r in (filtered(a, 0) for a in roots) if r is not None]
        else:
            actions = [serialize_action(a, sf, max_depth=max_depth) for a in roots]

        return to_json({"actions": actions, "total_actions": len(session.action_map)})

    @mcp.tool()
    def get_action(event_id: int) -> str:
        """Full detail of one action: name, flags, enclosing markers (the pass), counts,
        dispatch size, outputs and neighbouring events.

        Args:
            event_id: The event ID of the action.
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)

        action = session.get_action(event_id)
        if action is None:
            return to_json(make_error(f"Event ID {event_id} not found", "INVALID_EVENT_ID"))

        result = serialize_action_detail(action, session.structured_file)
        result["outputs"] = [session.describe(o) for o in result["outputs"]]
        if result["depth_output"]:
            result["depth_output"] = session.describe(result["depth_output"])
        return to_json(result)

    @mcp.tool()
    def set_event(event_id: int) -> str:
        """Move the replay to an event. Tools that take an optional event_id inspect the
        state at the current event when it is omitted.

        Args:
            event_id: The event ID to navigate to.
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)

        err = session.set_event(event_id)
        if err:
            return to_json(err)

        action = session.get_action(event_id)
        return to_json({
            "status": "ok",
            "event_id": event_id,
            "name": action.GetName(session.structured_file),
            "markers": marker_path(action, session.structured_file),
        })

    @mcp.tool()
    def search_actions(
        name_pattern: Optional[str] = None,
        flags: Optional[list[str]] = None,
        max_results: int = 200,
    ) -> str:
        """Search actions by name (regex, case-insensitive; matches marker/pass names too)
        and/or ActionFlags.

        Args:
            name_pattern: Regex matched against action names.
            flags: ActionFlags names; actions matching any are included.
            max_results: Maximum results (default 200).
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)

        flag_mask = 0
        if flags:
            mask = _flag_mask(flags)
            if isinstance(mask, dict):
                return to_json(mask)
            flag_mask = mask

        pattern = None
        if name_pattern:
            try:
                pattern = re.compile(name_pattern, re.IGNORECASE)
            except re.error as e:
                return to_json(make_error(f"Invalid regex pattern: {e}", "API_ERROR"))
        sf = session.structured_file
        results = []
        total = 0
        for eid, action in sorted(session.action_map.items()):
            if flag_mask and not (action.flags & flag_mask):
                continue
            action_name = action.GetName(sf)
            if pattern and not pattern.search(action_name):
                continue
            total += 1
            if len(results) < max_results:
                pass_ = session.pass_of(eid)
                entry = {"event_id": eid, "name": action_name, "flags": flags_to_list(action.flags)}
                if pass_:
                    entry["pass"] = pass_["name"]
                results.append(entry)

        return to_json({"matches": results, "count": total, "truncated": total > len(results)})

    @mcp.tool()
    def find_draws(
        pass_name: Optional[str] = None,
        blend: Optional[bool] = None,
        min_vertices: Optional[int] = None,
        texture_id: Optional[str] = None,
        shader_id: Optional[str] = None,
        render_target_id: Optional[str] = None,
        max_results: int = 50,
    ) -> str:
        """Find draw calls by pass and rendering state. Filters on blend, texture or shader
        replay each candidate draw, which takes a moment on large frames.

        Args:
            pass_name: Only draws in passes matching this regex (or a pass index).
            blend: Whether blending is enabled on the first render target.
            min_vertices: Minimum index/vertex count.
            texture_id: Draws reading this texture in any stage (ID, number or name).
            shader_id: Draws using this shader resource in any stage.
            render_target_id: Draws writing this render target (ID, number or name).
            max_results: Maximum results (default 50).
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)

        texture_key = rt_key = None
        if texture_id is not None:
            texture_key = session.resource_key(texture_id)
            if texture_key is None:
                return to_json(session.resource_error(texture_id))
        if render_target_id is not None:
            rt_key = session.resource_key(render_target_id)
            if rt_key is None:
                return to_json(session.resource_error(render_target_id))

        if pass_name is not None:
            passes = session.find_passes(pass_name)
            if not passes:
                return to_json(make_error(f"No pass matches '{pass_name}'", "API_ERROR"))
            candidates = sorted(e for p in passes for e in p["events"])
        else:
            candidates = sorted(session.action_map)

        sf = session.structured_file
        results = []
        saved_event = session.current_event
        needs_state = blend is not None or texture_key is not None or shader_id is not None

        for eid in candidates:
            if len(results) >= max_results:
                break
            action = session.action_map[eid]
            if not (action.flags & rd.ActionFlags.Drawcall):
                continue
            if min_vertices is not None and action.numIndices < min_vertices:
                continue
            if rt_key is not None and rt_key not in [str(o) for o in action.outputs] + [str(action.depthOut)]:
                continue

            if needs_state:
                session.set_event(eid)
                state = session.controller.GetPipelineState()
                if blend is not None:
                    blends = state.GetColorBlends()
                    if (bool(blends[0].enabled) if len(blends) else False) != blend:
                        continue
                if shader_id is not None:
                    shaders = [str(state.GetShader(s)) for _, s in GRAPHICS_STAGES]
                    if shader_id not in shaders and f"ResourceId::{shader_id}" not in shaders:
                        continue
                if texture_key is not None:
                    used = set()
                    for _, stage in GRAPHICS_STAGES:
                        if state.GetShaderReflection(stage) is None:
                            continue
                        for b in state.GetReadOnlyResources(stage):
                            used.add(str(b.descriptor.resource))
                    if texture_key not in used:
                        continue

            pass_ = session.pass_of(eid)
            results.append({
                "event_id": eid,
                "name": action.GetName(sf),
                "pass": pass_["name"] if pass_ else None,
                "num_indices": action.numIndices,
                "num_instances": action.numInstances,
            })

        if saved_event is not None:
            session.set_event(saved_event)

        return to_json({"matches": results, "count": len(results), "max_results": max_results})
