"""Helpers for building the sample scenes: a headless Tequila in folders of
its own, lights as environment pins, and a seamless backdrop."""
import math
import os
import tempfile
from pathlib import Path

import tequila


def session(cli, engine='tequila'):
    """A headless Tequila that never reads or writes the person's own
    settings, library or journal. Cycles is found where TEQUILA_CYCLES says."""
    box = Path(tempfile.mkdtemp(prefix='tequila-samples-'))
    for sub in ('home', 'Roaming', 'Local'):
        (box / sub).mkdir()
    os.environ.update(HOME=str(box / 'home'), USERPROFILE=str(box / 'home'),
                      APPDATA=str(box / 'Roaming'), LOCALAPPDATA=str(box / 'Local'))
    args = ['--engine', engine] + (['--device', 'OPTIX'] if engine == 'cycles' else [])
    return tequila.open(cli=cli, timeout=120, args=args)


def light(name, direction, size, intensity, softness=0.3, background=False, color=(1, 1, 1), shape='rectangle', roll=0.0):
    return {'name': name, 'kind': 'light', 'shape': shape, 'direction': list(direction),
            'angular_size': list(size), 'roll': roll, 'softness': softness, 'intensity': intensity,
            'color': list(color), 'blend': 'add', 'lighting': True, 'show_in_background': background,
            'enabled': True}


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

    def grade(self, exposure=0.0, transform='PBR Neutral'):
        style = self.c.context()['document']['image_style']
        style.update({'exposure': exposure, 'display_transform': transform})
        self.c.apply('image.set_style', style=style)

    def opaque(self):
        render = self.c.context()['document']['render']
        render['alpha_mode'] = 'none'
        self.c.apply('render.set_settings', render=render)
