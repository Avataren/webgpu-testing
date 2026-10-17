"""Diagnostic tools: diagnose_negative_values, diagnose_precision_issues."""

from __future__ import annotations

import math
import re
from typing import Optional

from mcp.server.mcpserver import MCPServer

from renderdoc_mcp.session import get_session
from renderdoc_mcp.util import rd, to_json, name_of

HALF_MAX = 65504.0
# Names of buffers whose negative values are expected (normals, velocities, positions...).
_SIGNED_NAMES = re.compile(r"normal|velocity|motion|position|offset|tangent|direction|delta", re.IGNORECASE)

_WRITE_USAGES = ("RWResource", "ColorTarget", "DepthStencilTarget", "CopyDst", "Clear",
                 "ResolveDst", "GenMips")


def _writes(session, rid) -> list[int]:
    """Events that write a resource, in order."""
    events = []
    for usage in session.controller.GetUsage(rid):
        kind = name_of(usage.usage, rd.ResourceUsage)
        if any(w in kind for w in _WRITE_USAGES) and usage.eventId not in events:
            events.append(usage.eventId)
    return events


def _action_at_or_after(session, event_id: int) -> int:
    """The action event to replay to so that `event_id` (maybe a non-action API call) has run."""
    if event_id in session.action_map:
        return event_id
    later = [e for e in session.action_map if e >= event_id]
    return min(later) if later else max(session.action_map)


def _minmax(session, rid) -> tuple[list[float], list[float]]:
    lo, hi = session.controller.GetMinMax(rid, rd.Subresource(0, 0, 0), rd.CompType.Typeless)
    return list(lo.floatValue[:4]), list(hi.floatValue[:4])


def _problems(lo: list[float], hi: list[float], channels: int) -> list[str]:
    found = []
    values = lo[:channels] + hi[:channels]
    if any(math.isnan(v) for v in values):
        found.append("NaN")
    if any(math.isinf(v) for v in values):
        found.append("Inf")
    if any(v < 0 for v in lo[:channels]):
        found.append("negative")
    return found


def _float_textures(session, only: Optional[list[str]]):
    keys = None
    if only:
        keys = {session.resource_key(r) for r in only} - {None}
    for tex in session.controller.GetTextures():
        if keys is not None and str(tex.resourceId) not in keys:
            continue
        fmt = str(tex.format.Name()).upper()
        if "FLOAT" not in fmt or tex.width * tex.height < 16:
            continue
        yield tex, fmt


def register(mcp: MCPServer):

    @mcp.tool()
    def diagnose_negative_values(
        check_targets: Optional[list[str]] = None,
        trace_depth: int = 12,
    ) -> str:
        """Find float textures that end the frame with NaN, Inf or negative values (exact
        min/max over every texel) and the first write - draw, dispatch or copy, with its
        pass - that introduced them. Signed data such as normals, motion vectors or
        positions is legitimately negative; judge by the texture's name.

        Args:
            check_targets: Texture IDs or names to check (default: every float texture
                           written during the frame).
            trace_depth: How many writes to step through when looking for the first bad
                         one (default 12).
        """
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)

        saved = session.current_event
        findings = []
        clean = []
        for tex, fmt in _float_textures(session, check_targets):
            writes = _writes(session, tex.resourceId)
            if not writes:
                continue
            channels = tex.format.compCount or 4
            session.set_event(_action_at_or_after(session, writes[-1]))
            lo, hi = _minmax(session, tex.resourceId)
            problems = _problems(lo, hi, channels)
            info = session.describe(tex.resourceId)
            if not problems:
                clean.append(info.get("name") or info["resource_id"])
                continue
            negative = [c for c, v in zip("rgba", lo[:channels]) if v < 0]
            entry = {**info, "problems": problems,
                     "likely_signed_data": problems == ["negative"] and bool(_SIGNED_NAMES.search(info.get("name", ""))),
                     "negative_channels": negative,
                     "min": [round(v, 5) for v in lo[:channels]],
                     "max": [round(v, 5) for v in hi[:channels]]}
            for write in writes[:trace_depth]:
                eid = _action_at_or_after(session, write)
                session.set_event(eid)
                lo2, hi2 = _minmax(session, tex.resourceId)
                first = _problems(lo2, hi2, channels)
                if first:
                    pass_ = session.pass_of(eid)
                    entry["first_bad_write"] = {
                        "event_id": eid,
                        "name": session.get_action(eid).GetName(session.structured_file),
                        "pass": pass_["name"] if pass_ else None,
                        "problems": first,
                    }
                    break
            entry["writes"] = len(writes)
            findings.append(entry)

        if saved is not None:
            session.set_event(saved)
        return to_json({
            "affected": findings,
            "clean": clean,
            "next_steps": ("Find a bad pixel with sample_pixel_region at first_bad_write, then "
                           "pixel_history and debug_shader_at_pixel (or debug_compute_thread)."
                           if findings else None),
        })

    @mcp.tool()
    def diagnose_precision_issues() -> str:
        """Check written render targets for precision problems: half-float targets that
        reach the fp16 limit (65504) or overflow to Inf, R11G11B10 targets (no sign bit,
        no alpha, ~6-bit mantissas) and 16-bit depth buffers."""
        session = get_session()
        err = session.require_open()
        if err:
            return to_json(err)

        saved = session.current_event
        issues = []
        for tex in session.controller.GetTextures():
            fmt = str(tex.format.Name()).upper()
            writes = None
            if "R11G11B10" in fmt or ("R16" in fmt and "FLOAT" in fmt) or fmt.startswith("D16"):
                writes = _writes(session, tex.resourceId)
            if not writes:
                continue
            info = session.describe(tex.resourceId)
            if fmt.startswith("D16"):
                issues.append({**info, "issue": "16-bit depth: little precision far from the camera",
                               "suggestion": "Use Depth32Float, ideally with reversed Z"})
                continue
            if "R11G11B10" in fmt:
                issues.append({**info, "issue": "R11G11B10 float stores no sign and has 5-6 bit "
                                                "mantissas; negative values clamp to 0 and gradients can band",
                               "suggestion": "Keep it for HDR color only; use RGBA16Float for signed or precise data"})
                continue
            last = _action_at_or_after(session, writes[-1])
            session.set_event(last)
            _lo, hi = _minmax(session, tex.resourceId)
            peak = max(hi[:tex.format.compCount or 4])
            if math.isinf(peak) or peak >= HALF_MAX * 0.99:
                pass_ = session.pass_of(last)
                issues.append({**info, "issue": f"Half-float target reaches {peak}, the fp16 maximum: "
                                                "overflowing values, or a deliberate far/sky sentinel",
                               "last_write": {"event_id": last, "pass": pass_["name"] if pass_ else None},
                               "suggestion": "Clamp before writing, or pre-expose / scale the values"})
        if saved is not None:
            session.set_event(saved)
        return to_json({"issues": issues, "count": len(issues)})
