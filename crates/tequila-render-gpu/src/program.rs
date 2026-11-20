//! Material graphs on the card (`docs/07` R3, stage 1): a material's
//! compiled program (`tequila_materials::graph::CompiledGraph`) lowered to
//! the bytecode `program.wgsl` interprets, one generic loop for every
//! material, so no shader is compiled because a material changed.
//!
//! The lowering, on the processor:
//! - literals become a constant pool, and an operand reads one directly;
//! - only what a driven parameter needs is kept (`restricted_to` all of
//!   them);
//! - every node is one instruction: the outputs the compiler applies one by
//!   one (a Noise read for its Fac and its Color) are one instruction
//!   writing a register the readers take their lane of, and Separate XYZ
//!   (or Separate Color in RGB) is no instruction at all, its readers
//!   reading the lane straight from the vector;
//! - registers are allocated by liveness into the card's fixed [`REGISTERS`];
//! - an operand is a register or a constant, read as it is, as one lane, or
//!   - a colour read as a number - as its luminance (`Value::as_float`);
//! - the driven parameters' write-back into the material (`shade.rs`'s
//!   `write_param`: colours and vectors as colours, scalars as numbers), the
//!   Bump and Tangent slots kept for the integrator;
//! - and for a height bump, the program restricted to the Bump alone, which
//!   the card evaluates twice more a step along the surface
//!   (`surface_bump.rs`).
//!
//! What the card cannot run yet - textures, Tequila's procedurals, surface
//! readers, layered surfaces, labels, a node not ported, a parameter the
//! card's stack does not read - it refuses, with the reason: the material
//! then renders as its constants, and says so. Never silently wrong.

use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use tequila_materials::graph::{CompiledGraph, CurvatureMode, Instruction, NodeKind, Register, SocketType, Value};
use tequila_materials::material_type::Param;
use tequila_materials::nodes::{MixData, PointInput, ShaderNode};
use tequila_texture::cycles::noise::FractalKind;
use tequila_texture::cycles::ops::{BlendMode, ColorModel, MapRangeKind, MappingKind, MathOp, VectorMathOp};
use tequila_texture::cycles::textures::{GaborKind, GradientKind, WaveDirection, WaveKind, WaveProfile};
use tequila_texture::cycles::voronoi::{VoronoiFeature, VoronoiMetric};
use tequila_texture::gradient::Interpolation;
use tequila_texture::mip::Wrap;
use tequila_texture::noise::{Basis, Fractal, Metric};
use tequila_texture::procedural::Procedural;
use tequila_texture::projection::{Axis, Mapping, Projection};
use crate::textures::{Pictures, MAX_PYRAMIDS};

/// The registers the card has: `program.wgsl`'s `prog_regs`.
pub const REGISTERS: usize = 48;
/// The most operands one instruction takes (a colour ramp's stops).
pub const MAX_OPERANDS: usize = 40;
/// No operand, no register.
pub const NONE: u32 = u32::MAX;

/// An operand: bits 0-11 a register or a constant's index in the program's
/// pool, bit 12 set for a constant, bits 13-15 how it is read.
pub const OPERAND_INDEX: u32 = 0xFFF;
pub const OPERAND_CONSTANT: u32 = 1 << 12;
pub const OPERAND_MODE_SHIFT: u32 = 13;
/// Read as it is.
pub const MODE_AS_IS: u32 = 0;
/// A colour read as a number: its luminance, everywhere.
pub const MODE_LUMINANCE: u32 = 1;
/// Lane 0 everywhere; lanes 1-3 are the next three modes.
pub const MODE_LANE0: u32 = 2;

// The instructions: word 0 is the opcode (bits 0-7), the register written
// (8-15), a second one (16-23, 0xFF for none) and the operand count
// (24-31); word 1 the node's settings; then the operands. program.wgsl
// holds the same numbers, which a test reads back.
pub const OP_INPUT: u32 = 1;
pub const OP_MATH: u32 = 2;
pub const OP_VECTOR_MATH: u32 = 3;
pub const OP_MAP_RANGE: u32 = 4;
pub const OP_MIX: u32 = 5;
pub const OP_NOISE: u32 = 6;
pub const OP_VORONOI: u32 = 7;
pub const OP_WHITE_NOISE: u32 = 8;
pub const OP_MAPPING: u32 = 9;
pub const OP_COMBINE_XYZ: u32 = 10;
pub const OP_SEPARATE_COLOR: u32 = 11;
pub const OP_GRADIENT: u32 = 12;
pub const OP_LEGACY: u32 = 13;
pub const OP_LAYER_WEIGHT: u32 = 14;
pub const OP_FRESNEL: u32 = 15;
pub const OP_CLAMP: u32 = 16;
pub const OP_COMBINE_COLOR: u32 = 17;
pub const OP_HSV: u32 = 18;
pub const OP_BRIGHT_CONTRAST: u32 = 19;
pub const OP_GAMMA: u32 = 20;
pub const OP_INVERT_COLOR: u32 = 21;
pub const OP_CHECKER: u32 = 22;
pub const OP_GRADIENT_TEXTURE: u32 = 23;
/// An image texture, its alpha, a Mapping 2D's coordinates, or a normal
/// map laid down by a projection (`textures.wgsl`): the settings word's
/// [`TEXTURE_MODE`] says which.
pub const OP_TEXTURE: u32 = 24;
/// One step of a Tequila procedural (`procedural.wgsl`): a stage of its
/// arithmetic, a fractal of noise, or Worley's cells, by the settings
/// word's [`PROCEDURAL_STEP`].
pub const OP_PROCEDURAL: u32 = 25;
/// The Wave Texture's point, or its value at the point (`procedural.wgsl`'s
/// `cy_wave`): the settings word's [`WAVE_VALUE`].
pub const OP_WAVE: u32 = 26;
/// What the snapshot measured at the mesh's corners, read at the point
/// (`program.wgsl`'s `prog_attribute`): the curvature as a mask, the baked
/// occlusion shaped, the distance to other parts, the vertex colour; the
/// settings word's [`ATTRIBUTE_WHICH`].
pub const OP_ATTRIBUTE: u32 = 27;
/// The Gabor Texture (`procedural.wgsl`'s `cy_gabor`): bit 0 of the
/// settings word for space rather than a plane.
pub const OP_GABOR: u32 = 28;

/// Every opcode by its WGSL name, for the test that holds the two sides to
/// one numbering.
pub const OPCODES: &[(&str, u32)] = &[
    ("OP_INPUT", OP_INPUT),
    ("OP_MATH", OP_MATH),
    ("OP_VECTOR_MATH", OP_VECTOR_MATH),
    ("OP_MAP_RANGE", OP_MAP_RANGE),
    ("OP_MIX", OP_MIX),
    ("OP_NOISE", OP_NOISE),
    ("OP_VORONOI", OP_VORONOI),
    ("OP_WHITE_NOISE", OP_WHITE_NOISE),
    ("OP_MAPPING", OP_MAPPING),
    ("OP_COMBINE_XYZ", OP_COMBINE_XYZ),
    ("OP_SEPARATE_COLOR", OP_SEPARATE_COLOR),
    ("OP_GRADIENT", OP_GRADIENT),
    ("OP_LEGACY", OP_LEGACY),
    ("OP_LAYER_WEIGHT", OP_LAYER_WEIGHT),
    ("OP_FRESNEL", OP_FRESNEL),
    ("OP_CLAMP", OP_CLAMP),
    ("OP_COMBINE_COLOR", OP_COMBINE_COLOR),
    ("OP_HSV", OP_HSV),
    ("OP_BRIGHT_CONTRAST", OP_BRIGHT_CONTRAST),
    ("OP_GAMMA", OP_GAMMA),
    ("OP_INVERT_COLOR", OP_INVERT_COLOR),
    ("OP_CHECKER", OP_CHECKER),
    ("OP_GRADIENT_TEXTURE", OP_GRADIENT_TEXTURE),
    ("OP_TEXTURE", OP_TEXTURE),
    ("OP_PROCEDURAL", OP_PROCEDURAL),
    ("OP_WAVE", OP_WAVE),
    ("OP_ATTRIBUTE", OP_ATTRIBUTE),
    ("OP_GABOR", OP_GABOR),
];

/// The Gabor Texture's normalization, six of its deviations, worked out as
/// `textures.rs`'s `gabor` works it out.
pub fn gabor_normalization(kind: GaborKind) -> f32 {
    const IMPULSES: f32 = 8.0;
    let deviation = match kind {
        GaborKind::Plane => (IMPULSES * 0.5 * 0.25).sqrt(),
        GaborKind::Space => (IMPULSES * 0.5 * (1.0 / (4.0 * std::f32::consts::SQRT_2))).sqrt(),
    };
    6.0 * deviation
}

/// `OP_ATTRIBUTE`'s settings word: bits 0-1 which, bits 2-3 the curvature's
/// mode (convex, concave, both).
pub const ATTRIBUTE_WHICH: u32 = 3;
pub const ATTRIBUTE_CURVATURE: u32 = 0;
pub const ATTRIBUTE_OCCLUSION: u32 = 1;
pub const ATTRIBUTE_DISTANCE: u32 = 2;
pub const ATTRIBUTE_VERTEX_COLOR: u32 = 3;

/// Whether `program` reads what the snapshot measured at the mesh's
/// corners (the curvature, the baked occlusion, the closeness of other
/// parts, the vertex colours), which the card then uploads.
pub fn reads_attributes(program: &CompiledGraph) -> bool {
    let reads = |instructions: &[Instruction]| {
        instructions.iter().any(|i| matches!(i, Instruction::Curvature { .. } | Instruction::Occlusion { .. } | Instruction::Distance { .. } | Instruction::VertexColor { .. }))
    };
    reads(&program.instructions) || program.layer.as_ref().is_some_and(|l| reads_attributes(&l.factor) || reads_attributes(&l.shader))
}

/// `OP_WAVE`'s settings word: rings (bit 0) or bands, the direction (bits
/// 1-2), the profile (3-4), and the value at the point rather than the point.
pub const WAVE_RINGS: u32 = 1;
pub const WAVE_VALUE: u32 = 1 << 5;

/// `OP_PROCEDURAL`'s settings word: bits 0-1 the step - a stage of a
/// pattern (its pattern in bits 2-6, which stage in 7-9, the axis in 10-11,
/// [`PROCEDURAL_MAPPED`]), a fractal (its kind in bits 2-3, the value basis
/// bit 4, [`FRACTAL_UNIT`], the octaves from bit 8) or Worley's cells (the
/// metric in bits 2-3).
pub const PROCEDURAL_STEP: u32 = 3;
pub const PROCEDURAL_STAGE: u32 = 0;
pub const PROCEDURAL_FRACTAL: u32 = 1;
pub const PROCEDURAL_WORLEY: u32 = 2;
/// A stage whose point is a mapping's (u, v), not the part's own.
pub const PROCEDURAL_MAPPED: u32 = 1 << 12;
/// A fractal's answer brought from [-1, 1] to [0, 1] (`* 0.5 + 0.5`).
pub const FRACTAL_UNIT: u32 = 1 << 5;
/// The patterns the card runs, by `procedural.wgsl`'s numbers.
pub const PATTERN_SCRATCHES: u32 = 0;
pub const PATTERN_BRUSHED_RADIAL: u32 = 1;
pub const PATTERN_LEATHER: u32 = 2;
/// The most octaves a fractal's settings word holds.
pub const MAX_OCTAVES: u32 = (1 << 24) - 1;

/// `OP_TEXTURE`'s settings word: bits 0-1 what it gives back - a colour
/// (`Instruction::Sample`), a number (`SampleAlpha`), the coordinates
/// (`Project`) or a normal (`BumpSource::NormalMap` through a
/// projection); bits 2-4 the projection, 5-6 its axis, 7-8 and 9-10 the
/// wraps across and up; the flags; and from bit 18 the pyramid
/// (`crate::textures`).
pub const TEXTURE_MODE: u32 = 3;
pub const TEXTURE_COLOUR: u32 = 0;
pub const TEXTURE_ALPHA: u32 = 1;
pub const TEXTURE_PROJECT: u32 = 2;
pub const TEXTURE_NORMAL: u32 = 3;
pub const TEXTURE_FLIP_U: u32 = 1 << 11;
pub const TEXTURE_FLIP_V: u32 = 1 << 12;
pub const TEXTURE_REAL_WORLD: u32 = 1 << 13;
pub const TEXTURE_RIGID: u32 = 1 << 14;
/// The mapping turns (`rotation != 0`).
pub const TEXTURE_ROTATES: u32 = 1 << 15;
/// At a place the graph worked out (`Instruction::Sample`'s `at`), its
/// register the first operand.
pub const TEXTURE_AT: u32 = 1 << 16;
/// A colour to bring from linear sRGB into the rendering space
/// (`shade.rs`'s `colour_of`).
pub const TEXTURE_MANAGED: u32 = 1 << 17;
pub const TEXTURE_PYRAMID_SHIFT: u32 = 18;

/// `textures.wgsl`'s code for a projection.
pub fn projection_code(projection: Projection) -> u32 {
    match projection {
        Projection::Uv => 0,
        Projection::Planar => 1,
        Projection::Cylindrical => 2,
        Projection::Spherical => 3,
        Projection::TriPlanar => 4,
    }
}

fn axis_code(axis: Axis) -> u32 {
    match axis {
        Axis::X => 0,
        Axis::Y => 1,
        Axis::Z => 2,
    }
}

fn wrap_code(wrap: Wrap) -> u32 {
    match wrap {
        Wrap::Repeat => 0,
        Wrap::Clamp => 1,
        Wrap::Mirror => 2,
    }
}

/// The three constants a lookup through `mapping` reads, worked out as the
/// processor works them out each time (`Mapping::finish_anisotropic`,
/// `spin`, `tri_planar`), so the card has the processor's bits: the scale
/// as repeats and the offset; the turn's cosine and sine and the pivot; the
/// tri-planar blend's exponent.
pub fn mapping_constants(mapping: &Mapping) -> [[f32; 4]; 3] {
    let (su, sv) = mapping.repeats();
    let (sin, cos) = (mapping.rotation * std::f32::consts::TAU).sin_cos();
    [
        [su, sv, mapping.offset.x, mapping.offset.y],
        [cos, sin, mapping.pivot.x, mapping.pivot.y],
        [mapping.blend_sharpness.max(1.0), 0.0, 0.0, 0.0],
    ]
}

/// `OP_LEGACY`'s settings word: which of Tequila's own nodes.
pub const LEGACY_MIX: u32 = 0;
pub const LEGACY_MULTIPLY: u32 = 1;
pub const LEGACY_ADD: u32 = 2;
pub const LEGACY_INVERT: u32 = 3;
pub const LEGACY_REMAP: u32 = 4;
pub const LEGACY_CLAMP: u32 = 5;
pub const LEGACY_CHANNEL: u32 = 6;
pub const LEGACY_BUMP: u32 = 7;

/// The code `program.wgsl`'s point inputs know a `PointInput` by.
pub fn point_input_code(input: PointInput) -> u32 {
    use PointInput::*;
    match input {
        Object => 0,
        Generated => 1,
        Uv => 2,
        ObjectNormal => 3,
        Reflection => 4,
        Position => 5,
        Normal => 6,
        TrueNormal => 7,
        Incoming => 8,
        Backfacing => 9,
        ObjectLocation => 10,
        ObjectRandom => 11,
        ObjectColor => 12,
        ObjectNumber => 13,
        IsCameraRay => 14,
        IsShadowRay => 15,
        IsDiffuseRay => 16,
        IsGlossyRay => 17,
        IsTransmissionRay => 18,
    }
}

fn index_of<T: PartialEq>(all: &[T], value: &T) -> u32 {
    all.iter().position(|v| v == value).expect("every value is in its ALL") as u32
}

pub fn math_code(op: MathOp) -> u32 {
    index_of(MathOp::ALL, &op)
}
pub fn vector_math_code(op: VectorMathOp) -> u32 {
    index_of(VectorMathOp::ALL, &op)
}
pub fn blend_code(blend: BlendMode) -> u32 {
    index_of(BlendMode::ALL, &blend)
}
pub fn map_range_code(kind: MapRangeKind) -> u32 {
    index_of(&MapRangeKind::ALL, &kind)
}
pub fn mapping_code(kind: MappingKind) -> u32 {
    index_of(&MappingKind::ALL, &kind)
}
/// fBM 0, multifractal 1, hybrid multifractal 2, ridged multifractal 3,
/// hetero terrain 4: `FractalKind::ALL`'s order.
pub fn fractal_code(kind: FractalKind) -> u32 {
    index_of(&FractalKind::ALL, &kind)
}
pub fn feature_code(feature: VoronoiFeature) -> u32 {
    index_of(&VoronoiFeature::ALL, &feature)
}
pub fn metric_code(metric: VoronoiMetric) -> u32 {
    index_of(&VoronoiMetric::ALL, &metric)
}
pub fn model_code(model: ColorModel) -> u32 {
    index_of(&ColorModel::ALL, &model)
}
pub fn gradient_kind_code(kind: GradientKind) -> u32 {
    index_of(&GradientKind::ALL, &kind)
}
pub fn mix_data_code(data: MixData) -> u32 {
    match data {
        MixData::Float => 0,
        MixData::Vector => 1,
        MixData::Color => 2,
    }
}

/// The settings words of the nodes with settings, as `program.wgsl` reads
/// them: shared by the lowering and the per-node tests.
pub mod settings {
    use super::*;

    pub fn math(op: MathOp, clamp: bool) -> u32 {
        math_code(op) | (clamp as u32) << 8
    }
    pub fn map_range(kind: MapRangeKind, clamp: bool) -> u32 {
        map_range_code(kind) | (clamp as u32) << 4
    }
    pub fn mix(data: MixData, blend: BlendMode, clamp_factor: bool, clamp_result: bool) -> u32 {
        mix_data_code(data) | blend_code(blend) << 2 | (clamp_factor as u32) << 8 | (clamp_result as u32) << 9
    }
    pub fn noise(dims: u8, fractal: FractalKind, normalize: bool, colour: bool) -> u32 {
        dims as u32 | fractal_code(fractal) << 4 | (normalize as u32) << 8 | (colour as u32) << 9
    }
    pub fn voronoi(dims: u8, feature: VoronoiFeature, metric: VoronoiMetric, normalize: bool, colour: bool) -> u32 {
        dims as u32 | feature_code(feature) << 4 | metric_code(metric) << 8 | (normalize as u32) << 12 | (colour as u32) << 13
    }
    pub fn white_noise(dims: u8, colour: bool) -> u32 {
        dims as u32 | (colour as u32) << 4
    }
    /// `OP_TEXTURE`'s: what it gives back, the mapping's projection, axis,
    /// wraps and flags, whether it reads a place the graph worked out,
    /// whether its colour is brought into the rendering space, and the
    /// pyramid.
    pub fn texture(mode: u32, mapping: &Mapping, at: bool, colour_managed: bool, pyramid: u32) -> u32 {
        let flag = |on: bool, bit: u32| if on { bit } else { 0 };
        mode | projection_code(mapping.projection) << 2
            | axis_code(mapping.axis) << 5
            | wrap_code(mapping.wrap_u) << 7
            | wrap_code(mapping.wrap_v) << 9
            | flag(mapping.flip_u, TEXTURE_FLIP_U)
            | flag(mapping.flip_v, TEXTURE_FLIP_V)
            | flag(mapping.real_world_units, TEXTURE_REAL_WORLD)
            | flag(mapping.rigid, TEXTURE_RIGID)
            | flag(mapping.rotation != 0.0, TEXTURE_ROTATES)
            | flag(at, TEXTURE_AT)
            | flag(colour_managed, TEXTURE_MANAGED)
            | pyramid << TEXTURE_PYRAMID_SHIFT
    }
    /// `OP_WAVE`'s: the point (`value` false) or the value at it.
    pub fn wave(kind: WaveKind, direction: WaveDirection, profile: WaveProfile, value: bool) -> u32 {
        let d = match direction {
            WaveDirection::X => 0,
            WaveDirection::Y => 1,
            WaveDirection::Z => 2,
            WaveDirection::Diagonal => 3,
        };
        let p = match profile {
            WaveProfile::Sine => 0,
            WaveProfile::Saw => 1,
            WaveProfile::Triangle => 2,
        };
        (kind == WaveKind::Rings) as u32 | d << 1 | p << 3 | if value { WAVE_VALUE } else { 0 }
    }
    /// `OP_PROCEDURAL`'s for a stage of `pattern`.
    pub fn procedural_stage(pattern: u32, stage: u32, axis: Axis, mapped: bool) -> u32 {
        PROCEDURAL_STAGE | pattern << 2 | stage << 7 | axis_code(axis) << 10 | if mapped { PROCEDURAL_MAPPED } else { 0 }
    }
    /// `OP_PROCEDURAL`'s for a fractal: fBm 0, turbulence 1, ridged 2.
    pub fn fractal(kind: u32, basis: Basis, unit: bool, octaves: u32) -> u32 {
        PROCEDURAL_FRACTAL | kind << 2 | ((basis == Basis::Value) as u32) << 4 | if unit { FRACTAL_UNIT } else { 0 } | octaves.clamp(1, MAX_OCTAVES) << 8
    }
    /// `OP_PROCEDURAL`'s for Worley's cells.
    pub fn worley(metric: Metric) -> u32 {
        let code = match metric {
            Metric::Euclidean => 0,
            Metric::Manhattan => 1,
            Metric::Chebyshev => 2,
        };
        PROCEDURAL_WORLEY | code << 2
    }
    pub fn gradient(interpolation: Interpolation, stops: usize) -> u32 {
        let i = match interpolation {
            Interpolation::Linear => 0,
            Interpolation::Ease => 1,
            Interpolation::Constant => 2,
        };
        i | (stops as u32) << 4
    }
}

/// Why the card does not run a material's program, and renders it as its
/// constants instead.
#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    /// A Mix or Add Shader of unlike surfaces.
    Layered,
    /// Labels.
    Decals,
    /// An instruction of a kind the card has no port of yet.
    Instruction(&'static str),
    /// A node the card has no port of yet.
    Node(&'static str),
    /// A Tequila procedural pattern the card has no port of yet.
    Procedural(&'static str),
    /// A driven parameter the card's stack does not read.
    Param(Param),
    /// More registers live at once than the card has.
    Registers(usize),
    /// An instruction with more operands than the card reads.
    Operands(usize),
    /// A register written twice, which the compiler never does.
    NotSsa,
    /// The scene's pictures take more than the card's buffers hold.
    PicturesTooLarge { bytes: u64, limit: u64 },
    /// More pictures than an instruction can name.
    TooManyPictures,
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Layered => write!(f, "it mixes unlike surfaces (Mix or Add Shader)"),
            Refusal::Decals => write!(f, "it carries labels"),
            Refusal::Instruction(what) => write!(f, "it reads {what}"),
            Refusal::Node(what) => write!(f, "its {what} node is not on the card yet"),
            Refusal::Procedural(what) => write!(f, "its {what} pattern is not on the card yet"),
            Refusal::Param(param) => write!(f, "it drives {}, which the card's surface does not read", param.label()),
            Refusal::Registers(n) => write!(f, "it holds {n} values at once, more than the card's {REGISTERS}"),
            Refusal::Operands(n) => write!(f, "a node of it takes {n} operands, more than the card's {MAX_OPERANDS}"),
            Refusal::NotSsa => write!(f, "a register is written twice"),
            Refusal::PicturesTooLarge { bytes, limit } => write!(
                f,
                "the scene's pictures take {} MB on the card, more than the {} MB it binds",
                bytes.div_ceil(1 << 20),
                limit / (1 << 20)
            ),
            Refusal::TooManyPictures => write!(f, "it reads a picture past the card's {MAX_PYRAMIDS}"),
        }
    }
}

/// A material's program as the card runs it, its offsets its own (the
/// renderer lays every material's after the one before).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CardProgram {
    /// The program that drives the parameters.
    pub main: Vec<u32>,
    /// The program restricted to the bump's height, when the material
    /// bumps its surface by one.
    pub bump: Vec<u32>,
    /// The constant pool both read.
    pub constants: Vec<[f32; 4]>,
    /// Pairs: the parameter's place in the material (a float offset in
    /// `GpuMaterial`, and bit 8 set for a colour, three floats), and the
    /// operand its value is in.
    pub writes: Vec<u32>,
    /// The operand the main program leaves the bump's height in.
    pub bump_here: u32,
    /// The operand the bump program leaves it in.
    pub bump_height: u32,
    /// The operand the main program leaves the Tangent in, `NONE` without.
    pub tangent: u32,
    /// The most registers either program holds at once.
    pub registers: usize,
    /// [`PROGRAM_NORMAL_UV`] or [`PROGRAM_NORMAL_PROJECTED`] for a normal
    /// map, its operand `bump_here`; 0 for none.
    pub normal_map: u32,
}

impl CardProgram {
    /// Whether it bumps the surface by a height.
    pub fn has_bump(&self) -> bool {
        self.bump_height != NONE
    }
}

/// `program.wgsl`'s `GpuProgram`: where a material's program is in the
/// shared buffers. Every offset is absolute; a material with no program
/// has `main_start == main_end` and no bump.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub struct GpuProgram {
    pub main_start: u32,
    pub main_end: u32,
    pub bump_start: u32,
    pub bump_end: u32,
    /// The first of its constants in the constant buffer.
    pub constants: u32,
    /// Its write-back pairs, in the word buffer.
    pub writes_start: u32,
    pub writes_count: u32,
    pub bump_here: u32,
    pub bump_height: u32,
    pub tangent: u32,
    /// Bit 0: a program to run; bit 1: a height bump.
    pub flags: u32,
    pub _pad: u32,
}

pub const PROGRAM_RUNS: u32 = 1;
pub const PROGRAM_BUMPS: u32 = 2;
/// A normal map through the UVs: its texel in `bump_here`.
pub const PROGRAM_NORMAL_UV: u32 = 4;
/// A normal map laid down by a projection: the normal it gives, in the
/// part's space, in `bump_here`.
pub const PROGRAM_NORMAL_PROJECTED: u32 = 8;

impl GpuProgram {
    pub const EMPTY: GpuProgram = GpuProgram {
        main_start: 0,
        main_end: 0,
        bump_start: 0,
        bump_end: 0,
        constants: 0,
        writes_start: 0,
        writes_count: 0,
        bump_here: NONE,
        bump_height: NONE,
        tangent: NONE,
        flags: 0,
        _pad: 0,
    };
}

/// Every material's program, laid one after another for the card.
#[derive(Clone, Debug, Default)]
pub struct ProgramTable {
    pub headers: Vec<GpuProgram>,
    pub words: Vec<u32>,
    pub constants: Vec<[f32; 4]>,
}

impl ProgramTable {
    /// The table of `programs`, one entry a material; `None` runs nothing.
    pub fn new(programs: &[Option<CardProgram>]) -> ProgramTable {
        let mut table = ProgramTable::default();
        for program in programs {
            let Some(p) = program else {
                table.headers.push(GpuProgram::EMPTY);
                continue;
            };
            let constants = table.constants.len() as u32;
            table.constants.extend_from_slice(&p.constants);
            let main_start = table.words.len() as u32;
            table.words.extend_from_slice(&p.main);
            let main_end = table.words.len() as u32;
            let bump_start = table.words.len() as u32;
            table.words.extend_from_slice(&p.bump);
            let bump_end = table.words.len() as u32;
            let writes_start = table.words.len() as u32;
            table.words.extend_from_slice(&p.writes);
            table.headers.push(GpuProgram {
                main_start,
                main_end,
                bump_start,
                bump_end,
                constants,
                writes_start,
                writes_count: (p.writes.len() / 2) as u32,
                bump_here: p.bump_here,
                bump_height: p.bump_height,
                tangent: p.tangent,
                flags: PROGRAM_RUNS | if p.has_bump() { PROGRAM_BUMPS } else { 0 } | p.normal_map,
                _pad: 0,
            });
        }
        // A buffer may not be empty.
        if table.headers.is_empty() {
            table.headers.push(GpuProgram::EMPTY);
        }
        if table.words.is_empty() {
            table.words.push(0);
        }
        if table.constants.is_empty() {
            table.constants.push([0.0; 4]);
        }
        table
    }
}

/// What a value is, as far as reading it goes: a number (or a flag) is held
/// in every lane, a colour and a vector in the first three.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ty {
    Float,
    Color,
    Vector,
}

fn ty_of(socket: SocketType) -> Ty {
    match socket {
        SocketType::Color => Ty::Color,
        SocketType::Vector | SocketType::Normal => Ty::Vector,
        _ => Ty::Float,
    }
}

fn canonical(value: Value) -> ([f32; 4], Ty) {
    match value {
        Value::Float(f) => ([f; 4], Ty::Float),
        Value::Bool(b) => ([if b { 1.0 } else { 0.0 }; 4], Ty::Float),
        Value::Color(c) => ([c.r, c.g, c.b, 0.0], Ty::Color),
        Value::Vector(v) => ([v.x, v.y, v.z, 0.0], Ty::Vector),
    }
}

fn point_ty(input: PointInput) -> Ty {
    use PointInput::*;
    match input {
        Backfacing | ObjectRandom | ObjectNumber | IsCameraRay | IsShadowRay | IsDiffuseRay | IsGlossyRay | IsTransmissionRay => Ty::Float,
        ObjectColor => Ty::Color,
        _ => Ty::Vector,
    }
}

/// Where a value is: a virtual register (an instruction's result) or a
/// constant, and how it is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Virtual(usize),
    Constant(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Operand {
    source: Source,
    mode: u32,
}

/// A value: where it is and what it is.
#[derive(Clone, Copy, Debug)]
struct Held {
    operand: Operand,
    ty: Ty,
}

impl Held {
    /// The operand a reader of socket type `wanted` takes: a colour read as
    /// a number is its luminance; anything else reads as it is held.
    fn read_as(&self, wanted: Ty) -> Operand {
        if wanted == Ty::Float && self.ty == Ty::Color && self.operand.mode == MODE_AS_IS {
            Operand { mode: MODE_LUMINANCE, ..self.operand }
        } else {
            self.operand
        }
    }

    /// Lane `k` of it, read as a vector (Separate XYZ, a packed output).
    fn lane(&self, k: u32) -> Operand {
        match self.operand.mode {
            MODE_AS_IS => Operand { mode: MODE_LANE0 + k, ..self.operand },
            // A lane of a number held in every lane is that number.
            _ => self.operand,
        }
    }
}

struct Op {
    op: u32,
    settings: u32,
    dst: Option<usize>,
    dst2: Option<usize>,
    operands: Vec<Operand>,
}

/// One program's instructions lowered to operations on virtual registers.
struct Builder<'a> {
    constants: &'a mut Vec<[f32; 4]>,
    pooled: &'a mut HashMap<[u32; 4], u32>,
    pictures: Pictures<'a>,
    held: HashMap<u16, Held>,
    ops: Vec<Op>,
    virtuals: usize,
}

impl Builder<'_> {
    fn constant(&mut self, value: [f32; 4]) -> Operand {
        let key = value.map(f32::to_bits);
        let index = *self.pooled.entry(key).or_insert_with(|| {
            self.constants.push(value);
            (self.constants.len() - 1) as u32
        });
        Operand { source: Source::Constant(index), mode: MODE_AS_IS }
    }

    fn fresh(&mut self) -> usize {
        self.virtuals += 1;
        self.virtuals - 1
    }

    fn held(&self, register: Register) -> Result<Held, Refusal> {
        // A register nothing wrote reads Float(0): `Registers::prepare`.
        self.held.get(&register.0).copied().ok_or(Refusal::NotSsa)
    }

    fn define(&mut self, register: Register, held: Held) -> Result<(), Refusal> {
        if self.held.insert(register.0, held).is_some() {
            return Err(Refusal::NotSsa);
        }
        Ok(())
    }

    fn push(&mut self, op: u32, settings: u32, operands: Vec<Operand>, two: bool) -> Result<(usize, Option<usize>), Refusal> {
        if operands.len() > MAX_OPERANDS {
            return Err(Refusal::Operands(operands.len()));
        }
        let dst = self.fresh();
        let dst2 = two.then(|| self.fresh());
        self.ops.push(Op { op, settings, dst: Some(dst), dst2, operands });
        Ok((dst, dst2))
    }

    fn virtual_held(dst: usize, ty: Ty, mode: u32) -> Held {
        Held { operand: Operand { source: Source::Virtual(dst), mode }, ty }
    }

    /// The operands of an Apply's arguments, each read as its socket asks.
    fn arguments(&self, kind: &NodeKind, args: &[Register]) -> Result<Vec<Operand>, Refusal> {
        let sockets = kind.inputs();
        args.iter()
            .enumerate()
            .map(|(i, r)| {
                let wanted = sockets.get(i).map_or(Ty::Float, |(_, ty, _)| ty_of(*ty));
                Ok(self.held(*r)?.read_as(wanted))
            })
            .collect()
    }

    fn lower(&mut self, instructions: &[Instruction]) -> Result<(), Refusal> {
        let mut i = 0;
        while i < instructions.len() {
            match &instructions[i] {
                Instruction::Load { dst, value } => {
                    let (v, ty) = canonical(*value);
                    let operand = self.constant(v);
                    self.define(*dst, Held { operand, ty })?;
                }
                Instruction::Input { dst, input } => {
                    let (d, _) = self.push(OP_INPUT, point_input_code(*input), Vec::new(), false)?;
                    self.define(*dst, Self::virtual_held(d, point_ty(*input), MODE_AS_IS))?;
                }
                Instruction::Apply { kind, args, .. } => {
                    // The compiler applies a node once for each output read,
                    // one after another: one instruction here.
                    let mut group = vec![i];
                    while let Some(Instruction::Apply { kind: k, args: a, .. }) = instructions.get(i + group.len()) {
                        if k == kind && a == args {
                            group.push(i + group.len());
                        } else {
                            break;
                        }
                    }
                    let outputs: Vec<(Register, u8)> = group
                        .iter()
                        .map(|&j| match &instructions[j] {
                            Instruction::Apply { dst, output, .. } => (*dst, *output),
                            _ => unreachable!(),
                        })
                        .collect();
                    self.apply(kind, args, &outputs)?;
                    i += group.len();
                    continue;
                }
                Instruction::Project { dst, mapping } => {
                    // Stage 1's lowering, with no pictures, ran no
                    // projections either.
                    if matches!(self.pictures, Pictures::Unknown) {
                        return Err(Refusal::Instruction("a projection (Mapping 2D)"));
                    }
                    let zero = self.constant([0.0; 4]);
                    let (d, _) = self.texture(TEXTURE_PROJECT, mapping, zero, false, false, 0)?;
                    self.define(*dst, Self::virtual_held(d, Ty::Vector, MODE_AS_IS))?;
                }
                Instruction::Sample { dst, texture, mapping, fallback, at } | Instruction::SampleAlpha { dst, texture, mapping, fallback, at } => {
                    let alpha = matches!(instructions[i], Instruction::SampleAlpha { .. });
                    let pictures = match self.pictures {
                        Pictures::Unknown => return Err(Refusal::Instruction(if alpha { "an image's alpha" } else { "an image texture" })),
                        Pictures::TooLarge { bytes, limit } => return Err(Refusal::PicturesTooLarge { bytes, limit }),
                        Pictures::On(pictures) => pictures,
                    };
                    // A picture not loaded: the value the material had
                    // (shade.rs, the placeholder rule).
                    let Some(card) = pictures.texture(*texture) else {
                        let (v, ty) = canonical(*fallback);
                        let operand = self.constant(v);
                        self.define(*dst, Held { operand, ty })?;
                        i += 1;
                        continue;
                    };
                    let pyramid = match (alpha, card.alpha) {
                        (false, _) => card.colour,
                        (true, Some(levels)) => levels,
                        // No alpha: fully present.
                        (true, None) => {
                            let operand = self.constant([1.0; 4]);
                            self.define(*dst, Held { operand, ty: Ty::Float })?;
                            i += 1;
                            continue;
                        }
                    };
                    let place = match at {
                        Some(r) => self.held(*r)?.operand,
                        None => self.constant([0.0; 4]),
                    };
                    let (mode, ty) = if alpha { (TEXTURE_ALPHA, Ty::Float) } else { (TEXTURE_COLOUR, Ty::Color) };
                    let (d, _) = self.texture(mode, mapping, place, at.is_some(), !alpha && card.colour_managed, pyramid)?;
                    self.define(*dst, Self::virtual_held(d, ty, MODE_AS_IS))?;
                }
                Instruction::Evaluate { dst, procedural, seed, mapping } => {
                    // Stage 1's lowering ran no procedurals.
                    if matches!(self.pictures, Pictures::Unknown) {
                        return Err(Refusal::Instruction("a Tequila procedural"));
                    }
                    // The point: the part's own in scene units, or where the
                    // mapping puts it (its first contribution's u and v, as a
                    // Project instruction has them).
                    let point = match mapping {
                        None => self.push(OP_INPUT, point_input_code(PointInput::Object), Vec::new(), false)?.0,
                        Some(m) => {
                            let zero = self.constant([0.0; 4]);
                            self.texture(TEXTURE_PROJECT, m, zero, false, false, 0)?.0
                        }
                    };
                    let point = Operand { source: Source::Virtual(point), mode: MODE_AS_IS };
                    let held = self.procedural(procedural, *seed, point, mapping.is_some())?;
                    self.define(*dst, held)?;
                }
                Instruction::Curvature { dst, mode, radius, contrast } => {
                    if matches!(self.pictures, Pictures::Unknown) {
                        return Err(Refusal::Instruction("the curvature"));
                    }
                    let mode = match mode {
                        CurvatureMode::Convex => 0,
                        CurvatureMode::Concave => 1,
                        CurvatureMode::Both => 2,
                    };
                    let c = self.constant([radius.max(0.0), *contrast, 0.0, 0.0]);
                    let (d, _) = self.push(OP_ATTRIBUTE, ATTRIBUTE_CURVATURE | mode << 2, vec![c], false)?;
                    self.define(*dst, Self::virtual_held(d, Ty::Float, MODE_AS_IS))?;
                }
                Instruction::Occlusion { dst, contrast } => {
                    if matches!(self.pictures, Pictures::Unknown) {
                        return Err(Refusal::Instruction("the baked occlusion"));
                    }
                    let c = self.constant([0.0, contrast.max(0.05), 0.0, 0.0]);
                    let (d, _) = self.push(OP_ATTRIBUTE, ATTRIBUTE_OCCLUSION, vec![c], false)?;
                    self.define(*dst, Self::virtual_held(d, Ty::Float, MODE_AS_IS))?;
                }
                Instruction::Distance { dst } => {
                    if matches!(self.pictures, Pictures::Unknown) {
                        return Err(Refusal::Instruction("the distance to other parts"));
                    }
                    let c = self.constant([0.0; 4]);
                    let (d, _) = self.push(OP_ATTRIBUTE, ATTRIBUTE_DISTANCE, vec![c], false)?;
                    self.define(*dst, Self::virtual_held(d, Ty::Float, MODE_AS_IS))?;
                }
                Instruction::VertexColor { dst } => {
                    if matches!(self.pictures, Pictures::Unknown) {
                        return Err(Refusal::Instruction("vertex colours"));
                    }
                    let c = self.constant([0.0; 4]);
                    let (d, _) = self.push(OP_ATTRIBUTE, ATTRIBUTE_VERTEX_COLOR, vec![c], false)?;
                    self.define(*dst, Self::virtual_held(d, Ty::Color, MODE_AS_IS))?;
                }
                Instruction::VolumeMap { .. } => return Err(Refusal::Instruction("a volume map")),
            }
            i += 1;
        }
        Ok(())
    }

    /// An `OP_TEXTURE` instruction: `place` its first operand, the
    /// mapping's constants the next three.
    fn texture(&mut self, mode: u32, mapping: &Mapping, place: Operand, at: bool, colour_managed: bool, pyramid: u32) -> Result<(usize, Option<usize>), Refusal> {
        if pyramid as usize >= MAX_PYRAMIDS {
            return Err(Refusal::TooManyPictures);
        }
        let [c1, c2, c3] = mapping_constants(mapping);
        let operands = vec![place, self.constant(c1), self.constant(c2), self.constant(c3)];
        self.push(OP_TEXTURE, settings::texture(mode, mapping, at, colour_managed, pyramid), operands, false)
    }

    /// A seed as a constant: its two sixteen-bit halves, each exact as a
    /// number.
    fn seed(&mut self, seed: u32) -> Operand {
        self.constant([(seed & 0xFFFF) as f32, (seed >> 16) as f32, 0.0, 0.0])
    }

    /// A fractal of noise at `point` times `scale` (`noise.rs`'s fbm 0,
    /// turbulence 1, ridged 2), its answer brought to [0, 1] where `unit`.
    fn fractal(&mut self, kind: u32, point: Operand, scale: f32, fractal: Fractal, seed: u32, unit: bool) -> Result<Operand, Refusal> {
        let shape = self.constant([scale, fractal.lacunarity, fractal.gain, 0.0]);
        let seed = self.seed(seed);
        let (d, _) = self.push(OP_PROCEDURAL, settings::fractal(kind, fractal.basis, unit, fractal.octaves), vec![point, shape, seed], false)?;
        Ok(Operand { source: Source::Virtual(d), mode: MODE_AS_IS })
    }

    /// A Tequila procedural at `point` (the part's own, or a mapping's u,
    /// v where `mapped`): its instructions, and its value.
    fn procedural(&mut self, procedural: &Procedural, seed: u32, point: Operand, mapped: bool) -> Result<Held, Refusal> {
        let number = |d: usize| Self::virtual_held(d, Ty::Float, MODE_AS_IS);
        let at = |d: usize| Operand { source: Source::Virtual(d), mode: MODE_AS_IS };
        match *procedural {
            Procedural::Scratches { frequency, width, density, spread } => {
                let c = self.constant([frequency.max(1e-4), width.max(1e-4), density.clamp(0.0, 1.0), spread.clamp(0.0, 0.5)]);
                let s = self.seed(seed);
                let (d, _) = self.push(OP_PROCEDURAL, settings::procedural_stage(PATTERN_SCRATCHES, 0, Axis::Y, mapped), vec![point, c, s], false)?;
                Ok(number(d))
            }
            Procedural::BrushedRadial { axis, frequency, stretch } => {
                let c = self.constant([frequency.max(1e-4), stretch.max(1e-3), 0.0, 0.0]);
                let (q, _) = self.push(OP_PROCEDURAL, settings::procedural_stage(PATTERN_BRUSHED_RADIAL, 0, axis, mapped), vec![point, c], false)?;
                // Perlin noise: one octave of fBm, bit for bit.
                let one = Fractal { octaves: 1, ..Fractal::default() };
                let n = self.fractal(0, at(q), 1.0, one, seed, true)?;
                Ok(Held { operand: n, ty: Ty::Float })
            }
            Procedural::Leather { frequency, crack_width, wrinkle } => {
                let c = self.constant([frequency.max(1e-4), 0.0, 0.0, 0.0]);
                let (q, _) = self.push(OP_PROCEDURAL, settings::procedural_stage(PATTERN_LEATHER, 0, Axis::Y, mapped), vec![point, c], false)?;
                let s = self.seed(seed);
                let (cells, _) = self.push(OP_PROCEDURAL, settings::worley(Metric::Euclidean), vec![at(q), s], false)?;
                let fine = self.fractal(0, at(q), 6.0, Fractal { octaves: 3, ..Fractal::default() }, seed ^ 0x6a09_e667, true)?;
                let c = self.constant([crack_width, wrinkle.clamp(0.0, 1.0), 0.0, 0.0]);
                let (d, _) = self.push(OP_PROCEDURAL, settings::procedural_stage(PATTERN_LEATHER, 1, Axis::Y, mapped), vec![point, at(cells), fine, c], false)?;
                Ok(number(d))
            }
            ref other => Err(Refusal::Procedural(other.label())),
        }
    }

    /// One node, for the outputs read of it.
    fn apply(&mut self, kind: &NodeKind, args: &[Register], outputs: &[(Register, u8)]) -> Result<(), Refusal> {
        let operands = self.arguments(kind, args)?;
        let out_ty = |index: u8| kind.output_type_at(index).map_or(Ty::Float, ty_of);
        let wants = |index: u8| outputs.iter().any(|(_, o)| *o == index);
        match kind {
            NodeKind::Node(node) => self.apply_node(node, operands, outputs, &out_ty, &wants),
            NodeKind::Mix | NodeKind::Multiply | NodeKind::Add | NodeKind::Invert | NodeKind::Remap { .. } | NodeKind::Clamp { .. } | NodeKind::Channel { .. } | NodeKind::Bump { .. } => {
                let mut operands = operands;
                let which = match kind {
                    NodeKind::Mix => LEGACY_MIX,
                    NodeKind::Multiply => LEGACY_MULTIPLY,
                    NodeKind::Add => LEGACY_ADD,
                    NodeKind::Invert => LEGACY_INVERT,
                    NodeKind::Remap { from, to } => {
                        operands.truncate(1);
                        let c = self.constant([from.x, from.y, to.x, to.y]);
                        operands.push(c);
                        LEGACY_REMAP
                    }
                    NodeKind::Clamp { low, high } => {
                        operands.truncate(1);
                        let c = self.constant([*low, *high, 0.0, 0.0]);
                        operands.push(c);
                        LEGACY_CLAMP
                    }
                    NodeKind::Channel { channel } => {
                        operands.truncate(1);
                        LEGACY_CHANNEL | (*channel as u32) << 4
                    }
                    NodeKind::Bump { depth } => {
                        operands.truncate(1);
                        let c = self.constant([*depth; 4]);
                        operands.push(c);
                        LEGACY_BUMP
                    }
                    _ => unreachable!(),
                };
                // Every argument the node has, padded as the processor pads
                // a short list: Float(0).
                let wanted = match kind {
                    NodeKind::Mix => 3,
                    NodeKind::Multiply | NodeKind::Add => 2,
                    _ => operands.len(),
                };
                while operands.len() < wanted {
                    let zero = self.constant([0.0; 4]);
                    operands.push(zero);
                }
                let (d, _) = self.push(OP_LEGACY, which, operands, false)?;
                self.define(outputs[0].0, Self::virtual_held(d, out_ty(0), MODE_AS_IS))
            }
            NodeKind::Gradient { gradient } => {
                let mut operands = operands;
                operands.truncate(1);
                for stop in gradient.stops() {
                    let c = self.constant([stop.color.r, stop.color.g, stop.color.b, stop.at]);
                    operands.push(c);
                }
                let (d, _) = self.push(OP_GRADIENT, settings::gradient(gradient.interpolation(), gradient.stops().len()), operands, false)?;
                self.define(outputs[0].0, Self::virtual_held(d, Ty::Color, MODE_AS_IS))
            }
            other => Err(Refusal::Node(other.label())),
        }
    }

    fn apply_node(
        &mut self,
        node: &ShaderNode,
        mut operands: Vec<Operand>,
        outputs: &[(Register, u8)],
        out_ty: &dyn Fn(u8) -> Ty,
        wants: &dyn Fn(u8) -> bool,
    ) -> Result<(), Refusal> {
        // Every input the node has: a short argument list reads Float(0)
        // past its end, as `shade` pads it.
        let count = node.inputs().len();
        while operands.len() < count {
            let zero = self.constant([0.0; 4]);
            operands.push(zero);
        }
        // An instruction writing one register, each output read as it is.
        let single = |b: &mut Self, op: u32, settings: u32, operands: Vec<Operand>| -> Result<(), Refusal> {
            let (d, _) = b.push(op, settings, operands, false)?;
            for (register, output) in outputs {
                b.define(*register, Self::virtual_held(d, out_ty(*output), MODE_AS_IS))?;
            }
            Ok(())
        };
        match *node {
            ShaderNode::Math { op, clamp } => single(self, OP_MATH, settings::math(op, clamp), operands),
            ShaderNode::VectorMath { op } => single(self, OP_VECTOR_MATH, vector_math_code(op), operands),
            ShaderNode::MapRange { kind, clamp } => single(self, OP_MAP_RANGE, settings::map_range(kind, clamp), operands),
            ShaderNode::Mix { data, blend, clamp_factor, clamp_result } => {
                single(self, OP_MIX, settings::mix(data, blend, clamp_factor, clamp_result), operands)
            }
            ShaderNode::Mapping { kind } => single(self, OP_MAPPING, mapping_code(kind), operands),
            ShaderNode::CombineXyz => single(self, OP_COMBINE_XYZ, 0, operands),
            ShaderNode::Clamp { range } => single(self, OP_CLAMP, range as u32, operands),
            ShaderNode::CombineColor { model } => single(self, OP_COMBINE_COLOR, model_code(model), operands),
            ShaderNode::HueSaturationValue => single(self, OP_HSV, 0, operands),
            ShaderNode::BrightContrast => single(self, OP_BRIGHT_CONTRAST, 0, operands),
            ShaderNode::Gamma => single(self, OP_GAMMA, 0, operands),
            ShaderNode::InvertColor => single(self, OP_INVERT_COLOR, 0, operands),
            ShaderNode::Fresnel => single(self, OP_FRESNEL, 0, operands),
            // Fac and Color as one: the Fac in lane 0 of the colour, or in
            // every lane when only it is read.
            ShaderNode::Checker | ShaderNode::GradientTexture { .. } => {
                let (op, s) = match *node {
                    ShaderNode::GradientTexture { kind } => (OP_GRADIENT_TEXTURE, gradient_kind_code(kind)),
                    _ => (OP_CHECKER, 0),
                };
                // Color output 0, Fac output 1: the colour in xyz, the Fac in w.
                let (d, _) = self.push(op, s, operands, false)?;
                for (register, output) in outputs {
                    let held = if *output == 0 { Self::virtual_held(d, Ty::Color, MODE_AS_IS) } else { Self::virtual_held(d, Ty::Float, MODE_LANE0 + 3) };
                    self.define(*register, held)?;
                }
                Ok(())
            }
            ShaderNode::Noise { dims, fractal, normalize } => {
                let colour = wants(1);
                let (d, _) = self.push(OP_NOISE, settings::noise(dims, fractal, normalize, colour), operands, false)?;
                for (register, output) in outputs {
                    let held = match (*output, colour) {
                        (0, true) => Self::virtual_held(d, Ty::Float, MODE_LANE0),
                        (0, false) => Self::virtual_held(d, Ty::Float, MODE_AS_IS),
                        _ => Self::virtual_held(d, Ty::Color, MODE_AS_IS),
                    };
                    self.define(*register, held)?;
                }
                Ok(())
            }
            ShaderNode::WhiteNoise { dims } => {
                let colour = wants(1);
                let (d, _) = self.push(OP_WHITE_NOISE, settings::white_noise(dims, colour), operands, false)?;
                for (register, output) in outputs {
                    let held = match (*output, colour) {
                        (0, true) => Self::virtual_held(d, Ty::Float, MODE_LANE0),
                        (0, false) => Self::virtual_held(d, Ty::Float, MODE_AS_IS),
                        _ => Self::virtual_held(d, Ty::Color, MODE_AS_IS),
                    };
                    self.define(*register, held)?;
                }
                Ok(())
            }
            // The colour and the distance (or radius) in one register, the
            // position and W in a second where they are read.
            ShaderNode::Voronoi { dims, feature, metric, normalize } => {
                let single_output = matches!(feature, VoronoiFeature::DistanceToEdge | VoronoiFeature::NSphereRadius);
                let colour = !single_output && wants(1);
                let position = !single_output && (wants(2) || wants(3));
                let (d, d2) = self.push(OP_VORONOI, settings::voronoi(dims, feature, metric, normalize, colour), operands, position)?;
                for (register, output) in outputs {
                    let held = match (*output, single_output) {
                        (_, true) | (0, false) => Self::virtual_held(d, Ty::Float, MODE_LANE0 + 3),
                        (1, false) => Self::virtual_held(d, Ty::Color, MODE_AS_IS),
                        (2, false) => Self::virtual_held(d2.expect("a position register"), Ty::Vector, MODE_AS_IS),
                        _ => Self::virtual_held(d2.expect("a position register"), Ty::Float, MODE_LANE0 + 3),
                    };
                    self.define(*register, held)?;
                }
                Ok(())
            }
            // Its point, the Noise Texture's 3D fBM there at the detail
            // scale (what its distortion reads), and its value: Color (the
            // value in three lanes) and Fac from one register.
            ShaderNode::Wave { kind, direction, profile } => {
                // Stage 1's lowering ran no Wave.
                if matches!(self.pictures, Pictures::Unknown) {
                    return Err(Refusal::Node(node.label()));
                }
                // The fBm unclamped, where the Noise Texture's clamps its
                // detail to 0-15 and its roughness to no less than 0: a
                // constant outside those is refused rather than read
                // otherwise.
                let outside = |o: &Operand, ok: &dyn Fn(f32) -> bool| match o.source {
                    Source::Constant(c) => !ok(self.constants[c as usize][0]),
                    Source::Virtual(_) => false,
                };
                let undistorted = matches!(operands[2].source, Source::Constant(c) if self.constants[c as usize][0] == 0.0);
                if !undistorted && (outside(&operands[3], &|d| (0.0..=15.0).contains(&d)) || outside(&operands[5], &|r| r >= 0.0)) {
                    return Err(Refusal::Instruction("a Wave Texture's detail outside 0 to 15, or its roughness below 0"));
                }
                let (p, _) = self.push(OP_WAVE, settings::wave(kind, direction, profile, false), vec![operands[0], operands[1]], false)?;
                let at = Operand { source: Source::Virtual(p), mode: MODE_AS_IS };
                let zero = self.constant([0.0; 4]);
                let two = self.constant([2.0; 4]);
                let one = self.constant([1.0; 4]);
                let noise = vec![at, zero, operands[4], operands[3], operands[5], two, zero, one, zero];
                let (n, _) = self.push(OP_NOISE, settings::noise(3, FractalKind::Fbm, true, false), noise, false)?;
                let fbm = Operand { source: Source::Virtual(n), mode: MODE_AS_IS };
                let (d, _) = self.push(OP_WAVE, settings::wave(kind, direction, profile, true), vec![at, fbm, operands[2], operands[6]], false)?;
                for (register, output) in outputs {
                    let ty = if *output == 0 { Ty::Color } else { Ty::Float };
                    self.define(*register, Self::virtual_held(d, ty, MODE_AS_IS))?;
                }
                Ok(())
            }
            // Value, Phase and Intensity in the first three lanes.
            ShaderNode::Gabor { kind } => {
                // Stage 1's lowering ran no Gabor.
                if matches!(self.pictures, Pictures::Unknown) {
                    return Err(Refusal::Node(node.label()));
                }
                let mut operands = operands;
                operands.truncate(6);
                let normalization = self.constant([gabor_normalization(kind); 4]);
                operands.push(normalization);
                let (d, _) = self.push(OP_GABOR, (kind == GaborKind::Space) as u32, operands, false)?;
                for (register, output) in outputs {
                    self.define(*register, Self::virtual_held(d, Ty::Float, MODE_LANE0 + (*output).min(2) as u32))?;
                }
                Ok(())
            }
            ShaderNode::LayerWeight => {
                let (d, _) = self.push(OP_LAYER_WEIGHT, 0, operands, false)?;
                for (register, output) in outputs {
                    self.define(*register, Self::virtual_held(d, Ty::Float, MODE_LANE0 + *output as u32))?;
                }
                Ok(())
            }
            // No instruction: each reader reads its lane of the vector.
            ShaderNode::SeparateXyz | ShaderNode::SeparateColor { model: ColorModel::Rgb } => {
                let source = Held { operand: operands[0], ty: Ty::Vector };
                for (register, output) in outputs {
                    let lane = source.lane((*output).min(2) as u32);
                    self.define(*register, Held { operand: lane, ty: Ty::Float })?;
                }
                Ok(())
            }
            ShaderNode::SeparateColor { model } => {
                let (d, _) = self.push(OP_SEPARATE_COLOR, model_code(model), operands, false)?;
                for (register, output) in outputs {
                    self.define(*register, Self::virtual_held(d, Ty::Float, MODE_LANE0 + (*output).min(2) as u32))?;
                }
                Ok(())
            }
            ref other => Err(Refusal::Node(other.label())),
        }
    }
}

/// Where in `GpuMaterial` a parameter the card's stack reads is, as a float
/// offset, and whether it is a colour (three floats).
pub fn material_slot(param: Param) -> Option<(u32, bool)> {
    use crate::material::GpuMaterial as M;
    use std::mem::offset_of;
    let at = |offset: usize| (offset / 4) as u32;
    Some(match param {
        Param::BaseWeight => (at(offset_of!(M, base_weight)), false),
        Param::BaseColor => (at(offset_of!(M, base_color)), true),
        Param::BaseMetalness => (at(offset_of!(M, base_metalness)), false),
        Param::DiffuseRoughness => (at(offset_of!(M, base_diffuse_roughness)), false),
        Param::SpecularWeight => (at(offset_of!(M, specular_weight)), false),
        Param::SpecularColor => (at(offset_of!(M, specular_color)), true),
        Param::SpecularRoughness => (at(offset_of!(M, specular_roughness)), false),
        Param::SpecularIor => (at(offset_of!(M, specular_ior)), false),
        Param::Anisotropy => (at(offset_of!(M, specular_roughness_anisotropy)), false),
        Param::AnisotropyRotation => (at(offset_of!(M, specular_rotation)), false),
        Param::CoatWeight => (at(offset_of!(M, coat_weight)), false),
        Param::CoatColor => (at(offset_of!(M, coat_color)), true),
        Param::CoatRoughness => (at(offset_of!(M, coat_roughness)), false),
        Param::CoatIor => (at(offset_of!(M, coat_ior)), false),
        Param::CoatDarkening => (at(offset_of!(M, coat_darkening)), false),
        Param::FuzzWeight => (at(offset_of!(M, fuzz_weight)), false),
        Param::FuzzColor => (at(offset_of!(M, fuzz_color)), true),
        Param::FuzzRoughness => (at(offset_of!(M, fuzz_roughness)), false),
        Param::Occlusion => (at(offset_of!(M, geometry_occlusion)), false),
        _ => return None,
    })
}

/// How a material's surface is bumped, as `BumpSource::of` decides.
enum Bump {
    None,
    /// By the height in this parameter.
    Height(Param),
    /// By a normal map through the UVs: its texel the Bump's register.
    NormalUv,
    /// By a normal map laid down by a projection of its own.
    NormalProjected { pyramid: u32, mapping: Mapping },
}

/// How `program` bumps its surface, as `BumpSource::of` decides: the Bump's
/// height, else a Displacement's with no detail to displace by; but a Bump
/// a picture drives is a normal map where the picture is one, and bumps
/// nothing where it is not loaded.
fn bump_of(program: &CompiledGraph, pictures: Pictures) -> Bump {
    let register = |wanted: Param| program.driven.iter().find(|(p, _)| *p == wanted).map(|(_, r)| *r);
    if let Some(register) = register(Param::Bump) {
        // The last instruction to write the register is what reaches the
        // socket; none, and nothing does.
        let Some(writer) = program.instructions.iter().rev().find(|i| i.destination() == register) else {
            return Bump::None;
        };
        if let (Instruction::Sample { texture, mapping, .. }, Pictures::On(pictures)) = (writer, pictures) {
            match pictures.texture(*texture) {
                Some(card) if card.normal && mapping.projection != Projection::Uv => {
                    return Bump::NormalProjected { pyramid: card.colour, mapping: **mapping };
                }
                Some(card) if card.normal => return Bump::NormalUv,
                Some(_) => {}
                None => return Bump::None,
            }
        }
        Bump::Height(Param::Bump)
    } else if register(Param::Displacement).is_some() && program.constants.displacement_detail <= 0.0 {
        Bump::Height(Param::Displacement)
    } else {
        Bump::None
    }
}

/// `program` lowered for the card: `Ok(None)` where the processor's `shade`
/// takes its fast path and the material is its constants, `Err` where the
/// card cannot run it. With no pictures to read, as stage 1 lowered it: no
/// image textures, no projections.
pub fn lower(program: &CompiledGraph) -> Result<Option<CardProgram>, Refusal> {
    lower_with(program, Pictures::Unknown)
}

/// [`lower`], reading the scene's `pictures`.
pub fn lower_with(program: &CompiledGraph, pictures: Pictures) -> Result<Option<CardProgram>, Refusal> {
    if program.layer.is_some() {
        return Err(Refusal::Layered);
    }
    if !program.decals.is_empty() {
        return Err(Refusal::Decals);
    }
    if program.instructions.is_empty() || program.driven.is_empty() {
        return Ok(None);
    }
    let shape = bump_of(program, pictures);
    let bump = match shape {
        Bump::Height(param) => Some(param),
        _ => None,
    };
    for (param, _) in &program.driven {
        let read = material_slot(*param).is_some()
            || *param == Param::Tangent
            || Some(*param) == bump
            // A Bump that bumps nothing (its picture not loaded).
            || *param == Param::Bump
            // A displacement with detail moved the geometry when the scene
            // was built, and shades no further.
            || (*param == Param::Displacement && program.constants.displacement_detail > 0.0);
        if !read {
            return Err(Refusal::Param(*param));
        }
    }
    let all: Vec<Param> = program.driven.iter().map(|(p, _)| *p).collect();
    let main = program.restricted_to(&all);

    let mut constants = Vec::new();
    let mut pooled = HashMap::new();
    let mut builder = Builder { constants: &mut constants, pooled: &mut pooled, pictures, held: HashMap::new(), ops: Vec::new(), virtuals: 0 };
    builder.lower(&main.instructions)?;
    let mut ends: Vec<Operand> = Vec::new();
    let mut writes: Vec<(u32, Operand)> = Vec::new();
    let mut here = None;
    let mut tangent = None;
    for (param, register) in &main.driven {
        let held = builder.held(*register)?;
        if Some(*param) == bump {
            let operand = held.read_as(Ty::Float);
            here = Some(operand);
            ends.push(operand);
        } else if *param == Param::Bump {
            // A normal map's texel, as the colour it is.
            if let Bump::NormalUv = shape {
                here = Some(held.operand);
                ends.push(held.operand);
            }
        } else if *param == Param::Tangent {
            tangent = Some(held.operand);
            ends.push(held.operand);
        } else if let Some((offset, colour)) = material_slot(*param) {
            let operand = held.read_as(if colour { Ty::Color } else { Ty::Float });
            writes.push((offset | (colour as u32) << 8, operand));
            ends.push(operand);
        }
    }
    // A normal map laid down by a projection: the normal it gives, one more
    // instruction after the program.
    if let Bump::NormalProjected { pyramid, mapping } = &shape {
        let zero = builder.constant([0.0; 4]);
        let (d, _) = builder.texture(TEXTURE_NORMAL, mapping, zero, false, false, *pyramid)?;
        let operand = Operand { source: Source::Virtual(d), mode: MODE_AS_IS };
        here = Some(operand);
        ends.push(operand);
    }
    let main_ops = std::mem::take(&mut builder.ops);
    let (main_words, main_registers, map) = encode(&main_ops, builder.virtuals, &ends)?;
    let resolve = |o: Operand, map: &[u32]| encode_operand(o, map);

    let mut card = CardProgram {
        main: main_words,
        writes: writes.iter().flat_map(|(slot, o)| [*slot, resolve(*o, &map)]).collect(),
        bump_here: here.map_or(NONE, |o| resolve(o, &map)),
        bump_height: NONE,
        tangent: tangent.map_or(NONE, |o| resolve(o, &map)),
        registers: main_registers,
        normal_map: match shape {
            Bump::NormalUv => PROGRAM_NORMAL_UV,
            Bump::NormalProjected { .. } => PROGRAM_NORMAL_PROJECTED,
            _ => 0,
        },
        ..CardProgram::default()
    };

    if let Some(param) = bump {
        let restricted = program.restricted_to(&[param]);
        let mut builder = Builder { constants: &mut constants, pooled: &mut pooled, pictures, held: HashMap::new(), ops: Vec::new(), virtuals: 0 };
        builder.lower(&restricted.instructions)?;
        let register = restricted.driven.iter().find(|(p, _)| *p == param).map(|(_, r)| *r).expect("the bump is driven");
        let height = builder.held(register)?.read_as(Ty::Float);
        let ops = std::mem::take(&mut builder.ops);
        let (words, registers, map) = encode(&ops, builder.virtuals, &[height])?;
        card.bump = words;
        card.bump_height = encode_operand(height, &map);
        card.registers = card.registers.max(registers);
    }
    card.constants = constants;
    Ok(Some(card))
}

fn encode_operand(o: Operand, map: &[u32]) -> u32 {
    let index = match o.source {
        Source::Virtual(v) => map[v],
        Source::Constant(c) => c | OPERAND_CONSTANT,
    };
    index | o.mode << OPERAND_MODE_SHIFT
}

/// The operations with physical registers, allocated by liveness: a
/// register is free once the last operation reading it has read it, which
/// may be the operation writing the next value into it (each reads all its
/// operands before it writes). `ends` are read after the program.
fn encode(ops: &[Op], virtuals: usize, ends: &[Operand]) -> Result<(Vec<u32>, usize, Vec<u32>), Refusal> {
    let mut last = vec![0usize; virtuals];
    for (i, op) in ops.iter().enumerate() {
        for o in &op.operands {
            if let Source::Virtual(v) = o.source {
                last[v] = last[v].max(i);
            }
        }
        for d in op.dst.iter().chain(op.dst2.iter()) {
            // A value never read is free after it is written.
            last[*d] = last[*d].max(i);
        }
    }
    for o in ends {
        if let Source::Virtual(v) = o.source {
            last[v] = usize::MAX;
        }
    }
    let mut map = vec![u32::MAX; virtuals];
    let mut free: Vec<u32> = (0..REGISTERS as u32).rev().collect();
    let mut in_use = 0usize;
    let mut peak = 0usize;
    let mut words = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        // Operands read for the last time here free their registers first.
        let mut freed: Vec<usize> = op
            .operands
            .iter()
            .filter_map(|o| match o.source {
                Source::Virtual(v) if last[v] == i => Some(v),
                _ => None,
            })
            .collect();
        freed.sort();
        freed.dedup();
        for v in freed {
            if map[v] != u32::MAX {
                free.push(map[v]);
                in_use -= 1;
            }
        }
        let mut allocate = |v: usize, free: &mut Vec<u32>, in_use: &mut usize| -> Result<u32, Refusal> {
            let r = free.pop().ok_or(Refusal::Registers(REGISTERS + 1))?;
            map[v] = r;
            *in_use += 1;
            Ok(r)
        };
        let dst = match op.dst {
            Some(v) => allocate(v, &mut free, &mut in_use)?,
            None => 0xFF,
        };
        let dst2 = match op.dst2 {
            Some(v) => allocate(v, &mut free, &mut in_use)?,
            None => 0xFF,
        };
        peak = peak.max(in_use);
        words.push(op.op | dst << 8 | dst2 << 16 | (op.operands.len() as u32) << 24);
        words.push(op.settings);
        for o in &op.operands {
            words.push(encode_operand(*o, &map));
        }
        // A value nothing reads is free once written.
        for d in op.dst.iter().chain(op.dst2.iter()) {
            if last[*d] == i {
                free.push(map[*d]);
                in_use -= 1;
            }
        }
    }
    if peak > REGISTERS {
        return Err(Refusal::Registers(peak));
    }
    Ok((words, peak, map))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opcodes_are_programs_wgsls() {
        let wgsl = crate::shader::module("program").expect("program.wgsl");
        for (name, value) in OPCODES {
            let line = format!("const {name}: u32 = {value}u;");
            assert!(wgsl.contains(&line), "program.wgsl has no `{line}`");
        }
    }

    #[test]
    fn the_procedural_numbers_are_procedural_wgsls() {
        let wgsl = crate::shader::module("procedural").expect("procedural.wgsl");
        for (name, value) in [
            ("TQ_SCRATCHES", PATTERN_SCRATCHES),
            ("TQ_BRUSHED_RADIAL", PATTERN_BRUSHED_RADIAL),
            ("TQ_LEATHER", PATTERN_LEATHER),
            ("TQ_STAGE", PROCEDURAL_STAGE),
            ("TQ_FRACTAL", PROCEDURAL_FRACTAL),
            ("TQ_WORLEY", PROCEDURAL_WORLEY),
        ] {
            let line = format!("const {name}: u32 = {value}u;");
            assert!(wgsl.contains(&line), "procedural.wgsl has no `{line}`");
        }
        let line = format!("const TQ_MAPPED: u32 = 0x{PROCEDURAL_MAPPED:X}u;");
        assert!(wgsl.contains(&line), "procedural.wgsl has no `{line}`");
    }
}
