"""Exercise a disposable app instance launched by the test runner."""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'python'))
from tequila import Client

discovery, pid, output = sys.argv[1:]
output = Path(output)
with Client(discovery, expected_pid=int(pid), timeout=30) as fs:
    api = fs.api()
    assert 'context' in api['queries']
    context = fs.context()
    assert all(k in context['document'] for k in ('environment', 'lighting', 'cameras', 'units'))
    material = context['materials']['materials'][0]
    key = material['id']
    initial = material['definition']['specular_roughness']
    fs.set_material_parameter(key, 'specular_roughness', 0.31)
    fs.set_material_parameter(key, 'specular_roughness', 0.67)
    def roughness():
        return next(m for m in fs.query('materials')['materials'] if m['id'] == key)['definition']['specular_roughness']
    assert abs(roughness() - 0.67) < 1e-5
    fs.undo()
    assert abs(roughness() - 0.31) < 1e-5, 'Script edits merged into one undo entry'
    fs.undo()
    assert abs(roughness() - initial) < 1e-5
    # Verify a camera edit survives subsequent frames and occupies one undo
    # entry, rather than being shadowed by an automatic orbit-rounding edit.
    camera_index = context['document']['active_camera']
    original_camera = context['document']['cameras'][camera_index]
    edited_camera = dict(original_camera)
    edited_camera.pop('basis', None)
    edited_camera['position'] = [x + d for x, d in zip(original_camera['position'], (0.5, 0.2, 0.3))]
    def at(position):
        return all(abs(a-b) < 1e-4 for a, b in zip(fs.query('camera')['position'], position))
    fs.apply('camera.set', index=camera_index, camera=edited_camera)
    assert at(edited_camera['position'])
    fs.undo()
    assert at(original_camera['position']), 'Scripted camera undo did not restore the viewport'
    fs.redo()
    assert at(edited_camera['position']), 'Scripted camera redo did not restore the viewport'
    fs.undo()
    image = fs.screenshot(output.with_suffix('.png'))
    fs.save(output.with_suffix('.tequila'))
    output.with_suffix('.json').write_text(json.dumps(context, indent=2), encoding='utf-8')
    print(json.dumps(dict(ok=True, pid=fs.pid, parts=context['scene']['parts'],
                          material_types=len(api['material_types']), screenshot=image)))
