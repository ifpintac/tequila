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

    def test_a_red_part_keeps_its_red_with_the_rules_finish(self):
        with tempfile.TemporaryDirectory() as data, tequila.open(MODEL) as fs:
            Path(data, 'settings.json').write_text(json.dumps({'rules': 'box: ABS White', 'builtin': False}), encoding='utf-8')
            box = next(p for p in fs.parts() if p.name.startswith('Box'))
            before = box.material
            running = Running(EXAMPLES / 'auto-materials', fs, data)
            try:
                running.send({'type': 'command', 'command': 'apply', 'values': {}})
                running.expect(lambda m: m['type'] == 'done')
            finally:
                running.close()
            box = next(p for p in fs.parts() if p.name.startswith('Box'))
            material = fs.materials[box.material]
            self.assertEqual(material.name, 'ABS White (red)')
            red = material.base_color
            self.assertGreater(red[0], 0.5)
            self.assertLess(red[1], 0.1)
            # The finish is the preset's: its roughness, not the import's.
            self.assertEqual(material.type.lower(), 'plastic')
            fs.undo()
            fs.undo()
            self.assertEqual(next(p for p in fs.parts() if p.name.startswith('Box')).material, before)


    def test_what_you_gave_a_part_is_learned_when_you_save(self):
        with tempfile.TemporaryDirectory() as data, tequila.open(MODEL) as fs:
            pin = next(p for p in fs.parts() if p.name == 'Pin')
            fs.library.apply('Brass Satin', pin)
            running = Running(EXAMPLES / 'auto-materials', fs, data)
            try:
                running.send({'type': 'event', 'event': 'saved', 'data': {'path': 'x.tequila'}})
                running.expect(lambda m: m['type'] == 'done')
            finally:
                running.close()
            learned = json.loads(Path(data, 'settings.json').read_text(encoding='utf-8'))['learned']
            self.assertEqual(learned, {'pin': 'Brass Satin'})
        # A new model with a part of that name: it gets the learned one.
        with tempfile.TemporaryDirectory() as data, tequila.open(MODEL) as fs:
            Path(data, 'settings.json').write_text(json.dumps({'learned': {'pin': 'Brass Satin'}, 'rules': ''}), encoding='utf-8')
            running = Running(EXAMPLES / 'auto-materials', fs, data)
            try:
                running.send({'type': 'command', 'command': 'preview', 'values': {}})
                heard = running.expect(lambda m: m['type'] == 'done')
                running.send({'type': 'command', 'command': 'apply', 'values': {}})
                running.expect(lambda m: m['type'] == 'done')
            finally:
                running.close()
            status = next(m['values']['status'] for m in heard if m['type'] == 'panel')
            self.assertIn('learned 1', status)
            pin = next(p for p in fs.parts() if p.name == 'Pin')
            self.assertEqual(fs.materials[pin.material].name, 'Brass Satin')

    def test_parts_nothing_names_still_get_a_finish(self):
        with tempfile.TemporaryDirectory() as data, tequila.open(MODEL) as fs:
            # No rules at all: the pin is a slender rod, the red box a red moulding.
            Path(data, 'settings.json').write_text(json.dumps({'rules': '', 'builtin': False}), encoding='utf-8')
            running = Running(EXAMPLES / 'auto-materials', fs, data)
            try:
                running.send({'type': 'command', 'command': 'apply', 'values': {}})
                running.expect(lambda m: m['type'] == 'done')
            finally:
                running.close()
            parts = {p.name: p for p in fs.parts()}
            self.assertEqual(fs.materials[parts['Pin'].material].name, 'Steel Turned')
            box = fs.materials[next(p for n, p in parts.items() if n.startswith('Box')).material]
            self.assertEqual(box.name, 'Plastic Semi-Gloss Grey (red)')
            self.assertGreater(box.base_color[0], 0.5)


class AutoMaterialsMatchingTests(unittest.TestCase):
    """How Auto Materials reads names, with no Tequila running."""

    @classmethod
    def setUpClass(cls):
        import importlib.util
        spec = importlib.util.spec_from_file_location('auto_materials', EXAMPLES / 'auto-materials' / 'main.py')
        cls.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(cls.module)

    def test_names_match_by_word(self):
        module = self.module
        rules = module.parse_rules(module.DEFAULTS['rules'])
        self.assertEqual(module.match_text('M3x8 Screws', rules), 'Stainless Brushed')
        self.assertEqual(module.match_text('FrontLens_02', rules), 'Glass Clear')
        self.assertEqual(module.match_text('O-Ring seal', rules), 'Rubber Black')
        self.assertIsNone(module.match_text('Peanut', rules), 'a word inside another word matched')
        with tempfile.TemporaryDirectory() as folder:
            csv = Path(folder, 'rules.csv')
            csv.write_text('# words, material\nknob|dial,Aluminium Knurled Straight\n', encoding='utf-8')
            self.assertEqual(module.match_text('Volume Dial', module.rules_from_file(csv)), 'Aluminium Knurled Straight')

    def test_what_a_part_is_made_of_wins_over_what_it_is_called(self):
        module = self.module
        rules = module.BUILTIN
        own = [('Housing', 'name'), ('Aluminum 6061-T6', 'CAD material')]
        self.assertEqual(module.match([own, []], rules), ('Aluminium Bead Blasted', 'CAD material'))
        self.assertEqual(module.match([[('Housing', 'name'), ('Default', 'CAD material')], []], rules)[0], 'Plastic VDI 24 Grey')
        self.assertEqual(module.match([[('ISO 4762 M3x10 SHCS', 'name')], []], rules)[0], 'Steel Black Oxide')
        self.assertEqual(module.match([[('PrintedBracket', 'name')], []], rules)[0], 'Plastic VDI 24 Grey')
        self.assertEqual(module.match([[('Bracket', 'name'), ('PLA', 'CAD material')], []], rules)[0], 'FDM PLA White')
        # An assembly only when the part itself says nothing.
        self.assertEqual(module.match([[('Part7', 'name')], [('Screws', 'assembly')]], rules), ('Stainless Brushed', 'assembly'))
        self.assertEqual(module.match([[('Lens', 'name')], [('Screws', 'assembly')]], rules)[0], 'Glass Clear')
        self.assertEqual(module.match([[('Part7', 'name')], []], rules), (None, None))

    def test_shorthand_other_languages_and_standards_are_read(self):
        module, rules = self.module, self.module.BUILTIN
        named = lambda name: module.match([[(name, 'name')]], rules)[0]
        for name, wanted in [
            ('Gehäuse_oben', 'Plastic VDI 24 Grey'), ('PRT-1023_HSG', 'Plastic VDI 24 Grey'), ('BRKT-LEFT', 'Plastic VDI 24 Grey'),
            ('Glas Front', 'Glass Clear'), ('Dichtung', 'Rubber Black'), ('Écrou M5', 'Stainless Brushed'),
            ('Tornillo M3', 'Stainless Brushed'), ('Vite TCEI', 'Stainless Brushed'), ('Rondelle M4', 'Stainless Brushed'),
            ('ISO 4762 M3x10', 'Steel Black Oxide'), ('DIN 934 M4', 'Stainless Brushed'), ('ISO 7089 - 4', 'Stainless Brushed'),
            ('91290A115', 'Stainless Brushed'), ('#4-40 x 1/2', 'Stainless Brushed'), ('M2.5x6', 'Stainless Brushed'),
        ]:
            self.assertEqual(named(name), wanted, name)
        for name in ('Peanut', 'Nutella jar', 'M4', 'Part12'):
            self.assertIsNone(named(name), f'{name} matched')

    def test_copies_and_generic_names_are_told_apart(self):
        module = self.module
        for name in ('Screw <3>', 'Screw_2', 'Screw (2)', 'Screw.001', 'Screw:1', 'Screw 2'):
            self.assertEqual(module.stem(name), 'Screw', name)
        for name in ('Part12', 'Body3', 'Solid1', 'Part 7', 'Unnamed'):
            self.assertTrue(module.says_nothing(name), name)
        self.assertFalse(module.says_nothing('Front Bezel'))

    def test_a_part_nothing_names_is_guessed_from_its_shape_and_colour(self):
        module = self.module
        part = lambda size: type('P', (), {'bounds': {'size': size}})()
        material = lambda **d: type('M', (), {'definition': d})()
        guess = module.guess
        self.assertEqual(guess(part([0.05, 0.05, 0.02]), material(transmission_weight=1.0), 1, 0.3)[0], 'Glass Clear')
        self.assertEqual(guess(part([0.006, 0.006, 0.012]), material(base_color=[0.5, 0.5, 0.5]), 12, 0.3)[0], 'Stainless Brushed')
        self.assertEqual(guess(part([0.008, 0.008, 0.06]), material(base_color=[0.5, 0.5, 0.5]), 1, 0.3)[0], 'Steel Turned')
        self.assertEqual(guess(part([0.08, 0.05, 0.02]), material(base_color=[0.01, 0.01, 0.01]), 1, 0.3)[0], 'Plastic Satin Black')
        self.assertEqual(guess(part([0.08, 0.05, 0.02]), material(base_color=[0.7, 0.1, 0.1]), 1, 0.3)[0], 'Plastic Semi-Gloss Grey')
        # One big shape used four times is not hardware.
        self.assertNotEqual(guess(part([0.1, 0.1, 0.2]), material(base_color=[0.5, 0.5, 0.5]), 4, 0.3)[0], 'Stainless Brushed')

    def test_every_built_in_rule_names_a_library_preset(self):
        library = HERE.parent / 'assets' / 'library' / 'standard'
        names = {json.loads(f.read_text(encoding='utf-8'))['name'] for f in library.glob('*.json')}
        for words, material in self.module.BUILTIN:
            self.assertIn(material, names, f'{words} name {material}, which the library does not have')
        for material in (self.module.MOULDED, self.module.MOULDED_BLACK, 'Glass Clear', 'Steel Turned', 'Stainless Brushed'):
            self.assertIn(material, names, f'a guess names {material}, which the library does not have')

    def test_a_models_own_colour_is_told_from_an_importers_grey(self):
        module = self.module
        colour = module.model_colour
        definition = lambda c: type('M', (), {'definition': {'base_color': c}})()
        self.assertIsNone(colour(definition([0.5, 0.5, 0.5])), 'a fallback grey kept')
        self.assertEqual(colour(definition([0.8, 0.05, 0.05])), [0.8, 0.05, 0.05])
        self.assertEqual(colour(definition([0.02, 0.02, 0.02])), [0.02, 0.02, 0.02], 'a black part lost its black')
        self.assertEqual(module.colour_name([0.8, 0.05, 0.05]), 'red')
        self.assertEqual(module.colour_name([0.05, 0.1, 0.8]), 'blue')
        recoloured = module.recoloured({'base_color': [0.8, 0.8, 0.8], 'graph': {'nodes': [
            {'literals': [{'socket': 'input:1', 'value': {'color': [0.8, 0.8, 0.8]}}]}]}}, [0.1, 0.2, 0.3])
        self.assertEqual(recoloured['base_color'], [0.1, 0.2, 0.3])
        self.assertEqual(recoloured['graph']['nodes'][0]['literals'][0]['value']['color'], [0.1, 0.2, 0.3])


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


STUDIO_MANIFEST = json.loads((EXAMPLES / 'studio-lighting' / 'plugin.json').read_text(encoding='utf-8'))
STUDIO_NAMES = next(f for p in STUDIO_MANIFEST['panels'] for f in p['fields'] if f['id'] == 'preset')['options']


@unittest.skipIf(MISSING, MISSING)
class StudioLightingTests(unittest.TestCase):
    """The Studio Lighting example: every studio goes on, its settings are
    kept and shown again, the person's own environment can come back, and
    Back to my look puts the look back exactly."""

    def run_command(self, running, command, **values):
        running.send({'type': 'command', 'command': command, 'values': values})
        return running.expect(lambda m: m['type'] == 'done')

    def test_every_studio_applies_and_back_to_my_look_puts_it_all_back(self):
        self.assertGreaterEqual(len(STUDIO_NAMES), 15)
        with tequila.open(MODEL) as fs:
            mine = look(fs)
            running = Running(EXAMPLES / 'studio-lighting', fs)
            try:
                for name in STUDIO_NAMES:
                    heard = self.run_command(running, 'apply', preset=name)
                    self.assertIn({'type': 'status', 'text': f'Studio: {name}'}, heard)
                    pins = fs.context()['document']['environment'].get('pins', [])
                    self.assertTrue(pins and all(p['name'].startswith('Studio: ') for p in pins), name)
                self.run_command(running, 'restore')
                self.assertEqual(look(fs), mine, f'{len(STUDIO_NAMES)} studios later, the look was not put back')
                heard = self.run_command(running, 'restore')
                self.assertTrue(any('already yours' in (m.get('text') or '') for m in heard), heard)
            finally:
                running.close()

    def test_settings_are_kept_shown_and_reset(self):
        with tequila.open(MODEL) as fs:
            running = Running(EXAMPLES / 'studio-lighting', fs)
            try:
                self.run_command(running, 'apply', preset='Spotlight', turn=30)
                self.run_command(running, 'apply', key=250, mirror=True)
                running.send({'type': 'event', 'event': 'panel_opened', 'data': {'panel': 'lights'}})
                panel = [m for m in running.expect(lambda m: m['type'] == 'panel') if m['type'] == 'panel'][-1]
                running.expect(lambda m: m['type'] == 'done')
                self.assertEqual((panel['values']['key'], panel['values']['mirror']), (250, True))
                heard = self.run_command(running, 'reset_lights')
                reset = next(m for m in heard if m['type'] == 'panel')
                self.assertEqual((reset['values']['key'], reset['values']['mirror']), (100, False))
            finally:
                running.close()

    def test_my_environment_brings_the_persons_own_image_back(self):
        with tequila.open(MODEL) as fs:
            own = fs.context()['document']['environment'].get('image')
            running = Running(EXAMPLES / 'studio-lighting', fs)
            try:
                self.run_command(running, 'apply', preset='Soft Light Tent')
                self.assertNotEqual(fs.context()['document']['environment'].get('image'), own)
                self.run_command(running, 'apply', backdrop='My environment (HDRI)')
                environment = fs.context()['document']['environment']
                self.assertEqual(environment.get('image'), own)
                self.assertFalse(any(p['name'] in ('Studio: Room', 'Studio: Backdrop') for p in environment['pins']))
            finally:
                running.close()

    def test_a_studio_is_one_undo_step(self):
        with tequila.open(MODEL) as fs:
            mine = look(fs)
            running = Running(EXAMPLES / 'studio-lighting', fs)
            try:
                self.run_command(running, 'apply', preset='Chiaroscuro')
                self.assertNotEqual(look(fs), mine)
                fs.undo()
                self.assertEqual(look(fs), mine)
            finally:
                running.close()


if __name__ == '__main__':
    unittest.main()
