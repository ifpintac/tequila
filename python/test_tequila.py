import json
import socket
import socketserver
import tempfile
import threading
import unittest
from pathlib import Path
from tequila import Client, TequilaError


class ClientTests(unittest.TestCase):
    def setUp(self):
        self.received = []
        received = self.received

        class Handler(socketserver.StreamRequestHandler):
            def handle(self):
                for line in self.rfile:
                    req = json.loads(line)
                    received.append(req)
                    if req['op'] in ('render', 'slow'):
                        import time
                        time.sleep(0.6)
                    result = {'protocol': 1, 'pid': 123} if req['op'] == 'ping' else req
                    reply = {'id': req['id'], 'ok': True, 'result': result}
                    if req['op'] == 'fail':
                        reply = {'id': req['id'], 'ok': False, 'error': 'no such material'}
                    if req['op'] == 'wrong_id':
                        reply['id'] += 1
                    self.wfile.write(json.dumps(reply).encode() + b'\n')
                    self.wfile.flush()

        self.server = socketserver.ThreadingTCPServer(('127.0.0.1', 0), Handler)
        self.server.daemon_threads = True
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.directory = tempfile.TemporaryDirectory()
        self.discovery = Path(self.directory.name) / 'channel.json'
        self.discovery.write_text(json.dumps(dict(port=self.server.server_address[1], token='test-token', pid=123)))

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.directory.cleanup()

    def test_handshake_authentication_and_exact_edit_shape(self):
        with Client(self.discovery, expected_pid=123) as fs:
            fs.set_material_parameter(9, 'specular_roughness', 0.25)
            fs.context()
        self.assertEqual([r['id'] for r in self.received], [1, 2, 3])
        self.assertTrue(all(r['token'] == 'test-token' for r in self.received))
        self.assertEqual(self.received[1]['command'], dict(command='material.set_parameter', material=9, parameter='specular_roughness', value=0.25))
        self.assertEqual(self.received[2]['what'], 'context')

    def test_a_render_waits_longer_than_an_edit(self):
        # A final render takes minutes; the connection's timeout is sized for
        # edits. Scripts that rendered at 512 samples lost the connection at
        # 30 seconds while the render went on without them.
        with Client(self.discovery, timeout=0.3) as fs:
            fs.render('out.png', width=8, height=8, samples=1, timeout=5)
            self.assertEqual(fs._socket.gettimeout(), 0.3)
        with Client(self.discovery, timeout=0.3) as fs:
            with self.assertRaises(TequilaError):
                fs.request('slow')

    def test_error_is_not_retried_and_connection_remains_usable(self):
        with Client(self.discovery) as fs:
            with self.assertRaisesRegex(TequilaError, 'no such material'):
                fs.request('fail')
            fs.query('scene')
        self.assertEqual(sum(r['op'] == 'fail' for r in self.received), 1)

    def test_a_batch_is_one_request_sent_on_leaving_the_block(self):
        with Client(self.discovery) as fs:
            with fs.batch() as b:
                b.apply('material.set_type', material=2, type='velvet')
                b.select([4, 5])
                b.look(view='front', frm=(0, 1, 3))
        batch = self.received[-1]
        self.assertEqual(batch['op'], 'batch')
        self.assertTrue(batch['atomic'])
        self.assertEqual([o['op'] for o in batch['ops']], ['apply', 'select', 'look'])
        self.assertEqual(batch['ops'][2], {'op': 'look', 'view': 'front', 'from': [0, 1, 3]})

    def test_a_batch_whose_block_raises_sends_nothing(self):
        with Client(self.discovery) as fs:
            with self.assertRaises(ValueError):
                with fs.batch() as b:
                    b.select([1])
                    raise ValueError('changed my mind')
        self.assertNotIn('batch', [r['op'] for r in self.received])

    def test_quit_sends_its_choice_and_closes_the_connection(self):
        fs = Client(self.discovery)
        fs.quit(discard=True)
        self.assertEqual(self.received[-1]['op'], 'quit')
        self.assertTrue(self.received[-1]['discard'])
        self.assertFalse(self.received[-1]['save'])

    def test_a_crashed_instance_falls_back_to_one_still_running(self):
        # The shared file names a port nobody listens on any more; the
        # running instance's own record is beside it.
        with socket.socket() as dead:
            dead.bind(('127.0.0.1', 0))
            dead_port = dead.getsockname()[1]
        live = self.discovery.read_text()
        self.discovery.write_text(json.dumps(dict(port=dead_port, token='gone', pid=77)))
        (self.discovery.parent / 'channels').mkdir()
        (self.discovery.parent / 'channels' / 'live.json').write_text(live)
        with Client(self.discovery, expected_pid=123) as fs:
            fs.context()
        self.assertTrue(all(r['token'] == 'test-token' for r in self.received))

    def test_no_instance_at_all_says_so(self):
        self.discovery.unlink()
        with self.assertRaisesRegex(TequilaError, 'not running'):
            Client(self.discovery)

    def test_wrong_session_is_refused_before_connecting(self):
        with self.assertRaises(TequilaError):
            Client(self.discovery, expected_pid=999)
        self.assertEqual(self.received, [])

    def test_mismatched_reply_is_not_retried(self):
        with Client(self.discovery) as fs:
            with self.assertRaisesRegex(TequilaError, 'Reply ID mismatch'):
                fs.request('wrong_id')
        self.assertEqual(len(self.received), 2)

    def test_existing_screenshot_cannot_be_mistaken_for_new_work(self):
        target = Path(self.directory.name) / 'old.png'
        target.write_bytes(b'old')
        with Client(self.discovery) as fs:
            with self.assertRaises(FileExistsError):
                fs.screenshot(target)
        self.assertEqual(len(self.received), 1)


if __name__ == '__main__':
    unittest.main()
