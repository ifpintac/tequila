"""The sample scenes that ship with Tequila (assets/samples): the
headphones and the perfume bottle, dressed in the library's materials, and
the palma P8 handheld in its own; all lit low-key, as the "After dark"
stills were. Each is saved as a project with its preview inside, and
rendered once at full size to look at.

    blender -b --factory-startup --python headphones.py -- MODELS/headphones.glb
    blender -b --factory-startup --python bottle.py -- MODELS/bottle.glb
    copy models/palma-p8.glb into MODELS
    python make_samples.py MODELS OUT [samples] [name,name...]

The command-line renderer is TEQUILA_CLI, or this checkout's release build;
Cycles is where TEQUILA_CYCLES says. The headphones and the palma render in
Cycles, the bottle - glass with a liquid inside - in Tequila's own renderer.
"""
import json
import os
import shutil
import sys
import zipfile
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]
sys.path.insert(0, str(REPO / 'python'))
sys.path.insert(0, str(HERE))
from studio import Studio, around, lcd, light, orbit, seamless, session  # noqa: E402
from wear import wear_graph  # noqa: E402

EXE = '.exe' if os.name == 'nt' else ''
CLI = os.environ.get('TEQUILA_CLI') or str(REPO / 'target' / 'release' / f'tequila{EXE}')


# The stills' backdrops measured a fifth as bright as these colours show
# today, which renders a gradient's colours as given: scaled to look as they
# did.
DIM = 0.22


def black(view, glow, spread):
    """A near-black seamless with a faint glow straight behind the product."""
    glow = tuple(v * DIM for v in glow)
    return seamless(view, 1.0, [(0.0, glow), (spread, tuple(v * 0.25 for v in glow)), (1.0, (0.0, 0.0, 0.0))])


def headphones(c, s, models):
    c.import_model(str(models / 'headphones.glb'))
    m = s.materials()
    nodes = {n['name']: n['id'] for n in c.query('nodes')['nodes']}

    def kind(name, type_, **params):
        c.apply('material.set_type', material=m[name], type=type_)
        for k, v in params.items():
            c.apply('material.set_parameter', material=m[name], parameter=k, value=v)

    def library(preset, *names):
        c.library.apply(preset, *[nodes[n] for n in names if n in nodes])

    def tint(preset, colour):
        for mat in c.query('materials')['materials']:
            if mat['name'] == preset:
                c.apply('material.set_parameter', material=mat['id'], parameter='base_color', value=colour)

    # The midnight colourway: black soft-touch, black leather.
    shells = [f'{p} {side}' for p in ('Shell', 'Knuckle') for side in 'LR'] + ['Headband', 'Buttons']
    library('Soft Touch Black', *shells)
    library('Plastic Satin Black', 'Back L', 'Back R')
    library('Plastic Leather Grain', 'Cushion L', 'Cushion R', 'Pad')
    tint('Plastic Leather Grain', [0.018, 0.018, 0.02])
    library('Stainless Hairline', 'Slider L', 'Slider R')
    library('Felt Grey', 'Cloth L', 'Cloth R')
    tint('Felt Grey', [0.025, 0.025, 0.027])
    kind('Thread', 'plastic', base_color=[0.03, 0.03, 0.032], specular_roughness=0.6)
    # Printed tone on tone: the same colour, gloss on the matte face.
    kind('Print', 'plastic', base_color=[0.02, 0.02, 0.022], specular_roughness=0.12)
    kind('Marks', 'plastic', base_color=[0.02, 0.02, 0.02], specular_roughness=0.5)
    kind('Port', 'plastic', base_color=[0.005, 0.005, 0.005], specular_roughness=0.6)
    kind('LED', 'plastic', base_color=[0.7, 0.7, 0.7], specular_roughness=0.2)
    c.apply('lighting.set_preset', preset='Product')

    target = [0.0, 0.094, 0.0]
    eye, view = orbit(target, 30, 6, 0.7)
    pins = [black(view, (0.022, 0.022, 0.025), 0.4),
            # Black on black, drawn only by its edges.
            light('Rim left', around(-140, 15), (0.08, 1.2), 40.0, softness=0.3),
            light('Rim right', around(145, 20), (0.08, 1.2), 34.0, softness=0.3, color=(0.85, 0.9, 1.0)),
            light('Top rim', around(180, 60), (0.6, 0.12), 14.0, softness=0.4),
            light('Whisper', around(-30, 30), (0.5, 0.4), 0.5, softness=0.9)]
    return dict(engine='cycles-gpu', target=target, eye=eye, focal=58, pins=pins, reflections=0.35, exposure=0.0)


def bottle(c, s, models):
    c.import_model(str(models / 'bottle.glb'))
    m = s.materials()
    gold = {'n': [0.15, 0.30, 1.0], 'k': [4.32, 2.746, 2.555]}

    def set_(name, parameter, value):
        c.apply('material.set_parameter', material=m[name], parameter=parameter, value=value)

    c.apply('material.set_type', material=m['Glass'], type='glass_solid')
    # A type keeps what carries over, the model's roughness with it: clear,
    # polished glass and a still liquid are smooth.
    set_('Glass', 'specular_roughness', 0.0)
    set_('Glass', 'specular_ior', 1.52)
    set_('Glass', 'medium_priority', 2.0)
    # Extra-clear flint: the faintest green, seen only through the thick base.
    set_('Glass', 'transmission_color', [0.94, 0.985, 0.97])
    set_('Glass', 'transmission_depth', 0.25)
    c.apply('material.set_type', material=m['Liquid'], type='liquid')
    set_('Liquid', 'specular_roughness', 0.0)
    set_('Liquid', 'specular_ior', 1.36)
    set_('Liquid', 'medium_priority', 1.0)
    set_('Liquid', 'transmission_color', [0.86, 0.47, 0.13])
    set_('Liquid', 'transmission_depth', 0.035)
    for part, rough in (('Collar', 0.07), ('Actuator', 0.09), ('Cap', 0.06)):
        c.apply('material.set_type', material=m[part], type='metal')
        set_(part, 'metal_ior', gold)
        set_(part, 'specular_roughness', rough)
    c.apply('material.set_type', material=m['Print'], type='metal')
    set_('Print', 'metal_ior', gold)
    set_('Print', 'specular_roughness', 0.32)
    c.apply('material.set_type', material=m['Dip tube'], type='plastic_transparent')
    set_('Dip tube', 'specular_ior', 1.5)
    set_('Dip tube', 'specular_roughness', 0.12)
    c.apply('lighting.set_preset', preset='Product')

    target = [0.0, 0.052, 0.0]
    eye, view = orbit(target, 18, 4, 0.44)
    amber = (1.0, 0.62, 0.25)
    pins = [black(view, (0.06, 0.03, 0.012), 0.25),
            # The glow, hard behind: the parfum lights up from inside.
            light('Glow', (-view[0], -view[1] + 0.03, -view[2]), (0.09, 0.15), 9.0, softness=1.0, background=True,
                  color=amber, shape='ellipse'),
            light('Rim left', (-0.95, 0.12, -0.45), (0.1, 1.2), 45.0, softness=0.3),
            light('Rim right', (0.95, 0.12, -0.4), (0.1, 1.2), 38.0, softness=0.3, color=amber),
            light('Cap', (-0.3, 0.9, 0.35), (0.25, 0.2), 10.0, softness=0.5, color=(1.0, 0.93, 0.82)),
            light('Edge', (0.8, 0.25, 0.55), (0.06, 0.9), 14.0, softness=0.3)]
    # Glass with liquid inside needs Tequila's own renderer.
    return dict(engine='tequila', target=target, eye=eye, focal=58, pins=pins, reflections=0.55, exposure=-0.15)


def palma(c, s, models):
    """The palma P8, a handheld designed for Tequila (models/palma-p8.glb,
    which comes with its own materials): its clear lime shell made solid,
    its screen made an LCD, lit low-key with the shell glowing at its edges."""
    c.import_model(str(models / 'palma-p8.glb'))
    mats = {m['name']: m for m in c.query('materials')['materials']}

    def set_(name, **values):
        for k, v in values.items():
            c.apply('material.set_parameter', material=mats[name]['id'], parameter=k, value=v)

    # The model's shell is thin-walled: seen through its front and its back,
    # four surfaces tinted it bottle green. Solid, it is tinted by how much
    # plastic the light crosses, as moulded polycarbonate is, with a breath of
    # haze.
    lime = [0.76, 0.96, 0.04]
    for name, rough in (('TQ_Shell_ClearIce', 0.035), ('TQ_Shell_ClearIce_Inner', 0.02)):
        set_(name, geometry_thin_walled=False, transmission_weight=1.0, transmission_color=lime,
             transmission_depth=0.005, transmission_scatter=[0.07, 0.09, 0.02], base_color=lime,
             specular_ior=1.585, specular_roughness=rough, medium_priority=1.0)
    # The screen's 80 by 60 pixel art as an LCD shows it: hard squares, and
    # the dark gaps between them.
    screen = models / 'palma-p8 screen.png'
    lcd(mats['TQ_Screen']['textures'][0]['path'], screen)
    for param in ('base_color', 'emission_color'):
        c.apply('material.set_texture', material=mats['TQ_Screen']['id'], param=param, path=str(screen),
                role='color', color_space='srgb')
    set_('TQ_Screen', emission_luminance=1.6, specular_roughness=0.25)
    set_('TQ_Graphite', base_color=[0.028, 0.029, 0.033], specular_roughness=0.36)
    set_('TQ_Graphite_Emboss', base_color=[0.05, 0.052, 0.058], specular_roughness=0.42)
    # A power LED is a pinprick: as it came, it lit the floor green.
    set_('TQ_LED_Green', emission_luminance=mats['TQ_LED_Green']['definition']['emission_luminance'] * 0.08)
    # Very faint handling marks, smudges, prints and dust, on the shell and
    # the screen's glass: a handheld someone has picked up, not a dirty one.
    for name, marks in (('TQ_Shell_ClearIce', dict(base=0.035, smudge=0.14, prints=0.2, dust=0.9)),
                        ('TQ_Lens_Smoke', dict(base=0.02, smudge=0.1, prints=0.28, dust=0.7, print_cells=30.0, smudge_scale=24.0))):
        # Its values as set above, with the marks as its graph.
        definition = dict(next(m for m in c.query('materials')['materials'] if m['name'] == name)['definition'])
        definition['graph'] = wear_graph(**marks)
        c.apply('material.set_graph', material=mats[name]['id'], definition=definition)
    c.apply('lighting.set_preset', preset='Product')

    parts = [n for n in c.query('nodes')['nodes'] if n.get('kind') == 'part']
    target = [(min(p['bounds']['min'][i] for p in parts) + max(p['bounds']['max'][i] for p in parts)) / 2
              for i in range(3)]
    eye, view = orbit(target, -24, 8, 0.8)
    pins = [seamless(view, 1.0, [(0.0, (0.05, 0.055, 0.06)), (0.09, (0.012, 0.013, 0.015)), (1.0, (0.0, 0.0, 0.0))]),
            light('Rim left', around(-140, 18), (0.08, 1.2), 40.0, softness=0.3),
            light('Rim right', around(145, 20), (0.08, 1.2), 34.0, softness=0.3, color=(0.85, 0.95, 1.0)),
            # From behind: the clear shell carries it round and glows.
            light('Glow', around(180, 10), (0.5, 0.35), 1.5, softness=1.0, color=(0.85, 1.0, 0.6)),
            light('Key', around(-30, 45), (0.5, 0.4), 2.0, softness=0.8),
            light('Top', (0.0, 1.0, 0.0), (0.5, 0.4), 1.0, softness=0.9)]
    return dict(engine='cycles-gpu', target=target, eye=eye, focal=80, pins=pins, reflections=0.2, exposure=0.0,
                textures='palma P8.textures')


def ship_textures(project, folder):
    """A project saves its textures where they were found. A shipped sample
    keeps them in a folder beside it, named relative to it, which Tequila
    resolves against wherever the project is opened from."""
    target = project.parent / folder
    if target.exists():
        shutil.rmtree(target)
    target.mkdir()
    with zipfile.ZipFile(project) as archive:
        entries = [(info, archive.read(info.filename)) for info in archive.infolist()]
    scene = json.loads(dict((info.filename, data) for info, data in entries)['scene.json'])
    for texture in scene.get('textures', []):
        source = Path(texture['path'])
        shutil.copyfile(source, target / source.name)
        texture['path'] = f'{folder}/{source.name}'
    with zipfile.ZipFile(project, 'w') as archive:
        for info, data in entries:
            if info.filename == 'scene.json':
                data = (json.dumps(scene, indent=2) + '\n').encode('utf-8')
            archive.writestr(info, data)


def main():
    models, out = Path(sys.argv[1]).resolve(), Path(sys.argv[2]).resolve()
    samples = int(sys.argv[3]) if len(sys.argv) > 3 else 512
    only = set(sys.argv[4].split(',')) if len(sys.argv) > 4 else None
    out.mkdir(parents=True, exist_ok=True)
    for name, build, engine in (('Headphones', headphones, 'cycles'), ('Perfume bottle', bottle, 'tequila'),
                                ('palma P8', palma, 'cycles')):
        if only and name not in only:
            continue
        with session(CLI, engine) as c:
            s = Studio(c)
            s.clear()
            look = build(c, s, models)
            s.environment(look['pins'], brightness=1.0, reflections=look['reflections'], ground_size=3.0)
            s.grade(exposure=look['exposure'])
            s.opaque()
            c.look(frm=look['eye'], at=look['target'], focal_length=look['focal'])
            render = c.context()['document']['render']
            render.update({'engine': look['engine'], 'resolution': [1920, 1080], 'max_samples': samples, 'denoise': True})
            c.apply('render.set_settings', render=render)
            project, hero, preview = out / f'{name}.tequila', out / f'{name}.png', out / f'{name} preview.png'
            for path in (project, hero, preview):
                if path.exists():
                    path.unlink()
            # The picture the start screen shows.
            c.render(str(preview), width=512, height=288, samples=max(64, samples // 4))
            c.save(str(project))
            result = c.render(str(hero), width=1920, height=1080, samples=samples)
            print(f'{name}: {result["seconds"]:.0f} s', flush=True)
        # Saved from a script, a project has no picture of its own: the
        # preview goes in as the application puts one (docs/file-format.md).
        with zipfile.ZipFile(project, 'a', compression=zipfile.ZIP_STORED) as archive:
            archive.writestr('preview.png', preview.read_bytes())
        if look.get('textures'):
            ship_textures(project, look['textures'])
        print(f'{name}: {project.stat().st_size / 1e6:.1f} MB', flush=True)


if __name__ == '__main__':
    main()
