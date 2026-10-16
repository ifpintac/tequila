//! The material graph and its compiler (`docs/02-MATERIAL-SYSTEM.md` §4).
//!
//! ## One model, two views
//!
//! §4.2 is the decision this module exists to enforce, and it is worth stating
//! plainly because it costs something and repays more:
//!
//! > The Material tab's simple parameter panel is a **view over the graph**, not
//! > a separate representation.
//!
//! A material with no textures is a graph holding one [`NodeKind::Surface`] node
//! whose literals *are* the §2.2 parameter block. Switching a parameter to a
//! texture inserts a node and a link; the panel then shows a "connected"
//! indicator where it used to show a slider. Opening the graph editor shows
//! exactly that graph, because there is nothing else to show.
//!
//! The alternative — a simple panel backed by one structure and a graph backed
//! by another, synchronised on edit — is where "I changed it in the graph and
//! the panel still shows the old value" comes from, and every one of those bugs
//! is unfixable in principle rather than merely unfixed. §1 insists on one
//! model for this reason.
//!
//! ## Why compile at all
//!
//! A graph could be walked directly at every shading point. It is not, because
//! a path tracer evaluates materials tens of millions of times per frame and a
//! pointer-chasing tree walk with a hash-map lookup per socket is an order of
//! magnitude slower than a flat instruction list over an array of registers.
//! §4.3 asks for the flat list.
//!
//! ## What the cache key is over
//!
//! §4.3 point 4: "parameter changes that do not alter topology must patch a
//! uniform block, not recompile". So there are two hashes here and the
//! distinction is load-bearing:
//!
//! - [`MaterialGraph::topology_hash`] covers the shape — nodes, kinds, links —
//!   and *not* literal values. Two graphs agreeing on it compile to the same
//!   instructions.
//! - [`MaterialGraph::content_hash`] covers the literals as well, and says
//!   whether the *result* would differ.
//!
//! Dragging a roughness slider changes the second and not the first, which is
//! exactly the case that must not trigger a recompile — sixty times a second,
//! while the user is watching.

use crate::material_type::Param;
use crate::openpbr::Material;
use tequila_core::math::{Vec2, Vec3};
use tequila_core::spectrum::Spectrum;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash, Hasher};

/// A socket's type (§4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SocketType {
    Float,
    Color,
    Vector,
    Normal,
    Bool,
}

impl SocketType {
    /// Whether a link from this type to `target` is allowed.
    ///
    /// §4.1: the conversions Cycles makes between its sockets, so a graph
    /// links as it would in Blender - numbers, colours and vectors to each
    /// other. A flag links to nothing else, a connection error the editor
    /// surfaces rather than a silent coercion.
    pub fn converts_to(self, target: SocketType) -> bool {
        use SocketType::*;
        match (self, target) {
            (a, b) if a == b => true,
            (Float, Color) | (Color, Float) => true,
            // A normal is a vector with a unit-length promise; treating one as
            // the other loses only the promise.
            (Vector, Normal) | (Normal, Vector) => true,
            // As Cycles links them: a number to a vector, a vector to a
            // number (averaged), colours and vectors either way.
            (Float, Vector) | (Float, Normal) | (Vector, Float) | (Normal, Float) => true,
            (Color, Vector) | (Color, Normal) | (Vector, Color) | (Normal, Color) => true,
            _ => false,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SocketType::Float => "Float",
            SocketType::Color => "Color",
            SocketType::Vector => "Vector",
            SocketType::Normal => "Normal",
            SocketType::Bool => "Bool",
        }
    }
}

/// A value flowing through the graph.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Value {
    Float(f32),
    Color(Spectrum),
    Vector(Vec3),
    Bool(bool),
}

impl Value {
    pub fn socket_type(&self) -> SocketType {
        match self {
            Value::Float(_) => SocketType::Float,
            Value::Color(_) => SocketType::Color,
            Value::Vector(_) => SocketType::Vector,
            Value::Bool(_) => SocketType::Bool,
        }
    }

    /// Apply §4.1's implicit conversions.
    pub fn convert(self, target: SocketType) -> Option<Value> {
        use SocketType::*;
        Some(match (self, target) {
            (v, t) if v.socket_type() == t => v,
            (Value::Float(f), Color) => Value::Color(Spectrum::splat(f)),
            (Value::Color(c), Float) => Value::Float(c.luminance()),
            (Value::Vector(v), Normal) | (Value::Vector(v), Vector) => Value::Vector(v),
            // As Cycles converts between its sockets: a number spread over a
            // vector, a vector averaged to a number, colours and vectors
            // taken component for component.
            (Value::Float(f), Vector) | (Value::Float(f), Normal) => Value::Vector(Vec3 { x: f, y: f, z: f }),
            (Value::Vector(v), Float) => Value::Float((v.x + v.y + v.z) / 3.0),
            (Value::Color(c), Vector) | (Value::Color(c), Normal) => Value::Vector(Vec3 { x: c.r, y: c.g, z: c.b }),
            (Value::Vector(v), Color) => Value::Color(Spectrum { r: v.x, g: v.y, b: v.z }),
            _ => return None,
        })
    }

    pub fn as_float(self) -> f32 {
        match self {
            Value::Float(f) => f,
            Value::Color(c) => c.luminance(),
            Value::Vector(v) => v.x,
            Value::Bool(b) => {
                if b {
                    1.0
                } else {
                    0.0
                }
            }
        }
    }

    pub fn as_vector(self) -> Vec3 {
        match self {
            Value::Float(f) => Vec3 { x: f, y: f, z: f },
            Value::Color(c) => Vec3 { x: c.r, y: c.g, z: c.b },
            Value::Vector(v) => v,
            Value::Bool(b) => {
                let f = if b { 1.0 } else { 0.0 };
                Vec3 { x: f, y: f, z: f }
            }
        }
    }

    pub fn as_color(self) -> Spectrum {
        match self {
            Value::Float(f) => Spectrum::splat(f),
            Value::Color(c) => c,
            Value::Vector(v) => Spectrum {
                r: v.x,
                g: v.y,
                b: v.z,
            },
            Value::Bool(b) => Spectrum::splat(if b { 1.0 } else { 0.0 }),
        }
    }
}

/// Which of a colour's three numbers to take.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Channel {
    #[default]
    Red,
    Green,
    Blue,
}

impl Channel {
    pub const ALL: [Channel; 3] = [Channel::Red, Channel::Green, Channel::Blue];

    pub fn label(self) -> &'static str {
        match self {
            Channel::Red => "Red",
            Channel::Green => "Green",
            Channel::Blue => "Blue",
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Channel::Red => "red",
            Channel::Green => "green",
            Channel::Blue => "blue",
        }
    }

    pub fn from_id(text: &str) -> Option<Channel> {
        Channel::ALL.into_iter().find(|c| c.id() == text)
    }

    #[inline]
    pub fn of(self, colour: Spectrum) -> f32 {
        match self {
            Channel::Red => colour.r,
            Channel::Green => colour.g,
            Channel::Blue => colour.b,
        }
    }
}

/// A handle into the scene's texture table.
///
/// The graph refers to textures rather than holding them, so that two materials
/// using the same file share one decoded copy and one cache entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TextureId(pub u32);

/// A node's identity. Never reused, so a stale link is detectable rather than
/// silently pointing at a different node.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(pub u32);

/// Where a link ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Socket {
    /// A §2.2 parameter on the [`NodeKind::Surface`] node.
    Param(Param),
    /// An indexed input on any other node.
    Input(u8),
}

/// The node catalog.
///
/// M2 implements the terminal, the texture path, and the utility nodes §6 calls
/// core. The procedural catalog arrives with M6; adding one is a variant here
/// plus an arm in [`NodeKind::evaluate`], which is the extensibility §1 is
/// after.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// The terminal surface node. Its inputs are the parameter block, and its
    /// literals are the values the simple panel edits (§4.2).
    Surface(Box<Material>),
    /// A bend in a wire (`docs/04` §7): passes its input through unchanged.
    /// Carries the type of what feeds it, so the wire keeps its colour and
    /// its checks. Removed before compiling, so it costs nothing to render.
    Reroute { ty: SocketType },
    /// A Blender shader node (`crate::nodes`): textures, converters, colour
    /// and vector nodes, and the surface inputs they read.
    Node(crate::nodes::ShaderNode),
    /// A literal.
    Constant(Value),
    /// Texture coordinates from the geometry, through a projection.
    Mapping2d {
        mapping: tequila_texture::projection::Mapping,
    },
    /// A colour ramp: a scalar in, a colour out.
    ///
    /// What lets a procedural drive a colour at all. Every pattern in
    /// `tequila-texture` produces one number, which is everything a roughness
    /// needs and nothing a colour does — wood is a grey field read through two
    /// browns, and this is the reading.
    Gradient {
        gradient: Box<tequila_texture::gradient::Gradient>,
    },
    /// One channel of a colour, as a number.
    ///
    /// The node that lets a packed map be used at all. glTF writes roughness
    /// into a texture's green channel and metalness into its blue, and so do
    /// most exporters and most material libraries, because it is three maps in
    /// one file. Without a way to select a channel the packed image can only
    /// be fed to a parameter whole, which converts it by luminance — a blend
    /// of all three that is neither of the things it packs.
    Channel {
        channel: Channel,
    },
    /// A procedural pattern, evaluated from the shading point (§6).
    ///
    /// A sibling of [`NodeKind::ImageTexture`] rather than a kind of it: both
    /// produce a value from the surface, and neither is expressible as the
    /// other. A procedural has no file, no MIP pyramid and no filtering
    /// problem; a texture has no parameters to tune.
    Procedural {
        procedural: Box<tequila_texture::procedural::Procedural>,
        /// So two copies of a part are not the same copy twice (§6).
        seed: u32,
    },
    /// A filtered texture lookup.
    ImageTexture {
        texture: TextureId,
        /// Falls back to this when the texture is missing or still loading.
        fallback: Value,
    },
    /// The alpha channel of a texture, as a number.
    ///
    /// A node of its own rather than a fourth [`Channel`], because alpha is
    /// not in the colour: a texture node outputs three components and the
    /// fourth never reaches the graph. Sampling it separately is also what
    /// keeps [`Channel`] honest — it takes one of three numbers out of a
    /// colour it was handed, and there is no colour here to take it from.
    ///
    /// This is where an alpha-tested surface comes from. glTF's `alphaMode`,
    /// and every foliage or grille texture, keep the cutout in the colour
    /// image's own fourth channel rather than in a separate file.
    ImageAlpha {
        texture: TextureId,
        /// Falls back to this when the texture is missing or still loading.
        fallback: Value,
    },
    /// `a * (1 - t) + b * t`.
    Mix,
    Multiply,
    Add,
    /// `1 - x`.
    Invert,
    /// Rescale from one range to another.
    Remap { from: Vec2, to: Vec2 },
    Clamp { low: f32, high: f32 },
    /// A height, 0 to 1, made into a bump `depth` scene units deep (§5.4).
    ///
    /// What the surface's Bump socket reads is a height *in scene units*, so
    /// that a bump is as deep on a small part as on a large one and does not
    /// change with the texture's resolution or the pattern's scale. A pattern
    /// or a height map is a number from 0 to 1; this is where it is given a
    /// depth. Wired straight into Bump, a 0-to-1 pattern would be a bump a
    /// whole scene unit deep.
    ///
    /// A tangent-space normal map is the exception: wired straight into Bump,
    /// it is read as a normal map and needs no depth.
    Bump { depth: f32 },
    /// §6.2's Curvature: how sharply the surface bends here, as a 0-to-1
    /// mask. The workhorse for automatic edge wear — worn paint and polished
    /// metal on convex edges, grime in the creases.
    ///
    /// `radius` is in scene units: an edge rounded this tightly or tighter
    /// reads as fully convex, a gentler one partly, a flat face not at all.
    Curvature {
        mode: CurvatureMode,
        radius: f32,
        /// About the middle of the mask; 1 leaves it, higher hardens it.
        contrast: f32,
    },
    /// §6.2's Occlusion: how enclosed the surface is by the part itself,
    /// within `radius` scene units, 0 open to 1 closed. Measured once per
    /// vertex of each part that uses it, as the spec suggests for static
    /// geometry, so it costs nothing per sample. Dirt in crevices, darkened
    /// seams, dust that settles where a cloth cannot reach.
    Occlusion {
        radius: f32,
        /// 1 leaves it; higher gathers it into the deepest corners.
        contrast: f32,
    },
    /// §6.2's Vertex Color: the colour a mesh carries per vertex, as a
    /// scanner, a sculpting tool or a glTF's COLOR_0 painted it. White on a
    /// mesh without.
    VertexColor,
    /// §6's Volume Map: a density grid read from an OpenVDB file, fitted to
    /// the part's bounding box, for a medium's Density.
    ///
    /// Its values belong to the part's interior, which only the renderer's
    /// walk through the medium visits, and the walk samples the grid itself
    /// (`tequila_render_core::scene::DensityField`). At the surface it
    /// reads 1: the medium as described.
    VolumeMap { volume: TextureId },
    /// §4.4's Bubbles: air bubbles inside a transmissive part. Wired to
    /// nothing — its presence in the graph is the request, as a geometry
    /// node's is. See `tequila_scene::bubbles` for how they are made.
    Bubbles {
        count: u32,
        /// Mean radius, in scene units.
        radius: f32,
        /// How much radii vary, 0 all the same to 1 from nothing to double.
        variation: f32,
        seed: u32,
    },
    /// §4.4's Fuzz, geometric: real fibres standing out of the surface - a
    /// peach skin, a felt's halo, a tennis ball's nap, a towel - grown as
    /// fine tapering tubes in the part's own material when the scene is
    /// built. Wired to nothing, as Bubbles. The shading-only fuzz is the
    /// surface's own sheen (`fuzz_weight`). See `tequila_scene::fuzz`.
    Fuzz {
        /// A fibre's length, in scene units.
        length: f32,
        /// Fibres to the square centimetre of surface.
        density: f32,
        /// A fibre's radius at its root, in scene units.
        thickness: f32,
        /// How far the fibres lean from standing straight up, 0 to 1.
        lean: f32,
        /// How much each fibre bends along its length, 0 to 1.
        curl: f32,
        seed: u32,
    },
}

/// An occluded fraction, gathered into the corners by `contrast`.
pub fn occlusion_shape(occlusion: f32, contrast: f32) -> f32 {
    occlusion.clamp(0.0, 1.0).powf(contrast.max(0.05))
}

/// A new [`NodeKind::Occlusion`]'s radius: a centimetre in a scene in
/// metres, which is the scale of the crevices it is for.
pub const DEFAULT_OCCLUSION_RADIUS: f32 = 0.01;

/// What a [`NodeKind::Curvature`] masks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CurvatureMode {
    /// Edges and bumps: where things get worn.
    #[default]
    Convex,
    /// Creases and grooves: where dirt collects.
    Concave,
    /// Both, about a middle grey: 1 convex, 0 concave, 0.5 flat.
    Both,
}

impl CurvatureMode {
    pub const ALL: [CurvatureMode; 3] = [CurvatureMode::Convex, CurvatureMode::Concave, CurvatureMode::Both];

    pub const fn label(self) -> &'static str {
        match self {
            CurvatureMode::Convex => "Convex",
            CurvatureMode::Concave => "Concave",
            CurvatureMode::Both => "Both",
        }
    }

    /// The mask for a curvature `k` (inverse scene units) at `radius`.
    pub fn mask(self, k: f32, radius: f32, contrast: f32) -> f32 {
        let scaled = k * radius.max(0.0);
        let value = match self {
            CurvatureMode::Convex => scaled.clamp(0.0, 1.0),
            CurvatureMode::Concave => (-scaled).clamp(0.0, 1.0),
            CurvatureMode::Both => 0.5 + 0.5 * scaled.clamp(-1.0, 1.0),
        };
        if contrast == 1.0 {
            return value;
        }
        ((value - 0.5) * contrast.max(0.0) + 0.5).clamp(0.0, 1.0)
    }
}

/// A new [`NodeKind::Curvature`]'s radius: two millimetres in a scene in
/// metres — the edge break of a machined or moulded part.
pub const DEFAULT_CURVATURE_RADIUS: f32 = 0.002;

/// A new [`NodeKind::Bump`]'s depth, in scene units: half a millimetre in a
/// scene measured in metres. Deep enough to read at product scale — a grain,
/// a weave, a stipple — and shallow enough not to look moulded.
pub const DEFAULT_BUMP_DEPTH: f32 = 0.0005;

impl NodeKind {
    /// The node's inputs, in socket order.
    pub fn inputs(&self) -> Vec<(Socket, SocketType, &'static str)> {
        match self {
            NodeKind::Surface(_) => Param::ALL
                .iter()
                .map(|p| {
                    let ty = match p.ui() {
                        _ if *p == Param::Tangent => SocketType::Vector,
                        crate::material_type::ParamUi::Color => SocketType::Color,
                        crate::material_type::ParamUi::Vector => SocketType::Vector,
                        crate::material_type::ParamUi::Flag => SocketType::Bool,
                        // A bump slot takes a height field.
                        crate::material_type::ParamUi::Slot => SocketType::Float,
                        _ => SocketType::Float,
                    };
                    (Socket::Param(*p), ty, p.label())
                })
                .collect(),
            NodeKind::Constant(_) | NodeKind::Mapping2d { .. } => Vec::new(),
            // A pattern is evaluated in object space unless a Mapping node is
            // wired here; the compiler has always looked for one, and without
            // the socket the wire was refused before it got that far.
            NodeKind::ImageTexture { .. }
            | NodeKind::ImageAlpha { .. }
            | NodeKind::Procedural { .. } => {
                vec![(Socket::Input(0), SocketType::Vector, "Coordinates")]
            }
            NodeKind::Mix => vec![
                (Socket::Input(0), SocketType::Color, "A"),
                (Socket::Input(1), SocketType::Color, "B"),
                (Socket::Input(2), SocketType::Float, "Factor"),
            ],
            NodeKind::Multiply | NodeKind::Add => vec![
                (Socket::Input(0), SocketType::Color, "A"),
                (Socket::Input(1), SocketType::Color, "B"),
            ],
            NodeKind::Invert
            | NodeKind::Remap { .. }
            | NodeKind::Clamp { .. }
            | NodeKind::Channel { .. } => {
                vec![(Socket::Input(0), SocketType::Color, "Input")]
            }
            NodeKind::Gradient { .. } => {
                vec![(Socket::Input(0), SocketType::Float, "Position")]
            }
            NodeKind::Bump { .. } => vec![(Socket::Input(0), SocketType::Float, "Height")],
            NodeKind::Curvature { .. }
            | NodeKind::Occlusion { .. }
            | NodeKind::VertexColor
            | NodeKind::VolumeMap { .. }
            | NodeKind::Bubbles { .. }
            | NodeKind::Fuzz { .. } => {
                Vec::new()
            }
            NodeKind::Reroute { ty } => vec![(Socket::Input(0), *ty, "In")],
            NodeKind::Node(node) => node
                .inputs()
                .iter()
                .enumerate()
                .map(|(i, spec)| (Socket::Input(i as u8), spec.ty, spec.name))
                .collect(),
        }
    }

    /// Every output, by name and type: one for most nodes, several for a
    /// Blender node such as Separate XYZ or Voronoi.
    pub fn outputs(&self) -> Vec<(&'static str, SocketType)> {
        match self {
            NodeKind::Node(node) => node.outputs().iter().map(|o| (o.name, o.ty)).collect(),
            other => other
                .output_type()
                .map(|ty| {
                    let name = match ty {
                        SocketType::Float => "Value",
                        SocketType::Color => "Color",
                        SocketType::Vector => "Vector",
                        SocketType::Normal => "Normal",
                        SocketType::Bool => "Flag",
                    };
                    vec![(name, ty)]
                })
                .unwrap_or_default(),
        }
    }

    /// What output `index` carries.
    pub fn output_type_at(&self, index: u8) -> Option<SocketType> {
        match self {
            NodeKind::Node(node) => node.outputs().get(index as usize).map(|o| o.ty),
            other if index == 0 => other.output_type(),
            _ => None,
        }
    }

    /// Input `index`'s value when nothing is wired and no literal is set,
    /// where the node has its own default rather than its type's zero.
    pub fn input_default(&self, index: usize) -> Option<Value> {
        match self {
            NodeKind::Node(node) => node.inputs().get(index).map(|s| s.default),
            _ => None,
        }
    }

    /// What input `index` reads from the surface when nothing is wired to
    /// it, if it does: a texture's coordinates, a normal.
    pub fn implicit_input(&self, index: usize) -> Option<crate::nodes::PointInput> {
        match self {
            NodeKind::Node(node) => node.inputs().get(index).and_then(|s| s.implicit),
            _ => None,
        }
    }

    /// Whether the editor draws input `index`.
    pub fn shows_input(&self, index: usize) -> bool {
        match self {
            NodeKind::Node(node) => node.shows(index),
            _ => true,
        }
    }

    /// Output `index` from evaluated inputs.
    pub fn evaluate_output(&self, inputs: &[Value], index: u8) -> Value {
        match self {
            NodeKind::Node(node) => node.evaluate(inputs, index),
            other => other.evaluate(inputs),
        }
    }

    /// What the node's single output carries. `None` for the terminal, which
    /// produces a material rather than a value.
    pub fn output_type(&self) -> Option<SocketType> {
        match self {
            NodeKind::Surface(_) => None,
            // Changes the geometry, and gives the graph no value to wire.
            NodeKind::Bubbles { .. } | NodeKind::Fuzz { .. } => None,
            NodeKind::Constant(v) => Some(v.socket_type()),
            NodeKind::Mapping2d { .. } => Some(SocketType::Vector),
            NodeKind::Procedural { .. } => Some(SocketType::Float),
            NodeKind::Channel { .. } => Some(SocketType::Float),
            NodeKind::Gradient { .. } => Some(SocketType::Color),
            NodeKind::ImageTexture { .. } => Some(SocketType::Color),
            NodeKind::ImageAlpha { .. } => Some(SocketType::Float),
            NodeKind::Bump { .. } => Some(SocketType::Float),
            NodeKind::Curvature { .. } => Some(SocketType::Float),
            NodeKind::Occlusion { .. } => Some(SocketType::Float),
            NodeKind::VertexColor => Some(SocketType::Color),
            NodeKind::VolumeMap { .. } => Some(SocketType::Float),
            NodeKind::Reroute { ty } => Some(*ty),
            NodeKind::Node(node) => node.outputs().first().map(|o| o.ty),
            NodeKind::Mix
            | NodeKind::Multiply
            | NodeKind::Add
            | NodeKind::Invert
            | NodeKind::Remap { .. }
            | NodeKind::Clamp { .. } => Some(SocketType::Color),
        }
    }

    /// Whether this node's output depends on the shading point.
    ///
    /// A node that does not is foldable: §4.3's constant-folding pass collapses
    /// any subgraph made entirely of them into a literal, so a "texture" that
    /// turned out to be a flat colour costs nothing at render time.
    pub fn is_varying(&self) -> bool {
        if let NodeKind::Node(node) = self {
            return node.is_varying();
        }
        matches!(
            self,
            NodeKind::Mapping2d { .. }
                | NodeKind::Procedural { .. }
                | NodeKind::ImageTexture { .. }
                | NodeKind::ImageAlpha { .. }
                | NodeKind::Curvature { .. }
                | NodeKind::Occlusion { .. }
                | NodeKind::VertexColor
                | NodeKind::VolumeMap { .. }
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            NodeKind::Surface(_) => "Surface",
            NodeKind::Constant(_) => "Constant",
            NodeKind::Mapping2d { .. } => "Mapping 2D",
            NodeKind::Gradient { .. } => "Gradient",
            NodeKind::Procedural { procedural, .. } => procedural.label(),
            NodeKind::ImageTexture { .. } => "Image Texture",
            NodeKind::ImageAlpha { .. } => "Image Alpha",
            NodeKind::Mix => "Mix",
            NodeKind::Multiply => "Multiply",
            NodeKind::Add => "Add",
            NodeKind::Invert => "Invert",
            NodeKind::Channel { .. } => "Channel",
            NodeKind::Remap { .. } => "Remap",
            NodeKind::Clamp { .. } => "Clamp",
            NodeKind::Bump { .. } => "Bump",
            NodeKind::Curvature { .. } => "Curvature",
            NodeKind::Occlusion { .. } => "Occlusion",
            NodeKind::VertexColor => "Vertex Color",
            NodeKind::VolumeMap { .. } => "Volume Map",
            NodeKind::Bubbles { .. } => "Bubbles",
            NodeKind::Fuzz { .. } => "Fuzz",
            NodeKind::Reroute { .. } => "Reroute",
            NodeKind::Node(node) => node.label(),
        }
    }

    /// Evaluate from already-evaluated inputs. Used both by the constant folder
    /// at compile time and by the register machine at shading time, so the two
    /// cannot disagree about what a node means.
    pub fn evaluate(&self, inputs: &[Value]) -> Value {
        let get = |i: usize| inputs.get(i).copied().unwrap_or(Value::Float(0.0));
        match self {
            NodeKind::Surface(_) => Value::Float(0.0),
            NodeKind::Constant(v) => *v,
            NodeKind::Mapping2d { .. } => Value::Vector(Vec3::ZERO),
            // Varying by construction, so the constant folder never asks for
            // its value; a mid-grey is the right answer if it ever does.
            NodeKind::Procedural { .. } => Value::Float(0.5),
            NodeKind::ImageTexture { fallback, .. } => *fallback,
            NodeKind::ImageAlpha { fallback, .. } => *fallback,
            NodeKind::Mix => {
                let (a, b, t) = (get(0).as_color(), get(1).as_color(), get(2).as_float());
                Value::Color(a * (1.0 - t) + b * t)
            }
            NodeKind::Multiply => Value::Color(get(0).as_color() * get(1).as_color()),
            NodeKind::Add => Value::Color(get(0).as_color() + get(1).as_color()),
            NodeKind::Gradient { gradient } => {
                // Read from the first channel, so a colour arriving here by
                // some other route is treated as the scalar it is standing in
                // for rather than refused.
                Value::Color(gradient.evaluate(get(0).as_float()))
            }
            NodeKind::Channel { channel } => Value::Float(channel.of(get(0).as_color())),
            NodeKind::Invert => {
                let c = get(0).as_color();
                Value::Color(Spectrum {
                    r: 1.0 - c.r,
                    g: 1.0 - c.g,
                    b: 1.0 - c.b,
                })
            }
            NodeKind::Remap { from, to } => {
                let c = get(0).as_color();
                let span = (from.y - from.x).abs().max(1e-8) * (from.y - from.x).signum();
                let map = |v: f32| to.x + (v - from.x) / span * (to.y - to.x);
                Value::Color(Spectrum {
                    r: map(c.r),
                    g: map(c.g),
                    b: map(c.b),
                })
            }
            NodeKind::Clamp { low, high } => {
                let c = get(0).as_color();
                Value::Color(Spectrum {
                    r: c.r.clamp(*low, *high),
                    g: c.g.clamp(*low, *high),
                    b: c.b.clamp(*low, *high),
                })
            }
            NodeKind::Bump { depth } => Value::Float(get(0).as_float() * depth),
            // Varying, so never folded; flat is the honest guess.
            NodeKind::Curvature { mode, radius, contrast } => Value::Float(mode.mask(0.0, *radius, *contrast)),
            NodeKind::Reroute { ty } => get(0).convert(*ty).unwrap_or(get(0)),
            // Varying, so never folded; open is the honest guess.
            NodeKind::Occlusion { contrast, .. } => Value::Float(occlusion_shape(0.0, *contrast)),
            NodeKind::VertexColor => Value::Color(Spectrum::ONE),
            NodeKind::VolumeMap { .. } => Value::Float(1.0),
            NodeKind::Bubbles { .. } | NodeKind::Fuzz { .. } => Value::Float(0.0),
            NodeKind::Node(node) => node.evaluate(inputs, 0),
        }
    }
}

/// A titled box around a group of nodes, in graph space.
#[derive(Clone, Debug, PartialEq)]
pub struct Frame {
    pub title: String,
    pub min: Vec2,
    pub max: Vec2,
}

impl Frame {
    /// Whether a node at `position` sits inside.
    pub fn contains(&self, position: Vec2) -> bool {
        position.x >= self.min.x && position.y >= self.min.y && position.x <= self.max.x && position.y <= self.max.y
    }
}

/// A node in the graph.
#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub kind: NodeKind,
    /// Literals for unconnected inputs (§4.1).
    pub literals: BTreeMap<Socket, Value>,
    /// Editor layout only; never affects compilation.
    pub position: Vec2,
    pub label: Option<String>,
    /// Switched off, as Blender's mute: each output passes on the first
    /// input wired in that can carry it, or nothing
    /// ([`MaterialGraph::without_muted`]).
    pub muted: bool,
    /// Drawn as its header alone. Editor layout only.
    pub collapsed: bool,
}

impl Node {
    pub fn new(kind: NodeKind) -> Node {
        Node {
            kind,
            literals: BTreeMap::new(),
            position: Vec2::ZERO,
            label: None,
            muted: false,
            collapsed: false,
        }
    }

    pub fn with_literal(mut self, socket: Socket, value: Value) -> Node {
        self.literals.insert(socket, value);
        self
    }
}

/// A connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Link {
    pub from: NodeId,
    /// Which of `from`'s outputs; 0 for a node with one.
    pub from_socket: u8,
    pub to: NodeId,
    pub to_socket: Socket,
}

/// Why a graph would not compile.
#[derive(Clone, Debug, PartialEq)]
pub enum GraphError {
    /// The graph has no terminal node.
    NoOutput,
    /// A link names a node that is not in the graph.
    DanglingLink(Link),
    /// A cycle, reported with the nodes involved so the editor can highlight
    /// them rather than saying "invalid graph".
    Cycle(Vec<NodeId>),
    /// A link between incompatible sockets (§4.1).
    TypeMismatch {
        link: Link,
        from: SocketType,
        to: SocketType,
    },
    /// A link into a socket the destination node does not have.
    NoSuchSocket { node: NodeId, socket: Socket },
    /// Two links into one input.
    DuplicateLink { node: NodeId, socket: Socket },
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::NoOutput => write!(f, "the graph has no surface node"),
            GraphError::DanglingLink(l) => {
                write!(f, "a link refers to node {} which is not present", l.from.0)
            }
            GraphError::Cycle(nodes) => {
                write!(f, "the graph contains a cycle through {} nodes", nodes.len())
            }
            GraphError::TypeMismatch { from, to, .. } => write!(
                f,
                "a {} output cannot drive a {} input",
                from.label(),
                to.label()
            ),
            GraphError::NoSuchSocket { node, .. } => {
                write!(f, "node {} has no such input", node.0)
            }
            GraphError::DuplicateLink { node, .. } => {
                write!(f, "node {} has two links into one input", node.0)
            }
        }
    }
}

impl std::error::Error for GraphError {}

/// A material as a graph (§4.1).
#[derive(Clone, Debug, PartialEq)]
pub struct MaterialGraph {
    pub decals: Vec<crate::decal::Decal>,
    /// Titled boxes around nodes (`docs/04` §7's groups). Layout only, like
    /// a node's position: never compiled, never part of the content hash.
    pub frames: Vec<Frame>,
    nodes: BTreeMap<NodeId, Node>,
    links: Vec<Link>,
    output: NodeId,
    next_id: u32,
}

impl MaterialGraph {
    /// The graph a material with no textures is: one surface node, holding the
    /// parameter block as its literals (§4.2).
    pub fn from_material(material: Material) -> MaterialGraph {
        let output = NodeId(1);
        let mut nodes = BTreeMap::new();
        nodes.insert(output, Node::new(NodeKind::Surface(Box::new(material))));
        MaterialGraph {
            decals: Vec::new(),
            frames: Vec::new(),
            nodes,
            links: Vec::new(),
            output,
            next_id: 2,
        }
    }

    pub fn output(&self) -> NodeId {
        self.output
    }

    pub fn nodes(&self) -> impl Iterator<Item = (NodeId, &Node)> {
        self.nodes.iter().map(|(k, n)| (*k, n))
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(&id)
    }

    pub fn links(&self) -> &[Link] {
        &self.links
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// The parameter block the surface node holds — what the simple panel edits.
    pub fn material(&self) -> &Material {
        match &self.nodes[&self.output].kind {
            NodeKind::Surface(m) => m,
            _ => unreachable!("the output node is always a surface"),
        }
    }

    pub fn material_mut(&mut self) -> &mut Material {
        match &mut self
            .nodes
            .get_mut(&self.output)
            .expect("the output node is always present")
            .kind
        {
            NodeKind::Surface(m) => m,
            _ => unreachable!("the output node is always a surface"),
        }
    }

    pub fn add_node(&mut self, node: Node) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        self.nodes.insert(id, node);
        id
    }

    /// Remove a node and every link touching it.
    pub fn remove_node(&mut self, id: NodeId) -> Option<Node> {
        if id == self.output {
            return None;
        }
        self.links.retain(|l| l.from != id && l.to != id);
        self.nodes.remove(&id)
    }

    /// Connect, replacing whatever drove that input before.
    ///
    /// An input takes one link. Silently replacing is right here: in an editor
    /// the user drags onto an occupied socket meaning "instead of", and
    /// refusing would make them delete first for no reason.
    pub fn connect(&mut self, from: NodeId, to: NodeId, to_socket: Socket) {
        self.connect_output(from, 0, to, to_socket);
    }

    /// Connect output `output` of `from`, replacing whatever drove that
    /// input before.
    pub fn connect_output(&mut self, from: NodeId, output: u8, to: NodeId, to_socket: Socket) {
        self.links
            .retain(|l| !(l.to == to && l.to_socket == to_socket));
        self.links.push(Link {
            from,
            from_socket: output,
            to,
            to_socket,
        });
        self.links.sort();
    }

    pub fn disconnect(&mut self, to: NodeId, to_socket: Socket) {
        self.links
            .retain(|l| !(l.to == to && l.to_socket == to_socket));
    }

    /// Whether a parameter is driven by the graph rather than by a literal.
    /// The simple panel shows a "connected" indicator for these (§4.2).
    pub fn is_connected(&self, param: Param) -> bool {
        self.links
            .iter()
            .any(|l| l.to == self.output && l.to_socket == Socket::Param(param))
    }

    /// The node driving a parameter, for the panel's jump-to-node affordance.
    pub fn driver_of(&self, param: Param) -> Option<NodeId> {
        self.links
            .iter()
            .find(|l| l.to == self.output && l.to_socket == Socket::Param(param))
            .map(|l| l.from)
    }

    /// A hash over the graph's *shape*, excluding literal values.
    ///
    /// Two graphs agreeing here compile to the same instruction list, so a
    /// change that leaves this alone can patch values in place. §4.3: "parameter
    /// changes that do not alter topology must patch a uniform block, not
    /// recompile."
    pub fn topology_hash(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}", self.decals).hash(&mut hasher);
        self.output.hash(&mut hasher);
        for (id, node) in &self.nodes {
            id.hash(&mut hasher);
            // The kind's *discriminant* and its structural configuration, not
            // its literals: a Surface node's parameter block is values, but a
            // Mapping node's projection mode changes what is emitted.
            std::mem::discriminant(&node.kind).hash(&mut hasher);
            match &node.kind {
                NodeKind::Mapping2d { mapping } => {
                    std::mem::discriminant(&mapping.projection).hash(&mut hasher);
                    mapping.uv_set.hash(&mut hasher);
                }
                NodeKind::ImageTexture { texture, .. }
                | NodeKind::ImageAlpha { texture, .. }
                | NodeKind::VolumeMap { volume: texture } => texture.hash(&mut hasher),
                // Its settings change what it computes and what it outputs.
                NodeKind::Node(node) => format!("{node:?}").hash(&mut hasher),
                NodeKind::Procedural { procedural, seed } => {
                    // The discriminant is already hashed above; what
                    // distinguishes two of the same kind is the seed, and the
                    // parameters go into the *content* hash rather than this
                    // one because changing a frequency does not change what
                    // the program looks like.
                    std::mem::discriminant(&**procedural).hash(&mut hasher);
                    seed.hash(&mut hasher);
                }
                _ => {}
            }
            for socket in node.literals.keys() {
                socket.hash(&mut hasher);
            }
            node.muted.hash(&mut hasher);
        }
        for link in &self.links {
            link.hash(&mut hasher);
        }
        hasher.finish()
    }

    /// The same graph with every reroute taken out and its wires joined:
    /// each link out of a reroute is re-made from whatever feeds it, through
    /// any chain of reroutes. A reroute fed by nothing leaves its targets on
    /// their literals.
    pub fn without_reroutes(&self) -> MaterialGraph {
        let is_reroute = |id: NodeId| matches!(self.nodes.get(&id).map(|n| &n.kind), Some(NodeKind::Reroute { .. }));
        // The node and output a wire really comes from, through reroutes.
        let source = |mut id: NodeId, mut socket: u8| {
            let mut hops = 0;
            while is_reroute(id) {
                let into = self.links.iter().find(|l| l.to == id)?;
                (id, socket) = (into.from, into.from_socket);
                hops += 1;
                if hops > self.nodes.len() {
                    return None;
                }
            }
            Some((id, socket))
        };
        let mut out = self.clone();
        out.links = self
            .links
            .iter()
            .filter(|link| !is_reroute(link.to))
            .filter_map(|link| {
                let (from, from_socket) = source(link.from, link.from_socket)?;
                Some(Link { from, from_socket, ..*link })
            })
            .collect();
        out.nodes.retain(|id, _| !is_reroute(*id));
        out
    }

    /// The same graph with every muted node taken out, as Blender bypasses
    /// one: each wire out of it is re-made from the first wire into it whose
    /// value can carry that output's type - the same type first, then one
    /// that converts - and dropped where none can, leaving its target on its
    /// literal. The surface is never muted; it is the material.
    pub fn without_muted(&self) -> MaterialGraph {
        let mut out = self.clone();
        loop {
            let Some(muted) = out.nodes.iter().find(|(id, n)| n.muted && **id != out.output).map(|(id, _)| *id) else {
                break;
            };
            let into: Vec<Link> = out.links.iter().filter(|l| l.to == muted).copied().collect();
            let source_type = |out: &MaterialGraph, link: &Link| out.nodes.get(&link.from).and_then(|n| n.kind.output_type_at(link.from_socket));
            let output_type = |index: u8| out.nodes.get(&muted).and_then(|n| n.kind.output_type_at(index));
            let mut links = Vec::with_capacity(out.links.len());
            for link in &out.links {
                if link.to == muted {
                    continue;
                }
                if link.from != muted {
                    links.push(*link);
                    continue;
                }
                let Some(wanted) = output_type(link.from_socket) else { continue };
                let pass = into
                    .iter()
                    .find(|l| source_type(&out, l) == Some(wanted))
                    .or_else(|| into.iter().find(|l| source_type(&out, l).is_some_and(|t| t.converts_to(wanted))));
                if let Some(pass) = pass {
                    links.push(Link { from: pass.from, from_socket: pass.from_socket, ..*link });
                }
            }
            out.links = links;
            out.nodes.remove(&muted);
        }
        out
    }

    /// The graph with one node's output shown on the surface and nothing
    /// else: the editor's solo. The surface glows with the node's value and
    /// reflects nothing, so what the node makes is seen exactly as it is,
    /// unlit. `None` for a node whose output is not a value or a colour.
    pub fn soloed(&self, node: NodeId) -> Option<MaterialGraph> {
        self.soloed_at(node, 0)
    }

    /// As [`MaterialGraph::soloed`], for the node's output `output`, as
    /// Blender's viewer shows any output: a vector seen as a colour.
    pub fn soloed_at(&self, node: NodeId, output_socket: u8) -> Option<MaterialGraph> {
        let kind = &self.nodes.get(&node)?.kind;
        let showable = match kind.output_type_at(output_socket)? {
            SocketType::Float | SocketType::Color => true,
            // A Blender node's vector, as Blender's viewer shows one; a
            // projection's coordinates are not a picture of anything.
            SocketType::Vector | SocketType::Normal => matches!(kind, NodeKind::Node(_)),
            SocketType::Bool => false,
        };
        if !showable {
            return None;
        }
        let mut out = self.clone();
        out.decals.clear();
        let output = out.output;
        out.links.retain(|l| l.to != output);
        if let Some(Node {
            kind: NodeKind::Surface(material),
            ..
        }) = out.nodes.get_mut(&output)
        {
            let glow = Material {
                base_weight: 0.0,
                specular_weight: 0.0,
                coat_weight: 0.0,
                transmission_weight: 0.0,
                subsurface_weight: 0.0,
                fuzz_weight: 0.0,
                thin_film_weight: 0.0,
                flake_weight: 0.0,
                geometry_opacity: 1.0,
                emission_color: Spectrum::ONE,
                emission_luminance: 1.0,
                ..(**material).clone()
            };
            **material = glow;
        }
        out.connect_output(node, output_socket, output, Socket::Param(Param::EmissionColor));
        Some(out)
    }

    /// A hash over shape *and* values: whether the rendered result would differ.
    pub fn content_hash(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}", self.decals).hash(&mut hasher);
        self.topology_hash().hash(&mut hasher);
        for (_, node) in &self.nodes {
            if let NodeKind::Surface(material) = &node.kind {
                for param in Param::ALL {
                    if let Some(value) = param.read(material) {
                        format!("{value:?}").hash(&mut hasher);
                    }
                }
            } else {
                // Everything the kind itself carries, via its Debug form.
                //
                // Enumerating the value-bearing fields by hand is what went
                // wrong before: `topology_hash` picks out a mapping's
                // projection and uv_set because those change the emitted
                // instructions, and its scale, offset and rotation were left
                // out of *both* hashes — so `snapshot::compile_for` returned
                // the cached program and a texture-scale edit never reached
                // the render. Procedural parameters had the same hole. Debug
                // covers every field, including the ones added next year.
                format!("{:?}", node.kind).hash(&mut hasher);
            }
            for (socket, value) in &node.literals {
                socket.hash(&mut hasher);
                format!("{value:?}").hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    /// Renumber every texture this graph refers to.
    ///
    /// An importer builds its materials before it knows what ids the document
    /// will give the images — the ids are reserved against a document the
    /// importer is deliberately not allowed to touch — so it numbers them from
    /// zero and the caller renumbers afterwards.
    pub fn remap_textures(&mut self, map: &dyn Fn(TextureId) -> TextureId) {
        for (_, node) in self.nodes.iter_mut() {
            if let NodeKind::ImageTexture { texture, .. }
            | NodeKind::ImageAlpha { texture, .. }
            | NodeKind::VolumeMap { volume: texture } = &mut node.kind
            {
                *texture = map(*texture);
            }
        }
    }

    /// Check the graph without compiling it, for the editor's live feedback.
    pub fn validate(&self) -> Result<(), GraphError> {
        if !self.nodes.contains_key(&self.output) {
            return Err(GraphError::NoOutput);
        }

        let mut seen: BTreeSet<(NodeId, Socket)> = BTreeSet::new();
        for link in &self.links {
            let (Some(from), Some(to)) = (self.nodes.get(&link.from), self.nodes.get(&link.to))
            else {
                return Err(GraphError::DanglingLink(*link));
            };
            if !seen.insert((link.to, link.to_socket)) {
                return Err(GraphError::DuplicateLink {
                    node: link.to,
                    socket: link.to_socket,
                });
            }

            let Some((_, to_type, _)) = to
                .kind
                .inputs()
                .into_iter()
                .find(|(s, _, _)| *s == link.to_socket)
            else {
                return Err(GraphError::NoSuchSocket {
                    node: link.to,
                    socket: link.to_socket,
                });
            };
            let from_type = from.kind.output_type_at(link.from_socket).ok_or(GraphError::TypeMismatch {
                link: *link,
                from: SocketType::Float,
                to: to_type,
            })?;
            if !from_type.converts_to(to_type) {
                return Err(GraphError::TypeMismatch {
                    link: *link,
                    from: from_type,
                    to: to_type,
                });
            }
        }

        self.topological_order().map(|_| ())
    }

    /// Nodes in dependency order, producers before consumers.
    ///
    /// Kahn's algorithm, because what remains when it stalls *is* the cycle —
    /// so the error can name the nodes involved and the editor can highlight
    /// them, rather than reporting "invalid graph" and leaving the user to
    /// find it.
    pub fn topological_order(&self) -> Result<Vec<NodeId>, GraphError> {
        let mut incoming: BTreeMap<NodeId, usize> =
            self.nodes.keys().map(|id| (*id, 0)).collect();
        for link in &self.links {
            if let Some(count) = incoming.get_mut(&link.to) {
                *count += 1;
            }
        }

        let mut ready: Vec<NodeId> = incoming
            .iter()
            .filter(|(_, n)| **n == 0)
            .map(|(id, _)| *id)
            .collect();
        let mut order = Vec::with_capacity(self.nodes.len());

        while let Some(id) = ready.pop() {
            order.push(id);
            for link in self.links.iter().filter(|l| l.from == id) {
                if let Some(count) = incoming.get_mut(&link.to) {
                    *count -= 1;
                    if *count == 0 {
                        ready.push(link.to);
                    }
                }
            }
        }

        if order.len() != self.nodes.len() {
            let stuck: Vec<NodeId> = incoming
                .into_iter()
                .filter(|(_, n)| *n > 0)
                .map(|(id, _)| id)
                .collect();
            return Err(GraphError::Cycle(stuck));
        }
        Ok(order)
    }
}

// ---------------------------------------------------------------------------
// Compilation (§4.3)
// ---------------------------------------------------------------------------

/// A slot in the register machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Register(pub u16);

/// One step of the compiled program.
#[derive(Clone, Debug, PartialEq)]
pub enum Instruction {
    /// A literal folded in at compile time.
    Load { dst: Register, value: Value },
    /// Texture coordinates from the shading point.
    Project {
        dst: Register,
        mapping: Box<tequila_texture::projection::Mapping>,
    },
    /// A filtered texture lookup.
    ///
    /// The mapping travels with the instruction rather than the coordinates
    /// travelling in a register. Tri-planar blends *three* lookups weighted by
    /// the normal, and a single `(u, v)` pair cannot express that — passing
    /// coordinates would silently reduce every tri-planar material to one of
    /// its three planes, which on a curved part is a texture that slides as the
    /// surface turns.
    Sample {
        dst: Register,
        texture: TextureId,
        mapping: Box<tequila_texture::projection::Mapping>,
        fallback: Value,
    },
    /// The same lookup, returning the texture's alpha as a number.
    SampleAlpha {
        dst: Register,
        texture: TextureId,
        mapping: Box<tequila_texture::projection::Mapping>,
        fallback: Value,
    },
    /// A procedural evaluated from the shading point.
    ///
    /// Like [`Instruction::Sample`], the mapping travels with the instruction
    /// rather than the coordinates travelling in a register — a procedural is
    /// a function of a 3D point, and a `(u, v)` pair has thrown that away.
    Evaluate {
        dst: Register,
        procedural: Box<tequila_texture::procedural::Procedural>,
        seed: u32,
        /// `None` is object space, which is the default §6 states.
        mapping: Option<Box<tequila_texture::projection::Mapping>>,
    },
    /// §6.2's Curvature, read from the shading point.
    Curvature {
        dst: Register,
        mode: CurvatureMode,
        radius: f32,
        contrast: f32,
    },
    /// §6.2's Occlusion, read from the shading point, where the snapshot
    /// left the value it measured at the program's radius.
    Occlusion { dst: Register, contrast: f32 },
    /// §6.2's Vertex Color, read from the shading point.
    VertexColor { dst: Register },
    /// A Volume Map's density at the surface, which is 1; see
    /// [`NodeKind::VolumeMap`].
    VolumeMap { dst: Register, volume: TextureId },
    /// Any pure node, applied to registers.
    Apply {
        dst: Register,
        kind: Box<NodeKind>,
        args: Vec<Register>,
        /// Which of the node's outputs.
        output: u8,
    },
    /// Something the shading point knows (`crate::nodes::PointInput`).
    Input { dst: Register, input: crate::nodes::PointInput },
}

impl Instruction {
    pub fn destination(&self) -> Register {
        match self {
            Instruction::Load { dst, .. }
            | Instruction::Project { dst, .. }
            | Instruction::Sample { dst, .. }
            | Instruction::SampleAlpha { dst, .. }
            | Instruction::Evaluate { dst, .. }
            | Instruction::Curvature { dst, .. }
            | Instruction::Occlusion { dst, .. }
            | Instruction::VertexColor { dst }
            | Instruction::VolumeMap { dst, .. }
            | Instruction::Apply { dst, .. }
            | Instruction::Input { dst, .. } => *dst,
        }
    }
}

/// What the renderer must arrange for a material (§4.3).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Requirements {
    /// UV sets the graph reads. Empty means the material needs no UVs at all,
    /// which for CAD geometry is the common case.
    pub uv_sets: BTreeSet<usize>,
    /// A tangent frame is needed for anisotropy and for tangent-space normals.
    pub needs_tangents: bool,
    pub textures: BTreeSet<TextureId>,
    /// Volume Maps the graph reads, loaded as density grids rather than
    /// images.
    pub volumes: BTreeSet<TextureId>,
    /// The surface encloses a participating medium.
    pub needs_medium: bool,
    /// The output changes with animation time, so it cannot be cached across
    /// frames (§4.5).
    pub time_varying: bool,
    /// The Occlusion node's radius, in scene units, when the graph has one:
    /// the parts wearing it need their occlusion measured at that radius.
    /// Held as bits so the requirements stay comparable; read it through
    /// [`Requirements::occlusion_radius`].
    pub occlusion_radius_bits: Option<u32>,
    /// What a Bubbles node asks for, when the graph has one.
    pub bubbles: Option<BubbleRequest>,
    /// What a Fuzz node asks for, when the graph has one.
    pub fuzz: Option<FuzzRequest>,
}

/// A Fuzz node's settings, comparable, for the snapshot to act on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FuzzRequest {
    pub length_bits: u32,
    pub density_bits: u32,
    pub thickness_bits: u32,
    pub lean_bits: u32,
    pub curl_bits: u32,
    pub seed: u32,
}

impl FuzzRequest {
    pub fn length(&self) -> f32 {
        f32::from_bits(self.length_bits)
    }
    pub fn density(&self) -> f32 {
        f32::from_bits(self.density_bits)
    }
    pub fn thickness(&self) -> f32 {
        f32::from_bits(self.thickness_bits)
    }
    pub fn lean(&self) -> f32 {
        f32::from_bits(self.lean_bits)
    }
    pub fn curl(&self) -> f32 {
        f32::from_bits(self.curl_bits)
    }
}

/// A Bubbles node's settings, comparable, for the snapshot to act on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BubbleRequest {
    pub count: u32,
    pub radius_bits: u32,
    pub variation_bits: u32,
    pub seed: u32,
}

impl BubbleRequest {
    pub fn radius(&self) -> f32 {
        f32::from_bits(self.radius_bits)
    }
    pub fn variation(&self) -> f32 {
        f32::from_bits(self.variation_bits)
    }
}

impl Requirements {
    pub fn occlusion_radius(&self) -> Option<f32> {
        self.occlusion_radius_bits.map(f32::from_bits)
    }
}

/// A graph reduced to a flat program.
#[derive(Clone, Debug, PartialEq)]
pub struct CompiledGraph {
    pub decals: Vec<crate::decal::Decal>,
    pub instructions: Vec<Instruction>,
    pub registers: usize,
    /// The parameter block after constant folding: every parameter whose value
    /// the graph could settle at compile time.
    pub constants: Material,
    /// Parameters the program computes, and where to find each result.
    pub driven: Vec<(Param, Register)>,
    pub requirements: Requirements,
    /// The topology this was compiled from. A graph whose topology hash still
    /// matches can reuse these instructions with patched literals.
    pub topology: u64,
}

impl CompiledGraph {
    /// The same program, reduced to the instructions that compute `wanted`.
    ///
    /// For a consumer that reads only some of the parameter block. The Solid
    /// viewport reads the base colour and a few scalars and never a normal
    /// map; running a textured material's full program for it sampled every
    /// map on every pixel, which was half the cost of a Solid frame.
    ///
    /// Instructions depend on one another only through [`Instruction::Apply`]
    /// arguments, so this is a single backward liveness pass. A register
    /// written twice is handled: a write kills liveness above it. Parameters
    /// not wanted keep their folded constant; decals are kept, because a label
    /// is part of what a surface looks like.
    pub fn restricted_to(&self, wanted: &[crate::material_type::Param]) -> CompiledGraph {
        let driven: Vec<(crate::material_type::Param, Register)> =
            self.driven.iter().filter(|(param, _)| wanted.contains(param)).cloned().collect();
        let mut live: BTreeSet<u16> = driven.iter().map(|(_, register)| register.0).collect();
        let mut keep = vec![false; self.instructions.len()];
        for (index, instruction) in self.instructions.iter().enumerate().rev() {
            let dst = instruction.destination().0;
            if live.remove(&dst) {
                keep[index] = true;
                if let Instruction::Apply { args, .. } = instruction {
                    live.extend(args.iter().map(|r| r.0));
                }
            }
        }
        CompiledGraph {
            decals: self.decals.clone(),
            instructions: self
                .instructions
                .iter()
                .zip(&keep)
                .filter(|(_, keep)| **keep)
                .map(|(instruction, _)| instruction.clone())
                .collect(),
            registers: self.registers,
            constants: self.constants.clone(),
            driven,
            requirements: self.requirements.clone(),
            topology: self.topology,
        }
    }

    /// Whether this program can be reused for `graph`, patching values rather
    /// than recompiling (§4.3 point 4).
    pub fn can_reuse_for(&self, graph: &MaterialGraph) -> bool {
        self.topology == graph.topology_hash()
    }
}

impl MaterialGraph {
    /// Compile to the register machine (§4.3).
    pub fn compile(&self) -> Result<CompiledGraph, GraphError> {
        self.validate()?;
        // A muted node is not there, its wires passed through it.
        if self.nodes.iter().any(|(id, n)| n.muted && *id != self.output) {
            return self.without_muted().compile();
        }
        // Reroutes are wiring, not work: compile the graph they describe,
        // with each wire joined through them.
        if self.nodes.values().any(|n| matches!(n.kind, NodeKind::Reroute { .. })) {
            return self.without_reroutes().compile();
        }
        let order = self.topological_order()?;

        // Which nodes depend on the shading point, and so cannot be folded.
        let mut varying: BTreeMap<NodeId, bool> = BTreeMap::new();
        for id in &order {
            let node = &self.nodes[id];
            let inherits = self
                .links
                .iter()
                .filter(|l| l.to == *id)
                .any(|l| *varying.get(&l.from).unwrap_or(&false));
            let reads_point = node.kind.inputs().iter().enumerate().any(|(i, (socket, _, _))| {
                node.kind.implicit_input(i).is_some() && !self.links.iter().any(|l| l.to == *id && l.to_socket == *socket)
            });
            varying.insert(*id, node.kind.is_varying() || inherits || reads_point);
        }

        let mut instructions = Vec::new();
        // By node and output.
        let mut registers: HashMap<(NodeId, u8), Register> = HashMap::new();
        let mut folded: HashMap<(NodeId, u8), Value> = HashMap::new();
        // One register a program for each thing the point is asked.
        let mut points: HashMap<crate::nodes::PointInput, Register> = HashMap::new();
        let mut next_register = 0u16;
        let mut requirements = Requirements::default();
        for decal in &self.decals {
            requirements.textures.insert(decal.texture);
            if decal.mapping.projection == tequila_texture::projection::Projection::Uv {
                requirements.uv_sets.insert(0);
            }
        }

        for id in &order {
            let node = &self.nodes[id];
            if *id == self.output {
                continue;
            }
            // A geometry node: a request for the snapshot, not an instruction.
            if let NodeKind::Bubbles { count, radius, variation, seed } = &node.kind {
                requirements.bubbles = Some(BubbleRequest {
                    count: *count,
                    radius_bits: radius.to_bits(),
                    variation_bits: variation.to_bits(),
                    seed: *seed,
                });
                continue;
            }
            if let NodeKind::Fuzz { length, density, thickness, lean, curl, seed } = &node.kind {
                requirements.fuzz = Some(FuzzRequest {
                    length_bits: length.to_bits(),
                    density_bits: density.to_bits(),
                    thickness_bits: thickness.to_bits(),
                    lean_bits: lean.to_bits(),
                    curl_bits: curl.to_bits(),
                    seed: *seed,
                });
                continue;
            }

            // Gather this node's inputs in socket order, taking a link where
            // there is one and the node's own literal where there is not.
            let sockets = node.kind.inputs();
            let mut arg_values = Vec::with_capacity(sockets.len());
            let mut arg_registers = Vec::with_capacity(sockets.len());
            let mut all_folded = true;
            for (index, (socket, ty, _)) in sockets.iter().enumerate() {
                if let Some(link) = self
                    .links
                    .iter()
                    .find(|l| l.to == *id && l.to_socket == *socket)
                {
                    let from = (link.from, link.from_socket);
                    match folded.get(&from) {
                        Some(value) => {
                            arg_values.push(value.convert(*ty).unwrap_or(*value));
                            arg_registers.push(registers.get(&from).copied());
                        }
                        None => {
                            all_folded = false;
                            arg_values.push(Value::Float(0.0));
                            arg_registers.push(registers.get(&from).copied());
                        }
                    }
                } else if let Some(point) = node.kind.implicit_input(index) {
                    // Read from the surface: a texture's coordinates, a
                    // normal.
                    all_folded = false;
                    let register = *points.entry(point).or_insert_with(|| {
                        let r = Register(next_register);
                        next_register += 1;
                        if point == crate::nodes::PointInput::Uv {
                            requirements.uv_sets.insert(0);
                        }
                        instructions.push(Instruction::Input { dst: r, input: point });
                        r
                    });
                    arg_values.push(Value::Float(0.0));
                    arg_registers.push(Some(register));
                } else {
                    let literal = node
                        .literals
                        .get(socket)
                        .copied()
                        .or_else(|| node.kind.input_default(index))
                        .unwrap_or(default_for(*ty));
                    arg_values.push(literal);
                    arg_registers.push(None);
                }
            }

            let is_varying = varying[id];

            // §4.3 step 2: a subgraph with no varying inputs becomes a value.
            if !is_varying && all_folded {
                for output in 0..node.kind.outputs().len().max(1) as u8 {
                    folded.insert((*id, output), node.kind.evaluate_output(&arg_values, output));
                }
                continue;
            }

            // The outputs something reads; the first if nothing does.
            let mut consumed: Vec<u8> = self.links.iter().filter(|l| l.from == *id).map(|l| l.from_socket).collect();
            consumed.sort();
            consumed.dedup();
            if consumed.is_empty() {
                consumed.push(0);
            }

            // A Blender node: a read of the surface, or a pure function of
            // its inputs with one instruction per output read.
            if let NodeKind::Node(shader) = &node.kind {
                if shader.point_output(0).is_some() {
                    for output in consumed {
                        let Some(point) = shader.point_output(output) else { continue };
                        let register = *points.entry(point).or_insert_with(|| {
                            let r = Register(next_register);
                            next_register += 1;
                            if point == crate::nodes::PointInput::Uv {
                                requirements.uv_sets.insert(0);
                            }
                            instructions.push(Instruction::Input { dst: r, input: point });
                            r
                        });
                        registers.insert((*id, output), register);
                    }
                    continue;
                }
                let mut args = Vec::with_capacity(arg_registers.len());
                for (slot, value) in arg_registers.iter().zip(arg_values.iter()) {
                    match slot {
                        Some(r) => args.push(*r),
                        None => {
                            let r = Register(next_register);
                            next_register += 1;
                            instructions.push(Instruction::Load { dst: r, value: *value });
                            args.push(r);
                        }
                    }
                }
                for output in consumed {
                    let dst = Register(next_register);
                    next_register += 1;
                    instructions.push(Instruction::Apply { dst, kind: Box::new(node.kind.clone()), args: args.clone(), output });
                    registers.insert((*id, output), dst);
                }
                continue;
            }

            // Otherwise it needs a register and an instruction.
            let dst = Register(next_register);
            next_register += 1;
            registers.insert((*id, 0), dst);

            match &node.kind {
                NodeKind::Mapping2d { mapping } => {
                    if mapping.projection == tequila_texture::projection::Projection::Uv {
                        requirements.uv_sets.insert(mapping.uv_set);
                    }
                    // Textures fold the mapping into their own instruction, so
                    // one that feeds nothing else emits nothing. Anything else
                    // consuming coordinates still gets them in a register.
                    let only_feeds_textures = self.links.iter().filter(|l| l.from == *id).all(|l| {
                        matches!(
                            self.nodes.get(&l.to).map(|n| &n.kind),
                            Some(
                                NodeKind::ImageTexture { .. }
                                    | NodeKind::ImageAlpha { .. }
                                    | NodeKind::Procedural { .. }
                            )
                        )
                    });
                    if !only_feeds_textures {
                        instructions.push(Instruction::Project {
                            dst,
                            mapping: Box::new(*mapping),
                        });
                    }
                }
                NodeKind::Procedural { procedural, seed } => {
                    // The mapping travels with the instruction, as it does for
                    // a texture: a procedural is evaluated in object space by
                    // default (§6) and a mapping node is what moves it into
                    // another.
                    let mapping = self
                        .links
                        .iter()
                        .find(|l| l.to == *id && l.to_socket == Socket::Input(0))
                        .and_then(|l| self.nodes.get(&l.from))
                        .and_then(|n| match &n.kind {
                            NodeKind::Mapping2d { mapping } => Some(*mapping),
                            _ => None,
                        });
                    instructions.push(Instruction::Evaluate {
                        dst,
                        procedural: procedural.clone(),
                        seed: *seed,
                        mapping: mapping.map(Box::new),
                    });
                }
                NodeKind::VertexColor => {
                    instructions.push(Instruction::VertexColor { dst });
                }
                NodeKind::VolumeMap { volume } => {
                    requirements.volumes.insert(*volume);
                    instructions.push(Instruction::VolumeMap { dst, volume: *volume });
                }
                NodeKind::Occlusion { radius, contrast } => {
                    let widest = requirements.occlusion_radius().unwrap_or(0.0).max(*radius);
                    requirements.occlusion_radius_bits = Some(widest.to_bits());
                    instructions.push(Instruction::Occlusion { dst, contrast: *contrast });
                }
                NodeKind::Curvature { mode, radius, contrast } => {
                    instructions.push(Instruction::Curvature {
                        dst,
                        mode: *mode,
                        radius: *radius,
                        contrast: *contrast,
                    });
                }
                NodeKind::ImageTexture { texture, fallback } => {
                    requirements.textures.insert(*texture);
                    // The mapping is resolved here, at compile time, from the
                    // Mapping 2D node feeding this texture's coordinate input.
                    // Nothing connected means plain UV, which is what a texture
                    // dropped on a parameter with no mapping node should do.
                    let mapping = self
                        .links
                        .iter()
                        .find(|l| l.to == *id && l.to_socket == Socket::Input(0))
                        .and_then(|l| self.nodes.get(&l.from))
                        .and_then(|n| match &n.kind {
                            NodeKind::Mapping2d { mapping } => Some(*mapping),
                            _ => None,
                        })
                        .unwrap_or_default();
                    if mapping.projection == tequila_texture::projection::Projection::Uv {
                        requirements.uv_sets.insert(mapping.uv_set);
                    }
                    instructions.push(Instruction::Sample {
                        dst,
                        texture: *texture,
                        mapping: Box::new(mapping),
                        fallback: *fallback,
                    });
                }
                NodeKind::ImageAlpha { texture, fallback } => {
                    requirements.textures.insert(*texture);
                    // The same coordinate resolution as a colour lookup, so a
                    // mask shares its picture's mapping exactly. Anything else
                    // would slide the holes off the leaf.
                    let mapping = self
                        .links
                        .iter()
                        .find(|l| l.to == *id && l.to_socket == Socket::Input(0))
                        .and_then(|l| self.nodes.get(&l.from))
                        .and_then(|n| match &n.kind {
                            NodeKind::Mapping2d { mapping } => Some(*mapping),
                            _ => None,
                        })
                        .unwrap_or_default();
                    if mapping.projection == tequila_texture::projection::Projection::Uv {
                        requirements.uv_sets.insert(mapping.uv_set);
                    }
                    instructions.push(Instruction::SampleAlpha {
                        dst,
                        texture: *texture,
                        mapping: Box::new(mapping),
                        fallback: *fallback,
                    });
                }
                kind => {
                    // Any argument that folded needs a register to live in.
                    let mut args = Vec::with_capacity(arg_registers.len());
                    for (slot, value) in arg_registers.iter().zip(arg_values.iter()) {
                        match slot {
                            Some(r) => args.push(*r),
                            None => {
                                let r = Register(next_register);
                                next_register += 1;
                                instructions.push(Instruction::Load { dst: r, value: *value });
                                args.push(r);
                            }
                        }
                    }
                    instructions.push(Instruction::Apply {
                        dst,
                        kind: Box::new(kind.clone()),
                        args,
                        output: 0,
                    });
                }
            }
        }

        // The terminal: folded parameters become literals in the block, and
        // varying ones become entries the shader writes at run time.
        let mut constants = self.material().clone();
        let mut driven = Vec::new();
        for link in self.links.iter().filter(|l| l.to == self.output) {
            let Socket::Param(param) = link.to_socket else {
                continue;
            };
            let from = (link.from, link.from_socket);
            if let Some(value) = folded.get(&from) {
                if param == Param::Tangent {
                    // A slot the material holds no value for: a constant
                    // direction - brushed along X, say - still has to reach
                    // the renderer, so it lives in a register like a
                    // varying one.
                    let r = Register(next_register);
                    next_register += 1;
                    instructions.push(Instruction::Load { dst: r, value: *value });
                    driven.push((param, r));
                } else {
                    write_param(&mut constants, param, *value);
                }
            } else if let Some(register) = registers.get(&from) {
                driven.push((param, *register));
            }
        }
        driven.sort_by_key(|(p, _)| p.key());

        requirements.needs_medium = constants.has_volume();
        requirements.needs_tangents = constants.specular_roughness_anisotropy > 0.0
            || driven.iter().any(|(p, _)| *p == Param::Anisotropy);

        Ok(CompiledGraph {
            decals: self.decals.clone(),
            instructions,
            registers: next_register as usize,
            constants,
            driven,
            requirements,
            topology: self.topology_hash(),
        })
    }
}

/// What an input of type `ty` reads with nothing wired, no literal and no
/// default of its node's own.
pub fn default_for(ty: SocketType) -> Value {
    match ty {
        SocketType::Float => Value::Float(0.0),
        SocketType::Color => Value::Color(Spectrum::ZERO),
        SocketType::Vector | SocketType::Normal => Value::Vector(Vec3::ZERO),
        SocketType::Bool => Value::Bool(false),
    }
}

/// Write a graph value into a parameter, converting to whatever the parameter
/// stores.
fn write_param(material: &mut Material, param: Param, value: Value) {
    use crate::material_type::{ParamUi, ParamValue};
    let written = match param.ui() {
        ParamUi::Color | ParamUi::Vector => ParamValue::Color(value.as_color()),
        ParamUi::Flag => ParamValue::Flag(value.as_float() > 0.5),
        ParamUi::Scalar => ParamValue::Scalar(value.as_float()),
        // A texture slot and a metal preset are not values a link can carry.
        ParamUi::Slot | ParamUi::Metal => return,
    };
    param.write(material, written);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Restriction keeps exactly what the wanted outputs need, including a
    /// chain through an Apply, and nothing feeding only the others.
    #[test]
    fn a_restricted_program_keeps_only_what_it_needs() {
        use crate::material_type::Param;
        let (a, b, c, d) = (Register(0), Register(1), Register(2), Register(3));
        let full = CompiledGraph {
            decals: Vec::new(),
            instructions: vec![
                Instruction::Load { dst: a, value: Value::Float(0.5) },
                Instruction::Load { dst: b, value: Value::Float(2.0) },
                Instruction::Apply { dst: c, kind: Box::new(NodeKind::Constant(Value::Float(0.0))), args: vec![a, b], output: 0 },
                Instruction::Load { dst: d, value: Value::Float(0.9) },
            ],
            registers: 4,
            constants: Default::default(),
            driven: vec![(Param::BaseWeight, c), (Param::SpecularRoughness, d)],
            requirements: Default::default(),
            topology: 7,
        };
        let solid = full.restricted_to(&[Param::BaseWeight]);
        assert_eq!(solid.instructions.len(), 3, "the multiply and both of its inputs");
        assert_eq!(solid.driven, vec![(Param::BaseWeight, c)]);
        assert!(!solid.instructions.iter().any(|i| i.destination() == d), "roughness is not computed");

        // A register reused for a second value: the earlier write is dead.
        let reused = CompiledGraph {
            instructions: vec![
                Instruction::Load { dst: a, value: Value::Float(1.0) },
                Instruction::Load { dst: a, value: Value::Float(3.0) },
            ],
            driven: vec![(Param::BaseWeight, a)],
            ..full.clone()
        };
        let pruned = reused.restricted_to(&[Param::BaseWeight]);
        assert_eq!(pruned.instructions, vec![Instruction::Load { dst: a, value: Value::Float(3.0) }]);

        assert!(full.restricted_to(&[]).instructions.is_empty());
    }
    use tequila_core::spectrum::spectrum;
    use tequila_texture::projection::{Mapping, Projection};

    fn base() -> MaterialGraph {
        MaterialGraph::from_material(Material::default())
    }

    fn texture_chain(graph: &mut MaterialGraph, param: Param) -> NodeId {
        let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
            mapping: Mapping::default(),
        }));
        let texture = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(7),
            fallback: Value::Color(Spectrum::splat(0.5)),
        }));
        graph.connect(mapping, texture, Socket::Input(0));
        let output = graph.output();
        graph.connect(texture, output, Socket::Param(param));
        texture
    }

    /// §4.2: a material with no textures *is* a graph — one surface node whose
    /// literals are the parameter block. The simple panel and the graph cannot
    /// disagree because there is only one of them.
    #[test]
    fn a_material_with_no_textures_is_a_one_node_graph() {
        let material = Material {
            specular_roughness: 0.37,
            ..Default::default()
        };
        let graph = MaterialGraph::from_material(material.clone());

        assert_eq!(graph.len(), 1);
        assert!(graph.links().is_empty());
        assert_eq!(graph.material(), &material);
        assert!(graph.validate().is_ok());

        // ...and compiling it produces no work at all: every parameter is a
        // constant.
        let compiled = graph.compile().expect("compiles");
        assert!(
            compiled.instructions.is_empty(),
            "a textureless material emitted {} instructions",
            compiled.instructions.len()
        );
        assert_eq!(compiled.constants, material);
        assert!(compiled.driven.is_empty());
    }

    /// §4.3 step 2: a subgraph with no varying inputs folds to a literal at
    /// compile time, so a "texture" that turned out to be a flat colour costs
    /// nothing per shading point.
    #[test]
    fn a_constant_subgraph_folds_away_entirely() {
        let mut graph = base();
        let a = graph.add_node(Node::new(NodeKind::Constant(Value::Color(spectrum(
            0.8, 0.2, 0.1,
        )))));
        let b = graph.add_node(Node::new(NodeKind::Constant(Value::Color(spectrum(
            0.0, 0.4, 0.9,
        )))));
        let mix = graph.add_node(
            Node::new(NodeKind::Mix).with_literal(Socket::Input(2), Value::Float(0.25)),
        );
        graph.connect(a, mix, Socket::Input(0));
        graph.connect(b, mix, Socket::Input(1));
        let output = graph.output();
        graph.connect(mix, output, Socket::Param(Param::BaseColor));

        let compiled = graph.compile().expect("compiles");
        assert!(
            compiled.instructions.is_empty(),
            "a wholly constant subgraph emitted {} instructions",
            compiled.instructions.len()
        );
        assert!(compiled.driven.is_empty());

        // The folded value landed in the parameter block, and it is the mix.
        let expected = spectrum(0.8, 0.2, 0.1) * 0.75 + spectrum(0.0, 0.4, 0.9) * 0.25;
        let got = compiled.constants.base_color;
        assert!(
            (got.r - expected.r).abs() < 1e-6 && (got.b - expected.b).abs() < 1e-6,
            "folded to {got:?}, expected {expected:?}"
        );
    }

    /// A texture makes its whole downstream chain varying, so none of it folds.
    #[test]
    fn a_texture_makes_everything_downstream_varying() {
        let mut graph = base();
        let texture = texture_chain(&mut graph, Param::BaseColor);
        // An Invert after the texture must not fold, even though Invert itself
        // is a pure function.
        let invert = graph.add_node(Node::new(NodeKind::Invert));
        graph.connect(texture, invert, Socket::Input(0));
        let output = graph.output();
        graph.connect(invert, output, Socket::Param(Param::BaseColor));

        let compiled = graph.compile().expect("compiles");
        assert!(
            compiled
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Sample { .. })),
            "no texture sample was emitted"
        );
        assert!(
            compiled
                .instructions
                .iter()
                .any(|i| matches!(i, Instruction::Apply { .. })),
            "the invert downstream of a texture was folded away"
        );
        assert_eq!(compiled.driven.len(), 1);
        assert_eq!(compiled.driven[0].0, Param::BaseColor);
    }

    /// §4.3: the compiler reports what the renderer must arrange.
    #[test]
    fn compilation_reports_what_the_renderer_must_supply() {
        let mut graph = base();
        let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
            mapping: Mapping {
                projection: Projection::Uv,
                uv_set: 2,
                ..Default::default()
            },
        }));
        let texture = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(42),
            fallback: Value::Color(Spectrum::ZERO),
        }));
        graph.connect(mapping, texture, Socket::Input(0));
        let output = graph.output();
        graph.connect(texture, output, Socket::Param(Param::BaseColor));

        let compiled = graph.compile().expect("compiles");
        assert!(compiled.requirements.uv_sets.contains(&2));
        assert!(compiled.requirements.textures.contains(&TextureId(42)));
    }

    /// Tri-planar needs no UVs at all — which is the point of it for CAD
    /// geometry, and the renderer needs to know so it does not demand them.
    #[test]
    fn a_tri_planar_material_requires_no_uv_sets() {
        let mut graph = base();
        let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
            mapping: Mapping {
                projection: Projection::TriPlanar,
                ..Default::default()
            },
        }));
        let texture = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(1),
            fallback: Value::Color(Spectrum::ZERO),
        }));
        graph.connect(mapping, texture, Socket::Input(0));
        let output = graph.output();
        graph.connect(texture, output, Socket::Param(Param::BaseColor));

        let compiled = graph.compile().expect("compiles");
        assert!(
            compiled.requirements.uv_sets.is_empty(),
            "a tri-planar material asked for UV sets it does not read"
        );
    }

    /// §4.3 point 4, and the reason there are two hashes.
    ///
    /// Dragging a slider must not recompile: it changes values, not shape. Sixty
    /// recompiles a second while the user watches is the thing the point exists
    /// to forbid.
    /// The content hash says "whether the rendered result would differ", and a
    /// mapping's scale, offset, rotation, axis and pivot all change what is
    /// rendered.
    ///
    /// This is not bookkeeping. The mapping is compiled *into* the
    /// instruction, and `snapshot::compile_for` hands back the cached program
    /// whenever the content hash matches — so a mapping the hash cannot see is
    /// a mapping edit that never reaches the renderer. Dragging the texture
    /// scale would move the control and leave the picture alone.
    #[test]
    fn every_part_of_a_mapping_changes_the_content_hash() {
        use tequila_core::math::{vec2, vec3};
        use tequila_texture::projection::{Axis, Projection};

        let variations: [(&str, fn(&mut Mapping)); 8] = [
            ("projection", |m| m.projection = Projection::TriPlanar),
            ("uv_set", |m| m.uv_set = 1),
            ("scale", |m| m.scale = vec3(4.0, 4.0, 1.0)),
            ("offset", |m| m.offset = vec2(0.25, -0.5)),
            ("rotation", |m| m.rotation = 0.125),
            ("axis", |m| m.axis = Axis::X),
            ("pivot", |m| m.pivot = vec2(0.5, 0.5)),
            ("real_world_units", |m| m.real_world_units = true),
        ];

        for (name, change) in variations {
            let mut graph = base();
            texture_chain(&mut graph, Param::BaseColor);
            let before = graph.content_hash();

            let node = graph
                .nodes
                .iter()
                .find(|(_, n)| matches!(n.kind, NodeKind::Mapping2d { .. }))
                .map(|(id, _)| *id)
                .expect("the chain has a mapping node");
            if let Some(NodeKind::Mapping2d { mapping }) =
                graph.nodes.get_mut(&node).map(|n| &mut n.kind)
            {
                change(mapping);
            }

            assert_ne!(
                graph.content_hash(),
                before,
                "changing a mapping's {name} left the content hash alone, so \
                 the cached program would be reused and the edit would never \
                 reach the render"
            );
        }
    }

    /// The same hole, on the nodes whose controls I most recently added.
    ///
    /// `topology_hash` deliberately keeps a procedural's parameters out,
    /// because changing a frequency does not change the emitted program — and
    /// says so, pointing at the content hash as the place they belong. They
    /// were in neither, so dragging Marble's band spacing would have moved the
    /// slider and left the render alone.
    #[test]
    fn a_procedurals_parameters_change_the_content_hash() {
        use tequila_texture::noise::Fractal;
        use tequila_texture::procedural::{Axis, Procedural};

        let marble = |frequency: f32, distortion: f32| Procedural::Marble {
            axis: Axis::Y,
            frequency,
            distortion,
            fractal: Fractal::default(),
        };
        let hash_of = |procedural: Procedural| {
            let mut graph = base();
            let node = graph.add_node(Node::new(NodeKind::Procedural {
                procedural: Box::new(procedural),
                seed: 4,
            }));
            let output = graph.output();
            graph.connect(node, output, Socket::Param(Param::SpecularRoughness));
            graph.content_hash()
        };

        let reference = hash_of(marble(3.0, 1.0));
        assert_ne!(
            hash_of(marble(9.0, 1.0)),
            reference,
            "changing the band spacing left the content hash alone"
        );
        assert_ne!(
            hash_of(marble(3.0, 4.0)),
            reference,
            "changing the vein distortion left the content hash alone"
        );
        // ...and the same procedural really does hash the same, or the
        // assertions above would hold for any change at all, including none.
        assert_eq!(hash_of(marble(3.0, 1.0)), reference);
    }

    #[test]
    fn moving_a_slider_does_not_change_the_topology() {
        let mut graph = base();
        texture_chain(&mut graph, Param::BaseColor);
        let compiled = graph.compile().expect("compiles");

        let topology_before = graph.topology_hash();
        let content_before = graph.content_hash();

        graph.material_mut().specular_roughness = 0.9;

        assert_eq!(
            graph.topology_hash(),
            topology_before,
            "changing a parameter value changed the topology, forcing a recompile"
        );
        assert_ne!(
            graph.content_hash(),
            content_before,
            "changing a parameter value left the content hash alone"
        );
        assert!(
            compiled.can_reuse_for(&graph),
            "the compiled program should be reusable with patched values"
        );
    }

    /// ...and adding a node *does* change the topology, or the cache would
    /// serve a stale program.
    #[test]
    fn adding_a_node_changes_the_topology() {
        let mut graph = base();
        let compiled = graph.compile().expect("compiles");
        let before = graph.topology_hash();

        texture_chain(&mut graph, Param::BaseColor);

        assert_ne!(graph.topology_hash(), before);
        assert!(
            !compiled.can_reuse_for(&graph),
            "a graph that gained a texture reused a program that has no sample in it"
        );
    }

    /// A muted node passes its first wire of the right type straight on, and
    /// a muted node with nothing to pass leaves the input on its literal.
    #[test]
    fn a_muted_node_passes_its_input_through() {
        use crate::nodes::ShaderNode;
        let mut graph = MaterialGraph::from_material(Material::default());
        let red = Value::Color(Spectrum { r: 0.9, g: 0.1, b: 0.1 });
        let colour = graph.add_node(Node::new(NodeKind::Constant(red)));
        let invert = graph.add_node(Node::new(NodeKind::Node(ShaderNode::InvertColor)));
        graph.connect(colour, invert, Socket::Input(1));
        let output = graph.output();
        graph.connect(invert, output, Socket::Param(Param::BaseColor));
        let inverted = graph.compile().unwrap().constants.base_color;
        assert!(inverted.g > 0.8, "inverted, the red is cyan: {inverted:?}");
        graph.node_mut(invert).unwrap().muted = true;
        assert_ne!(graph.topology_hash(), { let mut g = graph.clone(); g.node_mut(invert).unwrap().muted = false; g.topology_hash() });
        let passed = graph.compile().unwrap().constants.base_color;
        assert!((passed.r - 0.9).abs() < 1e-6 && (passed.g - 0.1).abs() < 1e-6, "muted, the red passes: {passed:?}");
        // Nothing wired in: the surface keeps its own colour.
        graph.disconnect(invert, Socket::Input(1));
        let own = graph.material().base_color;
        assert_eq!(graph.compile().unwrap().constants.base_color, own);
    }

    /// A wire through a reroute still comes from the output it left: a
    /// node's second output, not its first.
    #[test]
    fn a_reroute_keeps_the_output_a_wire_left() {
        use crate::nodes::ShaderNode;
        let mut graph = MaterialGraph::from_material(Material::default());
        let split = graph.add_node(Node::new(NodeKind::Node(ShaderNode::SeparateXyz)).with_literal(Socket::Input(0), Value::Vector(tequila_core::math::vec3(0.1, 0.6, 0.3))));
        let hop = graph.add_node(Node::new(NodeKind::Reroute { ty: SocketType::Float }));
        graph.connect_output(split, 1, hop, Socket::Input(0));
        let output = graph.output();
        graph.connect(hop, output, Socket::Param(Param::SpecularRoughness));
        let roughness = graph.compile().unwrap().constants.specular_roughness;
        assert!((roughness - 0.6).abs() < 1e-6, "Y through the reroute, not X: {roughness}");
    }

    /// §4.1's type rules, which are Cycles': numbers, colours and vectors
    /// link to each other as Cycles converts them; a flag links to nothing
    /// else, so the editor can say so.
    #[test]
    fn conversions_are_the_ones_cycles_makes() {
        use SocketType::*;
        assert!(Float.converts_to(Color), "float should broadcast to colour");
        assert!(Color.converts_to(Float), "colour should reduce to luminance");
        assert!(Float.converts_to(Float));
        assert!(Float.converts_to(Vector), "float should spread over a vector");
        assert!(Vector.converts_to(Float), "a vector should average to a number");
        assert!(Color.converts_to(Vector) && Vector.converts_to(Color));

        assert!(!Bool.converts_to(Color));
        assert!(!Color.converts_to(Bool));
        assert_eq!(Value::Vector(tequila_core::math::vec3(0.0, 0.3, 0.6)).convert(Float), Some(Value::Float(0.3)));

        // And the values follow the same rules.
        assert_eq!(
            Value::Float(0.5).convert(Color),
            Some(Value::Color(Spectrum::splat(0.5)))
        );
        assert_eq!(Value::Bool(true).convert(Color), None);
    }

    /// A link between incompatible sockets is refused with enough detail for
    /// the editor to highlight it.
    #[test]
    fn a_mistyped_link_is_refused() {
        let mut graph = base();
        // A Mapping node outputs a Vector; ThinWalled takes a Bool.
        let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
            mapping: Mapping::default(),
        }));
        let output = graph.output();
        graph.connect(mapping, output, Socket::Param(Param::ThinWalled));

        match graph.validate() {
            Err(GraphError::TypeMismatch { from, to, .. }) => {
                assert_eq!(from, SocketType::Vector);
                assert_eq!(to, SocketType::Bool);
            }
            other => panic!("expected a type mismatch, got {other:?}"),
        }
        assert!(graph.compile().is_err());
    }

    /// A cycle must be caught, and the error must name the nodes so the editor
    /// can point at them.
    #[test]
    fn a_cycle_is_reported_with_the_nodes_involved() {
        let mut graph = base();
        let a = graph.add_node(Node::new(NodeKind::Invert));
        let b = graph.add_node(Node::new(NodeKind::Invert));
        graph.connect(a, b, Socket::Input(0));
        graph.connect(b, a, Socket::Input(0));

        match graph.validate() {
            Err(GraphError::Cycle(nodes)) => {
                assert!(nodes.contains(&a) && nodes.contains(&b), "got {nodes:?}");
            }
            other => panic!("expected a cycle, got {other:?}"),
        }
    }

    #[test]
    fn a_dangling_link_is_refused() {
        let mut graph = base();
        let output = graph.output();
        graph.connect(NodeId(999), output, Socket::Param(Param::BaseColor));
        assert!(matches!(graph.validate(), Err(GraphError::DanglingLink(_))));
    }

    /// An input takes one link. Connecting again replaces rather than stacking,
    /// because in an editor dragging onto an occupied socket means "instead of".
    #[test]
    fn connecting_twice_replaces_rather_than_duplicating() {
        let mut graph = base();
        let a = graph.add_node(Node::new(NodeKind::Constant(Value::Float(0.1))));
        let b = graph.add_node(Node::new(NodeKind::Constant(Value::Float(0.9))));
        let output = graph.output();
        graph.connect(a, output, Socket::Param(Param::BaseColor));
        graph.connect(b, output, Socket::Param(Param::BaseColor));

        assert_eq!(graph.links().len(), 1);
        assert_eq!(graph.driver_of(Param::BaseColor), Some(b));
        assert!(graph.validate().is_ok());
    }

    /// §4.2's panel affordances: the panel needs to know which parameters are
    /// driven, and by what, so it can show an indicator and a jump-to-node.
    #[test]
    fn the_panel_can_tell_which_parameters_are_driven() {
        let mut graph = base();
        assert!(!graph.is_connected(Param::BaseColor));
        assert_eq!(graph.driver_of(Param::BaseColor), None);

        let texture = texture_chain(&mut graph, Param::BaseColor);

        assert!(graph.is_connected(Param::BaseColor));
        assert_eq!(graph.driver_of(Param::BaseColor), Some(texture));
        assert!(
            !graph.is_connected(Param::SpecularRoughness),
            "an unconnected parameter reported itself as driven"
        );
    }

    /// Removing a node takes its links with it, or validation trips over a
    /// dangling reference the user cannot see.
    #[test]
    fn removing_a_node_removes_its_links() {
        let mut graph = base();
        let texture = texture_chain(&mut graph, Param::BaseColor);
        assert!(!graph.links().is_empty());

        graph.remove_node(texture);

        assert!(graph.validate().is_ok(), "a stale link survived the removal");
        assert!(!graph.is_connected(Param::BaseColor));
    }

    /// The terminal cannot be deleted — a graph without one has nothing to
    /// render.
    #[test]
    fn the_output_node_cannot_be_removed() {
        let mut graph = base();
        let output = graph.output();
        assert_eq!(graph.remove_node(output), None);
        assert!(graph.validate().is_ok());
    }

    /// Producers come before consumers, whatever order the nodes were added in.
    #[test]
    fn instructions_come_out_in_dependency_order() {
        let mut graph = base();
        // Build back to front, so insertion order is the reverse of the
        // dependency order.
        let invert = graph.add_node(Node::new(NodeKind::Invert));
        let texture = graph.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(3),
            fallback: Value::Color(Spectrum::ZERO),
        }));
        let mapping = graph.add_node(Node::new(NodeKind::Mapping2d {
            mapping: Mapping::default(),
        }));
        graph.connect(mapping, texture, Socket::Input(0));
        graph.connect(texture, invert, Socket::Input(0));
        let output = graph.output();
        graph.connect(invert, output, Socket::Param(Param::BaseColor));

        let compiled = graph.compile().expect("compiles");

        // Every register is written before it is read.
        let mut written = BTreeSet::new();
        for instruction in &compiled.instructions {
            match instruction {
                // A sample reads no registers: its mapping is folded in at
                // compile time, because tri-planar blends three lookups and a
                // single coordinate pair cannot carry that.
                Instruction::Apply { args, .. } => {
                    for arg in args {
                        assert!(
                            written.contains(arg),
                            "an apply read register {arg:?} before it was written"
                        );
                    }
                }
                _ => {}
            }
            written.insert(instruction.destination());
        }
        assert!(compiled.registers > 0);
    }

    /// Every parameter must be reachable as a socket, or the graph cannot
    /// express something the simple panel can.
    #[test]
    fn every_parameter_is_a_socket_on_the_surface_node() {
        let surface = NodeKind::Surface(Box::new(Material::default()));
        let sockets: BTreeSet<Socket> = surface.inputs().into_iter().map(|(s, _, _)| s).collect();
        for param in Param::ALL {
            assert!(
                sockets.contains(&Socket::Param(param)),
                "`{}` cannot be driven by the graph",
                param.key()
            );
        }
        assert_eq!(sockets.len(), Param::ALL.len());
    }

    fn noise() -> Node {
        Node::new(NodeKind::Procedural {
            procedural: Box::new(tequila_texture::procedural::Procedural::catalog()[0]),
            seed: 3,
        })
    }

    /// A wire through any chain of reroutes compiles to exactly what the wire
    /// straight across does — including a mapping that reaches its texture
    /// through one, which the compiler looks for by walking the wire.
    #[test]
    fn reroutes_compile_to_the_wire_they_bend() {
        let base = MaterialGraph::from_material(Material::plastic(Spectrum::splat(0.5), 0.3));

        let mut direct = base.clone();
        let mapping = direct.add_node(Node::new(NodeKind::Mapping2d {
            mapping: tequila_texture::projection::Mapping {
                rotation: 0.7,
                ..Default::default()
            },
        }));
        let texture = direct.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(1),
            fallback: Value::Color(Spectrum::splat(0.2)),
        }));
        let pattern = direct.add_node(noise());
        let output = direct.output();
        direct.connect(mapping, texture, Socket::Input(0));
        direct.connect(texture, output, Socket::Param(Param::BaseColor));
        direct.connect(pattern, output, Socket::Param(Param::SpecularRoughness));

        let mut bent = base.clone();
        let mapping = bent.add_node(Node::new(NodeKind::Mapping2d {
            mapping: tequila_texture::projection::Mapping {
                rotation: 0.7,
                ..Default::default()
            },
        }));
        let texture = bent.add_node(Node::new(NodeKind::ImageTexture {
            texture: TextureId(1),
            fallback: Value::Color(Spectrum::splat(0.2)),
        }));
        let pattern = bent.add_node(noise());
        let hop = bent.add_node(Node::new(NodeKind::Reroute { ty: SocketType::Vector }));
        let first = bent.add_node(Node::new(NodeKind::Reroute { ty: SocketType::Float }));
        let second = bent.add_node(Node::new(NodeKind::Reroute { ty: SocketType::Float }));
        let output = bent.output();
        bent.connect(mapping, hop, Socket::Input(0));
        bent.connect(hop, texture, Socket::Input(0));
        bent.connect(texture, output, Socket::Param(Param::BaseColor));
        bent.connect(pattern, first, Socket::Input(0));
        bent.connect(first, second, Socket::Input(0));
        bent.connect(second, output, Socket::Param(Param::SpecularRoughness));

        let (a, b) = (direct.compile().unwrap(), bent.compile().unwrap());
        assert_eq!(format!("{:?}", a.instructions), format!("{:?}", b.instructions));
        assert_eq!(format!("{:?}", a.driven), format!("{:?}", b.driven));
        assert_eq!(a.requirements, b.requirements);
    }

    /// A reroute fed by nothing leaves what it feeds on its own value.
    #[test]
    fn an_unfed_reroute_changes_nothing() {
        let mut graph = MaterialGraph::from_material(Material::plastic(Spectrum::splat(0.5), 0.3));
        let plain = graph.compile().unwrap();
        let hop = graph.add_node(Node::new(NodeKind::Reroute { ty: SocketType::Float }));
        let output = graph.output();
        graph.connect(hop, output, Socket::Param(Param::SpecularRoughness));
        let rerouted = graph.compile().unwrap();
        assert_eq!(format!("{:?}", plain.constants), format!("{:?}", rerouted.constants));
    }

    /// Solo shows the one node, unlit: the surface glows with its value and
    /// reflects nothing, and nothing else drives it.
    #[test]
    fn solo_shows_one_node_and_nothing_else() {
        let mut graph = MaterialGraph::from_material(Material::plastic(Spectrum::splat(0.5), 0.3));
        let pattern = graph.add_node(noise());
        let colour = graph.add_node(Node::new(NodeKind::Constant(Value::Color(Spectrum::splat(0.9)))));
        let output = graph.output();
        graph.connect(pattern, output, Socket::Param(Param::SpecularRoughness));
        graph.connect(colour, output, Socket::Param(Param::BaseColor));

        let soloed = graph.soloed(pattern).expect("a pattern can be soloed");
        let into_surface: Vec<(NodeId, Socket)> = soloed
            .links()
            .iter()
            .filter(|l| l.to == soloed.output())
            .map(|l| (l.from, l.to_socket))
            .collect();
        assert_eq!(into_surface, vec![(pattern, Socket::Param(Param::EmissionColor))]);
        let Some(NodeKind::Surface(material)) = soloed.node(soloed.output()).map(|n| &n.kind) else {
            panic!("no surface")
        };
        assert_eq!((material.base_weight, material.specular_weight), (0.0, 0.0));
        assert!(material.emission_luminance > 0.0);
        assert!(soloed.compile().is_ok());

        // Something that is not a value or a colour has nothing to show.
        let mapping = graph.add_node(Node::new(NodeKind::Mapping2d { mapping: Default::default() }));
        assert!(graph.soloed(mapping).is_none());
        // The original is untouched.
        assert!(graph.links().iter().any(|l| l.from == colour));
    }

    /// Groups are layout: drawing one does not change what renders.
    #[test]
    fn frames_do_not_change_the_content() {
        let mut graph = MaterialGraph::from_material(Material::plastic(Spectrum::splat(0.5), 0.3));
        let before = graph.content_hash();
        graph.frames.push(Frame {
            title: "Grain".into(),
            min: Vec2 { x: 0.0, y: 0.0 },
            max: Vec2 { x: 300.0, y: 200.0 },
        });
        assert_eq!(graph.content_hash(), before);
    }

    /// A constant direction wired to the Tangent socket reaches the
    /// renderer: the material holds no tangent to fold it into, so it is
    /// loaded into a register and listed as driven, as a varying one is.
    #[test]
    fn a_constant_tangent_is_driven() {
        use crate::material_type::{MaterialType, Param};
        let mut graph = MaterialGraph::from_material(MaterialType::Anisotropic.defaults());
        let constant = graph.add_node(Node::new(NodeKind::Constant(Value::Vector(Vec3::X))));
        let output = graph.output();
        graph.connect(constant, output, Socket::Param(Param::Tangent));
        let program = graph.compile().expect("compiles");
        let register = program.driven.iter().find(|(p, _)| *p == Param::Tangent).map(|(_, r)| *r).expect("the tangent is driven");
        assert!(program
            .instructions
            .iter()
            .any(|i| matches!(i, Instruction::Load { dst, value: Value::Vector(v) } if *dst == register && *v == Vec3::X)));
    }
}
