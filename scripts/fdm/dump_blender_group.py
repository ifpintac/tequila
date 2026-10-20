"""Dump the "3DP Layer Lines" node group from a PLA 3D Print .blend to
`layer_lines.json`, the source `generate_layer_lines.py` reads.

    blender -b --factory-startup --disable-autoexec PLA_3DPrint_Basic.blend --python dump_blender_group.py -- layer_lines.json
"""
import json
import sys

import bpy

GROUP = '3DP Layer Lines'
out = sys.argv[sys.argv.index('--') + 1]

# What a node is, as the generator needs it: its type, its settings, and the
# values of its sockets. Layout, frames and Blender's own pointers are left out.
PROPS = {'operation', 'use_clamp', 'noise_dimensions', 'noise_type', 'normalize', 'mode', 'interpolation_type',
         'data_type', 'clamp', 'vector_type', 'convert_from', 'convert_to', 'invert'}


def value(v):
    if hasattr(v, '__len__') and not isinstance(v, str):
        return [value(x) for x in v]
    return v


def node(n):
    return {
        'name': n.name,
        'type': n.type,
        'props': {p: value(getattr(n, p)) for p in PROPS if hasattr(n, p)},
        'inputs': [{'name': s.name, 'id': s.identifier, 'linked': s.is_linked,
                    'default': value(getattr(s, 'default_value', None))} for s in n.inputs],
        'outputs': [{'name': s.name, 'id': s.identifier} for s in n.outputs],
    }


tree = bpy.data.node_groups[GROUP]
group = {
    'name': GROUP,
    'nodes': [node(n) for n in tree.nodes if n.type != 'FRAME'],
    'links': [{'from': l.from_node.name, 'from_socket': l.from_socket.identifier, 'from_socket_name': l.from_socket.name,
               'to': l.to_node.name, 'to_socket': l.to_socket.identifier, 'to_socket_name': l.to_socket.name}
              for l in tree.links if l.is_valid and not l.is_muted],
    'interface': [{'name': i.name, 'in_out': i.in_out, 'socket_type': i.socket_type,
                   'default': value(getattr(i, 'default_value', None)),
                   'min': getattr(i, 'min_value', None), 'max': getattr(i, 'max_value', None),
                   'description': i.description}
                  for i in tree.interface.items_tree if i.item_type == 'SOCKET'],
}
with open(out, 'w', encoding='utf-8') as f:
    json.dump(group, f, indent=1)
print(len(group['nodes']), 'nodes', len(group['links']), 'links')
