"""Plugins, end to end: tequila_plugin.py runs a plugin's code against a
headless Tequila, as the application runs it against its window.

Needs the command-line renderer built (`cargo build -p tequila-cli`, or a
distribution's tequila-cli); skipped when there is none.
"""
import json
import os
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

import tequila
from tequila import TequilaError

HERE = Path(__file__).resolve().parent
HOST = HERE / 'tequila_plugin.py'
EXAMPLES = HERE.parent / 'plugins' / 'examples'
MODEL = HERE.parent / 'crates' / 'tequila-import' / 'tests' / 'data' / 'two_bodies.step'

try:
    tequila._find_cli()
    MISSING = None
except TequilaError as error:
    MISSING = str(error)


class Running:
    """A plugin's host process, connected to `session`."""

    def __init__(self, folder, session, data=None):
        env = dict(os.environ)
        # Never the person's own plugin-data folder.
        self._data = None
        if data is None:
            self._data = tempfile.TemporaryDirectory()
            data = self._data.name
        env['TEQUILA_PLUGIN_DATA'] = str(data)
        env['TEQUILA_DISCOVERY'] = str(Path(session._folder.name) / 'channel.json')
        env['TEQUILA_PID'] = str(session.pid)
        env['PYTHONPATH'] = str(HERE)
        self.process = subprocess.Popen(
            [sys.executable, '-u', str(HOST), str(folder)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=env, text=True, encoding='utf-8')
        self.expect(lambda m: m['type'] == 'ready')

    def send(self, message):
        self.process.stdin.write(json.dumps(message) + '\n')
        self.process.stdin.flush()

    def expect(self, wanted):
        heard = []
        for line in self.process.stdout:
            message = json.loads(line)
            heard.append(message)
            if message['type'] == 'error':
                raise AssertionError(message['text'])
            if wanted(message):
                return heard
        raise AssertionError(f'the plugin stopped; it said {heard}; stderr: {self.process.stderr.read()}')

    def close(self):
        self.send({'type': 'shutdown'})
        self.process.wait(timeout=10)
        for pipe in (self.process.stdin, self.process.stdout, self.process.stderr):
            pipe.close()
        if self._data is not None:
            self._data.cleanup()


@unittest.skipIf(MISSING, MISSING)
class PluginTests(unittest.TestCase):
    def test_a_plugins_command_changes_the_scene_through_the_channel(self):
        with tempfile.TemporaryDirectory() as folder:
            folder = Path(folder)
            (folder / 'plugin.json').write_text(json.dumps({
                'id': 'test.rename', 'name': 'Rename', 'main': 'main.py', 'commands': [{'id': 'rename'}]}), encoding='utf-8')
            (folder / 'main.py').write_text(
                "import tequila_plugin as plugin\n\n"
                "@plugin.command('rename')\n"
                "def rename(fs, values):\n"
                "    fs.commands.scene.rename(node=values['node'], name=values['name'])\n"
                "    plugin.status('renamed')\n", encoding='utf-8')
            with tequila.open(MODEL) as fs:
                part = fs.parts()[0]
                running = Running(folder, fs)
                try:
                    running.send({'type': 'command', 'command': 'rename', 'values': {'node': part.id, 'name': 'Lid'}})
                    heard = running.expect(lambda m: m['type'] == 'done')
                    self.assertIn({'type': 'status', 'text': 'renamed'}, heard)
                finally:
                    running.close()
                self.assertEqual([p.name for p in fs.find(name='lid')], ['Lid'])
                # Undoable, like any edit.
                fs.undo()
                self.assertEqual(fs.find(name='lid'), [])

    def test_the_colourways_example_follows_the_selection(self):
        with tequila.open(MODEL) as fs:
            part = next(p for p in fs.parts() if p.material is not None)
            material = fs.materials[part.material]
            fs.select([part.id])
            running = Running(EXAMPLES / 'colourways', fs)
            try:
                running.send({'type': 'event', 'event': 'selection', 'data': {'nodes': [part.id]}})
                heard = running.expect(lambda m: m['type'] == 'panel')
                self.assertIn(material.name, heard[-1]['values']['status'])
            finally:
                running.close()


@unittest.skipIf(MISSING, MISSING)
class AutoMaterialsTests(unittest.TestCase):
    """The Auto Materials example: rules kept in its data folder, applied to
    an import's new parts as one undo step."""

    def test_an_imports_parts_get_their_rules_materials_in_one_step(self):
        with tempfile.TemporaryDirectory() as data, tequila.open(MODEL) as fs:
            Path(data, 'settings.json').write_text(json.dumps(
                {'rules': 'pin: Chrome\nbox: Not A Library Material', 'on_import': True}), encoding='utf-8')
            parts = {p.name: p for p in fs.parts()}
            pin, before = parts['Pin'], parts['Pin'].material
            running = Running(EXAMPLES / 'auto-materials', fs, data)
            try:
                running.send({'type': 'event', 'event': 'imported',
                              'data': {'path': str(MODEL), 'nodes': [p.id for p in parts.values()], 'parts': [p.id for p in parts.values()]}})
                heard = running.expect(lambda m: m['type'] == 'done')
            finally:
                running.close()
            said = next(m['text'] for m in heard if m['type'] == 'status')
            self.assertIn('1 part(s)', said)
            self.assertIn('Not A Library Material', said)
            now = {p.name: p for p in fs.parts()}
            self.assertEqual(fs.materials[now['Pin'].material].name, 'Chrome')
            self.assertEqual(now['Box (Colour #FF0000)'].material, parts['Box (Colour #FF0000)'].material, 'a missing material changed a part')
            fs.undo()
            self.assertEqual(next(p for p in fs.parts() if p.name == 'Pin').material, before, 'not one undo step')

    def test_kept_rules_fill_the_panel_and_imports_can_be_left_alone(self):
        with tempfile.TemporaryDirectory() as data, tequila.open(MODEL) as fs:
            running = Running(EXAMPLES / 'auto-materials', fs, data)
            try:
                running.send({'type': 'command', 'command': 'keep', 'values': {'rules': 'pin: Chrome', 'on_import': False, 'rules_file': ''}})
                running.expect(lambda m: m['type'] == 'done')
                running.send({'type': 'event', 'event': 'panel_opened', 'data': {'panel': 'auto'}})
                panel = running.expect(lambda m: m['type'] == 'panel')[-1]['values']
                self.assertEqual(panel['rules'], 'pin: Chrome')
                self.assertFalse(panel['on_import'])
                pin = next(p for p in fs.parts() if p.name == 'Pin')
                running.send({'type': 'event', 'event': 'imported', 'data': {'path': 'x', 'nodes': [pin.id], 'parts': [pin.id]}})
                running.expect(lambda m: m['type'] == 'done')
            finally:
                running.close()
            self.assertEqual(next(p for p in fs.parts() if p.name == 'Pin').material, pin.material, 'an import was not left alone')
            self.assertTrue(Path(data, 'settings.json').is_file())

    def test_names_match_by_word(self):
        import importlib.util
        spec = importlib.util.spec_from_file_location('auto_materials', EXAMPLES / 'auto-materials' / 'main.py')
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        rules = module.parse_rules(module.DEFAULT_RULES)
        self.assertEqual(module.match('M3x8 Screws', rules), 'Stainless Brushed')
        self.assertEqual(module.match('FrontLens_02', rules), 'Glass Clear')
        self.assertEqual(module.match('O-Ring seal', rules), 'Rubber Black')
        self.assertIsNone(module.match('Peanut', rules), 'a word inside another word matched')
        with tempfile.TemporaryDirectory() as folder:
            csv = Path(folder, 'rules.csv')
            csv.write_text('# words, material\nknob|dial,Aluminium Knurled Straight\n', encoding='utf-8')
            self.assertEqual(module.match('Volume Dial', module.rules_from_file(csv)), 'Aluminium Knurled Straight')


@unittest.skipIf(MISSING, MISSING)
class RenderEveryCameraTests(unittest.TestCase):
    def test_the_panel_names_the_saved_cameras(self):
        with tequila.open(MODEL) as fs:
            running = Running(EXAMPLES / 'render-every-camera', fs)
            try:
                running.send({'type': 'event', 'event': 'panel_opened', 'data': {'panel': 'cameras'}})
                status = running.expect(lambda m: m['type'] == 'panel')[-1]['values']['status']
            finally:
                running.close()
            self.assertIn('1 saved camera(s): Camera 1', status)


VIBE_NAMES = json.loads((EXAMPLES / 'vibes' / 'plugin.json').read_text(encoding='utf-8'))['panels'][0]['fields'][0]['options']


def look(fs):
    document = fs.context()['document']
    return json.dumps({'environment': document['environment'], 'image_style': document['image_style']}, sort_keys=True)


@unittest.skipIf(MISSING, MISSING)
class VibesTests(unittest.TestCase):
    """The Vibes example: what it changes, that it is undoable as one step,
    and that Back to my look puts back the person's own look."""

    def run_command(self, running, command, **values):
        running.send({'type': 'command', 'command': command, 'values': values})
        return running.expect(lambda m: m['type'] == 'done')

    def test_every_vibe_applies_and_back_to_my_look_puts_it_all_back(self):
        with tequila.open(MODEL) as fs:
            mine = look(fs)
            running = Running(EXAMPLES / 'vibes', fs)
            try:
                for name in VIBE_NAMES:
                    heard = self.run_command(running, 'apply', vibe=name, drama=100)
                    self.assertIn({'type': 'status', 'text': f'Vibe: {name}'}, heard)
                    document = fs.context()['document']
                    pins = document['environment'].get('pins', [])
                    self.assertTrue(pins and all(p['name'].startswith('Vibe: ') for p in pins), name)
                    self.assertEqual(document['image_style']['type'], 'Photographic', name)
                self.run_command(running, 'restore')
                self.assertEqual(look(fs), mine, 'eight vibes later, the look was not put back')
                heard = self.run_command(running, 'restore')
                self.assertTrue(any('already yours' in m.get('text', '') for m in heard), heard)
            finally:
                running.close()

    def test_a_vibe_is_one_undo_step(self):
        with tequila.open(MODEL) as fs:
            mine = look(fs)
            running = Running(EXAMPLES / 'vibes', fs)
            try:
                self.run_command(running, 'apply', vibe='Golden Hour', drama=100)
                self.assertNotEqual(look(fs), mine)
                fs.undo()
                self.assertEqual(look(fs), mine)
            finally:
                running.close()

    def test_a_look_changed_by_hand_between_vibes_is_the_one_kept(self):
        with tequila.open(MODEL) as fs:
            running = Running(EXAMPLES / 'vibes', fs)
            try:
                self.run_command(running, 'apply', vibe='Keynote', drama=100)
                fs.commands.environment.set_parameter(parameter='brightness', value=1.7)
                tuned = look(fs)
                self.run_command(running, 'apply', vibe='Noir', drama=100)
                self.run_command(running, 'restore')
                self.assertEqual(look(fs), tuned)
            finally:
                running.close()

    def test_shuffle_never_repeats_and_drama_scales_the_grade(self):
        with tequila.open(MODEL) as fs:
            running = Running(EXAMPLES / 'vibes', fs)
            try:
                shown = []
                for _ in range(12):
                    heard = self.run_command(running, 'shuffle', drama=100)
                    shown += [m['values']['vibe'] for m in heard if m['type'] == 'panel']
                self.assertEqual(len(shown), 12)
                self.assertTrue(all(a != b for a, b in zip(shown, shown[1:])), shown)
                self.assertTrue(set(shown) <= set(VIBE_NAMES))
                self.run_command(running, 'apply', vibe='Noir', drama=0)
                style = fs.context()['document']['image_style']
                self.assertEqual((style['saturation'], style['contrast'], style['vignette']), (1, 1, 0))
                self.run_command(running, 'apply', vibe='Noir', drama=100)
                self.assertEqual(fs.context()['document']['image_style']['saturation'], 0)
            finally:
                running.close()

    def test_a_vibe_on_an_empty_scene(self):
        # No parts and no camera to speak of: the vibe still goes on, and
        # the document's reply carries no infinite bounds.
        with tequila.open() as fs:
            running = Running(EXAMPLES / 'vibes', fs)
            try:
                self.run_command(running, 'apply', vibe='Deep Space', drama=100)
                self.assertFalse(fs.context()['document']['environment']['ground']['enabled'])
            finally:
                running.close()


if __name__ == '__main__':
    unittest.main()
