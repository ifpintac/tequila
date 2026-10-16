//! Blender's shader nodes, as material graph nodes.
//!
//! Each is a node of Blender's shader editor - the textures, converters,
//! colour and vector nodes and the inputs they read from the surface -
//! evaluated with the Cycles algorithms in `tequila_texture::cycles`, so
//! that Tequila's renderer and Cycles shade a graph of them the same, and
//! translated to Cycles node for node (`tequila_render_cycles`).
//!
//! A node describes itself: its settings (choices, numbers and switches
//! that are not sockets), its inputs in socket order with their defaults,
//! its outputs. The file format, the graph editor and the Cycles
//! translation all read those descriptions rather than knowing each node.

use crate::graph::{SocketType, Value};
use tequila_core::math::{vec3, Vec3};
use tequila_core::spectrum::Spectrum;
use tequila_texture::cycles::noise::{FractalKind, NoiseSettings};
use tequila_texture::cycles::ops::{self, BlendMode, ColorModel, MapRangeKind, MappingKind, MathOp, VectorMathOp};
use tequila_texture::cycles::textures::{self, BrickSettings, GaborKind, GradientKind, WaveDirection, WaveKind, WaveProfile};
use tequila_texture::cycles::voronoi::{VoronoiFeature, VoronoiMetric, VoronoiSettings};

/// What a Mix node mixes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum MixData {
    Float,
    Vector,
    #[default]
    Color,
}

/// A Blender shader node.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShaderNode {
    // ---- Input ---------------------------------------------------------
    TextureCoordinate,
    Geometry,
    LayerWeight,
    Fresnel,
    ObjectInfo,
    // ---- Texture -------------------------------------------------------
    Noise { dims: u8, fractal: FractalKind, normalize: bool },
    Voronoi { dims: u8, feature: VoronoiFeature, metric: VoronoiMetric, normalize: bool },
    Wave { kind: WaveKind, direction: WaveDirection, profile: WaveProfile },
    Magic { depth: i32 },
    Brick { offset: f32, offset_frequency: i32, squash: f32, squash_frequency: i32 },
    Checker,
    GradientTexture { kind: GradientKind },
    WhiteNoise { dims: u8 },
    Gabor { kind: GaborKind },
    // ---- Color ---------------------------------------------------------
    Mix { data: MixData, blend: BlendMode, clamp_factor: bool, clamp_result: bool },
    HueSaturationValue,
    BrightContrast,
    Gamma,
    InvertColor,
    // ---- Converter -----------------------------------------------------
    Math { op: MathOp, clamp: bool },
    VectorMath { op: VectorMathOp },
    MapRange { kind: MapRangeKind, clamp: bool },
    Clamp { range: bool },
    SeparateXyz,
    CombineXyz,
    SeparateColor { model: ColorModel },
    CombineColor { model: ColorModel },
    // ---- Vector --------------------------------------------------------
    Mapping { kind: MappingKind },
}

/// Where a node sits in the editor's Add menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeCategory {
    Input,
    Texture,
    Color,
    Converter,
    Vector,
}

impl NodeCategory {
    pub fn label(self) -> &'static str {
        match self {
            NodeCategory::Input => "Input",
            NodeCategory::Texture => "Texture",
            NodeCategory::Color => "Color",
            NodeCategory::Converter => "Converter",
            NodeCategory::Vector => "Vector",
        }
    }
}

/// Something the shading point knows, that a node reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PointInput {
    /// Object space, in scene units: Tequila's patterns' own space.
    Object,
    /// Object space normalized to the part's bounding box, 0 to 1.
    Generated,
    /// The first UV set, (u, v, 0), v up as Blender has it.
    Uv,
    /// The shading normal, in object space.
    ObjectNormal,
    /// The view reflected about the normal, in world space.
    Reflection,
    /// World space.
    Position,
    /// The shading normal, in world space.
    Normal,
    /// The geometric normal, in world space.
    TrueNormal,
    /// Towards the viewer, in world space.
    Incoming,
    /// 1 seen from behind, 0 from the front.
    Backfacing,
    /// The part's origin, in world space.
    ObjectLocation,
    /// A number from 0 to 1, the same over a part, different between parts.
    ObjectRandom,
}

/// One of a node's inputs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InputSpec {
    pub name: &'static str,
    /// Cycles' internal socket name (lower case: texture nodes' own mapping
    /// also has a "Scale").
    pub cycles: &'static str,
    pub ty: SocketType,
    pub default: Value,
    /// What it reads when nothing is wired to it, if not its default.
    pub implicit: Option<PointInput>,
    /// Never shown: an input the node reads from the surface itself.
    pub hidden: bool,
    /// A sensible range for its slider, if it has one.
    pub range: Option<(f32, f32)>,
}

/// One of a node's outputs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputSpec {
    pub name: &'static str,
    /// Cycles' name for the output, as a link from it is made.
    pub cycles: &'static str,
    pub ty: SocketType,
}

/// A node setting that is not a socket.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Setting {
    /// One of `options`, by its key.
    Choice(&'static str),
    Int(i32),
    Float(f32),
    Bool(bool),
}

/// A node setting as Cycles takes it, by its socket name there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CyclesSetting {
    Enum(&'static str),
    Int(i32),
    Float(f32),
    Bool(bool),
}

/// A node setting, described for the file format and the editor.
#[derive(Clone, Debug, PartialEq)]
pub struct SettingSpec {
    pub key: &'static str,
    pub label: &'static str,
    pub value: Setting,
    /// For a choice: every option, as (key, label).
    pub options: Vec<(&'static str, &'static str)>,
}

fn input(name: &'static str, cycles: &'static str, ty: SocketType, default: Value) -> InputSpec {
    InputSpec { name, cycles, ty, default, implicit: None, hidden: false, range: None }
}

fn float(name: &'static str, cycles: &'static str, default: f32) -> InputSpec {
    input(name, cycles, SocketType::Float, Value::Float(default))
}

fn ranged(name: &'static str, cycles: &'static str, default: f32, lo: f32, hi: f32) -> InputSpec {
    InputSpec { range: Some((lo, hi)), ..float(name, cycles, default) }
}

fn color(name: &'static str, cycles: &'static str, c: f32) -> InputSpec {
    input(name, cycles, SocketType::Color, Value::Color(Spectrum::splat(c)))
}

fn vector(name: &'static str, cycles: &'static str, v: Vec3) -> InputSpec {
    input(name, cycles, SocketType::Vector, Value::Vector(v))
}

/// A texture's coordinates: object space in scene units when nothing is
/// wired, as Tequila's patterns have always been read.
fn coordinates() -> InputSpec {
    InputSpec { implicit: Some(PointInput::Object), ..vector("Vector", "vector", Vec3::ZERO) }
}

fn surface_normal() -> InputSpec {
    InputSpec { implicit: Some(PointInput::Normal), ..input("Normal", "normal", SocketType::Normal, Value::Vector(Vec3::Y)) }
}

fn hidden(point: PointInput, ty: SocketType) -> InputSpec {
    InputSpec { implicit: Some(point), hidden: true, ..input("", "", ty, Value::Float(0.0)) }
}

fn output(name: &'static str, cycles: &'static str, ty: SocketType) -> OutputSpec {
    OutputSpec { name, cycles, ty }
}

fn choice<T: Copy + PartialEq>(key: &'static str, label: &'static str, value: T, all: &[T], name: impl Fn(T) -> (&'static str, &'static str)) -> SettingSpec {
    SettingSpec { key, label, value: Setting::Choice(name(value).0), options: all.iter().map(|v| name(*v)).collect() }
}

fn pick<T: Copy>(all: &[T], name: impl Fn(T) -> (&'static str, &'static str), key: &str) -> Option<T> {
    all.iter().copied().find(|v| name(*v).0 == key)
}

const DIMS: [u8; 4] = [1, 2, 3, 4];
fn dims_name(d: u8) -> (&'static str, &'static str) {
    match d {
        1 => ("1D", "1D"),
        2 => ("2D", "2D"),
        3 => ("3D", "3D"),
        _ => ("4D", "4D"),
    }
}
fn fractal_name(f: FractalKind) -> (&'static str, &'static str) {
    (f.cycles(), f.label())
}
fn feature_name(f: VoronoiFeature) -> (&'static str, &'static str) {
    (f.cycles(), f.label())
}
fn metric_name(m: VoronoiMetric) -> (&'static str, &'static str) {
    (m.cycles(), m.label())
}
const GABOR_KINDS: [GaborKind; 2] = [GaborKind::Plane, GaborKind::Space];
fn gabor_kind_name(k: GaborKind) -> (&'static str, &'static str) {
    (k.cycles(), k.cycles())
}
const WAVE_KINDS: [WaveKind; 2] = [WaveKind::Bands, WaveKind::Rings];
fn wave_kind_name(k: WaveKind) -> (&'static str, &'static str) {
    (k.cycles(), if k == WaveKind::Bands { "Bands" } else { "Rings" })
}
const WAVE_DIRECTIONS: [WaveDirection; 4] = [WaveDirection::X, WaveDirection::Y, WaveDirection::Z, WaveDirection::Diagonal];
fn wave_direction_name(d: WaveDirection) -> (&'static str, &'static str) {
    match d {
        WaveDirection::X => ("x", "X"),
        WaveDirection::Y => ("y", "Y"),
        WaveDirection::Z => ("z", "Z"),
        WaveDirection::Diagonal => ("diagonal", "Diagonal / spherical"),
    }
}
const WAVE_PROFILES: [WaveProfile; 3] = [WaveProfile::Sine, WaveProfile::Saw, WaveProfile::Triangle];
fn wave_profile_name(p: WaveProfile) -> (&'static str, &'static str) {
    (p.cycles(), match p {
        WaveProfile::Sine => "Sine",
        WaveProfile::Saw => "Saw",
        WaveProfile::Triangle => "Triangle",
    })
}
fn gradient_name(k: GradientKind) -> (&'static str, &'static str) {
    (k.cycles(), k.label())
}
const MIX_DATA: [MixData; 3] = [MixData::Float, MixData::Vector, MixData::Color];
fn mix_data_name(d: MixData) -> (&'static str, &'static str) {
    match d {
        MixData::Float => ("float", "Float"),
        MixData::Vector => ("vector", "Vector"),
        MixData::Color => ("color", "Color"),
    }
}
fn blend_name(b: BlendMode) -> (&'static str, &'static str) {
    (b.cycles(), b.label())
}
fn math_name(m: MathOp) -> (&'static str, &'static str) {
    (m.cycles(), m.label())
}
fn vector_math_name(m: VectorMathOp) -> (&'static str, &'static str) {
    (m.cycles(), m.label())
}
fn map_range_name(k: MapRangeKind) -> (&'static str, &'static str) {
    (k.cycles(), k.label())
}
fn model_name(m: ColorModel) -> (&'static str, &'static str) {
    (m.cycles(), m.label())
}
fn mapping_name(k: MappingKind) -> (&'static str, &'static str) {
    (k.cycles(), k.label())
}

impl ShaderNode {
    /// One of each, at its defaults, in Add-menu order.
    pub fn catalog() -> Vec<ShaderNode> {
        vec![
            ShaderNode::TextureCoordinate,
            ShaderNode::Geometry,
            ShaderNode::LayerWeight,
            ShaderNode::Fresnel,
            ShaderNode::ObjectInfo,
            ShaderNode::noise(),
            ShaderNode::voronoi(),
            ShaderNode::Wave { kind: WaveKind::Bands, direction: WaveDirection::X, profile: WaveProfile::Sine },
            ShaderNode::Magic { depth: 2 },
            ShaderNode::Brick { offset: 0.5, offset_frequency: 2, squash: 1.0, squash_frequency: 2 },
            ShaderNode::Checker,
            ShaderNode::GradientTexture { kind: GradientKind::Linear },
            ShaderNode::WhiteNoise { dims: 3 },
            ShaderNode::Gabor { kind: GaborKind::Plane },
            ShaderNode::Mix { data: MixData::Color, blend: BlendMode::Mix, clamp_factor: true, clamp_result: false },
            ShaderNode::HueSaturationValue,
            ShaderNode::BrightContrast,
            ShaderNode::Gamma,
            ShaderNode::InvertColor,
            ShaderNode::Math { op: MathOp::Add, clamp: false },
            ShaderNode::VectorMath { op: VectorMathOp::Add },
            ShaderNode::MapRange { kind: MapRangeKind::Linear, clamp: true },
            ShaderNode::Clamp { range: false },
            ShaderNode::SeparateXyz,
            ShaderNode::CombineXyz,
            ShaderNode::SeparateColor { model: ColorModel::Rgb },
            ShaderNode::CombineColor { model: ColorModel::Rgb },
            ShaderNode::Mapping { kind: MappingKind::Point },
        ]
    }

    pub fn noise() -> ShaderNode {
        ShaderNode::Noise { dims: 3, fractal: FractalKind::Fbm, normalize: true }
    }

    pub fn voronoi() -> ShaderNode {
        ShaderNode::Voronoi { dims: 3, feature: VoronoiFeature::F1, metric: VoronoiMetric::Euclidean, normalize: false }
    }

    /// Its stable name, in files and scripts.
    pub fn key(&self) -> &'static str {
        match self {
            ShaderNode::TextureCoordinate => "texture_coordinate",
            ShaderNode::Geometry => "geometry",
            ShaderNode::LayerWeight => "layer_weight",
            ShaderNode::Fresnel => "fresnel",
            ShaderNode::ObjectInfo => "object_info",
            ShaderNode::Noise { .. } => "noise_texture",
            ShaderNode::Voronoi { .. } => "voronoi_texture",
            ShaderNode::Wave { .. } => "wave_texture",
            ShaderNode::Magic { .. } => "magic_texture",
            ShaderNode::Brick { .. } => "brick_texture",
            ShaderNode::Checker => "checker_texture",
            ShaderNode::GradientTexture { .. } => "gradient_texture",
            ShaderNode::WhiteNoise { .. } => "white_noise_texture",
            ShaderNode::Gabor { .. } => "gabor_texture",
            ShaderNode::Mix { .. } => "mix",
            ShaderNode::HueSaturationValue => "hue_saturation",
            ShaderNode::BrightContrast => "brightness_contrast",
            ShaderNode::Gamma => "gamma",
            ShaderNode::InvertColor => "invert",
            ShaderNode::Math { .. } => "math",
            ShaderNode::VectorMath { .. } => "vector_math",
            ShaderNode::MapRange { .. } => "map_range",
            ShaderNode::Clamp { .. } => "clamp",
            ShaderNode::SeparateXyz => "separate_xyz",
            ShaderNode::CombineXyz => "combine_xyz",
            ShaderNode::SeparateColor { .. } => "separate_color",
            ShaderNode::CombineColor { .. } => "combine_color",
            ShaderNode::Mapping { .. } => "mapping",
        }
    }

    /// Cycles' name for the node type.
    pub fn cycles_type(&self) -> &'static str {
        match self {
            ShaderNode::Mix { data: MixData::Float, .. } => "mix_float",
            ShaderNode::Mix { data: MixData::Vector, .. } => "mix_vector",
            ShaderNode::Mix { data: MixData::Color, .. } => "mix_color",
            ShaderNode::HueSaturationValue => "hsv",
            // Every other key is Cycles' own name.
            other => other.key(),
        }
    }

    /// Its settings as Cycles takes them.
    pub fn cycles_settings(&self) -> Vec<(&'static str, CyclesSetting)> {
        use CyclesSetting::*;
        match *self {
            ShaderNode::Noise { dims, fractal, normalize } => vec![
                ("dimensions", Enum(dims_name(dims).0)),
                ("type", Enum(fractal.cycles())),
                ("use_normalize", Bool(normalize)),
            ],
            ShaderNode::Voronoi { dims, feature, metric, normalize } => vec![
                ("dimensions", Enum(dims_name(dims).0)),
                ("feature", Enum(feature.cycles())),
                ("metric", Enum(metric.cycles())),
                ("use_normalize", Bool(normalize)),
            ],
            ShaderNode::Wave { kind, direction, profile } => vec![
                ("wave_type", Enum(kind.cycles())),
                match kind {
                    WaveKind::Bands => ("bands_direction", Enum(direction.cycles_bands())),
                    WaveKind::Rings => ("rings_direction", Enum(direction.cycles_rings())),
                },
                ("profile", Enum(profile.cycles())),
            ],
            ShaderNode::Magic { depth } => vec![("depth", Int(depth))],
            ShaderNode::Brick { offset, offset_frequency, squash, squash_frequency } => vec![
                ("offset", Float(offset)),
                ("offset_frequency", Int(offset_frequency)),
                ("squash", Float(squash)),
                ("squash_frequency", Int(squash_frequency)),
            ],
            ShaderNode::GradientTexture { kind } => vec![("gradient_type", Enum(kind.cycles()))],
            ShaderNode::WhiteNoise { dims } => vec![("dimensions", Enum(dims_name(dims).0))],
            ShaderNode::Gabor { kind } => vec![("type", Enum(kind.cycles()))],
            ShaderNode::Mix { data, blend, clamp_factor, clamp_result } => {
                let mut out = vec![("use_clamp", Bool(clamp_factor))];
                if data == MixData::Color {
                    out.push(("blend_type", Enum(blend.cycles())));
                    out.push(("use_clamp_result", Bool(clamp_result)));
                }
                out
            }
            ShaderNode::Math { op, clamp } => vec![("math_type", Enum(op.cycles())), ("use_clamp", Bool(clamp))],
            ShaderNode::VectorMath { op } => vec![("math_type", Enum(op.cycles()))],
            ShaderNode::MapRange { kind, clamp } => vec![("range_type", Enum(kind.cycles())), ("clamp", Bool(clamp))],
            ShaderNode::Clamp { range } => vec![("clamp_type", Enum(if range { "range" } else { "minmax" }))],
            ShaderNode::SeparateColor { model } | ShaderNode::CombineColor { model } => vec![("color_type", Enum(model.cycles()))],
            ShaderNode::Mapping { kind } => vec![("mapping_type", Enum(kind.cycles()))],
            _ => Vec::new(),
        }
    }

    /// The node with this key, at its defaults.
    pub fn from_key(key: &str) -> Option<ShaderNode> {
        ShaderNode::catalog().into_iter().find(|n| n.key() == key)
    }

    pub fn label(&self) -> &'static str {
        match self {
            ShaderNode::TextureCoordinate => "Texture Coordinate",
            ShaderNode::Geometry => "Geometry",
            ShaderNode::LayerWeight => "Layer Weight",
            ShaderNode::Fresnel => "Fresnel",
            ShaderNode::ObjectInfo => "Object Info",
            ShaderNode::Noise { .. } => "Noise Texture",
            ShaderNode::Voronoi { .. } => "Voronoi Texture",
            ShaderNode::Wave { .. } => "Wave Texture",
            ShaderNode::Magic { .. } => "Magic Texture",
            ShaderNode::Brick { .. } => "Brick Texture",
            ShaderNode::Checker => "Checker Texture",
            ShaderNode::GradientTexture { .. } => "Gradient Texture",
            ShaderNode::WhiteNoise { .. } => "White Noise Texture",
            ShaderNode::Gabor { .. } => "Gabor Texture",
            ShaderNode::Mix { .. } => "Mix",
            ShaderNode::HueSaturationValue => "Hue / Saturation / Value",
            ShaderNode::BrightContrast => "Brightness / Contrast",
            ShaderNode::Gamma => "Gamma",
            ShaderNode::InvertColor => "Invert Color",
            ShaderNode::Math { op, .. } => op.label(),
            ShaderNode::VectorMath { op } => op.label(),
            ShaderNode::MapRange { .. } => "Map Range",
            ShaderNode::Clamp { .. } => "Clamp",
            ShaderNode::SeparateXyz => "Separate XYZ",
            ShaderNode::CombineXyz => "Combine XYZ",
            ShaderNode::SeparateColor { .. } => "Separate Color",
            ShaderNode::CombineColor { .. } => "Combine Color",
            ShaderNode::Mapping { .. } => "Mapping",
        }
    }

    /// The node's own name in the Add menu, whatever its settings.
    pub fn menu_label(&self) -> &'static str {
        match self {
            ShaderNode::Math { .. } => "Math",
            ShaderNode::VectorMath { .. } => "Vector Math",
            other => other.label(),
        }
    }

    pub fn category(&self) -> NodeCategory {
        match self {
            ShaderNode::TextureCoordinate | ShaderNode::Geometry | ShaderNode::LayerWeight | ShaderNode::Fresnel | ShaderNode::ObjectInfo => {
                NodeCategory::Input
            }
            ShaderNode::Noise { .. }
            | ShaderNode::Voronoi { .. }
            | ShaderNode::Wave { .. }
            | ShaderNode::Magic { .. }
            | ShaderNode::Brick { .. }
            | ShaderNode::Checker
            | ShaderNode::GradientTexture { .. }
            | ShaderNode::WhiteNoise { .. }
            | ShaderNode::Gabor { .. } => NodeCategory::Texture,
            ShaderNode::Mix { .. } | ShaderNode::HueSaturationValue | ShaderNode::BrightContrast | ShaderNode::Gamma | ShaderNode::InvertColor => {
                NodeCategory::Color
            }
            ShaderNode::Mapping { .. } => NodeCategory::Vector,
            _ => NodeCategory::Converter,
        }
    }

    /// Words the Add menu's search also matches.
    pub fn keywords(&self) -> &'static str {
        match self {
            ShaderNode::Noise { .. } => "perlin fbm fractal musgrave clouds turbulence",
            ShaderNode::Voronoi { .. } => "cells cellular worley crackle stone tiles",
            ShaderNode::Wave { .. } => "rings bands stripes wood",
            ShaderNode::Brick { .. } => "tiles bricks wall",
            ShaderNode::Gabor { .. } => "brushed streaks anisotropic grain fibres scratches",
            ShaderNode::Mix { .. } => "blend lerp overlay multiply screen",
            ShaderNode::Math { .. } => "add multiply power sine",
            ShaderNode::MapRange { .. } => "remap fit range",
            ShaderNode::LayerWeight => "facing fresnel edge rim",
            ShaderNode::ObjectInfo => "random variation per part",
            ShaderNode::TextureCoordinate => "uv object generated coordinates",
            _ => "",
        }
    }

    /// Its settings that are not sockets.
    pub fn settings(&self) -> Vec<SettingSpec> {
        match *self {
            ShaderNode::Noise { dims, fractal, normalize } => vec![
                choice("dimensions", "Dimensions", dims, &DIMS, dims_name),
                choice("type", "Type", fractal, &FractalKind::ALL, fractal_name),
                SettingSpec { key: "normalize", label: "Normalize", value: Setting::Bool(normalize), options: vec![] },
            ],
            ShaderNode::Voronoi { dims, feature, metric, normalize } => vec![
                choice("dimensions", "Dimensions", dims, &DIMS, dims_name),
                choice("feature", "Feature", feature, &VoronoiFeature::ALL, feature_name),
                choice("metric", "Distance", metric, &VoronoiMetric::ALL, metric_name),
                SettingSpec { key: "normalize", label: "Normalize", value: Setting::Bool(normalize), options: vec![] },
            ],
            ShaderNode::Wave { kind, direction, profile } => vec![
                choice("type", "Type", kind, &WAVE_KINDS, wave_kind_name),
                choice("direction", "Direction", direction, &WAVE_DIRECTIONS, wave_direction_name),
                choice("profile", "Profile", profile, &WAVE_PROFILES, wave_profile_name),
            ],
            ShaderNode::Magic { depth } => vec![SettingSpec { key: "depth", label: "Depth", value: Setting::Int(depth), options: vec![] }],
            ShaderNode::Brick { offset, offset_frequency, squash, squash_frequency } => vec![
                SettingSpec { key: "offset", label: "Offset", value: Setting::Float(offset), options: vec![] },
                SettingSpec { key: "offset_frequency", label: "Offset frequency", value: Setting::Int(offset_frequency), options: vec![] },
                SettingSpec { key: "squash", label: "Squash", value: Setting::Float(squash), options: vec![] },
                SettingSpec { key: "squash_frequency", label: "Squash frequency", value: Setting::Int(squash_frequency), options: vec![] },
            ],
            ShaderNode::GradientTexture { kind } => vec![choice("type", "Type", kind, &GradientKind::ALL, gradient_name)],
            ShaderNode::WhiteNoise { dims } => vec![choice("dimensions", "Dimensions", dims, &DIMS, dims_name)],
            ShaderNode::Gabor { kind } => vec![choice("type", "Type", kind, &GABOR_KINDS, gabor_kind_name)],
            ShaderNode::Mix { data, blend, clamp_factor, clamp_result } => {
                let mut out = vec![choice("data_type", "Data", data, &MIX_DATA, mix_data_name)];
                if data == MixData::Color {
                    out.push(choice("blend_type", "Blend", blend, BlendMode::ALL, blend_name));
                    out.push(SettingSpec { key: "clamp_result", label: "Clamp result", value: Setting::Bool(clamp_result), options: vec![] });
                }
                out.push(SettingSpec { key: "clamp_factor", label: "Clamp factor", value: Setting::Bool(clamp_factor), options: vec![] });
                out
            }
            ShaderNode::Math { op, clamp } => vec![
                choice("operation", "Operation", op, MathOp::ALL, math_name),
                SettingSpec { key: "clamp", label: "Clamp", value: Setting::Bool(clamp), options: vec![] },
            ],
            ShaderNode::VectorMath { op } => vec![choice("operation", "Operation", op, VectorMathOp::ALL, vector_math_name)],
            ShaderNode::MapRange { kind, clamp } => vec![
                choice("interpolation", "Interpolation", kind, &MapRangeKind::ALL, map_range_name),
                SettingSpec { key: "clamp", label: "Clamp", value: Setting::Bool(clamp), options: vec![] },
            ],
            ShaderNode::Clamp { range } => vec![SettingSpec { key: "range", label: "Min / max either way", value: Setting::Bool(range), options: vec![] }],
            ShaderNode::SeparateColor { model } | ShaderNode::CombineColor { model } => {
                vec![choice("mode", "Mode", model, &ColorModel::ALL, model_name)]
            }
            ShaderNode::Mapping { kind } => vec![choice("type", "Type", kind, &MappingKind::ALL, mapping_name)],
            _ => Vec::new(),
        }
    }

    /// Change one setting. False if the node has no such setting, or the
    /// value does not fit it.
    pub fn set(&mut self, key: &str, value: Setting) -> bool {
        use Setting::*;
        match (self, key, value) {
            (ShaderNode::Noise { dims, .. } | ShaderNode::Voronoi { dims, .. } | ShaderNode::WhiteNoise { dims }, "dimensions", Choice(v)) => {
                pick(&DIMS, dims_name, v).map(|d| *dims = d).is_some()
            }
            (ShaderNode::Noise { fractal, .. }, "type", Choice(v)) => pick(&FractalKind::ALL, fractal_name, v).map(|f| *fractal = f).is_some(),
            (ShaderNode::Noise { normalize, .. } | ShaderNode::Voronoi { normalize, .. }, "normalize", Bool(b)) => {
                *normalize = b;
                true
            }
            (ShaderNode::Voronoi { feature, .. }, "feature", Choice(v)) => pick(&VoronoiFeature::ALL, feature_name, v).map(|f| *feature = f).is_some(),
            (ShaderNode::Voronoi { metric, .. }, "metric", Choice(v)) => pick(&VoronoiMetric::ALL, metric_name, v).map(|m| *metric = m).is_some(),
            (ShaderNode::Wave { kind, .. }, "type", Choice(v)) => pick(&WAVE_KINDS, wave_kind_name, v).map(|k| *kind = k).is_some(),
            (ShaderNode::Wave { direction, .. }, "direction", Choice(v)) => pick(&WAVE_DIRECTIONS, wave_direction_name, v).map(|d| *direction = d).is_some(),
            (ShaderNode::Wave { profile, .. }, "profile", Choice(v)) => pick(&WAVE_PROFILES, wave_profile_name, v).map(|p| *profile = p).is_some(),
            (ShaderNode::Gabor { kind }, "type", Choice(v)) => pick(&GABOR_KINDS, gabor_kind_name, v).map(|k| *kind = k).is_some(),
            (ShaderNode::Magic { depth }, "depth", Int(d)) => {
                *depth = d.clamp(0, 10);
                true
            }
            (ShaderNode::Brick { offset, .. }, "offset", Float(f)) => {
                *offset = f.clamp(0.0, 1.0);
                true
            }
            (ShaderNode::Brick { offset_frequency, .. }, "offset_frequency", Int(i)) => {
                *offset_frequency = i.clamp(1, 99);
                true
            }
            (ShaderNode::Brick { squash, .. }, "squash", Float(f)) => {
                *squash = f.clamp(0.0, 99.0);
                true
            }
            (ShaderNode::Brick { squash_frequency, .. }, "squash_frequency", Int(i)) => {
                *squash_frequency = i.clamp(1, 99);
                true
            }
            (ShaderNode::GradientTexture { kind }, "type", Choice(v)) => pick(&GradientKind::ALL, gradient_name, v).map(|k| *kind = k).is_some(),
            (ShaderNode::Mix { data, .. }, "data_type", Choice(v)) => pick(&MIX_DATA, mix_data_name, v).map(|d| *data = d).is_some(),
            (ShaderNode::Mix { blend, .. }, "blend_type", Choice(v)) => pick(BlendMode::ALL, blend_name, v).map(|b| *blend = b).is_some(),
            (ShaderNode::Mix { clamp_factor, .. }, "clamp_factor", Bool(b)) => {
                *clamp_factor = b;
                true
            }
            (ShaderNode::Mix { clamp_result, .. }, "clamp_result", Bool(b)) => {
                *clamp_result = b;
                true
            }
            (ShaderNode::Math { op, .. }, "operation", Choice(v)) => pick(MathOp::ALL, math_name, v).map(|o| *op = o).is_some(),
            (ShaderNode::Math { clamp, .. } | ShaderNode::MapRange { clamp, .. }, "clamp", Bool(b)) => {
                *clamp = b;
                true
            }
            (ShaderNode::VectorMath { op }, "operation", Choice(v)) => pick(VectorMathOp::ALL, vector_math_name, v).map(|o| *op = o).is_some(),
            (ShaderNode::MapRange { kind, .. }, "interpolation", Choice(v)) => pick(&MapRangeKind::ALL, map_range_name, v).map(|k| *kind = k).is_some(),
            (ShaderNode::Clamp { range }, "range", Bool(b)) => {
                *range = b;
                true
            }
            (ShaderNode::SeparateColor { model } | ShaderNode::CombineColor { model }, "mode", Choice(v)) => {
                pick(&ColorModel::ALL, model_name, v).map(|m| *model = m).is_some()
            }
            (ShaderNode::Mapping { kind }, "type", Choice(v)) => pick(&MappingKind::ALL, mapping_name, v).map(|k| *kind = k).is_some(),
            _ => false,
        }
    }

    /// Its inputs, in socket order. Every one is listed whatever the
    /// settings, so a socket's index never moves; [`ShaderNode::shows`]
    /// says which the editor draws.
    pub fn inputs(&self) -> Vec<InputSpec> {
        use SocketType::*;
        match *self {
            ShaderNode::TextureCoordinate | ShaderNode::Geometry | ShaderNode::ObjectInfo => Vec::new(),
            ShaderNode::LayerWeight => vec![
                ranged("Blend", "blend", 0.5, 0.0, 1.0),
                surface_normal(),
                hidden(PointInput::Incoming, Vector),
                hidden(PointInput::Backfacing, Float),
            ],
            ShaderNode::Fresnel => vec![
                ranged("IOR", "IOR", 1.5, 1.0, 3.0),
                surface_normal(),
                hidden(PointInput::Incoming, Vector),
                hidden(PointInput::Backfacing, Float),
            ],
            ShaderNode::Noise { .. } => vec![
                coordinates(),
                float("W", "w", 0.0),
                ranged("Scale", "scale", 25.0, 0.0, 200.0),
                ranged("Detail", "detail", 2.0, 0.0, 15.0),
                ranged("Roughness", "roughness", 0.5, 0.0, 1.0),
                ranged("Lacunarity", "lacunarity", 2.0, 0.0, 8.0),
                ranged("Offset", "offset", 0.0, -2.0, 2.0),
                ranged("Gain", "gain", 1.0, 0.0, 4.0),
                ranged("Distortion", "distortion", 0.0, 0.0, 10.0),
            ],
            ShaderNode::Voronoi { .. } => vec![
                coordinates(),
                float("W", "w", 0.0),
                ranged("Scale", "scale", 25.0, 0.0, 200.0),
                ranged("Detail", "detail", 0.0, 0.0, 15.0),
                ranged("Roughness", "roughness", 0.5, 0.0, 1.0),
                ranged("Lacunarity", "lacunarity", 2.0, 0.0, 8.0),
                ranged("Smoothness", "smoothness", 1.0, 0.0, 1.0),
                ranged("Exponent", "exponent", 0.5, 0.0, 32.0),
                ranged("Randomness", "randomness", 1.0, 0.0, 1.0),
            ],
            ShaderNode::Wave { .. } => vec![
                coordinates(),
                ranged("Scale", "scale", 15.0, 0.0, 200.0),
                ranged("Distortion", "distortion", 0.0, -10.0, 10.0),
                ranged("Detail", "detail", 2.0, 0.0, 15.0),
                ranged("Detail Scale", "detail_scale", 1.0, 0.0, 10.0),
                ranged("Detail Roughness", "detail_roughness", 0.5, 0.0, 1.0),
                ranged("Phase Offset", "phase", 0.0, -10.0, 10.0),
            ],
            ShaderNode::Magic { .. } => vec![coordinates(), ranged("Scale", "scale", 15.0, 0.0, 200.0), ranged("Distortion", "distortion", 1.0, 0.0, 10.0)],
            ShaderNode::Brick { .. } => vec![
                coordinates(),
                color("Color1", "color1", 0.8),
                color("Color2", "color2", 0.2),
                color("Mortar", "mortar", 0.0),
                ranged("Scale", "scale", 10.0, 0.0, 200.0),
                ranged("Mortar Size", "mortar_size", 0.02, 0.0, 0.125),
                ranged("Mortar Smooth", "mortar_smooth", 0.1, 0.0, 1.0),
                ranged("Bias", "bias", 0.0, -1.0, 1.0),
                ranged("Brick Width", "brick_width", 0.5, 0.01, 100.0),
                ranged("Row Height", "row_height", 0.25, 0.01, 100.0),
            ],
            ShaderNode::Checker => vec![coordinates(), color("Color1", "color1", 0.8), color("Color2", "color2", 0.2), ranged("Scale", "scale", 10.0, 0.0, 200.0)],
            ShaderNode::GradientTexture { .. } => vec![coordinates()],
            ShaderNode::WhiteNoise { .. } => vec![coordinates(), float("W", "w", 0.0)],
            // Cycles' defaults: streaks two to a cell, five cells a unit,
            // fully anisotropic, at 45 degrees.
            ShaderNode::Gabor { .. } => vec![
                coordinates(),
                ranged("Scale", "scale", 5.0, 0.0, 100.0),
                ranged("Frequency", "frequency", 2.0, 0.0, 20.0),
                ranged("Anisotropy", "anisotropy", 1.0, 0.0, 1.0),
                ranged("Orientation", "orientation_2d", std::f32::consts::FRAC_PI_4, -std::f32::consts::PI, std::f32::consts::PI),
                vector("Orientation", "orientation_3d", vec3(std::f32::consts::SQRT_2, std::f32::consts::SQRT_2, 0.0)),
            ],
            ShaderNode::Mix { data, .. } => {
                let (a, b) = match data {
                    MixData::Float => (float("A", "a", 0.0), float("B", "b", 0.0)),
                    MixData::Vector => (vector("A", "a", Vec3::ZERO), vector("B", "b", Vec3::ZERO)),
                    MixData::Color => (color("A", "a", 0.5), color("B", "b", 0.5)),
                };
                vec![ranged("Factor", "fac", 0.5, 0.0, 1.0), a, b]
            }
            ShaderNode::HueSaturationValue => vec![
                ranged("Hue", "hue", 0.5, 0.0, 1.0),
                ranged("Saturation", "saturation", 1.0, 0.0, 2.0),
                ranged("Value", "value", 1.0, 0.0, 2.0),
                ranged("Fac", "fac", 1.0, 0.0, 1.0),
                color("Color", "color", 0.8),
            ],
            ShaderNode::BrightContrast => vec![color("Color", "color", 0.8), ranged("Bright", "bright", 0.0, -1.0, 1.0), ranged("Contrast", "contrast", 0.0, -1.0, 1.0)],
            ShaderNode::Gamma => vec![color("Color", "color", 1.0), ranged("Gamma", "gamma", 1.0, 0.001, 10.0)],
            ShaderNode::InvertColor => vec![ranged("Fac", "fac", 1.0, 0.0, 1.0), color("Color", "color", 0.0)],
            ShaderNode::Math { .. } => vec![float("Value", "value1", 0.5), float("Value", "value2", 0.5), float("Value", "value3", 0.0)],
            ShaderNode::VectorMath { .. } => vec![
                vector("Vector", "vector1", Vec3::ZERO),
                vector("Vector", "vector2", Vec3::ZERO),
                vector("Vector", "vector3", Vec3::ZERO),
                float("Scale", "scale", 1.0),
            ],
            ShaderNode::MapRange { .. } => vec![
                float("Value", "value", 1.0),
                float("From Min", "from_min", 0.0),
                float("From Max", "from_max", 1.0),
                float("To Min", "to_min", 0.0),
                float("To Max", "to_max", 1.0),
                float("Steps", "steps", 4.0),
            ],
            ShaderNode::Clamp { .. } => vec![float("Value", "value", 1.0), float("Min", "min", 0.0), float("Max", "max", 1.0)],
            ShaderNode::SeparateXyz => vec![vector("Vector", "vector", Vec3::ZERO)],
            ShaderNode::CombineXyz => vec![float("X", "x", 0.0), float("Y", "y", 0.0), float("Z", "z", 0.0)],
            ShaderNode::SeparateColor { .. } => vec![color("Color", "color", 0.8)],
            ShaderNode::CombineColor { model } => {
                let [a, b, c] = model.channel_names();
                vec![ranged(a, "r", 0.0, 0.0, 1.0), ranged(b, "g", 0.0, 0.0, 1.0), ranged(c, "b", 0.0, 0.0, 1.0)]
            }
            ShaderNode::Mapping { .. } => vec![
                InputSpec { implicit: Some(PointInput::Object), ..vector("Vector", "vector", Vec3::ZERO) },
                vector("Location", "location", Vec3::ZERO),
                vector("Rotation", "rotation", Vec3::ZERO),
                vector("Scale", "scale", Vec3::ONE),
            ],
        }
    }

    /// Whether the editor shows input `index` with these settings: W only
    /// in one and four dimensions, a Math operation's unused values hidden.
    pub fn shows(&self, index: usize) -> bool {
        let Some(spec) = self.inputs().get(index).copied() else {
            return false;
        };
        if spec.hidden {
            return false;
        }
        match *self {
            ShaderNode::Noise { dims, .. } | ShaderNode::Voronoi { dims, .. } | ShaderNode::WhiteNoise { dims } => match index {
                0 => dims != 1,
                1 => dims == 1 || dims == 4,
                _ => match self {
                    ShaderNode::Noise { fractal, .. } => match spec.cycles {
                        "offset" => !matches!(fractal, FractalKind::Fbm | FractalKind::Multifractal),
                        "gain" => matches!(fractal, FractalKind::HybridMultifractal | FractalKind::RidgedMultifractal),
                        _ => true,
                    },
                    ShaderNode::Voronoi { feature, metric, .. } => match spec.cycles {
                        "smoothness" => *feature == VoronoiFeature::SmoothF1,
                        "exponent" => *metric == VoronoiMetric::Minkowski,
                        "detail" | "roughness" | "lacunarity" => *feature != VoronoiFeature::NSphereRadius,
                        _ => true,
                    },
                    _ => true,
                },
            },
            // An angle in a plane, a direction in space.
            ShaderNode::Gabor { kind } => match index {
                4 => kind == GaborKind::Plane,
                5 => kind == GaborKind::Space,
                _ => true,
            },
            ShaderNode::Math { op, .. } => index < op.inputs() as usize,
            ShaderNode::VectorMath { op } => match index {
                3 => matches!(op, VectorMathOp::Scale | VectorMathOp::Refract),
                _ => index < op.inputs() as usize,
            },
            ShaderNode::MapRange { kind, .. } => index != 5 || kind == MapRangeKind::Stepped,
            _ => true,
        }
    }

    /// Its outputs.
    pub fn outputs(&self) -> Vec<OutputSpec> {
        use SocketType::*;
        match *self {
            ShaderNode::TextureCoordinate => vec![
                output("Generated", "Generated", Vector),
                output("Normal", "Normal", Normal),
                output("UV", "UV", Vector),
                output("Object", "Object", Vector),
                output("Reflection", "Reflection", Normal),
            ],
            ShaderNode::Geometry => vec![
                output("Position", "Position", Vector),
                output("Normal", "Normal", Normal),
                output("True Normal", "True Normal", Normal),
                output("Incoming", "Incoming", Vector),
                output("Backfacing", "Backfacing", Float),
            ],
            ShaderNode::LayerWeight => vec![output("Fresnel", "Fresnel", Float), output("Facing", "Facing", Float)],
            ShaderNode::Fresnel => vec![output("Fac", "Fac", Float)],
            ShaderNode::ObjectInfo => vec![output("Location", "Location", Vector), output("Random", "Random", Float)],
            ShaderNode::Noise { .. } => vec![output("Fac", "Fac", Float), output("Color", "Color", Color)],
            ShaderNode::Voronoi { feature, .. } => match feature {
                VoronoiFeature::DistanceToEdge => vec![output("Distance", "Distance", Float)],
                VoronoiFeature::NSphereRadius => vec![output("Radius", "Radius", Float)],
                _ => vec![
                    output("Distance", "Distance", Float),
                    output("Color", "Color", Color),
                    output("Position", "Position", Vector),
                    output("W", "W", Float),
                ],
            },
            ShaderNode::Wave { .. } | ShaderNode::Magic { .. } | ShaderNode::Brick { .. } | ShaderNode::Checker | ShaderNode::GradientTexture { .. } => {
                vec![output("Color", "Color", Color), output("Fac", "Fac", Float)]
            }
            ShaderNode::WhiteNoise { .. } => vec![output("Value", "Value", Float), output("Color", "Color", Color)],
            ShaderNode::Gabor { .. } => vec![output("Value", "Value", Float), output("Phase", "Phase", Float), output("Intensity", "Intensity", Float)],
            ShaderNode::Mix { data, .. } => vec![output(
                "Result",
                "Result",
                match data {
                    MixData::Float => Float,
                    MixData::Vector => Vector,
                    MixData::Color => Color,
                },
            )],
            ShaderNode::HueSaturationValue | ShaderNode::BrightContrast | ShaderNode::Gamma | ShaderNode::InvertColor | ShaderNode::CombineColor { .. } => {
                vec![output("Color", "Color", Color)]
            }
            ShaderNode::Math { .. } => vec![output("Value", "Value", Float)],
            ShaderNode::VectorMath { op } => {
                if op.gives_value() {
                    vec![output("Value", "Value", Float)]
                } else {
                    vec![output("Vector", "Vector", Vector)]
                }
            }
            ShaderNode::MapRange { .. } | ShaderNode::Clamp { .. } => vec![output("Result", "Result", Float)],
            ShaderNode::SeparateXyz => vec![output("X", "X", Float), output("Y", "Y", Float), output("Z", "Z", Float)],
            ShaderNode::CombineXyz => vec![output("Vector", "Vector", Vector)],
            ShaderNode::SeparateColor { model } => {
                let [a, b, c] = model.channel_names();
                vec![output(a, "Red", Float), output(b, "Green", Float), output(c, "Blue", Float)]
            }
            ShaderNode::Mapping { .. } => vec![output("Vector", "Vector", Vector)],
        }
    }

    /// For a node that only reads the surface: what output `index` reads.
    pub fn point_output(&self, index: u8) -> Option<PointInput> {
        use PointInput::*;
        let list: &[PointInput] = match self {
            ShaderNode::TextureCoordinate => &[Generated, ObjectNormal, Uv, Object, Reflection],
            ShaderNode::Geometry => &[Position, Normal, TrueNormal, Incoming, Backfacing],
            ShaderNode::ObjectInfo => &[ObjectLocation, ObjectRandom],
            _ => return None,
        };
        list.get(index as usize).copied()
    }

    /// Whether its outputs depend on the shading point however its inputs
    /// are wired. Textures depend on it through their coordinates, which
    /// read the point when nothing is wired to them.
    pub fn is_varying(&self) -> bool {
        self.point_output(0).is_some()
    }

    /// Output `index` from evaluated inputs, in socket order.
    pub fn evaluate(&self, inputs: &[Value], index: u8) -> Value {
        let get = |i: usize| inputs.get(i).copied().unwrap_or(Value::Float(0.0));
        let f = |i: usize| get(i).as_float();
        let v = |i: usize| {
            let p = get(i).as_vector();
            [p.x, p.y, p.z]
        };
        let c = |i: usize| {
            let s = get(i).as_color();
            [s.r, s.g, s.b]
        };
        let colour = |rgb: [f32; 3]| Value::Color(Spectrum { r: rgb[0], g: rgb[1], b: rgb[2] });
        let vect = |p: [f32; 3]| Value::Vector(vec3(p[0], p[1], p[2]));
        match *self {
            ShaderNode::TextureCoordinate | ShaderNode::Geometry | ShaderNode::ObjectInfo => Value::Float(0.0),
            ShaderNode::LayerWeight => {
                let (blend, n, incoming, backfacing) = (f(0), v(1), v(2), f(3) > 0.5);
                let cos = incoming[0] * n[0] + incoming[1] * n[1] + incoming[2] * n[2];
                if index == 0 {
                    let eta = (1.0 - blend).max(1e-5);
                    let eta = if backfacing { eta } else { 1.0 / eta };
                    Value::Float(fresnel_dielectric_cos(cos, eta))
                } else {
                    let mut facing = cos.abs();
                    if blend != 0.5 {
                        let b = blend.clamp(0.0, 1.0 - 1e-5);
                        let b = if b < 0.5 { 2.0 * b } else { 0.5 / (1.0 - b) };
                        facing = facing.powf(b);
                    }
                    Value::Float(1.0 - facing)
                }
            }
            ShaderNode::Fresnel => {
                let (n, incoming, backfacing) = (v(1), v(2), f(3) > 0.5);
                let eta = f(0).max(1e-5);
                let eta = if backfacing { 1.0 / eta } else { eta };
                Value::Float(fresnel_dielectric_cos(incoming[0] * n[0] + incoming[1] * n[1] + incoming[2] * n[2], eta))
            }
            ShaderNode::Noise { dims, fractal, normalize } => {
                let s = f(2);
                let settings = NoiseSettings { kind: fractal, detail: f(3), roughness: f(4), lacunarity: f(5), offset: f(6), gain: f(7), distortion: f(8), normalize };
                let p = v(0).map(|x| x * s);
                let w = f(1) * s;
                let (fac, col) = match dims {
                    1 => settings.texture(w),
                    2 => settings.texture([p[0], p[1]]),
                    3 => settings.texture(p),
                    _ => settings.texture([p[0], p[1], p[2], w]),
                };
                if index == 0 { Value::Float(fac) } else { colour(col) }
            }
            ShaderNode::Voronoi { dims, feature, metric, normalize } => {
                let s = f(2);
                let settings = VoronoiSettings {
                    feature,
                    metric,
                    scale: s,
                    detail: f(3),
                    roughness: f(4),
                    lacunarity: f(5),
                    smoothness: f(6),
                    exponent: f(7),
                    randomness: f(8),
                    normalize,
                };
                let out = settings.texture(dims, v(0).map(|x| x * s), f(1) * s);
                match feature {
                    VoronoiFeature::DistanceToEdge => Value::Float(out.distance),
                    VoronoiFeature::NSphereRadius => Value::Float(out.radius),
                    _ => match index {
                        0 => Value::Float(out.distance),
                        1 => colour(out.color),
                        2 => vect([out.position[0], out.position[1], out.position[2]]),
                        _ => Value::Float(out.position[3]),
                    },
                }
            }
            ShaderNode::Wave { kind, direction, profile } => {
                let s = f(1);
                let value = textures::wave(kind, direction, profile, v(0).map(|x| x * s), f(2), f(3), f(4), f(5), f(6));
                if index == 0 { colour([value; 3]) } else { Value::Float(value) }
            }
            ShaderNode::Magic { depth } => {
                let col = textures::magic(v(0), f(1), depth, f(2));
                if index == 0 { colour(col) } else { Value::Float((col[0] + col[1] + col[2]) / 3.0) }
            }
            ShaderNode::Brick { offset, offset_frequency, squash, squash_frequency } => {
                let s = f(4);
                let settings = BrickSettings {
                    mortar_size: f(5),
                    mortar_smooth: f(6),
                    bias: f(7),
                    brick_width: f(8),
                    row_height: f(9),
                    offset_amount: offset,
                    offset_frequency,
                    squash_amount: squash,
                    squash_frequency,
                };
                let (col, fac) = settings.texture(v(0).map(|x| x * s), c(1), c(2), c(3));
                if index == 0 { colour(col) } else { Value::Float(fac) }
            }
            ShaderNode::Checker => {
                let s = f(3);
                let fac = textures::checker(v(0).map(|x| x * s));
                if index == 0 { colour(if fac == 1.0 { c(1) } else { c(2) }) } else { Value::Float(fac) }
            }
            ShaderNode::GradientTexture { kind } => {
                let fac = textures::gradient(kind, v(0));
                if index == 0 { colour([fac; 3]) } else { Value::Float(fac) }
            }
            ShaderNode::WhiteNoise { dims } => {
                let (value, col) = textures::white_noise(dims, v(0), f(1));
                if index == 0 { Value::Float(value) } else { colour(col) }
            }
            ShaderNode::Gabor { kind } => {
                let (value, phase, intensity) = textures::gabor(kind, v(0), f(1), f(2), f(3), f(4), v(5));
                Value::Float(match index {
                    0 => value,
                    1 => phase,
                    _ => intensity,
                })
            }
            ShaderNode::Mix { data, blend, clamp_factor, clamp_result } => {
                let t = if clamp_factor { f(0).clamp(0.0, 1.0) } else { f(0) };
                match data {
                    MixData::Float => Value::Float(f(1) + t * (f(2) - f(1))),
                    MixData::Vector => {
                        let (a, b) = (v(1), v(2));
                        vect(std::array::from_fn(|i| a[i] + t * (b[i] - a[i])))
                    }
                    MixData::Color => {
                        let out = blend.blend(t, c(1), c(2));
                        colour(if clamp_result { out.map(|x| x.clamp(0.0, 1.0)) } else { out })
                    }
                }
            }
            ShaderNode::HueSaturationValue => colour(ops::hue_saturation_value(c(4), f(0), f(1), f(2), f(3))),
            ShaderNode::BrightContrast => colour(ops::bright_contrast(c(0), f(1), f(2))),
            ShaderNode::Gamma => colour(ops::gamma(c(0), f(1))),
            ShaderNode::InvertColor => colour(ops::invert(c(1), f(0))),
            ShaderNode::Math { op, clamp } => {
                let value = op.apply(f(0), f(1), f(2));
                Value::Float(if clamp { value.clamp(0.0, 1.0) } else { value })
            }
            ShaderNode::VectorMath { op } => {
                let (vector, value) = op.apply(v(0), v(1), v(2), f(3));
                if op.gives_value() { Value::Float(value) } else { vect(vector) }
            }
            ShaderNode::MapRange { kind, clamp } => Value::Float(ops::map_range(kind, f(0), f(1), f(2), f(3), f(4), f(5), clamp)),
            ShaderNode::Clamp { range } => Value::Float(ops::clamp(f(0), f(1), f(2), range)),
            ShaderNode::SeparateXyz => Value::Float(v(0)[index.min(2) as usize]),
            ShaderNode::CombineXyz => vect([f(0), f(1), f(2)]),
            ShaderNode::SeparateColor { model } => Value::Float(model.separate(c(0))[index.min(2) as usize]),
            ShaderNode::CombineColor { model } => colour(model.combine([f(0), f(1), f(2)])),
            ShaderNode::Mapping { kind } => vect(ops::mapping(kind, v(0), v(1), v(2), v(3))),
        }
    }
}

/// Fresnel reflectance for a cosine and relative index (Cycles'
/// `fresnel_dielectric_cos`).
fn fresnel_dielectric_cos(cosi: f32, eta: f32) -> f32 {
    let c = cosi.abs();
    let mut g = eta * eta - 1.0 + c * c;
    if g > 0.0 {
        g = g.sqrt();
        let a = (g - c) / (g + c);
        let b = (c * (g + c) - 1.0) / (c * (g - c) + 1.0);
        0.5 * a * a * (1.0 + b * b)
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every node's settings survive a round trip through their keys, and
    /// every key names a node.
    #[test]
    fn settings_round_trip() {
        for node in ShaderNode::catalog() {
            assert_eq!(ShaderNode::from_key(node.key()).map(|n| n.key()), Some(node.key()));
            let mut copy = ShaderNode::from_key(node.key()).unwrap();
            for setting in node.settings() {
                assert!(copy.set(setting.key, setting.value), "{} would not take its own {}", node.key(), setting.key);
            }
            assert_eq!(copy, node, "{}", node.key());
        }
    }

    /// Each node's inputs have the defaults its evaluation expects: a Noise
    /// at its defaults is a pattern, not a flat grey.
    #[test]
    fn a_noise_at_its_defaults_varies() {
        let node = ShaderNode::noise();
        let at = |p: Vec3| {
            let mut inputs: Vec<Value> = node.inputs().iter().map(|i| i.default).collect();
            inputs[0] = Value::Vector(p);
            node.evaluate(&inputs, 0).as_float()
        };
        let values: Vec<f32> = (0..20).map(|i| at(vec3(i as f32 * 0.013, 0.02, 0.0))).collect();
        let spread = values.iter().cloned().fold(f32::MIN, f32::max) - values.iter().cloned().fold(f32::MAX, f32::min);
        assert!(spread > 0.1, "{values:?}");
    }
}
