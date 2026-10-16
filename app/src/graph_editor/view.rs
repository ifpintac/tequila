//! Where everything in a node goes, and what each field in it edits.
//!
//! A node is laid out as Blender lays one out: its header, its outputs, a
//! preview of what it makes, its settings, then its inputs, each unwired
//! input with its value in place - a slider, a swatch, three numbers - so a
//! graph can be read and tuned without leaving the canvas. Everything here
//! is in canvas units; the canvas scales it by the zoom.

use tequila_core::spectrum::Spectrum;
use tequila_materials::graph::{MaterialGraph, Node, NodeId, NodeKind, Socket, SocketType, Value};
use tequila_materials::material_type::{Param, ParamUi, ParamValue};
use tequila_materials::nodes::{NodeCategory, Setting, ShaderNode};
use std::collections::HashSet;

pub const HEADER: f32 = 26.0;
pub const ROW: f32 = 22.0;
/// A vector input's X, Y and Z, stacked under its name.
pub const COMPONENT: f32 = 20.0;
/// Space inside a node's edge.
pub const PAD: f32 = 8.0;
pub const SOCKET_RADIUS: f32 = 5.0;
/// A reroute is a dot on a wire.
pub const REROUTE: f32 = 14.0;
/// Node preview resolution, width by height: the shape of the box it is
/// drawn in, so its pixels come out square.
pub const PREVIEW_PIXELS: [usize; 2] = [96, 60];

/// What a node is, for its colour and its place in the Add menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Family {
    Input,
    Texture,
    Pattern,
    Color,
    Converter,
    Vector,
    /// Nodes that read the part itself: its curvature, its crevices.
    Surface,
    Output,
    Layout,
}

impl Family {
    pub const MENU: [Family; 8] = [
        Family::Input,
        Family::Texture,
        Family::Pattern,
        Family::Color,
        Family::Converter,
        Family::Vector,
        Family::Surface,
        Family::Layout,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Family::Input => "Input",
            Family::Texture => "Texture",
            Family::Pattern => "Pattern",
            Family::Color => "Color",
            Family::Converter => "Converter",
            Family::Vector => "Vector",
            Family::Surface => "Surface",
            Family::Output => "Output",
            Family::Layout => "Layout",
        }
    }

    /// The header's tint. Muted, so a graph reads as a few families of
    /// nodes rather than a box of crayons, and none is mistaken for the
    /// selection's colour.
    pub fn tint(self) -> egui::Color32 {
        match self {
            Family::Input => egui::Color32::from_rgb(126, 62, 74),
            Family::Texture => egui::Color32::from_rgb(122, 86, 46),
            Family::Pattern => egui::Color32::from_rgb(112, 92, 52),
            Family::Color => egui::Color32::from_rgb(110, 102, 48),
            Family::Converter => egui::Color32::from_rgb(48, 86, 122),
            Family::Vector => egui::Color32::from_rgb(78, 64, 124),
            Family::Surface => egui::Color32::from_rgb(40, 104, 102),
            Family::Output => egui::Color32::from_rgb(52, 104, 70),
            Family::Layout => egui::Color32::from_rgb(70, 70, 74),
        }
    }
}

pub fn family(kind: &NodeKind) -> Family {
    match kind {
        NodeKind::Surface(_) => Family::Output,
        NodeKind::Node(node) => match node.category() {
            NodeCategory::Input => Family::Input,
            NodeCategory::Texture => Family::Texture,
            NodeCategory::Color => Family::Color,
            NodeCategory::Converter => Family::Converter,
            NodeCategory::Vector => Family::Vector,
        },
        NodeKind::Constant(_) | NodeKind::Mapping2d { .. } | NodeKind::VertexColor => Family::Input,
        NodeKind::ImageTexture { .. } | NodeKind::ImageAlpha { .. } | NodeKind::VolumeMap { .. } => Family::Texture,
        NodeKind::Procedural { .. } => Family::Pattern,
        NodeKind::Gradient { .. } | NodeKind::Remap { .. } | NodeKind::Clamp { .. } | NodeKind::Channel { .. } => Family::Converter,
        NodeKind::Mix | NodeKind::Multiply | NodeKind::Add | NodeKind::Invert => Family::Color,
        NodeKind::Bump { .. } => Family::Vector,
        NodeKind::Curvature { .. } | NodeKind::Occlusion { .. } | NodeKind::Bubbles { .. } | NodeKind::Fuzz { .. } => Family::Surface,
        NodeKind::Reroute { .. } => Family::Layout,
    }
}

/// A socket's colour, as Blender colours them, so a wire's type reads at a
/// glance to anyone who has used a node editor.
pub fn type_colour(ty: SocketType) -> egui::Color32 {
    match ty {
        SocketType::Float => egui::Color32::from_rgb(161, 161, 161),
        SocketType::Color => egui::Color32::from_rgb(199, 199, 41),
        SocketType::Vector | SocketType::Normal => egui::Color32::from_rgb(99, 99, 199),
        SocketType::Bool => egui::Color32::from_rgb(204, 102, 217),
    }
}

/// A node's title.
pub fn title(node: &Node) -> String {
    if let Some(label) = &node.label {
        return label.clone();
    }
    match &node.kind {
        NodeKind::Mapping2d { .. } => "Projection".into(),
        NodeKind::Gradient { .. } => "Color Ramp".into(),
        NodeKind::Constant(Value::Color(_)) => "RGB".into(),
        NodeKind::Constant(_) => "Value".into(),
        NodeKind::Node(node) => node.label().into(),
        other => other.label().into(),
    }
}

pub fn node_width(kind: &NodeKind) -> f32 {
    match kind {
        NodeKind::Surface(_) => 236.0,
        NodeKind::Reroute { .. } => REROUTE,
        NodeKind::Node(node) => match node.category() {
            NodeCategory::Texture => 192.0,
            NodeCategory::Input => 164.0,
            _ => 172.0,
        },
        NodeKind::Procedural { .. } | NodeKind::ImageTexture { .. } | NodeKind::ImageAlpha { .. } | NodeKind::Gradient { .. } => 204.0,
        _ => 172.0,
    }
}

/// Whether a node shows a picture of what it makes: the patterns and
/// pictures, whose output is something to look at.
pub fn has_preview(node: &Node) -> bool {
    match &node.kind {
        NodeKind::Node(shader) => shader.category() == NodeCategory::Texture,
        NodeKind::Procedural { .. } | NodeKind::ImageTexture { .. } | NodeKind::ImageAlpha { .. } | NodeKind::Gradient { .. } => true,
        _ => false,
    }
}

/// A setting a node's body offers as a dropdown, a number or a switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SettingRef {
    /// One of a Blender node's settings, by its place in `settings()`.
    Shader(usize),
    /// How a colour ramp goes between its stops.
    RampInterpolation,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RowKind {
    Output(u8),
    Preview,
    /// A colour ramp's bar.
    Ramp,
    Setting(SettingRef),
    /// An input, by its place in the kind's `inputs()`.
    Input(usize),
    /// One of an unwired vector input's three numbers.
    Component(usize, u8),
    /// A Value or RGB node's own value.
    Value,
    Note(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub kind: RowKind,
    pub top: f32,
    pub height: f32,
}

/// What an unwired input shows in place.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Inline {
    /// Its name alone: it reads the surface, or has nothing to type.
    Label,
    Number { range: Option<(f32, f32)> },
    Color,
    Bool,
    Vector,
}

/// A node laid out, in canvas units.
#[derive(Clone, Debug)]
pub struct NodeLayout {
    pub rect: egui::Rect,
    pub rows: Vec<Row>,
    /// Each output: its index, type and where its socket is.
    pub outputs: Vec<(u8, SocketType, egui::Pos2)>,
    /// Each drawn input: its socket, its index in `inputs()`, type and place.
    pub inputs: Vec<(Socket, usize, SocketType, egui::Pos2)>,
    pub collapsed: bool,
}

impl NodeLayout {
    pub fn header(&self) -> egui::Rect {
        egui::Rect::from_min_size(self.rect.min, egui::vec2(self.rect.width(), HEADER.min(self.rect.height())))
    }

    pub fn output_at(&self, index: u8) -> Option<egui::Pos2> {
        self.outputs.iter().find(|o| o.0 == index).map(|o| o.2)
    }

    pub fn input_at(&self, socket: Socket) -> Option<egui::Pos2> {
        self.inputs.iter().find(|i| i.0 == socket).map(|i| i.3)
    }

    /// A row's rectangle, inset to where its field is drawn.
    pub fn row_rect(&self, row: &Row) -> egui::Rect {
        egui::Rect::from_min_size(
            egui::pos2(self.rect.left() + PAD, self.rect.top() + row.top + 2.0),
            egui::vec2(self.rect.width() - 2.0 * PAD, row.height - 4.0),
        )
    }
}

/// The inputs a node draws, each with its index in `inputs()`.
///
/// The surface node has one per parameter of the whole parameter block;
/// showing all of them buries the few that matter, so it shows its material
/// type's own and any that are wired. A Blender node shows what its settings
/// use, as Blender hides a Math node's unused values.
pub fn visible_inputs(node: &Node, id: NodeId, connected: &HashSet<(NodeId, Socket)>, exposed: &[Param]) -> Vec<(usize, Socket, SocketType, &'static str)> {
    node.kind
        .inputs()
        .into_iter()
        .enumerate()
        .filter(|(index, (socket, _, _))| match &node.kind {
            NodeKind::Surface(_) => match socket {
                Socket::Param(p) => exposed.contains(p) || connected.contains(&(id, *socket)),
                Socket::Input(_) => false,
            },
            kind => kind.shows_input(*index),
        })
        .map(|(index, (socket, ty, label))| (index, socket, ty, label))
        .collect()
}

pub fn inline_of(node: &Node, index: usize, socket: Socket, ty: SocketType) -> Inline {
    match &node.kind {
        NodeKind::Surface(_) => match socket {
            Socket::Param(p) => match p.ui() {
                ParamUi::Scalar => Inline::Number { range: p.range().map(|r| (r.low, r.high)) },
                ParamUi::Color => Inline::Color,
                ParamUi::Flag => Inline::Bool,
                _ => Inline::Label,
            },
            Socket::Input(_) => Inline::Label,
        },
        NodeKind::Node(shader) => {
            let Some(spec) = shader.inputs().get(index).copied() else { return Inline::Label };
            if spec.implicit.is_some() {
                // It reads the surface when nothing is wired: nothing to type.
                return Inline::Label;
            }
            match spec.ty {
                SocketType::Float => Inline::Number { range: spec.range },
                SocketType::Color => Inline::Color,
                SocketType::Vector | SocketType::Normal => Inline::Vector,
                SocketType::Bool => Inline::Bool,
            }
        }
        NodeKind::Reroute { .. } => Inline::Label,
        NodeKind::Mix => match index {
            2 => Inline::Number { range: Some((0.0, 1.0)) },
            _ => Inline::Color,
        },
        _ => match ty {
            SocketType::Float => Inline::Number { range: None },
            SocketType::Color => Inline::Color,
            SocketType::Bool => Inline::Bool,
            // Coordinates come from a projection or the part.
            SocketType::Vector | SocketType::Normal => Inline::Label,
        },
    }
}

/// A short line saying what a node without fields of its own does.
fn note(node: &Node, texture_name: &dyn Fn(tequila_materials::graph::TextureId) -> String) -> Option<String> {
    Some(match &node.kind {
        NodeKind::Procedural { seed, .. } => format!("Seed {seed} · settings in the sidebar"),
        NodeKind::ImageTexture { texture, .. } | NodeKind::ImageAlpha { texture, .. } => texture_name(*texture),
        NodeKind::VolumeMap { volume } => texture_name(*volume),
        NodeKind::Mapping2d { mapping } => format!("{:?}", mapping.projection),
        NodeKind::Bump { depth } => format!("Depth {:.2} mm", depth * 1000.0),
        NodeKind::Curvature { mode, radius, .. } => format!("{} · {:.1} mm", mode.label(), radius * 1000.0),
        NodeKind::Occlusion { radius, .. } => format!("Reach {:.1} mm", radius * 1000.0),
        NodeKind::Bubbles { count, .. } => format!("{count} bubbles"),
        NodeKind::Fuzz { length, density, .. } => format!("{:.1} mm · {density:.0}/cm²", length * 1000.0),
        NodeKind::Remap { from, to } => format!("{:.2}–{:.2} to {:.2}–{:.2}", from.x, from.y, to.x, to.y),
        NodeKind::Clamp { low, high } => format!("Between {low:.2} and {high:.2}"),
        NodeKind::Channel { channel } => format!("{} channel", channel.label()),
        _ => return None,
    })
}

/// Lay a node out.
pub fn layout(
    node: &Node,
    id: NodeId,
    connected: &HashSet<(NodeId, Socket)>,
    exposed: &[Param],
    previews: bool,
    texture_name: &dyn Fn(tequila_materials::graph::TextureId) -> String,
) -> NodeLayout {
    let origin = egui::pos2(node.position.x, node.position.y);
    if let NodeKind::Reroute { ty } = node.kind {
        let rect = egui::Rect::from_center_size(origin, egui::vec2(REROUTE, REROUTE));
        return NodeLayout {
            rect,
            rows: Vec::new(),
            outputs: vec![(0, ty, origin)],
            inputs: vec![(Socket::Input(0), 0, ty, origin)],
            collapsed: false,
        };
    }
    let width = node_width(&node.kind);
    let outputs = node.kind.outputs();
    let inputs = visible_inputs(node, id, connected, exposed);

    if node.collapsed {
        let rect = egui::Rect::from_min_size(origin, egui::vec2(width, HEADER));
        let spread = |i: usize, n: usize| origin.y + HEADER * (i as f32 + 1.0) / (n as f32 + 1.0);
        return NodeLayout {
            rect,
            rows: Vec::new(),
            outputs: outputs
                .iter()
                .enumerate()
                .map(|(i, (_, ty))| (i as u8, *ty, egui::pos2(rect.right(), spread(i, outputs.len()))))
                .collect(),
            inputs: inputs
                .iter()
                .enumerate()
                .map(|(i, (index, socket, ty, _))| (*socket, *index, *ty, egui::pos2(rect.left(), spread(i, inputs.len()))))
                .collect(),
            collapsed: true,
        };
    }

    let mut rows = Vec::new();
    let mut y = HEADER + 4.0;
    let push = |rows: &mut Vec<Row>, kind: RowKind, height: f32, y: &mut f32| {
        rows.push(Row { kind, top: *y, height });
        *y += height;
    };
    for i in 0..outputs.len() {
        push(&mut rows, RowKind::Output(i as u8), ROW, &mut y);
    }
    if previews && has_preview(node) {
        let height = (width - 2.0 * PAD) * PREVIEW_PIXELS[1] as f32 / PREVIEW_PIXELS[0] as f32 + 6.0;
        push(&mut rows, RowKind::Preview, height, &mut y);
    }
    match &node.kind {
        NodeKind::Gradient { .. } => {
            push(&mut rows, RowKind::Setting(SettingRef::RampInterpolation), ROW, &mut y);
            push(&mut rows, RowKind::Ramp, ROW + 4.0, &mut y);
        }
        NodeKind::Node(shader) => {
            for i in 0..shader.settings().len() {
                push(&mut rows, RowKind::Setting(SettingRef::Shader(i)), ROW, &mut y);
            }
        }
        NodeKind::Constant(value) => {
            let height = if matches!(value, Value::Vector(_)) { ROW + 2.0 * COMPONENT } else { ROW };
            push(&mut rows, RowKind::Value, height, &mut y);
        }
        _ => {
            if let Some(text) = note(node, texture_name) {
                push(&mut rows, RowKind::Note(text), ROW, &mut y);
            }
        }
    }
    let mut input_sockets = Vec::new();
    for (index, socket, ty, _) in &inputs {
        input_sockets.push((*socket, *index, *ty, egui::pos2(origin.x, origin.y + y + ROW * 0.5)));
        push(&mut rows, RowKind::Input(*index), ROW, &mut y);
        if !connected.contains(&(id, *socket)) && inline_of(node, *index, *socket, *ty) == Inline::Vector {
            for axis in 0..3 {
                push(&mut rows, RowKind::Component(*index, axis), COMPONENT, &mut y);
            }
        }
    }
    let rect = egui::Rect::from_min_size(origin, egui::vec2(width, y + 6.0));
    let outputs = outputs
        .iter()
        .enumerate()
        .map(|(i, (_, ty))| {
            let row = &rows[i];
            (i as u8, *ty, egui::pos2(rect.right(), origin.y + row.top + row.height * 0.5))
        })
        .collect();
    NodeLayout { rect, rows, outputs, inputs: input_sockets, collapsed: false }
}

/// Something a field in a node edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// An input's value while nothing is wired to it: a literal, or on the
    /// surface node the parameter itself; `component` picks a vector's X, Y
    /// or Z.
    Input { node: NodeId, index: usize, component: Option<u8> },
    /// A Value or RGB node's own value, or one of its vector's components.
    Value { node: NodeId, component: Option<u8> },
    Setting { node: NodeId, setting: SettingRef },
}

impl Target {
    pub fn node(self) -> NodeId {
        match self {
            Target::Input { node, .. } | Target::Value { node, .. } | Target::Setting { node, .. } => node,
        }
    }
}

fn component(value: Value, component: Option<u8>) -> Value {
    match (value, component) {
        (Value::Vector(v), Some(c)) => Value::Float([v.x, v.y, v.z][c.min(2) as usize]),
        (Value::Color(s), Some(c)) => Value::Float([s.r, s.g, s.b][c.min(2) as usize]),
        (other, _) => other,
    }
}

fn with_component(old: Value, component: Option<u8>, new: Value) -> Value {
    match (old, component, new) {
        (Value::Vector(mut v), Some(c), Value::Float(f)) => {
            match c {
                0 => v.x = f,
                1 => v.y = f,
                _ => v.z = f,
            }
            Value::Vector(v)
        }
        (_, None, new) => new,
        (old, _, _) => old,
    }
}

/// An input's value as the compiler would use it while nothing is wired.
pub fn input_value(graph: &MaterialGraph, node: NodeId, index: usize) -> Option<Value> {
    let n = graph.node(node)?;
    let (socket, ty, _) = *n.kind.inputs().get(index)?;
    if let (NodeKind::Surface(material), Socket::Param(p)) = (&n.kind, socket) {
        return Some(match p.read(material)? {
            ParamValue::Scalar(v) => Value::Float(v),
            ParamValue::Color(c) => Value::Color(c),
            ParamValue::Flag(b) => Value::Bool(b),
            _ => return None,
        });
    }
    Some(
        n.literals
            .get(&socket)
            .copied()
            .or_else(|| n.kind.input_default(index))
            .unwrap_or_else(|| tequila_materials::graph::default_for(ty)),
    )
}

/// What a field shows.
pub fn read(graph: &MaterialGraph, target: Target) -> Option<Value> {
    match target {
        Target::Input { node, index, component: c } => input_value(graph, node, index).map(|v| component(v, c)),
        Target::Value { node, component: c } => match graph.node(node)?.kind {
            NodeKind::Constant(value) => Some(component(value, c)),
            _ => None,
        },
        Target::Setting { node, setting } => match (&graph.node(node)?.kind, setting) {
            (NodeKind::Node(shader), SettingRef::Shader(i)) => match shader.settings().get(i)?.value {
                Setting::Int(v) => Some(Value::Float(v as f32)),
                Setting::Float(v) => Some(Value::Float(v)),
                Setting::Bool(v) => Some(Value::Bool(v)),
                Setting::Choice(_) => None,
            },
            _ => None,
        },
    }
}

/// Write a field's value back. False if it did not take.
pub fn write(graph: &mut MaterialGraph, target: Target, value: Value) -> bool {
    match target {
        Target::Input { node, index, component: c } => {
            let Some(old) = input_value(graph, node, index) else { return false };
            let new = with_component(old, c, value);
            let Some(n) = graph.node_mut(node) else { return false };
            let Some((socket, ty, _)) = n.kind.inputs().get(index).copied() else { return false };
            if let (NodeKind::Surface(material), Socket::Param(p)) = (&mut n.kind, socket) {
                let value = match (p.ui(), new) {
                    (ParamUi::Scalar, Value::Float(f)) => ParamValue::Scalar(f),
                    (ParamUi::Color, Value::Color(c)) => ParamValue::Color(c),
                    (ParamUi::Color, Value::Float(f)) => ParamValue::Color(Spectrum::splat(f)),
                    (ParamUi::Flag, Value::Bool(b)) => ParamValue::Flag(b),
                    _ => return false,
                };
                return p.write(material, value);
            }
            let Some(converted) = new.convert(ty) else { return false };
            n.literals.insert(socket, converted);
            true
        }
        Target::Value { node, component: c } => match graph.node_mut(node).map(|n| &mut n.kind) {
            Some(NodeKind::Constant(old)) => {
                *old = with_component(*old, c, value);
                true
            }
            _ => false,
        },
        Target::Setting { node, setting } => {
            let Some(n) = graph.node_mut(node) else { return false };
            match (&mut n.kind, setting) {
                (NodeKind::Node(shader), SettingRef::Shader(i)) => {
                    let Some(spec) = shader.settings().get(i).cloned() else { return false };
                    let setting = match (spec.value, value) {
                        (Setting::Int(_), Value::Float(f)) => Setting::Int(f.round() as i32),
                        (Setting::Float(_), Value::Float(f)) => Setting::Float(f),
                        (Setting::Bool(_), Value::Bool(b)) => Setting::Bool(b),
                        _ => return false,
                    };
                    shader.set(spec.key, setting)
                }
                _ => false,
            }
        }
    }
}

/// A dropdown's options, as (key, label), and the one chosen.
pub fn choices(node: &Node, setting: SettingRef) -> Option<(Vec<(&'static str, &'static str)>, &'static str)> {
    match (&node.kind, setting) {
        (NodeKind::Node(shader), SettingRef::Shader(i)) => {
            let spec = shader.settings().get(i)?.clone();
            match spec.value {
                Setting::Choice(current) => Some((spec.options, current)),
                _ => None,
            }
        }
        (NodeKind::Gradient { gradient }, SettingRef::RampInterpolation) => {
            use tequila_texture::gradient::Interpolation;
            Some((Interpolation::ALL.iter().map(|i| (i.key(), i.label())).collect(), gradient.interpolation().key()))
        }
        _ => None,
    }
}

/// Choose a dropdown's option.
pub fn choose(graph: &mut MaterialGraph, node: NodeId, setting: SettingRef, key: &'static str) -> bool {
    let Some(n) = graph.node_mut(node) else { return false };
    match (&mut n.kind, setting) {
        (NodeKind::Node(shader), SettingRef::Shader(i)) => {
            let Some(spec) = shader.settings().get(i).cloned() else { return false };
            let before = *shader;
            let changed = shader.set(spec.key, Setting::Choice(key));
            // A node whose sockets moved (a Mix now mixing vectors, a
            // Voronoi with one output) keeps only the wires that still fit.
            if changed && *shader != before {
                let outputs = shader.outputs().len() as u8;
                let shader = *shader;
                let links: Vec<_> = graph.links().to_vec();
                for link in links {
                    if link.from == node && link.from_socket >= outputs {
                        graph.disconnect(link.to, link.to_socket);
                    }
                    if link.to == node {
                        let fits = match link.to_socket {
                            Socket::Input(i) => shader.inputs().get(i as usize).is_some_and(|spec| {
                                graph.node(link.from).and_then(|n| n.kind.output_type_at(link.from_socket)).is_some_and(|t| t.converts_to(spec.ty))
                            }),
                            _ => false,
                        };
                        if !fits {
                            graph.disconnect(link.to, link.to_socket);
                        }
                    }
                }
            }
            changed
        }
        (NodeKind::Gradient { gradient }, SettingRef::RampInterpolation) => {
            match tequila_texture::gradient::Interpolation::from_key(key) {
                Some(i) => {
                    **gradient = (**gradient).clone().with_interpolation(i);
                    true
                }
                None => false,
            }
        }
        _ => false,
    }
}

/// A setting row's label, and how it is edited.
pub enum SettingField {
    Dropdown { current: &'static str },
    Number { label: &'static str, int: bool, range: Option<(f32, f32)> },
    Toggle { label: &'static str, on: bool },
}

pub fn setting_field(node: &Node, setting: SettingRef) -> Option<SettingField> {
    match (&node.kind, setting) {
        (NodeKind::Node(shader), SettingRef::Shader(i)) => {
            let spec = shader.settings().get(i)?.clone();
            Some(match spec.value {
                Setting::Choice(current) => SettingField::Dropdown {
                    current: spec.options.iter().find(|(k, _)| *k == current).map(|(_, l)| *l).unwrap_or(current),
                },
                Setting::Int(_) => SettingField::Number { label: spec.label, int: true, range: setting_range(shader, spec.key) },
                Setting::Float(_) => SettingField::Number { label: spec.label, int: false, range: setting_range(shader, spec.key) },
                Setting::Bool(on) => SettingField::Toggle { label: spec.label, on },
            })
        }
        (NodeKind::Gradient { gradient }, SettingRef::RampInterpolation) => Some(SettingField::Dropdown { current: gradient.interpolation().label() }),
        _ => None,
    }
}

fn setting_range(shader: &ShaderNode, key: &str) -> Option<(f32, f32)> {
    match (shader, key) {
        (ShaderNode::Magic { .. }, "depth") => Some((0.0, 10.0)),
        (ShaderNode::Brick { .. }, "offset") => Some((0.0, 1.0)),
        (ShaderNode::Brick { .. }, "offset_frequency" | "squash_frequency") => Some((1.0, 10.0)),
        (ShaderNode::Brick { .. }, "squash") => Some((0.0, 4.0)),
        _ => None,
    }
}

/// Which inputs are wired, for the whole graph at once.
pub fn connected(graph: &MaterialGraph) -> HashSet<(NodeId, Socket)> {
    graph.links().iter().map(|l| (l.to, l.to_socket)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tequila_materials::graph::Node;

    /// A Noise node lays out as Blender's: two outputs, a preview, its
    /// settings, then the inputs its settings use - W hidden in three
    /// dimensions - and the sockets sit on the rows they belong to.
    #[test]
    fn a_noise_node_lays_out_like_blenders() {
        let mut graph = MaterialGraph::from_material(Default::default());
        let id = graph.add_node(Node::new(NodeKind::Node(ShaderNode::noise())));
        let node = graph.node(id).unwrap().clone();
        let layout = layout(&node, id, &connected(&graph), &[], true, &|_| String::new());
        let kinds: Vec<_> = layout.rows.iter().map(|r| r.kind.clone()).collect();
        assert_eq!(kinds[0], RowKind::Output(0));
        assert_eq!(kinds[1], RowKind::Output(1));
        assert_eq!(kinds[2], RowKind::Preview);
        assert!(matches!(kinds[3], RowKind::Setting(SettingRef::Shader(0))));
        let inputs: Vec<usize> = layout.inputs.iter().map(|i| i.1).collect();
        assert!(!inputs.contains(&1), "W is hidden in 3D: {inputs:?}");
        assert!(inputs.contains(&0) && inputs.contains(&2));
        for (_, index, _, at) in &layout.inputs {
            let row = layout.rows.iter().find(|r| r.kind == RowKind::Input(*index)).unwrap();
            assert!((at.y - (layout.rect.top() + row.top + row.height * 0.5)).abs() < 1e-3);
        }
    }

    /// A field writes where the compiler reads: a literal on a node, the
    /// parameter on the surface, one component of a vector.
    #[test]
    fn fields_write_where_the_compiler_reads() {
        let mut graph = MaterialGraph::from_material(Default::default());
        let id = graph.add_node(Node::new(NodeKind::Node(ShaderNode::noise())));
        let scale = Target::Input { node: id, index: 2, component: None };
        assert_eq!(read(&graph, scale), Some(Value::Float(25.0)), "the node's own default");
        assert!(write(&mut graph, scale, Value::Float(4.0)));
        assert_eq!(graph.node(id).unwrap().literals.get(&Socket::Input(2)), Some(&Value::Float(4.0)));

        let output = graph.output();
        let index = graph.node(output).unwrap().kind.inputs().iter().position(|i| i.0 == Socket::Param(Param::SpecularRoughness)).unwrap();
        assert!(write(&mut graph, Target::Input { node: output, index, component: None }, Value::Float(0.7)));
        assert!((graph.material().specular_roughness - 0.7).abs() < 1e-6);

        let mapping = graph.add_node(Node::new(NodeKind::Node(ShaderNode::from_key("mapping").unwrap())));
        let y = Target::Input { node: mapping, index: 3, component: Some(1) };
        assert_eq!(read(&graph, y), Some(Value::Float(1.0)));
        assert!(write(&mut graph, y, Value::Float(2.5)));
        assert_eq!(
            graph.node(mapping).unwrap().literals.get(&Socket::Input(3)),
            Some(&Value::Vector(tequila_core::math::vec3(1.0, 2.5, 1.0)))
        );
    }

    /// Changing a setting that removes an output drops the wires that left
    /// from it, rather than leaving a graph that no longer compiles.
    #[test]
    fn a_setting_that_removes_an_output_drops_its_wires() {
        let mut graph = MaterialGraph::from_material(Default::default());
        let cells = graph.add_node(Node::new(NodeKind::Node(ShaderNode::voronoi())));
        let output = graph.output();
        graph.connect_output(cells, 1, output, Socket::Param(Param::BaseColor));
        assert!(choose(&mut graph, cells, SettingRef::Shader(1), "distance_to_edge"));
        assert!(graph.links().iter().all(|l| l.from != cells), "the colour output is gone");
        assert!(graph.compile().is_ok());
    }
}
