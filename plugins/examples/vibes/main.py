"""Vibes: one click turns the scene into a mood.

A vibe is lights, a backdrop, a floor and a colour grade - never the
materials or the camera. The lights are pins painted on the environment, so
they light the product and show in its reflections the way a studio's
softboxes do; they are placed around the camera as it stands when the vibe
is applied, so the rim lights land behind the product from whichever side it
is seen. The backdrop faces the camera too, so a gradient runs from the
bottom of the picture to the top however the camera is tilted.

Everything a vibe changes goes in one batch, so one Ctrl+Z takes it off. The
look from before the first vibe is kept, and Back to my look puts it back
however many vibes came after; a look changed by hand in between counts as
the person's own, and is the one kept from then on.
"""
import json
import math
import random

import tequila
import tequila_plugin as plugin

TAU = math.tau
NEUTRAL_KELVIN = 6500.0


# ---- Where things go ------------------------------------------------------

class Around:
    """Directions in the world, given as the camera sees them, and how much
    of the world the picture takes in."""

    def __init__(self, camera, aspect=1.5):
        position = camera.get('position') or [0.0, 0.0, 0.0]
        target = camera.get('target') or [0.0, 0.0, 0.0]
        look = [t - p for t, p in zip(target, position)]
        flat = math.hypot(look[0], look[2])
        # No camera to speak of (an empty scene), or one looking straight
        # down: the default view's direction, down -Z.
        self.forward = (look[0] / flat, look[2] / flat) if flat > 1e-6 else (0.0, -1.0)
        length = math.sqrt(sum(c * c for c in look))
        self.view = [c / length for c in look] if length > 1e-6 and flat > 1e-6 else [self.forward[0], 0.0, self.forward[1]]
        # Half the picture's height as an angle: a 36 mm-wide sensor, as
        # Tequila's cameras have, at the lens's focal length.
        focal = max(float(camera.get('focal_length') or 50.0), 1.0)
        self.half_height = math.atan(18.0 / (focal * max(aspect, 0.1)))

    def aim(self, back=0.0, right=0.0, up=0.0):
        """`back` away from the camera, past the product; `right` to the
        camera's right; `up` up. Mixed and made a unit direction."""
        fx, fz = self.forward
        rx, rz = -fz, fx
        v = [fx * back + rx * right, up, fz * back + rz * right]
        n = math.sqrt(sum(c * c for c in v)) or 1.0
        return [c / n for c in v]

    def in_view(self, x, y):
        """A direction in the picture: `x` across and `y` up from its
        middle, in half picture-heights (1 is the top edge)."""
        fx, fz = self.forward
        right = [-fz, 0.0, fx]
        v = self.view
        up = [right[1] * v[2] - right[2] * v[1], right[2] * v[0] - right[0] * v[2], right[0] * v[1] - right[1] * v[0]]
        reach = math.tan(self.half_height)
        d = [v[i] + right[i] * x * reach + up[i] * y * reach for i in range(3)]
        n = math.sqrt(sum(c * c for c in d)) or 1.0
        return [c / n for c in d]


# ---- What a vibe is made of ----------------------------------------------

def _stops(stops):
    return [{'t': t, 'color': tequila.srgb(hex_)} for t, hex_ in stops]


def light(name, direction, size, colour, intensity, *, softness=0.4, shape='rectangle', roll=0.0):
    """A soft light on the environment: it lights the product and shows in
    its reflections, but not behind it."""
    return {
        'name': f'Vibe: {name}', 'kind': 'light', 'shape': shape, 'direction': direction,
        'angular_size': list(size), 'roll': roll, 'softness': softness, 'intensity': intensity,
        'color': tequila.srgb(colour), 'show_in_background': False, 'lighting': True,
        'blend': 'add', 'enabled': True,
    }


def room(stops, intensity=1.0):
    """The whole sphere, floor to ceiling, over whatever environment was
    there: the light that comes from everywhere no lamp is, and what the
    reflections show around the lamps."""
    return {
        'name': 'Vibe: Room', 'kind': 'gradient', 'stops': _stops(stops), 'radial': True,
        'shape': 'rectangle', 'direction': [0.0, -1.0, 0.0], 'angular_size': [TAU, TAU], 'roll': 0.0,
        'softness': 0.0, 'intensity': intensity, 'color': [1.0, 1.0, 1.0],
        'show_in_background': True, 'lighting': True, 'blend': 'over', 'enabled': True,
    }


def backdrop(around, stops, *, spot=None, intensity=1.0):
    """What the camera sees behind the product, lighting nothing: a
    gradient from the bottom of the picture (t 0.1) to the top (0.9) at the
    lens it is applied with. Or, with `spot`, a pool of light behind the
    product, from its middle (t 0) to as far out as `spot` picture-heights
    (t 1)."""
    radial = spot is not None
    tall = min(3.0, 2.0 * around.half_height / 0.8)
    return {
        'name': 'Vibe: Backdrop', 'kind': 'gradient', 'stops': _stops(stops), 'radial': radial,
        'shape': 'rectangle', 'direction': around.view,
        'angular_size': [min(3.0, 2.0 * around.half_height * spot)] * 2 if radial else [3.1, tall], 'roll': 0.0,
        'softness': 0.0, 'intensity': intensity, 'color': [1.0, 1.0, 1.0],
        'show_in_background': True, 'lighting': False, 'blend': 'over', 'enabled': True,
    }


def stars(around, count, seed):
    """Distant stars, out of focus: soft round dots behind the product, seen
    and not lighting, the same each time for the same seed. Points would be
    sharper, but the environment is 1024 texels around, and a long lens
    magnifies a one-texel star into a square; five texels or more stay
    round at any lens."""
    rng = random.Random(seed)
    out = []
    for i in range(count):
        size = max(0.03, around.half_height * rng.uniform(0.18, 0.4))
        tone = rng.choice(['#FFFFFF', '#CFE0FF', '#CFE0FF', '#FFE8C8', '#E0C8FF'])
        out.append({
            'name': f'Vibe: Star {i + 1}', 'kind': 'light', 'shape': 'ellipse',
            'direction': around.in_view(rng.uniform(-2.2, 2.2), rng.uniform(-1.2, 1.2)),
            'angular_size': [size, size], 'roll': 0.0, 'softness': 1.0,
            'intensity': rng.uniform(0.15, 0.6),
            'color': tequila.srgb(tone), 'show_in_background': True, 'lighting': False,
            'blend': 'add', 'enabled': True,
        })
    return out


def glow(around, name, direction, size, colour, intensity):
    """A haze seen behind the product - a nebula, a horizon - lighting
    nothing."""
    pin = light(name, direction, size, colour, intensity, softness=1.0, shape='ellipse')
    pin.update(show_in_background=True, lighting=False)
    return pin


def floor(*, shadows=True, shadow='#000000', reflections=0.0, blur=0.0, length=1.0, fresnel=False, enabled=True):
    return {
        'ground_enabled': enabled, 'ground_shadows': shadows, 'ground_shadow_color': tequila.srgb(shadow),
        'ground_occlusion': False, 'ground_reflections': reflections,
        'ground_reflection_blur': blur, 'ground_reflection_length': length, 'ground_reflection_fresnel': fresnel,
    }


# ---- The vibes -----------------------------------------------------------
#
# Each returns its pins, floor, background, sky and grade for a camera. The
# grade is relative to neutral, so Drama can scale it.

def cyberpunk(a):
    return dict(
        tagline='Neon rain, chrome dreams.',
        pins=[
            room([(0.0, '#0A0614'), (0.5, '#2A1240'), (1.0, '#120A24')]),
            backdrop(a, [(0.0, '#06030C'), (0.15, '#0C0618'), (0.33, '#4A1260'), (0.45, '#24104A'), (0.7, '#0C0820'), (1.0, '#04030A')]),
            light('Magenta neon', a.aim(back=0.8, right=-0.9, up=0.25), (0.1, 1.4), '#FF2BD6', 9.0, softness=0.25),
            light('Cyan neon', a.aim(back=0.8, right=0.9, up=0.25), (0.1, 1.4), '#00E5FF', 9.0, softness=0.25),
            light('Violet body', a.aim(back=-0.7, right=0.15, up=0.6), (1.6, 0.9), '#8C7CFF', 3.0, softness=0.8),
        ],
        floor=floor(reflections=0.55, blur=0.04),
        background='Environment',
        grade=dict(exposure=0.3, contrast=1.1, saturation=1.25, highlights=-0.2, blacks=-0.04,
                   temperature=6100.0, vignette=0.4, bloom=0.4, bloom_radius=0.05,
                   chromatic_aberration=0.003, sharpening=0.1),
    )


def golden_hour(a):
    sun = a.aim(back=0.85, right=-0.55, up=0.1)
    return dict(
        tagline='The fifteen minutes before sunset.',
        sky={'ground_albedo': 0.3, 'intensity': 0.6, 'sun': sun, 'sun_disc': True, 'sun_intensity': 4.0, 'turbidity': 4.0},
        pins=[
            backdrop(a, [(0.0, '#8E4A2A'), (0.2, '#B8673C'), (0.42, '#E39A62'), (0.55, '#E8B486'), (0.75, '#B7A6B4'), (1.0, '#7D88AE')], intensity=0.85),
            light('Sunset glow', a.aim(back=1.0, right=-0.3, up=0.12), (1.6, 0.35), '#FF9A50', 1.2, softness=0.9),
            light('Warm fill', a.aim(back=-0.8, right=0.6, up=0.35), (1.8, 1.1), '#FFD6A8', 4.0, softness=0.9),
        ],
        floor=floor(shadow='#3A1F12', reflections=0.12, blur=0.25, fresnel=True),
        background='Environment',
        grade=dict(exposure=0.5, contrast=1.05, saturation=1.1, highlights=-0.3, shadows=0.15,
                   temperature=7000.0, vignette=0.3, bloom=0.25, bloom_radius=0.06),
    )


def keynote(a):
    return dict(
        tagline='One more thing.',
        pins=[
            room([(0.0, '#000000'), (1.0, '#000000')]),
            light('Top softbox', a.aim(back=-0.15, up=1.0), (1.6, 1.0), '#FFFFFF', 3.5, softness=0.7),
            light('Front card', a.aim(back=-1.0, up=0.15), (1.8, 0.7), '#FFFFFF', 2.0, softness=1.0),
            light('Left rim', a.aim(back=1.0, right=-0.9, up=0.15), (0.08, 1.1), '#FFFFFF', 5.0, softness=0.3),
            light('Right rim', a.aim(back=1.0, right=0.9, up=0.15), (0.08, 1.1), '#FFFFFF', 5.0, softness=0.3),
        ],
        floor=floor(reflections=0.45, blur=0.02, length=0.35),
        background='Color',
        background_color='#000000',
        grade=dict(exposure=0.0, contrast=1.1, highlights=-0.15, blacks=-0.05, saturation=1.05,
                   vignette=0.1, bloom=0.08, sharpening=0.15),
    )


def vaporwave(a):
    return dict(
        tagline='A E S T H E T I C',
        pins=[
            room([(0.0, '#FF71CE'), (0.45, '#B967FF'), (1.0, '#01CDFE')], intensity=0.5),
            backdrop(a, [(0.0, '#FF5FC8'), (0.25, '#FF71CE'), (0.5, '#B967FF'), (0.78, '#01CDFE'), (1.0, '#05E8C8')]),
            light('Pink key', a.aim(back=-0.7, right=-0.6, up=0.5), (1.0, 0.9), '#FFC2EC', 1.6, softness=0.6),
            light('Teal rim', a.aim(back=0.9, right=0.8, up=0.3), (0.12, 1.2), '#01CDFE', 6.0, softness=0.3),
        ],
        floor=floor(shadow='#5B2A86', reflections=0.4, blur=0.15),
        background='Environment',
        grade=dict(exposure=0.1, contrast=0.95, saturation=1.2, blacks=0.08, highlights=-0.15,
                   tint=0.12, vignette=0.15, bloom=0.2, chromatic_aberration=0.002),
    )


def noir(a):
    return dict(
        tagline='It was a dark and stormy render.',
        pins=[
            room([(0.0, '#080808'), (1.0, '#141414')]),
            backdrop(a, [(0.0, '#5E5E5E'), (0.5, '#262626'), (1.0, '#060606')], spot=3.2),
            light('Hard key', a.aim(back=-0.5, right=-0.8, up=0.9), (0.1, 0.1), '#FFFFFF', 220.0, softness=0.1, shape='ellipse'),
            light('Soft fill', a.aim(back=-0.8, right=0.7, up=0.3), (1.4, 1.1), '#FFFFFF', 1.0, softness=0.9),
            light('Rim', a.aim(back=1.0, right=0.8, up=0.3), (0.06, 1.0), '#FFFFFF', 3.0, softness=0.3),
        ],
        floor=floor(reflections=0.15, blur=0.3),
        background='Environment',
        grade=dict(exposure=0.0, saturation=0.0, contrast=1.3, highlights=-0.1, blacks=-0.1,
                   vignette=0.5, bloom=0.1, sharpening=0.2),
    )


def candy(a):
    return dict(
        tagline='Sweet, soft and very shareable.',
        pins=[
            room([(0.0, '#FFE3EF'), (0.5, '#FFF4F8'), (1.0, '#E3F3FF')], intensity=0.9),
            backdrop(a, [(0.0, '#FFC2D9'), (0.3, '#FFD6E8'), (0.7, '#D6ECFF'), (1.0, '#C2E0FF')]),
            light('Big softbox', a.aim(back=-0.4, up=1.0), (1.4, 1.0), '#FFFFFF', 2.5, softness=0.8),
            light('Pink rim', a.aim(back=0.9, right=-0.8, up=0.2), (0.2, 1.0), '#FF9CC8', 2.5, softness=0.5),
            light('Blue rim', a.aim(back=0.9, right=0.8, up=0.2), (0.2, 1.0), '#9CCBFF', 2.5, softness=0.5),
        ],
        floor=floor(shadow='#C27BA0', reflections=0.1, blur=0.4, fresnel=True),
        background='Environment',
        grade=dict(exposure=0.3, contrast=0.9, saturation=1.1, blacks=0.08, highlights=-0.2, bloom=0.15),
    )


def lava(a):
    return dict(
        tagline='Forged in the floor.',
        pins=[
            room([(0.0, '#FF6A00'), (0.3, '#6A1400'), (0.55, '#160300'), (1.0, '#000000')], intensity=0.8),
            backdrop(a, [(0.0, '#FF8A1A'), (0.15, '#FF5A00'), (0.35, '#B82400'), (0.55, '#3A0800'), (0.8, '#0A0100'), (1.0, '#000000')]),
            light('Ember rim', a.aim(back=0.9, right=0.7, up=-0.05), (0.15, 0.9), '#FF7A1A', 8.0, softness=0.4),
            light('Lava glow', a.aim(back=-0.8, right=-0.2, up=-0.25), (1.6, 0.8), '#FF6A1A', 3.5, softness=0.9),
            light('Cold top', a.aim(back=-0.3, right=-0.3, up=1.0), (0.6, 0.5), '#4060FF', 0.8, softness=0.6),
        ],
        floor=floor(shadow='#1A0300', reflections=0.4, blur=0.1),
        background='Environment',
        grade=dict(exposure=0.1, contrast=1.15, saturation=1.1, highlights=-0.25, temperature=7000.0,
                   vignette=0.45, bloom=0.5, bloom_radius=0.05),
    )


def deep_space(a):
    return dict(
        tagline='Floating, far from home.',
        pins=[
            room([(0.0, '#000005'), (0.5, '#060A22'), (1.0, '#010210')]),
            backdrop(a, [(0.0, '#000004'), (0.5, '#050A24'), (1.0, '#010210')]),
            glow(a, 'Nebula', a.in_view(-1.1, 0.5), (0.9, 0.6), '#7A2C9E', 0.25),
            glow(a, 'Nebula, far', a.in_view(1.3, -0.2), (1.1, 0.5), '#1F4FA3', 0.2),
            *stars(a, 28, seed=7),
            light('Starlight rim', a.aim(back=0.9, right=0.85, up=0.3), (0.1, 1.2), '#7FB2FF', 6.0, softness=0.3),
            light('Planet glow', a.aim(back=0.6, right=-0.9, up=-0.1), (1.0, 0.6), '#FF8A3D', 0.8, softness=0.9),
            light('Cool key', a.aim(back=-0.8, right=-0.4, up=0.5), (1.4, 1.0), '#B8CAFF', 1.5, softness=0.8),
        ],
        # No floor: it floats.
        floor=floor(enabled=False),
        background='Environment',
        grade=dict(exposure=0.15, contrast=1.15, saturation=1.1, blacks=-0.05, vignette=0.35,
                   bloom=0.35, chromatic_aberration=0.002),
    )


VIBES = {
    'Cyberpunk': cyberpunk,
    'Golden Hour': golden_hour,
    'Keynote': keynote,
    'Vaporwave': vaporwave,
    'Noir': noir,
    'Candy': candy,
    'Lava': lava,
    'Deep Space': deep_space,
}


# ---- Grading ---------------------------------------------------------------

NEUTRAL = dict(exposure=0.0, contrast=1.0, saturation=1.0, highlights=0.0, shadows=0.0, whites=0.0,
               blacks=0.0, temperature=NEUTRAL_KELVIN, tint=0.0, vignette=0.0, bloom=0.0,
               bloom_radius=0.03, sharpening=0.0, chromatic_aberration=0.0)


def graded(grade, drama, display):
    """The image style for a grade at `drama` (1 as designed, 0 none, 2
    double), keeping the person's display transform."""
    out = dict(NEUTRAL, **grade)
    for key in ('contrast', 'saturation'):
        out[key] = 1.0 + (out[key] - 1.0) * drama
    out['temperature'] = NEUTRAL_KELVIN + (out['temperature'] - NEUTRAL_KELVIN) * drama
    for key in ('highlights', 'shadows', 'whites', 'blacks', 'tint', 'vignette', 'bloom', 'sharpening', 'chromatic_aberration'):
        out[key] *= drama
    clamp = lambda v, lo, hi: max(lo, min(hi, v))
    out['contrast'] = clamp(out['contrast'], 0.2, 3.0)
    out['saturation'] = clamp(out['saturation'], 0.0, 3.0)
    out['temperature'] = clamp(out['temperature'], 2000.0, 15000.0)
    for key in ('highlights', 'shadows', 'whites', 'blacks', 'tint'):
        out[key] = clamp(out[key], -1.0, 1.0)
    for key in ('vignette', 'bloom', 'sharpening'):
        out[key] = clamp(out[key], 0.0, 1.0)
    out['type'] = 'Photographic'
    out['display_transform'] = display
    return out


# ---- Reading, applying and putting back a look ----------------------------

# Each environment parameter, and where the document keeps it.
PARAMETERS = [
    ('rotation', None, 'rotation'), ('brightness', None, 'brightness'), ('contrast', None, 'contrast'),
    ('size', None, 'size'), ('height', None, 'height'), ('image', None, 'image'),
    ('background', 'background', 'mode'), ('background_color', 'background', 'color'),
    ('backplate', 'background', 'image'),
    ('ground_enabled', 'ground', 'enabled'), ('ground_shadows', 'ground', 'shadows'),
    ('ground_shadow_color', 'ground', 'shadow_color'), ('ground_occlusion', 'ground', 'occlusion_shadows'),
    ('ground_reflections', 'ground', 'reflections'), ('ground_reflection_blur', 'ground', 'reflection_blur'),
    ('ground_reflection_length', 'ground', 'reflection_length'),
    ('ground_reflection_fresnel', 'ground', 'reflection_fresnel'), ('ground_flatten', 'ground', 'flatten'),
    ('ground_size', 'ground', 'size'),
]


def look_of(document):
    """The environment and image style, as a document has them."""
    return {'environment': document['environment'], 'image_style': document['image_style']}


def current_look(fs):
    return look_of(fs.context()['document'])


def built(name, fs, drama=1.0, document=None):
    """What applying `name` sends, for the camera as it is now."""
    document = document or fs.context()['document']
    width, height = (document.get('render') or {}).get('resolution') or (1920, 1080)
    chosen = VIBES[name](Around(fs.query('camera'), width / max(height, 1)))
    display = document['image_style'].get('display_transform', 'PBR Neutral')
    params = {
        'rotation': 0.0, 'brightness': 1.0, 'contrast': 1.0, 'size': 0.0,
        'background': chosen['background'],
        'background_color': tequila.srgb(chosen.get('background_color', '#000000')),
        **chosen['floor'],
    }
    return {
        'sky': chosen.get('sky'), 'params': params, 'pins': chosen['pins'],
        'style': graded(chosen['grade'], drama, display), 'tagline': chosen['tagline'],
    }


def send(fs, look):
    """A look's commands, as one undo step."""
    with fs.batch() as batch:
        environment = batch.commands.environment
        environment.set_sky(sky=look['sky'])
        for parameter, value in look['params'].items():
            environment.set_parameter(parameter=parameter, value=value)
        environment.set_pins(pins=look['pins'])
        batch.commands.image.set_style(style=look['style'])


def restore(fs, saved):
    """The look `saved` by `current_look`, put back as one undo step."""
    environment = saved['environment']
    params = {}
    for parameter, section, key in PARAMETERS:
        holder = environment.get(section, {}) if section else environment
        # The image and backplate are left out when there is none; anything
        # else missing is a setting this look predates, left as it is.
        if key in holder or key == 'image':
            params[parameter] = holder.get(key)
    send(fs, {'sky': environment.get('sky'), 'params': params, 'pins': environment.get('pins', []),
              'style': saved['image_style']})


def _same(a, b):
    return a is not None and b is not None and json.dumps(a, sort_keys=True) == json.dumps(b, sort_keys=True)


# ---- The plugin --------------------------------------------------------------

_saved = None     # the person's look, from before the first vibe
_applied = None   # the look as the last vibe left it
_last = None      # that vibe's name


def _drama(values):
    try:
        return max(0.0, min(2.0, float(values.get('drama', 100)) / 100.0))
    except (TypeError, ValueError):
        return 1.0


def vibe(fs, name, values):
    global _saved, _applied, _last
    document = fs.context()['document']
    before = look_of(document)
    # Changed by hand since the last vibe, or the first vibe here: this is
    # the person's own look, the one to come back to.
    if _saved is None or not _same(before, _applied):
        _saved = before
    look = built(name, fs, _drama(values), document)
    send(fs, look)
    _applied = current_look(fs)
    _last = name
    plugin.set_panel('vibes', vibe=name, status=f'{name}: {look["tagline"]}')
    plugin.status(f'Vibe: {name}')


@plugin.command('apply')
def apply(fs, values):
    name = values.get('vibe') or 'Cyberpunk'
    if name not in VIBES:
        plugin.status(f'No vibe called {name}')
        return
    vibe(fs, name, values)


@plugin.command('shuffle')
def shuffle(fs, values):
    choices = [name for name in VIBES if name != _last]
    vibe(fs, random.choice(choices), values)


@plugin.command('restore')
def back(fs, values):
    global _saved, _applied, _last
    if _saved is None:
        plugin.status('No vibe to take off: the look is already yours')
        return
    restore(fs, _saved)
    _saved = _applied = _last = None
    plugin.set_panel('vibes', status='Back to your own look.')
    plugin.status('Back to your own look')
