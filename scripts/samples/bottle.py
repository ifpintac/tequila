"""A 50 ml luxury perfume bottle to real measurements, for Tequila.

    blender -b --factory-startup --python bottle.py -- OUT.glb [--exploded]

Built in millimetres, exported in metres, standing on z = 0 (glTF's y).

- Glass: a polished block 56 x 34 x 80 mm, 3.5 mm radii, with a 15 mm thick
  base, 6.5-7 mm walls and a 10 mm shoulder - heavy flint glass, the way a
  luxury bottle is. The inside is a domed 42 x 21 mm cavity; an FEA 15
  neck rises 6 mm with an 8 mm bore. One watertight solid.
- Liquid: filled to 82 %, with the meniscus a real liquid pulls up the walls
  (1.1 mm), and 0.15 mm into the glass all round, so Tequila's medium stack
  puts the glass-to-liquid boundary exactly on the cavity's wall.
- Hardware: a crimped zamak collar (18 mm, two fine grooves), the spray
  actuator with its orifice, a 2.4 mm polyethylene dip tube curving to the
  bottom corner, and a 30 mm block cap.
- Print: the brand screen-printed on the front, in ink 0.03 mm thick.
"""
import math
import sys

import bmesh
import bpy
from mathutils import Matrix, Vector

argv = sys.argv[sys.argv.index('--') + 1:]
OUT = argv[0]
EXPLODED = '--exploded' in argv

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene


def material(name, color):
    m = bpy.data.materials.new(name)
    m.diffuse_color = (*color, 1.0)
    return m


MATS = {name: material(name, c) for name, c in (
    ('Glass', (0.9, 0.95, 0.95)), ('Liquid', (0.85, 0.45, 0.12)), ('Collar', (0.9, 0.75, 0.4)),
    ('Actuator', (0.9, 0.75, 0.4)), ('Cap', (0.9, 0.75, 0.4)), ('Dip tube', (0.95, 0.95, 0.95)),
    ('Print', (0.92, 0.86, 0.72)))}


def link(mesh, name, mat, smooth_angle=None):
    mesh.materials.clear()
    mesh.materials.append(MATS[mat])
    ob = bpy.data.objects.new(name, mesh)
    scene.collection.objects.link(ob)
    if smooth_angle is not None:
        for p in mesh.polygons:
            p.use_smooth = True
        mesh.set_sharp_from_angle(angle=math.radians(smooth_angle))
    return ob


def bake(ob):
    """Apply the modifier stack."""
    depsgraph = bpy.context.evaluated_depsgraph_get()
    mesh = bpy.data.meshes.new_from_object(ob.evaluated_get(depsgraph))
    old = ob.data
    ob.modifiers.clear()
    ob.data = mesh
    if old.users == 0:
        bpy.data.meshes.remove(old)
    return ob


def rounded_box(name, size, radius, centre, segments=12):
    bm = bmesh.new()
    bmesh.ops.create_cube(bm, size=1.0)
    bmesh.ops.scale(bm, vec=Vector(size), verts=bm.verts)
    bmesh.ops.translate(bm, vec=Vector(centre), verts=bm.verts)
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    ob = bpy.data.objects.new(name, mesh)
    scene.collection.objects.link(ob)
    bevel = ob.modifiers.new('Round', 'BEVEL')
    bevel.width = radius
    bevel.segments = segments
    bevel.limit_method = 'NONE'
    bevel.profile = 0.5
    return bake(ob)


def cylinder(name, radius, z0, z1, segments=128, x=0.0, y=0.0):
    bm = bmesh.new()
    bmesh.ops.create_cone(bm, cap_ends=True, segments=segments, radius1=radius, radius2=radius, depth=z1 - z0)
    bmesh.ops.translate(bm, vec=Vector((x, y, (z0 + z1) / 2)), verts=bm.verts)
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    ob = bpy.data.objects.new(name, mesh)
    scene.collection.objects.link(ob)
    return ob


def boolean(target, tool, op):
    mod = target.modifiers.new(op, 'BOOLEAN')
    mod.operation = op
    mod.solver = 'EXACT'
    mod.object = tool
    bake(target)
    bpy.data.objects.remove(tool)
    return target


def revolve(name, profile, segments=192):
    """A solid of revolution round Z from a closed (r, z) profile."""
    bm = bmesh.new()
    loops = []
    for j in range(segments):
        a = 2 * math.pi * j / segments
        c, s = math.cos(a), math.sin(a)
        loops.append([bm.verts.new((r * c, r * s, z)) for r, z in profile])
    n = len(profile)
    for j in range(segments):
        a, b = loops[j], loops[(j + 1) % segments]
        for i in range(n):
            bm.faces.new((a[i], b[i], b[(i + 1) % n], a[(i + 1) % n]))
    bmesh.ops.remove_doubles(bm, verts=bm.verts, dist=1e-5)
    bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
    mesh = bpy.data.meshes.new(name)
    bm.to_mesh(mesh)
    bm.free()
    return mesh


def arc(cx, cz, r, a0, a1, n=10):
    return [(cx + r * math.cos(a0 + (a1 - a0) * i / n), cz + r * math.sin(a0 + (a1 - a0) * i / n)) for i in range(n + 1)]


# ---- glass --------------------------------------------------------------
W, DEPTH, H = 56.0, 34.0, 80.0
CAV_W, CAV_D, CAV_Z0, CAV_Z1 = 42.0, 21.0, 15.0, 70.0
NECK_R, NECK_TOP, BORE_R = 7.5, 86.0, 4.0

glass = rounded_box('Glass', (W, DEPTH, H), 3.5, (0, 0, H / 2), segments=14)
# The neck, with a small fillet down to the shoulder and the crimp bead at
# the top (FEA 15), turned as one profile.
neck = revolve('Neck', [(0.0, H - 1.0), (NECK_R + 1.2, H - 1.0), *arc(NECK_R + 1.2, H + 1.2, 1.2, -math.pi / 2, -math.pi, 6)[1:],
                        (NECK_R, NECK_TOP - 1.6), (NECK_R + 0.35, NECK_TOP - 1.2), (NECK_R + 0.35, NECK_TOP - 0.5),
                        *arc(NECK_R - 0.15, NECK_TOP - 0.5, 0.5, 0.0, math.pi / 2, 5)[1:], (0.0, NECK_TOP)])
neck_ob = bpy.data.objects.new('Neck', neck)
scene.collection.objects.link(neck_ob)
boolean(glass, neck_ob, 'UNION')
cavity = rounded_box('Cavity', (CAV_W, CAV_D, CAV_Z1 - CAV_Z0), 6.5, (0, 0, (CAV_Z0 + CAV_Z1) / 2), segments=16)
bore = cylinder('Bore', BORE_R, CAV_Z1 - 3.0, NECK_TOP + 1.0, segments=96)
boolean(cavity, bore, 'UNION')
boolean(glass, cavity, 'DIFFERENCE')
linked = link(glass.data, 'Glass', 'Glass', smooth_angle=35)
bpy.data.objects.remove(glass)
glass = linked
glass.name = 'Glass'


# ---- liquid -------------------------------------------------------------
FILL = CAV_Z0 + 0.82 * (CAV_Z1 - CAV_Z0)
OVERLAP = 0.15


def cavity_sdf(x, y):
    """Signed distance to the cavity's outline in plan (negative inside)."""
    r = 6.5
    qx, qy = abs(x) - (CAV_W / 2 - r), abs(y) - (CAV_D / 2 - r)
    outside = math.hypot(max(qx, 0), max(qy, 0))
    return outside + min(max(qx, qy), 0) - r


liquid_tool = rounded_box('LiquidVolume', (CAV_W + 2 * OVERLAP, CAV_D + 2 * OVERLAP, CAV_Z1 - CAV_Z0 + 2 * OVERLAP),
                          6.5 + OVERLAP, (0, 0, (CAV_Z0 + CAV_Z1) / 2), segments=16)
# The surface: a fine grid lifted where it meets the wall, closed into a
# block below; what of the cavity lies under it is the liquid.
nx, ny = 168, 84
bm = bmesh.new()
grid = {}
for i in range(nx + 1):
    for j in range(ny + 1):
        x = -CAV_W / 2 - 1 + (CAV_W + 2) * i / nx
        y = -CAV_D / 2 - 1 + (CAV_D + 2) * j / ny
        d = max(-cavity_sdf(x, y), 0.0)
        grid[i, j] = bm.verts.new((x, y, FILL + 1.1 * math.exp(-d / 0.85)))
for i in range(nx):
    for j in range(ny):
        bm.faces.new((grid[i, j], grid[i + 1, j], grid[i + 1, j + 1], grid[i, j + 1]))
edges = [e for e in bm.edges if e.is_boundary]
ext = bmesh.ops.extrude_edge_only(bm, edges=edges)
low = [v for v in ext['geom'] if isinstance(v, bmesh.types.BMVert)]
for v in low:
    v.co.z = CAV_Z0 - 5
bmesh.ops.contextual_create(bm, geom=[e for e in bm.edges if e.is_boundary])
bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
mesh = bpy.data.meshes.new('Surface')
bm.to_mesh(mesh)
bm.free()
surface = bpy.data.objects.new('Surface', mesh)
scene.collection.objects.link(surface)
boolean(liquid_tool, surface, 'INTERSECT')
link(liquid_tool.data, 'Liquid', 'Liquid', smooth_angle=30)
bpy.data.objects.remove(liquid_tool)


# ---- collar, actuator, dip tube, cap ------------------------------------
COLLAR_R, COLLAR_Z0, COLLAR_Z1 = 9.0, H + 0.3, H + 8.8
collar_profile = [(NECK_R + 0.05, COLLAR_Z0), (COLLAR_R - 0.6, COLLAR_Z0), *arc(COLLAR_R - 0.6, COLLAR_Z0 + 0.6, 0.6, -math.pi / 2, 0, 6)[1:],
                  (COLLAR_R, COLLAR_Z0 + 2.4), (COLLAR_R - 0.18, COLLAR_Z0 + 2.6), (COLLAR_R - 0.18, COLLAR_Z0 + 2.9), (COLLAR_R, COLLAR_Z0 + 3.1),
                  (COLLAR_R, COLLAR_Z0 + 4.4), (COLLAR_R - 0.18, COLLAR_Z0 + 4.6), (COLLAR_R - 0.18, COLLAR_Z0 + 4.9), (COLLAR_R, COLLAR_Z0 + 5.1),
                  (COLLAR_R, COLLAR_Z1 - 1.0), *arc(COLLAR_R - 1.0, COLLAR_Z1 - 1.0, 1.0, 0, math.pi / 2, 8)[1:],
                  (2.9, COLLAR_Z1), (2.9, COLLAR_Z1 - 0.6), (NECK_R + 0.05, COLLAR_Z1 - 0.6)]
collar = link(revolve('Collar', collar_profile), 'Collar', 'Collar', smooth_angle=40)

ACT_R, ACT_Z0, ACT_Z1 = 5.5, COLLAR_Z1 + 0.8, COLLAR_Z1 + 9.8
act_profile = [(0.0, ACT_Z0), (1.4, ACT_Z0), (1.4, COLLAR_Z1 - 0.3), (2.6, COLLAR_Z1 - 0.3), (2.6, ACT_Z0),
               (ACT_R - 0.5, ACT_Z0), *arc(ACT_R - 0.5, ACT_Z0 + 0.5, 0.5, -math.pi / 2, 0, 5)[1:],
               (ACT_R, ACT_Z1 - 1.2), *arc(ACT_R - 1.2, ACT_Z1 - 1.2, 1.2, 0, math.pi / 2, 8)[1:],
               (ACT_R * 0.6, ACT_Z1 + 0.25), (0.0, ACT_Z1 + 0.35)]
actuator = link(revolve('Actuator', act_profile, segments=160), 'Actuator', 'Actuator', smooth_angle=40)
# The orifice: a small dimple with the spray hole in it, facing front.
orifice = cylinder('Orifice', 0.35, -6.0, 0.0, segments=32)
orifice.rotation_euler = (math.radians(-90), 0, 0)
orifice.location = (0, -ACT_R + 0.9, ACT_Z1 - 3.0)
dimple = cylinder('Dimple', 1.3, -0.25, 0.4, segments=64)
dimple.rotation_euler = (math.radians(-90), 0, 0)
dimple.location = (0, -ACT_R - 0.05, ACT_Z1 - 3.0)
for tool in (orifice, dimple):
    bpy.context.view_layer.update()
    tool.data.transform(tool.matrix_world)
    tool.matrix_world = Matrix.Identity(4)
    boolean(actuator, tool, 'DIFFERENCE')
actuator.data.set_sharp_from_angle(angle=math.radians(40))

# The dip tube: down the bore, curving to the bottom corner.
pts = [Vector((0, 0, COLLAR_Z1 - 1.0)), Vector((0, 0, CAV_Z1 - 4)), Vector((3.5, 1.0, 40.0)),
       Vector((13.0, 3.5, CAV_Z0 + 4.0)), Vector((15.5, 4.0, CAV_Z0 + 1.2))]
bm = bmesh.new()
loops, normal, n_side = [], None, 20
samples = []
for k in range(len(pts) - 1):
    p0 = pts[max(k - 1, 0)]
    p1, p2 = pts[k], pts[k + 1]
    p3 = pts[min(k + 2, len(pts) - 1)]
    for s in range(30):
        x = s / 30
        samples.append(0.5 * ((2 * p1) + (-p0 + p2) * x + (2 * p0 - 5 * p1 + 4 * p2 - p3) * x * x
                              + (-p0 + 3 * p1 - 3 * p2 + p3) * x ** 3))
samples.append(pts[-1])
for n, q in enumerate(samples):
    tangent = (samples[min(n + 1, len(samples) - 1)] - samples[max(n - 1, 0)]).normalized()
    normal = tangent.orthogonal().normalized() if normal is None else (normal - tangent * normal.dot(tangent)).normalized()
    binormal = tangent.cross(normal)
    outer = [bm.verts.new(q + (normal * math.cos(a) + binormal * math.sin(a)) * 1.2) for a in (2 * math.pi * i / n_side for i in range(n_side))]
    inner = [bm.verts.new(q + (normal * math.cos(a) + binormal * math.sin(a)) * 0.8) for a in (2 * math.pi * i / n_side for i in range(n_side))]
    loops.append((outer, inner))
for (o1, i1), (o2, i2) in zip(loops, loops[1:]):
    for i in range(n_side):
        j = (i + 1) % n_side
        bm.faces.new((o1[i], o1[j], o2[j], o2[i]))
        bm.faces.new((i1[j], i1[i], i2[i], i2[j]))
for (o, i_), flip in ((loops[0], True), (loops[-1], False)):
    for i in range(n_side):
        j = (i + 1) % n_side
        f = (o[i], i_[i], i_[j], o[j])
        bm.faces.new(f[::-1] if flip else f)
bmesh.ops.recalc_face_normals(bm, faces=bm.faces)
mesh = bpy.data.meshes.new('Dip tube')
bm.to_mesh(mesh)
bm.free()
tube = link(mesh, 'Dip tube', 'Dip tube', smooth_angle=50)

CAP = 30.0
CAP_Z0 = H + 1.0
cap = rounded_box('Cap', (CAP, CAP, 32.0), 2.0, (0, 0, CAP_Z0 + 16.0), segments=10)
inside = cylinder('CapBore', 10.2, CAP_Z0 - 1.0, CAP_Z0 + 25.0, segments=128)
boolean(cap, inside, 'DIFFERENCE')
linked = link(cap.data, 'Cap', 'Cap', smooth_angle=35)
bpy.data.objects.remove(cap)
cap = linked
cap.name = 'Cap'


# ---- print --------------------------------------------------------------
def text(body, size, z, font=None, spacing=1.0, weight_x=1.0):
    curve = bpy.data.curves.new('Text', 'FONT')
    curve.body = body
    curve.size = size
    curve.align_x = 'CENTER'
    curve.space_character = spacing
    curve.extrude = 0.015
    if font:
        curve.font = bpy.data.fonts.load(font)
    ob = bpy.data.objects.new('Text', curve)
    scene.collection.objects.link(ob)
    ob.rotation_euler = (math.radians(90), 0, 0)
    ob.location = (0, -DEPTH / 2 - 0.02, z)
    ob.scale = (weight_x, 1, 1)
    bpy.context.view_layer.update()
    mesh = bpy.data.meshes.new_from_object(ob.evaluated_get(bpy.context.evaluated_depsgraph_get()))
    mesh.transform(ob.matrix_world)
    bpy.data.objects.remove(ob)
    return mesh


# All in Blender's own font (Inter, under the Open Font License), so the
# lettering in the model that ships with Tequila is free to ship.
pieces = [text('OREN', 7.2, 44.0, spacing=2.1), text('AMBRE NUIT', 2.5, 37.4, spacing=2.2),
          text('EAU DE PARFUM', 1.75, 33.6, spacing=2.4), text('50 ml  ·  1.7 fl. oz.', 1.45, 19.0, spacing=1.6)]
bm = bmesh.new()
for m in pieces:
    bm.from_mesh(m)
    bpy.data.meshes.remove(m)
# A hairline rule under the name.
rule = bmesh.ops.create_cube(bm, size=1.0)
bmesh.ops.scale(bm, vec=Vector((9.0, 0.03, 0.12)), verts=rule['verts'])
bmesh.ops.translate(bm, vec=Vector((0, -DEPTH / 2 - 0.03, 41.4)), verts=rule['verts'])
mesh = bpy.data.meshes.new('Print')
bm.to_mesh(mesh)
bm.free()
link(mesh, 'Print', 'Print', smooth_angle=20)

if EXPLODED:
    lift = {'Cap': 46.0, 'Actuator': 26.0, 'Collar': 15.0, 'Dip tube': 9.0}
    for ob in scene.collection.objects:
        if ob.name in lift:
            ob.data.transform(Matrix.Translation((0, 0, lift[ob.name])))

for ob in list(scene.collection.objects):
    ob.data.transform(Matrix.Scale(0.001, 4))
    print(f'{ob.name}: {len(ob.data.polygons)} faces')
print(f'fill line {FILL:.1f} mm; cavity {CAV_W} x {CAV_D} x {CAV_Z1 - CAV_Z0} mm, '
      f'about {CAV_W * CAV_D * (CAV_Z1 - CAV_Z0) / 1000 * 0.82:.0f} ml of liquid')
bpy.ops.export_scene.gltf(filepath=OUT, export_format='GLB', export_yup=True, export_normals=True,
                          export_materials='EXPORT')
print('wrote', OUT)
