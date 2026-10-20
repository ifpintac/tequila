"""What Blender's "3DP Layer Lines" group gives at the vertices of a test
sphere, baked by Cycles, for `fdm_layer_lines_match_blender` to compare
Tequila's port against.

    blender -b --factory-startup --disable-autoexec PLA_3DPrint_Basic.blend --python bake_reference.py -- OUT.json

The sphere is 24 mm across, so it has walls, overhangs, a top and a bottom,
and every slope between. Each pass bakes three of the group's outputs into
a float colour per face corner, offset so nothing is negative. Cycles bakes
a corner a little inside its face, not at the vertex, so the point and the
normal it baked at are baked too, and the comparison is made there.
"""
import json
import sys

import bpy

out = sys.argv[sys.argv.index('--') + 1]
group = bpy.data.node_groups['3DP Layer Lines']

scene = bpy.context.scene
scene.render.engine = 'CYCLES'
scene.cycles.device = 'CPU'
scene.cycles.samples = 1
scene.render.bake.target = 'VERTEX_COLORS'

bpy.ops.mesh.primitive_uv_sphere_add(segments=160, ring_count=120, radius=0.012, location=(0, 0, 0))
sphere = bpy.context.active_object
bpy.ops.object.shade_smooth()
mesh = sphere.data

material = bpy.data.materials.new('bake')
sphere.data.materials.append(material)
nodes, links = material.node_tree.nodes, material.node_tree.links
nodes.clear()
lines = nodes.new('ShaderNodeGroup')
lines.node_tree = group
emission = nodes.new('ShaderNodeEmission')
output = nodes.new('ShaderNodeOutputMaterial')
combine = nodes.new('ShaderNodeCombineXYZ')
links.new(combine.outputs[0], emission.inputs['Color'])
links.new(emission.outputs[0], output.inputs['Surface'])


def offset(socket, by):
    add = nodes.new('ShaderNodeMath')
    add.operation = 'ADD'
    add.inputs[1].default_value = by
    links.new(socket, add.inputs[0])
    return add.outputs[0]


def vector_offset(socket, by):
    add = nodes.new('ShaderNodeVectorMath')
    add.operation = 'ADD'
    add.inputs[1].default_value = (by, by, by)
    links.new(socket, add.inputs[0])
    return add.outputs[0]


PASSES = {
    # Small, so the point keeps its precision: a float near 1 resolves
    # a tenth of a micrometre, near 0.02 two hundredths of one.
    'object': (None, ('Object', 0.02)),
    'object_normal': (None, ('Normal', 1.0)),
    'height_cavity_layer': ([('Height (mm)', 1.0), ('Cavity', 0.0), ('Layer Random', 0.0)], None),
    'wall_top_bottom': ([('Wall Factor', 0.0), ('Top Mask', 0.0), ('Bottom Mask', 0.0)], None),
    'tangent': (None, ('Tangent', 1.0)),
    'position': (None, ('Position (mm)', 100.0)),
}

coordinates = nodes.new('ShaderNodeTexCoord')
result = {}
for name, (channels, vector) in PASSES.items():
    for link in list(emission.inputs['Color'].links):
        links.remove(link)
    if channels:
        for i, (socket, by) in enumerate(channels):
            for link in list(combine.inputs[i].links):
                links.remove(link)
            links.new(offset(lines.outputs[socket], by), combine.inputs[i])
        links.new(combine.outputs[0], emission.inputs['Color'])
    else:
        socket, by = vector
        source = coordinates if socket in ('Object', 'Normal') else lines
        links.new(vector_offset(source.outputs[socket], by), emission.inputs['Color'])
    attribute = mesh.color_attributes.new('bake_' + name, 'FLOAT_COLOR', 'CORNER')
    mesh.color_attributes.active_color = attribute
    bpy.ops.object.bake(type='EMIT')
    result[name] = [list(d.color)[:3] for d in mesh.color_attributes['bake_' + name].data]
    print(name, 'baked')

with open(out, 'w', encoding='utf-8') as f:
    json.dump(result, f)
print(len(result['object']), 'corners')
