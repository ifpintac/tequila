# Makes the Blender scene the .blend import is tested on.
#
#   blender --background --factory-startup --python make_blend_fixtures.py -- <out.blend>
#
# Everything in it is made here, from Blender's own primitives: a part with
# two materials, a child with a modifier, a hidden part, an excluded
# collection, an empty one, a collection instanced twice, text, a curve,
# two cameras, a light of each kind and a world; and materials that use the nodes the import
# translates - a packed image clipped onto the top face as a label, a node
# group, a muted node and a reroute, a ramp, curves, a bump, a normal map,
# mixes of shaders, a cut-out and a glow.
#
# tests/blend.rs reads it through Blender when Blender is installed, and
# reads the answer Blender gave for it, kept in data/blend, when it is not.
#
# SPDX-License-Identifier: Apache-2.0

import os
import sys

import bpy

out = sys.argv[sys.argv.index("--") + 1]
bpy.ops.wm.read_factory_settings(use_empty=True)
scene = bpy.context.scene
scene.name = "Fixture"
scene.render.resolution_x, scene.render.resolution_y = 1200, 800
scene.view_settings.view_transform = "Standard"


def collection(name, parent=None):
    c = bpy.data.collections.new(name)
    (parent or scene.collection).children.link(c)
    return c


def link(obj, c):
    for old in list(obj.users_collection):
        old.objects.unlink(obj)
    c.objects.link(obj)
    return obj


def nodes(material):
    try:
        material.use_nodes = True
    except Exception:
        pass
    tree = material.node_tree
    return tree, tree.nodes, tree.links


def principled(name, colour, roughness=0.4, metallic=0.0):
    m = bpy.data.materials.new(name)
    tree, n, l = nodes(m)
    p = n["Principled BSDF"]
    p.inputs["Base Color"].default_value = (*colour, 1.0)
    p.inputs["Roughness"].default_value = roughness
    p.inputs["Metallic"].default_value = metallic
    return m, tree, n, l, p


product = collection("Product")
studio = collection("Studio")
spare = collection("Spare")
empty = collection("Empty")
kit = collection("Kit")

# ---- materials ---------------------------------------------------------------

# A plain Principled with its scales: specular level, coat, anisotropy.
paint, _, n, l, p = principled("Paint", (0.8, 0.1, 0.05), 0.35)
p.inputs["Specular IOR Level"].default_value = 0.25
p.inputs["Coat Weight"].default_value = 0.6
p.inputs["Coat Roughness"].default_value = 0.1
p.inputs["IOR"].default_value = 1.45

steel, _, n, l, p = principled("Brushed", (0.9, 0.9, 0.9), 0.3, 1.0)
p.inputs["Anisotropic"].default_value = 0.7
p.inputs["Anisotropic Rotation"].default_value = 0.25
tangent = n.new("ShaderNodeTangent")
tangent.direction_type = "RADIAL"
tangent.axis = "Z"
l.new(tangent.outputs["Tangent"], p.inputs["Tangent"])

# A label: a packed picture projected from above in object space, clipped
# to itself and kept to the faces that look up, over the coat's colour.
image = bpy.data.images.new("label", 16, 8, alpha=True)
pixels = []
for y in range(8):
    for x in range(16):
        ink = (x // 2 + y // 2) % 2 == 0
        pixels += [1.0, 1.0, 1.0, 1.0 if ink else 0.0]
image.pixels = pixels
image.pack()
label, tree, n, l, p = principled("Label", (0.02, 0.1, 0.15), 0.5)
coords = n.new("ShaderNodeTexCoord")
mapping = n.new("ShaderNodeMapping")
mapping.inputs["Location"].default_value = (0.5, 0.5, 0.0)
mapping.inputs["Scale"].default_value = (2.0, 4.0, 1.0)
picture = n.new("ShaderNodeTexImage")
picture.image = image
picture.extension = "CLIP"
geometry = n.new("ShaderNodeNewGeometry")
split = n.new("ShaderNodeSeparateXYZ")
up = n.new("ShaderNodeMapRange")
up.inputs["From Min"].default_value = 0.9
up.inputs["From Max"].default_value = 0.98
mask = n.new("ShaderNodeMath")
mask.operation = "MULTIPLY"
over = n.new("ShaderNodeMix")
over.data_type = "RGBA"
over.inputs["B"].default_value = (0.9, 0.85, 0.7, 1.0)
l.new(coords.outputs["Object"], mapping.inputs["Vector"])
l.new(mapping.outputs["Vector"], picture.inputs["Vector"])
l.new(geometry.outputs["Normal"], split.inputs["Vector"])
l.new(split.outputs["Z"], up.inputs["Value"])
l.new(picture.outputs["Alpha"], mask.inputs[0])
l.new(up.outputs["Result"], mask.inputs[1])
l.new(mask.outputs["Value"], over.inputs["Factor"])
l.new(over.outputs["Result"], p.inputs["Base Color"])

# A speckle through a node group, a muted node and a reroute, read by a
# ramp, bumped by noise and curved.
group = bpy.data.node_groups.new("Speckle", "ShaderNodeTree")
group.interface.new_socket("Scale", in_out="INPUT", socket_type="NodeSocketFloat")
group.interface.new_socket("Value", in_out="OUTPUT", socket_type="NodeSocketFloat")
g_in = group.nodes.new("NodeGroupInput")
g_out = group.nodes.new("NodeGroupOutput")
voronoi = group.nodes.new("ShaderNodeTexVoronoi")
times = group.nodes.new("ShaderNodeMath")
times.operation = "MULTIPLY"
times.inputs[1].default_value = 2.0
group.links.new(g_in.outputs[0], voronoi.inputs["Scale"])
group.links.new(voronoi.outputs["Distance"], times.inputs[0])
group.links.new(times.outputs[0], g_out.inputs[0])
speckle, tree, n, l, p = principled("Speckle", (0.5, 0.5, 0.5), 0.5)
instance = n.new("ShaderNodeGroup")
instance.node_tree = group
instance.inputs[0].default_value = 40.0
muted = n.new("ShaderNodeInvert")
muted.mute = True
reroute = n.new("NodeReroute")
ramp = n.new("ShaderNodeValToRGB")
ramp.color_ramp.elements[0].color = (0.1, 0.2, 0.3, 1.0)
ramp.color_ramp.elements[1].color = (0.9, 0.8, 0.7, 1.0)
curves = n.new("ShaderNodeRGBCurve")
curves.mapping.curves[3].points[1].location = (1.0, 0.6)
curves.mapping.update()
noise = n.new("ShaderNodeTexNoise")
noise.inputs["Scale"].default_value = 80.0
bump = n.new("ShaderNodeBump")
bump.inputs["Strength"].default_value = 0.5
bump.inputs["Distance"].default_value = 0.002
l.new(instance.outputs[0], muted.inputs["Color"])
l.new(muted.outputs["Color"], reroute.inputs[0])
l.new(reroute.outputs[0], ramp.inputs["Fac"])
l.new(ramp.outputs["Color"], curves.inputs["Color"])
l.new(curves.outputs["Color"], p.inputs["Base Color"])
l.new(noise.outputs["Fac"], bump.inputs["Height"])
l.new(bump.outputs["Normal"], p.inputs["Normal"])

# A normal map from a packed picture, through the mesh's UVs.
flat = bpy.data.images.new("normal", 4, 4)
flat.pixels = [0.5, 0.5, 1.0, 1.0] * 16
flat.colorspace_settings.name = "Non-Color"
flat.pack()
dimpled, tree, n, l, p = principled("Dimpled", (0.2, 0.2, 0.2), 0.6)
normals = n.new("ShaderNodeTexImage")
normals.image = flat
normal_map = n.new("ShaderNodeNormalMap")
l.new(normals.outputs["Color"], normal_map.inputs["Color"])
l.new(normal_map.outputs["Normal"], p.inputs["Normal"])

# Shaders mixed: a glossy over a diffuse, a cut-out and a glow.
mixed = bpy.data.materials.new("Mixed")
tree, n, l = nodes(mixed)
n.remove(n["Principled BSDF"])
diffuse = n.new("ShaderNodeBsdfDiffuse")
diffuse.inputs["Color"].default_value = (0.2, 0.6, 0.2, 1.0)
glossy = n.new("ShaderNodeBsdfGlossy")
mix = n.new("ShaderNodeMixShader")
mix.inputs["Fac"].default_value = 0.3
transparent = n.new("ShaderNodeBsdfTransparent")
cut = n.new("ShaderNodeMixShader")
checker = n.new("ShaderNodeTexChecker")
emission = n.new("ShaderNodeEmission")
emission.inputs["Color"].default_value = (1.0, 0.5, 0.1, 1.0)
emission.inputs["Strength"].default_value = 2.0
add = n.new("ShaderNodeAddShader")
l.new(diffuse.outputs[0], mix.inputs[1])
l.new(glossy.outputs[0], mix.inputs[2])
l.new(checker.outputs["Fac"], cut.inputs["Fac"])
l.new(transparent.outputs[0], cut.inputs[1])
l.new(mix.outputs[0], cut.inputs[2])
l.new(cut.outputs[0], add.inputs[0])
l.new(emission.outputs[0], add.inputs[1])
l.new(add.outputs[0], n["Material Output"].inputs["Surface"])

glass, _, n, l, p = principled("Glass", (0.8, 0.9, 1.0), 0.0)
p.inputs["Transmission Weight"].default_value = 1.0

# ---- objects -------------------------------------------------------------------

bpy.ops.mesh.primitive_cube_add(size=0.1, location=(0, 0, 0.05))
body = link(bpy.context.object, product)
body.name = "Body"
body.data.materials.append(label)
body.data.materials.append(paint)
for face in body.data.polygons:
    face.material_index = 0 if face.normal.z > 0.5 else 1

bpy.ops.mesh.primitive_cylinder_add(radius=0.02, depth=0.03, location=(0.03, 0.0, 0.115))
knob = link(bpy.context.object, product)
knob.name = "Knob"
knob.data.materials.append(steel)
knob.modifiers.new("Rounded", "BEVEL").width = 0.003
knob.parent = body
knob.matrix_parent_inverse = body.matrix_world.inverted()

bpy.ops.mesh.primitive_uv_sphere_add(radius=0.01, location=(-0.03, 0.0, 0.11))
dome = link(bpy.context.object, product)
dome.name = "Dome"
dome.data.materials.append(speckle)

bpy.ops.mesh.primitive_plane_add(size=0.04, location=(0.0, -0.06, 0.05), rotation=(1.5708, 0, 0))
plate = link(bpy.context.object, product)
plate.name = "Plate"
plate.data.materials.append(dimpled)

bpy.ops.mesh.primitive_ico_sphere_add(radius=0.015, location=(0.0, 0.08, 0.02))
gem = link(bpy.context.object, product)
gem.name = "Gem"
gem.data.materials.append(mixed)

bpy.ops.mesh.primitive_cube_add(size=0.02, location=(0.08, 0.0, 0.01))
hidden = link(bpy.context.object, product)
hidden.name = "Hidden"
hidden.hide_render = True

bpy.ops.mesh.primitive_cone_add(radius1=0.01, depth=0.02, location=(-0.08, 0, 0.01))
spare_part = link(bpy.context.object, spare)
spare_part.name = "Spare part"
spare_part.data.materials.append(glass)

bpy.ops.mesh.primitive_plane_add(size=1.0)
floor = link(bpy.context.object, studio)
floor.name = "Floor"

# A collection instanced twice.
bpy.ops.mesh.primitive_uv_sphere_add(radius=0.005, segments=8, ring_count=4)
screw = link(bpy.context.object, kit)
screw.name = "Screw"
screw.data.materials.append(steel)
scene.view_layers[0].layer_collection.children["Kit"].exclude = True
for i, x in enumerate((-0.04, 0.04)):
    holder = bpy.data.objects.new(f"Screw {i + 1}", None)
    holder.instance_type = "COLLECTION"
    holder.instance_collection = kit
    holder.location = (x, -0.05, 0.0)
    product.objects.link(holder)

bpy.ops.object.text_add(location=(0, -0.08, 0))
words = link(bpy.context.object, product)
words.name = "Words"
words.data.body = "Tq"
words.data.size = 0.02
words.data.extrude = 0.001

curve = bpy.data.curves.new("Cable", "CURVE")
curve.dimensions = "3D"
curve.bevel_depth = 0.002
spline = curve.splines.new("POLY")
spline.points.add(2)
for i, point in enumerate(spline.points):
    point.co = (0.05 + 0.01 * i, 0.05, 0.01 * (i % 2), 1.0)
cable = bpy.data.objects.new("Cable", curve)
product.objects.link(cable)

scene.view_layers[0].layer_collection.children["Spare"].exclude = True

camera_data = bpy.data.cameras.new("Hero")
camera_data.lens = 85.0
camera_data.dof.use_dof = True
camera_data.dof.focus_distance = 0.5
camera_data.dof.aperture_fstop = 4.0
hero = bpy.data.objects.new("Hero", camera_data)
hero.location = (0.3, -0.3, 0.25)
hero.rotation_euler = (1.1, 0.0, 0.785)
studio.objects.link(hero)
scene.camera = hero

top_data = bpy.data.cameras.new("Top")
top_data.type = "ORTHO"
top_data.ortho_scale = 0.3
top = bpy.data.objects.new("Top", top_data)
top.location = (0, 0, 1.0)
studio.objects.link(top)

# Lights of each kind, and a world.
def lamp(name, kind, location, rotation=(0.0, 0.0, 0.0), **settings):
    data = bpy.data.lights.new(name, kind)
    for key, value in settings.items():
        setattr(data, key, value)
    obj = bpy.data.objects.new(name, data)
    obj.location = location
    obj.rotation_euler = rotation
    studio.objects.link(obj)
    return obj

lamp("Key", "AREA", (0.3, -0.3, 0.4), (0.8, 0.0, 0.785), shape="RECTANGLE", size=0.4, size_y=0.2, energy=50.0, spread=2.0943951)
lamp("Fill", "POINT", (-0.3, -0.2, 0.2), shadow_soft_size=0.05, energy=20.0, exposure=1.0, color=(1.0, 0.9, 0.8))
lamp("Spot", "SPOT", (0.0, 0.3, 0.5), (-0.6, 0.0, 0.0), spot_size=1.0471976, spot_blend=0.3, energy=100.0)
lamp("Sun", "SUN", (0.0, 0.0, 2.0), (0.3, 0.2, 0.0), energy=3.0, angle=0.0349066)
lamp("Off", "POINT", (0.0, 0.0, 0.3), energy=10.0).hide_render = True

world = bpy.data.worlds.new("Room")
tree, n, l = nodes(world)
n["Background"].inputs["Color"].default_value = (0.05, 0.06, 0.07, 1.0)
n["Background"].inputs["Strength"].default_value = 1.5
scene.world = world

bpy.ops.wm.save_as_mainfile(filepath=os.path.abspath(out), compress=True)
