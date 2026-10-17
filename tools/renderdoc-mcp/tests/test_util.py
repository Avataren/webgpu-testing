"""Helpers checked against the real RenderDoc module (no GPU or capture needed)."""

from types import SimpleNamespace

import renderdoc as rd

from renderdoc_mcp import util
from renderdoc_mcp.tools.advanced_tools import _diff, _flatten_diff
from renderdoc_mcp.tools.data_tools import _post_vs_layout, _triangles
from renderdoc_mcp.tools.shader_tools import _filter_lines


def test_enum_names_come_from_renderdoc():
    # Python 3.11+ prints IntEnums as bare numbers; names must still come through.
    assert util.name_of(rd.Topology.TriangleList) == "TriangleList"
    assert util.name_of(int(rd.Topology.TriangleList), rd.Topology) == "TriangleList"
    assert util.name_of(int(rd.CompareFunction.GreaterEqual), rd.CompareFunction) == "GreaterEqual"
    assert util.name_of(int(rd.ResourceUsage.CS_RWResource), rd.ResourceUsage) == "CS_RWResource"
    assert util.name_of(12345, rd.Topology) == "12345"


def test_stage_names():
    assert util.parse_stage("Fragment") == rd.ShaderStage.Pixel
    assert util.parse_stage("compute") == rd.ShaderStage.Compute
    assert util.parse_stage("mesh") == rd.ShaderStage.Mesh
    assert util.parse_stage("tessellation") is None


def test_flags_to_list_uses_single_bits():
    flags = int(rd.ActionFlags.Drawcall | rd.ActionFlags.Indexed | rd.ActionFlags.Instanced)
    assert set(util.flags_to_list(flags)) == {"Drawcall", "Indexed", "Instanced"}
    assert util.flags_to_list(0) == []


def test_blend_formula_uses_renderdoc_factor_names():
    names = [m.name for m in rd.BlendMultiplier]
    assert "SrcAlpha" in names and "InvSrcAlpha" in names
    formula = util.blend_formula("SrcAlpha", "InvSrcAlpha", "Add", "One", "InvSrcAlpha", "Add")
    assert formula == "rgb = src.a*src.rgb + (1-src.a)*dst.rgb | a = src.a + (1-src.a)*dst.a"
    assert "min(" in util.blend_formula("One", "One", "Minimum", "One", "One", "Maximum")


def _var(name, values, rows=1, columns=None, var_type=rd.VarType.Float, members=()):
    columns = columns if columns is not None else len(values)
    value = SimpleNamespace(f32v=list(values) + [0.0] * 16, u32v=list(values) + [0] * 16,
                            s32v=list(values) + [0] * 16)
    return SimpleNamespace(name=name, rows=rows, columns=columns, type=var_type,
                           value=value, members=list(members))


def test_shader_variable_reads_by_type_and_unwraps_wgsl_blocks():
    matrix = _var("view_proj", [float(i) for i in range(16)], rows=4, columns=4)
    count = _var("light_count", [3], var_type=rd.VarType.UInt)
    wrapper = _var("_child0", [], members=[matrix, count])
    block = util.serialize_shader_variable(_var("globals", [], members=[wrapper]))
    assert block["name"] == "globals"
    assert block["members"][0]["value"][1] == [4.0, 5.0, 6.0, 7.0]
    assert block["members"][1] == {"name": "light_count", "type": "uint", "value": [3]}


def _sig(name, comps, builtin=rd.ShaderBuiltin.Undefined, var_type=rd.VarType.Float):
    return SimpleNamespace(varName=name, semanticIdxName="", compCount=comps,
                           systemValue=builtin, varType=var_type)


def test_post_vs_layout_puts_position_first():
    refl = SimpleNamespace(outputSignature=[
        _sig("world_normal", 3), _sig("position", 4, rd.ShaderBuiltin.Position), _sig("uv", 2)])
    layout = _post_vs_layout(refl, aligned=False)
    assert layout["position"][:2] == (0, 4)
    assert layout["world_normal"][:2] == (16, 3)
    assert layout["uv"][:2] == (28, 2)


def test_triangles_follow_topology():
    assert _triangles([0, 1, 2, 2, 1, 3], "TriangleList", 4) == [(0, 1, 2), (2, 1, 3)]
    assert _triangles([0, 1, 2, 3], "TriangleStrip", 4) == [(0, 1, 2), (1, 3, 2)]
    assert _triangles([0, 1, 2, 3], "LineList", 4) == []


def test_filter_lines():
    text = "\n".join(f"line {i}" for i in range(1, 21))
    body, note = _filter_lines(text, [3, 4], None)
    assert body == "    3: line 3\n    4: line 4" and note == "Lines 3-4 of 20"
    body, note = _filter_lines(text, None, r"line 1[05]$")
    assert "   10: line 10" in body and "   15: line 15" in body and note.startswith("2 matching")


def test_diff_flattens_to_changed_fields():
    a = {"depth": {"test": True, "write": True}, "targets": ["A"]}
    b = {"depth": {"test": True, "write": False}, "targets": ["B"]}
    assert _flatten_diff(_diff(a, b)) == [
        {"field": "depth.write", "eid1": True, "eid2": False},
        {"field": "targets", "eid1": ["A"], "eid2": ["B"]},
    ]
