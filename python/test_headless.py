"""tequila.open(): a headless Tequila of the script's own, driven end to end.

Needs the command-line renderer built (`cargo build -p tequila-cli`, or a
distribution's tequila-cli); skipped when there is none.
"""
import tempfile
import unittest
from pathlib import Path

import tequila
from tequila import TequilaError

MODEL = Path(__file__).resolve().parents[1] / 'crates' / 'tequila-import' / 'tests' / 'data' / 'two_bodies.step'

try:
    tequila._find_cli()
    MISSING = None
except TequilaError as error:
    MISSING = str(error)


@unittest.skipIf(MISSING, MISSING)
class HeadlessTests(unittest.TestCase):
    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.out = Path(self.folder.name)

    def tearDown(self):
        self.folder.cleanup()

    def test_a_model_is_edited_undone_saved_and_reopened(self):
        with tequila.open(MODEL) as fs:
            self.assertTrue(fs.request('ping')['headless'])
            parts = fs.parts()
            self.assertGreaterEqual(len(parts), 2)
            first = parts[0]
            fs.commands.scene.rename(node=first.id, name='Lid')
            self.assertEqual([p.name for p in fs.find(name='lid')], ['Lid'])
            self.assertEqual(fs.undo()['undone'], 'scene.rename')
            self.assertEqual(fs.find(name='lid'), [])
            # Two edits, one step.
            with fs.batch() as b:
                b.commands.scene.rename(node=first.id, name='Lid')
                b.commands.scene.set_metadata(node=first.id, key='Finish', value='Gloss')
            self.assertEqual(fs.history()['undo']['commands'], 2)
            project = self.out / 'model.tequila'
            fs.save(project)
        with tequila.open(project) as fs:
            lid = fs.find(metadata={'Finish': 'Gloss'})
            self.assertEqual([p.name for p in lid], ['Lid'])

    def test_materials_are_set_by_attribute_and_assigned(self):
        with tequila.open(MODEL) as fs:
            materials = list(fs.materials)
            self.assertTrue(materials, 'the model brought no materials')
            paint = materials[0]
            paint.base_color = (0.72, 0.05, 0.06)
            again = fs.materials[paint.id]
            self.assertAlmostEqual(again.base_color[0], 0.72, places=4)
            part = fs.parts()[-1]
            paint.assign(part)
            self.assertEqual([p.material for p in fs.parts() if p.id == part.id], [paint.id])

    def test_a_library_preset_goes_on_a_part_as_one_undo_step(self):
        with tequila.open(MODEL) as fs:
            presets = fs.library.presets(category='Metal')
            self.assertTrue(any(p['name'] == 'Aluminium Spun' for p in presets))
            part = fs.parts()[0]
            fs.library.apply('aluminium spun', part)
            material = fs.materials[fs.find(name=part.name)[0].material]
            self.assertEqual(material.name, 'Aluminium Spun')
            fs.undo()
            self.assertNotEqual(fs.parts()[0].material, material.id)
            with self.assertRaises(TequilaError):
                fs.library.apply('No Such Preset', part)

    def test_it_renders_and_exports_without_a_window(self):
        with tequila.open(MODEL) as fs:
            fs.frame()
            fs.look(view='iso')
            image = self.out / 'shot.png'
            result = fs.render(image, width=96, height=64, samples=4)
            self.assertTrue(image.is_file() and image.stat().st_size > 0, result)
            glb = fs.export('glb', self.out / 'model.glb')
            self.assertGreater(glb['bytes'], 0)
            with self.assertRaises(TequilaError) as refused:
                fs.screenshot(self.out / 'window.png')
            self.assertIn('window', str(refused.exception))

    def test_a_plugin_request_reads_as_this_client_sends_it(self):
        # The client numbers every request with `id`; the plugin request
        # once used `id` for the plugin too, and every real one was refused
        # before it was read. Read now, it is refused only for needing the
        # window.
        with tequila.open() as fs:
            for fields in ({'action': 'list'}, {'action': 'enable', 'plugin': 'my.x'}):
                with self.assertRaises(TequilaError) as refused:
                    fs.request('plugin', **fields)
                self.assertIn('window', str(refused.exception))

    def test_a_project_is_opened_and_started_over(self):
        with tequila.open(MODEL) as fs:
            saved = self.out / 'first.tequila'
            fs.save(saved)
            fs.commands.scene.add_group(name='Unsaved')
            with self.assertRaises(TequilaError):
                fs.open_project(saved)
            opened = fs.open_project(saved, discard=True)
            self.assertTrue(opened['path'].endswith('first.tequila'))
            self.assertFalse(fs.find(name='Unsaved', kind=None))
            fs.new_project()
            self.assertEqual(fs.nodes(), [])

    def test_unsaved_work_is_not_quit_on_by_accident(self):
        fs = tequila.open()
        try:
            fs.commands.scene.add_group(name='Kept?')
            with self.assertRaises(TequilaError):
                fs.request('quit')
        finally:
            fs.close()
        self.assertIsNotNone(fs._process.poll(), 'the server outlived its session')


@unittest.skipIf(MISSING, MISSING)
class RecipeTests(unittest.TestCase):
    """A material of the script's own, from a recipe, end to end."""

    def test_a_recipe_is_tried_made_read_back_and_changed(self):
        with tempfile.TemporaryDirectory() as folder, tequila.open(MODEL) as fs:
            guide = fs.recipes.guide()
            self.assertGreaterEqual(len(guide['examples']), 10)
            terrazzo = next(e['recipe'] for e in guide['examples'] if e['recipe']['name'] == 'Terrazzo')
            picture = fs.recipes.preview(terrazzo, path=Path(folder) / 'terrazzo.png', size=96)
            self.assertGreater(Path(picture).stat().st_size, 1000)
            part = fs.parts()[0]
            made = fs.recipes.create(terrazzo, part)
            self.assertEqual(fs.parts()[0].material, made)
            recipe = fs.recipes.read(made)
            self.assertEqual(recipe['type'], 'plastic')
            fs.recipes.update(made, {'type': 'metal', 'metal_preset': 'Gold'})
            self.assertEqual(fs.materials[made].type.lower(), 'metal')
            # One undo takes the update back, another the material.
            fs.undo()
            self.assertEqual(fs.materials[made].type.lower(), 'plastic')
            with self.assertRaises(TequilaError) as refused:
                fs.recipes.create({'type': 'plastic', 'patterns': [{'param': 'base_colour', 'procedural': {'kind': 'noise'}}]}, part)
            self.assertIn('base_colour', str(refused.exception))


if __name__ == '__main__':
    unittest.main()

