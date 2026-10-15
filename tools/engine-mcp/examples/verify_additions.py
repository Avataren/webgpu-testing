#!/usr/bin/env python3
"""Exercise MCP additions against an isolated native editor; --web also builds/tests WASM.

Build editor/player and engine-mcp first. No existing editor or game project is touched.
Artifacts and the scratch project are retained under target/engine-mcp/ for inspection.
"""
import argparse
import base64
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

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
    mcp_env = os.environ.copy()
    desktop = {key: os.environ[key] for key in ['DISPLAY', 'WAYLAND_DISPLAY', 'XDG_RUNTIME_DIR'] if key in os.environ}
    if sys.platform.startswith('linux'):
        for key in ['DISPLAY', 'WAYLAND_DISPLAY']:
            mcp_env.pop(key, None)
    process = subprocess.Popen([str(REPO / 'tools/engine-mcp/target/debug/engine-mcp'),
                                '--repo', str(REPO), '--port', str(port)],
                               stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True, env=mcp_env)
    sequence = 0

    def rpc(method, params, transport=None):
        nonlocal sequence
        transport = transport or process
        sequence += 1
        transport.stdin.write(json.dumps({'jsonrpc': '2.0', 'id': sequence,
                                        'method': method, 'params': params}) + '\n')
        transport.stdin.flush()
        line = transport.stdout.readline()
        assert line, 'MCP server exited'
        response = json.loads(line)
        assert 'error' not in response, response
        return response['result']

    def tool(name, arguments=None, error=False, transport=None):
        response = rpc('tools/call', {'name': name, 'arguments': arguments or {}}, transport)
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
        resources = rpc('resources/list', {})['resources']
        assert any(r['uri'] == 'engine://workflow' for r in resources)
        assert 'Workflow:' in rpc('resources/read', {'uri': 'engine://workflow'})['contents'][0]['text']
        assert definitions['mesh_compose']['inputSchema']['properties']['parts']['maxItems'] == 128
        if sys.platform.startswith('linux'):
            preflight = tool('engine_launch', {'target': 'editor'}, error=True)
            assert 'No build was started' in preflight['content'][0]['text']
        launched = tool('engine_launch', {'target': 'editor', 'build': False, 'timeout_seconds': 120, 'env': desktop})
        assert launched['engine']['limits']['lights']['point'] == 4
        assert launched['engine']['limits']['geometry']['compose_parts'] == 128
        project = tool('project_create', {'name': 'fixture', 'location': str(work)})
        assert project['starter_scene']['saved']['written_count'] > 0
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
        committed = tool('batch', {'atomic': True, 'quiet': True, 'calls': calls})
        assert committed['committed'] and tool('scene_list_entities')['total'] == before + 2
        # aggregate: one summary instead of a row per call, and the verdict is untouched
        aggregated = tool('batch', {'atomic': True, 'dry_run': True, 'aggregate': True, 'calls': calls})
        assert 'results' not in aggregated and aggregated['summary']['calls'] == {'entity_spawn': 2}
        assert aggregated['summary']['entities']['count'] == 2 and 'group' in aggregated['summary']['aliases']
        broken = tool('batch', {'atomic': True, 'aggregate': True, 'calls': calls + [
            {'tool': 'entity_update', 'arguments': {'entity': 'DoesNotExist', 'visible': False}}
        ]}, error=True)
        assert broken['rolled_back'] and broken['failure']['index'] == 2 and broken['summary']['calls']
        assert tool('scene_list_entities')['total'] == before + 2
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
        before_geometry = tool('scene_list_entities')['total']
        failed_geometry = tool('batch', {'atomic': True, 'quiet': True, 'calls': [
            {'tool': 'mesh_lathe', 'arguments': {'name': 'RolledBack', 'profile': [[1, 0], [1, 1]], 'segments': 64}},
            {'tool': 'mesh_compose', 'arguments': {'parts': [{'shape': 'primitive', 'primitive': 'cube'}] * 129}}
        ]}, error=True)
        assert failed_geometry['rolled_back'] and failed_geometry['failure']['index'] == 1
        summary = failed_geometry['results'][0]['result']
        assert summary['id'] and summary['geometry']['vertices'] > 0
        assert 'recipe' not in summary['geometry'] and len(json.dumps(failed_geometry)) < 2000
        assert tool('scene_list_entities')['total'] == before_geometry
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
        function on_created(_, previous)
            GAME = previous or GAME
            if not GAME.pool then GAME.pool = spawn_entity("ReloadPool") end
            GAME.initial = math.random()
        end
        function on_destroyed(_, reason) if reason == "reload" then return GAME end end
        function update(_, dt)
            GAME.ticks = GAME.ticks + 1
            GAME.sum = GAME.sum + math.random()
            set_translation(GAME.pool, GAME.ticks, 2, 0)
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
        scenario_path = work / 'tests/nested/scenario.json'
        a = tool('playtest', {**scenario, 'save_scenario': str(scenario_path)})
        b = tool('playtest', {'scenario': str(scenario_path)})
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
        tool('script_reload', {'entity': controller}); tool('wait', {'frames': 3})
        assert tool('lua_eval', {'entity': controller, 'code': 'return GAME.ticks', 'read_only': True})['result'] == 2
        tool('runtime_step', {'steps': 3})
        assert tool('entity_get', {'entity': 'ReloadPool'})['transform']['translation'] == [5, 2, 0]
        assert tool('script_status')['failing'] == 0
        tool('runtime_set', {'mode': 'stop'}); tool('wait', {'frames': 3})
        print('Seeded replay, input checkpoints, failure reporting and render-independent stepping passed', flush=True)

        profile = tool('profile_capture', {'frames': 30})
        assert profile['samples'] == 30 and profile['frame_time_ms']['p99'] >= profile['frame_time_ms']['median']
        comparison = tool('profile_capture', {'frames': 20, 'baseline': profile})
        assert 'p95' in comparison['comparison']
        save_report = tool('project_save')
        assert save_report['written_count'] > 0 and save_report['scenes'] and 'written' not in save_report
        assert tool('prefab_inspect', {'document_id': prefab['document_id']})['instance_entities']
        manifest = json.loads((work / 'fixture/project.json').read_text())
        host = next(d for d in manifest['scenes'] if d['id'] != prefab['document_id'])
        scene_path = work / 'fixture' / host['relative_path']
        pretty = scene_path.read_text()
        saved = json.loads(pretty)
        materials = set((work / 'fixture/content/materials').glob('*.mat.json'))
        verbose_save = tool('project_save', {'compact': True, 'verbose': True})
        compact = scene_path.read_text()
        assert json.loads(compact) == saved and len(compact) < len(pretty) / 2
        assert len(verbose_save['written']) == verbose_save['written_count']
        assert set((work / 'fixture/content/materials').glob('*.mat.json')) == materials
        assert any((e.get('scene_ref') or {}).get('document') == prefab['document_id'] for e in saved['entities'])
        tool('project_open', {'path': str(work / 'fixture')})
        reopened = tool('prefab_inspect', {'document_id': prefab['document_id']})
        assert reopened['instance_entities'] and reopened['instance_overrides']
        validation = tool('project_validate')
        assert validation['ok'], validation
        assert validation['browser']['ok'], validation['browser']
        print('Profile capture, comparison, persistence and project validation passed', flush=True)
        tool('screenshot', {'camera': 'game', 'save_path': str(work / 'scene.png')})
        # A second MCP server attaches to the editor without owning its process.
        # Replacing it with a player must work on the first engine_launch call.
        observer = subprocess.Popen([str(REPO / 'tools/engine-mcp/target/debug/engine-mcp'),
                                     '--repo', str(REPO), '--port', str(port)],
                                    stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True)
        player = None
        try:
            tool('engine_launch', {'target': 'editor', 'build': False}, transport=observer)
            tool('engine_stop')
            player_env = {**os.environ, 'ENGINE_REMOTE_PORT': str(port), 'RUST_LOG': 'warn'}
            with (work / 'replacement-player.log').open('w') as log:
                player = subprocess.Popen([str(REPO / 'target/debug/player'), str(work / 'fixture')],
                                          cwd=REPO, env=player_env, stdout=log, stderr=log)
                deadline = time.monotonic() + 30
                while True:
                    assert player.poll() is None, 'Replacement player exited; see replacement-player.log'
                    try:
                        with socket.create_connection(('127.0.0.1', port), timeout=0.2):
                            break
                    except OSError:
                        assert time.monotonic() < deadline, 'Replacement player did not open its port'
                        time.sleep(0.1)
                attached = tool('engine_launch', {'target': 'player', 'build': False}, transport=observer)
                assert 'attached' in attached and attached['engine']['runtime_mode'] == 'playing'
            print('External editor-to-player replacement reconnects on the first call', flush=True)
        finally:
            if player is not None:
                player.terminate()
                player.wait(timeout=10)
            observer.stdin.close()
            observer.wait(timeout=10)
        if opts.web:
            tool('engine_launch', {'target': 'editor', 'build': False,
                                   'project': str(work / 'fixture'), 'env': desktop})
            fixture = work / 'fixture'
            # An asset outside content/ validates natively but is never staged, so the browser
            # preflight must catch it before the slow wasm build.
            (fixture / 'textures').mkdir(exist_ok=True)
            (fixture / 'textures/outside.json').write_text('{}')
            (fixture / 'content/scenes/outside.json').write_text('{"material": "textures/outside.json"}')
            native = tool('project_validate', {'path': str(fixture)})
            assert native['ok'] and not native['browser']['ok'], native
            blocked = tool('web_build', {'project': str(fixture), 'overwrite': True}, error=True)
            assert blocked['ok'] is False and 'browser-ready' in blocked['note'], blocked
            (fixture / 'content/scenes/outside.json').unlink()
            (fixture / 'textures/outside.json').unlink()
            (fixture / 'textures').rmdir()
            # B20: file names holding literal percent escapes must load in the browser.
            (fixture / 'content/scripts').mkdir(parents=True, exist_ok=True)
            (fixture / 'content/scripts/pct%2ehello.lua').write_text(
                'function on_created(self) print("PCT_SCRIPT_LOADED") end\n')
            marker = tool('entity_spawn', {'kind': 'empty', 'name': 'PctMarker'})['id']
            tool('entity_update', {'entity': marker, 'script': {'path': 'content/scripts/pct%2ehello.lua'}})
            tool('project_save')
            built = tool('web_build', {'project': str(fixture), 'overwrite': True})
            browser = tool('web_playtest', {'directory': built['directory'], 'seconds': 2})
            assert browser['ready'] and browser['ok'], browser
            assert any('PCT_SCRIPT_LOADED' in line['text'] for line in browser['console']), browser['console']
            print('WASM packaging, browser preflight and WebGPU browser smoke test passed', flush=True)
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
