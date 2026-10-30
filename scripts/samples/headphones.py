"""Over-ear wireless headphones to real proportions, for Tequila.

    blender -b --factory-startup --python headphones.py -- OUT.glb

Built in millimetres, exported in metres. They stand as they rest on a desk:
the cups facing each other along X, sprung in so the cushions almost meet,
the bottoms tilted 6 degrees in; the front is -Y.

About 176 mm wide and 194 mm tall (an AirPods Max is 169 x 187).
- Cups: 96 x 72 mm pebbles of soft-touch, an 8 mm radius rolling the side
  into a nearly flat face, a parting line 3 mm in front of the cushion.
- Cushions: protein leather, 19 mm deep and 3 mm deeper at the back, a
  58 x 38 mm opening; pleated where the leather is tucked into the mount,
  gathered into wrinkles round the inner rim; acoustic cloth set back.
- Hinges: a soft-touch knuckle on each cup's crown; a brushed steel slider
  12 x 1.8 mm with etched size marks rising from it into the band.
- Headband: soft-touch, 32 mm wide at the crown tapering to 16 mm at the
  ends; under the crown a protein leather pad, stitched along both edges.
- Details: two buttons, a USB-C port and an LED on the right cup's lower
  edge, microphone ports, the name printed gloss on matte.
"""
import math
import sys

import bmesh
import bpy
from mathutils import Matrix, Vector

OUT = sys.argv[sys.argv.index('--') + 1]

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene

PALETTE = {
    'Shell': (0.05, 0.05, 0.055), 'Back': (0.04, 0.04, 0.045), 'Cushion': (0.04, 0.04, 0.045),
    'Cloth': (0.03, 0.03, 0.035), 'Slider': (0.7, 0.7, 0.72), 'Headband': (0.05, 0.05, 0.055),
    'Pad': (0.04, 0.04, 0.045), 'Thread': (0.08, 0.08, 0.085), 'Port': (0.01, 0.01, 0.01),
    'Print': (0.12, 0.12, 0.13), 'Marks': (0.2, 0.2, 0.2), 'LED': (0.9, 0.9, 0.9),
}
MATS = {}
for name, colour in PALETTE.items():
    m = bpy.data.materials.new(name)
    m.diffuse_color = (*colour, 1.0)
    MATS[name] = m
OBJECTS = []


def new_object(name, bm, mat, smooth_angle=None):
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    mesh.materials.append(MATS[mat])
    ob = bpy.data.objects.new(name, mesh)
    scene.collection.objects.link(ob)
    if smooth_angle is not None:
        for p in mesh.polygons:
            p.use_smooth = True
        mesh.set_sharp_from_angle(angle=math.radians(smooth_angle))
    OBJECTS.append(ob)
    return ob


def grid(bm, rows, closed_u=True, closed_v=False):
    n = len(rows[0])
    pairs = list(zip(rows, rows[1:])) + ([(rows[-1], rows[0])] if closed_v else [])
    for a, b in pairs:
        for i in range(n if closed_u else n - 1):
            j = (i + 1) % n
            bm.faces.new((a[i], a[j], b[j], b[i]))


def fan(bm, row, centre):
    c = bm.verts.new(centre)
    for i in range(len(row)):
        bm.faces.new((row[i], row[(i + 1) % len(row)], c))


def uv_by(ob, fn):
    """Give `ob` UVs from a function of each vertex's position."""
    uv = ob.data.uv_layers.new(name='UVMap')
    for loop in ob.data.loops:
        uv.data[loop.index].uv = fn(ob.data.vertices[loop.vertex_index].co)


# ---- the cup's outline (in its own frame: Y across, Z up, X out) -------------
A, B = 36.0, 48.0
K = 2.3
N = 200
ts = [2 * math.pi * i / N for i in range(N)]


def outline(t, k=1.0):
    c, s = math.cos(t), math.sin(t)
    return Vector((0.0, A * k * math.copysign(abs(c) ** (2 / K), c), B * k * math.copysign(abs(s) ** (2 / K), s)))


def normal2d(t):
    e = 1e-4
    d = outline(t + e) - outline(t - e)
    n = Vector((0.0, d.z, -d.y)).normalized()
    return n if n.dot(outline(t)) > 0 else -n


FACE_IN = 44.0       # the cushion's face against the head
SEAT = 63.0          # where the cushion meets the cup
SPLIT = 66.0         # the parting line
OUTER = 87.5         # the cup's face, at its centre
HINGE = Vector((75.5, 3.0, B + 3.2))


def face_x(k):
    """How far out the cup's face stands at outline fraction k."""
    return 85.9 + 1.6 * (1 - min(k / 0.895, 1.0) ** 2)


def shell_profile():
    """(k, x) from the parting line round the 8 mm roll to the face's centre."""
    prof = [(0.996, SPLIT + 0.25), (1.0, SPLIT + 1.2), (1.0, 77.5)]
    for i in range(1, 11):
        a = math.pi / 2 * i / 10
        prof.append((1.0 - 0.105 * (1 - math.cos(a)), 77.5 + 8.4 * math.sin(a)))
    for k in (0.84, 0.7, 0.55, 0.4, 0.25, 0.1):
        prof.append((k, face_x(k)))
    return prof


def cup_frame(sx):
    """Tilt the cup's bottom in, about a front-back axis through its hinge."""
    pivot = Vector((sx * HINGE.x, 0, HINGE.z))
    return Matrix.Translation(pivot) @ Matrix.Rotation(sx * math.radians(6), 4, 'Y') @ Matrix.Translation(-pivot)


def place(bm, sx):
    if sx < 0:
        bmesh.ops.scale(bm, vec=Vector((-1, 1, 1)), verts=bm.verts)
        bmesh.ops.reverse_faces(bm, faces=bm.faces)
    bmesh.ops.transform(bm, matrix=cup_frame(sx), verts=bm.verts)
    return bm


def X(x):
    return Vector((x, 0, 0))


for side, sx in (('L', -1), ('R', 1)):
    # Shell: parting line round the roll to the face.
    bm = bmesh.new()
    rows = [[bm.verts.new(outline(t, k) + X(x)) for t in ts] for k, x in shell_profile()]
    grid(bm, rows)
    fan(bm, rows[-1], X(OUTER))
    inner = [bm.verts.new(outline(t, 0.985) + X(SPLIT + 0.25)) for t in ts]
    grid(bm, [inner, rows[0]])
    fan(bm, inner, X(SPLIT + 0.25))
    new_object(f'Shell {side}', place(bm, sx), 'Shell', 30)

    # Back: from the cushion's seat to the parting line, 0.3 mm short of it.
    bm = bmesh.new()
    prof = [(0.93, SEAT - 0.4), (0.975, SEAT - 0.2), (0.995, SEAT + 0.6), (1.0, SEAT + 1.5), (0.997, SPLIT - 0.05)]
    rows = [[bm.verts.new(outline(t, k) + X(x)) for t in ts] for k, x in prof]
    rows.append([bm.verts.new(outline(t, 0.97) + X(SPLIT - 0.05)) for t in ts])
    grid(bm, rows, closed_v=True)
    new_object(f'Back {side}', place(bm, sx), 'Back', 30)

    # Cushion.
    bm = bmesh.new()
    rows = []
    M = 44
    for i in range(M):
        a = 2 * math.pi * i / M
        c, s = math.cos(a), math.sin(a)
        u = 0.81 + 0.18 * math.copysign(abs(c) ** 0.5, c)      # across the ring
        depth_t = math.copysign(abs(s) ** 0.6, s)                 # -1 at the face, +1 at the seat
        row = []
        for t in ts:
            o = outline(t, u)
            face = FACE_IN - 1.5 * (o.y / A)                        # deeper at the back (+Y)
            x = (face + SEAT) / 2 + (SEAT - face) / 2 * depth_t
            n = normal2d(t)
            inner_wall = max(0.0, -c) * max(0.0, 0.4 - s)
            seat_edge = max(0.0, c) * max(0.0, s) ** 3
            wr = 0.45 * inner_wall * math.sin(t * 42 + 1.9 * math.sin(t * 5) + 0.7 * math.sin(t * 17 + 2.0))
            pl = 0.32 * seat_edge * math.sin(t * 64 + 1.3 * math.sin(t * 9))
            row.append(bm.verts.new(o + X(x) + n * (wr + pl)))
        rows.append(row)
    grid(bm, rows, closed_v=True)
    new_object(f'Cushion {side}', place(bm, sx), 'Cushion', 75)

    # Cloth, set back.
    bm = bmesh.new()
    row = [bm.verts.new(outline(t, 0.66) + X(SEAT - 2.5)) for t in ts]
    fan(bm, row, X(SEAT - 2.5))
    new_object(f'Cloth {side}', place(bm, sx), 'Cloth', 80)

    # Hinge knuckle on the crown: a capsule along Y.
    bm = bmesh.new()
    kn = bmesh.ops.create_cone(bm, cap_ends=False, segments=48, radius1=5.2, radius2=5.2, depth=14.0)
    bmesh.ops.rotate(bm, cent=Vector(), matrix=Matrix.Rotation(math.pi / 2, 3, 'X'), verts=kn['verts'])
    bmesh.ops.translate(bm, vec=HINGE, verts=kn['verts'])
    for end in (-1, 1):
        sp = bmesh.ops.create_uvsphere(bm, u_segments=48, v_segments=24, radius=5.2)
        bmesh.ops.scale(bm, vec=Vector((1, 0.45, 1)), verts=sp['verts'])
        bmesh.ops.translate(bm, vec=HINGE + Vector((0, end * 7.0, 0)), verts=sp['verts'])
    new_object(f'Knuckle {side}', place(bm, sx), 'Shell', 40)

# ---- sliders -----------------------------------------------------------------
BAND_X, BAND_Z, CROWN = 80.0, 72.0, 146.0
for side, sx in (('L', -1), ('R', 1)):
    bm = bmesh.new()
    start = Vector((sx * HINGE.x, HINGE.y, HINGE.z + 4.2))
    end = Vector((sx * BAND_X, HINGE.y, BAND_Z + 8.0))
    pts = []
    for i in range(25):
        f = i / 24
        p = start.lerp(end, f)
        p.x += sx * 1.5 * math.sin(math.pi * f)
        pts.append(p)
    rows = []
    for i, p in enumerate(pts):
        tangent = (pts[min(i + 1, 24)] - pts[max(i - 1, 0)]).normalized()
        across = Vector((0, 1, 0))
        thick = tangent.cross(across).normalized()
        row = []
        for j in range(20):
            a = 2 * math.pi * j / 20
            c, s = math.cos(a), math.sin(a)
            row.append(bm.verts.new(p + across * 6.0 * math.copysign(abs(c) ** 0.2, c) + thick * 0.9 * math.copysign(abs(s) ** 0.5, s)))
        rows.append(row)
    grid(bm, rows)
    bm.faces.new(rows[0][::-1])
    bm.faces.new(rows[-1])
    ob = new_object(f'Slider {side}', bm, 'Slider', 30)
    uv_by(ob, lambda co: (co.z / 30.0, co.y / 12.0))        # brushed along its length
    marks = bmesh.new()
    for i in range(7):
        f = 0.25 + i * 0.075
        p = start.lerp(end, f) + Vector((sx * (1.5 * math.sin(math.pi * f) + 0.93), 0, 0))
        r = bmesh.ops.create_cube(marks, size=1.0)
        bmesh.ops.scale(marks, vec=Vector((0.04, 5.0 if i % 3 == 0 else 2.6, 0.22)), verts=r['verts'])
        bmesh.ops.translate(marks, vec=p + Vector((0, -2.0 if i % 3 else -0.8, 0)), verts=r['verts'])
    new_object(f'Marks {side}', marks, 'Marks', 20)


# ---- headband ----------------------------------------------------------------
def arch(phi):
    return Vector((BAND_X * math.cos(phi), HINGE.y, BAND_Z + (CROWN - BAND_Z) * math.sin(phi)))


def arch_frame(phi):
    p = arch(phi)
    tangent = (arch(phi + 1e-3) - arch(phi - 1e-3)).normalized()
    out = tangent.cross(Vector((0, 1, 0))).normalized()
    if out.dot(p - Vector((0, HINGE.y, BAND_Z))) < 0:
        out = -out
    return p, tangent, out


bm = bmesh.new()
rows = []
SEGS = 160
for i in range(SEGS + 1):
    phi = math.pi * i / SEGS
    p, tangent, out = arch_frame(phi)
    crown = math.sin(phi) ** 1.4
    half_w = 8.0 + 8.0 * crown
    half_t = 2.6 + 0.7 * crown
    end = max(0.0, 1 - phi / 0.18, 1 - (math.pi - phi) / 0.18)      # swelling where the sliders go in
    half_t += 0.9 * end
    half_w += 1.2 * end
    rows.append([bm.verts.new(p + Vector((0, 1, 0)) * half_w * math.copysign(abs(math.cos(a)) ** 0.3, math.cos(a))
                              + out * half_t * math.copysign(abs(math.sin(a)) ** 0.5, math.sin(a)))
                 for a in (2 * math.pi * j / 36 for j in range(36))])
# Rounded end caps.
for idx, nxt in ((0, 1), (-1, -2)):
    ring = rows[idx]
    centre = sum((v.co for v in ring), Vector()) / len(ring)
    away = (centre - sum((v.co for v in rows[nxt]), Vector()) / len(ring)).normalized()
    caps = [[bm.verts.new(centre + (v.co - centre) * shrink + away * step) for v in ring]
            for shrink, step in ((0.9, 0.9), (0.62, 1.7), (0.25, 2.1))]
    rows = (caps[::-1] + rows) if idx == 0 else (rows + caps)
grid(bm, rows)
for row in (rows[0], rows[-1]):
    fan(bm, row, sum((v.co for v in row), Vector()) / len(row))
new_object('Headband', bm, 'Headband', 40)

# The pad under the crown, stitched along both edges.
bm = bmesh.new()
rows = []
seams = ([], [])
for i in range(101):
    phi = math.radians(38 + 104 * i / 100)
    p, tangent, out = arch_frame(phi)
    swell = math.sin(math.pi * i / 100) ** 0.3
    thick = 5.2 * swell + 0.4
    hw = 11.5
    rows.append([bm.verts.new(p - out * (3.0 + thick * (0.5 - 0.5 * math.sin(a)))
                              + Vector((0, 1, 0)) * hw * math.copysign(abs(math.cos(a)) ** 0.4, math.cos(a)))
                 for a in (2 * math.pi * j / 28 for j in range(28))])
    for k, y in enumerate((-hw + 1.4, hw - 1.4)):
        seams[k].append((p - out * (3.0 + thick * 0.97) + Vector((0, y, 0)), tangent, -out))
grid(bm, rows)
bm.faces.new(rows[0][::-1])
bm.faces.new(rows[-1])
new_object('Pad', bm, 'Pad', 50)
bm = bmesh.new()
for seam in seams:
    for q, tangent, normal in seam[3:-3]:
        sp = bmesh.ops.create_uvsphere(bm, u_segments=8, v_segments=6, radius=0.26)
        m = Matrix.Identity(3)
        m.col[0], m.col[1], m.col[2] = tangent * 2.5, Vector((0, 1, 0)), normal
        bmesh.ops.transform(bm, matrix=m.to_4x4(), verts=sp['verts'])
        bmesh.ops.translate(bm, vec=q, verts=sp['verts'])
new_object('Stitching', bm, 'Thread', 60)

# ---- details on the cups -------------------------------------------------------
bm = bmesh.new()
for t_deg, length in ((-118, 7.0), (-134, 5.0)):
    t = math.radians(t_deg)
    n = normal2d(t)
    centre = outline(t) + X(72.0) + n * 0.35
    r = bmesh.ops.create_cone(bm, cap_ends=True, segments=24, radius1=1.6, radius2=1.6, depth=0.9)
    bmesh.ops.scale(bm, vec=Vector((length / 3.2, 1, 1)), verts=r['verts'])
    rot = Vector((0, 0, 1)).rotation_difference(n).to_matrix().to_4x4()
    bmesh.ops.transform(bm, matrix=rot, verts=r['verts'])
    bmesh.ops.translate(bm, vec=centre, verts=r['verts'])
new_object('Buttons', place(bm, 1), 'Shell', 35)
bot = outline(-math.pi / 2)
bm = bmesh.new()
r = bmesh.ops.create_cube(bm, size=1.0)
bmesh.ops.scale(bm, vec=Vector((8.6, 2.6, 0.4)), verts=r['verts'])
bmesh.ops.translate(bm, vec=bot + Vector((71.0, 4.0, 0.05)), verts=r['verts'])
new_object('Port', place(bm, 1), 'Port', 30)
bm = bmesh.new()
d = bmesh.ops.create_circle(bm, cap_ends=True, segments=16, radius=0.55)
bmesh.ops.translate(bm, vec=bot + Vector((71.0, -6.0, -0.06)), verts=d['verts'])
new_object('LED', place(bm, 1), 'LED', 30)
for side, sx in (('L', -1), ('R', 1)):
    bm = bmesh.new()
    for (y, z) in ((-24.0, 30.0), (-22.6, 31.4), (20.0, -34.0), (-28.0, -12.0)):
        k = math.hypot(y / A, z / B)
        d = bmesh.ops.create_circle(bm, cap_ends=True, segments=12, radius=0.38)
        bmesh.ops.rotate(bm, cent=Vector(), matrix=Matrix.Rotation(math.pi / 2, 3, 'Y'), verts=d['verts'])
        bmesh.ops.translate(bm, vec=Vector((face_x(k) + 0.03, y, z)), verts=d['verts'])
    new_object(f'Microphones {side}', place(bm, sx), 'Port', 20)


def text_on_face(body, size, spacing, sx):
    curve = bpy.data.curves.new('T', 'FONT')
    curve.body = body
    curve.size = size
    curve.align_x = 'CENTER'
    curve.align_y = 'CENTER'
    curve.space_character = spacing
    curve.extrude = 0.015
    ob = bpy.data.objects.new('T', curve)
    scene.collection.objects.link(ob)
    bpy.context.view_layer.update()
    mesh = bpy.data.meshes.new_from_object(ob.evaluated_get(bpy.context.evaluated_depsgraph_get()))
    bpy.data.objects.remove(ob)
    tb = bmesh.new()
    tb.from_mesh(mesh)
    bpy.data.meshes.remove(mesh)
    for v in tb.verts:
        # Seen from outside the right cup, +Y is the reader's right.
        ty, tz, tx = v.co.x, v.co.y, v.co.z
        k = math.hypot(ty / A, tz / B)
        v.co = Vector((face_x(k) + 0.02 + tx, ty, tz))
    if sx < 0:
        # Seen from outside the left cup the reader's right is -Y: flip the
        # letters across before the cup's own mirror.
        bmesh.ops.scale(tb, vec=Vector((1, -1, 1)), verts=tb.verts)
        bmesh.ops.reverse_faces(tb, faces=tb.faces)
    return place(tb, sx)


new_object('Name R', text_on_face('OVRA', 6.0, 1.7, 1), 'Print', 20)
new_object('Name L', text_on_face('OVRA', 6.0, 1.7, -1), 'Print', 20)

pts_all = [(ob.matrix_world @ v.co) for ob in OBJECTS for v in ob.data.vertices]
lowest = min(p.z for p in pts_all)
print(f'{max(p.x for p in pts_all) - min(p.x for p in pts_all):.0f} mm wide, '
      f'{max(p.z for p in pts_all) - lowest:.0f} mm tall, {max(p.y for p in pts_all) - min(p.y for p in pts_all):.0f} mm deep')
for ob in OBJECTS:
    ob.data.transform(Matrix.Translation((0, 0, -lowest)))
    ob.data.transform(Matrix.Scale(0.001, 4))
bpy.ops.export_scene.gltf(filepath=OUT, export_format='GLB', export_yup=True, export_normals=True,
                          export_materials='EXPORT')
print('wrote', OUT)
