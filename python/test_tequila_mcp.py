import base64
import io
import json
import socketserver
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path

import tequila_mcp
from tequila_mcp import Bridge, handle, serve, TOOLS

# A 1x1 PNG, for a screenshot the fake application "takes".
PNG = base64.b64decode(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==')

NODES = {'nodes': [
    {'id': 1, 'name': 'Quest 3', 'kind': 'group', 'visible': True},
    {'id': 360, 'name': 'part 12', 'kind': 'part', 'material': 2, 'material_name': 'Strap Felt', 'visible': True,
     'bounds': {'min': [0, 0, 0], 'max': [0.1, 0.2, 0.3], 'size': [0.1, 0.2, 0.3]}},
    {'id': 361, 'name': 'Lens', 'kind': 'part', 'material': 3, 'material_name': 'Glass', 'visible': True,
     'bounds': {'min': [0, 0, 0], 'max': [1, 1, 1], 'size': [1, 1, 1]}},
]}


class FakeTequila:
    """Just enough of the channel to answer what the bridge asks."""

    def __init__(self):
        self.received = []
        received = self.received
        plugin_log = []

        class Handler(socketserver.StreamRequestHandler):
            def handle(self):
                for line in self.rfile:
                    req = json.loads(line)
                    received.append(req)
                    op = req['op']
                    result = req
                    ok = True
                    if op == 'ping':
                        result = {'protocol': 1, 'pid': 123}
                    elif op == 'query' and req.get('what') == 'nodes':
                        result = NODES
                    elif op == 'screenshot':
                        Path(req['path']).write_bytes(PNG)
                        result = {'path': req['path'], 'pending': True}
                    elif op == 'apply' and req['command'].get('command') == 'refuse':
                        ok, result = False, 'no such material'
                    elif op == 'plugin' and req['action'] == 'run':
                        # It runs on its own: its lines arrive a moment later.
                        threading.Timer(0.3, plugin_log.extend, [['working', f"finished `{req['command']}`"]]).start()
                        result = {'sent': True}
                    elif op == 'plugin' and req['action'] == 'log':
                        lines = (['old line'] * 3 + plugin_log)[-req.get('lines', 40):]
                        result = {'plugin': req['plugin'], 'lines': lines, 'total': 3 + len(plugin_log), 'running': True}
                    reply = {'id': req['id'], 'ok': ok}
                    reply['result' if ok else 'error'] = result
                    self.wfile.write(json.dumps(reply).encode() + b'\n')
                    self.wfile.flush()

        self.server = socketserver.ThreadingTCPServer(('127.0.0.1', 0), Handler)
        self.server.daemon_threads = True
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.directory = tempfile.TemporaryDirectory()
        self.discovery = Path(self.directory.name) / 'channel.json'
        self.discovery.write_text(json.dumps(dict(port=self.server.server_address[1], token='t', pid=123)))

    def close(self):
        self.server.shutdown()
        self.server.server_close()
        self.directory.cleanup()


class BridgeTests(unittest.TestCase):
    def setUp(self):
        self.app = FakeTequila()
        self.bridge = Bridge(self.app.discovery)

    def tearDown(self):
        self.bridge.close()
        self.app.close()

    def rpc(self, method, params=None, ident=1):
        return handle(self.bridge, {'jsonrpc': '2.0', 'id': ident, 'method': method, 'params': params or {}})

    def call(self, name, arguments=None):
        return self.rpc('tools/call', {'name': name, 'arguments': arguments or {}})['result']

    def test_initialize_speaks_the_clients_protocol_version(self):
        result = self.rpc('initialize', {'protocolVersion': '2025-03-26', 'capabilities': {}})['result']
        self.assertEqual(result['protocolVersion'], '2025-03-26')
        self.assertIn('tools', result['capabilities'])
        self.assertEqual(result['serverInfo']['name'], 'tequila')
        unknown = self.rpc('initialize', {'protocolVersion': '1999-01-01'})['result']
        self.assertEqual(unknown['protocolVersion'], tequila_mcp.PROTOCOL_VERSIONS[0])

    def test_every_listed_tool_has_a_handler_and_a_valid_schema(self):
        tools = self.rpc('tools/list')['result']['tools']
        names = [t['name'] for t in tools]
        self.assertEqual(len(names), len(set(names)))
        for t in tools:
            self.assertTrue(hasattr(Bridge, 'tool_' + t['name'].removeprefix('tequila_')), t['name'])
            schema = t['inputSchema']
            self.assertEqual(schema['type'], 'object')
            for required in schema['required']:
                self.assertIn(required, schema['properties'])

    def test_apply_forwards_the_command_exactly(self):
        result = self.call('tequila_apply', {'command': {'command': 'material.set_type', 'material': 2, 'type': 'velvet'}})
        self.assertFalse(result['isError'])
        sent = self.app.received[-1]
        self.assertEqual(sent['op'], 'apply')
        self.assertEqual(sent['command'], {'command': 'material.set_type', 'material': 2, 'type': 'velvet'})

    def test_a_refusal_is_a_result_the_assistant_reads(self):
        result = self.call('tequila_apply', {'command': {'command': 'refuse'}})
        self.assertTrue(result['isError'])
        self.assertIn('no such material', result['content'][0]['text'])
        # The connection survives a refusal.
        self.assertFalse(self.call('tequila_scene')['isError'])

    def test_nodes_are_filtered_and_slimmed(self):
        result = json.loads(self.call('tequila_nodes', {'filter': 'strap'})['content'][0]['text'])
        self.assertEqual(result['count'], 1)
        node = result['nodes'][0]
        self.assertEqual(node['id'], 360)
        self.assertEqual(node['size'], [0.1, 0.2, 0.3])
        self.assertNotIn('bounds', node)
        parts = json.loads(self.call('tequila_nodes', {'parts_only': True})['content'][0]['text'])
        self.assertEqual(parts['count'], 2)

    def test_a_screenshot_comes_back_as_an_image(self):
        result = self.call('tequila_screenshot', {'viewport': True})
        self.assertFalse(result['isError'])
        picture = result['content'][0]
        self.assertEqual(picture['type'], 'image')
        self.assertEqual(picture['mimeType'], 'image/png')
        self.assertEqual(base64.b64decode(picture['data']), PNG)
        self.assertTrue(self.app.received[-1]['viewport'])

    def test_look_passes_from_through(self):
        self.call('tequila_look', {'from': [0, 1, 3], 'at': [0, 0, 0]})
        sent = self.app.received[-1]
        self.assertEqual((sent['op'], sent['from'], sent['at']), ('look', [0, 1, 3], [0, 0, 0]))

    def test_running_a_plugin_command_waits_for_what_it_logs(self):
        result = self.call('tequila_plugin_run', {'id': 'my.test', 'command': 'go', 'values': {'n': 2}})
        said = json.loads(result['content'][0]['text'])
        self.assertTrue(said['finished'])
        # Only what it logged after the command was sent.
        self.assertEqual(said['log'], ['working', 'finished `go`'])
        run = next(r for r in self.app.received if r['op'] == 'plugin' and r['action'] == 'run')
        self.assertEqual(run['values'], {'n': 2})
        self.assertEqual(run['plugin'], 'my.test')

    def test_unknown_tools_and_methods_are_protocol_errors(self):
        self.assertEqual(self.rpc('tools/call', {'name': 'tequila_explode'})['error']['code'], -32602)
        self.assertEqual(self.rpc('resources/list')['error']['code'], -32601)
        self.assertIsNone(handle(self.bridge, {'jsonrpc': '2.0', 'method': 'notifications/initialized'}))

    def test_a_closed_application_is_explained(self):
        self.app.close()
        self.app = FakeTequila()  # tearDown closes this one
        bridge = Bridge(Path(tempfile.gettempdir()) / 'no-such-channel.json')
        result = handle(bridge, {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call', 'params': {'name': 'tequila_scene'}})['result']
        self.assertTrue(result['isError'])
        self.assertIn('open', result['content'][0]['text'])


class RestrictedTests(unittest.TestCase):
    """What ChatGPT reaches through the tunnel: the scene, nothing beyond it."""

    def setUp(self):
        self.app = FakeTequila()
        self.bridge = Bridge(self.app.discovery, restricted=True)

    def tearDown(self):
        self.bridge.close()
        self.app.close()

    def call(self, name, arguments=None):
        return handle(self.bridge, {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/call',
                                    'params': {'name': name, 'arguments': arguments or {}}})

    def test_tools_that_reach_outside_the_scene_are_not_offered(self):
        names = {t['name'] for t in handle(self.bridge, {'jsonrpc': '2.0', 'id': 1, 'method': 'tools/list'})['result']['tools']}
        self.assertNotIn('tequila_request', names)
        self.assertNotIn('tequila_import', names)
        self.assertFalse(any(n.startswith('tequila_plugin') for n in names))
        self.assertIn('tequila_apply', names)
        save = next(t for t in self.bridge.tools() if t['name'] == 'tequila_save')
        self.assertNotIn('path', save['inputSchema']['properties'])
        # The unrestricted list is untouched.
        self.assertIn('path', next(t for t in TOOLS if t['name'] == 'tequila_save')['inputSchema']['properties'])

    def test_hidden_tools_cannot_be_called_by_name(self):
        self.assertEqual(self.call('tequila_request', {'request': {'op': 'quit', 'discard': True}})['error']['code'], -32602)
        self.assertEqual(self.call('tequila_import', {'path': 'C:/x.glb'})['error']['code'], -32602)
        self.assertNotIn('quit', [r['op'] for r in self.app.received])

    def test_paths_are_refused(self):
        command = {'command': 'render.set_settings', 'render': {'output_directory': 'C:/outside', 'filename_template': 'redirected'}}
        for name, arguments in [('tequila_apply', {'command': command}),
                                ('tequila_batch', {'ops': [{'op': 'apply', 'command': command}]})]:
            result = self.call(name, arguments)['result']
            self.assertTrue(result['isError'])
            self.assertIn('controlled locally', result['content'][0]['text'])
        self.assertFalse(any(r['op'] in ('apply', 'batch') for r in self.app.received))
        for name, arguments in [('tequila_save', {'path': 'C:/Windows/evil.tequila'}),
                                ('tequila_queue', {'folder': 'C:/Windows'}),
                                ('tequila_batch', {'ops': [{'op': 'screenshot', 'path': 'C:/Windows/x.png'}]}),
                                ('tequila_batch', {'ops': [{'op': 'save', 'path': 'C:/x.tequila'}]})]:
            result = self.call(name, arguments)['result']
            self.assertTrue(result['isError'], name)
        self.assertFalse(any(r['op'] in ('save', 'screenshot', 'queue', 'batch') for r in self.app.received))

    def test_scene_work_still_goes_through(self):
        result = self.call('tequila_batch', {'ops': [{'op': 'select', 'nodes': [1]}, {'op': 'look', 'view': 'front'}]})['result']
        self.assertFalse(result['isError'])
        self.assertEqual(self.app.received[-1]['op'], 'batch')


class StdioTests(unittest.TestCase):
    """The program itself, as an MCP client would start it."""

    def test_the_server_answers_over_stdio(self):
        lines = [
            {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {'protocolVersion': '2025-06-18'}},
            {'jsonrpc': '2.0', 'method': 'notifications/initialized'},
            {'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list'},
        ]
        stdin = '\n'.join(json.dumps(l) for l in lines) + '\n'
        process = subprocess.run([sys.executable, str(Path(tequila_mcp.__file__))], input=stdin,
                                 capture_output=True, text=True, timeout=30)
        replies = [json.loads(l) for l in process.stdout.splitlines() if l.strip()]
        self.assertEqual([r['id'] for r in replies], [1, 2], process.stderr)
        self.assertEqual(len(replies[1]['result']['tools']), len(TOOLS))

    def test_garbage_is_a_parse_error_not_a_crash(self):
        out = io.StringIO()
        serve(Bridge(), io.StringIO('not json\n{"jsonrpc":"2.0","id":5,"method":"ping"}\n'), out)
        replies = [json.loads(l) for l in out.getvalue().splitlines()]
        self.assertEqual(replies[0]['error']['code'], -32700)
        self.assertEqual(replies[1], {'jsonrpc': '2.0', 'id': 5, 'result': {}})


if __name__ == '__main__':
    unittest.main()
