"""Models for the M6 reference renders (docs/05, M6 exit criterion).

Writes a spirits bottle with liquid and a solitaire ring as OBJ + MTL, in
metres, with nothing but the standard library, so the renders can be
reproduced from this file alone:

    python scripts/reference/models.py OUT_DIR

The shapes are built the way they are made: the bottle is turned (a profile
revolved), with glass walls of real thickness and a liquid that overlaps the
inner wall by half a millimetre, which is how a renderer with nested
dielectrics expects a filled container (the liquid's priority wins where the
two overlap). The stone is a round brilliant: the convex hull of its table,
crown, girdle and pavilion points, with flat facets.
"""

import math
import os
import sys
from itertools import combinations


# --- meshes -----------------------------------------------------------------

class Mesh:
    def __init__(self, name, material):
        self.name, self.material = name, material
        self.positions, self.faces = [], []
        # Per-face normals for faceted parts; None lets the importer decide.
        self.face_normals = None

    def add(self, p):
        self.positions.append(p)
        return len(self.positions) - 1

    def signed_volume(self):
        v = 0.0
        for f in self.faces:
            a = self.positions[f[0]]
            for i in range(1, len(f) - 1):
                b, c = self.positions[f[i]], self.positions[f[i + 1]]
                v += (a[0] * (b[1] * c[2] - b[2] * c[1])
                      - a[1] * (b[0] * c[2] - b[2] * c[0])
                      + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
        return v

    def outward(self):
        """Wind every face so its normal points out of the solid."""
        if self.signed_volume() < 0:
            self.faces = [list(reversed(f)) for f in self.faces]
        return self

    def transformed(self, fn):
        self.positions = [fn(p) for p in self.positions]
        return self


def lathe(name, material, profile, segments, axis_closed=True):
    """Revolve `profile`, a list of (radius, height), around +Y. Points on the
    axis (radius 0) become a single vertex; a profile that starts and ends on
    the axis is a closed solid, and one that does not is closed by wrapping
    (a torus) when `axis_closed` is False."""
    mesh = Mesh(name, material)
    rings = []
    for r, y in profile:
        if r <= 1e-9:
            rings.append([mesh.add((0.0, y, 0.0))])
        else:
            rings.append([
                mesh.add((r * math.cos(2 * math.pi * j / segments), y,
                          r * math.sin(2 * math.pi * j / segments)))
                for j in range(segments)
            ])
    pairs = list(zip(rings, rings[1:]))
    if not axis_closed:
        pairs.append((rings[-1], rings[0]))
    for a, b in pairs:
        for j in range(segments):
            k = (j + 1) % segments
            if len(a) == 1 and len(b) == 1:
                continue
            if len(a) == 1:
                mesh.faces.append([a[0], b[k], b[j]])
            elif len(b) == 1:
                mesh.faces.append([a[j], a[k], b[0]])
            else:
                mesh.faces.append([a[j], a[k], b[k], b[j]])
    return mesh.outward()


def arc(cx, cy, radius, start_deg, end_deg, steps):
    return [(cx + radius * math.cos(math.radians(start_deg + (end_deg - start_deg) * i / steps)),
             cy + radius * math.sin(math.radians(start_deg + (end_deg - start_deg) * i / steps)))
            for i in range(steps + 1)]


def blend(r0, y0, r1, y1, steps):
    """A shoulder: vertical at both ends, a cosine in between."""
    return [(r1 + (r0 - r1) * (1 + math.cos(math.pi * t)) / 2, y0 + (y1 - y0) * t)
            for t in (i / steps for i in range(steps + 1))]


# --- the bottle -------------------------------------------------------------

WALL = 0.003
BODY_R = 0.034
NECK_R = 0.013


def bottle():
    outer = (
        [(0.0, 0.004)]
        + [(0.022 * t, 0.004 - 0.0035 * t * t) for t in (i / 6 for i in range(1, 7))]
        + arc(0.028, 0.006, 0.006, 270, 360, 8)[0:]           # the heel
        + [(BODY_R, 0.150)]
        + blend(BODY_R, 0.150, NECK_R, 0.205, 18)[1:]         # the shoulder
        + [(NECK_R, 0.226), (0.0148, 0.2275)]                  # the collar
        + arc(0.0148 - 0.0012, 0.2300, 0.0012, 0, 90, 5)[1:]  # lip, rounded
        + [(0.0098, 0.2312)]
    )
    inner_r = BODY_R - WALL
    inner = (
        [(0.0098, 0.2300), (0.0098, 0.206)]
        + blend(NECK_R - WALL - 0.0002, 0.206, inner_r, 0.153, 18)[1:]
        + [(inner_r, 0.016)]
        + arc(0.025, 0.016, 0.006, 0, -90, 8)[1:]
        + [(0.0, 0.010)]
    )
    glass = lathe("Bottle", "Glass", outer + inner, 128)

    # The liquid: half a millimetre into the glass all round, and a meniscus
    # climbing the last few millimetres of the wall.
    fill = 0.118
    liquid_r = inner_r + 0.0005
    top = [(liquid_r - 0.0045 * (1 - u), fill + 0.0016 * u * u)
           for u in (i / 8 for i in range(8, -1, -1))]
    liquid_profile = (
        [(0.0, 0.0095)]
        + [(0.025, 0.0095)]
        + arc(0.025, 0.016, 0.0065, -90, 0, 8)[1:]
        + top
        + [(0.0, fill)]
    )
    liquid = lathe("Liquid", "Liquid", liquid_profile, 128)

    # A turned wooden stopper head, sitting on the lip.
    head = (
        [(0.0, 0.2312), (0.0170, 0.2312)]
        + arc(0.0170, 0.2342, 0.003, 270, 360, 5)[1:]
        + [(0.0200, 0.2500)]
        + arc(0.0170, 0.2500, 0.003, 0, 90, 6)[1:]
        + [(0.0, 0.2530)]
    )
    stopper = lathe("Stopper", "Wood", head, 96)
    return [glass, liquid, stopper]


# --- the ring ---------------------------------------------------------------

def convex_hull(points, eps=1e-10):
    """Faces of the convex hull, each a polygon of point indices wound
    outward, with its normal. Brute force over triples: fine for a gem's
    sixty-odd points, and it merges coplanar points into one facet."""
    def sub(a, b): return (a[0] - b[0], a[1] - b[1], a[2] - b[2])
    def cross(a, b): return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])
    def dot(a, b): return a[0] * b[0] + a[1] * b[1] + a[2] * b[2]

    planes = {}
    for i, j, k in combinations(range(len(points)), 3):
        n = cross(sub(points[j], points[i]), sub(points[k], points[i]))
        length = math.sqrt(dot(n, n))
        if length < 1e-14:
            continue
        n = (n[0] / length, n[1] / length, n[2] / length)
        d = dot(n, points[i])
        side = [dot(n, p) - d for p in points]
        if all(s <= eps for s in side):
            pass
        elif all(s >= -eps for s in side):
            n, d = (-n[0], -n[1], -n[2]), -d
        else:
            continue
        key = (round(n[0], 6), round(n[1], 6), round(n[2], 6), round(d, 9))
        planes.setdefault(key, (n, d))

    faces = []
    for n, d in planes.values():
        on = [i for i, p in enumerate(points) if abs(dot(n, p) - d) < 1e-9]
        # Order around the facet's centre, counter-clockwise seen from outside.
        c = [sum(points[i][a] for i in on) / len(on) for a in range(3)]
        u = sub(points[on[0]], c)
        ul = math.sqrt(dot(u, u))
        u = (u[0] / ul, u[1] / ul, u[2] / ul)
        v = cross(n, u)
        on.sort(key=lambda i: math.atan2(dot(sub(points[i], c), v), dot(sub(points[i], c), u)))
        faces.append((on, n))
    return faces


def brilliant(radius, girdle_y):
    """A round brilliant, table up, girdle centred at `girdle_y`."""
    table = 0.57 * radius          # half the table's width
    crown = math.tan(math.radians(34.5)) * (radius - table)
    girdle = 0.02 * radius
    pavilion = math.tan(math.radians(40.75)) * radius
    points = []
    for k in range(8):                             # the table
        a = math.radians(45 * k)
        points.append((table * math.cos(a), girdle / 2 + crown, table * math.sin(a)))
    for k in range(8):                             # star points, crown middle
        a = math.radians(45 * k + 22.5)
        r = 0.78 * radius
        points.append((r * math.cos(a), girdle / 2 + crown * (radius - r) / (radius - table) + 0.06 * crown,
                       r * math.sin(a)))
    for k in range(32):                            # the girdle, top and bottom
        a = math.radians(11.25 * k)
        for y in (girdle / 2, -girdle / 2):
            points.append((radius * math.cos(a), y, radius * math.sin(a)))
    for k in range(8):                             # lower girdle points
        a = math.radians(45 * k + 22.5)
        r = 0.55 * radius
        points.append((r * math.cos(a), -girdle / 2 - (radius - r) * math.tan(math.radians(42.5)),
                       r * math.sin(a)))
    points.append((0.0, -girdle / 2 - pavilion, 0.0))   # the culet
    points = [(x, y + girdle_y, z) for x, y, z in points]

    mesh = Mesh("Diamond", "Diamond")
    mesh.positions = points
    mesh.face_normals = []
    for polygon, n in convex_hull(points):
        mesh.faces.append(polygon)
        mesh.face_normals.append(n)
    return mesh


def capsule(name, material, a, b, ra, rb, segments=20, cap_steps=6):
    """A tapered rod from `a` to `b`, rounded at both ends."""
    axis = [b[i] - a[i] for i in range(3)]
    length = math.sqrt(sum(c * c for c in axis))
    profile = (
        arc(0.0, 0.0, ra, 270, 360, cap_steps)
        + arc(0.0, length, rb, 0, 90, cap_steps)
    )
    profile = [(max(r, 0.0), y) for r, y in profile]
    profile[0] = (0.0, -ra)
    profile[-1] = (0.0, length + rb)
    mesh = lathe(name, material, profile, segments)
    # Rotate +Y onto the axis, then move to `a`.
    d = [c / length for c in axis]
    up = (0.0, 1.0, 0.0)
    v = (up[1] * d[2] - up[2] * d[1], up[2] * d[0] - up[0] * d[2], up[0] * d[1] - up[1] * d[0])
    s = math.sqrt(sum(c * c for c in v))
    c = d[1]
    if s < 1e-12:
        rot = lambda p: p if c > 0 else (p[0], -p[1], -p[2])
    else:
        k = [x / s for x in v]
        def rot(p):
            kxp = (k[1] * p[2] - k[2] * p[1], k[2] * p[0] - k[0] * p[2], k[0] * p[1] - k[1] * p[0])
            kdp = sum(k[i] * p[i] for i in range(3))
            return tuple(p[i] * c + kxp[i] * s + k[i] * kdp * (1 - c) for i in range(3))
    return mesh.transformed(lambda p: tuple(rot(p)[i] + a[i] for i in range(3)))


def ring():
    # The band, turned about +Y and stood up about +Z: a comfort-fit shank,
    # domed outside and in, 2.4 mm wide and 1.7 mm deep.
    r0, a, b = 0.0094, 0.00085, 0.0012
    loop = []
    for i in range(64):
        t = 2 * math.pi * i / 64
        cx, cy = math.cos(t), math.sin(t)
        e = 2.0 / 2.6
        loop.append((r0 + a * math.copysign(abs(cx) ** e, cx), b * math.copysign(abs(cy) ** e, cy)))
    band = lathe("Band", "Gold", loop, 192, axis_closed=False)
    band.transformed(lambda p: (p[0], -p[2], p[1]))   # axis now along Z

    top = r0 + a                                      # the band's crown
    girdle_y = top + 0.0033
    stone_r = 0.00325
    parts = [band, brilliant(stone_r, girdle_y)]

    # Four prongs from the shoulder of the band, over the girdle, and a
    # gallery wire round the pavilion.
    for k in range(4):
        az = math.radians(45 + 90 * k)
        base = (0.0021 * math.cos(az), top - 0.0004, 0.0021 * math.sin(az))
        tip = ((stone_r + 0.00005) * math.cos(az), girdle_y + 0.00045, (stone_r + 0.00005) * math.sin(az))
        parts.append(capsule(f"Prong {k + 1}", "Gold", base, tip, 0.00042, 0.00036))
    wire = []
    for i in range(24):
        t = 2 * math.pi * i / 24
        wire.append((0.0024 + 0.00028 * math.cos(t), top + 0.0012 + 0.00028 * math.sin(t)))
    parts.append(lathe("Gallery", "Gold", wire, 96, axis_closed=False))
    return parts


# --- writing ----------------------------------------------------------------

MATERIALS = {
    "Glass": (0.95, 0.95, 0.95),
    "Liquid": (0.75, 0.38, 0.08),
    "Wood": (0.45, 0.28, 0.15),
    "Gold": (1.0, 0.78, 0.34),
    "Diamond": (1.0, 1.0, 1.0),
}


def write(path, meshes):
    library = os.path.splitext(os.path.basename(path))[0] + ".mtl"
    used = []
    with open(path, "w", newline="\n") as out:
        out.write(f"# Tequila M6 reference model, from scripts/reference/models.py\nmtllib {library}\n")
        base, nbase = 1, 1
        for mesh in meshes:
            out.write(f"o {mesh.name}\nusemtl {mesh.material}\n")
            if mesh.material not in used:
                used.append(mesh.material)
            for x, y, z in mesh.positions:
                out.write(f"v {x:.7f} {y:.7f} {z:.7f}\n")
            if mesh.face_normals:
                for x, y, z in mesh.face_normals:
                    out.write(f"vn {x:.7f} {y:.7f} {z:.7f}\n")
                for i, f in enumerate(mesh.faces):
                    out.write("f " + " ".join(f"{base + v}//{nbase + i}" for v in f) + "\n")
                nbase += len(mesh.face_normals)
            else:
                for f in mesh.faces:
                    out.write("f " + " ".join(str(base + v) for v in f) + "\n")
            base += len(mesh.positions)
    with open(os.path.join(os.path.dirname(path), library), "w", newline="\n") as out:
        for name in used:
            r, g, b = MATERIALS[name]
            out.write(f"newmtl {name}\nKd {r} {g} {b}\n\n")


if __name__ == "__main__":
    out_dir = sys.argv[1] if len(sys.argv) > 1 else "."
    os.makedirs(out_dir, exist_ok=True)
    write(os.path.join(out_dir, "bottle.obj"), bottle())
    write(os.path.join(out_dir, "ring.obj"), ring())
    print("wrote bottle.obj and ring.obj to", out_dir)
