"""The M6 reference renders (docs/05, M6 exit criterion), end to end.

    python scripts/reference/render.py OUT_DIR [--quest PATH_TO_QUEST3_GLTF] [--samples N]

Needs a running Tequila with scripting on; drives it over the scripting
channel and uses nothing but Tequila's own features. Builds the models and
the label artwork with the two scripts beside this one, then for each
product: clears the scene, imports, sets materials, lighting and camera,
renders a PNG and saves the project beside it.

  bottle   a spirits bottle: solid glass, an amber liquid modelled into the
           glass (the medium stack decides the overlap), a paper label with a
           foil layer wrapped round it, a turned oak stopper; backlit.
  ring     a solitaire: 18k yellow gold band and prongs, a round brilliant;
           the Jewelry preset in a light tent.
  quest3   an electronic device, from a glTF of your own (not shipped).
"""

import argparse
import math
import os
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, os.path.join(HERE, '..', '..', 'python'))
from tequila import Client  # noqa: E402


def light(name, direction, size, intensity, softness=0.3, background=False, color=(1, 1, 1), shape='rectangle'):
    return {'name': name, 'kind': 'light', 'shape': shape, 'direction': list(direction),
            'angular_size': list(size), 'roll': 0.0, 'softness': softness, 'intensity': intensity,
            'color': list(color), 'blend': 'add', 'lighting': True, 'show_in_background': background,
            'enabled': True}


def card(name, direction, size, softness=0.2):
    """Black, covering what is under it: a flag, or the camera itself."""
    pin = light(name, direction, size, 1.0, softness)
    pin.update({'blend': 'over', 'color': [0, 0, 0]})
    return pin


def seamless(view, intensity, stops):
    """A backdrop for the camera only, brightest straight behind the product."""
    return {'name': 'Seamless', 'kind': 'gradient', 'radial': True, 'shape': 'ellipse',
            'direction': [-v for v in view], 'angular_size': [2 * math.pi, 2 * math.pi], 'roll': 0.0,
            'softness': 0.0, 'intensity': intensity, 'color': [1, 1, 1], 'blend': 'over',
            'lighting': False, 'show_in_background': True, 'enabled': True,
            'stops': [{'t': t, 'color': list(c)} for t, c in stops]}


def around(azimuth_deg, elevation_deg):
    a, e = math.radians(azimuth_deg), math.radians(elevation_deg)
    return (math.sin(a) * math.cos(e), math.sin(e), math.cos(a) * math.cos(e))


def orbit(target, azimuth_deg, elevation_deg, distance):
    view = around(azimuth_deg, elevation_deg)
    return [target[i] + distance * view[i] for i in range(3)], view


class Studio:
    def __init__(self, client):
        self.c = client

    def clear(self):
        roots = [n['id'] for n in self.c.query('nodes')['nodes'] if n.get('parent') is None]
        if roots:
            self.c.apply('scene.delete_nodes', nodes=roots)

    def materials(self):
        return {m['name']: m['id'] for m in self.c.query('materials')['materials'] if m['used_by']}

    def environment(self, pins, brightness=1.0, reflections=0.0, ground_size=0.0):
        self.c.apply('environment.set_pins', pins=pins)
        self.c.apply('environment.set_parameter', parameter='brightness', value=brightness)
        self.c.apply('environment.set_parameter', parameter='ground_reflections', value=reflections)
        self.c.apply('environment.set_parameter', parameter='ground_size', value=ground_size)

    def grade(self, exposure=0.0, saturation=1.0):
        style = self.c.context()['document']['image_style']
        style.update({'exposure': exposure, 'saturation': saturation})
        self.c.apply('image.set_style', style=style)

    def opaque(self):
        render = self.c.context()['document']['render']
        render['alpha_mode'] = 'none'
        self.c.apply('render.set_settings', render=render)

    def shoot(self, out_dir, name, width, height, samples):
        path = os.path.join(out_dir, name + '.png')
        if os.path.exists(path):
            os.remove(path)
        # The size goes into the project too, so the saved .tequila renders
        # the same framing from the CLI or the Render window.
        render = self.c.context()['document']['render']
        render.update({'resolution': [width, height], 'max_samples': samples})
        self.c.apply('render.set_settings', render=render)
        result = self.c.render(path, width=width, height=height, samples=samples)
        self.c.save(os.path.join(out_dir, name + '.tequila'))
        print(f'{name}: {result["seconds"]:.0f} s -> {path}')


def bottle(s, out_dir, samples):
    c = s.c
    s.clear()
    c.import_model(os.path.join(out_dir, 'bottle.obj'))
    m = s.materials()
    c.apply('material.set_type', material=m['Glass'], type='glass_solid')
    c.apply('material.set_type', material=m['Liquid'], type='liquid')
    c.apply('material.set_parameter', material=m['Liquid'], parameter='transmission_color', value=[0.80, 0.42, 0.10])
    c.apply('material.set_parameter', material=m['Liquid'], parameter='transmission_depth', value=0.04)
    c.apply('material.set_type', material=m['Wood'], type='plastic')
    c.apply('material.set_procedural', material=m['Wood'], param='base_color',
            procedural={'kind': 'wood_advanced', 'axis': 'y', 'frequency': 90.0, 'distortion': 0.3,
                        'grain': 0.3, 'ring_variation': 0.5, 'latewood': 0.35, 'contrast': 0.8,
                        'flecks': 0.2, 'knots': 0.0})
    c.apply('material.set_gradient', material=m['Wood'], param='base_color',
            gradient=[{'at': 0.0, 'color': [0.20, 0.105, 0.045]}, {'at': 1.0, 'color': [0.075, 0.035, 0.015]}])
    c.apply('material.set_parameter', material=m['Wood'], parameter='specular_roughness', value=0.35)

    # The label and its foil, wrapped round the body: 72 by 60 mm, 30 mm up.
    c.apply('material.add_decal', material=m['Glass'], name='Label', path=os.path.join(out_dir, 'label.png'))
    c.apply('material.add_decal', material=m['Glass'], name='Foil', path=os.path.join(out_dir, 'foil.png'))
    radius, width, height, bottom = 0.034, 0.072, 0.060, 0.030
    turns = width / radius / (2 * math.pi)
    # Scale must be positive and the flip comes after the offset: the image's
    # left at the bottle's left, its top at the label's top.
    mapping = {'projection': 'cylindrical', 'axis': 'y', 'blend_sharpness': 6, 'flip_u': True, 'flip_v': True,
               'offset': [-(0.75 + turns / 2) / turns, -(bottom + height) / height], 'pivot': [0, 0],
               'real_world_units': False, 'rotation': 0, 'scale': [1 / turns, 1 / height, 1], 'uv_set': 0,
               'wrap_u': 'clamp', 'wrap_v': 'clamp'}
    glass = [x for x in c.query('materials')['materials'] if x['id'] == m['Glass']][0]
    decals = glass['definition']['graph']['decals']
    for d in decals:
        d['mapping'] = dict(mapping)
        d['roughness'] = 0.22 if d['name'] == 'Foil' else 0.5
        d['metallic'] = d['name'] == 'Foil'
    c.apply('material.set_decals', material=m['Glass'], decals=decals)

    # Backlit: a warm glow behind for the liquid to be seen against, rims and
    # strips for the glass's edges and highlights, a key on the label. The
    # studio itself is taken down to a fifth so the backdrop falls to dark.
    warm = (1.0, 0.86, 0.68)
    s.environment([
        light('Glow', (0.0, 0.12, -1.0), (0.42, 0.75), 7.0, softness=1.0, background=True, color=warm, shape='ellipse'),
        light('Rim left', (-1.0, 0.2, -0.35), (0.2, 1.2), 25.0),
        light('Rim right', (1.0, 0.2, -0.35), (0.2, 1.2), 25.0),
        light('Strip left', (-0.75, 0.15, 0.65), (0.3, 1.0), 60.0),
        light('Strip right', (0.8, 0.15, 0.6), (0.18, 1.0), 36.0),
        light('Key', (-0.6, 0.45, 1.0), (0.6, 0.6), 30.0, softness=0.6),
        light('Top', (0.15, 1.0, 0.1), (0.5, 0.5), 15.0, softness=0.5),
    ], brightness=0.2, reflections=0.35)
    c.apply('lighting.set_preset', preset='Product')
    s.grade()
    s.opaque()
    c.look(frm=[0.0, 0.105, 0.64], at=[0.0, 0.118, 0.0], focal_length=85)
    s.shoot(out_dir, 'bottle', 1200, 1500, samples)


def ring(s, out_dir, samples):
    c = s.c
    s.clear()
    c.import_model(os.path.join(out_dir, 'ring.obj'))
    m = s.materials()
    c.apply('material.set_type', material=m['Gold'], type='metal')
    # 18k yellow gold, paler than pure gold: n and k solved per channel from
    # reflectances of 0.97, 0.87 and 0.62.
    c.apply('material.set_parameter', material=m['Gold'], parameter='metal_ior',
            value={'n': [0.15, 0.30, 1.0], 'k': [4.32, 2.746, 2.555]})
    c.apply('material.set_parameter', material=m['Gold'], parameter='specular_roughness', value=0.06)
    c.apply('material.set_type', material=m['Diamond'], type='gem')
    c.apply('lighting.set_preset', preset='Jewelry')

    target = [0.0, 0.0135, 0.0]
    eye, view = orbit(target, 25, 18, 0.105)
    pins = [seamless(view, 2.3, [(0.0, (1.0, 0.985, 0.96)), (0.3, (0.82, 0.80, 0.78)), (1.0, (0.35, 0.35, 0.35))])]
    # A light tent: strips all round for the metal to mirror, black flags at
    # the sides to shape it, the camera dark in the stone's table, and small
    # hard lights above for the stone to break into sparks.
    pins += [card(f'Flag {i + 1}', around(a, 0), (0.5, 0.7)) for i, a in enumerate((90, -90))]
    pins.append(card('Camera card', view, (0.22, 0.22), softness=0.3))
    pins += [light(f'Strip {i + 1}', around(a, 25), (0.3, 1.2), 10.0) for i, a in enumerate((40, 140, 220, 320))]
    pins += [light(f'Spark {i + 1}', around(a, e), (0.035, 0.035), 250.0, softness=0.0)
             for i, (a, e) in enumerate(((20, 60), (100, 45), (170, 70), (250, 50), (300, 65), (60, 80)))]
    s.environment(pins, brightness=1.0, reflections=0.2, ground_size=2.0)
    s.grade(exposure=-1.0, saturation=0.9)
    s.opaque()
    c.look(frm=eye, at=target, focal_length=100)
    s.shoot(out_dir, 'ring', 1200, 1200, samples)


def quest3(s, out_dir, samples, path):
    c = s.c
    s.clear()
    c.import_model(path)
    target = [0.0, 0.17, 0.0]
    eye, view = orbit(target, 30, 12, 1.9)
    s.environment([
        seamless(view, 1.0, [(0.0, (0.95, 0.95, 0.96)), (0.35, (0.62, 0.63, 0.66)), (1.0, (0.25, 0.25, 0.27))]),
        light('Key', (-0.6, 0.7, 0.8), (0.8, 0.6), 3.0, softness=0.6),
    ])
    c.apply('lighting.set_preset', preset='Product')
    s.grade()
    s.opaque()
    c.look(frm=eye, at=target, focal_length=85)
    s.shoot(out_dir, 'quest3', 1600, 1200, samples)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('out_dir')
    parser.add_argument('--quest', help='a glTF of the Quest 3, for the electronic-device render')
    parser.add_argument('--samples', type=int, default=512)
    args = parser.parse_args()
    out_dir = os.path.abspath(args.out_dir)
    os.makedirs(out_dir, exist_ok=True)
    for script in ('models.py', 'label.py'):
        subprocess.run([sys.executable, os.path.join(HERE, script), out_dir], check=True)

    s = Studio(Client(timeout=3600))
    bottle(s, out_dir, args.samples)
    ring(s, out_dir, args.samples)
    if args.quest:
        quest3(s, out_dir, args.samples, args.quest)


if __name__ == '__main__':
    main()
