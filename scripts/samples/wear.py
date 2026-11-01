"""Handling marks for a glossy surface, as a node graph: faint smudges,
fingerprints and specks of dust, all in its roughness, which is how they
show on clear plastic and glass - in what the surface reflects.

    graph = wear_graph(base=0.035, smudge=0.14, prints=0.2, dust=0.9)

Coordinates are the part's own (Object), in metres, so the marks stay put on
it as it moves. make_samples.py puts them on the palma's shell and screen.
"""


def wear_graph(base, smudge, prints, dust, print_cells=26.0, ridge=450.0, dust_cells=1400.0, smudge_scale=18.0):
    nodes, links = [], []

    def node(kind, settings=None, **literals):
        i = len(nodes) + 2
        lit = [{'socket': f'input:{k[1:]}', 'value': {'float': float(v)}} for k, v in literals.items()]
        nodes.append({'id': i, 'kind': 'node', 'type': kind, 'position': [0, 0], 'literals': lit, 'settings': settings or {}})
        return i

    def link(a, socket, b, output=0):
        entry = {'from': a, 'socket': socket, 'to': b}
        if output:
            entry['output'] = output
        links.append(entry)

    noise = {'dimensions': '3D', 'normalize': True, 'type': 'fBM'}
    voronoi = {'dimensions': '3D', 'feature': 'f1', 'metric': 'euclidean', 'normalize': False}
    ranged = {'clamp': True, 'interpolation': 'linear'}

    coords = node('texture_coordinate')

    # Smudges: broad soft patches, a few centimetres across.
    s = node('noise_texture', noise, i2=smudge_scale, i3=4, i4=0.6)
    link(coords, 'input:0', s, output=3)
    s_mask = node('map_range', ranged, i1=0.52, i2=0.76, i3=0, i4=1)
    link(s, 'input:0', s_mask)

    # Fingerprints: a print in some of the cells, its ridges round the cell's
    # centre, about half a millimetre apart and wavering.
    cells = node('voronoi_texture', voronoi, i2=print_cells)
    link(coords, 'input:0', cells, output=3)
    rgb = node('separate_xyz')
    link(cells, 'input:0', rgb, output=1)
    present = node('map_range', ranged, i1=0.55, i2=0.6, i3=0, i4=1)
    link(rgb, 'input:0', present)
    blob = node('map_range', ranged, i1=0.3, i2=0.17, i3=0, i4=1)
    link(cells, 'input:0', blob)
    waver = node('noise_texture', noise, i2=300, i3=2, i4=0.5)
    link(coords, 'input:0', waver, output=3)
    wavered = node('math', {'clamp': False, 'operation': 'multiply_add'}, i1=0.02)
    link(waver, 'input:0', wavered)
    link(cells, 'input:2', wavered)
    phase = node('math', {'clamp': False, 'operation': 'multiply'}, i1=ridge)
    link(wavered, 'input:0', phase)
    wave = node('math', {'clamp': False, 'operation': 'sine'})
    link(phase, 'input:0', wave)
    ridges = node('map_range', ranged, i1=0.0, i2=0.7, i3=0, i4=1)
    link(wave, 'input:0', ridges)
    in_blob = node('math', {'clamp': False, 'operation': 'multiply'})
    link(ridges, 'input:0', in_blob)
    link(blob, 'input:1', in_blob)
    fp = node('math', {'clamp': False, 'operation': 'multiply'})
    link(in_blob, 'input:0', fp)
    link(present, 'input:1', fp)

    # Dust: a speck in a few of the tiny cells.
    specks = node('voronoi_texture', voronoi, i2=dust_cells)
    link(coords, 'input:0', specks, output=3)
    srgb = node('separate_xyz')
    link(specks, 'input:0', srgb, output=1)
    sparse = node('map_range', ranged, i1=0.965, i2=0.97, i3=0, i4=1)
    link(srgb, 'input:0', sparse)
    dot = node('map_range', ranged, i1=0.25, i2=0.1, i3=0, i4=1)
    link(specks, 'input:0', dot)
    d = node('math', {'clamp': False, 'operation': 'multiply'})
    link(dot, 'input:0', d)
    link(sparse, 'input:1', d)

    # Roughness: the clean surface's, plus each mark's.
    r1 = node('math', {'clamp': False, 'operation': 'multiply_add'}, i1=smudge, i2=base)
    link(s_mask, 'input:0', r1)
    r2 = node('math', {'clamp': False, 'operation': 'multiply_add'}, i1=prints)
    link(fp, 'input:0', r2)
    link(r1, 'input:2', r2)
    r3 = node('math', {'clamp': True, 'operation': 'multiply_add'}, i1=dust)
    link(d, 'input:0', r3)
    link(r2, 'input:2', r3)
    link(r3, 'param:specular_roughness', 1)
    return {'decals': [], 'links': links, 'nodes': nodes, 'output': 1}
