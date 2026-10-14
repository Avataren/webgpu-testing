#!/usr/bin/env python3
"""Exercise MCP additions against an isolated native editor; --web also builds/tests WASM.

Build editor/player and engine-mcp first. No existing editor or game project is touched.
Artifacts and the scratch project are retained under target/engine-mcp/ for inspection.
"""
import argparse
import base64
import json
from pathlib import Path
import socket
import subprocess
import tempfile

REPO = Path(__file__).resolve().parents[3]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--web', action='store_true')
    opts = parser.parse_args()
    output = REPO / 'target/engine-mcp'
    output.mkdir(parents=True, exist_ok=True)
    work = Path(tempfile.mkdtemp(prefix='additions-', dir=output))
    with socket.socket() as probe:
        probe.bind(('127.0.0.1', 0))
        port = probe.getsockname()[1]
    process = subprocess.Popen([str(REPO / 'tools/engine-mcp/target/debug/engine-mcp'),
                                '--repo', str(REPO), '--port', str(port)],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
    sequence = 0

    def rpc(method, params):
        nonlocal sequence
        sequence += 1
        process.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': sequence,
                                        'method': method, 'params': params}) + '\n')
        process.stdin.flush()
        line = process.stdout.readline()
        assert line, 'MCP server exited'
        response = json.loads(line)
        assert 'error' not in response, response
        return response['result']

    def tool(name, arguments=None, error=False):
        response = rpc('tools/call', {'name': name, 'arguments': arguments or {}})
        assert response.get('isError', False) == error, (name, response)
        if error and 'structuredContent' not in response:
            return response
        report = response['structuredContent']
        if not error:
            for field in definitions[name]['outputSchema'].get('required', []):
                assert field in report, (name, field, report)
        for content in response['content']:
            if content['type'] == 'image':
                (work / f'{name}.png').write_bytes(base64.b64decode(content['data']))
        return report

    try:
        rpc('initialize', {'protocolVersion': '2025-06-18', 'capabilities': {},
                           'clientInfo': {'name': 'additions-check', 'version': '1'}})
        definitions = {t['name']: t for t in rpc('tools/list', {})['tools']}
        for name in ['runtime_step', 'scene_query', 'scene_raycast', 'entity_place', 'scene_scatter',
                     'prefab_create', 'prefab_inspect', 'prefab_update', 'profile_capture',
                     'mesh_sweep', 'mesh_heightfield', 'project_validate', 'web_build', 'web_playtest']:
            assert definitions[name]['outputSchema']['type'] == 'object'
        assert definitions['scene_query']['annotations']['readOnlyHint']
        assert not definitions['entity_place']['annotations']['readOnlyHint']
        tool('engine_launch', {'target': 'editor', 'build': False, 'timeout_seconds': 120})
        project = tool('project_create', {'name': 'fixture', 'location': str(work)})
        print('Editor and scratch project opened', flush=True)

        calls = [
            {'tool': 'entity_spawn', 'as': 'group', 'arguments': {'name': 'Group', 'kind': 'empty'}},
            {'tool': 'entity_spawn', 'as': 'crate', 'arguments': {'name': 'Crate', 'kind': 'cube',
                'translation': [4, 3, 0], 'parent': {'$ref': 'group/id'}}},
        ]
        before = tool('scene_list_entities')['total']
        dry = tool('batch', {'atomic': True, 'dry_run': True, 'calls': calls})
        assert not dry['committed'] and tool('scene_list_entities')['total'] == before
        failed = tool('batch', {'atomic': True, 'calls': calls + [
            {'tool': 'entity_update', 'arguments': {'entity': 'DoesNotExist', 'visible': False}}
        ]}, error=True)
        assert failed['rolled_back'] and tool('scene_list_entities')['total'] == before
        committed = tool('batch', {'atomic': True, 'calls': calls})
        assert committed['committed'] and tool('scene_list_entities')['total'] == before + 2
        crate = committed['results'][1]['result']['id']
        tool('editor_undo'); tool('wait', {'frames': 3})
        assert tool('scene_list_entities')['total'] == before
        tool('editor_redo'); tool('wait', {'frames': 3})
        assert tool('entity_get', {'entity': crate})['name'] == 'Crate'
        print('Atomic validation, rollback, aliases, single undo and redo passed', flush=True)

        hits = tool('scene_raycast', {'origin': [4, 10, 0], 'direction': [0, -1, 0], 'exclude': ['Group']})
        assert hits['hits'][0]['name'] == 'Floor'
        placed = tool('entity_place', {'entity': crate, 'origin': [4, 10, 0], 'direction': [0, -1, 0], 'avoid_overlap': True})
        assert abs(placed['bounds']['min'][1] - hits['hits'][0]['position'][1]) < 1e-4
        nearby = tool('scene_query', {'center': [4, 0.5, 0], 'radius': 0.6})
        assert any(e['id'] == crate for e in nearby['entities'])
        original_pose = tool('entity_get', {'entity': crate})['transform']
        obstacle = tool('entity_spawn', {'kind': 'cube', 'translation': [7, 0.5, 0.7]})['id']
        tool('entity_place', {'entity': crate, 'origin': [7, 10, 0], 'direction': [0, -1, 0], 'avoid_overlap': True}, error=True)
        assert tool('entity_get', {'entity': crate})['transform'] == original_pose
        tool('entity_delete', {'entity': obstacle})
        before_scatter = tool('scene_list_entities')['total']
        tool('scene_scatter', {'entity': crate, 'bounds': {'min': [1, -1, 1], 'max': [1, 5, 1]}, 'count': 2, 'spacing': 2}, error=True)
        assert tool('scene_list_entities')['total'] == before_scatter
        scatter_args = {'entity': crate, 'bounds': {'min': [-8, -1, -8], 'max': [-2, 5, -2]}, 'count': 6, 'spacing': 1.1, 'seed': 51}
        first = tool('scene_scatter', scatter_args)
        tool('entity_delete', {'entities': [e['id'] for e in first['entities']]})
        second = tool('scene_scatter', scatter_args)
        assert [e['position'] for e in first['entities']] == [e['position'] for e in second['entities']]
        print('Raycast, placement, queries and seeded constrained scatter passed', flush=True)

        sweep = tool('mesh_sweep', {'name': 'Cable', 'path': [[-3, 1, 2], [-2, 1, 2], [-1, 2, 2]], 'radius': 0.12})
        terrain = tool('mesh_heightfield', {'name': 'Terrain', 'translation': [12, 3, 0], 'size': [6, 6], 'resolution': [32, 32], 'seed': 15})
        assert sweep['geometry']['triangles'] > 0 and terrain['geometry']['vertices'] == 1024
        prefab = tool('prefab_create', {'entity': 'Group', 'name': 'crate_group'})
        inspection = tool('prefab_inspect', {'document_id': prefab['document_id']})
        assert len(inspection['source']['entities']) == 8
        tool('entity_update', {'entity': 'Group', 'name': 'VariantGroup'})
        tool('prefab_update', {'entity': 'VariantGroup', 'document_id': prefab['document_id']})
        assert tool('prefab_inspect', {'document_id': prefab['document_id']})['source']['entities'][0]['name'] == 'Group'
        tool('entity_update', {'entity': 'VariantGroup', 'name': 'Group'})
        tool('scene_instance', {'document_id': prefab['document_id'], 'node_path': ['Group']})
        tool('wait', {'frames': 3})
        linked = tool('prefab_inspect', {'document_id': prefab['document_id']})
        assert linked['instance_entities'] and linked['instance_overrides']
        print('Sweep, heightfield and prefab source tools passed', flush=True)

        game = '''
        GAME = {ticks = 0, sum = 0, presses = 0}
        function on_created() GAME.initial = math.random() end
        function update(_, dt)
            GAME.ticks = GAME.ticks + 1
            GAME.sum = GAME.sum + math.random()
            if is_key_just_pressed("A") then GAME.presses = GAME.presses + 1 end
        end
        '''
        controller = tool('entity_spawn', {'name': 'TestController', 'kind': 'empty', 'script': {'source': game}})['id']
        scenario = {'entity': controller, 'steps': 30, 'seed': 123, 'dt': 1 / 60, 'lean': True,
                    'setup': 'GAME.setup = 77',
                    'input': [{'key': 'A', 'action': 'tap', 'delay': 10, 'frames': 8}],
                    'assertions': [{'name': 'initialized', 'at_step': 0, 'code': 'GAME.ticks == 0 and GAME.setup == 77'},
                                   {'name': 'checkpoint', 'at_step': 15, 'code': 'GAME.ticks == 15 and GAME.presses == 1 and is_key_pressed("A")'},
                                   {'name': 'completed', 'code': 'GAME.ticks == 30 and GAME.presses == 1 and not is_key_pressed("A")'}],
                    'eval': 'return {GAME.initial, GAME.sum, GAME.ticks, get_time()}'}
        a = tool('playtest', {**scenario, 'save_scenario': str(work / 'scenario.json')})
        b = tool('playtest', {'scenario': str(work / 'scenario.json')})
        assert a['ok'] and b['ok'] and a['eval']['result'] == b['eval']['result']
        failed = tool('playtest', {**scenario, 'lean': False, 'assertions': [{'at_step': 7, 'code': 'GAME.ticks == 8'}]}, error=True)
        assert failed['first_failure']['at_step'] == 7
        assert tool('engine_call', {'method': 'runtime.get'})['mode'] == 'editor'
        tool('runtime_set', {'mode': 'play', 'manual': True, 'seed': 123}); tool('wait', {'frames': 10})
        tool('playtest', {'steps': 0}, error=True)
        assert tool('engine_call', {'method': 'runtime.get'})['mode'] == 'playing'
        assert tool('lua_eval', {'entity': controller, 'code': 'return GAME.ticks', 'read_only': True})['result'] == 0
        tool('runtime_step', {'steps': 2}); tool('wait', {'frames': 10})
        assert tool('lua_eval', {'entity': controller, 'code': 'return GAME.ticks', 'read_only': True})['result'] == 2
        tool('runtime_set', {'mode': 'stop'}); tool('wait', {'frames': 3})
        print('Seeded replay, input checkpoints, failure reporting and render-independent stepping passed', flush=True)

        profile = tool('profile_capture', {'frames': 30})
        assert profile['samples'] == 30 and profile['frame_time_ms']['p99'] >= profile['frame_time_ms']['median']
        comparison = tool('profile_capture', {'frames': 20, 'baseline': profile})
        assert 'p95' in comparison['comparison']
        tool('project_save')
        assert tool('prefab_inspect', {'document_id': prefab['document_id']})['instance_entities']
        manifest = json.loads((work / 'fixture/project.json').read_text())
        host = next(d for d in manifest['scenes'] if d['id'] != prefab['document_id'])
        saved = json.loads((work / 'fixture' / host['relative_path']).read_text())
        assert any((e.get('scene_ref') or {}).get('document') == prefab['document_id'] for e in saved['entities'])
        tool('project_open', {'path': str(work / 'fixture')})
        reopened = tool('prefab_inspect', {'document_id': prefab['document_id']})
        assert reopened['instance_entities'] and reopened['instance_overrides']
        validation = tool('project_validate')
        assert validation['ok'], validation
        print('Profile capture, comparison, persistence and project validation passed', flush=True)
        tool('screenshot', {'camera': 'game', 'save_path': str(work / 'scene.png')})
        if opts.web:
            built = tool('web_build', {'project': str(work / 'fixture'), 'overwrite': True})
            browser = tool('web_playtest', {'directory': built['directory'], 'seconds': 2})
            assert browser['ready'] and browser['ok'], browser
            print('WASM packaging and WebGPU browser smoke test passed', flush=True)
        (work / 'report.json').write_text(json.dumps({'profile': profile, 'validation': validation}, indent=2))
        print(f'Artifacts: {work}', flush=True)
    finally:
        try:
            tool('engine_stop')
        finally:
            process.stdin.close()
            process.wait(timeout=10)


if __name__ == '__main__':
    main()
