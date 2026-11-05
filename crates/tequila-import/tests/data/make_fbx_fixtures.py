"""Write the FBX fixtures for tests/more_formats.rs, with Blender.

    blender -b --factory-startup --python make_fbx_fixtures.py -- OUT_DIR

Blender's own FBX exporter, on a synthetic scene in millimetres (Blender's
unit scale 0.001), Z up as Blender is:

* "Plinth": a box 80 x 60 x 12 mm standing on the ground, its origin at its
  centre (0, 0, 6). Material "Red plastic" (Principled, base colour
  0.8 0.05 0.03, roughness 0.35).
* "Column": a 16-sided cylinder, radius 8 mm and 50 mm tall, a child of the
  Plinth 15 mm to its -X and 31 mm up, tilted 20 degrees about Y. Material
  "Checker", whose base colour is checker.png (16 x 16, white and teal),
  roughness 0.6.
* "Ball": a UV sphere of radius 12 mm, a child of the Column 37 mm along its
  own Z. Material "Steel" (metallic 1, roughness 0.25).
* "Ball copy": the Ball's mesh again (a linked duplicate, so the FBX holds
  one geometry placed twice), a child of the Plinth at (22, 15, 18) from it,
  at half size.
* A camera and a light, which the importer leaves out.

Two files:

* primitives.fbx: binary, as Blender writes it, in millimetres
  (`apply_scale_options='FBX_SCALE_UNITS'`, so UnitScaleFactor 0.1), Y up
  (the exporter's default axes), checker.png packed inside it.
* primitives_ascii.fbx: the same scene in centimetres
  (`FBX_SCALE_NONE`, the exporter's default: UnitScaleFactor 1, its top
  objects scaled by a tenth), Z up (`axis_up='Z'`, `axis_forward='Y'`),
  checker.png named by a relative path, written out as FBX 7.4 ASCII text
  from the binary Blender wrote, since Blender writes binary only. Its
  Creator is renamed so it reads as a plain Phong file, as one from the
  FBX SDK would, rather than as Blender's.

The world centres each object should land at are printed, in millimetres,
Blender's Z up; tests/more_formats.rs states them Y up.
"""

import os
import sys

import bpy
from mathutils import Matrix, Vector

OUT = os.path.abspath(sys.argv[sys.argv.index("--") + 1]) if "--" in sys.argv else os.path.dirname(os.path.abspath(__file__))
os.makedirs(OUT, exist_ok=True)

bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.unit_settings.system = 'METRIC'
scene.unit_settings.scale_length = 0.001
scene.unit_settings.length_unit = 'MILLIMETERS'

# The picture: 16 x 16, cells of 4, white and teal.
image = bpy.data.images.new("checker", 16, 16, alpha=False)
pixels = []
for y in range(16):
    for x in range(16):
        light = ((x // 4) + (y // 4)) % 2 == 0
        pixels += [1.0, 1.0, 1.0, 1.0] if light else [0.05, 0.45, 0.5, 1.0]
image.pixels = pixels
image.filepath_raw = os.path.join(OUT, "checker.png")
image.file_format = 'PNG'
image.save()


def principled(name, colour, roughness, metallic=0.0, picture=None):
    material = bpy.data.materials.new(name)
    material.use_nodes = True
    bsdf = material.node_tree.nodes["Principled BSDF"]
    bsdf.inputs["Base Color"].default_value = (*colour, 1.0)
    bsdf.inputs["Roughness"].default_value = roughness
    bsdf.inputs["Metallic"].default_value = metallic
    if picture is not None:
        node = material.node_tree.nodes.new("ShaderNodeTexImage")
        node.image = picture
        material.node_tree.links.new(node.outputs["Color"], bsdf.inputs["Base Color"])
    return material


red = principled("Red plastic", (0.8, 0.05, 0.03), 0.35)
checker = principled("Checker", (1.0, 1.0, 1.0), 0.6, picture=image)
steel = principled("Steel", (0.9, 0.9, 0.92), 0.25, metallic=1.0)


def add(kind, name, material, **kw):
    kind(**kw)
    obj = bpy.context.active_object
    obj.name = name
    obj.data.name = name
    obj.data.materials.append(material)
    return obj


plinth = add(bpy.ops.mesh.primitive_cube_add, "Plinth", red, size=1.0, location=(0, 0, 6))
plinth.scale = (80, 60, 12)
bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)

column = add(bpy.ops.mesh.primitive_cylinder_add, "Column", checker, vertices=16, radius=8, depth=50, location=(0, 0, 0))
column.parent = plinth
column.location = (-15, 0, 31)
column.rotation_euler = (0, 0.3490658503988659, 0)  # 20 degrees about Y

ball = add(bpy.ops.mesh.primitive_uv_sphere_add, "Ball", steel, segments=16, ring_count=8, radius=12, location=(0, 0, 0))
ball.parent = column
ball.location = (0, 0, 37)
for polygon in ball.data.polygons:
    polygon.use_smooth = True

copy = ball.copy()  # shares the mesh
copy.name = "Ball copy"
scene.collection.objects.link(copy)
copy.parent = plinth
copy.location = (22, 15, 18)
copy.scale = (0.5, 0.5, 0.5)

camera = bpy.data.objects.new("Camera", bpy.data.cameras.new("Camera"))
camera.location = (200, -200, 150)
scene.collection.objects.link(camera)
light = bpy.data.objects.new("Light", bpy.data.lights.new("Light", 'POINT'))
light.location = (100, 50, 200)
scene.collection.objects.link(light)

bpy.context.view_layer.update()
for obj in (plinth, column, ball, copy):
    corners = [obj.matrix_world @ Vector(c) for c in obj.bound_box]
    centre = sum(corners, Vector()) / 8
    print(f"FIXTURE {obj.name}: centre ({centre.x:.4f}, {centre.y:.4f}, {centre.z:.4f}) mm")

common = dict(use_selection=False, object_types={'MESH', 'EMPTY', 'CAMERA', 'LIGHT'}, bake_anim=False, mesh_smooth_type='FACE')

binary = os.path.join(OUT, "primitives.fbx")
bpy.ops.export_scene.fbx(filepath=binary, path_mode='COPY', embed_textures=True, apply_unit_scale=True,
                         apply_scale_options='FBX_SCALE_UNITS', **common)
# COPY mode copies the picture beside the file as well as packing it.
scratch = os.path.join(OUT, "primitives.fbm")
if os.path.isdir(scratch):
    for name in os.listdir(scratch):
        os.remove(os.path.join(scratch, name))
    os.rmdir(scratch)

text_source = os.path.join(OUT, "primitives_z_up.fbx")
bpy.ops.export_scene.fbx(filepath=text_source, path_mode='RELATIVE', apply_unit_scale=True,
                         apply_scale_options='FBX_SCALE_NONE', axis_up='Z', axis_forward='Y', **common)


# Binary FBX to ASCII FBX 7.4, element by element.
from io_scene_fbx import parse_fbx  # noqa: E402

root, version = parse_fbx.parse(text_source)


def text_of(value, kind):
    if kind == 'S':
        s = value.decode('utf-8', 'replace')
        if '\x00\x01' in s:  # binary "Name\0\1Class" is ASCII "Class::Name"
            name, cls = s.split('\x00\x01', 1)
            s = f"{cls}::{name}"
        return '"' + s.replace('"', '&quot;') + '"'
    if kind == 'C':
        return 'T' if value else 'F'
    if kind in 'YZIL':
        return str(int(value))
    if kind in 'FD':
        return repr(float(value))
    raise ValueError(kind)


def write(out, elem, depth):
    tab = '\t' * depth
    name = elem.id.decode()
    kinds = elem.props_type.decode()
    if any(k in 'R' for k in kinds):
        return  # raw bytes (FileId, packed content): not carried in text
    if len(kinds) == 1 and kinds in 'fdilbc':
        values = list(elem.props[0])
        if kinds in 'fd':
            items = ','.join(repr(float(v)) for v in values)
        elif kinds == 'b':
            items = ','.join('1' if v else '0' for v in values)
        else:
            items = ','.join(str(int(v)) for v in values)
        out.append(f"{tab}{name}: *{len(values)} {{")
        out.append(f"{tab}\ta: {items}")
        out.append(f"{tab}}}")
        return
    props = ', '.join(text_of(v, k) for v, k in zip(elem.props, kinds))
    if name == 'Creator':
        props = '"Tequila test fixture"'
    if elem.elems:
        out.append(f"{tab}{name}: {props} {{" if props else f"{tab}{name}:  {{")
        for child in elem.elems:
            write(out, child, depth + 1)
        out.append(f"{tab}}}")
    else:
        out.append(f"{tab}{name}: {props}")


lines = ["; FBX 7.4.0 project file", "; Written for Tequila's tests by make_fbx_fixtures.py, from Blender's binary export", ""]
for elem in root.elems:
    write(lines, elem, 0)
with open(os.path.join(OUT, "primitives_ascii.fbx"), "w", encoding="utf-8", newline="\n") as f:
    f.write("\n".join(lines) + "\n")
os.remove(text_source)
print("FIXTURE wrote", OUT)
