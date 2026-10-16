"""Write the STEP fixtures for tests/step.rs.

    python make_step_fixtures.py

Hand-built B-reps, so every entity in them is one this script chose and the
tests can state exact expectations:

* two_bodies.step (millimetres): a product "Fixture" with two solids.
  - "Box", 10 x 20 x 30 mm at the origin, red, with its +Z face green.
  - "Pin", a cylinder of radius 5 running 40 mm along +X, centred on
    y = 50, coloured with a pre-defined colour ("blue"). Its placements leave
    out the reference direction (`$`), which ISO 10303-42 allows, and for an
    axis along X that is what truck-stepio 0.3 turned into NaN.
* assembly.step: parts placed by an assembly; see `assembly()`.
* inches.step: the same box, in inches. The file also defines millimetres, as
  the base the inch is converted from, so a reader that takes the first
  length unit it finds rather than the one the context assigns gets it wrong.
"""

import os

HERE = os.path.dirname(os.path.abspath(__file__))


def fmt(v):
    if isinstance(v, float):
        s = repr(v)
        if "e" in s or "E" in s:
            m, e = s.lower().split("e")
            if "." not in m:
                m += "."
            return f"{m}E{int(e)}"
        return s if "." in s else s + "."
    return str(v)


class Step:
    def __init__(self):
        self.lines = []
        self.next = 1

    def add(self, text):
        n = self.next
        self.next += 1
        self.lines.append(f"#{n}={text};")
        return f"#{n}"

    def point(self, p, label=""):
        return self.add(f"CARTESIAN_POINT('{label}',({','.join(fmt(float(c)) for c in p)}))")

    def direction(self, d):
        return self.add(f"DIRECTION('',({','.join(fmt(float(c)) for c in d)}))")

    def placement(self, origin, axis, ref):
        r = self.direction(ref) if ref is not None else "$"
        return self.add(f"AXIS2_PLACEMENT_3D('',{self.point(origin)},{self.direction(axis)},{r})")

    def text(self, name, schema_units):
        header = (
            "ISO-10303-21;\nHEADER;\n"
            "FILE_DESCRIPTION(('Tequila test fixture'),'2;1');\n"
            f"FILE_NAME('{name}','2026-09-27T00:00:00',('Tequila'),(''),'','','');\n"
            "FILE_SCHEMA(('AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }'));\n"
            "ENDSEC;\nDATA;\n"
        )
        return header + "\n".join(self.lines) + "\nENDSEC;\nEND-ISO-10303-21;\n"


def sub(a, b):
    return tuple(x - y for x, y in zip(a, b))


def cross(a, b):
    return (a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0])


def unit(v):
    n = sum(c * c for c in v) ** 0.5
    return tuple(c / n for c in v)


def colour_style(s, colour):
    """The AP214 chain from a colour to a presentation style assignment."""
    fasc = s.add(f"FILL_AREA_STYLE_COLOUR('',{colour})")
    fas = s.add(f"FILL_AREA_STYLE('',({fasc}))")
    ssfa = s.add(f"SURFACE_STYLE_FILL_AREA({fas})")
    sss = s.add(f"SURFACE_SIDE_STYLE('',({ssfa}))")
    ssu = s.add(f"SURFACE_STYLE_USAGE(.BOTH.,{sss})")
    return s.add(f"PRESENTATION_STYLE_ASSIGNMENT(({ssu}))")


def box(s, size):
    """A box from the origin to `size`: returns (brep, the +Z face)."""
    corner = lambda i, j, k: (size[0] * i, size[1] * j, size[2] * k)
    vertices = {}
    for i in (0, 1):
        for j in (0, 1):
            for k in (0, 1):
                vertices[(i, j, k)] = s.add(f"VERTEX_POINT('',{s.point(corner(i, j, k))})")
    # Each face's corners counter-clockwise seen from outside.
    faces = [
        [(0, 0, 0), (0, 1, 0), (1, 1, 0), (1, 0, 0)],
        [(0, 0, 1), (1, 0, 1), (1, 1, 1), (0, 1, 1)],
        [(0, 0, 0), (1, 0, 0), (1, 0, 1), (0, 0, 1)],
        [(0, 1, 0), (0, 1, 1), (1, 1, 1), (1, 1, 0)],
        [(0, 0, 0), (0, 0, 1), (0, 1, 1), (0, 1, 0)],
        [(1, 0, 0), (1, 1, 0), (1, 1, 1), (1, 0, 1)],
    ]
    edges = {}

    def edge(a, b):
        key = tuple(sorted((a, b)))
        if key not in edges:
            p, q = corner(*key[0]), corner(*key[1])
            d = sub(q, p)
            length = sum(c * c for c in d) ** 0.5
            vec = s.add(f"VECTOR('',{s.direction(unit(d))},{fmt(float(length))})")
            line = s.add(f"LINE('',{s.point(p)},{vec})")
            edges[key] = s.add(f"EDGE_CURVE('',{vertices[key[0]]},{vertices[key[1]]},{line},.T.)")
        return edges[key], key[0] == a

    ids = []
    top = None
    for f in faces:
        ps = [corner(*c) for c in f]
        normal = unit(cross(sub(ps[1], ps[0]), sub(ps[2], ps[0])))
        oriented = []
        for a, b in zip(f, f[1:] + f[:1]):
            e, forward = edge(a, b)
            oriented.append(s.add(f"ORIENTED_EDGE('',*,*,{e},{'.T.' if forward else '.F.'})"))
        loop = s.add(f"EDGE_LOOP('',({','.join(oriented)}))")
        bound = s.add(f"FACE_OUTER_BOUND('',{loop},.T.)")
        plane = s.add(f"PLANE('',{s.placement(ps[0], normal, unit(sub(ps[1], ps[0])))})")
        face = s.add(f"ADVANCED_FACE('',({bound}),{plane},.T.)")
        ids.append(face)
        if normal == (0.0, 0.0, 1.0):
            top = face
    shell = s.add(f"CLOSED_SHELL('',({','.join(ids)}))")
    return s.add(f"MANIFOLD_SOLID_BREP('Box',{shell})"), top


def pin(s, y, radius, length):
    """A cylinder along +X from x = 0, every placement's ref_direction left out."""
    centre0, centre1 = (0.0, y, 0.0), (float(length), y, 0.0)
    # ISO 10303-42's first_proj_axis: the omitted reference direction of an
    # X axis is +Y, so angle zero on every circle is at +Y.
    v0 = s.add(f"VERTEX_POINT('',{s.point((0.0, y + radius, 0.0))})")
    v1 = s.add(f"VERTEX_POINT('',{s.point((float(length), y + radius, 0.0))})")
    c0 = s.add(f"CIRCLE('',{s.placement(centre0, (1, 0, 0), None)},{fmt(float(radius))})")
    c1 = s.add(f"CIRCLE('',{s.placement(centre1, (1, 0, 0), None)},{fmt(float(radius))})")
    e0 = s.add(f"EDGE_CURVE('',{v0},{v0},{c0},.T.)")
    e1 = s.add(f"EDGE_CURVE('',{v1},{v1},{c1},.T.)")
    vec = s.add(f"VECTOR('',{s.direction((1, 0, 0))},{fmt(float(length))})")
    seam_line = s.add(f"LINE('',{s.point((0.0, y + radius, 0.0))},{vec})")
    seam = s.add(f"EDGE_CURVE('',{v0},{v1},{seam_line},.T.)")

    def face(edges, surface):
        oriented = [s.add(f"ORIENTED_EDGE('',*,*,{e},{'.T.' if fwd else '.F.'})") for e, fwd in edges]
        loop = s.add(f"EDGE_LOOP('',({','.join(oriented)}))")
        bound = s.add(f"FACE_OUTER_BOUND('',{loop},.T.)")
        return s.add(f"ADVANCED_FACE('',({bound}),{surface},.T.)")

    # Around the side: u runs with the circles, v along X, and u x v points
    # outward, so the loop is the bottom circle forward, up the seam, back
    # along the top circle, down the seam.
    cylinder = s.add(f"CYLINDRICAL_SURFACE('',{s.placement(centre0, (1, 0, 0), None)},{fmt(float(radius))})")
    side = face([(e0, True), (seam, True), (e1, False), (seam, False)], cylinder)
    # The caps face -X and +X; the circles turn about +X.
    cap0 = face([(e0, False)], s.add(f"PLANE('',{s.placement(centre0, (-1, 0, 0), (0, 1, 0))})"))
    cap1 = face([(e1, True)], s.add(f"PLANE('',{s.placement(centre1, (1, 0, 0), (0, 1, 0))})"))
    shell = s.add(f"CLOSED_SHELL('',({side},{cap0},{cap1}))")
    return s.add(f"MANIFOLD_SOLID_BREP('Pin',{shell})")


def product(s, name, inches):
    app = s.add("APPLICATION_CONTEXT('automotive design')")
    s.add(f"APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,{app})")
    context = s.add(f"PRODUCT_CONTEXT('',{app},'mechanical')")
    prod = s.add(f"PRODUCT('{name}','{name}','',({context}))")
    formation = s.add(f"PRODUCT_DEFINITION_FORMATION('','',{prod})")
    def_context = s.add(f"PRODUCT_DEFINITION_CONTEXT('part definition',{app},'design')")
    definition = s.add(f"PRODUCT_DEFINITION('design','',{formation},{def_context})")
    shape = s.add(f"PRODUCT_DEFINITION_SHAPE('','',{definition})")
    mm = s.add("( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) )")
    if inches:
        exponents = s.add("DIMENSIONAL_EXPONENTS(1.,0.,0.,0.,0.,0.,0.)")
        measure = s.add(f"LENGTH_MEASURE_WITH_UNIT(LENGTH_MEASURE(25.4),{mm})")
        length = s.add(f"( CONVERSION_BASED_UNIT('INCH',{measure}) LENGTH_UNIT() NAMED_UNIT({exponents}) )")
    else:
        length = mm
    angle = s.add("( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.) )")
    solid = s.add("( NAMED_UNIT(*) SI_UNIT($,.STERADIAN.) SOLID_ANGLE_UNIT() )")
    uncertainty = s.add(f"UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-07),{length},'distance_accuracy_value','')")
    geometry = s.add(
        f"( GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT(({uncertainty})) "
        f"GLOBAL_UNIT_ASSIGNED_CONTEXT(({length},{angle},{solid})) REPRESENTATION_CONTEXT('','') )"
    )
    return shape, geometry


def two_bodies():
    s = Step()
    shape, context = product(s, "Fixture", inches=False)
    box_brep, top = box(s, (10, 20, 30))
    pin_brep = pin(s, 50.0, 5.0, 40)
    rep = s.add(f"ADVANCED_BREP_SHAPE_REPRESENTATION('',({box_brep},{pin_brep}),{context})")
    s.add(f"SHAPE_DEFINITION_REPRESENTATION({shape},{rep})")
    red = colour_style(s, s.add("COLOUR_RGB('',1.,0.,0.)"))
    green = colour_style(s, s.add("COLOUR_RGB('',0.,1.,0.)"))
    blue = colour_style(s, s.add("DRAUGHTING_PRE_DEFINED_COLOUR('blue')"))
    styled = [
        s.add(f"STYLED_ITEM('color',({red}),{box_brep})"),
        s.add(f"STYLED_ITEM('color',({green}),{top})"),
        s.add(f"STYLED_ITEM('color',({blue}),{pin_brep})"),
    ]
    s.add(f"MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION('',({','.join(styled)}),{context})")
    return s.text("two_bodies.step", None)


def inches():
    s = Step()
    shape, context = product(s, "Inch box", inches=True)
    box_brep, _ = box(s, (1, 2, 3))
    rep = s.add(f"ADVANCED_BREP_SHAPE_REPRESENTATION('',({box_brep}),{context})")
    s.add(f"SHAPE_DEFINITION_REPRESENTATION({shape},{rep})")
    return s.text("inches.step", None)


def assembly():
    """An assembly, as CAD packages write one (AP214's NAUO structure).

    "Assembly" holds two instances of "Box part" and one of "Sub", which
    holds one of "Pin part":

    * the first box at (100, 0, 0);
    * the second at (0, 100, 0), turned 90 degrees about Z, so the box's
      x axis points along y;
    * Sub at (0, 0, 50), and the pin inside it at (5, 0, 0) — with its
      representation relationship written parent-first, which the standard
      allows and some exporters do, so the transformation must be inverted.

    Each part has the usual pair of representations: a plain
    SHAPE_REPRESENTATION holding its placement, and the B-rep's own
    representation, joined by a SHAPE_REPRESENTATION_RELATIONSHIP.
    """
    s = Step()
    app = s.add("APPLICATION_CONTEXT('automotive design')")
    s.add(f"APPLICATION_PROTOCOL_DEFINITION('international standard','automotive_design',2000,{app})")
    product_context = s.add(f"PRODUCT_CONTEXT('',{app},'mechanical')")
    definition_context = s.add(f"PRODUCT_DEFINITION_CONTEXT('part definition',{app},'design')")
    mm = s.add("( LENGTH_UNIT() NAMED_UNIT(*) SI_UNIT(.MILLI.,.METRE.) )")
    angle = s.add("( NAMED_UNIT(*) PLANE_ANGLE_UNIT() SI_UNIT($,.RADIAN.) )")
    solid = s.add("( NAMED_UNIT(*) SI_UNIT($,.STERADIAN.) SOLID_ANGLE_UNIT() )")
    uncertainty = s.add(f"UNCERTAINTY_MEASURE_WITH_UNIT(LENGTH_MEASURE(1.E-07),{mm},'distance_accuracy_value','')")
    context = s.add(
        f"( GEOMETRIC_REPRESENTATION_CONTEXT(3) GLOBAL_UNCERTAINTY_ASSIGNED_CONTEXT(({uncertainty})) "
        f"GLOBAL_UNIT_ASSIGNED_CONTEXT(({mm},{angle},{solid})) REPRESENTATION_CONTEXT('','') )"
    )

    def definition(name):
        prod = s.add(f"PRODUCT('{name}','{name}','',({product_context}))")
        formation = s.add(f"PRODUCT_DEFINITION_FORMATION('','',{prod})")
        return s.add(f"PRODUCT_DEFINITION('design','',{formation},{definition_context})")

    def shape_of(pd, items):
        pds = s.add(f"PRODUCT_DEFINITION_SHAPE('','',{pd})")
        rep = s.add(f"SHAPE_REPRESENTATION('',({','.join(items)}),{context})")
        s.add(f"SHAPE_DEFINITION_REPRESENTATION({pds},{rep})")
        return rep

    def part(name, body):
        pd = definition(name)
        origin = s.placement((0, 0, 0), (0, 0, 1), (1, 0, 0))
        rep = shape_of(pd, [origin])
        brep_rep = s.add(f"ADVANCED_BREP_SHAPE_REPRESENTATION('',({body},{origin}),{context})")
        s.add(f"SHAPE_REPRESENTATION_RELATIONSHIP('','',{rep},{brep_rep})")
        return pd, rep, origin

    def use(parent, child, where, axis, ref, parent_first=False):
        """Place `child` (pd, rep, origin) in `parent` at a new placement."""
        parent_pd, parent_rep, parent_items = parent
        child_pd, child_rep, child_origin = child
        placed = s.placement(where, axis, ref)
        parent_items.append(placed)
        nauo = s.add(f"NEXT_ASSEMBLY_USAGE_OCCURRENCE('','','',{parent_pd},{child_pd},$)")
        pds = s.add(f"PRODUCT_DEFINITION_SHAPE('','',{nauo})")
        if parent_first:
            transform = s.add(f"ITEM_DEFINED_TRANSFORMATION('','',{placed},{child_origin})")
            reps = f"{parent_rep},{child_rep}"
        else:
            transform = s.add(f"ITEM_DEFINED_TRANSFORMATION('','',{child_origin},{placed})")
            reps = f"{child_rep},{parent_rep}"
        relation = s.add(
            f"( REPRESENTATION_RELATIONSHIP('','',{reps}) "
            f"REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION({transform}) SHAPE_REPRESENTATION_RELATIONSHIP() )"
        )
        s.add(f"CONTEXT_DEPENDENT_SHAPE_REPRESENTATION({relation},{pds})")

    box_brep, _ = box(s, (10, 20, 30))
    pin_brep = pin(s, 0.0, 5.0, 40)
    box_part = part("Box part", box_brep)
    pin_part = part("Pin part", pin_brep)

    # Assemblies' representations list their placements, which are added
    # as instances are, so the representation is written last.
    root_pd, root_items = definition("Assembly"), [s.placement((0, 0, 0), (0, 0, 1), (1, 0, 0))]
    sub_pd, sub_items = definition("Sub"), [s.placement((0, 0, 0), (0, 0, 1), (1, 0, 0))]
    root_rep = s.next + 10_000  # placeholder ids, patched below
    sub_rep = s.next + 10_001
    root = (root_pd, f"#{root_rep}", root_items)
    sub = (sub_pd, f"#{sub_rep}", sub_items)
    use(root, box_part, (100, 0, 0), (0, 0, 1), (1, 0, 0))
    use(root, box_part, (0, 100, 0), (0, 0, 1), (0, 1, 0))
    use(sub, pin_part, (5, 0, 0), (0, 0, 1), (1, 0, 0), parent_first=True)
    sub_origin = sub_items[0]
    use(root, (sub_pd, f"#{sub_rep}", sub_origin), (0, 0, 50), (0, 0, 1), (1, 0, 0))
    real_root = shape_of(root_pd, root_items)
    real_sub = shape_of(sub_pd, sub_items)
    s.lines = [line.replace(f"#{root_rep},", f"{real_root},").replace(f"#{root_rep})", f"{real_root})")
               .replace(f"#{sub_rep},", f"{real_sub},").replace(f"#{sub_rep})", f"{real_sub})") for line in s.lines]
    red = colour_style(s, s.add("COLOUR_RGB('',1.,0.,0.)"))
    s.add(f"MECHANICAL_DESIGN_GEOMETRIC_PRESENTATION_REPRESENTATION('',(" + s.add(f"STYLED_ITEM('color',({red}),{box_brep})") + f"),{context})")
    return s.text("assembly.step", None)


for name, text in [("two_bodies.step", two_bodies()), ("inches.step", inches()), ("assembly.step", assembly())]:
    with open(os.path.join(HERE, name), "w", newline="\n") as f:
        f.write(text)
    print("wrote", name)
