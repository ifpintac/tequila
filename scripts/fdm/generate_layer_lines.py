"""Write `crates/tequila-materials/src/groups/fdm_layer_lines.rs` from the
"3DP Layer Lines" Blender node group (`layer_lines.json`, dumped from the
PLA 3D Print .blend with `dump_blender_group.py`).

    python scripts/fdm/generate_layer_lines.py

The group runs in the print's own frame, as it does in Blender: millimetres,
X and Y across the bed, Z up the build, the build being the part's own Z.
That is how an STL, a STEP or an IGES file is drawn, and Tequila keeps it:
such a part is stood up by turning its node, not its points. A part drawn
Y up (glTF, OBJ, FBX) is built along its Y, so the group has one input of
its own, Build Along Z: 1 reads the part's coordinates as they are, 0 turns
them into the print's frame (x, -z, y) and the tangent it writes back.
Everything else is node for node: the same Math, Vector Math, Noise, White
Noise, Map Range and Separate/Combine nodes, which Tequila evaluates as
Cycles does.

Three of Blender's nodes have no counterpart and are replaced by what they
compute here:
  * Vector Transform (object to world) of an axis, whose length is the
    part's scale: Tequila's Object coordinates already include it, so the
    axis is passed as it is and its length is 1.
  * Geometry's Normal, for the up-the-wall tangent: the part's own normal,
    so the tangent is in the part's space, as Tequila's Tangent input wants.
  * Bump: Tequila's Bump socket takes a height in scene units, so the
    group's output is its height in millimetres times the bump distance.
"""
import json
import os
import struct

HERE = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.normpath(os.path.join(HERE, '..', '..'))
SOURCE = os.path.join(HERE, 'layer_lines.json')
OUT = os.path.join(REPO, 'crates', 'tequila-materials', 'src', 'groups', 'fdm_layer_lines.rs')

group = json.load(open(SOURCE, encoding='utf-8'))
nodes = {n['name']: n for n in group['nodes']}

# The group's inputs, less "mm per Blender Unit": Tequila is in metres.
HIDDEN_INPUT = 'mm per Blender Unit'
MM_PER_UNIT = 1000.0
# And one of Tequila's own, first: which of the part's axes the print was
# built along.
AXIS_INPUT = {'name': 'Build Along Z', 'default': 1.0, 'min': 0.0, 'max': 1.0,
              'description': '1: the part was printed along its own Z, as STL, STEP and IGES files are drawn (stood up '
                             'with Turn, Upright or not). 0: along its Y, as glTF, OBJ and FBX files are'}
inputs = [AXIS_INPUT] + [i for i in group['interface'] if i['in_out'] == 'INPUT' and i['name'] != HIDDEN_INPUT]
outputs = [i for i in group['interface'] if i['in_out'] == 'OUTPUT']
OUTPUT_NAMES = {'Normal': 'Bump'}
INPUT_TIPS = {
    'Layer Height (mm)': "The printer's layer height",
    'Z Banding Period (mm)': 'How far apart the bands are',
    'Bump Strength': '1 is the real depth',
}
OUTPUT_TIPS = {
    'Normal': 'The surface relief, a height in scene units: wire it to the material\'s Bump',
    'Tangent': 'Up the wall, in the part\'s own space: wire it to Tangent for the anisotropic sheen along the layers',
    'Position (mm)': 'The point in the print\'s frame, in millimetres: X and Y across the bed, Z up the build',
    'Height (mm)': 'The relief in millimetres, before the bump distance',
    'Top Mask': '1 on up-facing flats',
    'Bottom Mask': '1 on down-facing flats (the bed side)',
}

MATH = {
    'ADD': 'Add', 'SUBTRACT': 'Subtract', 'MULTIPLY': 'Multiply', 'DIVIDE': 'Divide', 'MULTIPLY_ADD': 'MultiplyAdd',
    'POWER': 'Power', 'LOGARITHM': 'Logarithm', 'SQRT': 'Sqrt', 'INVERSE_SQRT': 'InvSqrt', 'ABSOLUTE': 'Absolute',
    'EXPONENT': 'Exponent', 'MINIMUM': 'Minimum', 'MAXIMUM': 'Maximum', 'LESS_THAN': 'LessThan',
    'GREATER_THAN': 'GreaterThan', 'SIGN': 'Sign', 'COMPARE': 'Compare', 'SMOOTH_MIN': 'SmoothMin',
    'SMOOTH_MAX': 'SmoothMax', 'ROUND': 'Round', 'FLOOR': 'Floor', 'CEIL': 'Ceil', 'TRUNC': 'Trunc',
    'FRACT': 'Fraction', 'MODULO': 'Modulo', 'FLOORED_MODULO': 'FlooredModulo', 'WRAP': 'Wrap', 'SNAP': 'Snap',
    'PINGPONG': 'PingPong', 'SINE': 'Sine', 'COSINE': 'Cosine', 'TANGENT': 'Tangent', 'ARCSINE': 'Arcsine',
    'ARCCOSINE': 'Arccosine', 'ARCTANGENT': 'Arctangent', 'ARCTAN2': 'Arctan2', 'SINH': 'Sinh', 'COSH': 'Cosh',
    'TANH': 'Tanh', 'RADIANS': 'Radians', 'DEGREES': 'Degrees',
}
VECTOR_MATH = {
    'ADD': 'Add', 'SUBTRACT': 'Subtract', 'MULTIPLY': 'Multiply', 'DIVIDE': 'Divide', 'CROSS_PRODUCT': 'CrossProduct',
    'DOT_PRODUCT': 'DotProduct', 'DISTANCE': 'Distance', 'LENGTH': 'Length', 'SCALE': 'Scale',
    'NORMALIZE': 'Normalize', 'ABSOLUTE': 'Absolute', 'MINIMUM': 'Minimum', 'MAXIMUM': 'Maximum',
}
# Blender socket identifiers to Tequila's input order, by node type.
IN_INDEX = {
    'MATH': {'Value': 0, 'Value_001': 1, 'Value_002': 2},
    'VECT_MATH': {'Vector': 0, 'Vector_001': 1, 'Vector_002': 2, 'Scale': 3},
    'SEPXYZ': {'Vector': 0},
    'COMBXYZ': {'X': 0, 'Y': 1, 'Z': 2},
    'TEX_NOISE': {'Vector': 0, 'W': 1, 'Scale': 2, 'Detail': 3, 'Roughness': 4, 'Lacunarity': 5, 'Offset': 6,
                  'Gain': 7, 'Distortion': 8},
    'TEX_WHITE_NOISE': {'Vector': 0, 'W': 1},
    'SEPARATE_COLOR': {'Color': 0},
    'MAP_RANGE': {'Value': 0, 'From Min': 1, 'From Max': 2, 'To Min': 3, 'To Max': 4, 'Steps': 5},
}
OUT_INDEX = {
    'MATH': {'Value': 0},
    'VECT_MATH': {'Vector': 0, 'Value': 0},
    'SEPXYZ': {'X': 0, 'Y': 1, 'Z': 2},
    'COMBXYZ': {'Vector': 0},
    'TEX_NOISE': {'Fac': 0, 'Color': 1},
    'TEX_WHITE_NOISE': {'Value': 0, 'Color': 1},
    'SEPARATE_COLOR': {'Red': 0, 'Green': 1, 'Blue': 2},
    'MAP_RANGE': {'Result': 0},
}


def lit(v):
    if isinstance(v, list):
        return 'Value::Vector(vec3({}, {}, {}))'.format(*(f32(x) for x in v[:3]))
    return f'Value::Float({f32(v)})'


def f32(v):
    """The shortest literal that is the same f32."""
    v = float(v)
    single = struct.unpack('f', struct.pack('f', v))[0]
    for digits in range(1, 10):
        s = '%.*g' % (digits, single)
        if struct.unpack('f', struct.pack('f', float(s)))[0] == single:
            break
    if single == 0.0 or 1e-4 <= abs(single) < 1e7:
        s = repr(float(s))
    elif 'e' in s:
        mantissa, exponent = s.split('e')
        s = (mantissa if '.' in mantissa else mantissa + '.0') + 'e' + str(int(exponent))
    elif '.' not in s:
        s += '.0'
    return s


lines = []
emit = lines.append
ids = {}          # Blender node name -> Rust variable
count = [0]


def new(var_kind, literals=()):
    count[0] += 1
    var = f'n{count[0]}'
    lits = ', '.join(f'({i}, {v})' for i, v in literals)
    emit(f'    let {var} = e.node({var_kind}, &[{lits}]);')
    return var


def shader(expr, literals=()):
    return new(f'NodeKind::Node({expr})', literals)


# What a link from (Blender node, output identifier) comes from in Tequila:
# (variable, output index), or a group input's index.
source = {}

MIX_VECTOR = 'ShaderNode::Mix { data: MixData::Vector, blend: BlendMode::Mix, clamp_factor: true, clamp_result: false }'

# Build Along Z, as 0 or 1.
along_z = shader('ShaderNode::Math { op: MathOp::Round, clamp: true }')
emit(f'    e.read(0, {along_z}, 0);')


def swizzled(var, out, turn):
    """`var`'s output `out` with its components rearranged: `turn` is, for
    each of x, y and z, (which component, negated)."""
    sep = shader('ShaderNode::SeparateXyz')
    emit(f'    e.link({var}, {out}, {sep}, 0);')
    comb = shader('ShaderNode::CombineXyz')
    for axis, (component, negate) in enumerate(turn):
        if negate:
            neg = shader('ShaderNode::Math { op: MathOp::Multiply, clamp: false }', [(1, 'Value::Float(-1.0)')])
            emit(f'    e.link({sep}, {component}, {neg}, 0);')
            emit(f'    e.link({neg}, 0, {comb}, {axis});')
        else:
            emit(f'    e.link({sep}, {component}, {comb}, {axis});')
    return comb


def by_axis(var, out, turn):
    """`var`'s output as it is when built along Z, turned by `turn` when
    built along Y."""
    y_built = swizzled(var, out, turn)
    mix = shader(MIX_VECTOR)
    emit(f'    e.link({along_z}, 0, {mix}, 0);')
    emit(f'    e.link({y_built}, 0, {mix}, 1);')
    emit(f'    e.link({var}, {out}, {mix}, 2);')
    return mix


# Into the print's frame from a part built along its Y: (x, -z, y).
INTO_PRINT = ((0, False), (2, True), (1, False))
# And back: (x, z, -y).
OUT_OF_PRINT = ((0, False), (2, False), (1, True))

# The part's position and normal, in the print's frame.
coords = shader('ShaderNode::TextureCoordinate')
turned = {name: by_axis(coords, out, INTO_PRINT) for name, out in (('Object', 3), ('Normal', 1))}

for n in group['nodes']:
    t = n['type']
    name = n['name']
    if t in ('FRAME', 'GROUP_INPUT', 'GROUP_OUTPUT'):
        continue
    if t == 'TEX_COORD':
        source[(name, 'Object')] = (turned['Object'], 0)
        source[(name, 'Normal')] = (turned['Normal'], 0)
        continue
    if t == 'NEW_GEOMETRY':
        source[(name, 'Normal')] = (turned['Normal'], 0)
        continue
    if t == 'VECT_TRANSFORM':
        v = next(i['default'] for i in n['inputs'] if i['id'] == 'Vector')
        var = new(f'NodeKind::Constant({lit(v)})')
        source[(name, 'Vector')] = (var, 0)
        continue
    if t == 'BUMP':
        # Height (mm) times Distance (mm to scene units), as a height.
        var = shader('ShaderNode::Math { op: MathOp::Multiply, clamp: false }')
        bump = new('NodeKind::Bump { depth: 1.0 }')
        emit(f'    e.link({var}, 0, {bump}, 0);')
        ids[name] = var
        source[(name, 'Normal')] = (bump, 0)
        continue
    p = n['props']
    if t == 'MATH':
        expr = f"ShaderNode::Math {{ op: MathOp::{MATH[p['operation']]}, clamp: {str(p['use_clamp']).lower()} }}"
    elif t == 'VECT_MATH':
        expr = f"ShaderNode::VectorMath {{ op: VectorMathOp::{VECTOR_MATH[p['operation']]} }}"
    elif t == 'SEPXYZ':
        expr = 'ShaderNode::SeparateXyz'
    elif t == 'COMBXYZ':
        expr = 'ShaderNode::CombineXyz'
    elif t == 'TEX_NOISE':
        assert p['noise_type'] == 'FBM' and p['noise_dimensions'] == '3D', n
        expr = f"ShaderNode::Noise {{ dims: 3, fractal: FractalKind::Fbm, normalize: {str(p['normalize']).lower()} }}"
    elif t == 'TEX_WHITE_NOISE':
        dims = {'1D': 1, '2D': 2, '3D': 3, '4D': 4}[p['noise_dimensions']]
        expr = f'ShaderNode::WhiteNoise {{ dims: {dims} }}'
    elif t == 'SEPARATE_COLOR':
        assert p['mode'] == 'RGB'
        expr = 'ShaderNode::SeparateColor { model: ColorModel::Rgb }'
    elif t == 'MAP_RANGE':
        assert p['data_type'] == 'FLOAT', n
        kind = {'LINEAR': 'Linear', 'STEPPED': 'Stepped', 'SMOOTHSTEP': 'Smoothstep', 'SMOOTHERSTEP': 'Smootherstep'}[p['interpolation_type']]
        expr = f"ShaderNode::MapRange {{ kind: MapRangeKind::{kind}, clamp: {str(p['clamp']).lower()} }}"
    else:
        raise SystemExit(f'no translation for {t} ({name})')
    # The literals: every unlinked input Tequila has, as the group set it.
    literals = []
    table = IN_INDEX[t]
    for i in n['inputs']:
        if i['linked'] or i['id'] not in table or i['default'] is None:
            continue
        literals.append((table[i['id']], lit(i['default'])))
    var = shader(expr, literals)
    ids[name] = var
    for out_id, index in OUT_INDEX[t].items():
        source[(name, out_id)] = (var, index)

input_index = {i['name']: k for k, i in enumerate(inputs)}
output_index = {o['name']: k for k, o in enumerate(outputs)}

for l in group['links']:
    frm, to = nodes[l['from']], nodes[l['to']]
    if to['type'] == 'GROUP_OUTPUT':
        o = output_index[l['to_socket_name']]
        if l['to_socket_name'] == 'Tangent':
            # Back into the part's own frame.
            var, out = source[(l['from'], l['from_socket'])]
            emit(f'    e.write({o}, {by_axis(var, out, OUT_OF_PRINT)}, 0);')
        else:
            var, out = source[(l['from'], l['from_socket'])]
            emit(f'    e.write({o}, {var}, {out});')
        continue
    # Where it goes.
    if to['type'] == 'BUMP':
        index = {'Height': 0, 'Distance': 1}.get(l['to_socket_name'])
        if index is None:
            continue
        target = ids[l['to']]
    else:
        target = ids[l['to']]
        index = IN_INDEX[to['type']][l['to_socket']]
    if frm['type'] == 'GROUP_INPUT':
        if l['from_socket_name'] == HIDDEN_INPUT:
            emit(f'    e.set({target}, {index}, Value::Float({f32(MM_PER_UNIT)}));')
        else:
            emit(f'    e.read({input_index[l["from_socket_name"]]}, {target}, {index});')
        continue
    var, out = source[(l['from'], l['from_socket'])]
    emit(f'    e.link({var}, {out}, {target}, {index});')


blender_nodes = sum(1 for n in group['nodes'] if n['type'] not in ('GROUP_INPUT', 'GROUP_OUTPUT'))


def rust_str(s):
    return json.dumps(s, ensure_ascii=False)


header = f'''//! FDM Layer Lines: the surface of a print from a filament (FDM) printer,
//! node for node from the "3DP Layer Lines" group of the PLA 3D Print
//! materials. Generated by `scripts/fdm/generate_layer_lines.py`; change the
//! generator, not this file.
//!
//! {blender_nodes} Blender nodes make {count[0]} here: turning the coordinates of a part
//! built along its Y into the print's frame, and the tangent back, adds a few.

use super::{{Expansion, GroupInput, GroupOutput}};
use crate::graph::{{NodeKind, SocketType, Value}};
use crate::nodes::{{MixData, ShaderNode}};
use tequila_core::math::vec3;
use tequila_texture::cycles::noise::FractalKind;
use tequila_texture::cycles::ops::{{BlendMode, ColorModel, MapRangeKind, MathOp, VectorMathOp}};

pub(super) const INPUTS: &[GroupInput] = &[
'''
for i in inputs:
    header += (f"    GroupInput {{ name: {rust_str(i['name'])}, default: {f32(i['default'])}, min: {f32(i['min'])}, "
               f"max: {f32(i['max'])}, tip: {rust_str(i['description'] or INPUT_TIPS.get(i['name'], ''))} }},\n")
header += '];\n\npub(super) const OUTPUTS: &[GroupOutput] = &[\n'
for o in outputs:
    ty = {'NodeSocketFloat': 'Float', 'NodeSocketVector': 'Vector'}[o['socket_type']]
    if o['name'] == 'Normal':
        ty = 'Float'
    tip = OUTPUT_TIPS.get(o['name'], o['description'])
    header += (f"    GroupOutput {{ name: {rust_str(OUTPUT_NAMES.get(o['name'], o['name']))}, ty: SocketType::{ty}, "
               f"tip: {rust_str(tip)} }},\n")
header += '];\n\npub(super) fn build() -> Expansion {\n    let mut e = Expansion::new(INPUTS.len(), OUTPUTS.len());\n'
footer = '    e\n}\n'
with open(OUT, 'w', encoding='utf-8', newline='\n') as f:
    f.write(header + '\n'.join(lines) + '\n' + footer)
print('wrote', OUT, count[0], 'nodes')
