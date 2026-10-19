"""Studio Lighting: product-photography studios, one click each.

A studio is lights, a backdrop, a floor and a colour grade - never the
materials or the camera. The lights are pins on the environment, so they
light the product and show in its reflections the way softboxes, strip boxes
and flags do in a real studio. They are placed around the camera as it stands
when the studio is applied (or around the scene's front, if the person turns
that off), so the key light lands on the side it was designed for whichever
way the product is seen.

Every studio is built from lights with a role - key, fill, rim, accent, flag
- so the fine-tuning panels work the same on all of them: the Key slider
scales the key, Rims the rims, Warmth tints the white lights and leaves the
gels alone, and so on. Settings are kept between sessions and carry over
from studio to studio.

One Apply is one undo step. The look from before the first studio is kept,
and Back to my look puts it back however many studios came after; a look
changed by hand in between counts as the person's own from then on.
"""
import json
import math
import random

import tequila
import tequila_plugin as plugin

TAU = math.tau
NEUTRAL_KELVIN = 6500.0


# ---- Settings -----------------------------------------------------------------

DEFAULTS = {
    # Studio Lighting panel
    'preset': 'Three-Point Classic', 'turn': 0.0, 'brightness': 0.0, 'mood': 100.0,
    # Lights panel
    'key': 100.0, 'key_size': 100.0, 'key_height': 0.0, 'fill': 100.0, 'rim': 100.0, 'accents': 100.0,
    'warmth': 0.0, 'mirror': False, 'use_gels': False, 'gel_a': [1.0, 0.18, 0.58], 'gel_b': [0.18, 0.77, 1.0],
    # Backdrop & floor panel
    'backdrop': 'As the studio', 'backdrop_colour': [0.95, 0.72, 0.63], 'glow': 100.0,
    'floor': 'As the studio', 'reflection': 100.0, 'shadows': True, 'follow': True,
}
LIGHT_KEYS = ('key', 'key_size', 'key_height', 'fill', 'rim', 'accents', 'warmth', 'mirror', 'use_gels', 'gel_a', 'gel_b')
SET_KEYS = ('backdrop', 'backdrop_colour', 'glow', 'floor', 'reflection', 'shadows', 'follow')
PANELS = {
    'studio': ('preset', 'turn', 'brightness', 'mood'),
    'lights': LIGHT_KEYS,
    'set': SET_KEYS,
}


def settings():
    return plugin.load_settings(DEFAULTS)


def number(s, key, low, high):
    try:
        return max(low, min(high, float(s.get(key, DEFAULTS[key]))))
    except (TypeError, ValueError):
        return float(DEFAULTS[key])


# ---- Colour -------------------------------------------------------------------

def rgb(colour):
    """A swatch (hex or sRGB 0-1 triple) in the rendering space."""
    if isinstance(colour, (list, tuple)):
        colour = [min(max(float(c), 0.0), 1.0) for c in colour]
        # tequila.srgb reads values over 1 as 0-255; ours never are.
    return tequila.srgb(colour)


def scaled(colour, k):
    return [c * k for c in colour]


def mixed(a, b, t):
    return [x + (y - x) * t for x, y in zip(a, b)]


def kelvin_tint(kelvin):
    """The colour of light at `kelvin`, as an sRGB 0-1 triple with its
    luminance 1 (Tanner Helland's fit), so a tint changes hue, not
    brightness."""
    t = max(1000.0, min(40000.0, kelvin)) / 100.0
    r = 255.0 if t <= 66 else 329.698727446 * (t - 60) ** -0.1332047592
    g = 99.4708025861 * math.log(t) - 161.1195681661 if t <= 66 else 288.1221695283 * (t - 60) ** -0.0755148492
    b = 255.0 if t >= 66 else (0.0 if t <= 19 else 138.5177312231 * math.log(t - 10) - 305.0447927307)
    c = [min(max(v, 0.0), 255.0) / 255.0 for v in (r, g, b)]
    white = [min(max(v, 0.0), 255.0) / 255.0 for v in (255.0, 99.4708025861 * math.log(65) - 161.1195681661, 255.0)]
    c = [x / max(w, 1e-6) for x, w in zip(c, white)]          # 6500 K is white
    lum = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
    return [x / max(lum, 1e-6) for x in c]


def warmth_multiplier(warmth):
    """Warmth -100..100 as a per-channel multiplier: 0 is none, +100 about
    3400 K, -100 about 12000 K."""
    if abs(warmth) < 1e-6:
        return [1.0, 1.0, 1.0]
    kelvin = NEUTRAL_KELVIN * 2.0 ** (-warmth / 100.0 * 0.92)
    swatch = kelvin_tint(kelvin)
    top = max(swatch)
    tint = rgb([c / top for c in swatch])           # a swatch is 0-1; brightness is put back below
    neutral = rgb([1.0, 1.0, 1.0])
    lum = 0.2722 * tint[0] + 0.6741 * tint[1] + 0.0537 * tint[2]
    return [t / n / max(lum, 1e-6) for t, n in zip(tint, neutral)]


# ---- Where things go ----------------------------------------------------------

class Rig:
    """Directions in the world, given the way a photographer gives them -
    behind or in front of the product, to the left or right of the camera,
    up - for a camera, turned and mirrored as the person asked."""

    def __init__(self, camera, aspect, turn=0.0, mirror=False, follow=True):
        position = camera.get('position') or [0.0, 0.0, 0.0]
        target = camera.get('target') or [0.0, 0.0, 0.0]
        look = [t - p for t, p in zip(target, position)]
        flat = math.hypot(look[0], look[2])
        length = math.sqrt(sum(c * c for c in look))
        if flat > 1e-6:
            # The camera's bearing and line of sight: the backdrop faces it
            # and the ring light circles it, whatever the lights do.
            self.camera_forward = (look[0] / flat, look[2] / flat)
            self.view = [c / length for c in look]
        else:
            # No camera to speak of, or one looking straight down: the
            # scene's front, down -Z.
            self.camera_forward = (0.0, -1.0)
            self.view = [0.0, 0.0, -1.0] if flat < 1e-6 and length < 1e-6 else [c / length for c in look]
        # The lights' bearing: the camera's, or the scene's own front.
        self.forward = self.camera_forward if follow else (0.0, -1.0)
        focal = max(float(camera.get('focal_length') or 50.0), 1.0)
        # Half the picture's height as an angle: a 36 mm-wide sensor, as
        # Tequila's cameras have, at this focal length and aspect.
        self.half_height = math.atan(18.0 / (focal * max(aspect, 0.1)))
        self.turn = math.radians(turn)
        self.mirror = mirror

    def aim(self, back=0.0, right=0.0, up=0.0, height=0.0):
        """`back` past the product, away from the camera (negative: in front,
        on the camera's side); `right` to the camera's right; `up` up.
        `height` raises (or lowers) it by that many degrees."""
        c, s = math.cos(self.turn), math.sin(self.turn)
        b, r = back * c - right * s, back * s + right * c
        if self.mirror:
            r = -r
        if height:
            across = math.hypot(b, r)
            elevation = math.atan2(up, across) + math.radians(height)
            elevation = max(-1.4, min(1.45, elevation))
            reach = math.sqrt(b * b + r * r + up * up) or 1.0
            scale = reach * math.cos(elevation) / (across or 1.0)
            b, r, up = b * scale, r * scale, reach * math.sin(elevation)
        fx, fz = self.forward
        rx, rz = -fz, fx
        v = [fx * b + rx * r, up, fz * b + rz * r]
        n = math.sqrt(sum(x * x for x in v)) or 1.0
        return [x / n for x in v]

    def frame(self):
        """The camera's right and up, as directions."""
        fx, fz = self.camera_forward
        right = [-fz, 0.0, fx]
        v = self.view
        up = [right[1] * v[2] - right[2] * v[1], right[2] * v[0] - right[0] * v[2], right[0] * v[1] - right[1] * v[0]]
        return right, up

    def around_lens(self, radius, angle):
        """A direction `radius` radians off the line from the product back
        to the camera, at `angle` round it: where a ring light's bulbs are."""
        right, up = self.frame()
        back = [-c for c in self.view]
        d = [back[i] + (right[i] * math.cos(angle) + up[i] * math.sin(angle)) * math.tan(radius) for i in range(3)]
        n = math.sqrt(sum(x * x for x in d)) or 1.0
        return [x / n for x in d]


# ---- What a studio is made of ---------------------------------------------------

class Light:
    """One light (or flag) of a studio, by its role.

    role: key, fill, rim, accent or flag. A flag is a black card: it blocks
    the light from its direction and shows black in reflections. `gel` is
    'a' or 'b' for a coloured gel the person can recolour."""

    def __init__(self, role, name, where, size, colour='#FFFFFF', intensity=1.0, *, softness=0.5,
                 shape='rectangle', roll=0.0, gel=None, gradient=None, seen=False):
        self.role, self.name, self.where = role, name, where
        self.size, self.colour, self.intensity = list(size), colour, intensity
        self.softness, self.shape, self.roll = softness, shape, roll
        self.gel, self.gradient, self.seen = gel, gradient, seen


def key(name, where, size, intensity, colour='#FFFFFF', **kw):
    return Light('key', name, where, size, colour, intensity, **kw)


def fill(name, where, size, intensity, colour='#FFFFFF', **kw):
    return Light('fill', name, where, size, colour, intensity, **kw)


def rim(name, where, size, intensity, colour='#FFFFFF', **kw):
    return Light('rim', name, where, size, colour, intensity, **kw)


def accent(name, where, size, intensity, colour='#FFFFFF', **kw):
    return Light('accent', name, where, size, colour, intensity, **kw)


def flag(name, where, size, softness=0.2):
    return Light('flag', name, where, size, '#000000', 1.0, softness=softness)


def at(back=0.0, right=0.0, up=0.0):
    """A place round the product (see Rig.aim), worked out when applied."""
    return ('aim', back, right, up)


class Studio:
    def __init__(self, name, about, *, lights, room, backdrop, glow=None, glow_gain=1.0, glow_size=0.35,
                 backdrop_lights=False, floor=None, grade=None, flat_background=None):
        self.name, self.about = name, about
        self.lights = lights
        self.room = room                      # [(t, hex)], nadir 0 to zenith 1
        self.backdrop = backdrop              # the backdrop's colour at the edges of the picture
        self.glow = glow or backdrop          # its colour in the glow straight behind the product
        self.glow_gain = glow_gain            # over 1 for a glow brighter than white
        self.glow_size = glow_size            # how far out the glow reaches, 0-1
        # A backdrop that is itself the light - a lit panel behind glass, a
        # halo - lights the product and shows through glass; others are
        # only seen.
        self.backdrop_lights = backdrop_lights
        self.floor = floor or {}
        self.grade = grade or {}
        self.flat_background = flat_background   # a plain colour behind instead of the backdrop pin


def floor(*, reflections=0.0, blur=0.2, length=1.0, fresnel=True, shadows=True, shadow='#000000', enabled=True):
    return dict(reflections=reflections, blur=blur, length=length, fresnel=fresnel, shadows=shadows,
                shadow=shadow, enabled=enabled)


def scatter(count, seed, low=25, high=80, size=0.03, intensity=380.0):
    """Small hard lights spread over the upper half: what makes a cut stone
    flash. The same each time."""
    rng = random.Random(seed)
    out = []
    for i in range(count):
        azimuth = rng.uniform(0, TAU)
        elevation = math.radians(rng.uniform(low, high))
        back, right, up = math.cos(azimuth), math.sin(azimuth), math.tan(elevation)
        s = size * rng.uniform(0.8, 1.25)
        out.append(accent(f'Sparkle {i + 1}', at(back, right, up), (s, s), intensity * rng.uniform(0.7, 1.2),
                          softness=0.05, shape='ellipse'))
    return out


# ---- The studios -----------------------------------------------------------------
# Directions: at(back, right, up) - back past the product (negative: in front),
# right of the camera, up. Sizes in radians across and tall. Intensities are
# the environment's: a 1 x 1 radian light at 1 is a gentle softbox.

STUDIOS = [
    Studio(
        'Three-Point Classic',
        'The all-rounder: a soft key 45° front-left, a gentle fill opposite and a rim behind to lift the edges off a soft '
        'grey backdrop. Start here when you are not sure.',
        lights=[
            key('Key softbox', at(-0.75, -0.75, 0.75), (0.9, 0.75), 3.8, softness=0.7),
            fill('Fill', at(-0.8, 0.85, 0.25), (1.3, 1.0), 1.1, softness=0.9),
            fill('Top', at(0.0, 0.0, 1.0), (0.8, 0.8), 0.6, softness=0.8),
            rim('Rim', at(0.9, 0.75, 0.35), (0.12, 1.0), 6.0, softness=0.35),
        ],
        room=[(0.0, '#141416'), (0.5, '#1E1F22'), (1.0, '#2A2B2F')],
        backdrop='#3C3E42', glow='#8E9196', glow_gain=1.0, glow_size=0.45,
        floor=floor(reflections=0.06, blur=0.3),
        grade=dict(contrast=1.05, highlights=-0.1, vignette=0.15)),
    Studio(
        'White Infinity',
        'Clean e-commerce white: a pure white background, big soft lights from above and both sides, and a light contact '
        "shadow so the product doesn't float. For catalogue and marketplace shots.",
        lights=[
            key('Overhead softbox', at(-0.2, 0.0, 1.0), (1.6, 1.2), 2.2, softness=0.85),
            fill('Left softbox', at(-0.6, -0.9, 0.3), (1.0, 1.2), 1.2, softness=0.9),
            fill('Right softbox', at(-0.6, 0.9, 0.3), (1.0, 1.2), 1.2, softness=0.9),
            fill('Front card', at(-1.0, 0.0, 0.25), (1.4, 0.7), 0.8, softness=1.0),
        ],
        room=[(0.0, '#D8D8D8'), (0.5, '#F2F2F2'), (1.0, '#FFFFFF')],
        backdrop='#FFFFFF', glow='#FFFFFF', glow_gain=1.0, glow_size=0.5, flat_background='#FFFFFF',
        floor=floor(shadow='#7A7A7A'),
        grade=dict(exposure=0.35, contrast=0.95, highlights=-0.25, whites=0.1)),
    Studio(
        'Soft Light Tent',
        'Light from everywhere, like inside a diffusion tent: almost no shadows and smooth, even reflections. For glossy, '
        'reflective and small products - cosmetics, ceramics, jewellery.',
        lights=[
            key('Front-left panel', at(-0.65, -0.75, 0.45), (1.1, 1.2), 1.5, softness=1.0),
            fill('Front-right panel', at(-0.65, 0.75, 0.45), (1.1, 1.2), 1.2, softness=1.0),
            rim('Back-left panel', at(0.65, -0.75, 0.45), (1.1, 1.2), 1.0, softness=1.0),
            rim('Back-right panel', at(0.65, 0.75, 0.45), (1.1, 1.2), 1.0, softness=1.0),
            fill('Tent roof', at(0.0, 0.0, 1.0), (1.5, 1.5), 1.4, softness=1.0),
        ],
        room=[(0.0, '#BEBCBA'), (0.5, '#E6E4E1'), (1.0, '#F7F6F4')],
        backdrop='#C9C6C2', glow='#EDEAE6', glow_gain=1.0, glow_size=0.5,
        floor=floor(reflections=0.08, blur=0.5, shadow='#8E8A86'),
        grade=dict(exposure=0.2, contrast=0.95, highlights=-0.2)),
    Studio(
        'Clamshell Beauty',
        "A beauty dish high in front and a reflector low in front - 'clamshell' style - for even, flattering light with "
        'round catchlights, and two soft rims. For cosmetics, skincare, fragrance and packaging.',
        lights=[
            key('Beauty dish', at(-1.0, 0.0, 0.8), (1.0, 0.75), 3.0, softness=0.8, shape='ellipse'),
            fill('Reflector below', at(-1.0, 0.0, -0.22), (1.4, 0.45), 1.1, softness=1.0),
            rim('Left rim', at(0.8, -0.9, 0.3), (0.1, 0.9), 3.5, softness=0.4),
            rim('Right rim', at(0.8, 0.9, 0.3), (0.1, 0.9), 3.5, softness=0.4),
        ],
        room=[(0.0, '#3A3631'), (0.5, '#5A544E'), (1.0, '#6E6862')],
        backdrop='#8C8379', glow='#D2C7BA', glow_gain=1.0, glow_size=0.45,
        floor=floor(reflections=0.1, blur=0.25, shadow='#3B332C'),
        grade=dict(exposure=0.1, highlights=-0.15, saturation=1.05, temperature=6200.0, vignette=0.2)),
    Studio(
        'Overhead Flat-Lay',
        'One big softbox straight overhead with soft strips either side: short, soft shadows under everything. For '
        'top-down flat-lays, tabletop arrangements and anything shot from above.',
        lights=[
            key('Overhead softbox', at(0.15, -0.25, 1.0), (1.6, 1.2), 3.0, softness=0.85),
            rim('Left strip', at(0.0, -1.0, 0.6), (0.15, 1.2), 2.5, softness=0.5),
            rim('Right strip', at(0.0, 1.0, 0.6), (0.15, 1.2), 2.5, softness=0.5),
            fill('Bounce', at(-1.0, 0.0, 0.3), (1.2, 0.8), 0.5, softness=1.0),
        ],
        room=[(0.0, '#1A1A1A'), (0.5, '#2A2A2A'), (1.0, '#3C3C3C')],
        backdrop='#7A7A7A', glow='#B4B4B4', glow_gain=1.0, glow_size=0.55,
        floor=floor(shadow='#2A2A2A'),
        grade=dict(exposure=0.15)),
    Studio(
        'Window Light',
        "Soft daylight from a big window to one side and a white bounce card opposite: airy, natural and gentle - the "
        "lifestyle and editorial look. You'll see the window's panes in glossy surfaces.",
        lights=[
            key('Window', at(-0.35, -1.0, 0.45), (1.0, 1.3), 3.2, '#FFF8EE', softness=0.35),
            flag('Window bar, upright', at(-0.35, -1.0, 0.45), (0.035, 1.3), softness=0.1),
            flag('Window bar, across', at(-0.35, -1.0, 0.45), (1.0, 0.035), softness=0.1),
            fill('Bounce card', at(-0.3, 1.0, 0.2), (1.2, 1.0), 0.7, '#FFF9F2', softness=1.0),
        ],
        room=[(0.0, '#A9A8A6'), (0.5, '#D9D8D5'), (1.0, '#ECECEC')],
        backdrop='#C8C4BE', glow='#F0ECE6', glow_gain=1.0, glow_size=0.5,
        floor=floor(reflections=0.1, blur=0.4, shadow='#6E6A64'),
        grade=dict(exposure=0.3, contrast=0.98, highlights=-0.25, shadows=0.1, temperature=6300.0)),
    Studio(
        'Overcast Daylight',
        'An overcast sky: soft, cool light from everywhere above and gentle shadows underneath - natural and honest. For '
        'outdoor gear, shoes, plants, anything that should feel real.',
        lights=[
            key('Sky', at(0.0, 0.0, 1.0), (2.0, 2.0), 1.2, '#F2F6FF', softness=1.0),
            fill('Bright horizon', at(-1.0, -0.4, 0.15), (2.2, 0.5), 0.6, '#EEF2F8', softness=1.0),
        ],
        room=[(0.0, '#6F7378'), (0.45, '#B8BEC6'), (0.6, '#CDD3DA'), (1.0, '#E6EBF1')],
        backdrop='#B8BEC6', glow='#E2E7ED', glow_gain=1.0, glow_size=0.55,
        floor=floor(reflections=0.05, blur=0.5, shadow='#5C6066'),
        grade=dict(exposure=0.2, contrast=0.95, saturation=0.95, temperature=6900.0)),
    Studio(
        'Ring Light',
        'A ring of light round the lens: flat, nearly shadowless front light and a circular catchlight in every glossy '
        'surface. The social-media and beauty look - strong with cosmetics and glossy packaging.',
        lights=[],                     # the ring is built round the lens when applied
        room=[(0.0, '#4A4A4E'), (0.5, '#6A6A70'), (1.0, '#7E7E84')],
        backdrop='#9A9AA0', glow='#D6D6DC', glow_gain=1.0, glow_size=0.45,
        floor=floor(reflections=0.15, blur=0.2, shadow='#4A4A50'),
        grade=dict(exposure=0.15, contrast=0.98, highlights=-0.1)),
    Studio(
        'Hero Gradient',
        'The classic launch hero: a dark set with a soft glow behind the product, a sculpting key and two strip lights '
        'tracing its edges, on a semi-gloss floor. Works for almost anything that should look premium.',
        lights=[
            key('Key', at(-0.55, -0.8, 0.7), (0.7, 0.9), 4.0, softness=0.6),
            fill('Top', at(-0.1, 0.1, 1.0), (0.6, 0.4), 1.2, softness=0.7),
            rim('Left strip', at(0.85, -0.85, 0.2), (0.09, 1.2), 9.0, softness=0.3),
            rim('Right strip', at(0.85, 0.85, 0.2), (0.09, 1.2), 9.0, softness=0.3),
        ],
        room=[(0.0, '#050506'), (0.5, '#08080A'), (1.0, '#0C0C0E')],
        backdrop='#050506', glow='#55585E', glow_gain=1.0, glow_size=0.32,
        floor=floor(reflections=0.3, blur=0.12),
        grade=dict(contrast=1.12, highlights=-0.15, blacks=-0.04, vignette=0.35, bloom=0.08)),
    Studio(
        'Gradient Chrome',
        'For polished metal and chrome, which only show what they reflect: two tall graduated softboxes and a graded top '
        'light give long smooth highlights, and black flags give the defining dark lines. Watches, cutlery, appliances.',
        lights=[
            key('Left graduated softbox', at(-0.35, -1.0, 0.35), (0.9, 1.6), 4.0, softness=0.3,
                gradient=[(0.0, '#000000'), (0.35, '#3A3A3A'), (1.0, '#FFFFFF')]),
            fill('Right graduated softbox', at(-0.35, 1.0, 0.35), (0.9, 1.6), 3.0, softness=0.3,
                 gradient=[(0.0, '#000000'), (0.35, '#3A3A3A'), (1.0, '#FFFFFF')]),
            fill('Graded top', at(0.2, 0.0, 1.0), (1.4, 0.9), 2.2, softness=0.3,
                 gradient=[(0.0, '#FFFFFF'), (1.0, '#101010')]),
            flag('Front-left flag', at(-1.0, -0.45, 0.2), (0.45, 1.2)),
            flag('Front-right flag', at(-1.0, 0.45, 0.2), (0.45, 1.2)),
            rim('Left edge strip', at(0.9, -0.8, 0.25), (0.06, 1.1), 8.0, softness=0.3),
            rim('Right edge strip', at(0.9, 0.8, 0.25), (0.06, 1.1), 8.0, softness=0.3),
        ],
        room=[(0.0, '#08080A'), (0.5, '#0E0E10'), (1.0, '#141416')],
        backdrop='#070708', glow='#3A3C40', glow_gain=1.0, glow_size=0.35,
        floor=floor(reflections=0.35, blur=0.05),
        grade=dict(contrast=1.15, highlights=-0.1, saturation=0.95, vignette=0.3)),
    Studio(
        'Glass: Bright Field',
        'Glass lit from behind: a bright panel behind the product and black flags either side draw crisp dark outlines on '
        'every edge, and liquids glow with their colour. The standard for bottles, glasses and clear plastics.',
        lights=[
            accent('Backlight', at(1.0, 0.0, 0.1), (0.55, 0.75), 6.0, softness=0.6),
            flag('Left flag', at(0.55, -0.95, 0.1), (0.5, 1.4)),
            flag('Right flag', at(0.55, 0.95, 0.1), (0.5, 1.4)),
            fill('Front fill', at(-1.0, -0.3, 0.4), (0.6, 0.5), 0.6, softness=0.8),
            key('Top strip', at(0.2, 0.0, 1.0), (0.9, 0.12), 3.0, softness=0.4),
        ],
        room=[(0.0, '#0C0C0C'), (0.5, '#161616'), (1.0, '#202020')],
        backdrop='#0A0A0A', glow='#FFFFFF', glow_gain=1.6, glow_size=0.34, backdrop_lights=True,
        floor=floor(reflections=0.25, blur=0.05),
        grade=dict(contrast=1.1, highlights=-0.1, whites=0.05)),
    Studio(
        'Glass: Dark Field',
        'Glass on black: tall strips behind either side and one above outline the glass in bright lines against a black '
        'void, with a hidden light behind to make liquids glow. The moody, premium spirits-ad look.',
        lights=[
            rim('Left strip', at(0.9, -1.0, 0.15), (0.08, 1.4), 14.0, softness=0.3),
            rim('Right strip', at(0.9, 1.0, 0.15), (0.08, 1.4), 14.0, softness=0.3),
            key('Top strip', at(0.4, 0.0, 1.0), (1.0, 0.08), 6.0, softness=0.3),
            accent('Hidden glow behind', at(1.0, 0.0, 0.0), (0.35, 0.5), 4.0, softness=0.8),
            fill('Front whisper', at(-1.0, -0.2, 0.3), (0.6, 0.5), 0.2, softness=1.0),
        ],
        room=[(0.0, '#000000'), (1.0, '#030303')],
        backdrop='#010101', glow='#1A1A1C', glow_gain=1.0, glow_size=0.3,
        floor=floor(reflections=0.45, blur=0.02),
        grade=dict(contrast=1.15, blacks=-0.05, vignette=0.3)),
    Studio(
        'Jewellery Sparkle',
        "For gems and polished metal: a soft tent shapes the metal, rims trace it, and a scatter of tiny hard lights makes "
        "stones flash. The camera is flagged black so a stone's table reads deep and rich.",
        lights=[
            key('Left panel', at(-0.6, -0.8, 0.5), (0.9, 1.0), 2.2, softness=0.9),
            fill('Right panel', at(-0.6, 0.8, 0.5), (0.9, 1.0), 1.6, softness=0.9),
            fill('Roof', at(0.0, 0.0, 1.0), (1.2, 1.0), 1.6, softness=0.9),
            rim('Left rim', at(0.9, -0.8, 0.25), (0.1, 1.1), 8.0, softness=0.35),
            rim('Right rim', at(0.9, 0.8, 0.25), (0.1, 1.1), 8.0, softness=0.35),
            flag('Camera flag', at(-1.0, 0.0, 0.05), (0.25, 0.25), softness=0.3),
            *scatter(14, seed=11),
        ],
        room=[(0.0, '#090705'), (0.5, '#100C09'), (1.0, '#16120E')],
        backdrop='#060403', glow='#4A3826', glow_gain=1.0, glow_size=0.3,
        floor=floor(reflections=0.4, blur=0.03),
        grade=dict(contrast=1.1, highlights=-0.15, vignette=0.35, bloom=0.15, bloom_radius=0.02)),
    Studio(
        'Low-Key Glossy',
        'Luxury on black glass: a long overhead strip and two edge strips draw clean lines on glossy surfaces, mirrored in '
        'a black floor. For watches, phones, premium electronics.',
        lights=[
            key('Overhead strip', at(-0.2, 0.0, 1.0), (1.1, 0.25), 5.0, softness=0.4),
            rim('Left edge strip', at(0.75, -0.95, 0.25), (0.07, 1.2), 12.0, softness=0.3),
            rim('Right edge strip', at(0.75, 0.95, 0.25), (0.07, 1.2), 12.0, softness=0.3),
            fill('Front line', at(-1.0, 0.0, 0.1), (1.2, 0.15), 1.2, softness=0.5),
        ],
        room=[(0.0, '#000000'), (1.0, '#040405')],
        backdrop='#020203', glow='#2A2C30', glow_gain=1.0, glow_size=0.3,
        floor=floor(reflections=0.7, blur=0.01, length=0.5),
        grade=dict(contrast=1.15, blacks=-0.05, vignette=0.35, sharpening=0.1)),
    Studio(
        'Rim Silhouette',
        'Black on black, drawn only by its edges: two strong rims behind and one above trace the silhouette, with almost '
        'no front light. Dramatic for dark products - headphones, cameras, sneakers, phones.',
        lights=[
            rim('Left rim', at(0.95, -0.9, 0.2), (0.1, 1.3), 26.0, softness=0.3),
            rim('Right rim', at(0.95, 0.9, 0.25), (0.1, 1.3), 22.0, '#E8F0FF', softness=0.3),
            rim('Top rim', at(1.0, 0.0, 1.1), (0.7, 0.12), 8.0, softness=0.4),
            key('Whisper', at(-0.8, -0.4, 0.5), (0.5, 0.4), 0.12, softness=0.9),
        ],
        room=[(0.0, '#000000'), (1.0, '#020203')],
        backdrop='#010102', glow='#2E3036', glow_gain=1.0, glow_size=0.28,
        floor=floor(reflections=0.35, blur=0.03),
        grade=dict(contrast=1.2, blacks=-0.06, vignette=0.4)),
    Studio(
        'Spotlight',
        'One hard light from almost straight above, like a theatre spot: a crisp, short shadow, a pool of light behind, '
        'everything else falling to black. Bold and graphic.',
        lights=[
            key('Spot', at(-0.35, -0.2, 1.0), (0.09, 0.09), 650.0, softness=0.3, shape='ellipse'),
            rim('Faint rim', at(0.9, 0.6, 0.35), (0.06, 0.9), 4.0, softness=0.3),
        ],
        room=[(0.0, '#000000'), (1.0, '#010101')],
        backdrop='#000000', glow='#8A8A8A', glow_gain=1.0, glow_size=0.42,
        floor=floor(reflections=0.15, blur=0.2),
        grade=dict(contrast=1.25, highlights=-0.1, blacks=-0.08, vignette=0.6, bloom=0.12)),
    Studio(
        'Chiaroscuro',
        'Light and shadow, as in a Rembrandt: one large soft light from the side, black on the other side and a faint '
        'kick on the far edge. Painterly and sculptural - for shapes, textures, food and still life.',
        lights=[
            key('Side softbox', at(-0.1, -1.0, 0.35), (1.0, 1.2), 6.0, '#FFF1E0', softness=0.9),
            flag('Black card', at(0.0, 1.0, 0.3), (1.2, 1.4), softness=0.6),
            rim('Kick', at(0.8, 0.9, 0.2), (0.05, 0.8), 2.0, softness=0.3),
        ],
        room=[(0.0, '#000000'), (1.0, '#030303')],
        backdrop='#030202', glow='#4A443D', glow_gain=1.0, glow_size=0.4,
        floor=floor(reflections=0.08, blur=0.3),
        grade=dict(contrast=1.2, saturation=0.9, temperature=6000.0, vignette=0.5, blacks=-0.04)),
    Studio(
        'Backlit Halo',
        'A glowing halo straight behind the product throws it into silhouette, with light wrapping round every edge and a '
        'touch of bloom. The dramatic reveal shot.',
        lights=[
            accent('Halo light', at(1.0, 0.0, 0.2), (0.4, 0.5), 3.0, softness=0.7),
            rim('Left rim', at(0.9, -0.7, 0.3), (0.06, 0.9), 10.0, softness=0.3),
            rim('Right rim', at(0.9, 0.7, 0.3), (0.06, 0.9), 10.0, softness=0.3),
            key('Front whisper', at(-1.0, 0.0, 0.3), (0.8, 0.6), 0.25, softness=1.0),
        ],
        room=[(0.0, '#000000'), (1.0, '#020202')],
        backdrop='#000000', glow='#FFFFFF', glow_gain=2.5, glow_size=0.3, backdrop_lights=True,
        floor=floor(reflections=0.3, blur=0.05),
        grade=dict(contrast=1.15, highlights=-0.2, vignette=0.45, bloom=0.2, bloom_radius=0.05)),
    Studio(
        'Cinematic Teal & Orange',
        'The movie-poster look: a cool teal key against a warm orange back light, on a deep teal set with a glossy floor. '
        'Moody and colourful - gadgets, gear, sneakers, bottles. Change the two gels in Lights….',
        lights=[
            key('Teal key', at(-0.6, 0.8, 0.6), (0.6, 0.8), 3.0, '#9FD8E8', softness=0.6, gel='a'),
            rim('Teal edge', at(0.3, 1.0, 0.05), (0.05, 0.6), 10.0, '#3FD0D8', softness=0.3, gel='a'),
            rim('Orange back rim', at(0.9, -0.8, 0.3), (0.1, 1.0), 18.0, '#FF8A2A', softness=0.3, gel='b'),
            rim('Orange back glow', at(1.0, 0.3, 0.6), (0.35, 0.35), 6.0, '#FF7A1A', softness=0.6, gel='b'),
        ],
        room=[(0.0, '#010303'), (0.5, '#030707'), (1.0, '#05090B')],
        backdrop='#021012', glow='#1E5A60', glow_gain=1.0, glow_size=0.35,
        floor=floor(reflections=0.4, blur=0.06),
        grade=dict(contrast=1.15, saturation=1.15, vignette=0.4, bloom=0.1)),
    Studio(
        'Split Gel',
        'Two coloured gels, one from each side, meeting on the product: bold, graphic colour on a dark set. Magenta and '
        'cyan as it comes - set your own pair in Lights….',
        lights=[
            key('Gel A, left', at(-0.2, -1.0, 0.35), (0.35, 1.0), 6.0, '#FF2E93', softness=0.5, gel='a'),
            rim('Gel B, right', at(-0.1, 1.0, 0.35), (0.35, 1.0), 6.0, '#2EC5FF', softness=0.5, gel='b'),
            fill('White top', at(0.0, 0.0, 1.0), (0.6, 0.5), 0.6, softness=0.8),
        ],
        room=[(0.0, '#030206'), (0.5, '#06050C'), (1.0, '#0B0914')],
        backdrop='#05030C', glow='#3A2660', glow_gain=1.0, glow_size=0.35,
        floor=floor(reflections=0.45, blur=0.05),
        grade=dict(contrast=1.1, saturation=1.25, vignette=0.35, bloom=0.15)),
    Studio(
        'Hard Sun Pop',
        'Bright, hard sunlight on a coloured seamless: crisp graphic shadows and punchy colour - the trendy brand look. '
        'Pick your own colour in Backdrop & floor… (Backdrop: Custom colour).',
        lights=[
            key('Sun', at(-0.3, -0.9, 0.9), (0.05, 0.05), 900.0, '#FFF4E2', softness=0.1, shape='ellipse'),
            fill('Sky', at(0.0, 0.3, 1.0), (1.6, 1.2), 0.4, '#EAF2FF', softness=1.0),
        ],
        room=[(0.0, '#C98E78'), (0.5, '#E8C8B6'), (1.0, '#F6EDE2')],
        backdrop='#F0B49B', glow='#F6CDB9', glow_gain=1.0, glow_size=0.6,
        floor=floor(shadow='#A8644E'),
        grade=dict(exposure=0.25, contrast=1.1, saturation=1.2, highlights=-0.2, temperature=6800.0)),
]
BY_NAME = {s.name: s for s in STUDIOS}
NAMES = [s.name for s in STUDIOS]

BACKDROPS = {'Black': '#030303', 'Charcoal': '#202124', 'Mid grey': '#6E7075', 'Light grey': '#C4C5C8', 'White': '#FFFFFF'}
FLOORS = {
    'Shadows only': dict(reflections=0.0, blur=0.2, length=1.0, fresnel=True),
    'Satin': dict(reflections=0.15, blur=0.35, length=1.0, fresnel=True),
    'Glossy': dict(reflections=0.35, blur=0.06, length=1.0, fresnel=True),
    'Black mirror': dict(reflections=0.75, blur=0.0, length=1.0, fresnel=True),
}


# ---- Building a look ----------------------------------------------------------

def ring_lights(rig):
    """A ring light: 20 bulbs round the lens, read as one ring in a gloss."""
    return [key(f'Ring {i + 1}', ('lens', 0.085, TAU * i / 20), (0.04, 0.04), 200.0, softness=0.6, shape='ellipse')
            for i in range(20)]


def pin_for(light, rig, s, warm):
    where = light.where
    if where[0] == 'lens':
        direction = rig.around_lens(where[1], where[2])
    else:
        height = number(s, 'key_height', -40, 40) if light.role == 'key' else 0.0
        direction = rig.aim(*where[1:], height=height)
    size = list(light.size)
    intensity = light.intensity
    if light.role == 'key':
        k = number(s, 'key_size', 25, 400) / 100.0
        size = [min(x * k, 3.0) for x in size]
        intensity /= k * k                       # the same light, spread wider
        intensity *= number(s, 'key', 0, 300) / 100.0
    elif light.role == 'fill':
        intensity *= number(s, 'fill', 0, 300) / 100.0
    elif light.role == 'rim':
        intensity *= number(s, 'rim', 0, 300) / 100.0
    elif light.role == 'accent':
        intensity *= number(s, 'accents', 0, 300) / 100.0

    if light.role == 'flag':
        colour = [0.0, 0.0, 0.0]
    elif light.gel and s.get('use_gels'):
        colour = rgb(s.get('gel_a' if light.gel == 'a' else 'gel_b') or [1.0, 1.0, 1.0])
    else:
        colour = rgb(light.colour)
        if not light.gel:
            colour = [c * w for c, w in zip(colour, warm)]

    pin = {
        'name': f'Studio: {light.name}', 'kind': 'light', 'shape': light.shape, 'direction': direction,
        'angular_size': size, 'roll': light.roll, 'softness': light.softness, 'intensity': max(intensity, 0.0),
        'color': colour, 'show_in_background': light.seen, 'lighting': True,
        'blend': 'over' if light.role == 'flag' else 'add', 'enabled': True,
    }
    if light.gradient:
        pin.update(kind='gradient', radial=False, color=[1.0, 1.0, 1.0],
                   stops=[{'t': t, 'color': [c * w for c, w in zip(rgb(hex_), warm)]} for t, hex_ in light.gradient])
        # A gradient's colour is its stops; the intensity still scales it.
    return pin


def room_pin(studio, fill_scale):
    return {
        'name': 'Studio: Room', 'kind': 'gradient', 'radial': True,
        'stops': [{'t': t, 'color': rgb(hex_)} for t, hex_ in studio.room],
        'shape': 'rectangle', 'direction': [0.0, -1.0, 0.0], 'angular_size': [TAU, TAU], 'roll': 0.0,
        'softness': 0.0, 'intensity': max(fill_scale, 0.0), 'color': [1.0, 1.0, 1.0],
        'show_in_background': True, 'lighting': True, 'blend': 'over', 'enabled': True,
    }


def backdrop_pin(rig, edge, centre, glow_size, lights=False):
    """What the camera sees behind the product: `centre` straight behind
    it, falling off to `edge` by `glow_size` of the way out. It covers the
    whole picture, floor included. It lights nothing unless `lights` - a
    lit panel or halo that is itself the light."""
    spread = 2.6                                   # half picture-heights: past the corners
    # A bell-shaped falloff, so even a glow brighter than white fades out
    # softly instead of ending in an edge.
    bell = [(0.0, 1.0), (0.2, 0.78), (0.4, 0.48), (0.6, 0.22), (0.8, 0.07), (1.0, 0.0)]
    stops = [(glow_size * x, mixed(edge, centre, w)) for x, w in bell] + [(1.0, edge)]
    return {
        'name': 'Studio: Backdrop', 'kind': 'gradient', 'radial': True,
        'stops': [{'t': t, 'color': c} for t, c in stops],
        'shape': 'ellipse', 'direction': rig.view,
        'angular_size': [min(3.1, 2.0 * rig.half_height * spread)] * 2, 'roll': 0.0,
        'softness': 0.0, 'intensity': 1.0, 'color': [1.0, 1.0, 1.0],
        'show_in_background': True, 'lighting': lights, 'blend': 'over', 'enabled': True,
    }


NEUTRAL = dict(exposure=0.0, contrast=1.0, saturation=1.0, highlights=0.0, shadows=0.0, whites=0.0,
               blacks=0.0, temperature=NEUTRAL_KELVIN, tint=0.0, vignette=0.0, bloom=0.0,
               bloom_radius=0.03, sharpening=0.0, chromatic_aberration=0.0)


def graded(grade, mood, brightness, display):
    """The image style for a studio's grade at `mood` (1 as designed, 0 none,
    2 double), `brightness` stops brighter, keeping the person's display
    transform."""
    out = dict(NEUTRAL, **grade)
    for k in ('contrast', 'saturation'):
        out[k] = 1.0 + (out[k] - 1.0) * mood
    out['temperature'] = NEUTRAL_KELVIN + (out['temperature'] - NEUTRAL_KELVIN) * mood
    for k in ('highlights', 'shadows', 'whites', 'blacks', 'tint', 'vignette', 'bloom', 'sharpening', 'chromatic_aberration'):
        out[k] *= mood
    out['exposure'] = out['exposure'] + brightness
    clamp = lambda v, lo, hi: max(lo, min(hi, v))
    out['exposure'] = clamp(out['exposure'], -10.0, 10.0)
    out['contrast'] = clamp(out['contrast'], 0.2, 3.0)
    out['saturation'] = clamp(out['saturation'], 0.0, 3.0)
    out['temperature'] = clamp(out['temperature'], 2000.0, 15000.0)
    for k in ('highlights', 'shadows', 'whites', 'blacks', 'tint'):
        out[k] = clamp(out[k], -1.0, 1.0)
    for k in ('vignette', 'bloom', 'sharpening'):
        out[k] = clamp(out[k], 0.0, 1.0)
    out['type'] = 'Photographic'
    out['display_transform'] = display
    return out


def build(fs, studio, s, document=None):
    """Everything applying `studio` with settings `s` sends, for the camera
    as it is now."""
    document = document or fs.context()['document']
    width, height = (document.get('render') or {}).get('resolution') or (1920, 1080)
    rig = Rig(fs.query('camera'), width / max(height, 1), turn=number(s, 'turn', -180, 180),
              mirror=bool(s.get('mirror')), follow=bool(s.get('follow', True)))
    warm = warmth_multiplier(number(s, 'warmth', -100, 100))
    fill_scale = number(s, 'fill', 0, 300) / 100.0

    lights = list(studio.lights) + (ring_lights(rig) if studio.name == 'Ring Light' else [])
    pins = []
    choice = s.get('backdrop') or 'As the studio'
    mine = choice == 'My environment (HDRI)'
    if not mine:
        pins.append(room_pin(studio, fill_scale))

    # The backdrop: the studio's, or the person's colour; its glow scaled.
    # 100 % is the glow as designed, 0 an even backdrop, 300 three times the
    # step from the edge to the glow.
    g = number(s, 'glow', 0, 300) / 100.0
    overridden = choice != 'As the studio'
    if choice in BACKDROPS or choice == 'Custom colour':
        colour = rgb(BACKDROPS[choice] if choice in BACKDROPS else (s.get('backdrop_colour') or DEFAULTS['backdrop_colour']))
        # Brighter in the middle by a share of itself, and by a little grey
        # so a black one still shows a glow through the tone curve's toe.
        centre = [c * (1.0 + 0.6 * g) + 0.05 * g for c in colour]
    else:
        colour = rgb(studio.backdrop)
        centre = mixed(colour, scaled(rgb(studio.glow), studio.glow_gain), g)
    flat = None if overridden else studio.flat_background
    if not mine and flat is None:
        pins.append(backdrop_pin(rig, colour, centre, studio.glow_size, studio.backdrop_lights and not overridden))
    pins += [pin_for(light, rig, s, warm) for light in lights]

    # The floor: the studio's, or the person's choice; reflection scaled.
    f = dict(studio.floor or floor())
    floor_choice = s.get('floor') or 'As the studio'
    if floor_choice in FLOORS:
        f.update(FLOORS[floor_choice])
    reflections = f.get('reflections', 0.0) * number(s, 'reflection', 0, 200) / 100.0
    if overridden and not mine:
        # Shadows in the backdrop's own hue, as a coloured seamless has them.
        shadow = scaled(colour, 0.28)
    else:
        shadow = rgb(f.get('shadow', '#000000'))
    params = {
        'rotation': 0.0, 'brightness': 1.0, 'contrast': 1.0, 'size': 0.0,
        'ground_enabled': floor_choice != 'No floor (floating)' and f.get('enabled', True),
        'ground_shadows': bool(s.get('shadows', True)) and f.get('shadows', True),
        'ground_shadow_color': shadow, 'ground_occlusion': False,
        'ground_reflections': min(reflections, 1.0), 'ground_reflection_blur': f.get('blur', 0.2),
        'ground_reflection_length': f.get('length', 1.0), 'ground_reflection_fresnel': f.get('fresnel', True),
    }
    if not mine:
        params['image'] = str(black_environment())
    elif _saved is not None:
        # The person's own image, as it was before the first studio put the
        # black one in its place.
        params['image'] = _saved['environment'].get('image')
    if mine:
        params.update(background='Environment')
    elif flat is not None:
        params.update(background='Color', background_color=rgb(flat))
    else:
        params.update(background='Environment')

    display = document['image_style'].get('display_transform', 'PBR Neutral')
    style = graded(studio.grade, number(s, 'mood', 0, 200) / 100.0, number(s, 'brightness', -3, 3), display)
    return {'sky': None, 'params': params, 'pins': pins, 'style': style}


def black_environment():
    """A black 2048 x 1024 environment, made once in the plugin's data
    folder. A studio covers it completely, but its lights are painted at the
    environment image's resolution: on a small image (a 16 x 8 sky, say)
    the small ones - spots, sparkles, ring bulbs - would vanish, and on the
    built-in studio's 1024 they would blur."""
    path = plugin.data_folder() / 'studio-black-2048x1024.hdr'
    if not path.exists():
        w, h = 2048, 1024
        temporary = path.with_suffix('.tmp')
        with open(temporary, 'wb') as f:
            f.write(b'#?RADIANCE' + bytes([10]) + b'FORMAT=32-bit_rle_rgbe' + bytes([10, 10]))
            f.write(f'-Y {h} +X {w}'.encode() + bytes([10]))
            f.write(bytes(w * h * 4))                # RGBE zero: black
        temporary.replace(path)
    return path


def send(fs, look):
    """A look's commands, as one undo step."""
    with fs.batch() as batch:
        environment = batch.commands.environment
        environment.set_sky(sky=look['sky'])
        for parameter, value in look['params'].items():
            environment.set_parameter(parameter=parameter, value=value)
        environment.set_pins(pins=look['pins'])
        batch.commands.image.set_style(style=look['style'])


# ---- Putting the person's own look back -------------------------------------------

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
    return {'environment': document['environment'], 'image_style': document['image_style']}


def restore_look(fs, saved):
    environment = saved['environment']
    params = {}
    for parameter, section, k in PARAMETERS:
        holder = environment.get(section, {}) if section else environment
        if k in holder or k == 'image':
            params[parameter] = holder.get(k)
    send(fs, {'sky': environment.get('sky'), 'params': params, 'pins': environment.get('pins', []),
              'style': saved['image_style']})


def _same(a, b):
    return a is not None and b is not None and json.dumps(a, sort_keys=True) == json.dumps(b, sort_keys=True)


# ---- The plugin ------------------------------------------------------------------

_saved = None      # the person's own look, from before the first studio
_applied = None    # the look as the last studio left it


def merged(values):
    """The kept settings with a panel's values over them, kept again."""
    s = settings()
    for k, v in (values or {}).items():
        if k in DEFAULTS:
            s[k] = v
    plugin.save_settings(s)
    return s


def apply_studio(fs, s):
    global _saved, _applied
    name = s.get('preset') if s.get('preset') in BY_NAME else DEFAULTS['preset']
    studio = BY_NAME[name]
    document = fs.context()['document']
    before = look_of(document)
    if _saved is None or not _same(before, _applied):
        _saved = before
    look = build(fs, studio, s, document)
    send(fs, look)
    _applied = look_of(fs.context()['document'])
    plugin.set_panel('studio', preset=name, about=f'{name}: {studio.about}')
    plugin.status(f'Studio: {name}')
    return studio


@plugin.command('apply')
def apply(fs, values):
    apply_studio(fs, merged(values))


@plugin.command('next')
def next_studio(fs, values):
    s = merged(values)
    current = s.get('preset') if s.get('preset') in BY_NAME else NAMES[-1]
    s['preset'] = NAMES[(NAMES.index(current) + 1) % len(NAMES)]
    plugin.save_settings(s)
    apply_studio(fs, s)


@plugin.command('reset_lights')
def reset_lights(fs, values):
    s = settings()
    s.update({k: DEFAULTS[k] for k in LIGHT_KEYS})
    plugin.save_settings(s)
    plugin.set_panel('lights', status='Lights reset to each studio as designed.', **{k: DEFAULTS[k] for k in LIGHT_KEYS})
    if _saved is not None:
        apply_studio(fs, s)


@plugin.command('reset_set')
def reset_set(fs, values):
    s = settings()
    s.update({k: DEFAULTS[k] for k in SET_KEYS})
    plugin.save_settings(s)
    plugin.set_panel('set', status='Backdrop and floor reset to each studio as designed.', **{k: DEFAULTS[k] for k in SET_KEYS})
    if _saved is not None:
        apply_studio(fs, s)


@plugin.command('restore')
def back(fs, values):
    global _saved, _applied
    if _saved is None:
        plugin.status('No studio to take off: the look is already yours')
        return
    restore_look(fs, _saved)
    _saved = _applied = None
    plugin.set_panel('studio', about='Back to your own look. Pick a studio and click Apply.')
    plugin.status('Back to your own look')


@plugin.on('panel_opened')
def opened(fs, data):
    """Show the kept settings in the panel that opened."""
    panel = data.get('panel')
    keys = PANELS.get(panel)
    if not keys:
        return
    s = settings()
    values = {k: s[k] for k in keys}
    if panel == 'studio' and s.get('preset') in BY_NAME:
        values['about'] = f"{s['preset']}: {BY_NAME[s['preset']].about}"
    plugin.set_panel(panel, **values)
