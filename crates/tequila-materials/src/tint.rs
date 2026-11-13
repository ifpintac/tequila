//! The same material in another colour, wherever its colour is kept.
//!
//! A plain plastic keeps its colour in `base_color` and nowhere else. A
//! leather keeps it in a node that multiplies a grain by a brown; a denim in
//! the three blues its threads are mixed from; a wood in the stops of a
//! ramp. Changing `base_color` on any of those changes nothing a person can
//! see, because the graph drives the parameter and the value under it is
//! never read. [`tint`] is the one step that finds every colour the material
//! is seen by and moves them together.
//!
//! The rule, in the order it is applied:
//!
//! 1. The material has one main colour: what it glows with if it glows, what
//!    it lets through if it is clear, and its base colour otherwise. The
//!    colour asked for is what that becomes.
//! 2. Every other colour of the same family - the darker creases of a
//!    leather, the paler threads of a denim, the sheen of a suede, the glow
//!    inside a wax - keeps its relation to the main one: as much lighter or
//!    darker as it was, as much more or less saturated, as far round the
//!    colour wheel. A pattern keeps its contrast, and is not flattened into
//!    one colour.
//! 3. A colour that is not of the family is left exactly as it is: the
//!    white threads of a blue denim stay white in a red one, and the rust on
//!    a grey steel stays rust on a blue one. The family of a coloured main
//!    colour is what shares its hue; the family of a grey one is the greys,
//!    which have no hue to keep and take the new colour whole, so a black
//!    leather made green has green creases.
//! 4. A clear material is seen through its thickness: its colour is what
//!    one `transmission_depth` of it lets through, and a part several times
//!    that thick is far darker and deeper. So a clear material that already
//!    has a colour keeps its clarity: the colour asked for is thinned, as a
//!    dye is, until it takes as much out of the light as the old one did,
//!    and a pale blue acrylic asked for in amber is a pale amber acrylic,
//!    not a near-black one. One with no colour yet, a clear glass, takes
//!    the colour as it is given.
//!
//! The arithmetic is done in Oklab, where equal steps look equal, on the
//! colour as it is seen: a metal's colour is its measured reflectance times
//! its base colour, so that product is what is matched.

use crate::graph::{MaterialGraph, Node, NodeId, NodeKind, Socket, SocketType, Value, LAYER_INPUT};
use crate::material_type::{MaterialType, Param, ParamValue};
use crate::nodes::{MixData, ShaderNode};
use crate::openpbr::Material;
use std::collections::BTreeMap;
use tequila_core::math::{vec2, vec3};
use tequila_core::spectrum::Spectrum;
use tequila_texture::cycles::ops::{BlendMode, MathOp};

/// What a tint did, for the answer a script or an assistant is given.
#[derive(Clone, Debug, PartialEq)]
pub struct Tint {
    /// The main colour as it was seen before.
    pub was: Spectrum,
    /// The main colour as it is seen now: the colour asked for, unless a
    /// metal could not reach it ([`Tint::limited`]) or a clear material was
    /// kept as clear as it was ([`Tint::thinned`]).
    pub now: Spectrum,
    /// The parameter the main colour is: `base_color` for most.
    pub main: Param,
    /// Every parameter whose colours changed, the main one first.
    pub changed: Vec<Changed>,
    /// A metal reflects no more than the metal itself does, and the colour
    /// asked for was brighter than that: `now` is as near as it gets.
    pub limited: bool,
    /// A clear material that had a colour was kept as clear as it was:
    /// `now` is the colour asked for, thinned as a dye is until it takes as
    /// much out of the light as the old colour did.
    pub thinned: bool,
    /// Part of the colour comes from a picture, which was left as it is.
    pub pictures: bool,
    /// Part of the colour is worked out by nodes with no colour to set (a
    /// noise's own colours, a vertex colour), and was left as it is.
    pub computed: bool,
}

/// One parameter's share of a [`Tint`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Changed {
    pub param: Param,
    /// How many colours were changed: 1 for a plain value, more for a
    /// pattern.
    pub colors: usize,
    /// Whether they sit in the graph driving the parameter rather than in
    /// the parameter's own value.
    pub pattern: bool,
    /// How many more colours the parameter has that were left as they are,
    /// being none of the main colour's family: white threads, a rust.
    pub kept: usize,
}

/// Why a material was not tinted. Its `Display` finishes the sentence
/// "<the material> ...".
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Refused {
    /// The type has no colour parameter at all: a thin film's colours come
    /// from its thickness.
    NoColour(MaterialType),
    /// The colour is a picture's and nothing else.
    Picture(Param),
    /// The colour is worked out by nodes and nothing else.
    Computed(Param),
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::NoColour(kind) => write!(f, "has no colour to change: a {} material has no colour of its own", kind.label()),
            Refused::Picture(param) => write!(f, "has no colour to change: its `{}` comes from a picture (a texture map), which a tint does not repaint", param.key()),
            Refused::Computed(param) => write!(
                f,
                "has no colour to change: its `{}` is worked out by nodes that hold no colour (a noise's own colours, a vertex colour); read it with material recipe and change those",
                param.key()
            ),
        }
    }
}

impl std::error::Error for Refused {}

/// The parameters a material is seen by, in the order one is looked for as
/// the main colour's companions. Specular and coat colours are not here:
/// they are the white of a highlight, not the colour of the thing.
const BODY: [Param; 6] = [
    Param::BaseColor,
    Param::SubsurfaceColor,
    Param::TransmissionColor,
    Param::FuzzColor,
    Param::FlakeColor,
    Param::EmissionColor,
];

/// Make `graph` the same material in `colour`. See the module's notes for
/// the rule. Nothing is changed when it is refused.
pub fn tint(graph: &mut MaterialGraph, material_type: MaterialType, colour: Spectrum) -> Result<Tint, Refused> {
    // On a copy, so that a material that is refused is left as it was.
    let mut tinted = graph.clone();
    let done = tint_in_place(&mut tinted, material_type, colour)?;
    *graph = tinted;
    Ok(done)
}

fn tint_in_place(graph: &mut MaterialGraph, material_type: MaterialType, colour: Spectrum) -> Result<Tint, Refused> {
    let colour = colour.sanitized().clamp(0.0, 1.0);
    let output = graph.output();
    let main = main_param(graph, material_type).ok_or(Refused::NoColour(material_type))?;

    // Every surface the material is made of: its own, and any mixed over it.
    let mut surfaces = vec![output];
    while let Some(next) = layer_under(graph, *surfaces.last().expect("never empty")) {
        if surfaces.contains(&next) {
            break;
        }
        surfaces.push(next);
    }

    // A main colour that is only a grey level - a pattern of numbers wired
    // straight into it, as a cast iron's mottle is - is given a colour to
    // change.
    let material = graph.material().clone();
    let lead = colours_of(graph, output, &material, main);
    if lead.driven && lead.places.is_empty() && !lead.pictures && !lead.computed {
        colour_a_grey_level(graph, main, lead.own);
    }

    // What there is to change, parameter by parameter.
    let mut found: Vec<(NodeId, Param, Found)> = Vec::new();
    for &surface in &surfaces {
        let material = surface_material(graph, surface).expect("a surface").clone();
        for param in BODY {
            let wanted = (surface == output && param == main) || is_live(graph, surface, &material, param);
            if wanted {
                found.push((surface, param, colours_of(graph, surface, &material, param)));
            }
        }
    }

    // The main colour as it stands: its own value, or for a pattern the
    // colour the pattern is a pattern of.
    let (_, _, lead) = found.iter().find(|(s, p, _)| *s == output && *p == main).expect("the main colour was looked for");
    if lead.driven && lead.places.is_empty() {
        return Err(if lead.pictures { Refused::Picture(main) } else { Refused::Computed(main) });
    }
    let was = lead.anchor();
    let asked = colour;
    let colour = if main == Param::TransmissionColor { as_clear_as(was, colour) } else { colour };
    let thinned = !same(asked, colour);

    let shift = Shift::new(was, colour, found.iter().flat_map(|(_, _, f)| f.seen()));
    let mut limited = false;
    let mut now = colour;

    // One new colour for each place, however many parameters reach it: a
    // marble's base and its subsurface are driven by the same nodes.
    let mut writes: BTreeMap<Place, Spectrum> = BTreeMap::new();
    let mut changed = Vec::new();
    for (surface, param, f) in &found {
        let lead = *surface == output && *param == main;
        let (mut moved, mut kept) = (0usize, 0usize);
        let mut count = |was: Spectrum, is: Spectrum| if same(was, is) { kept += 1 } else { moved += 1 };
        for &(place, value) in &f.places {
            let new = *writes.entry(place).or_insert_with(|| {
                // A colour of another family is not touched at all, where
                // there and back through a metal would move its last digit.
                let seen = value * f.filter;
                let turned = shift.apply(seen);
                if same(turned, seen) {
                    return value;
                }
                let (new, short) = f.unseen(turned);
                limited |= short;
                new
            });
            count(value, new);
        }
        // The parameter's own value, which nothing reads while a graph
        // drives it but which the panel shows and the quick views shade
        // with: the main one becomes the colour asked for.
        let seen = f.own * f.filter;
        let turned = if lead { colour } else { shift.apply(seen) };
        let (new, short) = if same(turned, seen) { (f.own, false) } else { f.unseen(turned) };
        if lead {
            limited |= short;
            now = new * f.filter;
        }
        if !f.driven {
            limited |= short;
            count(f.own, new);
        }
        writes.insert(Place::Value(*surface, *param), new);
        // The main colour is listed even when it was that colour already,
        // as the one colour it is.
        if lead && moved == 0 {
            (moved, kept) = (1, kept.saturating_sub(1));
        }
        if moved > 0 {
            changed.push(Changed { param: *param, colors: moved, pattern: f.driven, kept });
        }
    }
    changed.sort_by_key(|c| c.param != main);
    // One line for a parameter, where several surfaces have it.
    let mut merged: Vec<Changed> = Vec::new();
    for change in changed {
        match merged.iter_mut().find(|m| m.param == change.param) {
            Some(m) => {
                m.colors += change.colors;
                m.kept += change.kept;
                m.pattern |= change.pattern;
            }
            None => merged.push(change),
        }
    }

    // How far light of each colour travels under the surface is a colour of
    // the material too: a red gummy sweet made green with a red glow inside
    // it would be neither.
    for &surface in &surfaces {
        let material = surface_material(graph, surface).expect("a surface");
        let radius = material.subsurface_radius;
        let reach = radius.max_component();
        let uneven = reach > 0.0 && radius.min_component() < reach * 0.98;
        if uneven && !shift.is_none() && is_live(graph, surface, material, Param::SubsurfaceColor) && !drives(graph, surface, Param::SubsurfaceRadius) {
            let turned = shift.apply(radius * (1.0 / reach));
            let scale = reach / turned.max_component().max(1e-6);
            if let Some(material) = surface_material_mut(graph, surface) {
                material.subsurface_radius = turned * scale;
            }
        }
    }

    for (place, value) in writes {
        write(graph, place, value);
    }
    Ok(Tint {
        was,
        now,
        main,
        changed: merged,
        limited,
        thinned,
        pictures: found.iter().any(|(_, _, f)| f.pictures),
        computed: found.iter().any(|(_, _, f)| f.computed),
    })
}

/// The colour `asked` for a clear material that was `was`, as clear as it
/// was: the same dye, thinned until it takes as much out of the light as
/// the old colour did - `asked` raised to a power under one, which is what
/// less of a dye does. Only ever thinned: a colour paler than the old one
/// is taken as it is, and so is any colour for a material that had none (a
/// clear glass lets nearly everything through, and has no depth of colour
/// to keep).
fn as_clear_as(was: Spectrum, asked: Spectrum) -> Spectrum {
    // How much a colour takes out of the light, over its three parts.
    let density = |c: Spectrum| -(c.r.clamp(1e-4, 1.0).ln() + c.g.clamp(1e-4, 1.0).ln() + c.b.clamp(1e-4, 1.0).ln()) / 3.0;
    let (old, new) = (density(was), density(asked));
    // The library's clear glasses, acrylics and water take out less than a
    // twentieth; its palest tinted ones a quarter.
    if old < 0.18 || new <= old {
        return asked;
    }
    let k = old / new;
    Spectrum::from_rendering_rgb(vec3(asked.r.max(1e-4).powf(k), asked.g.max(1e-4).powf(k), asked.b.max(1e-4).powf(k))).clamp(0.0, 1.0)
}

/// Where numbers drive the main colour directly, put a colour between
/// them: the numbers, as a share of the grey the surface's own value names,
/// times that grey. What is seen is the same until the grey is changed, and
/// the grey is now a colour that can be.
fn colour_a_grey_level(graph: &mut MaterialGraph, main: Param, own: Spectrum) {
    let output = graph.output();
    let Some(link) = graph.links().iter().find(|l| l.to == output && l.to_socket == Socket::Param(main)).copied() else {
        return;
    };
    let grey = own.luminance().clamp(1e-3, 1.0);
    let beside = graph.node(link.from).map(|n| n.position).unwrap_or_default();
    let mut share = Node::new(NodeKind::Node(ShaderNode::Math { op: MathOp::Multiply, clamp: false })).with_literal(Socket::Input(1), Value::Float(1.0 / grey));
    share.position = beside + vec2(220.0, 0.0);
    let share = graph.add_node(share);
    graph.connect_output(link.from, link.from_socket, share, Socket::Input(0));
    let mut times = Node::new(NodeKind::Node(ShaderNode::Mix { data: MixData::Color, blend: BlendMode::Multiply, clamp_factor: true, clamp_result: false }))
        .with_literal(Socket::Input(0), Value::Float(1.0))
        .with_literal(Socket::Input(1), Value::Color(Spectrum::splat(grey)));
    times.position = beside + vec2(440.0, 0.0);
    let times = graph.add_node(times);
    graph.connect(share, times, Socket::Input(2));
    graph.connect(times, output, Socket::Param(main));
    if let Some(material) = surface_material_mut(graph, output) {
        main.write(material, ParamValue::Color(Spectrum::splat(grey)));
    }
}

/// Which parameter is the material's main colour, if it has one.
fn main_param(graph: &MaterialGraph, material_type: MaterialType) -> Option<Param> {
    let shown = material_type.exposed();
    let material = graph.material();
    let output = graph.output();
    let glows = material.emission_luminance > 0.0 || drives(graph, output, Param::EmissionLuminance);
    if shown.contains(&Param::EmissionColor) && glows {
        Some(Param::EmissionColor)
    } else if shown.contains(&Param::TransmissionColor) && material.transmission_weight > 0.5 {
        Some(Param::TransmissionColor)
    } else if shown.contains(&Param::BaseColor) {
        Some(Param::BaseColor)
    } else {
        // A type that shows one of them and not the base colour: a light,
        // whose glow is all it has.
        [Param::EmissionColor, Param::TransmissionColor].into_iter().find(|p| shown.contains(p))
    }
}

/// Whether `param` is part of how this surface is seen, so that it should
/// follow the main colour: a sheen colour only where there is a sheen.
fn is_live(graph: &MaterialGraph, surface: NodeId, material: &Material, param: Param) -> bool {
    let on = |weight: f32, by: Param| weight > 0.0 || drives(graph, surface, by);
    match param {
        // Under a wholly clear surface there is no base to see, unless it
        // is a metal's.
        Param::BaseColor => material.transmission_weight < 1.0 || material.base_metalness > 0.0 || drives(graph, surface, Param::TransmissionWeight),
        Param::SubsurfaceColor => on(material.subsurface_weight, Param::SubsurfaceWeight),
        Param::TransmissionColor => on(material.transmission_weight, Param::TransmissionWeight),
        Param::FuzzColor => on(material.fuzz_weight, Param::FuzzWeight),
        Param::FlakeColor => on(material.flake_weight, Param::FlakeWeight),
        Param::EmissionColor => on(material.emission_luminance, Param::EmissionLuminance),
        _ => false,
    }
}

fn drives(graph: &MaterialGraph, surface: NodeId, param: Param) -> bool {
    graph.links().iter().any(|l| l.to == surface && l.to_socket == Socket::Param(param))
}

fn surface_material(graph: &MaterialGraph, surface: NodeId) -> Option<&Material> {
    match &graph.node(surface)?.kind {
        NodeKind::Surface(material) | NodeKind::Shader(material) => Some(material),
        _ => None,
    }
}

fn surface_material_mut(graph: &mut MaterialGraph, surface: NodeId) -> Option<&mut Material> {
    match &mut graph.node_mut(surface)?.kind {
        NodeKind::Surface(material) | NodeKind::Shader(material) => Some(material),
        _ => None,
    }
}

/// The surface mixed over `surface` by a Mix or Add Shader, if one is.
fn layer_under(graph: &MaterialGraph, surface: NodeId) -> Option<NodeId> {
    let mix = graph.links().iter().find(|l| l.to == surface && l.to_socket == LAYER_INPUT)?.from;
    if !matches!(graph.node(mix)?.kind, NodeKind::MixShader { .. }) {
        return None;
    }
    let shader = graph.links().iter().find(|l| l.to == mix && l.to_socket == Socket::Input(1))?.from;
    matches!(graph.node(shader)?.kind, NodeKind::Shader(_)).then_some(shader)
}

/// Where one colour of a material is kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Place {
    /// A surface's own value of a parameter.
    Value(NodeId, Param),
    /// An input of a node, with nothing wired into it.
    Input(NodeId, u8),
    /// A constant.
    Constant(NodeId),
    /// One stop of a colour ramp.
    Stop(NodeId, usize),
}

/// The colours one parameter of one surface is seen by.
#[derive(Clone, Debug)]
struct Found {
    /// The parameter's own value.
    own: Spectrum,
    /// Whether a graph drives it, so that `own` is not what is seen.
    driven: bool,
    /// The colours in that graph.
    places: Vec<(Place, Spectrum)>,
    pictures: bool,
    computed: bool,
    /// What the colours are seen through: a metal's reflectance for its
    /// base colour, and nothing (white) for everything else.
    filter: Spectrum,
}

impl Found {
    /// Every colour here, as seen.
    fn seen(&self) -> impl Iterator<Item = Spectrum> + '_ {
        let own = (!self.driven).then_some(self.own);
        self.places.iter().map(|(_, c)| *c).chain(own).map(|c| c * self.filter)
    }

    /// The value to keep for a colour to be seen as `seen`, and whether
    /// that is more than the filter lets through.
    fn unseen(&self, seen: Spectrum) -> (Spectrum, bool) {
        let part = |s: f32, f: f32| if f > 1e-4 { s / f } else { s };
        let kept = Spectrum::from_rendering_rgb(vec3(part(seen.r, self.filter.r), part(seen.g, self.filter.g), part(seen.b, self.filter.b)));
        (kept.clamp(0.0, 1.0), kept.max_component() > 1.01)
    }

    /// The one colour this parameter is a pattern of: its own value where
    /// nothing drives it. Under a graph the value is not read, but whoever
    /// built the graph usually left the colour they meant there - often a
    /// blend of the pattern's colours, as a denim's is of its threads. So
    /// the main colour is the pattern's own colour nearest that value,
    /// among those of its family that are at least as vivid: the blue
    /// thread of an Oxford cloth and not the white one, though the value
    /// between them is a pale blue nearer the white. A value that has
    /// nothing to do with the pattern (a recipe that set none) is passed
    /// over for the colour nearest the middle of them all. Always one of
    /// the pattern's own, so that it comes out exactly the colour asked for
    /// and the next tint starts from there.
    fn anchor(&self) -> Spectrum {
        if !self.driven || self.places.is_empty() {
            return self.own * self.filter;
        }
        let labs: Vec<Lab> = self.places.iter().map(|(_, c)| Lab::of(*c * self.filter)).collect();
        let own = Lab::of(self.own * self.filter);
        let nearest = |to: Lab, among: &dyn Fn(Lab) -> bool| {
            (0..labs.len()).filter(|i| among(labs[*i])).min_by(|a, b| to.distance(labs[*a]).total_cmp(&to.distance(labs[*b])))
        };
        let vivid = own.vivid();
        let n = labs.len() as f32;
        let sum = labs.iter().fold(Lab { l: 0.0, a: 0.0, b: 0.0 }, |s, c| Lab { l: s.l + c.l, a: s.a + c.a, b: s.b + c.b });
        let middle = Lab { l: sum.l / n, a: sum.a / n, b: sum.b / n };
        let chosen = nearest(own, &|c| kin(own, c) > 0.5 && c.vivid() >= vivid * 0.85)
            .or_else(|| nearest(own, &|c| kin(own, c) > 0.5))
            .or_else(|| nearest(middle, &|_| true))
            .expect("there are places");
        self.places[chosen].1 * self.filter
    }
}

/// Everything `param` of `surface` is coloured by.
fn colours_of(graph: &MaterialGraph, surface: NodeId, material: &Material, param: Param) -> Found {
    let own = match param.read(material) {
        Some(ParamValue::Color(c)) => c,
        _ => Spectrum::ONE,
    };
    // A metal is seen as its measured reflectance times its base colour.
    let metal = param == Param::BaseColor && material.base_metalness > 0.5 && !drives(graph, surface, Param::BaseMetalness);
    let mut found = Found {
        own,
        driven: false,
        places: Vec::new(),
        pictures: false,
        computed: false,
        filter: if metal { material.metal_ior.normal_incidence_reflectance().clamp(0.0, 1.0) } else { Spectrum::ONE },
    };
    if let Some(link) = graph.links().iter().find(|l| l.to == surface && l.to_socket == Socket::Param(param)) {
        found.driven = true;
        follow(graph, link.from, link.from_socket, &mut found, 0);
    }
    found
}

/// Walk back from an output that feeds a colour, noting every colour that
/// can be set on the way. Answers whether there was one.
///
/// Only what carries a colour is followed: a Mix's two colours and not its
/// factor, a ramp's stops and not the pattern read through it. In a product
/// (Multiply, Overlay and their kind) one side is the colour and the other
/// is what shades it, and moving both would move the result twice: the first
/// side that holds a colour is taken as the colour.
fn follow(graph: &MaterialGraph, node: NodeId, output: u8, found: &mut Found, depth: usize) -> bool {
    // A graph is checked for loops when it compiles, not when it is built.
    if depth > 256 {
        return false;
    }
    let Some(n) = graph.node(node) else { return false };
    if n.kind.output_type_at(output) != Some(SocketType::Color) {
        // A number: a grey level, with no colour to change.
        return false;
    }
    let note = |found: &mut Found, place: Place, colour: Spectrum| {
        if !found.places.iter().any(|(p, _)| *p == place) {
            found.places.push((place, colour));
        }
        true
    };
    let input = |found: &mut Found, index: u8| -> bool {
        if let Some(link) = graph.links().iter().find(|l| l.to == node && l.to_socket == Socket::Input(index)) {
            return follow(graph, link.from, link.from_socket, found, depth + 1);
        }
        match n.literals.get(&Socket::Input(index)).copied().or_else(|| n.kind.input_default(index as usize)) {
            Some(Value::Color(c)) => note(found, Place::Input(node, index), c),
            _ => false,
        }
    };
    match &n.kind {
        NodeKind::Constant(Value::Color(c)) => note(found, Place::Constant(node), *c),
        NodeKind::Gradient { gradient } => {
            for (index, stop) in gradient.stops().iter().enumerate() {
                note(found, Place::Stop(node, index), stop.color);
            }
            !gradient.stops().is_empty()
        }
        NodeKind::Reroute { .. } | NodeKind::Clamp { .. } => input(found, 0),
        NodeKind::Mix => {
            let a = input(found, 0);
            let b = input(found, 1);
            a || b
        }
        NodeKind::Multiply | NodeKind::Add => input(found, 0) || input(found, 1),
        NodeKind::Node(ShaderNode::Mix { data: MixData::Color, blend, .. }) => match blend {
            BlendMode::Mix | BlendMode::Darken | BlendMode::Lighten => {
                let a = input(found, 1);
                let b = input(found, 2);
                a || b
            }
            _ => input(found, 1) || input(found, 2),
        },
        NodeKind::Node(ShaderNode::HueSaturationValue) => input(found, 4),
        NodeKind::Node(ShaderNode::BrightContrast | ShaderNode::Gamma) => input(found, 0),
        NodeKind::Node(ShaderNode::Brick { .. }) => {
            let hits = [input(found, 1), input(found, 2), input(found, 3)];
            hits.contains(&true)
        }
        NodeKind::Node(ShaderNode::Checker) => {
            let hits = [input(found, 1), input(found, 2)];
            hits.contains(&true)
        }
        NodeKind::ImageTexture { .. } => {
            found.pictures = true;
            false
        }
        // Numbers passing through a node that happens to carry colours.
        NodeKind::Invert | NodeKind::Remap { .. } | NodeKind::Constant(_) => false,
        // A colour made by the node itself: a noise's, a vertex's.
        NodeKind::VertexColor | NodeKind::Node(_) | NodeKind::Group(_) => {
            found.computed = true;
            false
        }
        _ => false,
    }
}

fn write(graph: &mut MaterialGraph, place: Place, colour: Spectrum) {
    match place {
        Place::Value(surface, param) => {
            if let Some(material) = surface_material_mut(graph, surface) {
                param.write(material, ParamValue::Color(colour));
            }
        }
        Place::Input(node, index) => {
            if let Some(node) = graph.node_mut(node) {
                node.literals.insert(Socket::Input(index), Value::Color(colour));
            }
        }
        Place::Constant(node) => {
            if let Some(node) = graph.node_mut(node) {
                node.kind = NodeKind::Constant(Value::Color(colour));
            }
        }
        Place::Stop(node, index) => {
            if let Some(NodeKind::Gradient { gradient }) = graph.node_mut(node).map(|n| &mut n.kind) {
                let mut stops = gradient.stops().to_vec();
                if let Some(stop) = stops.get_mut(index) {
                    stop.color = colour;
                }
                **gradient = gradient.with_stops(stops);
            }
        }
    }
}

/// A colour in Oklab (Björn Ottosson's, 2020): `l` lightness from 0 to 1,
/// `a` green to red, `b` blue to yellow, laid out so that equal distances
/// look like equal differences.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Lab {
    l: f32,
    a: f32,
    b: f32,
}

impl Lab {
    fn of(colour: Spectrum) -> Lab {
        let c = colour.to_linear_srgb();
        let l = (0.412_221_47 * c.x + 0.536_332_55 * c.y + 0.051_445_995 * c.z).cbrt();
        let m = (0.211_903_5 * c.x + 0.680_699_5 * c.y + 0.107_396_96 * c.z).cbrt();
        let s = (0.088_302_46 * c.x + 0.281_718_85 * c.y + 0.629_978_7 * c.z).cbrt();
        Lab {
            l: 0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            a: 1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            b: 0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        }
    }

    fn color(self) -> Spectrum {
        let l = (self.l + 0.396_337_78 * self.a + 0.215_803_76 * self.b).powi(3);
        let m = (self.l - 0.105_561_346 * self.a - 0.063_854_17 * self.b).powi(3);
        let s = (self.l - 0.089_484_18 * self.a - 1.291_485_5 * self.b).powi(3);
        Spectrum::from_linear_srgb(vec3(
            4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
            -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
            -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
        ))
    }

    /// The colour's hue and saturation as one point: its `a` and `b` over
    /// its lightness, so that every shade of a colour has the same one.
    fn tone(self) -> (f32, f32) {
        if self.l > 0.02 {
            (self.a / self.l, self.b / self.l)
        } else {
            (0.0, 0.0)
        }
    }

    /// How vivid: the length of [`Lab::tone`], 0 for a grey.
    fn vivid(self) -> f32 {
        let (a, b) = self.tone();
        (a * a + b * b).sqrt()
    }

    fn distance(self, other: Lab) -> f32 {
        ((self.l - other.l).powi(2) + (self.a - other.a).powi(2) + (self.b - other.b).powi(2)).sqrt()
    }
}

fn step(low: f32, high: f32, x: f32) -> f32 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How much of a hue a main colour has, 0 a grey to 1 a colour: whether the
/// others turn with it or are carried along beside it.
fn hued(main: Lab) -> f32 {
    step(0.03, 0.10, main.vivid())
}

/// How much `c` is of the family of the main colour `main`, 0 to 1.
///
/// What has a hue near the main colour's own and is not far paler: the
/// deep and pale threads of a denim, a suede's sheen. And beside a main
/// colour that is a grey, or nearly one, the greys: beside a dull blue a
/// grey is half of the family, beside a vivid one it is none of it, and so
/// white threads stay white.
fn kin(main: Lab, c: Lab) -> f32 {
    // A black has no colour to be of another family with.
    if c.l <= 0.02 {
        return 1.0;
    }
    let (ca, cb) = c.tone();
    let (fa, fb) = main.tone();
    let grey = (1.0 - hued(main)) * (1.0 - step(0.06, 0.12, c.vivid()));
    // A hue is only known of a colour that has some colour.
    let known = step(0.015, 0.04, main.vivid());
    if known <= 0.0 {
        return grey;
    }
    let apart = (ca * fb - cb * fa).atan2(ca * fa + cb * fb).abs().to_degrees();
    let coloured = known * step(0.08, 0.2, c.vivid() / main.vivid()) * (1.0 - step(40.0, 75.0, apart));
    coloured.max(grey)
}

fn same(a: Spectrum, b: Spectrum) -> bool {
    (a.r - b.r).abs() < 1e-6 && (a.g - b.g).abs() < 1e-6 && (a.b - b.b).abs() < 1e-6
}

/// The move from one main colour to another, as it applies to any colour
/// of the same material.
#[derive(Clone, Copy, Debug)]
struct Shift {
    from: Lab,
    to: Lab,
    /// The two as they were given: the main colour itself comes out as the
    /// colour asked for to the last digit, not as near as the arithmetic
    /// there and back leaves it.
    exactly: (Spectrum, Spectrum),
    /// [`hued`] of the old main colour.
    hued: f32,
    /// How much of the way above the main colour the lighter colours keep,
    /// where keeping all of it would take the lightest past white.
    headroom: f32,
}

impl Shift {
    fn new(from: Spectrum, to: Spectrum, all: impl Iterator<Item = Spectrum>) -> Shift {
        let exactly = (from, to);
        let (from, to) = (Lab::of(from), Lab::of(to));
        // The lightest colour of the family, as it would come out.
        let mut shift = Shift { from, to, exactly, hued: hued(from), headroom: 1.0 };
        let top = all.map(Lab::of).filter(|c| kin(from, *c) > 0.5).map(|c| Shift::lightness(from, to, c.l)).fold(to.l, f32::max);
        if top > 1.0 {
            shift.headroom = ((1.0 - to.l) / (top - to.l)).clamp(0.0, 1.0);
        }
        shift
    }

    /// A lightness carried across with its ratio to the main colour's kept,
    /// which is what keeps a pattern's contrast: a crease half as bright as
    /// the leather is half as bright as the new leather. From a black there
    /// is no ratio to keep, and the difference is kept instead.
    fn lightness(from: Lab, to: Lab, l: f32) -> f32 {
        if from.l > 0.02 {
            to.l * (l / from.l)
        } else {
            to.l + (l - from.l)
        }
    }

    /// Whether the material is that colour already, so that nothing moves,
    /// not even by a last digit.
    fn is_none(&self) -> bool {
        same(self.exactly.0, self.exactly.1)
    }

    fn apply(&self, colour: Spectrum) -> Spectrum {
        let (old, new) = self.exactly;
        if same(colour, old) {
            return new;
        }
        if self.is_none() {
            return colour;
        }
        let c = Lab::of(colour);
        let kin = kin(self.from, c);
        if kin <= 0.0 {
            return colour;
        }
        let mut l = Shift::lightness(self.from, self.to, c.l);
        if l > self.to.l {
            l = self.to.l + (l - self.to.l) * self.headroom;
        }
        let l = l.clamp(0.0, 1.0);

        let (ca, cb) = c.tone();
        let (fa, fb) = self.from.tone();
        let (ta, tb) = self.to.tone();
        // Carried along: as far from the new main colour as it was from the
        // old one.
        let beside = (ta + ca - fa, tb + cb - fb);
        // Turned with it: the same angle round the wheel from the new main
        // colour, and the same share of its saturation. A grey stays grey.
        let turned = if self.hued > 0.0 {
            let d = fa * fa + fb * fb;
            // c / from, as complex numbers, held to three times as vivid.
            let (mut qa, mut qb) = ((ca * fa + cb * fb) / d, (cb * fa - ca * fb) / d);
            let q = (qa * qa + qb * qb).sqrt();
            if q > 3.0 {
                qa *= 3.0 / q;
                qb *= 3.0 / q;
            }
            (ta * qa - tb * qb, ta * qb + tb * qa)
        } else {
            beside
        };
        let tone = (beside.0 + (turned.0 - beside.0) * self.hued, beside.1 + (turned.1 - beside.1) * self.hued);
        // Half of the family goes half of the way.
        let l = c.l + (l - c.l) * kin;
        let tone = (ca + (tone.0 - ca) * kin, cb + (tone.1 - cb) * kin);

        // Less vivid, a step at a time, until it is a colour a surface can
        // be: no part below nothing or above all of the light.
        let inside = |c: Spectrum| c.min_component() >= -1e-4 && c.max_component() <= 1.0 + 1e-4;
        let at = |k: f32| Lab { l, a: tone.0 * l * k, b: tone.1 * l * k }.color();
        let full = at(1.0);
        if inside(full) {
            return full.clamp(0.0, 1.0);
        }
        let (mut low, mut high) = (0.0, 1.0);
        for _ in 0..12 {
            let middle = (low + high) * 0.5;
            if inside(at(middle)) {
                low = middle;
            } else {
                high = middle;
            }
        }
        at(low).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_texture::gradient::{Gradient, Stop};

    fn srgb(hex: u32) -> Spectrum {
        let part = |shift: u32| tequila_core::spectrum::srgb_eotf(((hex >> shift) & 0xff) as f32 / 255.0);
        Spectrum::from_linear_srgb(vec3(part(16), part(8), part(0)))
    }

    fn close(a: Spectrum, b: Spectrum, within: f32) -> bool {
        (a.r - b.r).abs() <= within && (a.g - b.g).abs() <= within && (a.b - b.b).abs() <= within
    }

    /// Hue in degrees round Oklab's wheel, and how vivid.
    fn hue(c: Spectrum) -> (f32, f32) {
        let lab = Lab::of(c);
        (lab.b.atan2(lab.a).to_degrees().rem_euclid(360.0), (lab.a * lab.a + lab.b * lab.b).sqrt())
    }

    #[test]
    fn oklab_goes_there_and_back() {
        for hex in [0x000000, 0xffffff, 0xc0392b, 0x1f4d2b, 0x2f5fa8, 0x808080, 0xf5e6c8] {
            let c = srgb(hex);
            assert!(close(Lab::of(c).color(), c, 2e-4), "{hex:06x}");
        }
        assert!((Lab::of(Spectrum::ONE).l - 1.0).abs() < 1e-3);
    }

    #[test]
    fn a_plain_material_takes_the_colour_asked_for() {
        let mut material = MaterialType::Plastic.defaults();
        material.base_color = srgb(0xc0392b);
        let mut graph = MaterialGraph::from_material(material);
        let green = srgb(0x1f4d2b);
        let done = tint(&mut graph, MaterialType::Plastic, green).unwrap();
        assert!(close(graph.material().base_color, green, 1e-6));
        assert_eq!(done.main, Param::BaseColor);
        assert_eq!(done.changed, vec![Changed { param: Param::BaseColor, colors: 1, pattern: false, kept: 0 }]);
        assert!(close(done.was, srgb(0xc0392b), 1e-6) && close(done.now, green, 1e-6));
        assert!(!done.limited && !done.thinned && !done.pictures && !done.computed);
    }

    /// A leather as the library builds one: a brown, times a grain. The
    /// brown is in the node, and that is where the green has to go.
    #[test]
    fn a_colour_times_a_pattern_is_changed_in_the_node() {
        let brown = srgb(0x6b4423);
        let mut material = MaterialType::Generic.defaults();
        material.base_color = brown;
        let mut graph = MaterialGraph::from_material(material);
        let grain = graph.add_node(Node::new(NodeKind::Node(ShaderNode::noise())));
        let mix = graph.add_node(
            Node::new(NodeKind::Node(ShaderNode::Mix { data: MixData::Color, blend: BlendMode::Multiply, clamp_factor: true, clamp_result: false }))
                .with_literal(Socket::Input(0), Value::Float(1.0))
                .with_literal(Socket::Input(1), Value::Color(brown)),
        );
        graph.connect(grain, mix, Socket::Input(2));
        let output = graph.output();
        graph.connect(mix, output, Socket::Param(Param::BaseColor));
        let green = srgb(0x1f4d2b);
        let done = tint(&mut graph, MaterialType::Generic, green).unwrap();
        assert_eq!(graph.node(mix).unwrap().literals[&Socket::Input(1)], Value::Color(green));
        assert!(close(graph.material().base_color, green, 1e-6), "the value under the graph follows, for the panel");
        assert_eq!(done.changed, vec![Changed { param: Param::BaseColor, colors: 1, pattern: true, kept: 0 }]);
        graph.compile().expect("still a graph that compiles");
    }

    /// A ramp from a dark brown to a light one, made blue: both stops blue,
    /// the light one still as much lighter.
    #[test]
    fn a_ramp_keeps_its_contrast() {
        let (dark, light) = (srgb(0x4a2f1a), srgb(0x9c6b3f));
        let mut material = MaterialType::Generic.defaults();
        material.base_color = srgb(0x6b4423);
        let mut graph = MaterialGraph::from_material(material);
        let pattern = graph.add_node(Node::new(NodeKind::Node(ShaderNode::noise())));
        let ramp = graph.add_node(Node::new(NodeKind::Gradient { gradient: Box::new(Gradient::new(vec![Stop { at: 0.0, color: dark }, Stop { at: 1.0, color: light }])) }));
        graph.connect(pattern, ramp, Socket::Input(0));
        let output = graph.output();
        graph.connect(ramp, output, Socket::Param(Param::BaseColor));
        let before = Lab::of(light).l / Lab::of(dark).l;
        let blue = srgb(0x2f5fa8);
        let done = tint(&mut graph, MaterialType::Generic, blue).unwrap();
        let NodeKind::Gradient { gradient } = &graph.node(ramp).unwrap().kind else { panic!() };
        let (new_dark, new_light) = (gradient.stops()[0].color, gradient.stops()[1].color);
        let (wanted, _) = hue(blue);
        for stop in [new_dark, new_light] {
            let (h, vivid) = hue(stop);
            assert!((h - wanted).abs() < 12.0 && vivid > 0.04, "a stop is not blue: hue {h}, wanted {wanted}");
        }
        let after = Lab::of(new_light).l / Lab::of(new_dark).l;
        assert!((after / before - 1.0).abs() < 0.03, "contrast {before} became {after}");
        assert_eq!(done.changed, vec![Changed { param: Param::BaseColor, colors: 2, pattern: true, kept: 0 }]);
    }

    /// The white threads of a blue denim stay white when it is made red;
    /// the blue ones turn red.
    #[test]
    fn grey_beside_a_colour_stays_grey() {
        let (blue, white) = (srgb(0x27408b), srgb(0xe8e8e8));
        let mut material = MaterialType::RealCloth.defaults();
        material.base_color = blue;
        material.fuzz_weight = 0.0;
        let mut graph = MaterialGraph::from_material(material);
        let weave = graph.add_node(Node::new(NodeKind::Node(ShaderNode::noise())));
        let mix = graph.add_node(
            Node::new(NodeKind::Node(ShaderNode::Mix { data: MixData::Color, blend: BlendMode::Mix, clamp_factor: true, clamp_result: false }))
                .with_literal(Socket::Input(1), Value::Color(blue))
                .with_literal(Socket::Input(2), Value::Color(white)),
        );
        graph.connect(weave, mix, Socket::Input(0));
        let output = graph.output();
        graph.connect(mix, output, Socket::Param(Param::BaseColor));
        let red = srgb(0xa52a2a);
        let done = tint(&mut graph, MaterialType::RealCloth, red).unwrap();
        let literals = &graph.node(mix).unwrap().literals;
        assert_eq!(literals[&Socket::Input(1)], Value::Color(red));
        assert_eq!(literals[&Socket::Input(2)], Value::Color(white), "the white threads are left as they are");
        assert_eq!(done.changed, vec![Changed { param: Param::BaseColor, colors: 1, pattern: true, kept: 1 }]);
    }

    /// A grey steel with rust on it, made blue: the steel is blue and the
    /// rust is still rust.
    #[test]
    fn a_colour_on_a_grey_is_left_alone() {
        let (steel, rust) = (Spectrum::splat(0.42), srgb(0x8a4a22));
        let mut material = MaterialType::Generic.defaults();
        material.base_color = steel;
        let mut graph = MaterialGraph::from_material(material);
        let where_rusted = graph.add_node(Node::new(NodeKind::Node(ShaderNode::noise())));
        let mix = graph.add_node(
            Node::new(NodeKind::Node(ShaderNode::Mix { data: MixData::Color, blend: BlendMode::Mix, clamp_factor: true, clamp_result: false }))
                .with_literal(Socket::Input(1), Value::Color(steel))
                .with_literal(Socket::Input(2), Value::Color(rust)),
        );
        graph.connect(where_rusted, mix, Socket::Input(0));
        let output = graph.output();
        graph.connect(mix, output, Socket::Param(Param::BaseColor));
        let blue = srgb(0x2f5fa8);
        tint(&mut graph, MaterialType::Generic, blue).unwrap();
        let literals = &graph.node(mix).unwrap().literals;
        assert_eq!(literals[&Socket::Input(1)], Value::Color(blue));
        assert_eq!(literals[&Socket::Input(2)], Value::Color(rust));
    }

    /// A black made green: there is no hue to turn, so the darker and
    /// lighter blacks of the pattern go green beside it.
    #[test]
    fn a_black_pattern_takes_the_colour_whole() {
        let (black, crease, shine) = (Spectrum::splat(0.012), Spectrum::splat(0.006), Spectrum::splat(0.02));
        let mut material = MaterialType::Generic.defaults();
        material.base_color = black;
        let mut graph = MaterialGraph::from_material(material);
        let pattern = graph.add_node(Node::new(NodeKind::Node(ShaderNode::noise())));
        let ramp = graph.add_node(Node::new(NodeKind::Gradient {
            gradient: Box::new(Gradient::new(vec![Stop { at: 0.0, color: crease }, Stop { at: 0.5, color: black }, Stop { at: 1.0, color: shine }])),
        }));
        graph.connect(pattern, ramp, Socket::Input(0));
        let output = graph.output();
        graph.connect(ramp, output, Socket::Param(Param::BaseColor));
        let green = srgb(0x1f4d2b);
        tint(&mut graph, MaterialType::Generic, green).unwrap();
        let NodeKind::Gradient { gradient } = &graph.node(ramp).unwrap().kind else { panic!() };
        let stops: Vec<Spectrum> = gradient.stops().iter().map(|s| s.color).collect();
        assert!(close(stops[1], green, 2e-3), "{:?}", stops[1]);
        let (wanted, _) = hue(green);
        for stop in &stops {
            assert!((hue(*stop).0 - wanted).abs() < 6.0, "{stop:?} is not green");
        }
        assert!(stops[0].luminance() < stops[1].luminance() && stops[1].luminance() < stops[2].luminance());
    }

    /// A suede's sheen is a paler shade of its colour, in a parameter of
    /// its own: it follows, and stays paler.
    #[test]
    fn a_sheen_follows_the_colour() {
        let mut material = MaterialType::Velvet.defaults();
        material.base_color = srgb(0x5a1a22);
        material.fuzz_color = srgb(0xa0646c);
        material.fuzz_weight = 1.0;
        let mut graph = MaterialGraph::from_material(material);
        let navy = srgb(0x1b2a4a);
        let done = tint(&mut graph, MaterialType::Velvet, navy).unwrap();
        let sheen = graph.material().fuzz_color;
        assert!((hue(sheen).0 - hue(navy).0).abs() < 25.0, "the sheen is {sheen:?}");
        assert!(sheen.luminance() > navy.luminance() * 1.5);
        assert_eq!(done.changed.iter().map(|c| c.param).collect::<Vec<_>>(), vec![Param::BaseColor, Param::FuzzColor]);
    }

    /// A clear material's colour is what it lets through; a lamp's what it
    /// gives off.
    #[test]
    fn glass_and_glow_have_their_own_main_colour() {
        let mut graph = MaterialGraph::from_material(MaterialType::GlassSolid.defaults());
        let amber = srgb(0xd98a1e);
        let done = tint(&mut graph, MaterialType::GlassSolid, amber).unwrap();
        assert_eq!(done.main, Param::TransmissionColor);
        assert!(close(graph.material().transmission_color, amber, 1e-6));

        let mut graph = MaterialGraph::from_material(MaterialType::Emissive.defaults());
        let done = tint(&mut graph, MaterialType::Emissive, amber).unwrap();
        assert_eq!(done.main, Param::EmissionColor);
        assert!(close(graph.material().emission_color, amber, 1e-6));
    }

    /// A pale blue acrylic asked for in amber is a pale amber acrylic: as
    /// clear as it was, since what is seen is the colour through the whole
    /// thickness of the part. A clear glass takes the colour as given.
    #[test]
    fn a_tinted_clear_material_stays_as_clear() {
        let mut material = MaterialType::PlasticTransparent.defaults();
        material.transmission_color = tequila_core::spectrum::spectrum(0.4609, 0.5865, 0.8558);
        let density = |c: Spectrum| -(c.r.ln() + c.g.ln() + c.b.ln()) / 3.0;
        let pale = density(material.transmission_color);
        let mut graph = MaterialGraph::from_material(material);
        let amber = srgb(0xd98a1e);
        let done = tint(&mut graph, MaterialType::PlasticTransparent, amber).unwrap();
        let now = graph.material().transmission_color;
        assert!(done.thinned && close(done.now, now, 1e-6));
        assert!((density(now) / pale - 1.0).abs() < 0.01, "it takes out {} where it took out {pale}", density(now));
        assert!(now.luminance() > amber.luminance() * 1.5, "it is no paler than the amber asked for: {now:?}");
        assert!((hue(now).0 - hue(amber).0).abs() < 12.0, "{now:?} is not amber");
        assert!(now.r > now.g && now.g > now.b);
        // A second time it is that colour already.
        tint(&mut graph, MaterialType::PlasticTransparent, amber).unwrap();
        assert!(close(graph.material().transmission_color, now, 1e-5));

        let mut clear = MaterialType::GlassSolid.defaults();
        clear.transmission_color = Spectrum::ONE;
        let mut graph = MaterialGraph::from_material(clear);
        let done = tint(&mut graph, MaterialType::GlassSolid, amber).unwrap();
        assert!(!done.thinned && close(graph.material().transmission_color, amber, 1e-6));
    }

    /// A metal is seen as its measured reflectance times its base colour:
    /// the product is what comes out the colour asked for, and where the
    /// metal cannot be that bright the answer says so.
    #[test]
    fn a_metal_is_matched_as_it_is_seen() {
        let mut material = MaterialType::Metal.defaults();
        material.metal_ior = crate::fresnel::metals::GOLD;
        let seen = material.metal_ior.normal_incidence_reflectance();
        let mut graph = MaterialGraph::from_material(material);
        let blue = srgb(0x1b2a6b);
        let done = tint(&mut graph, MaterialType::Metal, blue).unwrap();
        assert!(close(graph.material().base_color * seen, blue, 2e-3));
        assert!(close(done.now, blue, 2e-3) && !done.limited);
        assert!(close(done.was, seen.clamp(0.0, 1.0), 1e-4), "it was gold: {:?}", done.was);

        let done = tint(&mut graph, MaterialType::Metal, Spectrum::ONE).unwrap();
        assert!(done.limited, "gold cannot be white");
        assert!(done.now.b < 0.9);
    }

    /// Numbers wired straight into the colour: a mottle of greys, with no
    /// colour anywhere. It gets one, looks the same until it is changed,
    /// and a second tint finds it.
    #[test]
    fn a_grey_level_pattern_is_given_a_colour() {
        let mut material = MaterialType::Plastic.defaults();
        material.base_color = Spectrum::splat(0.3);
        let mut graph = MaterialGraph::from_material(material);
        let mottle = graph.add_node(Node::new(NodeKind::Node(ShaderNode::noise())));
        let output = graph.output();
        graph.connect(mottle, output, Socket::Param(Param::BaseColor));
        let green = srgb(0x1f4d2b);
        let done = tint(&mut graph, MaterialType::Plastic, green).unwrap();
        assert!(close(done.was, Spectrum::splat(0.3), 1e-4) && close(done.now, green, 1e-6));
        assert_eq!(done.changed, vec![Changed { param: Param::BaseColor, colors: 1, pattern: true, kept: 0 }]);
        let driver = graph.driver_of(Param::BaseColor).unwrap();
        assert_eq!(graph.node(driver).unwrap().literals[&Socket::Input(1)], Value::Color(green));
        graph.compile().expect("it compiles");
        let nodes = graph.len();
        let again = tint(&mut graph, MaterialType::Plastic, srgb(0xc0392b)).unwrap();
        assert!(close(again.was, green, 1e-6));
        assert_eq!(graph.len(), nodes, "a second tint adds nothing");
    }

    #[test]
    fn a_material_with_no_colour_is_refused_and_left_alone() {
        let mut graph = MaterialGraph::from_material(MaterialType::ThinFilm.defaults());
        let before = graph.clone();
        let refused = tint(&mut graph, MaterialType::ThinFilm, srgb(0xc0392b)).unwrap_err();
        assert_eq!(refused, Refused::NoColour(MaterialType::ThinFilm));
        assert!(refused.to_string().starts_with("has no colour to change"));
        assert_eq!(graph, before);

        // A colour that is a picture's.
        let mut graph = MaterialGraph::from_material(MaterialType::Plastic.defaults());
        let picture = graph.add_node(Node::new(NodeKind::ImageTexture { texture: crate::graph::TextureId(1), fallback: Value::Color(Spectrum::ONE) }));
        let output = graph.output();
        graph.connect(picture, output, Socket::Param(Param::BaseColor));
        let before = graph.clone();
        assert_eq!(tint(&mut graph, MaterialType::Plastic, srgb(0xc0392b)), Err(Refused::Picture(Param::BaseColor)));
        assert_eq!(graph, before);
    }

    /// Tinted twice comes to the same as tinted once with the second
    /// colour's hue: the second tint starts from where the first left the
    /// main colour, not from a value nothing reads.
    #[test]
    fn a_second_tint_starts_from_the_first() {
        let mut material = MaterialType::Generic.defaults();
        material.base_color = srgb(0x6b4423);
        let mut graph = MaterialGraph::from_material(material);
        let pattern = graph.add_node(Node::new(NodeKind::Node(ShaderNode::noise())));
        let ramp = graph.add_node(Node::new(NodeKind::Gradient {
            gradient: Box::new(Gradient::new(vec![Stop { at: 0.0, color: srgb(0x4a2f1a) }, Stop { at: 1.0, color: srgb(0x9c6b3f) }])),
        }));
        graph.connect(pattern, ramp, Socket::Input(0));
        let output = graph.output();
        graph.connect(ramp, output, Socket::Param(Param::BaseColor));
        tint(&mut graph, MaterialType::Generic, srgb(0x2f5fa8)).unwrap();
        let second = tint(&mut graph, MaterialType::Generic, srgb(0x1f4d2b)).unwrap();
        assert!(close(second.was, srgb(0x2f5fa8), 1e-4), "{:?}", second.was);
        let NodeKind::Gradient { gradient } = &graph.node(ramp).unwrap().kind else { panic!() };
        for stop in gradient.stops() {
            assert!((hue(stop.color).0 - hue(srgb(0x1f4d2b)).0).abs() < 12.0);
        }
    }
}
